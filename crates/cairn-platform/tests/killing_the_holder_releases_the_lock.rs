//! What happens to the instance lock when the process holding it does not get to tidy up.
//!
//! The unit tests next to the module drop the lock, which is what an orderly shutdown does.
//! This one kills the process instead, because that is the case a sentinel file holding a
//! process identifier gets wrong: the file survives, the identifier in it means nothing, and the
//! next run has to decide whether to believe it. Here there is nothing to decide, because the
//! lock was the open handle and the operating system closes it when the process ends however it
//! ends.
//!
//! The second process is this test binary run again with an environment variable set, which is
//! the usual way to get a real, separate process without adding a helper crate to the workspace.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test that cannot fail loudly is a test that reports success for the wrong reason"
)]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use cairn_platform::single_instance::{LOCK_FILE, SingleInstanceError, acquire};

/// Set on the child, and nothing else reads it. Its value is the directory to lock.
const HOLD: &str = "CAIRN_TEST_HOLD_INSTANCE_LOCK";

/// How long the child holds the lock before giving up on its own.
///
/// A ceiling rather than a schedule. The test kills it long before this, and the timeout only
/// matters if the test itself dies: without one, a failed run would leave a process behind.
const HOLD_FOR: Duration = Duration::from_secs(60);

/// How long the parent waits for the child to have taken the lock.
const WAIT_FOR_CHILD: Duration = Duration::from_secs(10);

#[test]
fn killing_the_process_that_holds_the_lock_leaves_nothing_to_clean_up() {
    // The child arm. Running the test binary again re-enters here, takes the lock, and waits to
    // be killed.
    if let Ok(directory) = env::var(HOLD) {
        let _held = acquire(PathBuf::from(directory).as_path()).expect("the child takes the lock");
        std::thread::sleep(HOLD_FOR);
        return;
    }

    let directory = scratch();
    fs::create_dir_all(&directory).expect("a temporary directory can be created");

    let mut child = Command::new(env::current_exe().expect("the test binary knows its own path"))
        .env(HOLD, &directory)
        // The exact test, so the child runs this function and nothing else in the file.
        .arg("killing_the_process_that_holds_the_lock_leaves_nothing_to_clean_up")
        .arg("--exact")
        .arg("--nocapture")
        .spawn()
        .expect("the test binary can be run again");

    // Waited for rather than slept through: the child has the lock when this process is refused,
    // and that is the only moment the rest of the test means anything.
    let started = Instant::now();
    loop {
        match acquire(&directory) {
            Err(SingleInstanceError::AlreadyRunning) => break,
            Err(SingleInstanceError::GuaranteedByThePlatform) => {
                // Nothing to test on a platform that has no lock to take.
                let _killed = child.kill();
                let _reaped = child.wait();
                let _removed = fs::remove_dir_all(&directory);
                return;
            }
            Ok(lock) => {
                drop(lock);
                assert!(
                    started.elapsed() < WAIT_FOR_CHILD,
                    "the child never took the lock"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(other) => panic!("the lock could not be asked about: {other:?}"),
        }
    }

    // No chance to run a destructor, close a handle or remove a file. This is the task manager.
    child.kill().expect("the child can be killed");
    let _reaped = child.wait().expect("the child can be reaped");

    // The file is allowed to survive, and it means nothing: the lock was the handle.
    let taken = wait_for_the_lock(&directory);
    assert!(
        taken,
        "the lock was still held after the process holding it was killed, which means there is a \
         stale lock somebody has to clean up by hand"
    );

    let _removed = fs::remove_dir_all(&directory);
}

/// Tries to take the lock until it succeeds or the wait runs out.
///
/// Windows closes the handle as part of tearing the process down, and that happens after `wait`
/// returns often enough that asking once would be a test that fails now and then for a reason
/// that has nothing to do with the code.
fn wait_for_the_lock(directory: &std::path::Path) -> bool {
    let started = Instant::now();
    while started.elapsed() < WAIT_FOR_CHILD {
        if let Ok(lock) = acquire(directory) {
            assert!(
                lock.path().ends_with(LOCK_FILE),
                "the lock was taken on something other than the lock file"
            );
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    false
}

/// A directory of this test's own, unique per run as well as per test.
fn scratch() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    env::temp_dir().join(format!("cairn-lock-killed-{}-{nanos}", std::process::id()))
}
