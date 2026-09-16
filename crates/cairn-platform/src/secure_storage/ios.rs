//! The iOS store, which does not exist yet and says so on every call.
//!
//! The Keychain with the secure enclave behind it is what this will be, and writing it needs
//! a device to test on. That device arrives in a later phase. What exists now is a type that
//! compiles on iOS and refuses every operation with
//! [`SecureStorageError::NotImplementedOnThisPlatform`].
//!
//! A stub that refuses is worth having rather than nothing at all, for two reasons. The
//! compile job that runs on every pull request builds this file, so the interface cannot
//! drift into something iOS could never implement without anybody noticing. And the refusal
//! is a distinct error rather than a silent absence, so the caller that asks for it is told
//! to stop offering the feature instead of quietly getting `None` and assuming the store was
//! simply empty.

use zeroize::Zeroizing;

use super::{Capabilities, SecureStorage, SecureStorageError, check_label};

/// The Keychain, once there is one.
#[derive(Debug, Clone, Copy, Default)]
pub struct IosKeychain;

impl IosKeychain {
    /// The store for this device.
    #[must_use]
    pub const fn for_this_device() -> Self {
        Self
    }
}

impl SecureStorage for IosKeychain {
    fn store(&self, label: &str, _secret: &[u8]) -> Result<(), SecureStorageError> {
        // The label is still checked. The rule about what a label may be is a property of the
        // interface rather than of any one store, and a caller that only ever runs here would
        // otherwise find out about it the first time it ran somewhere else.
        check_label(label)?;
        Err(SecureStorageError::NotImplementedOnThisPlatform)
    }

    fn retrieve(&self, label: &str) -> Result<Option<Zeroizing<Vec<u8>>>, SecureStorageError> {
        check_label(label)?;
        Err(SecureStorageError::NotImplementedOnThisPlatform)
    }

    fn delete(&self, label: &str) -> Result<(), SecureStorageError> {
        check_label(label)?;
        Err(SecureStorageError::NotImplementedOnThisPlatform)
    }

    fn capabilities(&self) -> Capabilities {
        // Not hardware backed, because there is no store at all. Reporting the enclave that
        // the eventual implementation will use would be describing a plan as a fact.
        Capabilities::new(false, None)
    }
}

#[cfg(test)]
mod tests {
    use super::{IosKeychain, SecureStorage, SecureStorageError};

    #[test]
    fn every_operation_says_this_platform_has_no_store() {
        let keychain = IosKeychain::for_this_device();

        assert!(matches!(
            keychain.store("una-nota", b"nada"),
            Err(SecureStorageError::NotImplementedOnThisPlatform)
        ));
        assert!(matches!(
            keychain.retrieve("una-nota"),
            Err(SecureStorageError::NotImplementedOnThisPlatform)
        ));
        assert!(matches!(
            keychain.delete("una-nota"),
            Err(SecureStorageError::NotImplementedOnThisPlatform)
        ));
    }

    #[test]
    fn a_bad_label_is_still_a_bad_label() {
        assert!(matches!(
            IosKeychain::for_this_device().store("../escapada", b"nada"),
            Err(SecureStorageError::LabelRejected { .. })
        ));
    }

    #[test]
    fn nothing_that_opens_the_vault_may_be_kept_here_either() {
        let capabilities = IosKeychain::for_this_device().capabilities();

        assert!(!capabilities.hardware_backed());
        assert!(!capabilities.may_hold_vault_material());
    }
}
