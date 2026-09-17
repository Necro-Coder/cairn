//! Cryptography for Cairn: key derivation, the key hierarchy, authenticated encryption
//! and the framing used by encrypted exports.
//!
//! This crate performs no input or output and knows nothing about the database, the window
//! or the transport. Everything it does is a pure function of its arguments, which is what
//! makes it testable with property tests and checkable under Miri.
//!
//! Three rules hold everywhere inside it, and each one is enforced by something other than
//! good intentions.
//!
//! Encryption happens in one place. [`seal`] and [`open`] are the only functions that
//! touch the cipher, and a test walks the source tree to prove that no other crate so much
//! as names the cipher library.
//!
//! A nonce is used once. [`FreshNonce`] can only be read from the operating system and is
//! consumed by value, so a second use is a compile error rather than a review comment, and
//! a test in `tests/ui` asserts that the compiler really does refuse it.
//!
//! Nothing prints a secret. Every key type writes `[REDACTED]` from its own `Debug`, so a
//! struct that happens to hold a key cannot leak it through a derived one.
#![forbid(unsafe_code)]

mod aad;
mod aead;
mod error;
mod header;
mod hierarchy;
mod kdf;
mod keys;
mod nonce;
mod random;
mod vault;

pub use aad::{Aad, ID_LEN, MAX_NAME_LEN};
pub use aead::{MAX_PLAINTEXT_LEN, Sealed, TAG_LEN, open, seal};
pub use error::CryptoError;
pub use header::{
    AUTHENTICATED_PREFIX_LEN, FORMAT_VERSION, HEADER_LEN, MAGIC, VaultHeader, WRAPPED_DEK_LEN,
};
pub use hierarchy::{Purpose, database_key, export_key, sync_key, wrap_key};
pub use kdf::{
    Argon2Params, MAX_LANES, MAX_MEMORY_KIB, MAX_PASSES, MAX_PASSWORD_BYTES, MIN_MEMORY_KIB,
    MIN_PASSES, SALT_LEN, derive_kek,
};
pub use keys::{DataKey, DatabaseKey, KEY_LEN, Kek, SyncKey};
pub use nonce::{FreshNonce, NONCE_LEN};
pub use random::fill as fill_random;
pub use vault::{UnlockedVault, change_kdf_params, change_password, create, unlock};

/// The version of this crate, taken from its manifest at compile time.
///
/// Every crate in the workspace inherits the same version from `[workspace.package]`, so
/// this is also the version of the application as a whole.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    /// Guards against a malformed version in the manifest. The value reaches the
    /// diagnostics screen and the release artefacts, so a typo here is not cosmetic.
    #[test]
    fn version_is_three_numeric_components() {
        let components: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(
            components.len(),
            3,
            "version must be major.minor.patch, got {VERSION}"
        );
        for component in components {
            assert!(
                component.parse::<u32>().is_ok(),
                "non numeric component in version {VERSION}"
            );
        }
    }
}
