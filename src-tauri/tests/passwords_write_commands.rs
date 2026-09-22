//! Drives the six writing commands of the passwords module against a real vault and a real file.
//!
//! Three properties are what this file is for, and each of them is invisible from any single
//! crate. An entry is written whole or not at all, so a refusal halfway through leaves the file
//! exactly as it was. The password is only pushed to the history when it actually changed, so
//! saving the same form ten times does not empty the ten entries before it. And a secret field
//! whose value never crossed the bridge comes back intact after a save, which is the other half
//! of the rule that stopped it crossing in the first place.
//!
//! As in the reading tests, what must never travel is asserted against the **text** of the
//! serialised answer rather than against the types.
// Every function in an integration test file is test code, but the lint that forbids panicking
// constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]` functions. The
// helpers below are neither, and a helper that cannot panic would have to return a Result that
// every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES};
use cairn_db::repositories::vault::{self as repository, NewEntry};
use cairn_db::{Connection, DbError, FieldCodec};
use cairn_domain::Hlc;
use cairn_domain::vault::EntryKind;
use cairn_lib::commands::passwords::{
    DraftFieldDto, EntryDraftDto, EntryFilter, EntryKindDto, HistoryScopeDto, PasswordsError,
    create, folder_delete, folder_save, folders, folders_reorder, get, history, history_clear,
    list, search, update,
};
use cairn_lib::commands::vault::create as create_vault;
use cairn_lib::state::AppState;
use cairn_lib::storage::{DataDirectory, Storage};
use cairn_lib::vault::Vault;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Not a real password. A phrase invented for these tests, in a directory removed afterwards.
const NOT_A_REAL_PASSWORD: &str = "una frase larga inventada para la prueba";

/// The password these drafts carry, and one of the two needles every leak check looks for.
const A_WRITTEN_PASSWORD: &str = "cairn-canary-password";

/// The value of the secret custom field, and the other needle.
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
                "cairn-passwords-write-{name}-{}-{unique}",
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

/// The password an entry actually holds, read straight out of the file.
fn stored_password(state: &AppState, id: Uuid) -> Option<String> {
    in_database(state, |_storage, codec, connection| {
        Ok(repository::entry(connection, codec, id)?
            .and_then(|entry| entry.password)
            .map(|password| password.to_string()))
    })
    .expect("the entry can be read")
}

/// The custom fields an entry actually holds, as label and value.
fn stored_fields(state: &AppState, id: Uuid) -> Vec<(String, String)> {
    in_database(state, |_storage, codec, connection| {
        Ok(repository::fields(connection, codec, id)?
            .iter()
            .map(|field| (field.label.to_string(), field.value.to_string()))
            .collect())
    })
    .expect("the fields can be read")
}

/// The simplest acceptable draft there is.
fn draft(title: &str) -> EntryDraftDto {
    EntryDraftDto {
        kind: EntryKindDto::Account,
        title: title.to_owned(),
        username: None,
        password: None,
        notes: None,
        urls: Vec::new(),
        fields: Vec::new(),
        folder_id: None,
        favorite: false,
    }
}

/// A draft with a user name, a password, two addresses and three fields, one of them secret.
fn furnished(title: &str) -> EntryDraftDto {
    EntryDraftDto {
        username: Some("alguien@ejemplo".to_owned()),
        password: Some(A_WRITTEN_PASSWORD.to_owned()),
        notes: Some("una nota".to_owned()),
        urls: vec![
            "https://banco.es/login".to_owned(),
            "banco.example".to_owned(),
        ],
        fields: vec![
            DraftFieldDto {
                label: "Oficina".to_owned(),
                value: Some("Central".to_owned()),
                secret: false,
            },
            DraftFieldDto {
                label: "Titular".to_owned(),
                value: Some("Alguien".to_owned()),
                secret: false,
            },
            DraftFieldDto {
                label: "PIN".to_owned(),
                value: Some(A_SECRET_VALUE.to_owned()),
                secret: true,
            },
        ],
        ..draft(title)
    }
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

/// Every problem a refusal carries, or a failure naming what came back instead.
fn problems_of(outcome: &PasswordsError) -> &[cairn_lib::commands::passwords::FieldProblem] {
    match outcome {
        PasswordsError::Invalid { problems } => problems,
        other => panic!("expected a refusal about the draft, got {other:?}"),
    }
}

/// Whether a refusal names that field.
fn names_field(outcome: &PasswordsError, field: &str) -> bool {
    problems_of(outcome)
        .iter()
        .any(|problem| problem.field == field)
}

/// How many entries the vault holds, counting neither the bin nor the tombstones.
fn how_many_entries(state: &AppState) -> usize {
    list(state, &EntryFilter::All, None, NOW_US)
        .expect("the list reads")
        .items
        .len()
}

#[test]
fn all_six_refuse_while_the_vault_is_locked() {
    let scratch = Scratch::new("locked");
    let state = scratch.state();
    let somewhere = Uuid::from_bytes([1; 16]).to_string();

    assert_eq!(
        create(&state, &draft("Banco"), NOW_US),
        Err(PasswordsError::Locked)
    );
    assert_eq!(
        update(&state, &somewhere, &draft("Banco"), NOW_US),
        Err(PasswordsError::Locked)
    );
    assert_eq!(
        folder_save(&state, None, "Bancos", NOW_US),
        Err(PasswordsError::Locked)
    );
    assert_eq!(
        folder_delete(&state, &somewhere, NOW_US),
        Err(PasswordsError::Locked)
    );
    assert_eq!(
        folders_reorder(&state, std::slice::from_ref(&somewhere), NOW_US),
        Err(PasswordsError::Locked)
    );
    assert_eq!(
        history_clear(&state, &HistoryScopeDto::All, NOW_US),
        Err(PasswordsError::Locked)
    );
}

#[test]
fn a_new_entry_comes_back_with_everything_it_was_given_and_no_secret_value() {
    let scratch = Scratch::new("create");
    let state = unlocked(&scratch);

    let detail = create(&state, &furnished("Banco"), NOW_US).expect("a good draft is written");

    assert_eq!(detail.summary.title, "Banco");
    assert_eq!(detail.summary.username.as_deref(), Some("alguien@ejemplo"));
    assert_eq!(detail.urls.len(), 2);
    assert_eq!(detail.fields.len(), 3);
    assert!(detail.has_password);
    assert_eq!(detail.history_len, 0);

    let secret = detail.fields.get(2).expect("three fields");
    assert!(secret.secret);
    assert_eq!(secret.label, "PIN");
    assert_eq!(secret.value, None, "the secret value crossed the bridge");

    carries_no_secret("a created entry", &as_json(&detail));
}

#[test]
fn a_draft_without_a_title_is_refused_and_writes_nothing() {
    let scratch = Scratch::new("create-no-title");
    let state = unlocked(&scratch);

    let refusal = create(&state, &furnished("   "), NOW_US).expect_err("a title is required");

    assert!(names_field(&refusal, "title"));
    assert_eq!(
        how_many_entries(&state),
        0,
        "a refused draft left rows behind"
    );
}

#[test]
fn a_draft_with_more_addresses_than_allowed_is_refused_and_writes_nothing() {
    let scratch = Scratch::new("create-too-many-urls");
    let state = unlocked(&scratch);

    let too_many = EntryDraftDto {
        urls: (0..33).map(|number| format!("sitio{number}.es")).collect(),
        ..draft("Banco")
    };

    let refusal = create(&state, &too_many, NOW_US).expect_err("thirty three is one too many");

    assert!(names_field(&refusal, "urls"));
    assert_eq!(how_many_entries(&state), 0);
}

#[test]
fn a_draft_with_four_things_wrong_is_refused_with_four_problems() {
    let scratch = Scratch::new("create-four-problems");
    let state = unlocked(&scratch);

    let wrong = EntryDraftDto {
        title: "  ".to_owned(),
        username: Some("u".repeat(257)),
        urls: (0..33).map(|number| format!("sitio{number}.es")).collect(),
        fields: vec![DraftFieldDto {
            label: String::new(),
            value: Some("sin nombre".to_owned()),
            secret: false,
        }],
        ..draft("ignorado")
    };

    let refusal = create(&state, &wrong, NOW_US).expect_err("four things are wrong with it");

    assert_eq!(
        problems_of(&refusal).len(),
        4,
        "somebody filling a form is told about all of them at once, not one at a time: {:?}",
        problems_of(&refusal)
    );
    for field in ["title", "username", "urls", "fields"] {
        assert!(names_field(&refusal, field), "no problem named {field}");
    }
}

#[test]
fn saving_a_form_that_never_saw_the_password_leaves_it_alone() {
    let scratch = Scratch::new("update-password-absent");
    let state = unlocked(&scratch);
    let id = created(&state, &furnished("Banco"));

    let detail = update(
        &state,
        &id.to_string(),
        &EntryDraftDto {
            password: None,
            ..furnished("Banco renombrado")
        },
        NOW_US,
    )
    .expect("the entry is saved");

    assert_eq!(detail.summary.title, "Banco renombrado");
    assert!(detail.has_password);
    assert_eq!(
        stored_password(&state, id).as_deref(),
        Some(A_WRITTEN_PASSWORD)
    );
    assert_eq!(
        detail.history_len, 0,
        "a save that did not change the password recorded one"
    );
}

#[test]
fn changing_the_password_keeps_the_old_one_in_the_history() {
    let scratch = Scratch::new("update-password-new");
    let state = unlocked(&scratch);
    let id = created(&state, &furnished("Banco"));

    let detail = update(
        &state,
        &id.to_string(),
        &EntryDraftDto {
            password: Some("otra-distinta".to_owned()),
            ..furnished("Banco")
        },
        NOW_US,
    )
    .expect("the entry is saved");

    assert_eq!(
        stored_password(&state, id).as_deref(),
        Some("otra-distinta")
    );
    assert_eq!(detail.history_len, 1);
    assert_eq!(history(&state, &id.to_string()).expect("it reads").len(), 1);
}

#[test]
fn saving_the_same_password_again_does_not_grow_the_history() {
    let scratch = Scratch::new("update-password-same");
    let state = unlocked(&scratch);
    let id = created(&state, &furnished("Banco"));

    for _save in 0..3 {
        let detail = update(&state, &id.to_string(), &furnished("Banco"), NOW_US)
            .expect("the entry is saved");
        assert_eq!(
            detail.history_len, 0,
            "saving the form without touching the password recorded a change"
        );
    }

    assert_eq!(
        stored_password(&state, id).as_deref(),
        Some(A_WRITTEN_PASSWORD)
    );
}

#[test]
fn emptying_the_password_is_a_change_and_is_recorded_as_one() {
    let scratch = Scratch::new("update-password-empty");
    let state = unlocked(&scratch);
    let id = created(&state, &furnished("Banco"));

    let detail = update(
        &state,
        &id.to_string(),
        &EntryDraftDto {
            password: Some(String::new()),
            ..furnished("Banco")
        },
        NOW_US,
    )
    .expect("the entry is saved");

    assert!(!detail.has_password);
    assert_eq!(stored_password(&state, id), None);
    assert_eq!(detail.history_len, 1, "the password it had was not kept");
}

#[test]
fn a_secret_field_that_was_never_sent_back_survives_the_save() {
    let scratch = Scratch::new("update-secret-field");
    let state = unlocked(&scratch);
    let id = created(&state, &furnished("Banco"));

    // Exactly what the form does with what `get` gave it: the labels came back, the secret value
    // did not, and this is what it can therefore send.
    let unchanged = EntryDraftDto {
        fields: vec![
            DraftFieldDto {
                label: "Oficina".to_owned(),
                value: Some("Central".to_owned()),
                secret: false,
            },
            DraftFieldDto {
                label: "Titular".to_owned(),
                value: Some("Alguien".to_owned()),
                secret: false,
            },
            DraftFieldDto {
                label: "PIN".to_owned(),
                value: None,
                secret: true,
            },
        ],
        ..furnished("Banco")
    };

    let detail = update(&state, &id.to_string(), &unchanged, NOW_US).expect("the entry is saved");

    assert_eq!(detail.fields.len(), 3);
    assert_eq!(
        stored_fields(&state, id).get(2),
        Some(&("PIN".to_owned(), A_SECRET_VALUE.to_owned())),
        "the value of the secret field was lost by saving the form"
    );
    carries_no_secret("a saved entry", &as_json(&detail));
}

#[test]
fn reordering_the_addresses_keeps_the_rows_and_moves_the_values() {
    let scratch = Scratch::new("update-urls-order");
    let state = unlocked(&scratch);
    let id = created(&state, &furnished("Banco"));

    let before = get(&state, &id.to_string()).expect("the entry reads");
    let swapped = EntryDraftDto {
        urls: vec![
            "banco.example".to_owned(),
            "https://banco.es/login".to_owned(),
        ],
        ..furnished("Banco")
    };

    let after = update(&state, &id.to_string(), &swapped, NOW_US).expect("the entry is saved");

    let ids_before: Vec<&str> = before.urls.iter().map(|url| url.id.as_str()).collect();
    let ids_after: Vec<&str> = after.urls.iter().map(|url| url.id.as_str()).collect();
    assert_eq!(ids_before, ids_after, "the rows were replaced, not moved");
    assert_eq!(
        after.urls.first().map(|url| url.value.as_str()),
        Some("banco.example")
    );
    assert_eq!(
        after.urls.get(1).map(|url| url.value.as_str()),
        Some("https://banco.es/login")
    );
}

#[test]
fn something_in_the_bin_cannot_be_edited() {
    let scratch = Scratch::new("update-binned");
    let state = unlocked(&scratch);
    let id = created(&state, &furnished("Banco"));

    in_database(&state, |_storage, codec, connection| {
        repository::set_trashed(
            connection,
            codec,
            Hlc::new(9_000, 0, [4; 6]),
            NOW_US,
            id,
            true,
        )
    })
    .expect("the entry goes in the bin");

    assert_eq!(
        update(&state, &id.to_string(), &furnished("Otro nombre"), NOW_US),
        Err(PasswordsError::NotFound)
    );
}

#[test]
fn a_save_that_fails_at_the_last_step_leaves_nothing_behind() {
    let scratch = Scratch::new("update-rollback");
    let state = unlocked(&scratch);
    let id = created(&state, &furnished("Banco"));

    // The sequence `update` runs, driven by hand so that the last step can be made to fail. It is
    // forced from outside because no draft the domain accepts can make the repository refuse
    // halfway, which is the point: what is being proved is the transaction around the sequence,
    // and the only way to prove it is to fail the sequence.
    let outcome: Result<(), DbError> = state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage.database().in_transaction(|transaction| {
                repository::update_entry(
                    transaction,
                    &codec,
                    storage.device(),
                    Hlc::new(9_100, 0, [4; 6]),
                    NOW_US,
                    id,
                    NewEntry {
                        title: "Escrito a medias",
                        username: None,
                        password: Some(A_WRITTEN_PASSWORD),
                        notes: None,
                        folder_id: None,
                        favorite: false,
                    },
                    EntryKind::Account,
                )?;
                repository::replace_urls(
                    transaction,
                    &codec,
                    storage.device(),
                    Hlc::new(9_101, 0, [4; 6]),
                    NOW_US,
                    id,
                    &["solo-una.example"],
                )?;

                // The step that refuses. Everything above it has already run.
                Err(DbError::NotFound)
            })
        })
        .expect("the vault is open");

    assert!(matches!(outcome, Err(DbError::NotFound)));

    let detail = get(&state, &id.to_string()).expect("the entry reads");
    assert_eq!(detail.summary.title, "Banco", "the row was kept");
    assert_eq!(detail.urls.len(), 2, "the addresses were kept");
    assert_eq!(detail.fields.len(), 3, "the fields were kept");
    assert_eq!(detail.history_len, 0, "the history was written to");
}

#[test]
fn a_new_entry_can_be_found_without_unlocking_again() {
    let scratch = Scratch::new("create-then-search");
    let state = unlocked(&scratch);

    let _written = create(&state, &furnished("Caja Rural"), NOW_US).expect("it is written");

    let found = search(&state, "rural", 0).expect("the search runs");
    assert_eq!(found.total, 1);
    assert_eq!(
        found.hits.first().map(|hit| hit.title.as_str()),
        Some("Caja Rural")
    );
}

#[test]
fn renaming_an_entry_moves_it_in_the_index_rather_than_leaving_both_names() {
    let scratch = Scratch::new("update-then-search");
    let state = unlocked(&scratch);
    let id = created(&state, &furnished("Caja Rural"));

    let _saved = update(&state, &id.to_string(), &furnished("Caja Postal"), NOW_US)
        .expect("the entry is saved");

    assert_eq!(
        search(&state, "rural", 0).expect("the search runs").total,
        0,
        "the old title is still findable"
    );
    assert_eq!(
        search(&state, "postal", 0).expect("the search runs").total,
        1
    );
}

#[test]
fn a_folder_is_created_once_and_renamed_in_place() {
    let scratch = Scratch::new("folder-save");
    let state = unlocked(&scratch);

    let made = folder_save(&state, None, "  Bancos  ", NOW_US).expect("the folder is created");
    assert_eq!(made.name, "Bancos", "the name was not trimmed");

    let renamed = folder_save(&state, Some(&made.id), "Cuentas", NOW_US).expect("it is renamed");

    assert_eq!(renamed.id, made.id, "renaming made a second folder");
    assert_eq!(renamed.name, "Cuentas");
    assert_eq!(folders(&state).expect("they read").len(), 1);
}

#[test]
fn deleting_a_folder_says_how_many_entries_came_out_of_it_and_keeps_them() {
    let scratch = Scratch::new("folder-delete");
    let state = unlocked(&scratch);

    let folder = folder_save(&state, None, "Bancos", NOW_US).expect("the folder is created");
    for number in 0..3 {
        let inside = EntryDraftDto {
            folder_id: Some(folder.id.clone()),
            ..draft(&format!("Cuenta {number}"))
        };
        let _written = create(&state, &inside, NOW_US).expect("it is written");
    }

    let gone = folder_delete(&state, &folder.id, NOW_US).expect("the folder is deleted");

    assert_eq!(gone.entries, 3);
    assert!(folders(&state).expect("they read").is_empty());

    let left = list(&state, &EntryFilter::All, None, NOW_US).expect("the list reads");
    assert_eq!(
        left.items.len(),
        3,
        "deleting a folder took entries with it"
    );
    assert!(
        left.items.iter().all(|one| one.folder_id.is_none()),
        "the entries were left pointing at a folder that is gone"
    );
}

#[test]
fn an_order_that_is_not_the_whole_set_is_refused_and_changes_nothing() {
    let scratch = Scratch::new("folders-reorder");
    let state = unlocked(&scratch);

    let first = folder_save(&state, None, "Bancos", NOW_US).expect("it is created");
    let second = folder_save(&state, None, "Compras", NOW_US).expect("it is created");
    let before = folders(&state).expect("they read");

    assert_eq!(
        folders_reorder(&state, std::slice::from_ref(&second.id), NOW_US),
        Err(PasswordsError::IncompleteOrder)
    );
    assert_eq!(folders(&state).expect("they read"), before);

    folders_reorder(&state, &[second.id.clone(), first.id.clone()], NOW_US)
        .expect("the whole set is accepted");
    let after = folders(&state).expect("they read");
    assert_eq!(
        after.first().map(|one| one.id.as_str()),
        Some(second.id.as_str())
    );
}

#[test]
fn emptying_one_history_leaves_every_other_one_alone() {
    let scratch = Scratch::new("history-clear");
    let state = unlocked(&scratch);

    let one = created(&state, &furnished("Banco"));
    let other = created(&state, &furnished("Correo"));
    for id in [one, other] {
        let _saved = update(
            &state,
            &id.to_string(),
            &EntryDraftDto {
                password: Some("otra-distinta".to_owned()),
                ..furnished("Igual")
            },
            NOW_US,
        )
        .expect("the entry is saved");
    }

    let cleared = history_clear(
        &state,
        &HistoryScopeDto::Entry {
            id: one.to_string(),
        },
        NOW_US,
    )
    .expect("the history is emptied");

    assert_eq!(cleared.rows, 1);
    assert!(
        history(&state, &one.to_string())
            .expect("it reads")
            .is_empty()
    );
    assert_eq!(
        history(&state, &other.to_string()).expect("it reads").len(),
        1,
        "emptying one history emptied another"
    );

    let all = history_clear(&state, &HistoryScopeDto::All, NOW_US).expect("they are emptied");
    assert_eq!(all.rows, 1);
    assert!(
        history(&state, &other.to_string())
            .expect("it reads")
            .is_empty()
    );
}

#[test]
fn nothing_any_of_the_six_answers_with_carries_a_password_or_a_secret_value() {
    let scratch = Scratch::new("no-secrets-anywhere");
    let state = unlocked(&scratch);

    let created_detail = create(&state, &furnished("Banco"), NOW_US).expect("it is written");
    let id = created_detail.summary.id.clone();
    let saved = update(&state, &id, &furnished("Banco"), NOW_US).expect("it is saved");
    let folder = folder_save(&state, None, "Bancos", NOW_US).expect("it is created");
    let deleted = folder_delete(&state, &folder.id, NOW_US).expect("it is deleted");
    let cleared = history_clear(&state, &HistoryScopeDto::All, NOW_US).expect("it is emptied");

    carries_no_secret("a created entry", &as_json(&created_detail));
    carries_no_secret("a saved entry", &as_json(&saved));
    carries_no_secret("a saved folder", &as_json(&folder));
    carries_no_secret("a deleted folder", &as_json(&deleted));
    carries_no_secret("an emptied history", &as_json(&cleared));
}

/// Writes a draft through the command under test and answers with its identifier.
fn created(state: &AppState, draft: &EntryDraftDto) -> Uuid {
    let detail = create(state, draft, NOW_US).expect("a good draft is written");
    Uuid::parse_str(&detail.summary.id).expect("what came back is an identifier")
}
