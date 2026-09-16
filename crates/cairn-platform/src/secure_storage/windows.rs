//! The Windows store, which is a file, and says so.
//!
//! Windows Hello backed by the platform trusted module is what this will eventually be, and
//! it is not what this is. What is here is a file under the local application data
//! directory, and [`LocalFileStore::capabilities`] reports it as not hardware backed, which
//! means [`super::Capabilities::may_hold_vault_material`] refuses it anything that opens the
//! vault.
//!
//! That refusal is the feature. A file in the user profile is protected by whatever protects
//! the user profile, which is the account somebody is already logged into, and on a stolen
//! disk that is nothing at all. Writing the wrapped key here would hand anybody with the
//! drive a copy of the vault, and no amount of asking for a fingerprint first would change
//! it, because the fingerprint guards the program and the attacker is not running the
//! program.
//!
//! Encrypting the file with the data protection interface of the operating system was
//! considered and is not done. It is keyed to the account rather than to hardware, so it
//! moves the problem to the account without changing the answer, and shipping it would make
//! the capability flag arguable. An unarguable no is worth more here than a qualified one.

use std::fs;
use std::io;
use std::path::PathBuf;

use zeroize::Zeroizing;

use super::{Capabilities, SecureStorage, SecureStorageError, check_label};

/// Secrets kept as files in a directory of their own.
#[derive(Debug, Clone)]
pub struct LocalFileStore {
    directory: PathBuf,
}

impl LocalFileStore {
    /// A store in the local application data directory of the current account.
    ///
    /// # Errors
    ///
    /// Returns [`SecureStorageError::Io`] if the environment does not say where that
    /// directory is, which on Windows means the process was started without it.
    pub fn for_this_account() -> Result<Self, SecureStorageError> {
        let base = std::env::var_os("LOCALAPPDATA").ok_or_else(|| SecureStorageError::Io {
            operation: "located, because the account has no local application data directory",
            cause: io::Error::from(io::ErrorKind::NotFound),
        })?;

        Ok(Self::in_directory(
            PathBuf::from(base).join("cairn").join("secure-storage"),
        ))
    }

    /// A store in a directory given explicitly.
    ///
    /// Exists so that a test can work somewhere of its own rather than in the real profile of
    /// whoever is running it.
    #[must_use]
    pub fn in_directory(directory: PathBuf) -> Self {
        Self { directory }
    }

    /// Where a label lives, once the label has been checked.
    ///
    /// Takes the checked label rather than checking here, so that there is exactly one place
    /// the rule lives and no path is ever built from a string nobody looked at.
    fn path_of(&self, checked_label: &str) -> PathBuf {
        self.directory.join(format!("{checked_label}.secret"))
    }
}

impl SecureStorage for LocalFileStore {
    fn store(&self, label: &str, secret: &[u8]) -> Result<(), SecureStorageError> {
        check_label(label)?;

        fs::create_dir_all(&self.directory).map_err(|cause| SecureStorageError::Io {
            operation: "written, because its directory could not be created",
            cause,
        })?;

        // Written whole and renamed over, for the same reason the vault header is: a half
        // written secret read back later is worse than one that was never written.
        let temporary = self.path_of(label).with_extension("secret.new");
        fs::write(&temporary, secret).map_err(|cause| SecureStorageError::Io {
            operation: "written to a temporary file",
            cause,
        })?;
        fs::rename(&temporary, self.path_of(label)).map_err(|cause| SecureStorageError::Io {
            operation: "renamed over the old one",
            cause,
        })
    }

    fn retrieve(&self, label: &str) -> Result<Option<Zeroizing<Vec<u8>>>, SecureStorageError> {
        check_label(label)?;

        match fs::read(self.path_of(label)) {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(cause) => Err(SecureStorageError::Io {
                operation: "read",
                cause,
            }),
        }
    }

    fn delete(&self, label: &str) -> Result<(), SecureStorageError> {
        check_label(label)?;

        match fs::remove_file(self.path_of(label)) {
            Ok(()) => Ok(()),
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(cause) => Err(SecureStorageError::Io {
                operation: "removed",
                cause,
            }),
        }
    }

    fn capabilities(&self) -> Capabilities {
        // Both halves false, and both of them honest. See the module documentation for why
        // the data protection interface does not make the first one true.
        Capabilities::new(false, None)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::{LocalFileStore, SecureStorage, SecureStorageError};

    /// Nothing here is key material. These are three sentences somebody made up, kept in a
    /// scratch directory that is removed when the test ends.
    const NOT_A_SECRET: &[u8] = b"un apunte cualquiera, inventado para la prueba";

    struct Scratch {
        directory: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);

            let directory = std::env::temp_dir().join(format!(
                "cairn-secure-storage-{name}-{}-{unique}",
                std::process::id()
            ));
            Self { directory }
        }

        fn store(&self) -> LocalFileStore {
            LocalFileStore::in_directory(self.directory.clone())
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn what_goes_in_comes_back_out() {
        let scratch = Scratch::new("round-trip");
        let store = scratch.store();

        store.store("una-nota", NOT_A_SECRET).unwrap();
        let read = store.retrieve("una-nota").unwrap().unwrap();

        assert_eq!(read.as_slice(), NOT_A_SECRET);
    }

    #[test]
    fn nothing_stored_reads_back_as_nothing_rather_than_as_a_failure() {
        let scratch = Scratch::new("absent");
        assert!(scratch.store().retrieve("una-nota").unwrap().is_none());
    }

    #[test]
    fn storing_twice_keeps_the_second_one() {
        let scratch = Scratch::new("overwrite");
        let store = scratch.store();

        store.store("una-nota", b"lo primero").unwrap();
        store.store("una-nota", b"lo segundo").unwrap();

        assert_eq!(
            store.retrieve("una-nota").unwrap().unwrap().as_slice(),
            b"lo segundo"
        );
    }

    #[test]
    fn deleting_removes_it_and_deleting_again_is_not_an_error() {
        let scratch = Scratch::new("delete");
        let store = scratch.store();

        store.store("una-nota", NOT_A_SECRET).unwrap();
        store.delete("una-nota").unwrap();

        assert!(store.retrieve("una-nota").unwrap().is_none());
        assert!(store.delete("una-nota").is_ok());
    }

    #[test]
    fn two_labels_are_two_secrets() {
        let scratch = Scratch::new("two-labels");
        let store = scratch.store();

        store.store("primera", b"uno").unwrap();
        store.store("segunda", b"dos").unwrap();

        assert_eq!(
            store.retrieve("primera").unwrap().unwrap().as_slice(),
            b"uno"
        );
        assert_eq!(
            store.retrieve("segunda").unwrap().unwrap().as_slice(),
            b"dos"
        );
    }

    #[test]
    fn a_label_that_leaves_the_directory_never_reaches_the_file_system() {
        // Checked before a path is built rather than after, so there is no moment at which a
        // path made from an unchecked string exists.
        let scratch = Scratch::new("traversal");
        let store = scratch.store();

        for label in ["../escapada", "..\\escapada", "sub/carpeta"] {
            assert!(matches!(
                store.store(label, NOT_A_SECRET),
                Err(SecureStorageError::LabelRejected { .. })
            ));
            assert!(matches!(
                store.retrieve(label),
                Err(SecureStorageError::LabelRejected { .. })
            ));
            assert!(matches!(
                store.delete(label),
                Err(SecureStorageError::LabelRejected { .. })
            ));
        }

        assert!(
            !scratch.directory.exists(),
            "a refused label created a file"
        );
    }

    #[test]
    fn storing_leaves_no_temporary_file_behind() {
        let scratch = Scratch::new("no-temporary");
        let store = scratch.store();

        store.store("una-nota", NOT_A_SECRET).unwrap();

        let leftovers: Vec<String> = fs::read_dir(&scratch.directory)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();

        assert_eq!(leftovers, vec!["una-nota.secret".to_owned()]);
    }

    #[test]
    fn a_read_that_fails_for_another_reason_is_not_reported_as_nothing_stored() {
        // The difference between "there is no secret" and "the secret could not be read" is
        // the difference between offering to create one and saying something is wrong. A
        // directory where the file should be produces the second, and the guard that tells
        // them apart is what this pins.
        let scratch = Scratch::new("read-fails");
        let store = scratch.store();

        fs::create_dir_all(scratch.directory.join("una-nota.secret")).unwrap();

        assert!(
            matches!(
                store.retrieve("una-nota"),
                Err(SecureStorageError::Io { .. })
            ),
            "a read that failed for another reason was reported as nothing stored"
        );
    }

    #[test]
    fn a_removal_that_fails_for_another_reason_is_not_reported_as_success() {
        // Same distinction on the way out. Reporting success for a secret that is still
        // there would leave the caller believing it had been destroyed.
        let scratch = Scratch::new("delete-fails");
        let store = scratch.store();

        fs::create_dir_all(scratch.directory.join("una-nota.secret")).unwrap();

        assert!(
            matches!(store.delete("una-nota"), Err(SecureStorageError::Io { .. })),
            "a removal that failed was reported as success"
        );
    }

    #[test]
    fn this_store_may_not_hold_what_opens_the_vault() {
        // The assertion this whole file exists to make true. If somebody ever makes this
        // return a store that claims hardware backing without one, this fails.
        let capabilities = Scratch::new("capabilities").store().capabilities();

        assert!(!capabilities.hardware_backed());
        assert_eq!(capabilities.biometry(), None);
        assert!(!capabilities.may_hold_vault_material());
    }
}
