//! Drives the one command in this application through which a secret leaves the core.
//!
//! Everything else in the passwords module exists so that this is the only door, so this file is
//! about how narrow it is rather than about whether it opens. A value is asked for by naming one
//! entry and one closed target; a field that belongs to somebody else is refused, and refused by
//! the query rather than after decrypting it; what is in the bin is not consulted; and the only
//! trace left behind is the moment on the entry, with nothing written to the audit table.
//!
//! The last one is asserted by counting `audit_events` before and after. A record of when each
//! password is looked at is a record of somebody's own habits kept inside their own vault, and
//! the way that arrives is by somebody adding one line to a command that already had a
//! connection in its hand.
// Every function in an integration test file is test code, but the lint that forbids panicking
// constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]` functions. The
// helpers below are neither, and a helper that cannot panic would have to return a Result that
// every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES};
use cairn_db::repositories::vault::{self as repository, MAX_HISTORY};
use cairn_db::{Connection, DbError, FieldCodec};
use cairn_lib::commands::passwords::{
    DraftFieldDto, EntryDraftDto, EntryKindDto, PasswordsError, RevealTarget, create, delete, get,
    history, reveal, trash, update,
};
use cairn_lib::commands::vault::create as create_vault;
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

/// The value of the field that is not secret, which may also be revealed.
const A_PLAIN_VALUE: &str = "Central";

/// A moment in the middle of the range, so the arithmetic either side of it is ordinary.
const NOW_US: i64 = 1_700_000_000_000_000;

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
                "cairn-passwords-reveal-{name}-{}-{unique}",
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

/// A draft with a password, a plain field and a secret one.
fn furnished(title: &str) -> EntryDraftDto {
    EntryDraftDto {
        kind: EntryKindDto::Account,
        title: title.to_owned(),
        username: Some("alguien@ejemplo".to_owned()),
        password: Some(A_WRITTEN_PASSWORD.to_owned()),
        notes: None,
        urls: vec!["banco.example".to_owned()],
        fields: vec![
            DraftFieldDto {
                label: "Oficina".to_owned(),
                value: Some(A_PLAIN_VALUE.to_owned()),
                secret: false,
            },
            DraftFieldDto {
                label: "PIN".to_owned(),
                value: Some(A_SECRET_VALUE.to_owned()),
                secret: true,
            },
        ],
        folder_id: None,
        favorite: false,
    }
}

/// A note with no password at all.
fn a_note(title: &str) -> EntryDraftDto {
    EntryDraftDto {
        kind: EntryKindDto::Note,
        title: title.to_owned(),
        username: None,
        password: None,
        notes: Some("sólo texto".to_owned()),
        urls: Vec::new(),
        fields: Vec::new(),
        folder_id: None,
        favorite: false,
    }
}

/// Writes a draft and answers with its identifier.
fn written(state: &AppState, draft: &EntryDraftDto) -> Uuid {
    let detail = create(state, draft, NOW_US).expect("a good draft is written");
    Uuid::parse_str(&detail.summary.id).expect("what came back is an identifier")
}

/// The identifier of the field in that position of an entry.
fn field_at(state: &AppState, id: Uuid, position: usize) -> String {
    get(state, &id.to_string())
        .expect("the entry reads")
        .fields
        .get(position)
        .expect("the entry has that field")
        .id
        .clone()
}

/// When an entry was last looked at, read straight out of the file.
fn last_used_at(state: &AppState, id: Uuid) -> Option<i64> {
    in_database(state, |_storage, codec, connection| {
        Ok(repository::any_entry(connection, codec, id)?.and_then(|entry| entry.last_used_at))
    })
    .expect("the entry can be read")
}

/// How many rows the audit table holds.
fn audit_rows(state: &AppState) -> i64 {
    in_database(state, |_storage, _codec, connection| {
        Ok(connection.query_row("SELECT count(*) FROM audit_events", [], |row| row.get(0))?)
    })
    .expect("the rows can be counted")
}

#[test]
fn it_refuses_while_the_vault_is_locked() {
    let scratch = Scratch::new("locked");
    let state = scratch.state();

    assert_eq!(
        reveal(
            &state,
            &Uuid::from_bytes([1; 16]).to_string(),
            &RevealTarget::Password,
            NOW_US
        ),
        Err(PasswordsError::Locked)
    );
}

#[test]
fn the_password_of_an_entry_comes_back_with_how_long_to_show_it() {
    let scratch = Scratch::new("password");
    let state = unlocked(&scratch);
    let id = written(&state, &furnished("Banco"));

    let shown = reveal(&state, &id.to_string(), &RevealTarget::Password, NOW_US)
        .expect("the password is revealed");

    assert_eq!(shown.value, A_WRITTEN_PASSWORD);
    assert_eq!(shown.hide_after_s, 20);

    // Twice in a row. There is no state that gets spent, no token and no single use.
    let again = reveal(&state, &id.to_string(), &RevealTarget::Password, NOW_US)
        .expect("it can be revealed again");
    assert_eq!(again.value, A_WRITTEN_PASSWORD);
}

#[test]
fn an_entry_with_no_password_is_refused_rather_than_answered_with_nothing() {
    let scratch = Scratch::new("no-password");
    let state = unlocked(&scratch);
    let id = written(&state, &a_note("Una nota"));

    assert_eq!(
        reveal(&state, &id.to_string(), &RevealTarget::Password, NOW_US),
        Err(PasswordsError::NotFound),
        "an empty string is a value and a screen would draw it as one"
    );
}

#[test]
fn both_kinds_of_custom_field_can_be_revealed() {
    let scratch = Scratch::new("fields");
    let state = unlocked(&scratch);
    let id = written(&state, &furnished("Banco"));

    let plain = field_at(&state, id, 0);
    let secret = field_at(&state, id, 1);

    assert_eq!(
        reveal(
            &state,
            &id.to_string(),
            &RevealTarget::Field { id: secret },
            NOW_US
        )
        .expect("the secret field is revealed")
        .value,
        A_SECRET_VALUE
    );

    // Revealing one that is not secret is allowed, because its value already travelled in `get`.
    assert_eq!(
        reveal(
            &state,
            &id.to_string(),
            &RevealTarget::Field { id: plain },
            NOW_US
        )
        .expect("the plain field is revealed")
        .value,
        A_PLAIN_VALUE
    );
}

#[test]
fn a_field_of_another_entry_is_not_reachable_by_naming_this_one() {
    let scratch = Scratch::new("someone-elses-field");
    let state = unlocked(&scratch);
    let mine = written(&state, &furnished("Banco"));
    let theirs = written(&state, &furnished("Correo"));

    let not_mine = field_at(&state, theirs, 1);

    assert_eq!(
        reveal(
            &state,
            &mine.to_string(),
            &RevealTarget::Field { id: not_mine },
            NOW_US
        ),
        Err(PasswordsError::NotFound)
    );
    assert_eq!(
        reveal(
            &state,
            &mine.to_string(),
            &RevealTarget::Field {
                id: Uuid::from_bytes([5; 16]).to_string()
            },
            NOW_US
        ),
        Err(PasswordsError::NotFound)
    );
    assert_eq!(
        reveal(
            &state,
            &mine.to_string(),
            &RevealTarget::Field {
                id: "no soy un identificador".to_owned()
            },
            NOW_US
        ),
        Err(PasswordsError::NotFound)
    );
}

#[test]
fn an_old_password_comes_back_and_one_of_another_entry_does_not() {
    let scratch = Scratch::new("history");
    let state = unlocked(&scratch);
    let mine = written(&state, &furnished("Banco"));
    let theirs = written(&state, &furnished("Correo"));

    for id in [mine, theirs] {
        let _saved = update(
            &state,
            &id.to_string(),
            &EntryDraftDto {
                password: Some("la nueva".to_owned()),
                ..furnished("Igual")
            },
            NOW_US,
        )
        .expect("the entry is saved");
    }

    let mine_moment = history(&state, &mine.to_string())
        .expect("the history reads")
        .first()
        .expect("there is one")
        .id
        .clone();
    let theirs_moment = history(&state, &theirs.to_string())
        .expect("the history reads")
        .first()
        .expect("there is one")
        .id
        .clone();

    assert_eq!(
        reveal(
            &state,
            &mine.to_string(),
            &RevealTarget::HistoryEntry { id: mine_moment },
            NOW_US
        )
        .expect("the old password is revealed")
        .value,
        A_WRITTEN_PASSWORD
    );
    assert_eq!(
        reveal(
            &state,
            &mine.to_string(),
            &RevealTarget::HistoryEntry { id: theirs_moment },
            NOW_US
        ),
        Err(PasswordsError::NotFound)
    );
}

#[test]
fn a_moment_the_cap_trimmed_holds_nothing_and_says_so() {
    let scratch = Scratch::new("history-trimmed");
    let state = unlocked(&scratch);
    let id = written(&state, &furnished("Banco"));

    // Each replacement at its own moment, because the cap keeps the newest and a row's moment is
    // what makes one newer than another. Eleven replacements at the same microsecond would be
    // eleven rows the cap has to break a tie between.
    let replace = |number: usize| {
        let _saved = update(
            &state,
            &id.to_string(),
            &EntryDraftDto {
                password: Some(format!("la número {number}")),
                ..furnished("Banco")
            },
            NOW_US + i64::try_from(number).unwrap_or(0),
        )
        .expect("the entry is saved");
    };

    // The first replacement is the row that will fall off the end, and it is noted here while it
    // is still the only one there is.
    replace(0);
    let oldest = history(&state, &id.to_string())
        .expect("the history reads")
        .first()
        .expect("there is one by now")
        .id
        .clone();

    // One more than the cap fits, so that one is trimmed and keeps no ciphertext.
    for number in 1..=MAX_HISTORY {
        replace(number);
    }

    assert_eq!(
        reveal(
            &state,
            &id.to_string(),
            &RevealTarget::HistoryEntry { id: oldest },
            NOW_US
        ),
        Err(PasswordsError::NotFound),
        "a row the cap trimmed still handed something over"
    );
}

#[test]
fn nothing_of_something_in_the_bin_is_revealed() {
    let scratch = Scratch::new("binned");
    let state = unlocked(&scratch);
    let id = written(&state, &furnished("Banco"));
    let secret = field_at(&state, id, 1);

    let _saved = update(
        &state,
        &id.to_string(),
        &EntryDraftDto {
            password: Some("la nueva".to_owned()),
            ..furnished("Banco")
        },
        NOW_US,
    )
    .expect("the entry is saved");
    let moment = history(&state, &id.to_string())
        .expect("the history reads")
        .first()
        .expect("there is one")
        .id
        .clone();

    let _binned = trash(&state, &id.to_string(), true, NOW_US).expect("it goes in the bin");

    for target in [
        RevealTarget::Password,
        RevealTarget::Field { id: secret },
        RevealTarget::HistoryEntry { id: moment },
    ] {
        assert_eq!(
            reveal(&state, &id.to_string(), &target, NOW_US),
            Err(PasswordsError::NotFound),
            "what was thrown away answered {target:?}"
        );
    }
}

#[test]
fn nothing_of_something_destroyed_is_revealed() {
    let scratch = Scratch::new("destroyed");
    let state = unlocked(&scratch);
    let id = written(&state, &furnished("Banco"));

    let _binned = trash(&state, &id.to_string(), true, NOW_US).expect("it goes in the bin");
    delete(&state, &id.to_string(), NOW_US).expect("it is destroyed");

    assert_eq!(
        reveal(&state, &id.to_string(), &RevealTarget::Password, NOW_US),
        Err(PasswordsError::NotFound)
    );
}

#[test]
fn looking_at_a_value_is_written_down_once_and_looking_at_an_old_one_is_not() {
    let scratch = Scratch::new("last-used");
    let state = unlocked(&scratch);
    let id = written(&state, &furnished("Banco"));

    assert_eq!(last_used_at(&state, id), None);

    let _shown = reveal(&state, &id.to_string(), &RevealTarget::Password, NOW_US)
        .expect("the password is revealed");
    assert_eq!(last_used_at(&state, id), Some(NOW_US));

    let _saved = update(
        &state,
        &id.to_string(),
        &EntryDraftDto {
            password: Some("la nueva".to_owned()),
            ..furnished("Banco")
        },
        NOW_US,
    )
    .expect("the entry is saved");
    let moment = history(&state, &id.to_string())
        .expect("the history reads")
        .first()
        .expect("there is one")
        .id
        .clone();

    let _old = reveal(
        &state,
        &id.to_string(),
        &RevealTarget::HistoryEntry { id: moment },
        NOW_US + 1_000,
    )
    .expect("the old password is revealed");

    assert_eq!(
        last_used_at(&state, id),
        Some(NOW_US),
        "looking at what a password used to be counted as using the entry"
    );
}

#[test]
fn revealing_writes_nothing_to_the_audit_table() {
    let scratch = Scratch::new("no-audit");
    let state = unlocked(&scratch);
    let id = written(&state, &furnished("Banco"));
    let secret = field_at(&state, id, 1);

    let before = audit_rows(&state);

    let _password = reveal(&state, &id.to_string(), &RevealTarget::Password, NOW_US)
        .expect("the password is revealed");
    let _field = reveal(
        &state,
        &id.to_string(),
        &RevealTarget::Field { id: secret },
        NOW_US,
    )
    .expect("the field is revealed");

    assert_eq!(
        audit_rows(&state),
        before,
        "a record of when each password is looked at is a record of somebody's own habits"
    );
}
