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

use cairn_db::codec::FieldCodec;
use cairn_db::repositories::settings;
use cairn_db::repositories::vault::{self as repository, Entry, MAX_PAGE};
use cairn_db::search::Matched;
use cairn_db::{Connection, DbError};
use cairn_domain::Hlc;
use cairn_domain::vault::{EntryKind, TrashState};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::clock::now_us;
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
