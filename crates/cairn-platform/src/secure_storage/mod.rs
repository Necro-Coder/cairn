//! Where a secret can be kept between one run of the application and the next, and how much
//! that is worth on the machine it is kept on.
//!
//! The interesting part of this module is not the three operations. It is
//! [`Capabilities`], which says what the store on this machine actually does, and
//! [`Capabilities::may_hold_vault_material`], which turns that into an answer to the only
//! question a caller ever needs to ask: may something that opens the vault be kept here?
//!
//! Today, on every platform this compiles for, the answer is no. Windows has a store and it
//! is an ordinary file; iOS has no store at all yet. Keeping the wrapped key in an ordinary
//! file would replace the master password with whatever protects that file, which is the
//! account somebody is already logged into. The whole design rests on the vault being
//! unreadable to somebody holding the disk, and a file beside it that opens it would end
//! that.
//!
//! So the store exists for the things that are merely inconvenient to lose and useless to
//! steal, and the capability flag is the gate. When a hardware backed store arrives, the
//! flag turns true in one place and the callers that were refused start being allowed.

use std::fmt;
use std::io;

use zeroize::Zeroizing;

// Both implementations compile everywhere, and neither is behind a `cfg`. Nothing in
// either of them touches a platform interface: one is a file, the other refuses. Gating
// them by target would mean each is compiled, linted and tested on exactly one machine, and
// the one that is never built on the machine somebody is working on is the one that rots.
// Which store a running program uses is a decision for the layer that builds one, and that
// decision is a `cfg` of two lines rather than a wall around a thousand.
mod ios;
mod windows;

pub use ios::IosKeychain;
pub use windows::LocalFileStore;

/// Longest label a stored secret may be named with.
pub const MAX_LABEL_LEN: usize = 64;

/// Which sensor, if any, guards the store.
///
/// Named individually rather than as a boolean, because what stands behind each of them
/// differs: one is a credential in a platform module, another is a key in a secure enclave,
/// and a threat model that treats them as the same thing is a threat model that has stopped
/// describing anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Biometry {
    /// Windows Hello, backed by the platform trusted module.
    WindowsHello,
    /// Face ID, backed by the secure enclave.
    FaceId,
    /// Touch ID, backed by the secure enclave.
    TouchId,
}

/// What the store on this machine is actually able to promise.
///
/// Read by the layer that decides what may be kept, never by the layer that keeps it. A
/// store that reported optimistically would be worse than no store: the caller would hand it
/// something on the strength of a promise nothing keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    hardware_backed: bool,
    biometry: Option<Biometry>,
}

impl Capabilities {
    /// Declares what a store can do.
    #[must_use]
    pub const fn new(hardware_backed: bool, biometry: Option<Biometry>) -> Self {
        Self {
            hardware_backed,
            biometry,
        }
    }

    /// Whether the secret is held by hardware that will not give it up to a copy of the disk.
    #[must_use]
    pub const fn hardware_backed(&self) -> bool {
        self.hardware_backed
    }

    /// Which sensor guards it, if any.
    #[must_use]
    pub const fn biometry(&self) -> Option<Biometry> {
        self.biometry
    }

    /// Whether anything that opens the vault may be kept in this store.
    ///
    /// The one question worth asking, answered in one place. It is deliberately stricter
    /// than "is there a sensor": a fingerprint in front of a file that the file system will
    /// hand to anybody with the disk protects the interface, not the data.
    #[must_use]
    pub const fn may_hold_vault_material(&self) -> bool {
        self.hardware_backed
    }
}

/// Why a secure storage operation did not happen.
///
/// [`Self::NotImplementedOnThisPlatform`] and [`Self::AuthenticationFailed`] are the two that
/// must never be collapsed into one. The first means this build cannot do the thing at all,
/// and the caller should stop offering it. The second means the person is standing there and
/// the sensor said no, and the caller should offer it again. Treating them alike would
/// either hide a missing implementation behind a retry loop or turn a shaky fingerprint into
/// a feature that disappears.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecureStorageError {
    /// This build has no store for this platform.
    #[error("this platform has no secure storage in this build")]
    NotImplementedOnThisPlatform,

    /// The person was asked to prove who they were, and did not.
    #[error("the secure storage did not accept the authentication")]
    AuthenticationFailed,

    /// The label was not a name this store accepts.
    ///
    /// Labels become part of a path on at least one platform, so they are checked against a
    /// closed set of characters rather than escaped. Anything that is not in the set is
    /// refused with this, and the reason it is refused is not negotiable per platform.
    #[error("{reason}")]
    LabelRejected {
        /// What was wrong with it, in a form that fits the sentence.
        reason: &'static str,
    },

    /// The store could not be read or written.
    #[error("the secure storage could not be {operation}")]
    Io {
        /// What was being attempted, in a form that fits the sentence above.
        operation: &'static str,
        /// The underlying failure, kept so the cause survives to the log.
        #[source]
        cause: io::Error,
    },
}

/// Somewhere a secret can be kept between one run and the next.
///
/// Deliberately small. Every platform store in existence can do these four things, and
/// anything richer would be an interface shaped by whichever one was written first.
pub trait SecureStorage {
    /// Keeps `secret` under `label`, replacing whatever was there.
    ///
    /// # Errors
    ///
    /// Returns [`SecureStorageError::NotImplementedOnThisPlatform`] where there is no store,
    /// [`SecureStorageError::LabelRejected`] for a label outside the accepted set, and
    /// [`SecureStorageError::Io`] if the store cannot be written.
    fn store(&self, label: &str, secret: &[u8]) -> Result<(), SecureStorageError>;

    /// Reads back what is kept under `label`, or `None` if nothing is.
    ///
    /// # Errors
    ///
    /// The same as [`Self::store`], plus [`SecureStorageError::AuthenticationFailed`] where
    /// the store asks the person to prove who they are.
    fn retrieve(&self, label: &str) -> Result<Option<Zeroizing<Vec<u8>>>, SecureStorageError>;

    /// Removes what is kept under `label`. Removing what is not there is not an error.
    ///
    /// # Errors
    ///
    /// The same as [`Self::store`].
    fn delete(&self, label: &str) -> Result<(), SecureStorageError>;

    /// What this store is able to promise.
    fn capabilities(&self) -> Capabilities;
}

/// Checks that a label is a name every platform store will accept.
///
/// Lower case letters, digits and hyphens, and nothing else. A label becomes a file name on
/// at least one platform, so the choice is between a closed set and escaping; a closed set
/// is the one that cannot be got wrong later by somebody adding a platform.
///
/// # Errors
///
/// Returns [`SecureStorageError::LabelRejected`] for an empty label, one longer than
/// [`MAX_LABEL_LEN`], or one containing anything outside that set.
pub fn check_label(label: &str) -> Result<(), SecureStorageError> {
    if label.is_empty() {
        return Err(SecureStorageError::LabelRejected {
            reason: "a secure storage label cannot be empty",
        });
    }
    if label.len() > MAX_LABEL_LEN {
        return Err(SecureStorageError::LabelRejected {
            reason: "the secure storage label is longer than this store accepts",
        });
    }
    if !label
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(SecureStorageError::LabelRejected {
            reason: "a secure storage label may hold only lower case letters, digits and hyphens",
        });
    }

    Ok(())
}

impl fmt::Display for Capabilities {
    /// Reads the way the diagnostics screen shows it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let backing = if self.hardware_backed {
            "hardware backed"
        } else {
            "not hardware backed"
        };
        match self.biometry {
            Some(sensor) => write!(formatter, "{backing}, guarded by {sensor:?}"),
            None => write!(formatter, "{backing}, no biometric sensor"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Biometry, Capabilities, MAX_LABEL_LEN, SecureStorageError, check_label};

    /// What somebody must not be able to write by accident.
    ///
    /// The two variants mean opposite things to the caller: one says stop offering this, the
    /// other says offer it again. A match that folded them into the same arm would compile
    /// and would be wrong, so the distinction is written down as a test rather than left to
    /// whoever reads the enum next.
    #[test]
    fn a_missing_implementation_and_a_refused_fingerprint_are_not_the_same_answer() {
        fn should_offer_again(error: &SecureStorageError) -> bool {
            match error {
                SecureStorageError::AuthenticationFailed => true,
                // No catch all arm, and none is needed: `non_exhaustive` only forces one on
                // callers outside this crate. A variant added later stops this compiling,
                // which is the point. The next person to extend the enum is asked the
                // question rather than given a default.
                SecureStorageError::NotImplementedOnThisPlatform
                | SecureStorageError::LabelRejected { .. }
                | SecureStorageError::Io { .. } => false,
            }
        }

        assert!(should_offer_again(
            &SecureStorageError::AuthenticationFailed
        ));
        assert!(!should_offer_again(
            &SecureStorageError::NotImplementedOnThisPlatform
        ));
    }

    #[test]
    fn the_two_variants_say_different_things() {
        assert_ne!(
            SecureStorageError::NotImplementedOnThisPlatform.to_string(),
            SecureStorageError::AuthenticationFailed.to_string()
        );
    }

    #[test]
    fn nothing_that_is_not_hardware_backed_may_hold_what_opens_the_vault() {
        // A sensor in front of a file is not the same as a key in hardware, and this is the
        // line the whole module exists to draw.
        let file = Capabilities::new(false, None);
        let file_behind_a_sensor = Capabilities::new(false, Some(Biometry::WindowsHello));
        let hardware = Capabilities::new(true, Some(Biometry::WindowsHello));

        assert!(!file.may_hold_vault_material());
        assert!(!file_behind_a_sensor.may_hold_vault_material());
        assert!(hardware.may_hold_vault_material());
    }

    #[test]
    fn the_capabilities_report_what_they_were_given() {
        // Read back field by field rather than only through the gate above, because a gate
        // that always says no would pass every other test in this file while the two things
        // it is computed from had stopped meaning anything.
        let hardware = Capabilities::new(true, Some(Biometry::TouchId));

        assert!(hardware.hardware_backed());
        assert_eq!(hardware.biometry(), Some(Biometry::TouchId));

        let neither = Capabilities::new(false, None);

        assert!(!neither.hardware_backed());
        assert_eq!(neither.biometry(), None);
    }

    #[test]
    fn the_capabilities_read_as_a_sentence() {
        assert_eq!(
            Capabilities::new(false, None).to_string(),
            "not hardware backed, no biometric sensor"
        );
        assert_eq!(
            Capabilities::new(true, Some(Biometry::FaceId)).to_string(),
            "hardware backed, guarded by FaceId"
        );
    }

    #[test]
    fn an_ordinary_label_is_accepted() {
        assert!(check_label("wrapped-dek").is_ok());
        assert!(check_label("a").is_ok());
        assert!(check_label(&"a".repeat(MAX_LABEL_LEN)).is_ok());
    }

    #[test]
    fn a_label_that_could_walk_out_of_the_directory_is_refused() {
        // The reason the set is closed rather than escaped. Each of these becomes a path on
        // at least one platform, and each of them leaves the directory it was meant for.
        for label in [
            "../vault.header",
            "..\\vault.header",
            "a/b",
            "a\\b",
            "..",
            ".",
            "a:b",
            "a\0b",
        ] {
            assert!(
                check_label(label).is_err(),
                "a label that leaves the directory was accepted: {label}"
            );
        }
    }

    #[test]
    fn an_empty_label_and_an_overlong_one_are_refused() {
        assert!(check_label("").is_err());
        assert!(check_label(&"a".repeat(MAX_LABEL_LEN + 1)).is_err());
    }

    #[test]
    fn upper_case_is_refused_rather_than_folded() {
        // Folding would mean two labels that look different naming one secret, and on a case
        // insensitive file system that is a silent overwrite.
        assert!(check_label("Wrapped-Dek").is_err());
    }
}
