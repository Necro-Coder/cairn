//! A plain hash, for the things that need one and are not secrets.
//!
//! Not a keyed hash and not a signature. What this is for is telling whether two byte strings
//! are the same when both of them are already visible: whether the migration numbered four in
//! the ledger is the migration numbered four this build carries. Anybody who can edit the
//! database can recompute it, and that is written down here rather than left for somebody to
//! discover, because a checksum that looks like tamper detection and is not is worse than no
//! checksum at all.
//!
//! It lives in this crate for the same reason the cipher does: so that there is one place in
//! the workspace that names a hash function, and changing it is one decision made once.

use sha2::{Digest as _, Sha256};

/// Length of a digest, in bytes.
pub const DIGEST_LEN: usize = 32;

/// The SHA-256 of some bytes.
///
/// Deterministic and unkeyed. Two calls with the same input always agree, on every machine and
/// in every version of this program, which is the only property the callers need.
#[must_use]
pub fn digest(bytes: &[u8]) -> [u8; DIGEST_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);

    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::{DIGEST_LEN, digest};

    #[test]
    fn the_empty_input_has_the_published_digest() {
        // The published SHA-256 of nothing. A vector rather than a round trip, so that swapping
        // the hash function for another one fails here instead of passing quietly.
        let expected = [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
            0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
            0x78, 0x52, 0xb8, 0x55,
        ];

        assert_eq!(digest(b""), expected);
    }

    #[test]
    fn the_same_input_always_gives_the_same_answer() {
        assert_eq!(digest(b"a migration"), digest(b"a migration"));
    }

    #[test]
    fn one_changed_byte_changes_the_answer() {
        assert_ne!(digest(b"a migration"), digest(b"a migratioo"));
    }

    #[test]
    fn a_digest_is_always_the_declared_length() {
        assert_eq!(digest(b"anything").len(), DIGEST_LEN);
    }
}
