//! Drives the sweep against a real vault, a real database and rows written by both writers.
//!
//! One question decides whether this command may exist at all: can it reach a habit somebody
//! wrote? Everything else about it — how many rows, how fast, what it answers — is a detail that
//! a wrong answer here makes irrelevant, so the assertions below are about the boundary between
//! the two kinds of row and nothing else.
//!
//! The names that must survive are the interesting half. A habit called `Hábito de pruebas`, one
//! called `Hábito de prueba de verdad` and one called `hábito de prueba 3` all look like the
//! module's own to any rule written in a hurry, and all three are somebody's.
// Every function in an integration test file is test code, but the lint that forbids panicking
// constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]` functions. The
// helpers below are neither, and a helper that cannot panic would have to return a Result that
// every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES};
use cairn_db::repositories::habits::{self as repository, NewHabit};
use cairn_domain::CivilDay;
use cairn_lib::commands::sample::{SAMPLE_NAME, sweep};
use cairn_lib::commands::vault::create as create_vault;
use cairn_lib::state::AppState;
use cairn_lib::storage::DataDirectory;
use cairn_lib::vault::Vault;
use zeroize::Zeroizing;

/// Not a real password. A phrase invented for these tests, in a directory removed afterwards.
const NOT_A_REAL_PASSWORD: &str = "una frase larga inventada para la prueba";

/// A moment in the middle of the range, so the arithmetic either side of it is ordinary.
const NOW_US: i64 = 1_700_000_000_000_000;

/// The same moment in milliseconds, which is what the clock the rows are stamped with takes.
const NOW_MS: u64 = 1_700_000_000_000;

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
                "cairn-sweep-{name}-{}-{unique}",
                std::process::id()
            )),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// A created, open vault in a scratch directory.
fn unlocked(scratch: &Scratch) -> AppState {
    let state = AppState::new(
        Vault::open_at(&scratch.directory).expect("the directory can be read"),
        DataDirectory::new(scratch.directory.clone()),
    );

    let status = tauri::async_runtime::block_on(create_vault(
        &state,
        Zeroizing::new(NOT_A_REAL_PASSWORD.to_owned()),
        // The cheapest parameters the cryptographic crate accepts. What the real ones are is
        // asserted in the crate that owns them; what these are for is finishing in milliseconds.
        Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES).expect("the lowest values"),
        NOW_US,
    ))
    .expect("a vault can be created in an empty directory");

    assert!(status.unlocked, "the vault has to be open for these tests");
    state
}

/// Writes habits with exactly these names, through the repository rather than through a command.
///
/// The repository on purpose: the command that creates a habit validates a draft, and what this
/// file needs is a row with a name of its choosing, including names no form would accept.
fn write(state: &AppState, names: &[&str]) {
    let written = state.session().with_open(|vault, storage| {
        let codec = storage.codec(vault);
        let device = storage.device();

        storage.database().with(|connection| {
            for (ordinal, name) in names.iter().enumerate() {
                repository::create(
                    connection,
                    &codec,
                    device,
                    storage.next_hlc(NOW_MS),
                    NOW_US,
                    NewHabit::plain(
                        name,
                        CivilDay::new(2026, 1, 1).expect("a day this calendar can name"),
                        i64::try_from(ordinal).expect("a small index"),
                    ),
                )?;
            }
            Ok::<(), cairn_db::DbError>(())
        })
    });

    written.expect("the vault is open").expect("the rows go in");
}

/// The names of every habit still alive, in no particular order.
fn survivors(state: &AppState) -> Vec<String> {
    let read = state.session().with_open(|vault, storage| {
        let codec = storage.codec(vault);
        storage
            .database()
            .with(|connection| repository::page(connection, &codec, None, 200))
    });

    let mut names: Vec<String> = read
        .expect("the vault is open")
        .expect("the page is read")
        .iter()
        .map(|habit| habit.name.clone())
        .collect();
    names.sort();
    names
}

#[test]
fn a_sweep_takes_the_rows_this_module_wrote_and_nothing_else() {
    let scratch = Scratch::new("boundary");
    let state = unlocked(&scratch);

    write(
        &state,
        &[
            // The two shapes the module's own writers produce.
            SAMPLE_NAME,
            "Hábito de prueba 0",
            "Hábito de prueba 41",
            // Everything a rule written in a hurry would take with them.
            "Hábito de pruebas",
            "Hábito de prueba de verdad",
            "hábito de prueba 3",
            "Hábito de prueba\u{20}", // trailing space, which is a different name
            "\u{20}Hábito de prueba", // leading space, likewise
            "Meditar",
        ],
    );

    let report = sweep(&state, NOW_US, NOW_MS).expect("the vault is open");

    assert_eq!(report.removed, 3, "only the three the module wrote go");
    assert_eq!(report.remaining, 6, "the other six are somebody's own");
    assert_eq!(
        survivors(&state),
        vec![
            "\u{20}Hábito de prueba".to_owned(),
            "Hábito de prueba\u{20}".to_owned(),
            "Hábito de prueba de verdad".to_owned(),
            "Hábito de pruebas".to_owned(),
            "Meditar".to_owned(),
            "hábito de prueba 3".to_owned(),
        ],
        "every name that is not one of the two shapes survives, byte for byte"
    );
}

#[test]
fn a_sweep_over_more_rows_than_one_page_holds_leaves_none_behind() {
    let scratch = Scratch::new("paging");
    let state = unlocked(&scratch);

    // Past the two hundred a page holds, twice over, because the walk that collects them pages
    // by the same clock reading that deleting changes. A sweep that deleted while it walked
    // would answer a number and leave the rest, which is the failure this number exists to catch.
    let names: Vec<String> = (0..450).map(|at| format!("{SAMPLE_NAME} {at}")).collect();
    let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
    write(&state, &borrowed);
    write(&state, &["Meditar"]);

    let report = sweep(&state, NOW_US, NOW_MS).expect("the vault is open");

    assert_eq!(report.removed, 450, "every one of them, not the first page");
    assert_eq!(report.remaining, 1);
    assert_eq!(survivors(&state), vec!["Meditar".to_owned()]);
}

#[test]
fn a_sweep_with_nothing_to_sweep_removes_nothing_and_says_so() {
    let scratch = Scratch::new("empty");
    let state = unlocked(&scratch);

    write(&state, &["Meditar", "Leer"]);
    let report = sweep(&state, NOW_US, NOW_MS).expect("the vault is open");

    assert_eq!(report.removed, 0);
    assert_eq!(report.remaining, 2);
    assert_eq!(
        survivors(&state),
        vec!["Leer".to_owned(), "Meditar".to_owned()]
    );
}

#[test]
fn a_second_sweep_finds_the_first_one_finished_the_job() {
    let scratch = Scratch::new("twice");
    let state = unlocked(&scratch);

    write(&state, &[SAMPLE_NAME, "Hábito de prueba 1", "Meditar"]);

    assert_eq!(sweep(&state, NOW_US, NOW_MS).expect("open").removed, 2);

    let again = sweep(&state, NOW_US, NOW_MS).expect("open");
    assert_eq!(again.removed, 0, "a tombstone is not swept a second time");
    assert_eq!(again.remaining, 1);
}

#[test]
fn a_sweep_refuses_while_the_vault_is_locked() {
    let scratch = Scratch::new("locked");
    let state = unlocked(&scratch);
    write(&state, &[SAMPLE_NAME]);
    state.session().lock();

    assert!(
        sweep(&state, NOW_US, NOW_MS).is_err(),
        "a closed vault has no rows to answer with"
    );
}
