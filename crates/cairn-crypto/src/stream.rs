//! Nonces for a stream of chunks, where writing the nonce down would be the vulnerability.
//!
//! Everywhere else in this project a nonce is twenty-four random bytes that travel beside
//! the ciphertext they belong to. A backup cannot do that, and the reason is worth reading
//! rather than skipping: a nonce stored in front of a chunk is a nonce that whoever holds
//! the file gets to choose. Swap two chunks and their stored nonces travel with them, so
//! both tags still verify and the file decrypts into a different file.
//!
//! So the nonces are not stored. Each one is [`NONCE_BASE_LEN`] bytes of base, read once
//! from the operating system and written into the header, followed by the chunk's index as
//! eight big endian bytes. Reordering a chunk now computes the wrong nonce for it, and the
//! index is inside the associated data as well, so the same edit fails twice. It also saves
//! twenty-four bytes per chunk, which is the least interesting thing about it.
//!
//! That leaves one obligation. A counter is only safe while it never goes backwards and
//! never restarts under the same base, and this is a file format, so "never" has to survive
//! a mistake made two years from now. [`ChunkNonces`] is the answer: it owns its base, it
//! cannot be cloned, it cannot be copied, it cannot be reset, it hands out each nonce by
//! value exactly once, and it refuses rather than wrapping when the counter runs out.

use core::fmt;

use crate::error::CryptoError;
use crate::nonce::{FreshNonce, NONCE_LEN};
use crate::random;

/// Length of the random part of a chunk nonce, in bytes.
///
/// Sixteen, which leaves eight for the counter and adds up to the twenty-four
/// XChaCha20-Poly1305 wants. A hundred and twenty-eight random bits per file means two
/// backups sharing a base is not a thing that happens.
pub const NONCE_BASE_LEN: usize = 16;

/// How many chunks one base may cover.
///
/// The whole range of the counter. At the chunk size this format uses it is orders of
/// magnitude beyond any file the limits allow, so the exhaustion path exists to be
/// impossible rather than to be hit; it is written and tested anyway, because the
/// alternative to refusing is wrapping, and wrapping means two chunks under one nonce.
pub const MAX_CHUNKS: u64 = u64::MAX;

/// Builds the nonce for one chunk.
///
/// Free function rather than a method, because both sides need it and only one of them is
/// allowed to encrypt. The reader recomputes nonces for chunks it is about to open, and
/// giving it a [`ChunkNonces`] would be handing the read path permission to encrypt.
#[must_use]
pub fn chunk_nonce(base: &[u8; NONCE_BASE_LEN], index: u64) -> [u8; NONCE_LEN] {
    let mut nonce = [0_u8; NONCE_LEN];
    let (front, back) = nonce.split_at_mut(NONCE_BASE_LEN);

    front.copy_from_slice(base);
    back.copy_from_slice(&index.to_be_bytes());

    nonce
}

/// The nonces of one backup, handed out in order and never twice.
///
/// Intentionally missing `Clone`, `Copy` and `Default`. Each of those would be a way to
/// have two of these over the same base, which is two counters starting from zero, which is
/// every nonce used twice. There is no way to read the counter back to a lower value and no
/// way to build one at an arbitrary position.
pub struct ChunkNonces {
    base: [u8; NONCE_BASE_LEN],
    issued: u64,
}

impl fmt::Debug for ChunkNonces {
    /// Says how far the stream has got and nothing about the base.
    ///
    /// A base is not a secret, and it still does not belong in a log. The one question
    /// nobody should have to answer out of a log file is whether the same base was used
    /// twice, and a base that is never written down is a base that cannot be searched for.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChunkNonces")
            .field("base", &"[REDACTED]")
            .field("issued", &self.issued)
            .finish()
    }
}

impl ChunkNonces {
    /// Starts a new stream over a base read from the operating system.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Entropy`] if the operating system refuses. There is no
    /// fallback, here least of all: a base somebody can predict is a base somebody can
    /// arrange to see twice.
    pub fn generate() -> Result<Self, CryptoError> {
        let mut base = [0_u8; NONCE_BASE_LEN];
        random::fill(&mut base)?;

        Ok(Self { base, issued: 0 })
    }

    /// The base, so it can be written into the header of the file being produced.
    #[must_use]
    pub fn base(&self) -> &[u8; NONCE_BASE_LEN] {
        &self.base
    }

    /// How many nonces have been handed out, which is the index the next one will carry.
    #[must_use]
    pub fn issued(&self) -> u64 {
        self.issued
    }

    /// The next nonce, consumed from this stream.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::ChunkCountExhausted`] when the counter has no room left.
    /// Refusing rather than wrapping is the point: a wrap would reuse the nonce of chunk
    /// zero, and losing the file is a far better outcome than sealing two chunks under one
    /// nonce and calling it a backup.
    pub fn issue(&mut self) -> Result<FreshNonce, CryptoError> {
        let index = self.issued;
        let Some(following) = index.checked_add(1) else {
            return Err(CryptoError::ChunkCountExhausted);
        };

        self.issued = following;

        Ok(FreshNonce::from_counter(chunk_nonce(&self.base, index)))
    }
}

#[cfg(test)]
mod tests {
    use super::{ChunkNonces, MAX_CHUNKS, NONCE_BASE_LEN, chunk_nonce};
    use crate::error::CryptoError;
    use crate::nonce::NONCE_LEN;

    const BASE: [u8; NONCE_BASE_LEN] = [0x5a; NONCE_BASE_LEN];

    #[test]
    fn a_nonce_is_the_base_followed_by_the_index() {
        // The layout is part of the published file format: somebody writing an independent
        // decryptor reads this order out of the documentation and has to get the same bytes.
        let nonce = chunk_nonce(&BASE, 0x0102_0304_0506_0708);

        assert_eq!(nonce.len(), NONCE_LEN);
        assert_eq!(nonce.get(..NONCE_BASE_LEN), Some(BASE.as_slice()));
        assert_eq!(
            nonce.get(NONCE_BASE_LEN..),
            Some([1, 2, 3, 4, 5, 6, 7, 8].as_slice())
        );
    }

    #[test]
    fn the_counter_is_big_endian() {
        // Not a preference. Big endian means chunk one is the byte string after chunk zero,
        // which is what makes a documented format readable by somebody comparing two nonces
        // by eye. Little endian would work and would be impossible to check by hand.
        assert_eq!(chunk_nonce(&BASE, 1).last(), Some(&1_u8));
    }

    #[test]
    fn two_indices_never_share_a_nonce() {
        let mut seen = std::collections::HashSet::new();
        for index in 0..4096_u64 {
            assert!(
                seen.insert(chunk_nonce(&BASE, index)),
                "index {index} produced a nonce that had already been used"
            );
        }
    }

    #[test]
    fn two_bases_never_share_a_nonce() {
        let mut other = BASE;
        other[0] ^= 0x01;

        for index in 0..64_u64 {
            assert_ne!(chunk_nonce(&BASE, index), chunk_nonce(&other, index));
        }
    }

    #[test]
    fn a_stream_hands_out_consecutive_nonces() {
        let mut nonces = ChunkNonces::generate().unwrap();
        let base = *nonces.base();

        assert_eq!(nonces.issued(), 0);
        for index in 0..8_u64 {
            let produced = nonces.issue().unwrap();
            assert_eq!(produced.into_bytes(), chunk_nonce(&base, index));
            assert_eq!(nonces.issued(), index + 1);
        }
    }

    #[test]
    fn two_streams_do_not_share_a_base() {
        // Sixteen random bytes make a collision a non-event, and this is still worth a test:
        // what it actually catches is a base that stopped being random, for instance because
        // somebody replaced the generator with a constant while chasing a flaky test.
        let first = ChunkNonces::generate().unwrap();
        let second = ChunkNonces::generate().unwrap();

        assert_ne!(first.base(), second.base());
    }

    #[test]
    fn a_stream_with_no_room_left_refuses_instead_of_wrapping() {
        // Reaching this by counting would take longer than the universe has had, so the
        // state is built directly. What is under test is the branch, and the branch is the
        // difference between a refused export and two chunks sealed under one nonce.
        let mut nonces = ChunkNonces::generate().unwrap();
        nonces.issued = MAX_CHUNKS;

        assert!(matches!(
            nonces.issue(),
            Err(CryptoError::ChunkCountExhausted)
        ));
        assert_eq!(
            nonces.issued(),
            MAX_CHUNKS,
            "a refused nonce still moved the counter"
        );
    }

    #[test]
    fn the_last_nonce_before_exhaustion_is_still_handed_out() {
        // The other side of the boundary. A check written as "at or above" rather than
        // "above" would throw away the last legal chunk and pass every other test here.
        let mut nonces = ChunkNonces::generate().unwrap();
        nonces.issued = MAX_CHUNKS - 1;

        assert!(nonces.issue().is_ok());
        assert!(nonces.issue().is_err());
    }

    #[test]
    fn a_stream_says_nothing_about_its_base_when_printed() {
        // A nonce base is not a secret, and it is still not something that belongs in a log:
        // the one question nobody should have to answer from a log file is whether the same
        // base appeared twice.
        let nonces = ChunkNonces::generate().unwrap();
        let printed = format!("{nonces:?}");

        assert!(
            !printed.contains(&format!("{:?}", nonces.base())),
            "the base reached a debug representation"
        );
    }
}
