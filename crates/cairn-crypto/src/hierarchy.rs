//! Which key hangs off which, and the domain separation that keeps them apart.
//!
//! ```text
//! master password
//!   └─ Argon2id(password, salt, parameters from the header) ──► key encryption key
//!         └─ HKDF-SHA512(kek, "cairn/v1/wrap") ──► wrap key, which seals the data key
//!
//! data key (thirty-two random bytes, wrapped in the header) ──► encrypts every record
//!   └─ HKDF-SHA512(dek, "cairn/v1/db")     ──► the raw key SQLCipher is given
//!   └─ HKDF-SHA512(dek, "cairn/v1/export") ──► encrypts a backup
//!   └─ HKDF-SHA512(dek, "cairn/v1/sync")   ──► the pre-shared key for synchronisation
//! ```
//!
//! Everything except the wrap key hangs off the data key rather than off the key encryption
//! key, and that is the decision this module exists to make concrete. The key encryption key
//! changes whenever the password or the Argon2id parameters change. The data key never
//! changes. So with the database key derived from the data key, changing the master password
//! rewrites a hundred and sixty-eight bytes of header and touches nothing else; with it
//! derived from the key encryption key, the same operation would have to re-encrypt the
//! whole database file, which is the most dangerous thing this application could ever do.
//!
//! What that costs is small and worth saying out loud: anybody holding the data key can
//! already read every record, and the two keys live in the memory of the same process at the
//! same time, so deriving the database key from the one rather than the other does not widen
//! what a successful attack yields.
//!
//! The purposes are a closed enumeration rather than a string a caller passes in. A typo in
//! a string produces a key that works, is wrong, and collides with nothing until the day it
//! collides with something. The exact bytes of each one are frozen by a test, because
//! changing them silently would leave every vault that already exists unopenable.

use crate::keys::{DataKey, DatabaseKey, KEY_LEN, Kek, SyncKey};
use hkdf::Hkdf;
use sha2::Sha512;

/// What a subkey is for.
///
/// Closed on purpose. A new purpose means a new variant here and a new frozen value in the
/// test below, which is exactly the amount of friction a new key deserves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Wraps the data key inside the vault header. Derived from the key encryption key.
    Wrap,
    /// The raw key handed to SQLCipher for the whole database file.
    Database,
    /// Encrypts an exported backup.
    Export,
    /// The pre-shared key for the synchronisation handshake.
    Sync,
}

impl Purpose {
    /// The exact bytes that go into the `info` argument of HKDF.
    ///
    /// Versioned. Without `v1` in the string, a future version of this scheme deriving a key
    /// for the same purpose would derive the same key, which is the kind of collision nobody
    /// sees coming because nothing about it looks wrong.
    #[must_use]
    pub const fn info(self) -> &'static [u8] {
        match self {
            Self::Wrap => b"cairn/v1/wrap",
            Self::Database => b"cairn/v1/db",
            Self::Export => b"cairn/v1/export",
            Self::Sync => b"cairn/v1/sync",
        }
    }
}

/// HKDF refuses an output longer than two hundred and fifty-five times the hash length,
/// which for SHA-512 is over eight kilobytes. Stated here as a compile time fact so that the
/// only failure mode of the derivation below is provably unreachable rather than merely
/// unlikely.
const _: () = assert!(KEY_LEN <= 255 * 64);

/// Derives thirty-two bytes for one purpose from one key.
///
/// No salt. HKDF without one falls back to a string of zeroes as the extract salt, which is
/// what the specification describes and is correct here: the input is already a uniformly
/// random key rather than a password, so the extract step has nothing left to do and the
/// separation comes entirely from the `info` argument.
#[expect(
    clippy::expect_used,
    reason = "INVARIANT: expand only refuses an output longer than 255 hash lengths, and the const assertion above proves the one length ever requested is far below that; returning a zeroed key instead would be a silent failure that works perfectly and is wrong"
)]
fn derive(input: &[u8; KEY_LEN], purpose: Purpose) -> [u8; KEY_LEN] {
    let mut derived = [0_u8; KEY_LEN];

    Hkdf::<Sha512>::new(None, input)
        .expand(purpose.info(), &mut derived)
        .expect("the requested length is a compile time constant far below the HKDF limit");

    derived
}

/// The key that seals the data key inside the vault header.
///
/// The one subkey that hangs off the key encryption key, because wrapping the data key is
/// the only thing the key encryption key is for.
#[must_use]
pub fn wrap_key(kek: &Kek) -> DataKey {
    DataKey::from_derived(derive(kek.expose(), Purpose::Wrap))
}

/// The raw key SQLCipher is given for the whole database file.
#[must_use]
pub fn database_key(dek: &DataKey) -> DatabaseKey {
    DatabaseKey::from_derived(derive(dek.expose(), Purpose::Database))
}

/// The key an exported backup is encrypted with.
#[must_use]
pub fn export_key(dek: &DataKey) -> DataKey {
    DataKey::from_derived(derive(dek.expose(), Purpose::Export))
}

/// The pre-shared key for the synchronisation handshake.
#[must_use]
pub fn sync_key(dek: &DataKey) -> SyncKey {
    SyncKey::from_derived(derive(dek.expose(), Purpose::Sync))
}

#[cfg(test)]
mod tests {
    use subtle::ConstantTimeEq as _;

    use super::{Purpose, database_key, derive, export_key, sync_key, wrap_key};
    use crate::keys::{DataKey, KEY_LEN, Kek};

    const KEY_BYTES: [u8; KEY_LEN] = [0x5a; KEY_LEN];

    #[test]
    fn the_info_strings_are_frozen() {
        // These bytes are part of the file format in the same way the header layout is.
        // Change one and every vault that already exists derives a different database key
        // and stops opening. The test is here so that the change cannot be quiet.
        assert_eq!(Purpose::Wrap.info(), b"cairn/v1/wrap");
        assert_eq!(Purpose::Database.info(), b"cairn/v1/db");
        assert_eq!(Purpose::Export.info(), b"cairn/v1/export");
        assert_eq!(Purpose::Sync.info(), b"cairn/v1/sync");
    }

    #[test]
    fn every_purpose_yields_a_different_key() {
        let purposes = [
            Purpose::Wrap,
            Purpose::Database,
            Purpose::Export,
            Purpose::Sync,
        ];

        let derived: Vec<[u8; KEY_LEN]> = purposes
            .iter()
            .map(|purpose| derive(&KEY_BYTES, *purpose))
            .collect();

        for (left, one) in derived.iter().enumerate() {
            assert_ne!(one, &[0_u8; KEY_LEN], "purpose {left} derived nothing");
            for (right, other) in derived.iter().enumerate() {
                if left != right {
                    assert_ne!(one, other, "purposes {left} and {right} collided");
                }
            }
        }
    }

    #[test]
    fn the_derivation_is_deterministic() {
        // Not a nicety: if this ever stopped holding, a vault would open once and never
        // again, and the failure would look exactly like a wrong password.
        assert_eq!(
            derive(&KEY_BYTES, Purpose::Database),
            derive(&KEY_BYTES, Purpose::Database)
        );
    }

    #[test]
    fn a_different_input_key_yields_a_different_subkey() {
        let mut other = KEY_BYTES;
        other[0] ^= 0x01;
        assert_ne!(
            derive(&KEY_BYTES, Purpose::Database),
            derive(&other, Purpose::Database)
        );
    }

    #[test]
    fn the_wrap_key_hangs_off_the_key_encryption_key() {
        let kek = Kek::from_bytes(KEY_BYTES);
        let expected = derive(&KEY_BYTES, Purpose::Wrap);
        assert!(bool::from(
            wrap_key(&kek).ct_eq(&DataKey::from_bytes(expected))
        ));
    }

    #[test]
    fn the_other_three_hang_off_the_data_key() {
        let dek = DataKey::from_bytes(KEY_BYTES);

        assert_eq!(
            database_key(&dek).expose(),
            &derive(&KEY_BYTES, Purpose::Database)
        );
        assert_eq!(sync_key(&dek).expose(), &derive(&KEY_BYTES, Purpose::Sync));

        let expected_export = DataKey::from_bytes(derive(&KEY_BYTES, Purpose::Export));
        assert!(bool::from(export_key(&dek).ct_eq(&expected_export)));
    }

    #[test]
    fn a_derived_key_reveals_nothing_when_printed() {
        let dek = DataKey::from_bytes(KEY_BYTES);
        assert_eq!(
            format!("{:?}", database_key(&dek)),
            "DatabaseKey([REDACTED])"
        );
        assert_eq!(format!("{:?}", sync_key(&dek)), "SyncKey([REDACTED])");
    }

    /// What this derivation answers for one known input, recorded so it cannot move.
    ///
    /// Not a published vector: RFC 5869 gives its examples for SHA-256 and SHA-1, and there
    /// is none for SHA-512 with these arguments. So the value was produced here and checked
    /// against an unrelated HKDF-SHA512 implementation before being written down, which is
    /// what makes it evidence rather than a photograph of whatever this code happened to do.
    ///
    /// What it catches is a change of hash, of salt handling or of the way `info` is passed,
    /// any one of which would leave every existing vault unopenable while every other test in
    /// this file still passed.
    #[test]
    fn the_derivation_matches_a_recorded_answer() {
        let derived = derive(&[0x00; KEY_LEN], Purpose::Database);
        assert_eq!(
            hex(&derived),
            "e71184e0250be90da95f68bd9457280a20116ee57eb60bf02e0cb3acfd610e20"
        );
    }

    fn hex(bytes: &[u8]) -> String {
        use core::fmt::Write as _;

        bytes.iter().fold(String::new(), |mut text, byte| {
            write!(text, "{byte:02x}").unwrap();
            text
        })
    }
}
