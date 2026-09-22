//! Drives the six reading commands of the passwords module against a real vault and a real file.
//!
//! What the unit tests below this cover is what a draft is, what the bin is and how the index
//! matches. What this covers is the one property the whole module exists for, and it is asserted
//! against the **text of the serialised answer** rather than against the types: no answer of any
//! of these six contains a password or the value of a secret field. The type is exactly what
//! somebody adds a field to a year from now, so the assertion is made against what actually
//! crosses the bridge.
// Every function in an integration test file is test code, but the lint that forbids panicking
// constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]` functions. The
// helpers below are neither, and a helper that cannot panic would have to return a Result that
// every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES};
use cairn_db::repositories::settings;
use cairn_db::repositories::vault::{self as repository, NewEntry, NewField};
use cairn_db::{Connection, DbError, FieldCodec};
use cairn_domain::Hlc;
use cairn_domain::vault::EntryKind;
use cairn_lib::commands::passwords::{
    CLIPBOARD_SECONDS_KEY, EntryFilter, PasswordsError, folders, get, history, list, search,
    settings as passwords_settings,
};
use cairn_lib::commands::vault::create as create_vault;
use cairn_lib::state::AppState;
use cairn_lib::storage::{DataDirectory, Storage};
use cairn_lib::vault::Vault;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Not a real password. A phrase invented for these tests, in a directory removed afterwards.
const NOT_A_REAL_PASSWORD: &str = "una frase larga inventada para la prueba";

/// The password of the entries these tests write, and the needle every leak check looks for.
const A_WRITTEN_PASSWORD: &str = "cairn-canary-password";

/// The value of the secret custom field, and the second needle.
const A_SECRET_VALUE: &str = "cairn-canary-secret";

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
                "cairn-passwords-{name}-{}-{unique}",
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

/// Runs something against the open database, for the rows these commands cannot write yet.
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

/// A clock reading for a write these tests make by hand.
fn at(step: u64) -> Hlc {
    Hlc::new(step, 0, [3; 6])
}

/// Writes one furnished entry straight into the file, and answers with its identifier.
///
/// Through the repository rather than through a command, because the writing commands are the
/// next task. What matters here is that the rows exist and that they hold a password and a secret
/// value, so that the assertions about what never crosses the bridge have something to look for.
fn write_entry(state: &AppState, step: u64, title: &str, kind: EntryKind) -> Uuid {
    in_database(state, |storage, codec, connection| {
        let written = repository::create_entry(
            connection,
            codec,
            storage.device(),
            at(step),
            NOW_US,
            NewEntry {
                title,
                username: Some("alguien@ejemplo"),
                password: Some(A_WRITTEN_PASSWORD),
                notes: Some("una nota"),
                folder_id: None,
                favorite: false,
            },
            kind,
        )?;

        repository::replace_urls(
            connection,
            codec,
            storage.device(),
            at(step + 1),
            NOW_US,
            written.id,
            &["https://banco.es/login", "banco.example"],
        )?;
        repository::replace_fields(
            connection,
            codec,
            storage.device(),
            at(step + 2),
            NOW_US,
            written.id,
            &[
                NewField {
                    label: "Oficina",
                    value: "Central",
                    secret: false,
                },
                NewField {
                    label: "PIN",
                    value: A_SECRET_VALUE,
                    secret: true,
                },
            ],
        )?;

        Ok(written.id)
    })
    .expect("the entry is written")
}

/// The serialised form of an answer, for the assertions about what crosses the bridge.
fn as_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("the answer serialises")
}

/// Fails if the serialised answer carries either of the two things it must never carry.
fn carries_no_secret(what: &str, json: &str) {
    for needle in [A_WRITTEN_PASSWORD, A_SECRET_VALUE] {
        assert!(
            !json.contains(needle),
            "{what} carried {needle} across the bridge: {json}"
        );
    }
}

#[test]
fn every_reading_command_refuses_a_locked_vault() {
    let scratch = Scratch::new("locked");
    let state = scratch.state();

    assert_eq!(
        list(&state, &EntryFilter::All, None, NOW_US),
        Err(PasswordsError::Locked)
    );
    assert_eq!(search(&state, "banco", 0), Err(PasswordsError::Locked));
    assert_eq!(
        get(&state, &Uuid::from_bytes([1; 16]).to_string()),
        Err(PasswordsError::Locked)
    );
    assert_eq!(
        history(&state, &Uuid::from_bytes([1; 16]).to_string()),
        Err(PasswordsError::Locked)
    );
    assert_eq!(folders(&state), Err(PasswordsError::Locked));
    assert_eq!(passwords_settings(&state), Err(PasswordsError::Locked));
}

#[test]
fn a_list_leaves_out_what_is_in_the_bin_and_the_bin_shows_only_that() {
    let scratch = Scratch::new("list-bin");
    let state = unlocked(&scratch);

    write_entry(&state, 1, "Banco", EntryKind::Account);
    write_entry(&state, 10, "Correo", EntryKind::Account);
    let binned = write_entry(&state, 20, "Vieja", EntryKind::Note);

    in_database(&state, |_storage, codec, connection| {
        repository::set_trashed(connection, codec, at(50), NOW_US, binned, true)
    })
    .expect("the entry goes in the bin");

    let live = list(&state, &EntryFilter::All, None, NOW_US).expect("the list reads");
    assert_eq!(live.items.len(), 2);
    assert!(live.items.iter().all(|one| one.trashed.is_none()));

    let bin = list(&state, &EntryFilter::Trash, None, NOW_US).expect("the bin reads");
    assert_eq!(bin.items.len(), 1);
    assert_eq!(
        bin.items.first().map(|one| one.title.as_str()),
        Some("Vieja")
    );
    let left = bin
        .items
        .first()
        .and_then(|one| one.trashed)
        .expect("something in the bin says how long it has");
    assert_eq!(left.days_left, 30);

    carries_no_secret("a list", &as_json(&live));
    carries_no_secret("the bin", &as_json(&bin));
}

#[test]
fn a_folder_that_is_not_there_is_an_empty_list_rather_than_a_refusal() {
    let scratch = Scratch::new("list-folder");
    let state = unlocked(&scratch);
    write_entry(&state, 1, "Banco", EntryKind::Account);

    let missing = list(
        &state,
        &EntryFilter::Folder {
            id: Uuid::from_bytes([9; 16]).to_string(),
        },
        None,
        NOW_US,
    )
    .expect("a folder that is not there is not an error");
    assert!(missing.items.is_empty());

    let nonsense = list(
        &state,
        &EntryFilter::Folder {
            id: "no soy un identificador".to_owned(),
        },
        None,
        NOW_US,
    )
    .expect("a folder identifier that is not one is not an error either");
    assert!(nonsense.items.is_empty());
}

#[test]
fn only_what_somebody_marked_comes_back_from_the_favourites() {
    let scratch = Scratch::new("list-favorites");
    let state = unlocked(&scratch);
    write_entry(&state, 1, "Banco", EntryKind::Account);
    let marked = write_entry(&state, 10, "Correo", EntryKind::Account);

    in_database(&state, |_storage, _codec, connection| {
        connection
            .execute(
                "UPDATE vault_entries SET favorite = 1 WHERE id = ?1",
                [marked.as_bytes().as_slice()],
            )
            .map(|_rows| ())
            .map_err(DbError::from)
    })
    .expect("the entry can be marked");

    let expected = marked.to_string();
    let found = list(&state, &EntryFilter::Favorites, None, NOW_US).expect("the list reads");
    assert_eq!(found.items.len(), 1);
    assert_eq!(
        found.items.first().map(|one| one.id.as_str()),
        Some(expected.as_str())
    );
}

#[test]
fn an_entry_comes_back_whole_and_its_secret_field_comes_back_empty() {
    let scratch = Scratch::new("get");
    let state = unlocked(&scratch);
    let id = write_entry(&state, 1, "Banco", EntryKind::Account);

    let detail = get(&state, &id.to_string()).expect("the entry reads");

    assert_eq!(detail.summary.title, "Banco");
    assert_eq!(detail.notes.as_deref(), Some("una nota"));
    assert_eq!(detail.urls.len(), 2);
    assert_eq!(detail.fields.len(), 2);
    assert!(
        detail.has_password,
        "the entry has one and the screen is not told"
    );
    assert_eq!(detail.history_len, 0);

    let plain = detail.fields.first().expect("two fields");
    assert!(!plain.secret);
    assert_eq!(plain.value.as_deref(), Some("Central"));

    let secret = detail.fields.get(1).expect("two fields");
    assert!(secret.secret);
    assert_eq!(secret.label, "PIN");
    assert_eq!(
        secret.value, None,
        "the value of a secret field crossed the bridge"
    );

    carries_no_secret("an entry", &as_json(&detail));
}

#[test]
fn an_entry_in_the_bin_is_not_readable_and_neither_is_its_history() {
    let scratch = Scratch::new("get-bin");
    let state = unlocked(&scratch);
    let id = write_entry(&state, 1, "Banco", EntryKind::Account);

    in_database(&state, |_storage, codec, connection| {
        repository::set_trashed(connection, codec, at(50), NOW_US, id, true)
    })
    .expect("the entry goes in the bin");

    assert_eq!(get(&state, &id.to_string()), Err(PasswordsError::NotFound));
    assert_eq!(
        history(&state, &id.to_string()),
        Err(PasswordsError::NotFound)
    );
}

#[test]
fn something_that_is_not_an_identifier_is_the_same_answer_as_something_that_is_not_there() {
    let scratch = Scratch::new("get-nonsense");
    let state = unlocked(&scratch);

    assert_eq!(
        get(&state, "no soy un identificador"),
        Err(PasswordsError::NotFound)
    );
    assert_eq!(history(&state, "tampoco"), Err(PasswordsError::NotFound));
}

#[test]
fn an_entry_with_every_address_and_every_field_it_may_have_comes_back_in_order() {
    let scratch = Scratch::new("get-full");
    let state = unlocked(&scratch);

    let addresses: Vec<String> = (0..32).map(|number| format!("sitio{number}.es")).collect();
    let labels: Vec<String> = (0..256).map(|number| format!("campo {number}")).collect();

    let id = in_database(&state, |storage, codec, connection| {
        let written = repository::create_entry(
            connection,
            codec,
            storage.device(),
            at(1),
            NOW_US,
            NewEntry {
                title: "Completa",
                username: None,
                password: None,
                notes: None,
                folder_id: None,
                favorite: false,
            },
            EntryKind::Account,
        )?;

        let borrowed: Vec<&str> = addresses.iter().map(String::as_str).collect();
        repository::replace_urls(
            connection,
            codec,
            storage.device(),
            at(2),
            NOW_US,
            written.id,
            &borrowed,
        )?;

        let fields: Vec<NewField<'_>> = labels
            .iter()
            .map(|label| NewField {
                label: label.as_str(),
                value: "x",
                secret: false,
            })
            .collect();
        repository::replace_fields(
            connection,
            codec,
            storage.device(),
            at(3),
            NOW_US,
            written.id,
            &fields,
        )?;

        Ok(written.id)
    })
    .expect("the entry is written");

    let detail = get(&state, &id.to_string()).expect("the entry reads");

    assert_eq!(detail.urls.len(), 32);
    assert_eq!(detail.fields.len(), 256);
    assert!(!detail.has_password);
    assert_eq!(
        detail.urls.first().map(|url| url.value.as_str()),
        Some("sitio0.es")
    );
    assert_eq!(
        detail.fields.last().map(|field| field.label.as_str()),
        Some("campo 255")
    );
}

#[test]
fn the_history_is_a_list_of_moments_and_carries_no_password() {
    let scratch = Scratch::new("history");
    let state = unlocked(&scratch);
    let id = write_entry(&state, 1, "Banco", EntryKind::Account);

    in_database(&state, |storage, codec, connection| {
        for step in 1..=3_u64 {
            repository::replace_password(
                connection,
                codec,
                storage.device(),
                at(100 + step),
                NOW_US + i64::try_from(step).unwrap_or(0),
                id,
                &format!("cairn-canary-password-{step}"),
            )?;
        }
        Ok(())
    })
    .expect("the password is replaced three times");

    let moments = history(&state, &id.to_string()).expect("the history reads");

    assert_eq!(moments.len(), 3);
    let json = as_json(&moments);
    carries_no_secret("the history", &json);
    for step in 1..=3 {
        assert!(
            !json.contains(&format!("cairn-canary-password-{step}")),
            "the history carried an old password: {json}"
        );
    }

    assert_eq!(
        get(&state, &id.to_string())
            .expect("the entry reads")
            .history_len,
        3
    );
}

#[test]
fn a_search_pages_through_what_it_found_and_says_how_many_there_were() {
    let scratch = Scratch::new("search");
    let state = unlocked(&scratch);

    for number in 0..60_u64 {
        write_entry(
            &state,
            number * 10 + 1,
            &format!("Cuenta {number}"),
            EntryKind::Account,
        );
    }

    // The index is built on the unlock, and these rows were written after it, so it is rebuilt
    // here the way a write will rebuild it once the writing commands exist.
    state
        .session()
        .with_open(|vault, storage| storage.rebuild_titles(vault))
        .expect("the vault is open")
        .expect("the index rebuilds");

    let first = search(&state, "cuenta", 0).expect("the search runs");
    let second = search(&state, "cuenta", 1).expect("the search runs");

    assert_eq!(first.hits.len(), 50);
    assert_eq!(second.hits.len(), 10);
    assert_eq!(first.total, 60);
    assert_eq!(second.total, 60);
    assert!(first.complete);

    carries_no_secret("a search", &as_json(&first));
}

#[test]
fn the_index_is_built_by_the_unlock_and_emptied_by_the_lock() {
    let scratch = Scratch::new("index-lifecycle");
    let state = unlocked(&scratch);
    write_entry(&state, 1, "Banco Santander", EntryKind::Account);

    state
        .session()
        .with_open(|vault, storage| storage.rebuild_titles(vault))
        .expect("the vault is open")
        .expect("the index rebuilds");
    assert_eq!(
        search(&state, "santander", 0)
            .expect("the search runs")
            .total,
        1
    );

    assert!(state.session().lock(), "the vault was open");
    assert_eq!(search(&state, "santander", 0), Err(PasswordsError::Locked));

    // Opened again, over the same file. The index is rebuilt by the unlock itself, with nothing
    // else asked for, which is the property this test exists for.
    let reopened = tauri::async_runtime::block_on(cairn_lib::commands::vault::unlock(
        &state,
        Zeroizing::new(NOT_A_REAL_PASSWORD.to_owned()),
        NOW_US + 1,
    ))
    .expect("the vault reopens");
    assert!(reopened.unlocked);

    assert_eq!(
        search(&state, "santander", 0)
            .expect("the search runs")
            .total,
        1,
        "the unlock did not build the index"
    );
}

#[test]
fn the_settings_say_fifteen_seconds_until_somebody_says_otherwise_and_admit_what_is_not_hardened() {
    let scratch = Scratch::new("settings");
    let state = unlocked(&scratch);

    let before = passwords_settings(&state).expect("the settings read");
    assert_eq!(before.clipboard_clear_s, 15);
    assert_eq!(before.reveal_s, 20);
    assert_eq!(before.max_history, 10);
    assert!(
        !before.clipboard_hardened,
        "the clipboard is not hardened until phase 08 and must not claim to be"
    );
    assert!(!before.screen_hardened);

    in_database(&state, |storage, codec, connection| {
        settings::put(
            connection,
            codec,
            storage.device(),
            at(9),
            NOW_US,
            CLIPBOARD_SECONDS_KEY,
            Some(b"42"),
        )
        .map(|_written| ())
    })
    .expect("the preference is written");

    assert_eq!(
        passwords_settings(&state)
            .expect("the settings read")
            .clipboard_clear_s,
        42
    );
}

#[test]
fn a_folder_comes_back_with_what_is_in_it() {
    let scratch = Scratch::new("folders");
    let state = unlocked(&scratch);

    assert!(
        folders(&state)
            .expect("an empty list is not an error")
            .is_empty()
    );

    let folder = in_database(&state, |storage, codec, connection| {
        repository::save_folder(
            connection,
            codec,
            storage.device(),
            at(1),
            NOW_US,
            None,
            "Bancos",
        )
    })
    .expect("the folder is written");

    let id = write_entry(&state, 10, "Banco", EntryKind::Account);
    in_database(&state, |_storage, _codec, connection| {
        connection
            .execute(
                "UPDATE vault_entries SET folder_id = ?2 WHERE id = ?1",
                [id.as_bytes().as_slice(), folder.id.as_bytes().as_slice()],
            )
            .map(|_rows| ())
            .map_err(DbError::from)
    })
    .expect("the entry can be filed");

    let listed = folders(&state).expect("the folders read");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed.first().map(|one| one.name.as_str()), Some("Bancos"));
    assert_eq!(listed.first().map(|one| one.entries), Some(1));

    let filed = list(
        &state,
        &EntryFilter::Folder {
            id: folder.id.to_string(),
        },
        None,
        NOW_US,
    )
    .expect("the folder reads");
    assert_eq!(filed.items.len(), 1);
}
