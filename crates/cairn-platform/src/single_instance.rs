//! One running copy of the application per data directory, and how that is enforced.
//!
//! The rule is not about tidiness. Two processes with the same database file open is two
//! connections, two logical clocks issuing readings from the same device identifier, and two
//! sets of keys in memory. The clock is the part that does real damage: both processes resume
//! from the highest reading in the file and then hand out readings independently, so they
//! produce the same reading twice, and two rows with the same reading are two rows no merge can
//! order.
//!
//! The lock is a file opened with the sharing mode set to zero. That is not an advisory flag and
//! not a convention: the operating system refuses to open the same file a second time while the
//! handle is alive. It is Rust from the standard library, so it adds no dependency and no
//! `unsafe`, and the handle is closed by the kernel when the process ends — including when it is
//! killed from the task manager, which is the case a sentinel file holding a process identifier
//! gets wrong. There is no stale lock to clean up, ever, because there is nothing written in the
//! file to go stale.
//!
//! On a platform where the system already guarantees a single instance, this reports that rather
//! than pretending to do the work, the same way [`crate::secure_storage`] reports a store it does
//! not have. A guard that silently succeeded would make "did the lock work" unanswerable.

use std::fmt;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// What the lock file is called inside the data directory.
pub const LOCK_FILE: &str = "cairn.lock";

/// Why this process may not run.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SingleInstanceError {
    /// Another copy of the application already has this directory.
    ///
    /// The one a caller shows a screen for. It carries nothing about the other process — not a
    /// process identifier, not a user, not a window — because the lock does not know any of that
    /// and because none of it would help the person reading the message.
    #[error("another copy of Cairn is already using this data directory")]
    AlreadyRunning,

    /// The lock file could not be created or opened for a reason other than it being held.
    ///
    /// A directory that does not exist, a disk that is full, a permission that was taken away.
    /// Distinct from [`Self::AlreadyRunning`] on purpose: one means "close the other window" and
    /// the other means "something is wrong with this machine", and a caller that showed the same
    /// screen for both would send somebody looking for a window that is not open.
    #[error("the lock file could not be opened")]
    Unavailable {
        /// The underlying failure, kept so the cause survives to the log.
        #[source]
        cause: io::Error,
    },

    /// This platform guarantees a single instance by itself, so there is nothing to take.
    ///
    /// Reported rather than silently succeeding. On iOS the system runs one copy of an
    /// application and there is no second process to exclude; a guard that returned as though it
    /// had locked something would make the question "is the lock working" unanswerable on the
    /// platform where it matters least and hide it on the one where it matters most.
    #[error("this platform guarantees a single instance without a lock")]
    GuaranteedByThePlatform,
}

/// The held lock.
///
/// Holds the open file and nothing else. Dropping it closes the handle, which releases the lock:
/// there is no unlock method, because a lock that can be released without being dropped is a lock
/// somebody releases while still using what it protected.
pub struct InstanceLock {
    path: PathBuf,
    // Never read. The lock is the open handle, and the handle lives exactly as long as this
    // value: closing it is what lets another process in, so it must not be closed early.
    _handle: File,
}

impl fmt::Debug for InstanceLock {
    /// Names the type and nothing else.
    ///
    /// Not the path. It contains the account name of whoever is logged in, and this value is
    /// reachable from the application state that a diagnostic dump would print.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InstanceLock(held)")
    }
}

impl InstanceLock {
    /// Where the lock file is.
    ///
    /// For tests and for the code that cleans a profile up. Not reported to anybody.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Takes the lock for a data directory, or says why it could not.
///
/// Called once, before anything opens the database, and the value is held for the whole run.
///
/// # Errors
///
/// Returns [`SingleInstanceError::AlreadyRunning`] if another process holds it,
/// [`SingleInstanceError::GuaranteedByThePlatform`] where there is nothing to take, and
/// [`SingleInstanceError::Unavailable`] if the file could not be opened for any other reason.
#[cfg(windows)]
pub fn acquire(directory: &Path) -> Result<InstanceLock, SingleInstanceError> {
    use std::os::windows::fs::OpenOptionsExt as _;

    let path = directory.join(LOCK_FILE);

    // Zero means no other handle may be opened to this file at all: not to read it, not to write
    // it, not to delete it. The second process asking for the same thing is refused by the
    // operating system rather than by anything this code has to remember to check.
    let handle = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .share_mode(0)
        .open(&path)
        .map_err(classify)?;

    Ok(InstanceLock {
        path,
        _handle: handle,
    })
}

/// Takes the lock for a data directory, or says why it could not.
///
/// # Errors
///
/// On a platform that guarantees a single instance, always
/// [`SingleInstanceError::GuaranteedByThePlatform`].
#[cfg(not(windows))]
pub fn acquire(_directory: &Path) -> Result<InstanceLock, SingleInstanceError> {
    Err(SingleInstanceError::GuaranteedByThePlatform)
}

/// Tells "somebody else has it" apart from "this machine has a problem".
///
/// Windows reports a sharing violation as `PermissionDenied`, which is also what a directory
/// somebody has no rights to reports. The raw code is what separates them, and it is checked
/// rather than guessed because the two lead to different screens.
#[cfg(windows)]
fn classify(cause: io::Error) -> SingleInstanceError {
    /// `ERROR_SHARING_VIOLATION`. The process cannot access the file because it is being used by
    /// another process.
    const SHARING_VIOLATION: i32 = 32;
    /// `ERROR_LOCK_VIOLATION`. The same answer for a byte range already locked.
    const LOCK_VIOLATION: i32 = 33;

    match cause.raw_os_error() {
        Some(SHARING_VIOLATION | LOCK_VIOLATION) => SingleInstanceError::AlreadyRunning,
        _other => SingleInstanceError::Unavailable { cause },
    }
}

#[cfg(test)]
mod tests {
    use super::{SingleInstanceError, acquire};

    /// A directory of its own for one test, removed when the value is dropped.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);

            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let ordinal = COUNTER.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "cairn-lock-{label}-{}-{nanos}-{ordinal}",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).expect("a temporary directory can be created");

            Self(directory)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_first_copy_takes_the_lock_and_the_second_is_refused() {
        let scratch = Scratch::new("second");

        let first = acquire(&scratch.0).expect("the first copy takes the lock");
        assert!(first.path().ends_with(super::LOCK_FILE));

        match acquire(&scratch.0) {
            Err(SingleInstanceError::AlreadyRunning) => {}
            other => panic!("the second copy was not refused: {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn releasing_the_lock_lets_the_next_copy_in() {
        // What the operating system does when a process ends, including when it is killed: the
        // handle is closed, and nothing is left behind to clean up. Dropping the value is the
        // same event from this side of the boundary.
        let scratch = Scratch::new("release");

        let first = acquire(&scratch.0).expect("the first copy takes the lock");
        drop(first);

        let second = acquire(&scratch.0).expect("the lock is free once the holder lets go");
        drop(second);
    }

    #[cfg(windows)]
    #[test]
    fn a_lock_file_left_behind_is_not_a_lock() {
        // The failure mode of a sentinel file holding a process identifier, which this design
        // does not have: the file survives a process that was killed, and the next run has to
        // decide whether the identifier in it still means anything. Here the file is allowed to
        // survive and means nothing at all, because the lock was the handle.
        let scratch = Scratch::new("stale");

        drop(acquire(&scratch.0).expect("the first copy takes the lock"));
        assert!(
            scratch.0.join(super::LOCK_FILE).exists(),
            "the test is not exercising what it claims: the file was removed"
        );

        let again = acquire(&scratch.0).expect("a file left behind does not hold the lock");
        drop(again);
    }

    #[cfg(windows)]
    #[test]
    fn a_directory_that_is_not_there_is_a_different_problem_from_a_second_copy() {
        let scratch = Scratch::new("missing");
        let missing = scratch.0.join("no-such-directory");

        match acquire(&missing) {
            Err(SingleInstanceError::Unavailable { .. }) => {}
            other => panic!("a missing directory was not reported as a machine problem: {other:?}"),
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn a_platform_that_guarantees_one_instance_says_so_instead_of_pretending() {
        let scratch = Scratch::new("guaranteed");

        match acquire(&scratch.0) {
            Err(SingleInstanceError::GuaranteedByThePlatform) => {}
            other => panic!("the platform stub did not report what it is: {other:?}"),
        }
    }
}
