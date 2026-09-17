//! Drives the whole life of a vault through the operations the commands are built on.
//!
//! Everything below runs against a real application state and a real directory: a header is
//! written, read back by a second state as if the process had restarted, and opened with a
//! password that is actually derived. The unit tests in the modules themselves cover the
//! pieces; this covers the sequence, which is where the mistakes that matter live.
//!
//! Three of them are worth naming, because each would be invisible from any single module.
//! Changing the master password must leave the data key alone, or every stored byte becomes
//! unreadable. Changing the parameters must do the same. And the count of failed attempts has
//! to survive a restart, or closing the window is a way to have the lockout forgotten.
//!
//! The clock is a parameter here as it is everywhere else in the project, so the whole
//! backoff sequence is walked in microseconds rather than waited out in minutes.
// Every function in an integration test file is test code, but the lint that forbids
// panicking constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]`
// functions. The helpers below are neither, and a helper that cannot panic would have to
// return a Result that every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES};
use cairn_lib::commands::vault::{
    ConditionReport, InactivityChoice, VaultError, VaultStatus, change_kdf_params, change_password,
    create, heartbeat, set_inactivity, unlock,
};
use cairn_lib::state::AppState;
use cairn_lib::storage::DataDirectory;
use cairn_lib::vault::Vault;
use zeroize::Zeroizing;

/// Not a real password. A phrase invented for these tests, in a directory removed afterwards.
const NOT_A_REAL_PASSWORD: &str = "una frase larga inventada para la prueba";

/// The replacement used by the password change tests. Also invented, also not in use.
const ANOTHER_ONE: &str = "otra frase distinta y tambien larga";

/// A moment in the middle of the range, so the arithmetic either side of it is ordinary.
const NOW_US: i64 = 1_700_000_000_000_000;

/// One second, in the units every moment in this project uses.
const ONE_SECOND: i64 = 1_000_000;

/// A scratch directory that belongs to one test and is removed when it ends.
struct Scratch {
    directory: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);

        Self {
            directory: std::env::temp_dir().join(format!(
                "cairn-lifecycle-{name}-{}-{unique}",
                std::process::id()
            )),
        }
    }

    /// A fresh application state over the same directory.
    ///
    /// Calling this twice is what a restart looks like from in here: nothing is carried over
    /// except what was written to the disk.
    fn state(&self) -> AppState {
        AppState::new(
            Vault::open_at(&self.directory).expect("the directory can be read"),
            DataDirectory::new(self.directory.clone()),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// The cheapest parameters the cryptographic crate accepts.
///
/// These are not what a vault ships with. They are what makes a test that performs a dozen
/// derivations finish in seconds, and what the real parameters are is asserted in the crate
/// that owns them.
fn cheap() -> Argon2Params {
    Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES).expect("the lowest accepted values")
}

/// Slightly more expensive parameters, for the test that changes them.
fn cheap_but_slower() -> Argon2Params {
    Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES + 1, MAX_LANES).expect("inside the range")
}

fn phrase(text: &str) -> Zeroizing<String> {
    Zeroizing::new(text.to_owned())
}

/// Runs an operation on the runtime the application uses.
fn run<F: std::future::Future>(task: F) -> F::Output {
    tauri::async_runtime::block_on(task)
}

/// Creates a vault in a scratch directory and hands back the state holding it open.
fn a_created_vault(scratch: &Scratch) -> AppState {
    let state = scratch.state();
    let status = run(create(&state, phrase(NOT_A_REAL_PASSWORD), cheap(), NOW_US))
        .expect("a vault can be created in an empty directory");

    assert!(status.exists);
    assert!(status.unlocked);

    state
}

#[test]
fn a_created_vault_is_open_and_is_there_after_a_restart() {
    let scratch = Scratch::new("create");
    let created = a_created_vault(&scratch);

    assert_eq!(
        ConditionReport::from(created.vault().condition()),
        ConditionReport::Readable
    );

    // What a restart looks like: a second state over the same directory, carrying nothing
    // but what reached the disk.
    let restarted = scratch.state();
    assert!(restarted.vault().exists());
    assert!(
        !restarted.session().is_unlocked(),
        "a restarted process began with the keys already in it"
    );
}

#[test]
fn a_second_vault_is_refused_rather_than_written_over_the_first() {
    // The single most destructive thing this program could do. A second header would make
    // every byte encrypted under the first unreadable, with no warning and no way back.
    let scratch = Scratch::new("second");
    let _first = a_created_vault(&scratch);

    let restarted = scratch.state();
    let refused = run(create(&restarted, phrase(ANOTHER_ONE), cheap(), NOW_US))
        .expect_err("a second vault was created over the first");

    assert_eq!(refused, VaultError::AlreadyExists);
}

#[test]
fn a_password_that_is_too_short_creates_nothing_at_all() {
    let scratch = Scratch::new("short");
    let state = scratch.state();

    let refused = run(create(&state, phrase("corta"), cheap(), NOW_US))
        .expect_err("a short password created a vault");

    assert!(matches!(refused, VaultError::PasswordTooShort { .. }));
    assert!(!state.vault().exists());
    assert!(!state.session().is_unlocked());
    assert!(
        !scratch.directory.join("cairn.header").exists(),
        "a refused creation left a header behind"
    );
}

#[test]
fn the_right_password_opens_the_vault_after_a_restart() {
    let scratch = Scratch::new("unlock");
    let _created = a_created_vault(&scratch);

    let restarted = scratch.state();
    let status = run(unlock(&restarted, phrase(NOT_A_REAL_PASSWORD), NOW_US))
        .expect("the right password opens the vault");

    assert!(status.unlocked);
    assert_eq!(status.failed_attempts, 0);
    assert_eq!(status.locked_out_for_s, 0);
}

#[test]
fn a_wrong_password_is_counted_and_the_count_survives_a_restart() {
    // Closing the window must not be a way to have the lockout forgotten, which is the whole
    // reason the count is written to the header rather than kept in memory.
    let scratch = Scratch::new("wrong");
    let _created = a_created_vault(&scratch);

    let restarted = scratch.state();
    let refused = run(unlock(&restarted, phrase(ANOTHER_ONE), NOW_US))
        .expect_err("a wrong password opened the vault");

    assert_eq!(refused, VaultError::NotOpened);
    assert!(!restarted.session().is_unlocked());

    let restarted_again = scratch.state();
    assert_eq!(restarted_again.vault().failed_attempts(), 1);
}

#[test]
fn the_wait_doubles_with_each_failure_and_goes_back_to_nothing_after_a_success() {
    let scratch = Scratch::new("backoff");
    let _created = a_created_vault(&scratch);
    let state = scratch.state();

    // One, two, four. Each attempt is made after the previous wait has elapsed, so what is
    // being measured is the schedule rather than the refusal.
    let mut at = NOW_US;
    for expected_wait in [1_i64, 2, 4] {
        let refused = run(unlock(&state, phrase(ANOTHER_ONE), at))
            .expect_err("a wrong password opened the vault");
        assert_eq!(refused, VaultError::NotOpened);

        // Immediately afterwards the next attempt is refused without deriving anything, and
        // the refusal says how long is left.
        let locked_out = run(unlock(&state, phrase(NOT_A_REAL_PASSWORD), at))
            .expect_err("the right password was accepted during the lockout");
        assert_eq!(
            locked_out,
            VaultError::LockedOut {
                remaining_s: u32::try_from(expected_wait).expect("a small number"),
            }
        );

        at += expected_wait * ONE_SECOND;
    }

    let opened = run(unlock(&state, phrase(NOT_A_REAL_PASSWORD), at))
        .expect("the right password opens the vault once the wait has passed");

    assert!(opened.unlocked);
    assert_eq!(
        opened.failed_attempts, 0,
        "the count was not reset after a successful unlock"
    );
    assert_eq!(opened.locked_out_for_s, 0);
}

#[test]
fn an_unlock_on_a_machine_with_no_vault_says_so_rather_than_failing_to_open_one() {
    // A different answer from a wrong password, and not an oracle: whether a file exists is
    // something anybody looking at the machine can see for themselves.
    let scratch = Scratch::new("no-vault");
    let state = scratch.state();

    let refused = run(unlock(&state, phrase(NOT_A_REAL_PASSWORD), NOW_US))
        .expect_err("a vault that does not exist was opened");

    assert_eq!(refused, VaultError::NoVault);
}

#[test]
fn changing_the_password_keeps_the_data_key_and_retires_the_old_one() {
    // The operation the whole key hierarchy is arranged to make cheap. If the data key
    // changed here, every byte stored under it would be unreadable afterwards.
    let scratch = Scratch::new("change-password");
    let state = a_created_vault(&scratch);
    let key_id_before = *state.vault().header().expect("the vault exists").key_id();

    run(change_password(
        &state,
        phrase(NOT_A_REAL_PASSWORD),
        phrase(ANOTHER_ONE),
        NOW_US,
    ))
    .expect("the password can be changed");

    let restarted = scratch.state();
    assert_eq!(
        *restarted
            .vault()
            .header()
            .expect("the vault exists")
            .key_id(),
        key_id_before,
        "changing the password changed the data key, which loses every stored byte"
    );

    assert_eq!(
        run(unlock(&restarted, phrase(NOT_A_REAL_PASSWORD), NOW_US))
            .expect_err("the old password still opens the vault"),
        VaultError::NotOpened
    );
    // A second later, because the attempt just above was a failure and failures cost a
    // second. That the new password has to wait it out is the schedule working.
    assert!(
        run(unlock(&restarted, phrase(ANOTHER_ONE), NOW_US + ONE_SECOND))
            .expect("the new password opens the vault")
            .unlocked
    );
}

#[test]
fn a_password_change_with_the_wrong_current_password_changes_nothing() {
    let scratch = Scratch::new("change-refused");
    let state = a_created_vault(&scratch);

    let refused = run(change_password(
        &state,
        phrase(ANOTHER_ONE),
        phrase("una tercera frase igual de larga"),
        NOW_US,
    ))
    .expect_err("the wrong current password changed the vault");

    assert_eq!(refused, VaultError::NotOpened);

    let restarted = scratch.state();
    assert!(
        run(unlock(&restarted, phrase(NOT_A_REAL_PASSWORD), NOW_US))
            .expect("the original password still opens the vault")
            .unlocked
    );
}

#[test]
fn changing_the_parameters_keeps_the_password_and_the_data_key() {
    // What the phase exists to prove. The parameters have to be raisable on hardware nobody
    // has measured yet, and raising them must not cost a single stored byte.
    let scratch = Scratch::new("change-params");
    let state = a_created_vault(&scratch);
    let key_id_before = *state.vault().header().expect("the vault exists").key_id();

    let status = run(change_kdf_params(
        &state,
        phrase(NOT_A_REAL_PASSWORD),
        cheap_but_slower(),
        NOW_US,
    ))
    .expect("the parameters can be changed");

    assert_eq!(
        status.kdf.expect("a vault has parameters").passes,
        cheap_but_slower().passes()
    );

    let restarted = scratch.state();
    assert_eq!(
        *restarted
            .vault()
            .header()
            .expect("the vault exists")
            .key_id(),
        key_id_before,
        "changing the parameters changed the data key, which loses every stored byte"
    );
    assert!(
        run(unlock(&restarted, phrase(NOT_A_REAL_PASSWORD), NOW_US))
            .expect("the same password opens the vault at the new parameters")
            .unlocked
    );
}

#[test]
fn locking_closes_the_vault_and_a_heartbeat_afterwards_is_told_to_stop() {
    let scratch = Scratch::new("lock");
    let state = a_created_vault(&scratch);

    assert!(heartbeat(&state, NOW_US).is_ok());

    state.session().lock();

    assert_eq!(
        heartbeat(&state, NOW_US).expect_err("a locked vault accepted a heartbeat"),
        VaultError::Locked
    );
}

#[test]
fn a_heartbeat_puts_the_inactivity_timer_back_to_the_beginning() {
    let scratch = Scratch::new("heartbeat");
    let state = a_created_vault(&scratch);
    let four_minutes = 4 * 60 * ONE_SECOND;

    let status: VaultStatus =
        heartbeat(&state, NOW_US + four_minutes).expect("an open vault accepts a heartbeat");

    assert_eq!(
        status.idle_remaining_s,
        Some(300),
        "the timer was not put back to the beginning"
    );
}

#[test]
fn choosing_a_shorter_period_applies_at_once_and_does_not_lock_the_vault_in_the_same_instant() {
    // Somebody who has been reading for four minutes and then picks one minute should get a
    // minute, not an immediate lock.
    let scratch = Scratch::new("inactivity");
    let state = a_created_vault(&scratch);
    let four_minutes_later = NOW_US + 4 * 60 * ONE_SECOND;

    let status = set_inactivity(&state, InactivityChoice::One, four_minutes_later);

    assert_eq!(status.inactivity, InactivityChoice::One);
    assert_eq!(status.idle_remaining_s, Some(60));
    assert!(state.session().is_unlocked());
}

#[test]
fn choosing_never_stops_the_countdown_altogether() {
    let scratch = Scratch::new("never");
    let state = a_created_vault(&scratch);

    let status = set_inactivity(&state, InactivityChoice::Never, NOW_US);

    assert_eq!(status.inactivity, InactivityChoice::Never);
    assert_eq!(
        status.idle_remaining_s, None,
        "a vault that never locks was given a countdown to draw"
    );
    assert_eq!(state.session().due_to_lock(i64::MAX), None);
}
