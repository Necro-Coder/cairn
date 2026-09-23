//! Drives the three commands of the bin, and the sweep the unlock runs before anything else.
//!
//! The property worth the file is the one nobody notices until the day it matters: something
//! thrown away comes back **identical**. Not similar, not with its title and most of its fields:
//! the same title, the same user name, the same notes, the same addresses, the same custom fields
//! and the same password. So the round trip below compares what the entry holds before and after,
//! reading it straight out of the file, rather than checking that the command answered without
//! complaining.
//!
//! The other two are about the two ways out. There is no path from a list to an irreversible
//! deletion — going through the bin is the confirmation, and it lasts thirty days — and what has
//! run out of those days stops existing on the way in, on the unlock, without the unlock ever
//! depending on it.
// Every function in an integration test file is test code, but the lint that forbids panicking
// constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]` functions. The
// helpers below are neither, and a helper that cannot panic would have to return a Result that
// every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES};
use cairn_db::repositories::vault as repository;
use cairn_db::{Connection, DbError, FieldCodec};
use cairn_lib::commands::passwords::{
    DraftFieldDto, EntryDraftDto, EntryFilter, EntryKindDto, PasswordsError, create, delete,
    empty_trash, get, list, search, trash,
};
use cairn_lib::commands::vault::{create as create_vault, unlock as unlock_vault};
use cairn_lib::state::AppState;
use cairn_lib::storage::{DataDirectory, Storage};
use cairn_lib::vault::Vault;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Not a real password. A phrase invented for these tests, in a directory removed afterwards.
const NOT_A_REAL_PASSWORD: &str = "una frase larga inventada para la prueba";

/// The password these drafts carry.
const A_WRITTEN_PASSWORD: &str = "cairn-canary-password";

/// The value of the secret custom field.
const A_SECRET_VALUE: &str = "cairn-canary-secret";

/// A moment in the middle of the range, so the arithmetic either side of it is ordinary.
const NOW_US: i64 = 1_700_000_000_000_000;

/// One day, in microseconds, for the two entries the sweep has to tell apart.
const A_DAY_US: i64 = 24 * 60 * 60 * 1_000_000;

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
                "cairn-passwords-trash-{name}-{}-{unique}",
                std::process::id()
            )),
        }
    }

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
fn cheap() -> Argon2Params {
    Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES).expect("the lowest accepted values")
}

/// A created, open vault in a scratch directory.
fn unlocked(scratch: &Scratch) -> AppState {
    let state = scratch.state();
    let status = tauri::async_runtime::block_on(create_vault(
        &state,
        Zeroizing::new(NOT_A_REAL_PASSWORD.to_owned()),
        cheap(),
        NOW_US,
    ))
    .expect("a vault can be created in an empty directory");

    assert!(status.unlocked, "the vault has to be open for these tests");

    state
}

/// Closes the vault and opens it again at that moment, which is what runs the sweep.
fn reopened(state: &AppState, now: i64) {
    assert!(state.session().lock(), "the vault was open");
    let status = tauri::async_runtime::block_on(unlock_vault(
        state,
        Zeroizing::new(NOT_A_REAL_PASSWORD.to_owned()),
        now,
    ))
    .expect("the vault reopens");

    assert!(status.unlocked, "the unlock did not open the vault");
}

/// Runs something against the open database, for what the commands cannot say by themselves.
fn in_database<T>(
    state: &AppState,
    work: impl FnOnce(&Storage, &FieldCodec<'_>, &Connection) -> Result<T, DbError>,
) -> Result<T, DbError> {
    state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage
                .database()
                .with(|connection| work(storage, &codec, connection))
        })
        .expect("the vault is open")
}

/// A draft with a user name, a password, two addresses and two fields, one of them secret.
fn furnished(title: &str) -> EntryDraftDto {
    EntryDraftDto {
        kind: EntryKindDto::Account,
        title: title.to_owned(),
        username: Some("alguien@ejemplo".to_owned()),
        password: Some(A_WRITTEN_PASSWORD.to_owned()),
        notes: Some("una nota".to_owned()),
        urls: vec![
            "https://banco.es/login".to_owned(),
            "banco.example".to_owned(),
        ],
        fields: vec![
            DraftFieldDto {
                id: None,
                label: "Oficina".to_owned(),
                value: Some("Central".to_owned()),
                secret: false,
            },
            DraftFieldDto {
                id: None,
                label: "PIN".to_owned(),
                value: Some(A_SECRET_VALUE.to_owned()),
                secret: true,
            },
        ],
        folder_id: None,
        favorite: false,
    }
}

/// Writes a draft and answers with its identifier.
fn written(state: &AppState, title: &str) -> Uuid {
    let detail = create(state, &furnished(title), NOW_US).expect("a good draft is written");
    Uuid::parse_str(&detail.summary.id).expect("what came back is an identifier")
}

/// Everything an entry holds, flattened to text so that one comparison covers all of it.
///
/// Read through the repository rather than through `get`, because `get` deliberately leaves the
/// password and the value of every secret field behind, and those are exactly the two things a
/// restore could silently lose.
#[derive(Debug, PartialEq, Eq)]
struct Contents {
    entry: String,
    urls: Vec<String>,
    fields: Vec<String>,
}

/// What an entry holds, or nothing if it is not readable at all.
fn contents(state: &AppState, id: Uuid) -> Option<Contents> {
    in_database(state, |_storage, codec, connection| {
        let Some(found) = repository::entry(connection, codec, id)? else {
            return Ok(None);
        };

        Ok(Some(Contents {
            entry: format!(
                "{}|{:?}|{:?}|{:?}|{}",
                found.title.as_str(),
                found.username.as_deref().map(String::as_str),
                found.password.as_deref().map(String::as_str),
                found.notes.as_deref().map(String::as_str),
                found.favorite
            ),
            urls: repository::urls(connection, codec, id)?
                .iter()
                .map(|url| url.value.to_string())
                .collect(),
            fields: repository::fields(connection, codec, id)?
                .iter()
                .map(|field| format!("{}={}", field.label.as_str(), field.value.as_str()))
                .collect(),
        }))
    })
    .expect("the entry can be read")
}

/// The row of an entry as it stands, however deleted it is: whether it is a tombstone, and how
/// many of its encrypted columns still hold anything.
fn row_census(state: &AppState, id: Uuid) -> (i64, i64) {
    in_database(state, |_storage, _codec, connection| {
        Ok(connection.query_row(
            "SELECT deleted,
                    count(title) + count(username) + count(password) + count(notes)
               FROM vault_entries WHERE id = ?1",
            [id.as_bytes().as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    })
    .expect("the row can be counted")
}

/// How many rows of a child table an entry has, and how many of them still hold ciphertext.
fn child_census(state: &AppState, statement: &str, id: Uuid) -> (i64, i64) {
    let owned = statement.to_owned();
    in_database(state, move |_storage, _codec, connection| {
        Ok(
            connection.query_row(&owned, [id.as_bytes().as_slice()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?,
        )
    })
    .expect("the rows can be counted")
}

/// Moves the moment something went into the bin, for the tests about the sweep.
fn antedate(state: &AppState, id: Uuid, moment: i64) {
    in_database(state, |_storage, _codec, connection| {
        connection.execute(
            "UPDATE vault_entries SET trashed_at = ?2 WHERE id = ?1",
            (id.as_bytes().as_slice(), moment),
        )?;
        Ok(())
    })
    .expect("the moment can be moved");
}

#[test]
fn all_three_refuse_while_the_vault_is_locked() {
    let scratch = Scratch::new("locked");
    let state = scratch.state();
    let somewhere = Uuid::from_bytes([1; 16]).to_string();

    assert_eq!(
        trash(&state, &somewhere, true, NOW_US),
        Err(PasswordsError::Locked)
    );
    assert_eq!(
        delete(&state, &somewhere, NOW_US),
        Err(PasswordsError::Locked)
    );
    assert_eq!(empty_trash(&state, NOW_US), Err(PasswordsError::Locked));
}

#[test]
fn something_that_is_not_an_identifier_is_the_same_answer_as_something_that_is_not_there() {
    let scratch = Scratch::new("nonsense");
    let state = unlocked(&scratch);

    assert_eq!(
        trash(&state, "no soy un identificador", true, NOW_US),
        Err(PasswordsError::NotFound)
    );
    assert_eq!(
        delete(&state, "tampoco", NOW_US),
        Err(PasswordsError::NotFound)
    );
}

#[test]
fn what_goes_in_the_bin_leaves_the_list_and_says_how_long_it_has() {
    let scratch = Scratch::new("throw-away");
    let state = unlocked(&scratch);
    let id = written(&state, "Banco");

    let summary = trash(&state, &id.to_string(), true, NOW_US).expect("it goes in the bin");

    let left = summary
        .trashed
        .expect("what is in the bin says how long it has");
    assert_eq!(left.days, 0);
    assert_eq!(left.days_left, 30);
    assert_eq!(left.trashed_at, NOW_US);

    assert!(
        list(&state, &EntryFilter::All, None, NOW_US)
            .expect("the list reads")
            .items
            .is_empty()
    );
    assert_eq!(
        list(&state, &EntryFilter::Trash, None, NOW_US)
            .expect("the bin reads")
            .items
            .len(),
        1
    );
}

#[test]
fn what_goes_in_the_bin_stops_being_findable_and_is_findable_again_when_it_comes_out() {
    let scratch = Scratch::new("index");
    let state = unlocked(&scratch);
    let id = written(&state, "Caja Rural");

    assert_eq!(search(&state, "rural", 0).expect("it runs").total, 1);

    let _binned = trash(&state, &id.to_string(), true, NOW_US).expect("it goes in the bin");
    assert_eq!(
        search(&state, "rural", 0).expect("it runs").total,
        0,
        "something thrown away is still findable by typing its name"
    );

    let back = trash(&state, &id.to_string(), false, NOW_US).expect("it comes back out");
    assert_eq!(back.trashed, None);
    assert_eq!(
        search(&state, "rural", 0).expect("it runs").total,
        1,
        "something taken back out of the bin cannot be found again"
    );
    assert_eq!(
        search(&state, "banco.es", 0).expect("it runs").total,
        1,
        "its addresses did not go back into the index with it"
    );
}

#[test]
fn something_thrown_away_comes_back_identical() {
    let scratch = Scratch::new("round-trip");
    let state = unlocked(&scratch);
    let id = written(&state, "Banco");

    let before = contents(&state, id).expect("it is there");

    let _binned = trash(&state, &id.to_string(), true, NOW_US).expect("it goes in the bin");
    assert_eq!(contents(&state, id), None, "the bin is not a hiding place");

    let _back = trash(&state, &id.to_string(), false, NOW_US).expect("it comes back out");

    assert_eq!(
        contents(&state, id).expect("it is there again"),
        before,
        "it came back from the bin changed"
    );

    let detail = get(&state, &id.to_string()).expect("the entry reads");
    assert_eq!(detail.urls.len(), 2);
    assert_eq!(detail.fields.len(), 2);
    assert!(detail.has_password);
}

#[test]
fn throwing_the_same_thing_away_twice_does_not_restart_its_thirty_days() {
    let scratch = Scratch::new("idempotent");
    let state = unlocked(&scratch);
    let id = written(&state, "Banco");

    let first = trash(&state, &id.to_string(), true, NOW_US).expect("it goes in the bin");
    let again = trash(&state, &id.to_string(), true, NOW_US + A_DAY_US).expect("nothing happens");

    assert_eq!(
        again.trashed.map(|left| left.trashed_at),
        first.trashed.map(|left| left.trashed_at),
        "clicking twice restarted the thirty days"
    );
    assert_eq!(again.trashed.map(|left| left.days_left), Some(29));

    // And the other direction is idempotent too.
    let _out = trash(&state, &id.to_string(), false, NOW_US).expect("it comes out");
    let out_again = trash(&state, &id.to_string(), false, NOW_US).expect("it is already out");
    assert_eq!(out_again.trashed, None);
}

#[test]
fn nothing_can_be_destroyed_without_going_through_the_bin_first() {
    let scratch = Scratch::new("not-in-trash");
    let state = unlocked(&scratch);
    let id = written(&state, "Banco");
    let before = contents(&state, id).expect("it is there");

    assert_eq!(
        delete(&state, &id.to_string(), NOW_US),
        Err(PasswordsError::NotInTrash)
    );
    assert_eq!(
        contents(&state, id).expect("it is still there"),
        before,
        "a refused deletion took something with it"
    );
}

#[test]
fn destroying_something_in_the_bin_leaves_a_skeleton_and_nothing_else() {
    let scratch = Scratch::new("destroy");
    let state = unlocked(&scratch);
    let id = written(&state, "Banco");

    let _binned = trash(&state, &id.to_string(), true, NOW_US).expect("it goes in the bin");
    delete(&state, &id.to_string(), NOW_US).expect("what is in the bin can be destroyed");

    assert_eq!(
        trash(&state, &id.to_string(), false, NOW_US),
        Err(PasswordsError::NotFound),
        "something destroyed can still be fetched back"
    );
    assert_eq!(
        list(&state, &EntryFilter::Trash, None, NOW_US)
            .expect("the bin reads")
            .items
            .len(),
        0
    );
    assert_eq!(search(&state, "banco", 0).expect("it runs").total, 0);

    // The row stays, because a deletion that removed it would never reach the other device. What
    // must not stay is anything it said: not the title, not the addresses, not the fields, and
    // not one of the old passwords.
    assert_eq!(row_census(&state, id), (1, 0));
    assert_eq!(
        child_census(
            &state,
            "SELECT count(*), count(value) FROM vault_urls WHERE entry_id = ?1",
            id
        )
        .1,
        0
    );
    assert_eq!(
        child_census(
            &state,
            "SELECT count(*), count(label) + count(value) FROM vault_fields WHERE entry_id = ?1",
            id
        )
        .1,
        0
    );
    assert_eq!(
        child_census(
            &state,
            "SELECT count(*), count(password) FROM vault_password_history WHERE entry_id = ?1",
            id
        )
        .1,
        0
    );
}

#[test]
fn emptying_the_bin_takes_what_is_in_it_and_nothing_else() {
    let scratch = Scratch::new("empty");
    let state = unlocked(&scratch);

    let inside: Vec<Uuid> = (0..3)
        .map(|number| written(&state, &format!("Dentro {number}")))
        .collect();
    let outside: Vec<Uuid> = (0..2)
        .map(|number| written(&state, &format!("Fuera {number}")))
        .collect();
    for id in &inside {
        let _binned = trash(&state, &id.to_string(), true, NOW_US).expect("it goes in the bin");
    }

    let emptied = empty_trash(&state, NOW_US).expect("the bin is emptied");

    assert_eq!(emptied.entries, 3);
    for id in &inside {
        assert_eq!(row_census(&state, *id), (1, 0));
    }
    for id in &outside {
        assert!(
            contents(&state, *id).is_some(),
            "emptying the bin took something that was not in it"
        );
    }

    assert_eq!(
        empty_trash(&state, NOW_US)
            .expect("an empty bin is not an error")
            .entries,
        0
    );
}

#[test]
fn opening_the_vault_destroys_what_has_run_out_and_leaves_what_has_not() {
    let scratch = Scratch::new("sweep");
    let state = unlocked(&scratch);

    let expired = written(&state, "Caducada");
    let recent = written(&state, "Reciente");
    for id in [expired, recent] {
        let _binned = trash(&state, &id.to_string(), true, NOW_US).expect("it goes in the bin");
    }
    antedate(&state, expired, NOW_US - 31 * A_DAY_US);
    antedate(&state, recent, NOW_US - 29 * A_DAY_US);

    reopened(&state, NOW_US);

    assert_eq!(
        row_census(&state, expired),
        (1, 0),
        "what had run out of its thirty days survived the unlock"
    );
    let still_there = list(&state, &EntryFilter::Trash, None, NOW_US).expect("the bin reads");
    assert_eq!(still_there.items.len(), 1);
    assert_eq!(
        still_there.items.first().map(|one| one.id.as_str()),
        Some(recent.to_string().as_str())
    );
}

#[test]
fn a_sweep_that_cannot_run_does_not_keep_the_vault_shut() {
    let scratch = Scratch::new("sweep-fails");
    let state = unlocked(&scratch);
    let intact = written(&state, "Intacta");

    // Something the sweep would destroy if it ran at all. It is here so the assertion at the end
    // proves the sweep really did fail, rather than the damaged row having been quietly skipped.
    let doomed = written(&state, "Caducada");
    let _binned = trash(&state, &doomed.to_string(), true, NOW_US).expect("it goes in the bin");
    antedate(&state, doomed, NOW_US - 31 * A_DAY_US);

    // A row the sweep will choke on: its identifier is not sixteen bytes, so reading it back out
    // fails before anything can be destroyed. The column has a check constraint that normally
    // makes this impossible, which is why it is switched off for the one statement that writes
    // it. Nothing in the application can produce this row; what it stands for is a file damaged
    // by something outside this application, and the property being proved is that such a file
    // still opens.
    in_database(&state, |storage, _codec, connection| {
        connection.execute_batch("PRAGMA ignore_check_constraints = ON")?;
        connection.execute(
            "INSERT INTO vault_entries
                 (id, created_at, updated_at, device_id, deleted, hlc, rev, favorite,
                  kind, trashed_at)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, 0, 0, 0, ?5)",
            (
                [9_u8; 3].as_slice(),
                NOW_US,
                storage.device().as_bytes().as_slice(),
                [8_u8; 16].as_slice(),
                NOW_US - 31 * A_DAY_US,
            ),
        )?;
        connection.execute_batch("PRAGMA ignore_check_constraints = OFF")?;
        Ok(())
    })
    .expect("a damaged row can be written for this test");

    reopened(&state, NOW_US);

    assert!(
        contents(&state, intact).is_some(),
        "a sweep that could not run took the vault with it"
    );
    assert_eq!(
        row_census(&state, doomed),
        (0, 4),
        "the sweep ran after all, so this test is not proving what it says it proves"
    );
}
