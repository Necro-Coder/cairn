//! The passwords module, from the other side of the bridge.
//!
//! One rule governs every line of this file, and it is worth reading before any of them:
//! **nothing decrypted crosses the bridge except what is about to be drawn on the screen at that
//! instant.** In practice that means the six commands below carry no password and no secret
//! value, in any shape — not whole, not truncated, not as a length, not as a count, not in an
//! error message. `passwords_get` carries the *labels* of the custom fields and, for a field
//! marked secret, `None` where the value would be. Getting a value out of this process takes a
//! separate command, a closed enumeration naming which single value, and somebody pressing a
//! button.
//!
//! `EntryDetail::has_password` is a boolean rather than a length for the same reason. A length is
//! information about a password, and a screen that knows a password is eleven characters long is
//! a screen that told somebody standing behind it eleven characters' worth.
//!
//! Where the search index lives is the other decision worth the words. It is the one piece of
//! plaintext this application keeps between calls, so it lives inside [`Storage`], which lives
//! inside the session beside the keys and is dropped by the lock. Building it is not allowed to
//! fail an unlock: refusing to open a vault because one title does not decrypt turns a problem
//! with one row into the loss of everything else, so the vault opens with an empty index and the
//! interface is told the search is not available.

use core::fmt::Write as _;

use cairn_crypto::constant_time_eq;
use cairn_db::codec::FieldCodec;
use cairn_db::repositories::settings;
use cairn_db::repositories::vault::{
    self as repository, Entry, HistoryScope, MAX_PAGE, NewEntry, NewField, Sweep,
};
use cairn_db::search::{Matched, Searchable};
use cairn_db::{Connection, DbError};
use cairn_domain::Hlc;
use cairn_domain::vault::{
    DraftField, EntryDraft, EntryKind, FieldError, FieldKind, Problem, TrashState, ValidEntry,
    trash, validate_folder_name,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::clock::{now_ms, now_us};
use crate::state::AppState;
use crate::storage::Storage;

/// The preference that says how long a copied secret stays on the clipboard.
pub const CLIPBOARD_SECONDS_KEY: &str = "vault.clipboard_clear_seconds";

/// How long a copied secret stays on the clipboard when nothing says otherwise.
pub const DEFAULT_CLIPBOARD_SECONDS: u16 = 15;

/// How long a revealed value stays on screen. Twenty seconds.
///
/// Here rather than in a component so that the countdown somebody watches and the policy are the
/// same number, and so that changing it later is an edit to a constant rather than a hunt through
/// the interface for a literal.
pub const REVEAL_SECONDS: u16 = 20;

/// Why something about the passwords module did not happen.
///
/// Every failure of the database collapses into [`PasswordsError::Storage`] with no detail. What
/// SQLite said belongs in a log on this side; what reaches a screen is that storage failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[non_exhaustive]
pub enum PasswordsError {
    /// The vault is closed, and every one of these needs it open.
    #[error("the vault is locked")]
    Locked,

    /// There is no such entry, folder or row — or what arrived was not an identifier at all.
    #[error("there is no such entry")]
    NotFound,

    /// A field of the draft is not acceptable. Every problem, not the first one.
    #[error("the draft was refused")]
    #[serde(rename_all = "camelCase")]
    Invalid {
        /// Everything wrong with the draft, one entry per problem.
        problems: Vec<FieldProblem>,
    },

    /// Asked to destroy something that is not in the bin.
    ///
    /// Destroying is only reachable from the bin, so that there is no path in this application
    /// from a list straight to an irreversible deletion.
    #[error("that entry is not in the bin")]
    NotInTrash,

    /// The order offered is not the set of folders there are.
    #[error("the order was not the whole set")]
    IncompleteOrder,

    /// The database refused. Deliberately without the reason.
    #[error("the database refused")]
    Storage,
}

impl From<DbError> for PasswordsError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::Closed => Self::Locked,
            DbError::NotFound => Self::NotFound,
            DbError::IncompleteOrder => Self::IncompleteOrder,
            _other => Self::Storage,
        }
    }
}

/// One thing wrong with a draft, named by the field it is wrong about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldProblem {
    /// The field, spelled the way the form spells it.
    pub field: String,
    /// Which position in a list, for the fields that are lists.
    pub index: Option<u32>,
    /// What is wrong with it, as a word the interface turns into a sentence.
    pub code: String,
    /// The ceiling that was passed, for the problems that have one.
    pub limit: Option<u32>,
}

/// Whether an entry is an account or a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKindDto {
    /// A site, a user name and a password.
    Account,
    /// Text and nothing else.
    Note,
}

impl EntryKindDto {
    /// What the bridge calls what the domain holds.
    const fn of(kind: EntryKind) -> Self {
        match kind {
            EntryKind::Account => Self::Account,
            EntryKind::Note => Self::Note,
        }
    }

    /// What the domain calls what the bridge sent.
    const fn kind(self) -> EntryKind {
        match self {
            Self::Account => EntryKind::Account,
            Self::Note => EntryKind::Note,
        }
    }
}

/// Where something in the bin stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashedDto {
    /// When it was thrown away, in microseconds.
    pub trashed_at: i64,
    /// How many whole days it has been there.
    pub days: i64,
    /// How many are left before it goes on its own.
    pub days_left: i64,
}

/// What a list shows. No password, no secret value, ever.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    /// The entry, as text the bridge can carry.
    pub id: String,
    /// Whether it is an account or a note.
    pub kind: EntryKindDto,
    /// What it is called.
    pub title: String,
    /// The user name, if it has one.
    pub username: Option<String>,
    /// The folder it is in, or `None` for one at the root.
    pub folder_id: Option<String>,
    /// Whether somebody marked it as a favourite.
    pub favorite: bool,
    /// When it was last revealed or copied, if ever.
    pub last_used_at: Option<i64>,
    /// Set only for what is in the bin, with how many days it has left.
    pub trashed: Option<TrashedDto>,
}

/// One address as the screen sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlDto {
    /// The row, as text.
    pub id: String,
    /// The address, exactly as somebody wrote it. Never parsed and never completed.
    pub value: String,
}

/// One custom field as the screen sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDto {
    /// The row, as text.
    pub id: String,
    /// What the field is called.
    pub label: String,
    /// `None` for a secret one. **This is the rule of the module in one field.**
    ///
    /// Emptied here, in the core, and not hidden by whoever draws it. A value that travels and is
    /// then not shown is a value that travelled.
    pub value: Option<String>,
    /// Whether the interface hides it until somebody asks.
    pub secret: bool,
}

/// One entry opened on the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDetail {
    /// Everything a list would have shown.
    pub summary: EntrySummary,
    /// The notes, if there are any.
    pub notes: Option<String>,
    /// Every address, in order.
    pub urls: Vec<UrlDto>,
    /// Every custom field, in order.
    pub fields: Vec<FieldDto>,
    /// How many old passwords there are. A number, not the passwords.
    pub history_len: u32,
    /// Whether there is a password at all, so the screen can draw the reveal button.
    ///
    /// A boolean and never a length: a length is information about the password.
    pub has_password: bool,
    /// When the entry was first written, in microseconds.
    pub created_at: i64,
    /// When it was last written.
    pub updated_at: i64,
}

/// One folder as the screen sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDto {
    /// The folder, as text.
    pub id: String,
    /// What it is called.
    pub name: String,
    /// Where it sits in the list.
    pub position: i64,
    /// How many live entries are in it, not counting the bin.
    pub entries: u32,
}

/// One row of the history: when, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItemDto {
    /// The row, as text. What asking for that one old password takes.
    pub id: String,
    /// When the password it holds stopped being the current one, in microseconds.
    pub replaced_at: i64,
}

/// One page of a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing<T> {
    /// What is on this page.
    pub items: Vec<T>,
    /// What to pass as `after` for the next page, or `None` when this was the last one.
    pub next: Option<String>,
}

/// Which entries a listing asks for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EntryFilter {
    /// Everything that is not in the bin.
    All,
    /// What is in one folder.
    Folder {
        /// The folder.
        id: String,
    },
    /// What somebody marked.
    Favorites,
    /// The bin, and the only filter that returns anything with `trashed_at` set.
    Trash,
}

/// One entry a search found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// The entry, as text.
    pub id: String,
    /// Its title, which is what the list draws.
    pub title: String,
    /// Which field answered: `title`, `username` or `url`.
    pub matched: String,
}

/// One page of a search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    /// The matches on the page that was asked for.
    pub hits: Vec<SearchHit>,
    /// How many matched in total, across every page.
    pub total: u32,
    /// `false` if the index does not hold every entry in the file, so the interface can say that
    /// the search is not seeing everything rather than letting somebody believe it is.
    pub complete: bool,
}

/// What the module is set to, and what is not hardened yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordsSettings {
    /// How long a copied secret stays on the clipboard.
    pub clipboard_clear_s: u16,
    /// How long a revealed value stays on screen.
    pub reveal_s: u16,
    /// How many old passwords are kept per entry.
    pub max_history: u16,
    /// **False until phase 08.** What the warning on the entry screen is drawn from.
    ///
    /// Until the clipboard contents are marked with the Windows exclusion formats, every password
    /// copied lands in the clipboard history in the clear and survives the vault being locked.
    pub clipboard_hardened: bool,
    /// **False until phase 08.** Likewise, for screen capture.
    pub screen_hardened: bool,
}

/// An entry as it arrives from the form.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDraftDto {
    /// Whether it is an account or a note.
    pub kind: EntryKindDto,
    /// What to call it.
    pub title: String,
    /// The user name, if there is one.
    pub username: Option<String>,
    /// `None` means "leave the password as it is". An empty string means "remove it".
    ///
    /// The distinction is the whole reason this field is an `Option` of a `String` and not a
    /// `String`: the edit form never receives the current password, so it cannot send it back,
    /// and a missing field has to mean "unchanged" or every save would wipe it.
    pub password: Option<String>,
    /// The notes, if there are any.
    pub notes: Option<String>,
    /// Every address, in the order the form drew them.
    pub urls: Vec<String>,
    /// Every custom field, in the order the form drew them.
    pub fields: Vec<DraftFieldDto>,
    /// The folder it goes in, or nothing for the root.
    pub folder_id: Option<String>,
    /// Whether somebody marked it.
    pub favorite: bool,
}

/// One custom field as it arrives.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftFieldDto {
    /// What names it.
    pub label: String,
    /// `None` on a secret field means "leave it as it was", for the same reason as above.
    pub value: Option<String>,
    /// Whether its value is hidden until somebody asks for it.
    pub secret: bool,
}

/// What deleting a folder did to what was in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDeleted {
    /// How many entries came out of it and now sit in the root.
    pub entries: u32,
}

/// What emptying a history did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleared {
    /// How many old passwords stopped existing.
    pub rows: u32,
}

/// What emptying the bin destroyed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Emptied {
    /// How many entries stopped existing.
    pub entries: u32,
}

/// Whose history to empty.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HistoryScopeDto {
    /// One entry's.
    Entry {
        /// Which one.
        id: String,
    },
    /// Every entry's, which is the button in the settings screen.
    All,
}

/// Every entry a filter admits, one page at a time.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::Storage`] if the database
/// refuses.
pub fn list(
    state: &AppState,
    filter: &EntryFilter,
    after: Option<&str>,
    now: i64,
) -> Result<Listing<EntrySummary>, PasswordsError> {
    if matches!(filter, EntryFilter::Trash) {
        return in_storage(state, |_storage, codec, connection| {
            let binned = repository::trashed(connection, codec, now, MAX_PAGE)?;
            Ok(Listing {
                items: binned.iter().map(binned_summary).collect(),
                next: None,
            })
        });
    }

    // A folder that is not there is an empty list rather than a refusal. It is what a screen sees
    // for a heartbeat after something else deletes the folder it is showing, and an error page is
    // the wrong answer to "there is nothing here".
    let folder = match filter {
        EntryFilter::Folder { id } => match Uuid::parse_str(id) {
            Ok(id) => Some(id),
            Err(_not_a_uuid) => {
                return Ok(Listing {
                    items: Vec::new(),
                    next: None,
                });
            }
        },
        _other => None,
    };
    let favorites = matches!(filter, EntryFilter::Favorites);
    let start = after.map(cursor_to_hlc).transpose()?;

    in_storage(state, |_storage, codec, connection| {
        let mut items = Vec::new();
        let mut cursor = start;

        // The filter is applied after the page is read, so a page of two hundred rows can yield
        // one summary. Reading until the page is full or the file runs out is what keeps "there
        // is more" from meaning "there were more rows", which is a different question.
        loop {
            let page = repository::entries(connection, codec, cursor, MAX_PAGE)?;
            let Some(last) = page.last() else { break };
            let was_full = page.len() == MAX_PAGE;
            cursor = Some(last.hlc);

            items.extend(
                page.into_iter()
                    .filter(|entry| admits(entry, folder, favorites))
                    .map(|entry| summary_of(&entry)),
            );

            if !was_full || items.len() >= MAX_PAGE {
                return Ok(Listing {
                    items,
                    next: was_full.then(|| hlc_to_cursor(cursor)).flatten(),
                });
            }
        }

        Ok(Listing { items, next: None })
    })
}

/// One page of the entries matching what was typed.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed.
pub fn search(state: &AppState, query: &str, page: u16) -> Result<SearchPage, PasswordsError> {
    let found = state
        .session()
        .with_open(|_vault, storage| storage.search_entries(query, page))
        .ok_or(PasswordsError::Locked)?;

    Ok(SearchPage {
        hits: found
            .hits
            .iter()
            .map(|hit| SearchHit {
                id: hit.id.to_string(),
                title: hit.title.to_string(),
                matched: match hit.matched {
                    Matched::Title => "title",
                    Matched::Username => "username",
                    Matched::Url => "url",
                }
                .to_owned(),
            })
            .collect(),
        total: u32::try_from(found.total).unwrap_or(u32::MAX),
        complete: found.complete,
    })
}

/// One entry in full, with the value of every secret field left behind.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::NotFound`] if there is no
/// such entry or it is in the bin, [`PasswordsError::Storage`] if the database refuses.
pub fn get(state: &AppState, id: &str) -> Result<EntryDetail, PasswordsError> {
    let id = parsed(id)?;

    in_storage(state, |_storage, codec, connection| {
        detail_of(connection, codec, id)
    })
}

/// When each password of an entry was replaced. No passwords.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::NotFound`] if there is no
/// such entry or it is in the bin, [`PasswordsError::Storage`] if the database refuses.
pub fn history(state: &AppState, id: &str) -> Result<Vec<HistoryItemDto>, PasswordsError> {
    let id = parsed(id)?;

    in_storage(state, |_storage, codec, connection| {
        // Read first, so that the history of something in the bin is as unreachable as the entry.
        repository::entry(connection, codec, id)?.ok_or(PasswordsError::NotFound)?;

        Ok(repository::history_moments(connection, id)?
            .into_iter()
            .map(|moment| HistoryItemDto {
                id: moment.id.to_string(),
                replaced_at: moment.replaced_at,
            })
            .collect())
    })
}

/// Every folder, in the order somebody arranged them.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::Storage`] if the database
/// refuses.
pub fn folders(state: &AppState) -> Result<Vec<FolderDto>, PasswordsError> {
    in_storage(state, |_storage, codec, connection| {
        Ok(repository::folders(connection, codec)?
            .iter()
            .map(folder_dto)
            .collect())
    })
}

/// What the module is set to.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::Storage`] if the database
/// refuses.
pub fn settings(state: &AppState) -> Result<PasswordsSettings, PasswordsError> {
    in_storage(state, |_storage, codec, connection| {
        Ok(PasswordsSettings {
            clipboard_clear_s: clipboard_seconds(connection, codec)?,
            reveal_s: REVEAL_SECONDS,
            max_history: u16::try_from(cairn_db::repositories::vault::MAX_HISTORY)
                .unwrap_or(u16::MAX),
            // Both false until phase 08, which is the one that marks the clipboard contents with
            // the Windows exclusion formats and keeps the window out of a screen capture. The
            // warning the entry screen draws is drawn from exactly these two.
            clipboard_hardened: false,
            screen_hardened: false,
        })
    })
}

/// Writes a new entry down whole, with its addresses and its custom fields.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::Invalid`] carrying every
/// problem the draft has, [`PasswordsError::Storage`] if the database refuses. Nothing is written
/// unless all of it is.
pub fn create(
    state: &AppState,
    draft: &EntryDraftDto,
    now: i64,
) -> Result<EntryDetail, PasswordsError> {
    let millis = now_ms();

    writing(state, |storage, codec, connection| {
        // Nothing to carry over: there is no entry yet, so a secret field with no value is a
        // secret field somebody left empty rather than one they did not retype.
        let valid = validated(draft, &[])?;
        let inner = valid.draft();

        let written = repository::create_entry(
            connection,
            codec,
            storage.device(),
            storage.next_hlc(millis),
            now,
            NewEntry {
                title: &inner.title,
                username: inner.username.as_deref(),
                password: inner.password.as_deref(),
                notes: inner.notes.as_deref(),
                folder_id: inner.folder_id,
                favorite: inner.favorite,
            },
            inner.kind,
        )?;

        write_children(connection, codec, storage, millis, now, written.id, inner)?;

        Ok((
            detail_of(connection, codec, written.id)?,
            Some((written.id, Some(searchable_of(written.id, inner)))),
        ))
    })
}

/// Saves a whole draft over an entry that already exists.
///
/// The order is the contract: read what is there, judge the draft, write the row, write the
/// addresses, write the custom fields, and the password last of all. The password is last because
/// it is the one write that pushes to the history, and pushing an old password into the history
/// for a save that then failed would record a change that never happened.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::NotFound`] if there is no
/// such entry or it is in the bin, [`PasswordsError::Invalid`] carrying every problem the draft
/// has, [`PasswordsError::Storage`] if the database refuses.
pub fn update(
    state: &AppState,
    id: &str,
    draft: &EntryDraftDto,
    now: i64,
) -> Result<EntryDetail, PasswordsError> {
    let id = parsed(id)?;
    let millis = now_ms();

    writing(state, |storage, codec, connection| {
        // Through the reader that leaves the bin out, so editing something somebody threw away is
        // the same answer as editing something that was never there.
        let existing = repository::entry(connection, codec, id)?.ok_or(PasswordsError::NotFound)?;
        let existing_fields = repository::fields(connection, codec, id)?;

        let valid = validated(draft, &existing_fields)?;
        let inner = valid.draft();

        // What the entry has now. The row is written with it, because the password is a write of
        // its own further down and this one must not change it either way.
        let current = existing.password.as_deref().map(String::as_str);

        repository::update_entry(
            connection,
            codec,
            storage.device(),
            storage.next_hlc(millis),
            now,
            id,
            NewEntry {
                title: &inner.title,
                username: inner.username.as_deref(),
                password: current,
                notes: inner.notes.as_deref(),
                folder_id: inner.folder_id,
                favorite: inner.favorite,
            },
            inner.kind,
        )?;

        write_children(connection, codec, storage, millis, now, id, inner)?;

        // A form that never received the password cannot send it back, so a missing field is the
        // password staying as it was, and staying as it was is not a change to record.
        let wanted = if draft.password.is_none() {
            current
        } else {
            inner.password.as_deref()
        };
        if password_changed(current, wanted) {
            repository::replace_password(
                connection,
                codec,
                storage.device(),
                storage.next_hlc(millis),
                now,
                id,
                wanted,
            )?;
        }

        Ok((
            detail_of(connection, codec, id)?,
            Some((id, Some(searchable_of(id, inner)))),
        ))
    })
}

/// Creates a folder, or renames the one that identifier names.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::NotFound`] if the folder
/// to rename is not there, [`PasswordsError::Invalid`] if the name is not acceptable,
/// [`PasswordsError::Storage`] if the database refuses.
pub fn folder_save(
    state: &AppState,
    id: Option<&str>,
    name: &str,
    now: i64,
) -> Result<FolderDto, PasswordsError> {
    let id = id.map(parsed).transpose()?;
    let millis = now_ms();

    // Judged here rather than left to the repository, because a refusal has to arrive as the list
    // of problems every other refusal in this module arrives as.
    let name = validate_folder_name(name).map_err(|problem| PasswordsError::Invalid {
        problems: vec![field_problem(&problem)],
    })?;

    writing(state, |storage, codec, connection| {
        let folder = repository::save_folder(
            connection,
            codec,
            storage.device(),
            storage.next_hlc(millis),
            now,
            id,
            &name,
        )?;

        Ok((folder_dto(&folder), None))
    })
}

/// Removes a folder and leaves what was in it at the root.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::NotFound`] if there is no
/// such folder, [`PasswordsError::Storage`] if the database refuses.
pub fn folder_delete(
    state: &AppState,
    id: &str,
    now: i64,
) -> Result<FolderDeleted, PasswordsError> {
    let id = parsed(id)?;
    let millis = now_ms();

    writing(state, |storage, codec, connection| {
        let moved =
            repository::delete_folder(connection, codec, storage.next_hlc(millis), now, id)?;

        Ok((FolderDeleted { entries: moved }, None))
    })
}

/// Puts the folders in the order they arrive in.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::IncompleteOrder`] if the
/// list is not exactly the set of folders there are, [`PasswordsError::NotFound`] if one of them
/// is not an identifier, [`PasswordsError::Storage`] if the database refuses.
pub fn folders_reorder(state: &AppState, ids: &[String], now: i64) -> Result<(), PasswordsError> {
    let ids = ids
        .iter()
        .map(|one| parsed(one))
        .collect::<Result<Vec<Uuid>, PasswordsError>>()?;
    let millis = now_ms();

    writing(state, |storage, codec, connection| {
        repository::reorder_folders(connection, codec, storage.next_hlc(millis), now, &ids)?;

        Ok(((), None))
    })
}

/// Empties the history of one entry, or of every entry.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::NotFound`] if the scope
/// names something that is not an identifier, [`PasswordsError::Storage`] if the database
/// refuses.
pub fn history_clear(
    state: &AppState,
    scope: &HistoryScopeDto,
    now: i64,
) -> Result<Cleared, PasswordsError> {
    let scope = match scope {
        HistoryScopeDto::Entry { id } => HistoryScope::Entry(parsed(id)?),
        HistoryScopeDto::All => HistoryScope::All,
    };
    let millis = now_ms();

    writing(state, |storage, _codec, connection| {
        let rows = repository::clear_history(connection, storage.next_hlc(millis), now, scope)?;

        Ok((Cleared { rows }, None))
    })
}

/// Throws an entry away, or takes it back out, and says what it now is.
///
/// Idempotent in both directions. Throwing away something already in the bin leaves the moment it
/// went in, rather than restarting its thirty days every time somebody clicks twice.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::NotFound`] if there is no
/// such entry, [`PasswordsError::Storage`] if the database refuses.
pub fn trash(
    state: &AppState,
    id: &str,
    trashed: bool,
    now: i64,
) -> Result<EntrySummary, PasswordsError> {
    let id = parsed(id)?;
    let millis = now_ms();

    writing(state, |storage, codec, connection| {
        repository::set_trashed(
            connection,
            codec,
            storage.next_hlc(millis),
            now,
            id,
            trashed,
        )?;

        // Read afterwards, and through the reader that sees the bin: what the screen needs is
        // what the entry now is, and the ordinary reader stops seeing it the moment it goes in.
        let found =
            repository::any_entry(connection, codec, id)?.ok_or(PasswordsError::NotFound)?;
        let summary = EntrySummary {
            trashed: trashed_dto(
                trash::state(now, found.trashed_at, found.deleted),
                found.trashed_at,
            ),
            ..summary_of(&found)
        };

        // Out of the index on the way in, back into it on the way out. Finding something by
        // typing its name moments after throwing it away is the opposite of having thrown it away.
        let update = if trashed {
            Some((id, None))
        } else {
            let addresses = repository::urls(connection, codec, id)?;
            Some((id, Some(searchable_of_entry(&found, &addresses))))
        };

        Ok((summary, update))
    })
}

/// Destroys an entry that is in the bin, with everything hanging off it.
///
/// Refuses anything that is not in the bin. There is no path in this application from a list
/// straight to an irreversible deletion, not even behind a confirmation: going through the bin
/// **is** the confirmation, and it lasts thirty days.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::NotFound`] if there is no
/// such entry, [`PasswordsError::NotInTrash`] if it never reached the bin,
/// [`PasswordsError::Storage`] if the database refuses.
pub fn delete(state: &AppState, id: &str, now: i64) -> Result<(), PasswordsError> {
    let id = parsed(id)?;
    let millis = now_ms();

    writing(state, |storage, codec, connection| {
        let found =
            repository::any_entry(connection, codec, id)?.ok_or(PasswordsError::NotFound)?;
        if found.trashed_at.is_none() {
            return Err(PasswordsError::NotInTrash);
        }

        repository::delete_entry(connection, storage.next_hlc(millis), now, id)?;

        Ok(((), Some((id, None))))
    })
}

/// Destroys everything in the bin, whether or not its thirty days have run out.
///
/// # Errors
///
/// [`PasswordsError::Locked`] if the vault is closed, [`PasswordsError::Storage`] if the database
/// refuses.
pub fn empty_trash(state: &AppState, now: i64) -> Result<Emptied, PasswordsError> {
    let millis = now_ms();

    writing(state, |storage, _codec, connection| {
        let destroyed =
            repository::empty_bin(connection, storage.next_hlc(millis), now, Sweep::All)?;

        // Nothing to tell the index. Everything the bin held left it when it went in, and an
        // index built by an unlock never saw what was already in there.
        Ok((Emptied { entries: destroyed }, None))
    })
}

/// Every entry a filter admits, one page at a time.
///
/// # Errors
///
/// See [`list`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_list(
    state: tauri::State<'_, AppState>,
    filter: EntryFilter,
    after: Option<String>,
) -> Result<Listing<EntrySummary>, PasswordsError> {
    list(&state, &filter, after.as_deref(), now_us())
}

/// One page of the entries matching what was typed.
///
/// # Errors
///
/// See [`search`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_search(
    state: tauri::State<'_, AppState>,
    query: String,
    page: u16,
) -> Result<SearchPage, PasswordsError> {
    search(&state, &query, page)
}

/// One entry in full.
///
/// # Errors
///
/// See [`get`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_get(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<EntryDetail, PasswordsError> {
    get(&state, &id)
}

/// When each password of an entry was replaced.
///
/// # Errors
///
/// See [`history`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_history(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Vec<HistoryItemDto>, PasswordsError> {
    history(&state, &id)
}

/// Every folder.
///
/// # Errors
///
/// See [`folders`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_folders(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<FolderDto>, PasswordsError> {
    folders(&state)
}

/// What the module is set to.
///
/// # Errors
///
/// See [`settings`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_settings(
    state: tauri::State<'_, AppState>,
) -> Result<PasswordsSettings, PasswordsError> {
    settings(&state)
}

/// Writes a new entry down whole.
///
/// # Errors
///
/// See [`create`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_create(
    state: tauri::State<'_, AppState>,
    draft: EntryDraftDto,
) -> Result<EntryDetail, PasswordsError> {
    create(&state, &draft, now_us())
}

/// Saves a whole draft over an entry that already exists.
///
/// # Errors
///
/// See [`update`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_update(
    state: tauri::State<'_, AppState>,
    id: String,
    draft: EntryDraftDto,
) -> Result<EntryDetail, PasswordsError> {
    update(&state, &id, &draft, now_us())
}

/// Creates or renames a folder.
///
/// # Errors
///
/// See [`folder_save`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_folder_save(
    state: tauri::State<'_, AppState>,
    id: Option<String>,
    name: String,
) -> Result<FolderDto, PasswordsError> {
    folder_save(&state, id.as_deref(), &name, now_us())
}

/// Removes a folder, leaving what was in it at the root.
///
/// # Errors
///
/// See [`folder_delete`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_folder_delete(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<FolderDeleted, PasswordsError> {
    folder_delete(&state, &id, now_us())
}

/// Puts the folders in the order they arrive in.
///
/// # Errors
///
/// See [`folders_reorder`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_folders_reorder(
    state: tauri::State<'_, AppState>,
    ids: Vec<String>,
) -> Result<(), PasswordsError> {
    folders_reorder(&state, &ids, now_us())
}

/// Empties a history.
///
/// # Errors
///
/// See [`history_clear`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_history_clear(
    state: tauri::State<'_, AppState>,
    scope: HistoryScopeDto,
) -> Result<Cleared, PasswordsError> {
    history_clear(&state, &scope, now_us())
}

/// Throws an entry away, or takes it back out.
///
/// # Errors
///
/// See [`trash`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_trash(
    state: tauri::State<'_, AppState>,
    id: String,
    trashed: bool,
) -> Result<EntrySummary, PasswordsError> {
    trash(&state, &id, trashed, now_us())
}

/// Destroys an entry that is in the bin.
///
/// # Errors
///
/// See [`delete`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_delete(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), PasswordsError> {
    delete(&state, &id, now_us())
}

/// Destroys everything in the bin.
///
/// # Errors
///
/// See [`empty_trash`].
#[tauri::command(async)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn passwords_empty_trash(state: tauri::State<'_, AppState>) -> Result<Emptied, PasswordsError> {
    empty_trash(&state, now_us())
}

/// What the search index has to be told after a write, once it is certain the write happened.
type IndexUpdate = Option<(Uuid, Option<Searchable>)>;

/// Runs a write inside one transaction, and tells the index about it only after it committed.
///
/// Two things this does that [`in_storage`] does not, and both are the reason it exists. The work
/// runs inside a transaction, so an entry never exists with half its addresses: every refusal,
/// including a draft the domain would not accept, leaves the file exactly as it was. And the
/// index is updated after the commit rather than inside it, because an index taught about a write
/// that then rolled back would find an entry that is not there.
///
/// The refusal is carried out past the transaction by hand. The transaction only knows how to
/// roll back on a [`DbError`], and the errors this module refuses with do not all have one, so
/// the real refusal is set aside and a sentinel is returned to make the rollback happen.
fn writing<T>(
    state: &AppState,
    work: impl FnOnce(
        &Storage,
        &FieldCodec<'_>,
        &Connection,
    ) -> Result<(T, IndexUpdate), PasswordsError>,
) -> Result<T, PasswordsError> {
    state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            let mut refused: Option<PasswordsError> = None;

            let committed = storage.database().in_transaction(|transaction| {
                match work(storage, &codec, transaction) {
                    Ok(produced) => Ok(produced),
                    Err(problem) => {
                        refused = Some(problem);
                        // Any error rolls the transaction back, and this one is never seen: the
                        // refusal set aside just above is what the caller is answered with.
                        Err(DbError::NotFound)
                    }
                }
            });

            match committed {
                Ok((produced, update)) => {
                    if let Some((id, entry)) = update {
                        storage.note_entry(id, entry);
                    }
                    Ok(produced)
                }
                Err(cause) => Err(refused.unwrap_or_else(|| PasswordsError::from(cause))),
            }
        })
        .unwrap_or(Err(PasswordsError::Locked))
}

/// Writes the addresses and the custom fields of an entry, in that order.
fn write_children(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    storage: &Storage,
    millis: u64,
    now: i64,
    id: Uuid,
    draft: &EntryDraft,
) -> Result<(), PasswordsError> {
    let urls: Vec<&str> = draft.urls.iter().map(String::as_str).collect();
    repository::replace_urls(
        connection,
        codec,
        storage.device(),
        storage.next_hlc(millis),
        now,
        id,
        &urls,
    )?;

    let fields: Vec<NewField<'_>> = draft
        .fields
        .iter()
        .map(|field| NewField {
            label: &field.label,
            value: &field.value,
            secret: matches!(field.kind, FieldKind::Secret),
        })
        .collect();
    repository::replace_fields(
        connection,
        codec,
        storage.device(),
        storage.next_hlc(millis),
        now,
        id,
        &fields,
    )?;

    Ok(())
}

/// Turns what arrived from the form into something the repository may be handed.
///
/// Two substitutions happen before the judging, and both exist because [`get`] deliberately did
/// not send the value back. A secret field whose value is absent keeps the one it had, paired by
/// position exactly as `replace_fields` pairs them. The password is not substituted here: it is
/// carried verbatim by [`update`], so that a save which does not touch it cannot alter it by
/// being trimmed on the way past.
fn validated(
    draft: &EntryDraftDto,
    existing_fields: &[repository::Field],
) -> Result<ValidEntry, PasswordsError> {
    let folder_id = match draft.folder_id.as_deref() {
        Some(text) => {
            Some(
                Uuid::parse_str(text).map_err(|_not_a_uuid| PasswordsError::Invalid {
                    problems: vec![FieldProblem {
                        field: "folderId".to_owned(),
                        index: None,
                        code: "unknown".to_owned(),
                        limit: None,
                    }],
                })?,
            )
        }
        None => None,
    };

    let fields = draft
        .fields
        .iter()
        .enumerate()
        .map(|(at, field)| DraftField {
            label: field.label.clone(),
            value: kept_value(field, existing_fields.get(at)),
            kind: if field.secret {
                FieldKind::Secret
            } else {
                FieldKind::Text
            },
        })
        .collect();

    EntryDraft {
        kind: draft.kind.kind(),
        title: draft.title.clone(),
        username: draft.username.clone(),
        password: draft.password.clone(),
        notes: draft.notes.clone(),
        urls: draft.urls.clone(),
        fields,
        folder_id,
        favorite: draft.favorite,
    }
    .validate()
    .map_err(|problems| PasswordsError::Invalid {
        problems: problems.iter().map(field_problem).collect(),
    })
}

/// The value a custom field is saved with, which may be the one it already had.
///
/// Absent only means "as it was" on a secret field, and only where the field in that position was
/// also secret. Anywhere else there is nothing to keep: the form was sent the value, so a value
/// it did not send back is a box somebody emptied.
fn kept_value(field: &DraftFieldDto, previous: Option<&repository::Field>) -> String {
    match field.value.as_deref() {
        Some(value) => value.to_owned(),
        None => match previous {
            Some(previous) if field.secret && previous.secret => previous.value.to_string(),
            _nothing_to_keep => String::new(),
        },
    }
}

/// Whether the password of an entry is about to become a different one.
///
/// Compared in constant time, because this runs on every save and a comparison that returns
/// sooner for a password that shares a prefix is a comparison that says how long the prefix is.
fn password_changed(current: Option<&str>, wanted: Option<&str>) -> bool {
    match (current, wanted) {
        (None, None) => false,
        (Some(_), None) | (None, Some(_)) => true,
        (Some(current), Some(wanted)) => !constant_time_eq(current.as_bytes(), wanted.as_bytes()),
    }
}

/// What the index needs to know about an entry that is already in the file.
fn searchable_of_entry(entry: &Entry, addresses: &[repository::Url]) -> Searchable {
    Searchable {
        id: entry.id,
        title: entry.title.clone(),
        username: entry.username.clone(),
        urls: addresses
            .iter()
            .map(|address| address.value.clone())
            .collect(),
    }
}

/// What the index needs to know about an entry that was just written.
fn searchable_of(id: Uuid, draft: &EntryDraft) -> Searchable {
    Searchable {
        id,
        title: Zeroizing::new(draft.title.clone()),
        username: draft.username.clone().map(Zeroizing::new),
        urls: draft
            .urls
            .iter()
            .map(|url| Zeroizing::new(url.clone()))
            .collect(),
    }
}

/// One problem from the domain, as the form names it.
fn field_problem(error: &FieldError) -> FieldProblem {
    let (code, limit) = match error.problem {
        Problem::Missing => ("missing", None),
        Problem::TooLong { limit, .. } => ("tooLong", Some(limit)),
        Problem::TooMany { limit, .. } => ("tooMany", Some(limit)),
        Problem::Control => ("control", None),
    };

    FieldProblem {
        field: error.field.to_owned(),
        index: error.index.map(|at| u32::try_from(at).unwrap_or(u32::MAX)),
        code: code.to_owned(),
        limit: limit.map(|value| u32::try_from(value).unwrap_or(u32::MAX)),
    }
}

/// Runs something that needs the keys and the open database, collapsing the two error types.
pub(crate) fn in_storage<T>(
    state: &AppState,
    work: impl FnOnce(&Storage, &FieldCodec<'_>, &Connection) -> Result<T, PasswordsError>,
) -> Result<T, PasswordsError> {
    let outcome = state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage
                .database()
                .with(|connection| Ok(work(storage, &codec, connection)))
        })
        .ok_or(PasswordsError::Locked)?;

    outcome.map_err(PasswordsError::from)?
}

/// One entry in full, from an open connection.
pub(crate) fn detail_of(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    id: Uuid,
) -> Result<EntryDetail, PasswordsError> {
    let found = repository::entry(connection, codec, id)?.ok_or(PasswordsError::NotFound)?;
    let urls = repository::urls(connection, codec, id)?;
    let fields = repository::fields(connection, codec, id)?;
    let history_len = repository::history_len(connection, id)?;

    Ok(EntryDetail {
        notes: found
            .notes
            .as_deref()
            .map(String::as_str)
            .map(str::to_owned),
        has_password: found.password.is_some(),
        created_at: found.created_at,
        updated_at: found.updated_at,
        summary: summary_of(&found),
        urls: urls
            .iter()
            .map(|url| UrlDto {
                id: url.id.to_string(),
                value: url.value.to_string(),
            })
            .collect(),
        fields: fields
            .iter()
            .map(|field| FieldDto {
                id: field.id.to_string(),
                label: field.label.to_string(),
                // The one line this module exists for.
                value: (!field.secret).then(|| field.value.to_string()),
                secret: field.secret,
            })
            .collect(),
        history_len: u32::try_from(history_len).unwrap_or(u32::MAX),
    })
}

/// What a list shows of one entry.
pub(crate) fn summary_of(entry: &Entry) -> EntrySummary {
    EntrySummary {
        id: entry.id.to_string(),
        kind: EntryKindDto::of(entry.kind),
        title: entry.title.to_string(),
        username: entry
            .username
            .as_deref()
            .map(String::as_str)
            .map(str::to_owned),
        folder_id: entry.folder_id.map(|folder| folder.to_string()),
        favorite: entry.favorite,
        last_used_at: entry.last_used_at,
        trashed: None,
    }
}

/// What the bin shows of one entry.
fn binned_summary(binned: &repository::Trashed) -> EntrySummary {
    EntrySummary {
        id: binned.id.to_string(),
        kind: EntryKindDto::of(binned.kind),
        title: binned.title.to_string(),
        username: None,
        folder_id: None,
        favorite: false,
        last_used_at: None,
        trashed: trashed_dto(binned.state, None),
    }
}

/// Where something in the bin stands, as the screen prints it.
pub(crate) fn trashed_dto(state: TrashState, trashed_at: Option<i64>) -> Option<TrashedDto> {
    match state {
        TrashState::InBin { days, days_left } => Some(TrashedDto {
            trashed_at: trashed_at.unwrap_or_default(),
            days,
            days_left,
        }),
        // Something the sweep has not reached yet. It is in the bin as far as anybody looking at
        // it is concerned, with nothing left on the clock.
        TrashState::Expired => Some(TrashedDto {
            trashed_at: trashed_at.unwrap_or_default(),
            days: cairn_domain::vault::TRASH_DAYS,
            days_left: 0,
        }),
        TrashState::Live | TrashState::Gone => None,
    }
}

/// One folder, as the screen sees it.
fn folder_dto(folder: &repository::Folder) -> FolderDto {
    FolderDto {
        id: folder.id.to_string(),
        name: folder.name.to_string(),
        position: folder.position,
        entries: folder.entries,
    }
}

/// Whether an entry belongs in the list that was asked for.
fn admits(entry: &Entry, folder: Option<Uuid>, favorites: bool) -> bool {
    if favorites && !entry.favorite {
        return false;
    }
    match folder {
        Some(wanted) => entry.folder_id == Some(wanted),
        None => true,
    }
}

/// How long a copied secret stays on the clipboard, or the default when nothing says.
pub(crate) fn clipboard_seconds(
    connection: &Connection,
    codec: &FieldCodec<'_>,
) -> Result<u16, PasswordsError> {
    let Some(stored) = settings::get(connection, codec, CLIPBOARD_SECONDS_KEY)? else {
        return Ok(DEFAULT_CLIPBOARD_SECONDS);
    };
    let Some(value) = stored.value else {
        return Ok(DEFAULT_CLIPBOARD_SECONDS);
    };

    let text = core::str::from_utf8(&value).map_err(|_not_text| PasswordsError::Storage)?;

    text.trim()
        .parse::<u16>()
        .map_err(|_not_a_number| PasswordsError::Storage)
}

/// An identifier that arrived as text.
///
/// Something that is not an identifier cannot name a row, so it is the same answer as one that
/// names nothing: the WebView has no reason to be able to tell a typo from a deletion.
pub(crate) fn parsed(id: &str) -> Result<Uuid, PasswordsError> {
    Uuid::parse_str(id).map_err(|_not_a_uuid| PasswordsError::NotFound)
}

/// A clock reading as the cursor the bridge carries.
fn hlc_to_cursor(hlc: Option<Hlc>) -> Option<String> {
    hlc.map(|reading| {
        reading
            .to_bytes()
            .iter()
            .fold(String::with_capacity(32), |mut text, byte| {
                // `write!` into a `String` cannot fail, and the value it answers with is the one
                // thing here there is nothing sensible to do about, so it is dropped on purpose.
                let _written = write!(text, "{byte:02x}");
                text
            })
    })
}

/// A cursor the bridge handed back, as the clock reading it came from.
///
/// A cursor that is not one is [`PasswordsError::NotFound`], the same as any other identifier
/// that names nothing. Starting from the beginning instead would mean a corrupted cursor silently
/// repeats a page nobody asked for twice.
fn cursor_to_hlc(cursor: &str) -> Result<Hlc, PasswordsError> {
    if cursor.len() != 32 {
        return Err(PasswordsError::NotFound);
    }

    let mut bytes = [0_u8; 16];
    for (at, slot) in bytes.iter_mut().enumerate() {
        let from = at.saturating_mul(2);
        let pair = cursor
            .get(from..from.saturating_add(2))
            .ok_or(PasswordsError::NotFound)?;
        *slot = u8::from_str_radix(pair, 16).map_err(|_not_hex| PasswordsError::NotFound)?;
    }

    Ok(Hlc::from_bytes(bytes))
}
