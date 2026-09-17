//! The body of a backup: a run of sealed chunks, and the rules that make cutting one off,
//! moving one or rewriting one fail instead of going unnoticed.
//!
//! The naive encrypted file is one tag over the whole thing, which cannot be verified until
//! the whole thing is in memory, and this format has to run on a phone. The obvious repair
//! is to cut the stream into chunks and seal each one, and the obvious repair is wrong on
//! its own: an attacker can drop the tail, reorder two chunks or repeat one, and every
//! remaining tag still verifies. What is missing is that a chunk says nothing about where
//! it sits.
//!
//! So four things go into the associated data of every chunk, and none of them is stored
//! anywhere an attacker can edit:
//!
//! - the whole sixty-four byte header, so the salt, the parameters and the nonce base
//!   cannot be changed without every tag in the file failing;
//! - the chunk's index, so two chunks cannot be swapped;
//! - whether the chunk is the last one, so the tail cannot be cut off at a chunk boundary;
//! - and, through the nonce, the index again, because the nonce is recomputed from the
//!   header's base rather than read from the file.
//!
//! A chunk on disk is ciphertext followed by its sixteen byte tag, with no length in front
//! of it. Nothing to lie about before the tag has had its say. The reader knows a chunk is
//! not the last one because at least one byte follows it, and it knows that is sound
//! because every chunk is at least a tag long: after a full chunk there are always at least
//! sixteen more bytes, or the file ended there and that chunk was supposed to be the last.
//!
//! Truncation exactly on a chunk boundary is the case worth following through. The last
//! full chunk was sealed with "I am not the last" in its associated data. With the tail
//! gone, the reader now finds nothing after it, calls it the last chunk, and the tag fails.
//! There is no arrangement of a shortened file that verifies.

use zeroize::Zeroizing;

use crate::aad::Aad;
use crate::aead::{Sealed, TAG_LEN, open, seal};
use crate::backup::header::BackupHeader;
use crate::error::CryptoError;
use crate::keys::DataKey;
use crate::stream::{ChunkNonces, chunk_nonce};

/// Length of the associated data of one chunk, in bytes.
///
/// The header, then eight bytes of index, then one byte saying whether this is the last
/// chunk. Seventy-three bytes per sixty-four kibibytes of content is below the noise floor
/// of any measurement, which is why the header goes into every chunk rather than only into
/// the first: it removes the case where one chunk is treated differently from the others,
/// and that case is exactly where this kind of bug lives.
pub const CHUNK_AAD_LEN: usize = crate::backup::header::BACKUP_HEADER_LEN + 8 + 1;

/// Builds the associated data of one chunk.
fn chunk_aad(
    header: &[u8; crate::backup::header::BACKUP_HEADER_LEN],
    index: u64,
    last: bool,
) -> Aad {
    let mut bytes = Vec::with_capacity(CHUNK_AAD_LEN);

    bytes.extend_from_slice(header);
    bytes.extend_from_slice(&index.to_be_bytes());
    bytes.push(u8::from(last));

    Aad::authenticated_prefix(&bytes)
}

/// Seals the chunks of one backup, in order.
///
/// Holds the nonce stream, so there is one of these per file and no way to start a second
/// one over the same base. It takes plaintext and gives back bytes to write; it opens
/// nothing and knows nothing about a file, which is what keeps this crate free of input and
/// output and checkable under Miri.
#[derive(Debug)]
pub struct ChunkSealer<'a> {
    key: &'a DataKey,
    header: [u8; crate::backup::header::BACKUP_HEADER_LEN],
    chunk_len: usize,
    nonces: ChunkNonces,
    pending: Vec<u8>,
    finished: bool,
}

impl<'a> ChunkSealer<'a> {
    /// Starts sealing a backup whose header and nonce stream have already been decided.
    ///
    /// The nonce stream is taken by value. It cannot be cloned, so this is the only sealer
    /// that will ever exist over that base, which is the property the whole framing rests
    /// on.
    #[must_use]
    pub fn new(key: &'a DataKey, header: &BackupHeader, nonces: ChunkNonces) -> Self {
        Self {
            key,
            header: header.to_bytes(),
            chunk_len: header.chunk_len() as usize,
            nonces,
            pending: Vec::new(),
            finished: false,
        }
    }

    /// Adds plaintext, appending every chunk that is now complete to `out`.
    ///
    /// A chunk is emitted only when a whole one has accumulated, so a caller pushing one
    /// byte at a time and a caller pushing a mebibyte at a time produce the same file.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::ChunkCountExhausted`] if the file has more chunks than one
    /// nonce base may cover, and [`CryptoError::Open`] if the cipher refuses.
    pub fn push(&mut self, plaintext: &[u8], out: &mut Vec<u8>) -> Result<(), CryptoError> {
        debug_assert!(!self.finished, "a sealer was used after it was finished");

        self.pending.extend_from_slice(plaintext);

        while self.pending.len() >= self.chunk_len {
            let rest = self.pending.split_off(self.chunk_len);
            let full = core::mem::replace(&mut self.pending, rest);

            self.emit(&full, false, out)?;
        }

        Ok(())
    }

    /// Seals whatever is left as the last chunk, and reports how many chunks the file has.
    ///
    /// Always emits one, even when nothing is left. An empty vault is a header and one
    /// chunk holding an empty compressed stream, which is a valid file that imports into an
    /// empty database; a file with no chunks at all would have no "last" tag, and therefore
    /// nothing to distinguish it from a file whose body was deleted.
    ///
    /// # Errors
    ///
    /// The same as [`ChunkSealer::push`].
    pub fn finish(mut self, out: &mut Vec<u8>) -> Result<u64, CryptoError> {
        let last = core::mem::take(&mut self.pending);
        self.emit(&last, true, out)?;
        self.finished = true;

        Ok(self.nonces.issued())
    }

    /// Seals one chunk and appends it to `out`.
    fn emit(&mut self, plaintext: &[u8], last: bool, out: &mut Vec<u8>) -> Result<(), CryptoError> {
        let index = self.nonces.issued();
        let nonce = self.nonces.issue()?;
        let aad = chunk_aad(&self.header, index, last);

        let sealed = seal(self.key, nonce, &aad, plaintext)?;
        // The nonce is not written. It is recomputed from the header's base and the chunk's
        // position, so there is nothing about it in the file for anybody to choose.
        out.extend_from_slice(sealed.ciphertext());

        Ok(())
    }
}

/// Opens the chunks of one backup, in order.
///
/// Mirrors [`ChunkSealer`] and, deliberately, does not hold a [`ChunkNonces`]. The read
/// path recomputes the nonces it needs from the header; handing it a nonce stream would be
/// handing the code that decrypts a permission to encrypt.
#[derive(Debug)]
pub struct ChunkOpener<'a> {
    key: &'a DataKey,
    header: [u8; crate::backup::header::BACKUP_HEADER_LEN],
    nonce_base: [u8; crate::stream::NONCE_BASE_LEN],
    on_disk_len: usize,
    index: u64,
    pending: Vec<u8>,
}

impl<'a> ChunkOpener<'a> {
    /// Starts reading a backup whose header has already been parsed.
    #[must_use]
    pub fn new(key: &'a DataKey, header: &BackupHeader) -> Self {
        Self {
            key,
            header: header.to_bytes(),
            nonce_base: *header.nonce_base(),
            on_disk_len: header.chunk_len() as usize + TAG_LEN,
            index: 0,
            pending: Vec::new(),
        }
    }

    /// Adds bytes read from the file, appending the plaintext of every complete chunk.
    ///
    /// The test for "complete" is that strictly more than one chunk's worth of bytes is
    /// waiting. That is sound rather than convenient: every chunk on disk is at least a tag
    /// long, so a chunk that is not the last is always followed by at least sixteen bytes.
    /// A file that ends exactly at a chunk boundary therefore leaves that chunk waiting,
    /// and [`ChunkOpener::finish`] opens it as the last one, which is the check that makes
    /// truncation on a boundary fail.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Open`] for anything that does not verify: a wrong key, a
    /// chunk moved, a chunk repeated, a byte changed, a file cut short. All of them are one
    /// error, because any difference between them is an oracle for whoever is editing the
    /// file.
    pub fn push(&mut self, bytes: &[u8], out: &mut Zeroizing<Vec<u8>>) -> Result<(), CryptoError> {
        self.pending.extend_from_slice(bytes);

        while self.pending.len() > self.on_disk_len {
            let rest = self.pending.split_off(self.on_disk_len);
            let full = core::mem::replace(&mut self.pending, rest);

            let plaintext = self.take(full, false)?;
            out.extend_from_slice(&plaintext);
        }

        Ok(())
    }

    /// Opens what is left as the last chunk, and reports how many chunks the file had.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Damaged`] if what remains is too short to be a chunk at all,
    /// which is what a file cut off in the middle of a tag looks like, and
    /// [`CryptoError::Open`] for anything that does not verify.
    pub fn finish(mut self, out: &mut Zeroizing<Vec<u8>>) -> Result<u64, CryptoError> {
        if self.pending.len() < TAG_LEN {
            return Err(CryptoError::Damaged);
        }

        let last = core::mem::take(&mut self.pending);
        let plaintext = self.take(last, true)?;
        out.extend_from_slice(&plaintext);

        Ok(self.index)
    }

    /// Opens one chunk at the position this opener has reached.
    fn take(&mut self, ciphertext: Vec<u8>, last: bool) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        let index = self.index;
        let Some(following) = index.checked_add(1) else {
            return Err(CryptoError::ChunkCountExhausted);
        };
        self.index = following;

        let nonce = chunk_nonce(&self.nonce_base, index);
        let aad = chunk_aad(&self.header, index, last);

        open(self.key, &Sealed::from_parts(nonce, ciphertext)?, &aad)
    }
}

#[cfg(test)]
mod tests {
    use zeroize::Zeroizing;

    use super::{CHUNK_AAD_LEN, ChunkOpener, ChunkSealer};
    use crate::aead::TAG_LEN;
    use crate::backup::header::{BACKUP_HEADER_LEN, BackupHeader, MIN_CHUNK_LEN};
    use crate::error::CryptoError;
    use crate::kdf::{Argon2Params, SALT_LEN};
    use crate::keys::{DataKey, KEY_LEN};
    use crate::stream::ChunkNonces;

    const SALT: [u8; SALT_LEN] = [0x33; SALT_LEN];

    fn key() -> DataKey {
        DataKey::from_bytes([0x5a; KEY_LEN])
    }

    /// A header with the smallest legal chunk size, so a test can make several chunks
    /// without allocating megabytes.
    fn small_chunks() -> (BackupHeader, ChunkNonces) {
        let nonces = ChunkNonces::generate().unwrap();
        let base = *nonces.base();

        let mut bytes = BackupHeader::new(SALT, Argon2Params::DEFAULT, base).to_bytes();
        if let Some(target) = bytes.get_mut(56..60) {
            target.copy_from_slice(&MIN_CHUNK_LEN.to_le_bytes());
        }

        (BackupHeader::parse(&bytes).unwrap(), nonces)
    }

    /// Seals a body and hands back the header it was sealed under and the bytes on disk.
    fn sealed(body: &[u8]) -> (BackupHeader, Vec<u8>, u64) {
        let (header, nonces) = small_chunks();
        let key = key();

        let mut out = Vec::new();
        let mut sealer = ChunkSealer::new(&key, &header, nonces);
        sealer.push(body, &mut out).unwrap();
        let chunks = sealer.finish(&mut out).unwrap();

        (header, out, chunks)
    }

    /// Opens a body, pushing the whole thing in one go.
    fn opened(header: &BackupHeader, on_disk: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let key = key();
        let mut out = Zeroizing::new(Vec::new());
        let mut opener = ChunkOpener::new(&key, header);

        opener.push(on_disk, &mut out)?;
        opener.finish(&mut out)?;

        Ok(out.to_vec())
    }

    #[test]
    fn the_associated_data_is_the_length_the_format_says() {
        // Published byte by byte, so the number is frozen rather than derived from whatever
        // the code happens to build.
        assert_eq!(CHUNK_AAD_LEN, BACKUP_HEADER_LEN + 8 + 1);
        assert_eq!(CHUNK_AAD_LEN, 73);
    }

    #[test]
    fn an_empty_body_is_one_chunk_and_comes_back_empty() {
        // An empty vault still produces a file, and that file still has a last chunk with a
        // tag over it. Without one there would be nothing to tell an empty backup from a
        // backup whose body somebody deleted.
        let (header, on_disk, chunks) = sealed(b"");

        assert_eq!(chunks, 1);
        assert_eq!(on_disk.len(), TAG_LEN);
        assert!(opened(&header, &on_disk).unwrap().is_empty());
    }

    #[test]
    fn a_body_shorter_than_one_chunk_round_trips() {
        let body = b"un cuerpo corto";
        let (header, on_disk, chunks) = sealed(body);

        assert_eq!(chunks, 1);
        assert_eq!(opened(&header, &on_disk).unwrap(), body);
    }

    #[test]
    fn a_body_of_exactly_one_chunk_round_trips() {
        // The boundary where a naive implementation writes one chunk and then has nothing
        // to mark as last. This format writes an empty last chunk instead, which costs
        // sixteen bytes and removes the case entirely.
        let body = vec![0xab_u8; MIN_CHUNK_LEN as usize];
        let (header, on_disk, chunks) = sealed(&body);

        assert_eq!(chunks, 2);
        assert_eq!(opened(&header, &on_disk).unwrap(), body);
    }

    #[test]
    fn a_body_of_one_chunk_and_one_byte_round_trips() {
        let body = vec![0xcd_u8; MIN_CHUNK_LEN as usize + 1];
        let (header, on_disk, chunks) = sealed(&body);

        assert_eq!(chunks, 2);
        assert_eq!(opened(&header, &on_disk).unwrap(), body);
    }

    #[test]
    fn a_body_of_several_chunks_round_trips() {
        let body: Vec<u8> = (0..(MIN_CHUNK_LEN as usize * 3 + 17))
            .map(|index| u8::try_from(index % 251).unwrap_or(0))
            .collect();
        let (header, on_disk, chunks) = sealed(&body);

        assert_eq!(chunks, 4);
        assert_eq!(opened(&header, &on_disk).unwrap(), body);
    }

    #[test]
    fn the_same_file_comes_out_whatever_size_the_pushes_are() {
        // The property that lets the caller stream: a writer handed a byte at a time and a
        // writer handed the whole body produce the same file, so buffering upstream cannot
        // change the format.
        let body: Vec<u8> = (0..(MIN_CHUNK_LEN as usize * 2 + 9))
            .map(|index| u8::try_from(index % 251).unwrap_or(0))
            .collect();

        let (header, nonces) = small_chunks();
        let key = key();
        let mut byte_at_a_time = Vec::new();
        let mut sealer = ChunkSealer::new(&key, &header, nonces);
        for byte in &body {
            sealer
                .push(core::slice::from_ref(byte), &mut byte_at_a_time)
                .unwrap();
        }
        sealer.finish(&mut byte_at_a_time).unwrap();

        assert_eq!(opened(&header, &byte_at_a_time).unwrap(), body);
    }

    #[test]
    fn the_reader_does_not_care_how_the_bytes_arrive() {
        let body = vec![0x77_u8; MIN_CHUNK_LEN as usize * 2 + 5];
        let (header, on_disk, _) = sealed(&body);

        let key = key();
        let mut out = Zeroizing::new(Vec::new());
        let mut opener = ChunkOpener::new(&key, &header);
        for piece in on_disk.chunks(7) {
            opener.push(piece, &mut out).unwrap();
        }
        opener.finish(&mut out).unwrap();

        assert_eq!(out.as_slice(), body.as_slice());
    }

    #[test]
    fn a_file_truncated_anywhere_fails() {
        // Every possible cut, not a representative one. A format where some truncations are
        // caught and others are not is a format that silently loses the tail, which is the
        // single worst thing a backup can do: it restores, and it is not what was saved.
        let body = vec![0x5e_u8; MIN_CHUNK_LEN as usize * 2 + 3];
        let (header, on_disk, _) = sealed(&body);

        for cut in 0..on_disk.len() {
            let shortened = on_disk.get(..cut).unwrap_or_default();
            assert!(
                opened(&header, shortened).is_err(),
                "a file truncated to {cut} of {} bytes was accepted",
                on_disk.len()
            );
        }
    }

    #[test]
    fn a_file_truncated_exactly_on_a_chunk_boundary_fails() {
        // The case a length prefix would miss and the one the "last chunk" marker exists
        // for. What is left is a run of whole chunks with valid tags; the only thing wrong
        // with it is that the chunk now at the end was sealed saying it was not.
        let body = vec![0x5e_u8; MIN_CHUNK_LEN as usize * 3];
        let (header, on_disk, _) = sealed(&body);
        let boundary = MIN_CHUNK_LEN as usize + TAG_LEN;

        assert!(matches!(
            opened(&header, on_disk.get(..boundary).unwrap()),
            Err(CryptoError::Open)
        ));
    }

    #[test]
    fn changing_any_single_byte_of_the_body_fails() {
        let body = vec![0x5e_u8; MIN_CHUNK_LEN as usize + 40];
        let (header, on_disk, _) = sealed(&body);

        for index in 0..on_disk.len() {
            let mut damaged = on_disk.clone();
            if let Some(byte) = damaged.get_mut(index) {
                *byte ^= 0x01;
            }
            assert!(
                opened(&header, &damaged).is_err(),
                "a change to byte {index} of the body went unnoticed"
            );
        }
    }

    #[test]
    fn two_chunks_swapped_fail() {
        // The attack that a per-chunk tag alone does not stop. Both chunks are intact and
        // both were sealed by us; the only thing wrong is where they are. The index inside
        // the associated data and the nonce recomputed from the index both catch it.
        let body: Vec<u8> = (0..(MIN_CHUNK_LEN as usize * 2 + 8))
            .map(|index| u8::try_from(index % 251).unwrap_or(0))
            .collect();
        let (header, on_disk, _) = sealed(&body);

        let width = MIN_CHUNK_LEN as usize + TAG_LEN;
        let mut swapped = Vec::with_capacity(on_disk.len());
        swapped.extend_from_slice(on_disk.get(width..width * 2).unwrap());
        swapped.extend_from_slice(on_disk.get(..width).unwrap());
        swapped.extend_from_slice(on_disk.get(width * 2..).unwrap());

        assert!(matches!(opened(&header, &swapped), Err(CryptoError::Open)));
    }

    #[test]
    fn a_chunk_repeated_fails() {
        let body: Vec<u8> = vec![0x21_u8; MIN_CHUNK_LEN as usize * 2 + 8];
        let (header, on_disk, _) = sealed(&body);

        let width = MIN_CHUNK_LEN as usize + TAG_LEN;
        let mut repeated = Vec::with_capacity(on_disk.len() + width);
        repeated.extend_from_slice(on_disk.get(..width).unwrap());
        repeated.extend_from_slice(on_disk.get(..width).unwrap());
        repeated.extend_from_slice(on_disk.get(width..).unwrap());

        assert!(matches!(opened(&header, &repeated), Err(CryptoError::Open)));
    }

    #[test]
    fn a_header_edited_after_the_fact_fails() {
        // Why the whole header is in the associated data of every chunk rather than only of
        // the first. Somebody lowering the Argon2id parameters to make guessing cheap has
        // to break every tag in the file, not one of them.
        let body = vec![0x9a_u8; MIN_CHUNK_LEN as usize * 2 + 4];
        let (header, on_disk, _) = sealed(&body);

        let mut edited_bytes = header.to_bytes();
        if let Some(target) = edited_bytes.get_mut(28..32) {
            target.copy_from_slice(&crate::kdf::MIN_MEMORY_KIB.to_le_bytes());
        }
        let edited = BackupHeader::parse(&edited_bytes).unwrap();

        assert!(matches!(opened(&edited, &on_disk), Err(CryptoError::Open)));
        // And the failure is not only in the first chunk: the last one is bound to the same
        // header, which is what makes truncating to the first chunk no help either.
        assert!(matches!(
            opened(
                &edited,
                on_disk.get(..MIN_CHUNK_LEN as usize + TAG_LEN).unwrap()
            ),
            Err(CryptoError::Open)
        ));
    }

    #[test]
    fn another_key_does_not_open_it() {
        let body = b"contenido";
        let (header, on_disk, _) = sealed(body);

        let other = DataKey::from_bytes([0x5b; KEY_LEN]);
        let mut out = Zeroizing::new(Vec::new());
        let mut opener = ChunkOpener::new(&other, &header);

        let refused = opener
            .push(&on_disk, &mut out)
            .and_then(|()| opener.finish(&mut out).map(|_count| ()));
        assert!(matches!(refused, Err(CryptoError::Open)));
    }

    #[test]
    fn a_file_with_no_room_for_a_tag_is_damaged_rather_than_refused_as_a_key_problem() {
        // The distinction that is safe to make. Fewer than sixteen bytes cannot be a chunk
        // under any key at all, so saying so reveals nothing, and it is the honest answer
        // for the likeliest damaged file there is.
        let (header, _) = small_chunks();

        for len in 0..TAG_LEN {
            assert!(
                matches!(opened(&header, &vec![0_u8; len]), Err(CryptoError::Damaged)),
                "a body of {len} bytes was not reported as damaged"
            );
        }
    }
}

#[cfg(test)]
mod properties {
    use proptest::prelude::{ProptestConfig, any, prop_assert, prop_assert_eq};
    use proptest::proptest;
    use zeroize::Zeroizing;

    use super::{ChunkOpener, ChunkSealer};
    use crate::backup::header::{BackupHeader, MIN_CHUNK_LEN};
    use crate::kdf::{Argon2Params, SALT_LEN};
    use crate::keys::{DataKey, KEY_LEN};
    use crate::stream::ChunkNonces;

    /// Far fewer under Miri, which interprets every instruction.
    const CASES: u32 = if cfg!(miri) { 2 } else { 64 };

    /// A header with the smallest legal chunk size.
    fn small_chunks() -> (BackupHeader, ChunkNonces) {
        let nonces = ChunkNonces::generate().unwrap();
        let mut bytes =
            BackupHeader::new([0x33; SALT_LEN], Argon2Params::DEFAULT, *nonces.base()).to_bytes();
        if let Some(target) = bytes.get_mut(56..60) {
            target.copy_from_slice(&MIN_CHUNK_LEN.to_le_bytes());
        }

        (BackupHeader::parse(&bytes).unwrap(), nonces)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(CASES))]

        /// Any body of any length comes back exactly as it went in.
        ///
        /// The lengths are drawn around the chunk boundary on purpose: nothing, one byte,
        /// one short of a chunk, exactly a chunk, one past it, and on into several.
        #[test]
        fn any_body_round_trips(
            key_bytes in any::<[u8; KEY_LEN]>(),
            body in proptest::collection::vec(
                any::<u8>(),
                0..(MIN_CHUNK_LEN as usize * 2 + 3),
            ),
        ) {
            let key = DataKey::from_bytes(key_bytes);
            let (header, nonces) = small_chunks();

            let mut on_disk = Vec::new();
            let mut sealer = ChunkSealer::new(&key, &header, nonces);
            sealer.push(&body, &mut on_disk).unwrap();
            let written = sealer.finish(&mut on_disk).unwrap();

            let mut out = Zeroizing::new(Vec::new());
            let mut opener = ChunkOpener::new(&key, &header);
            opener.push(&on_disk, &mut out).unwrap();
            let read = opener.finish(&mut out).unwrap();

            prop_assert_eq!(out.as_slice(), body.as_slice());
            prop_assert_eq!(written, read);
        }

        /// No prefix of a file shorter than the whole of it ever opens.
        #[test]
        fn no_short_prefix_ever_opens(
            key_bytes in any::<[u8; KEY_LEN]>(),
            body in proptest::collection::vec(any::<u8>(), 1..(MIN_CHUNK_LEN as usize + 64)),
            cut in 0_usize..512,
        ) {
            let key = DataKey::from_bytes(key_bytes);
            let (header, nonces) = small_chunks();

            let mut on_disk = Vec::new();
            let mut sealer = ChunkSealer::new(&key, &header, nonces);
            sealer.push(&body, &mut on_disk).unwrap();
            sealer.finish(&mut on_disk).unwrap();

            let keep = cut.min(on_disk.len().saturating_sub(1));
            let shortened = on_disk.get(..keep).unwrap_or_default();

            let mut out = Zeroizing::new(Vec::new());
            let mut opener = ChunkOpener::new(&key, &header);
            let outcome = opener
                .push(shortened, &mut out)
                .and_then(|()| opener.finish(&mut out).map(|_count| ()));

            prop_assert!(outcome.is_err(), "a prefix of {} bytes opened", keep);
        }
    }
}
