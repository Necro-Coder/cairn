//! The first sixty-four bytes of a backup file: everything needed to start decrypting it,
//! and nothing that says whose it is.
//!
//! ```text
//! offset  bytes  field
//!      0      8  magic, "CAIRNBAK"
//!      8      2  format version, currently 1
//!     10      2  compression algorithm: 1 is zstd
//!     12     16  salt for this file's own Argon2id
//!     28      4  Argon2id memory cost, in kibibytes
//!     32      4  Argon2id passes
//!     36      4  Argon2id lanes
//!     40     16  nonce base
//!     56      4  chunk size, in bytes
//!     60      4  reserved, must be zero
//! ```
//!
//! Three properties of this layout are load-bearing.
//!
//! It is a fixed length with no length fields in it, so the parser has no loop and no
//! arithmetic on a number the file supplied. A backup is the one artefact of this project
//! that leaves the machine, so its parser is the piece of code most likely to be handed
//! something hostile, and the cheapest way to make a parser safe is to give it nothing to
//! decide.
//!
//! What is not in it matters as much as what is. No installation identifier, no record
//! count, no module names, no internal date. A file that says which machine made it is a
//! file that says something about its owner to anybody who finds it, and the only thing
//! that buys is a friendlier error message when somebody opens the wrong backup. That
//! trade is refused on purpose, and the consequence — importing another vault's backup
//! reports a wrong password rather than a wrong vault — is written into the user
//! documentation so it does not look like a fault.
//!
//! The Argon2id parameters are checked against the same floor and ceiling the vault header
//! uses, and checked before a single byte is allocated. A header claiming four gibibytes
//! would otherwise kill the process on memory before anything had been validated, which is
//! a twelve byte edit away from a denial of service (CWE-770).
//!
//! The reserved field must be zero. That is what lets a version two add a field without
//! changing the length: a version one writer emits zeroes there, a version one reader
//! refuses anything else, so no version one file can ever have meant something in those
//! bytes.

use crate::error::CryptoError;
use crate::kdf::{Argon2Params, SALT_LEN};
use crate::stream::NONCE_BASE_LEN;

/// Total size of the header, in bytes.
pub const BACKUP_HEADER_LEN: usize = 64;

/// What every backup of this format starts with.
///
/// Eight bytes that are not valid anything else, so a file that is not one of ours is
/// refused on the first comparison. It is also the only thing visible when somebody opens a
/// backup in a text editor, which is the point of the manual check that does exactly that.
pub const BACKUP_MAGIC: [u8; 8] = *b"CAIRNBAK";

/// The format version this build writes.
pub const BACKUP_FORMAT_VERSION: u16 = 1;

/// The oldest format version this build can still read.
///
/// The same number today, because there is only one version. It is written as its own
/// constant rather than as a literal so that the conversion path added for version two has
/// somewhere to say what it supports, and so the test that exercises that path today has
/// something to assert against.
pub const OLDEST_READABLE_FORMAT_VERSION: u16 = 1;

/// Size of one plaintext chunk, in bytes.
///
/// Sixty-four kibibytes. Large enough that the sixteen byte tag on each one is noise, small
/// enough that the memory a reader needs is a constant rather than a function of the file.
/// It is written into the header rather than assumed, so a future version can change it
/// without every reader having to be changed with it, and it is bounded on the way back in
/// because it decides the size of an allocation.
pub const CHUNK_LEN: u32 = 64 * 1024;

/// Smallest chunk size a file may declare, in bytes.
///
/// A file claiming sixteen byte chunks is a file with one tag per sixteen bytes of content
/// and hundreds of thousands of Poly1305 verifications per mebibyte. Refused rather than
/// obeyed.
pub const MIN_CHUNK_LEN: u32 = 4 * 1024;

/// Largest chunk size a file may declare, in bytes.
///
/// Four mebibytes. This is the ceiling on what a reader is asked to hold for one chunk, so
/// it is a bound on memory rather than a matter of taste (CWE-770).
pub const MAX_CHUNK_LEN: u32 = 4 * 1024 * 1024;

/// How the body was compressed before it was encrypted.
///
/// A closed enumeration rather than an integer passed around, so that a reader cannot carry
/// on with a value it does not understand. Nothing decompresses in this crate; the number
/// travels through it so that the header stays the one authenticated description of the
/// file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Compression {
    /// The body is a zstd stream.
    Zstd,
}

impl Compression {
    /// What goes in the header.
    #[must_use]
    pub const fn as_code(self) -> u16 {
        match self {
            Self::Zstd => 1,
        }
    }

    /// Reads the code back, refusing anything this build does not know.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::BackupCompression`] for any other value. Guessing at a
    /// compression somebody else invented is how a reader ends up handing a decompressor
    /// bytes that are not a stream.
    pub const fn from_code(code: u16) -> Result<Self, CryptoError> {
        match code {
            1 => Ok(Self::Zstd),
            _ => Err(CryptoError::BackupCompression { found: code }),
        }
    }
}

/// Where each field starts. Written out rather than computed, so the table above and the
/// code can be compared by eye.
mod offset {
    pub(super) const MAGIC: usize = 0;
    pub(super) const FORMAT_VERSION: usize = 8;
    pub(super) const COMPRESSION: usize = 10;
    pub(super) const KDF_SALT: usize = 12;
    pub(super) const ARGON2_MEMORY: usize = 28;
    pub(super) const ARGON2_PASSES: usize = 32;
    pub(super) const ARGON2_LANES: usize = 36;
    pub(super) const NONCE_BASE: usize = 40;
    pub(super) const CHUNK_LEN: usize = 56;
    pub(super) const RESERVED: usize = 60;
}

/// A parsed backup header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupHeader {
    format_version: u16,
    compression: Compression,
    kdf_salt: [u8; SALT_LEN],
    params: Argon2Params,
    nonce_base: [u8; NONCE_BASE_LEN],
    chunk_len: u32,
}

impl BackupHeader {
    /// The header of a backup that is being written now.
    #[must_use]
    pub const fn new(
        kdf_salt: [u8; SALT_LEN],
        params: Argon2Params,
        nonce_base: [u8; NONCE_BASE_LEN],
    ) -> Self {
        Self {
            format_version: BACKUP_FORMAT_VERSION,
            compression: Compression::Zstd,
            kdf_salt,
            params,
            nonce_base,
            chunk_len: CHUNK_LEN,
        }
    }

    /// Reads a header from exactly [`BACKUP_HEADER_LEN`] bytes.
    ///
    /// Total: every input of every length produces either a header or an error, and none of
    /// them panics or allocates on a number the file chose. A fuzzing target asserts that
    /// over inputs nobody thought of.
    ///
    /// # Errors
    ///
    /// - [`CryptoError::NotABackup`] for a wrong length or a wrong magic. The two are one
    ///   error because they answer one question, and because a caller told which of the two
    ///   it was learns nothing it can use.
    /// - [`CryptoError::BackupVersion`] for a version this build cannot read, whether older
    ///   than the conversion path covers or newer than this build knows about.
    /// - [`CryptoError::BackupCompression`] for an algorithm this build does not carry.
    /// - [`CryptoError::ParamOutOfRange`] for an Argon2id parameter or a chunk size outside
    ///   the allowed range, checked before anything is reserved.
    /// - [`CryptoError::BackupReserved`] if the reserved field is not zero.
    pub fn parse(bytes: &[u8]) -> Result<Self, CryptoError> {
        let bytes: &[u8; BACKUP_HEADER_LEN] = bytes
            .try_into()
            .map_err(|_wrong_length| CryptoError::NotABackup)?;

        if read::<{ offset::MAGIC }, 8>(bytes) != BACKUP_MAGIC {
            return Err(CryptoError::NotABackup);
        }

        let format_version = read_u16::<{ offset::FORMAT_VERSION }>(bytes);
        if format_version < OLDEST_READABLE_FORMAT_VERSION || format_version > BACKUP_FORMAT_VERSION
        {
            return Err(CryptoError::BackupVersion {
                found: format_version,
            });
        }

        let compression = Compression::from_code(read_u16::<{ offset::COMPRESSION }>(bytes))?;

        // Before anything is allocated and before the password is asked for, because the
        // point of the ceiling is to refuse a header that would exhaust memory rather than
        // to discover afterwards that it did.
        let params = Argon2Params::new(
            read_u32::<{ offset::ARGON2_MEMORY }>(bytes),
            read_u32::<{ offset::ARGON2_PASSES }>(bytes),
            read_u32::<{ offset::ARGON2_LANES }>(bytes),
        )?;

        let chunk_len = read_u32::<{ offset::CHUNK_LEN }>(bytes);
        if !(MIN_CHUNK_LEN..=MAX_CHUNK_LEN).contains(&chunk_len) {
            return Err(CryptoError::ParamOutOfRange {
                field: "chunk_len",
                value: chunk_len,
                min: MIN_CHUNK_LEN,
                max: MAX_CHUNK_LEN,
            });
        }

        if read_u32::<{ offset::RESERVED }>(bytes) != 0 {
            return Err(CryptoError::BackupReserved);
        }

        Ok(Self {
            format_version,
            compression,
            kdf_salt: read::<{ offset::KDF_SALT }, SALT_LEN>(bytes),
            params,
            nonce_base: read::<{ offset::NONCE_BASE }, NONCE_BASE_LEN>(bytes),
            chunk_len,
        })
    }

    /// Writes the header out.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; BACKUP_HEADER_LEN] {
        let mut bytes = [0_u8; BACKUP_HEADER_LEN];

        write::<{ offset::MAGIC }, 8>(&mut bytes, &BACKUP_MAGIC);
        write::<{ offset::FORMAT_VERSION }, 2>(&mut bytes, &self.format_version.to_le_bytes());
        write::<{ offset::COMPRESSION }, 2>(&mut bytes, &self.compression.as_code().to_le_bytes());
        write::<{ offset::KDF_SALT }, SALT_LEN>(&mut bytes, &self.kdf_salt);
        write::<{ offset::ARGON2_MEMORY }, 4>(&mut bytes, &self.params.memory_kib().to_le_bytes());
        write::<{ offset::ARGON2_PASSES }, 4>(&mut bytes, &self.params.passes().to_le_bytes());
        write::<{ offset::ARGON2_LANES }, 4>(&mut bytes, &self.params.lanes().to_le_bytes());
        write::<{ offset::NONCE_BASE }, NONCE_BASE_LEN>(&mut bytes, &self.nonce_base);
        write::<{ offset::CHUNK_LEN }, 4>(&mut bytes, &self.chunk_len.to_le_bytes());
        // The reserved field stays zero. Written out rather than left to the initialiser, so
        // that this function and the layout above list the same fields in the same order.
        write::<{ offset::RESERVED }, 4>(&mut bytes, &0_u32.to_le_bytes());

        bytes
    }

    /// The format version the file declares.
    #[must_use]
    pub const fn format_version(&self) -> u16 {
        self.format_version
    }

    /// How the body was compressed.
    #[must_use]
    pub const fn compression(&self) -> Compression {
        self.compression
    }

    /// The salt this file's own Argon2id runs over.
    #[must_use]
    pub const fn kdf_salt(&self) -> &[u8; SALT_LEN] {
        &self.kdf_salt
    }

    /// The Argon2id parameters this file's own derivation runs at.
    #[must_use]
    pub const fn params(&self) -> Argon2Params {
        self.params
    }

    /// The base every chunk nonce in this file is built from.
    #[must_use]
    pub const fn nonce_base(&self) -> &[u8; NONCE_BASE_LEN] {
        &self.nonce_base
    }

    /// How many bytes of plaintext one chunk of this file holds.
    #[must_use]
    pub const fn chunk_len(&self) -> u32 {
        self.chunk_len
    }
}

/// Reads a fixed slice at a fixed offset.
///
/// Both are compile time constants, so the slice either exists for every input of this
/// length or the code does not build. The fallback is unreachable and is written rather
/// than indexed, because indexing is denied across this workspace for exactly the case
/// where somebody later makes one of these a variable.
fn read<const AT: usize, const LEN: usize>(bytes: &[u8; BACKUP_HEADER_LEN]) -> [u8; LEN] {
    const { assert!(AT + LEN <= BACKUP_HEADER_LEN) };

    bytes
        .get(AT..AT + LEN)
        .and_then(|slice| slice.try_into().ok())
        .unwrap_or([0_u8; LEN])
}

/// Reads two little endian bytes at a fixed offset.
fn read_u16<const AT: usize>(bytes: &[u8; BACKUP_HEADER_LEN]) -> u16 {
    u16::from_le_bytes(read::<AT, 2>(bytes))
}

/// Reads four little endian bytes at a fixed offset.
fn read_u32<const AT: usize>(bytes: &[u8; BACKUP_HEADER_LEN]) -> u32 {
    u32::from_le_bytes(read::<AT, 4>(bytes))
}

/// Writes a fixed slice at a fixed offset.
fn write<const AT: usize, const LEN: usize>(
    bytes: &mut [u8; BACKUP_HEADER_LEN],
    value: &[u8; LEN],
) {
    const { assert!(AT + LEN <= BACKUP_HEADER_LEN) };

    if let Some(target) = bytes.get_mut(AT..AT + LEN) {
        target.copy_from_slice(value);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BACKUP_FORMAT_VERSION, BACKUP_HEADER_LEN, BACKUP_MAGIC, BackupHeader, CHUNK_LEN,
        Compression, MAX_CHUNK_LEN, MIN_CHUNK_LEN, offset,
    };
    use crate::error::CryptoError;
    use crate::kdf::{Argon2Params, MAX_MEMORY_KIB, MIN_MEMORY_KIB, MIN_PASSES, SALT_LEN};
    use crate::stream::NONCE_BASE_LEN;

    const SALT: [u8; SALT_LEN] = [0x11; SALT_LEN];
    const BASE: [u8; NONCE_BASE_LEN] = [0x22; NONCE_BASE_LEN];

    fn a_header() -> BackupHeader {
        BackupHeader::new(SALT, Argon2Params::DEFAULT, BASE)
    }

    /// Edits one little endian `u32` of a serialised header in place.
    fn with_u32(bytes: &mut [u8; BACKUP_HEADER_LEN], at: usize, value: u32) {
        if let Some(target) = bytes.get_mut(at..at + 4) {
            target.copy_from_slice(&value.to_le_bytes());
        }
    }

    /// Edits one little endian `u16` of a serialised header in place.
    fn with_u16(bytes: &mut [u8; BACKUP_HEADER_LEN], at: usize, value: u16) {
        if let Some(target) = bytes.get_mut(at..at + 2) {
            target.copy_from_slice(&value.to_le_bytes());
        }
    }

    #[test]
    fn the_header_is_the_length_the_format_says() {
        // Frozen rather than derived. The number is published byte by byte in the
        // architecture documentation, and somebody writing an independent decryptor reads
        // sixty-four there and seeks that far before looking for the first chunk.
        assert_eq!(BACKUP_HEADER_LEN, 64);
        assert_eq!(a_header().to_bytes().len(), BACKUP_HEADER_LEN);
    }

    #[test]
    fn the_magic_is_the_eight_bytes_the_format_says() {
        assert_eq!(&BACKUP_MAGIC, b"CAIRNBAK");
        assert_eq!(
            a_header().to_bytes().get(..8),
            Some(BACKUP_MAGIC.as_slice())
        );
    }

    #[test]
    fn the_magic_differs_from_the_one_the_vault_header_uses() {
        // Two files of two formats live in the same directory. If they began the same way,
        // handing one to the other's parser would get past the first check and fail later,
        // somewhere that reports it as damage rather than as the wrong kind of file.
        assert_ne!(BACKUP_MAGIC, crate::MAGIC);
    }

    #[test]
    fn a_header_survives_a_trip_through_bytes() {
        let header = a_header();
        let restored = BackupHeader::parse(&header.to_bytes()).unwrap();

        assert_eq!(restored, header);
        assert_eq!(restored.kdf_salt(), &SALT);
        assert_eq!(restored.nonce_base(), &BASE);
        assert_eq!(restored.params(), Argon2Params::DEFAULT);
        assert_eq!(restored.chunk_len(), CHUNK_LEN);
        assert_eq!(restored.compression(), Compression::Zstd);
        assert_eq!(restored.format_version(), BACKUP_FORMAT_VERSION);
    }

    #[test]
    fn a_length_that_is_not_the_header_length_is_refused() {
        // Every length from nothing to one byte past, because the likeliest damaged file is
        // a truncated one and a parser that indexes before checking is how that becomes a
        // panic instead of a message.
        for len in 0..=(BACKUP_HEADER_LEN + 1) {
            if len == BACKUP_HEADER_LEN {
                continue;
            }
            assert!(
                matches!(
                    BackupHeader::parse(&vec![0_u8; len]),
                    Err(CryptoError::NotABackup)
                ),
                "a header of {len} bytes was not refused"
            );
        }
    }

    #[test]
    fn something_that_is_not_a_backup_is_refused_before_anything_else() {
        let mut bytes = a_header().to_bytes();
        // A wrong magic and, at the same time, a version and parameters that are nonsense.
        // The magic has to be what answers, because it is the check that costs nothing.
        bytes[0] = b'X';
        with_u16(&mut bytes, offset::FORMAT_VERSION, 9_999);

        assert!(matches!(
            BackupHeader::parse(&bytes),
            Err(CryptoError::NotABackup)
        ));
    }

    #[test]
    fn a_version_from_the_future_is_refused_rather_than_guessed_at() {
        let mut bytes = a_header().to_bytes();
        with_u16(
            &mut bytes,
            offset::FORMAT_VERSION,
            BACKUP_FORMAT_VERSION + 1,
        );

        assert!(matches!(
            BackupHeader::parse(&bytes),
            Err(CryptoError::BackupVersion { found }) if found == BACKUP_FORMAT_VERSION + 1
        ));
    }

    #[test]
    fn version_zero_is_refused() {
        let mut bytes = a_header().to_bytes();
        with_u16(&mut bytes, offset::FORMAT_VERSION, 0);

        assert!(matches!(
            BackupHeader::parse(&bytes),
            Err(CryptoError::BackupVersion { found: 0 })
        ));
    }

    #[test]
    fn a_compression_this_build_does_not_carry_is_refused() {
        let mut bytes = a_header().to_bytes();
        with_u16(&mut bytes, offset::COMPRESSION, 7);

        assert!(matches!(
            BackupHeader::parse(&bytes),
            Err(CryptoError::BackupCompression { found: 7 })
        ));
    }

    #[test]
    fn parameters_below_the_floor_are_refused() {
        let mut bytes = a_header().to_bytes();
        with_u32(&mut bytes, offset::ARGON2_MEMORY, MIN_MEMORY_KIB - 1);

        assert!(matches!(
            BackupHeader::parse(&bytes),
            Err(CryptoError::ParamOutOfRange {
                field: "argon2_m_kib",
                ..
            })
        ));
    }

    #[test]
    fn parameters_above_the_ceiling_are_refused_before_anything_is_reserved() {
        // The denial of service the ceiling exists for. Four gibibytes of declared memory
        // has to be a refusal rather than an allocation, and the refusal has to come out of
        // the parser rather than out of the derivation, because by then the reservation has
        // already been attempted.
        let mut bytes = a_header().to_bytes();
        with_u32(&mut bytes, offset::ARGON2_MEMORY, 4 * 1024 * 1024);

        assert!(matches!(
            BackupHeader::parse(&bytes),
            Err(CryptoError::ParamOutOfRange { field: "argon2_m_kib", max, .. })
                if max == MAX_MEMORY_KIB
        ));
    }

    #[test]
    fn passes_below_the_floor_are_refused() {
        let mut bytes = a_header().to_bytes();
        with_u32(&mut bytes, offset::ARGON2_PASSES, MIN_PASSES - 1);

        assert!(matches!(
            BackupHeader::parse(&bytes),
            Err(CryptoError::ParamOutOfRange {
                field: "argon2_t",
                ..
            })
        ));
    }

    #[test]
    fn a_chunk_size_outside_the_range_is_refused() {
        for outside in [0, MIN_CHUNK_LEN - 1, MAX_CHUNK_LEN + 1, u32::MAX] {
            let mut bytes = a_header().to_bytes();
            with_u32(&mut bytes, offset::CHUNK_LEN, outside);

            assert!(
                matches!(
                    BackupHeader::parse(&bytes),
                    Err(CryptoError::ParamOutOfRange {
                        field: "chunk_len",
                        ..
                    })
                ),
                "a chunk size of {outside} was accepted"
            );
        }
    }

    #[test]
    fn a_chunk_size_at_either_end_of_the_range_is_accepted() {
        // The other half of the boundary. Without it, a check written with the wrong
        // comparison refuses a legal file and passes every test above.
        for inside in [MIN_CHUNK_LEN, MAX_CHUNK_LEN] {
            let mut bytes = a_header().to_bytes();
            with_u32(&mut bytes, offset::CHUNK_LEN, inside);

            assert_eq!(BackupHeader::parse(&bytes).unwrap().chunk_len(), inside);
        }
    }

    #[test]
    fn a_reserved_field_that_is_not_zero_is_refused() {
        // What keeps a version two possible. If a version one reader tolerated rubbish
        // here, a version one file could already be carrying something in those bytes, and
        // version two would have to guess whether it meant anything.
        let mut bytes = a_header().to_bytes();
        with_u32(&mut bytes, offset::RESERVED, 1);

        assert!(matches!(
            BackupHeader::parse(&bytes),
            Err(CryptoError::BackupReserved)
        ));
    }

    #[test]
    fn a_written_header_leaves_the_reserved_field_zero() {
        assert_eq!(
            a_header().to_bytes().get(60..64),
            Some([0_u8; 4].as_slice())
        );
    }

    #[test]
    fn changing_any_single_byte_is_either_noticed_or_changes_the_header() {
        // Totality, checked byte by byte. Every edit of every byte either produces an error
        // or produces a header that differs from the original, and none of them panics.
        // What this rules out is a byte nobody reads, which is a byte a version two would
        // later discover somebody had been writing rubbish into.
        let header = a_header();
        let original = header.to_bytes();

        for index in 0..BACKUP_HEADER_LEN {
            let mut damaged = original;
            if let Some(byte) = damaged.get_mut(index) {
                *byte ^= 0xff;
            }

            match BackupHeader::parse(&damaged) {
                Err(_refused) => {}
                Ok(parsed) => assert_ne!(
                    parsed, header,
                    "byte {index} was changed and the header came back identical"
                ),
            }
        }
    }
}

#[cfg(test)]
mod properties {
    use proptest::prelude::{ProptestConfig, any, prop_assert, prop_assert_eq};
    use proptest::proptest;

    use super::{BACKUP_HEADER_LEN, BackupHeader, MAX_CHUNK_LEN, MIN_CHUNK_LEN};
    use crate::kdf::{
        Argon2Params, MAX_LANES, MAX_MEMORY_KIB, MAX_PASSES, MIN_MEMORY_KIB, MIN_PASSES, SALT_LEN,
    };
    use crate::stream::NONCE_BASE_LEN;

    /// Far fewer under Miri, which interprets every instruction.
    const CASES: u32 = if cfg!(miri) { 2 } else { 512 };

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(CASES))]

        /// Any header this build can produce survives a trip through bytes unchanged.
        #[test]
        fn a_header_round_trips(
            salt in any::<[u8; SALT_LEN]>(),
            base in any::<[u8; NONCE_BASE_LEN]>(),
            memory_kib in MIN_MEMORY_KIB..=MAX_MEMORY_KIB,
            passes in MIN_PASSES..=MAX_PASSES,
            lanes in 1_u32..=MAX_LANES,
        ) {
            let params = Argon2Params::new(memory_kib, passes, lanes).unwrap();
            let header = BackupHeader::new(salt, params, base);

            prop_assert_eq!(BackupHeader::parse(&header.to_bytes()).unwrap(), header);
        }

        /// Any sixty-four bytes at all either parse or are refused, and never panic.
        ///
        /// The header is the piece of this project most likely to be handed something
        /// hostile, because a backup is the only artefact that leaves the machine. The
        /// fuzzing target covers the same ground for longer; this keeps it in the suite
        /// that runs on every change.
        #[test]
        fn arbitrary_bytes_never_panic(bytes in any::<[u8; BACKUP_HEADER_LEN]>()) {
            if let Ok(parsed) = BackupHeader::parse(&bytes) {
                prop_assert_eq!(parsed.to_bytes(), bytes);
                prop_assert!(
                    (MIN_CHUNK_LEN..=MAX_CHUNK_LEN).contains(&parsed.chunk_len())
                );
            }
        }

        /// Any length that is not the header length is refused rather than read.
        #[test]
        fn any_other_length_is_refused(
            bytes in proptest::collection::vec(any::<u8>(), 0..256)
        ) {
            if bytes.len() != BACKUP_HEADER_LEN {
                prop_assert!(BackupHeader::parse(&bytes).is_err());
            }
        }
    }
}
