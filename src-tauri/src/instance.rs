//! Whether this process is the one that owns the data directory.
//!
//! Decided once, before anything reads the vault, and held for the whole run. The value exists
//! so that the answer is a thing the interface can ask about rather than a decision taken in
//! silence: a second copy that simply vanished would look like an application that fails to
//! start, and the person would try again.
//!
//! What is held is the lock itself. Dropping this value releases it, which is why it is managed
//! for the lifetime of the application and never cloned.

use cairn_platform::single_instance::{InstanceLock, SingleInstanceError, acquire};

use crate::storage::DataDirectory;

/// What this process is allowed to do with the data directory.
///
/// Serialised for the interface. Five states rather than a boolean, because they lead to five
/// different screens: carry on, explain that another copy has it, explain that something is
/// wrong with the machine, explain that there is nowhere to keep a vault, and carry on because
/// there was nothing to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
#[non_exhaustive]
pub enum InstanceStatus {
    /// This process took the lock and may use the directory.
    Held,
    /// Another copy of the application has it.
    AlreadyRunning,
    /// The lock could not be taken for a reason that is not another copy.
    Unavailable,
    /// There is no directory to take a lock in.
    ///
    /// The vault lives in one place decided by this build, and something stopped that place
    /// from being settled on: a `CAIRN_PROFILE` that is not a name this accepts, a system that
    /// did not say where per-user data belongs, or a directory that could not be created. All
    /// three end the same way — there is nowhere to put a vault — and all three have to say so
    /// rather than closing the window, because a window that vanishes looks like a broken
    /// application and the person tries again.
    NoDirectory,
    /// The platform runs one copy of an application by itself.
    GuaranteedByThePlatform,
}

impl InstanceStatus {
    /// Whether the application may go on to read and write the vault.
    #[must_use]
    pub const fn may_use_the_directory(self) -> bool {
        matches!(self, Self::Held | Self::GuaranteedByThePlatform)
    }
}

/// The lock on the data directory, for as long as this process runs.
#[derive(Debug)]
pub struct Instance {
    status: InstanceStatus,
    /// Never read. The lock is the open handle inside it, and it must live as long as the
    /// process: dropping it is what lets another copy in.
    _lock: Option<InstanceLock>,
}

impl Instance {
    /// Takes the lock for a directory, whatever the answer turns out to be.
    ///
    /// Does not fail. Every outcome is a state the application has a screen for, and a startup
    /// that returned an error here would close the window before anything could be said in it.
    #[must_use]
    pub fn take(directory: &DataDirectory) -> Self {
        match acquire(directory.path()) {
            Ok(lock) => Self {
                status: InstanceStatus::Held,
                _lock: Some(lock),
            },
            Err(SingleInstanceError::AlreadyRunning) => Self {
                status: InstanceStatus::AlreadyRunning,
                _lock: None,
            },
            Err(SingleInstanceError::GuaranteedByThePlatform) => Self {
                status: InstanceStatus::GuaranteedByThePlatform,
                _lock: None,
            },
            // The cause is deliberately not carried any further. It is an `io::Error` whose
            // message names the path, the path names the account somebody is logged in as, and
            // this value is what a command hands to the WebView.
            //
            // The wildcard is here because `SingleInstanceError` is `#[non_exhaustive]`: a
            // variant added in the platform crate later arrives as "something is wrong with
            // this machine", which is the honest answer for a refusal this build cannot name,
            // and it never arrives as permission to open the vault.
            Err(_machine_problem) => Self {
                status: InstanceStatus::Unavailable,
                _lock: None,
            },
        }
    }

    /// The answer when this build could not settle on a directory at all.
    ///
    /// There is no lock to hold, because there is no place to hold one. It is a constructor
    /// rather than a value the caller builds so that the one way to get an [`Instance`] that
    /// may use the directory stays [`Self::take`], which is the call that actually takes the
    /// lock.
    #[must_use]
    pub const fn without_a_directory() -> Self {
        Self {
            status: InstanceStatus::NoDirectory,
            _lock: None,
        }
    }

    /// What this process is allowed to do.
    #[must_use]
    pub const fn status(&self) -> InstanceStatus {
        self.status
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::{Instance, InstanceStatus};
    use crate::storage::DataDirectory;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "cairn-instance-{name}-{}-{unique}",
                std::process::id()
            ));
            let _created = fs::create_dir_all(&directory);

            Self(directory)
        }

        fn directory(&self) -> DataDirectory {
            DataDirectory::new(self.0.clone())
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_first_copy_may_use_the_directory() {
        let scratch = Scratch::new("first");
        let instance = Instance::take(&scratch.directory());

        assert!(instance.status().may_use_the_directory());
    }

    #[cfg(windows)]
    #[test]
    fn a_second_copy_is_told_which_of_the_two_problems_it_has() {
        let scratch = Scratch::new("second");
        let first = Instance::take(&scratch.directory());
        assert_eq!(first.status(), InstanceStatus::Held);

        let second = Instance::take(&scratch.directory());
        assert_eq!(second.status(), InstanceStatus::AlreadyRunning);
        assert!(
            !second.status().may_use_the_directory(),
            "a refused copy was about to open the vault anyway"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_machine_problem_is_not_reported_as_another_copy() {
        // The two lead to different screens, and a person sent looking for a window that is not
        // open will not find the disk that is full.
        let scratch = Scratch::new("machine");
        let missing = DataDirectory::new(scratch.0.join("no-such-directory"));

        assert_eq!(
            Instance::take(&missing).status(),
            InstanceStatus::Unavailable
        );
    }

    #[test]
    fn a_process_with_nowhere_to_put_a_vault_is_refused_rather_than_closed() {
        // The defect this keeps fixed: a `CAIRN_PROFILE` that is not a name this accepts used
        // to end the startup with an error, which closes the window before anything can be
        // said in it. Refusing is right; vanishing is not. What matters here is that the
        // refusal is a state with a screen behind it and that it never reads as permission.
        let instance = Instance::without_a_directory();

        assert_eq!(instance.status(), InstanceStatus::NoDirectory);
        assert!(
            !instance.status().may_use_the_directory(),
            "a process with no directory was about to open a vault in it"
        );
    }

    #[test]
    fn the_status_carries_nothing_but_its_own_name() {
        // What the interface receives. A path, a process identifier or an operating system
        // message would all name the account somebody is logged in as, and this value crosses
        // the bridge into a WebView.
        let scratch = Scratch::new("privacy");
        let serialised = serde_json::to_string(&Instance::take(&scratch.directory()).status())
            .expect("the status serialises");

        assert!(
            !serialised.contains(&std::process::id().to_string()),
            "the status carried a process identifier: {serialised}"
        );
        assert!(
            !serialised.contains("cairn-instance"),
            "the status carried a path: {serialised}"
        );
    }
}
