//! Drives the copying commands against a clipboard in a variable rather than the desktop's.
//!
//! The property this file is for is what does **not** come back. `Copied` carries no text, and
//! the assertion is made against the serialised answer rather than against the type, because the
//! type is exactly what somebody adds a "what was copied" field to a year from now.
//!
//! The rest is about the timer. There is one, copying something else replaces it rather than
//! adding to it, it leaves alone anything somebody copied in the meantime, and locking the vault
//! does its work immediately — which is what makes locking mean something while a password is
//! still sitting on the clipboard.
// Every function in an integration test file is test code, but the lint that forbids panicking
// constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]` functions. The
// helpers below are neither, and a helper that cannot panic would have to return a Result that
// every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES};
use cairn_lib::clipboard::Clipboard;
use cairn_lib::commands::passwords::{
    CopyTarget, DraftFieldDto, EntryDraftDto, EntryKindDto, PasswordsError, copy, create, get,
    set_clipboard_seconds, trash,
};
use cairn_lib::commands::vault::create as create_vault;
use cairn_lib::state::AppState;
use cairn_lib::storage::DataDirectory;
use cairn_lib::vault::Vault;
use cairn_platform::clipboard::{ClipboardError, Fingerprint, fingerprint_of};
use uuid::Uuid;
use zeroize::Zeroizing;

/// Not a real password. A phrase invented for these tests, in a directory removed afterwards.
const NOT_A_REAL_PASSWORD: &str = "una frase larga inventada para la prueba";

/// The password these drafts carry, and the needle the leak check looks for.
const A_WRITTEN_PASSWORD: &str = "cairn-canary-password";

/// The value of the secret custom field.
const A_SECRET_VALUE: &str = "cairn-canary-secret";

/// The user name these drafts carry.
const A_USER_NAME: &str = "alguien@ejemplo";

/// A moment in the middle of the range, so the arithmetic either side of it is ordinary.
const NOW_US: i64 = 1_700_000_000_000_000;

/// A clipboard in a variable, which is what the desktop's is from this side.
#[derive(Debug, Default)]
struct Fake {
    held: Mutex<Option<String>>,
    refuse: Mutex<Option<ClipboardError>>,
}

impl Fake {
    fn holding(&self) -> Option<String> {
        self.held.lock().expect("no test panics here").clone()
    }

    fn put(&self, text: &str) {
        *self.held.lock().expect("no test panics here") = Some(text.to_owned());
    }

    fn refusing(&self, error: ClipboardError) {
        *self.refuse.lock().expect("no test panics here") = Some(error);
    }

    fn refusal(&self) -> Option<ClipboardError> {
        *self.refuse.lock().expect("no test panics here")
    }
}

impl Clipboard for Fake {
    fn write(&self, text: &str) -> Result<(), ClipboardError> {
        if let Some(error) = self.refusal() {
            return Err(error);
        }
        self.put(text);
        Ok(())
    }

    fn clear_if_matches(&self, expected: &Fingerprint) -> Result<bool, ClipboardError> {
        if let Some(error) = self.refusal() {
            return Err(error);
        }
        let mut held = self.held.lock().expect("no test panics here");
        match held.as_deref() {
            Some(text) if &fingerprint_of(text) == expected => {
                *held = None;
                Ok(true)
            }
            _somebody_elses => Ok(false),
        }
    }

    fn fingerprint(&self) -> Result<Option<Fingerprint>, ClipboardError> {
        if let Some(error) = self.refusal() {
            return Err(error);
        }
        Ok(self.holding().as_deref().map(fingerprint_of))
    }
}

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
                "cairn-passwords-clip-{name}-{}-{unique}",
                std::process::id()
            )),
        }
    }

    fn state(&self, clipboard: &Arc<Fake>) -> AppState {
        AppState::with_clipboard(
            Vault::open_at(&self.directory).expect("the directory can be read"),
            DataDirectory::new(self.directory.clone()),
            Arc::clone(clipboard) as Arc<dyn Clipboard>,
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

/// A created, open vault over a clipboard in a variable.
fn unlocked(scratch: &Scratch, clipboard: &Arc<Fake>) -> AppState {
    let state = scratch.state(clipboard);
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

/// A draft with a password, a user name, an address and two fields, one of them secret.
fn furnished(title: &str) -> EntryDraftDto {
    EntryDraftDto {
        kind: EntryKindDto::Account,
        title: title.to_owned(),
        username: Some(A_USER_NAME.to_owned()),
        password: Some(A_WRITTEN_PASSWORD.to_owned()),
        notes: None,
        urls: vec!["banco.example".to_owned()],
        fields: vec![
            DraftFieldDto {
                label: "Oficina".to_owned(),
                value: Some("Central".to_owned()),
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

/// Writes a draft and answers with its identifier.
fn written(state: &AppState, title: &str) -> Uuid {
    let detail = create(state, &furnished(title), NOW_US).expect("a good draft is written");
    Uuid::parse_str(&detail.summary.id).expect("what came back is an identifier")
}

/// Waits for the clipboard to hold what is wanted, or gives up.
///
/// Polling rather than sleeping for a fixed stretch: what is being waited for is the clipboard
/// changing, not a number of milliseconds, and a fixed sleep long enough to be reliable on a
/// loaded build agent is a fixed sleep nobody wants in a test suite.
fn settles(clipboard: &Fake, wanted: Option<&str>) -> bool {
    for _attempt in 0..600 {
        if clipboard.holding().as_deref() == wanted {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    false
}

#[test]
fn both_refuse_while_the_vault_is_locked() {
    let clipboard = Arc::new(Fake::default());
    let scratch = Scratch::new("locked");
    let state = scratch.state(&clipboard);

    assert_eq!(
        copy(
            &state,
            &Uuid::from_bytes([1; 16]).to_string(),
            &CopyTarget::Password,
            NOW_US
        ),
        Err(PasswordsError::Locked)
    );
    assert_eq!(
        set_clipboard_seconds(&state, 30, NOW_US),
        Err(PasswordsError::Locked)
    );
    assert_eq!(clipboard.holding(), None);
}

#[test]
fn a_copied_password_is_written_and_the_answer_carries_no_text() {
    let clipboard = Arc::new(Fake::default());
    let scratch = Scratch::new("password");
    let state = unlocked(&scratch, &clipboard);
    let id = written(&state, "Banco");

    let copied = copy(&state, &id.to_string(), &CopyTarget::Password, NOW_US)
        .expect("the password is copied");

    assert_eq!(copied.clears_in_s, Some(15));
    assert!(
        !copied.hardened,
        "the clipboard is not hardened until phase 08 and must not claim to be"
    );
    assert_eq!(clipboard.holding().as_deref(), Some(A_WRITTEN_PASSWORD));

    let json = serde_json::to_string(&copied).expect("the answer serialises");
    assert!(
        !json.contains(A_WRITTEN_PASSWORD),
        "the answer carried the value across the bridge: {json}"
    );
}

#[test]
fn what_is_not_a_secret_of_the_same_kind_is_not_taken_back() {
    let clipboard = Arc::new(Fake::default());
    let scratch = Scratch::new("username");
    let state = unlocked(&scratch, &clipboard);
    let id = written(&state, "Banco");

    let copied = copy(&state, &id.to_string(), &CopyTarget::Username, NOW_US)
        .expect("the user name is copied");

    assert_eq!(copied.clears_in_s, None);
    assert_eq!(clipboard.holding().as_deref(), Some(A_USER_NAME));
    assert!(!state.session().clipboard().is_armed());

    let address = get(&state, &id.to_string())
        .expect("the entry reads")
        .urls
        .first()
        .expect("it has one")
        .id
        .clone();
    let copied = copy(
        &state,
        &id.to_string(),
        &CopyTarget::Url { id: address },
        NOW_US,
    )
    .expect("the address is copied");

    assert_eq!(copied.clears_in_s, None);
    assert_eq!(clipboard.holding().as_deref(), Some("banco.example"));
}

#[test]
fn copying_twice_leaves_one_timer_and_the_first_does_not_wipe_the_second() {
    let clipboard = Arc::new(Fake::default());
    let scratch = Scratch::new("two-timers");
    let state = unlocked(&scratch, &clipboard);
    let id = written(&state, "Banco");
    let secret = get(&state, &id.to_string())
        .expect("the entry reads")
        .fields
        .get(1)
        .expect("it has two")
        .id
        .clone();

    // Five seconds is the floor, so the first timer cannot have fired by the time the second
    // copy replaces it, and the assertion below is about the replacement rather than about luck.
    let _first = set_clipboard_seconds(&state, 5, NOW_US).expect("five is accepted");
    let _password = copy(&state, &id.to_string(), &CopyTarget::Password, NOW_US)
        .expect("the password is copied");
    let _field = copy(
        &state,
        &id.to_string(),
        &CopyTarget::Field { id: secret },
        NOW_US,
    )
    .expect("the field is copied");

    assert_eq!(clipboard.holding().as_deref(), Some(A_SECRET_VALUE));
    assert!(state.session().clipboard().is_armed());

    assert!(
        settles(&clipboard, None),
        "the second value was never taken back"
    );
}

#[test]
fn what_somebody_else_copied_before_the_timer_woke_up_is_left_alone() {
    let clipboard = Arc::new(Fake::default());
    let scratch = Scratch::new("someone-else");
    let state = unlocked(&scratch, &clipboard);
    let id = written(&state, "Banco");

    let _first = set_clipboard_seconds(&state, 5, NOW_US).expect("five is accepted");
    let _password = copy(&state, &id.to_string(), &CopyTarget::Password, NOW_US)
        .expect("the password is copied");

    clipboard.put("la lista de la compra");

    assert!(
        !settles(&clipboard, None),
        "the timer wiped what somebody else had copied"
    );
    assert_eq!(
        clipboard.holding().as_deref(),
        Some("la lista de la compra")
    );
}

#[test]
fn locking_the_vault_takes_the_password_back_at_once() {
    let clipboard = Arc::new(Fake::default());
    let scratch = Scratch::new("lock");
    let state = unlocked(&scratch, &clipboard);
    let id = written(&state, "Banco");

    let _sixty = set_clipboard_seconds(&state, 60, NOW_US).expect("sixty is accepted");
    let _password = copy(&state, &id.to_string(), &CopyTarget::Password, NOW_US)
        .expect("the password is copied");
    assert_eq!(clipboard.holding().as_deref(), Some(A_WRITTEN_PASSWORD));

    assert!(state.session().lock(), "the vault was open");

    assert_eq!(
        clipboard.holding(),
        None,
        "locking the vault left the password on the clipboard for another minute"
    );
    assert!(!state.session().clipboard().is_armed());
}

#[test]
fn a_clipboard_that_refuses_is_a_typed_refusal_and_never_the_value() {
    let clipboard = Arc::new(Fake::default());
    let scratch = Scratch::new("busy");
    let state = unlocked(&scratch, &clipboard);
    let id = written(&state, "Banco");

    clipboard.refusing(ClipboardError::Busy);

    assert_eq!(
        copy(&state, &id.to_string(), &CopyTarget::Password, NOW_US),
        Err(PasswordsError::Clipboard)
    );
    assert!(!state.session().clipboard().is_armed());
}

#[test]
fn a_field_of_another_entry_is_not_copied_by_naming_this_one() {
    let clipboard = Arc::new(Fake::default());
    let scratch = Scratch::new("someone-elses-field");
    let state = unlocked(&scratch, &clipboard);
    let mine = written(&state, "Banco");
    let theirs = written(&state, "Correo");

    let not_mine = get(&state, &theirs.to_string())
        .expect("the entry reads")
        .fields
        .get(1)
        .expect("it has two")
        .id
        .clone();

    assert_eq!(
        copy(
            &state,
            &mine.to_string(),
            &CopyTarget::Field { id: not_mine },
            NOW_US
        ),
        Err(PasswordsError::NotFound)
    );
    assert_eq!(clipboard.holding(), None);
}

#[test]
fn nothing_of_something_in_the_bin_is_copied() {
    let clipboard = Arc::new(Fake::default());
    let scratch = Scratch::new("binned");
    let state = unlocked(&scratch, &clipboard);
    let id = written(&state, "Banco");

    let _binned = trash(&state, &id.to_string(), true, NOW_US).expect("it goes in the bin");

    assert_eq!(
        copy(&state, &id.to_string(), &CopyTarget::Password, NOW_US),
        Err(PasswordsError::NotFound)
    );
    assert_eq!(clipboard.holding(), None);
}

#[test]
fn the_wipe_delay_is_bounded_at_both_ends_and_what_is_set_is_what_is_used() {
    let clipboard = Arc::new(Fake::default());
    let scratch = Scratch::new("seconds");
    let state = unlocked(&scratch, &clipboard);
    let id = written(&state, "Banco");

    assert!(matches!(
        set_clipboard_seconds(&state, 4, NOW_US),
        Err(PasswordsError::Invalid { .. })
    ));
    assert!(matches!(
        set_clipboard_seconds(&state, 61, NOW_US),
        Err(PasswordsError::Invalid { .. })
    ));

    assert_eq!(
        set_clipboard_seconds(&state, 5, NOW_US)
            .expect("five is accepted")
            .clipboard_clear_s,
        5
    );
    assert_eq!(
        set_clipboard_seconds(&state, 60, NOW_US)
            .expect("sixty is accepted")
            .clipboard_clear_s,
        60
    );

    let _thirty = set_clipboard_seconds(&state, 30, NOW_US).expect("thirty is accepted");
    assert_eq!(
        copy(&state, &id.to_string(), &CopyTarget::Password, NOW_US)
            .expect("the password is copied")
            .clears_in_s,
        Some(30)
    );
}
