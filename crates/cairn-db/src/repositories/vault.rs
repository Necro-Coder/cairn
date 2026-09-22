//! The password vault: folders, entries and the history of what each password used to be.
//!
//! Nothing a person reads is in the clear here, which makes this module the opposite of the
//! habits one in the only way that matters: no query can order or filter by anything meaningful,
//! because the database cannot read any of it. What structure there is — the parent of a folder,
//! which entry a row belongs to, a position, a flag, a moment — stays readable so that the shape
//! of the data is still walkable, and everything else is sealed.
//!
//! Searching is therefore not a query. The titles are decrypted once when the vault opens, held
//! in memory, and searched there; that is what [`crate::search`] is for, and why it is a separate
//! module with a separate lifetime. There is no plain text index in the file and there will not
//! be one, because an index over titles is a copy of every title.
//!
//! The password history is capped at [`MAX_HISTORY`] per entry. Trimming marks the oldest as
//! deleted and empties its ciphertext, which is the same shape every deletion in this schema has:
//! the skeleton survives so a merge can see that it went, and the content does not, because a
//! password somebody replaced two years ago is not something this file should still be holding.
//!
//! The addresses and the custom fields of an entry are written whole, every time. A list that
//! arrives replaces the list that was there, matching the rows up by position, so moving an
//! address is two updates rather than four writes in the synchronisation log. What is not here,
//! and will not be, is tags: the tables exist from migration 0003 and nothing reads or writes
//! them, because the module they were for is not one this product has.

use std::collections::HashMap;

use cairn_domain::vault::{
    EntryKind, MAX_FIELD_LABEL_CHARS, MAX_FIELD_VALUE_BYTES, MAX_FIELDS, MAX_FOLDER_NAME_CHARS,
    MAX_URL_CHARS, MAX_URLS, TrashState, trash, validate_folder_name,
};
use cairn_domain::{Hlc, Rev};
use rusqlite::{Connection, OptionalExtension as _, params};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::codec::{FieldCodec, RowKey, SealedColumns};
use crate::device::DeviceId;
use crate::error::DbError;
use crate::row::{RowStamp, StoredStamp, sixteen};
use crate::search::Searchable;

/// The table entries live in.
pub const ENTRIES_TABLE: &str = "vault_entries";

/// The encrypted columns of an entry, in the order the schema declares them.
pub const ENTRIES_SEALED: SealedColumns =
    SealedColumns::new(&["title", "username", "password", "notes"]);

/// The table folders live in.
pub const FOLDERS_TABLE: &str = "vault_folders";

/// The encrypted columns of a folder.
pub const FOLDERS_SEALED: SealedColumns = SealedColumns::new(&["name"]);

/// The table addresses live in.
pub const URLS_TABLE: &str = "vault_urls";

/// Its one encrypted column.
pub const URLS_SEALED: SealedColumns = SealedColumns::new(&["value"]);

/// The table custom fields live in.
pub const FIELDS_TABLE: &str = "vault_fields";

/// Both of its encrypted columns. The label says as much as the value beside it.
pub const FIELDS_SEALED: SealedColumns = SealedColumns::new(&["label", "value"]);

/// The table the old passwords live in.
pub const HISTORY_TABLE: &str = "vault_password_history";

/// The encrypted columns of one old password.
pub const HISTORY_SEALED: SealedColumns = SealedColumns::new(&["password"]);

/// How many old passwords are kept per entry.
///
/// Ten. Enough to recover from a change somebody regrets, and a number rather than no number,
/// because a history with no ceiling is an encrypted table that grows for ever and whose oldest
/// rows are the ones least likely to ever be wanted and most likely to still be valid somewhere.
pub const MAX_HISTORY: usize = 10;

/// The longest a title, a user name or a password may be, in bytes of plaintext.
///
/// A ceiling on what one row may hold, checked here rather than left to the encryption. The
/// column takes whatever it is handed, so without this a single paste could put a hundred
/// megabytes into a file that the vault then has to decrypt on every search.
pub const MAX_VALUE_BYTES: usize = 64 * 1024;

/// The most rows one page may hold.
pub const MAX_PAGE: usize = 200;

/// One entry, as it comes back.
///
/// Every decrypted value clears itself when it is dropped. That is the difference between this
/// and the habit type beside it: there, the name is public and the note is content; here, all
/// four are content, and the type says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The row's identifier.
    pub id: Uuid,
    /// What the entry is called.
    pub title: Zeroizing<String>,
    /// The user name, if it has one.
    pub username: Option<Zeroizing<String>>,
    /// The password, if it has one.
    pub password: Option<Zeroizing<String>>,
    /// The notes, if it has any.
    pub notes: Option<Zeroizing<String>>,
    /// The folder it is in, or `None` for one at the root.
    pub folder_id: Option<Uuid>,
    /// Whether somebody marked it as a favourite.
    pub favorite: bool,
    /// When it was last used, in microseconds since the epoch, or `None` if it never has been.
    pub last_used_at: Option<i64>,
    /// Whether it is an account or a note.
    pub kind: EntryKind,
    /// When it was thrown away, or `None` for one that is not in the bin.
    ///
    /// A state before [`Entry::deleted`] rather than a shade of it: a row in the bin keeps every
    /// byte of its ciphertext, and a deleted one has lost all of it.
    pub trashed_at: Option<i64>,
    /// When the row was first written, in microseconds since the epoch.
    pub created_at: i64,
    /// When it was last written.
    pub updated_at: i64,
    /// Whether it is a tombstone.
    pub deleted: bool,
    /// The clock reading of the last write, which is also where the next page starts.
    pub hlc: Hlc,
}

/// What is needed to write an entry down.
#[derive(Debug, Clone, Copy)]
pub struct NewEntry<'a> {
    /// What to call it.
    pub title: &'a str,
    /// The user name, or nothing.
    pub username: Option<&'a str>,
    /// The password, or nothing.
    pub password: Option<&'a str>,
    /// The notes, or nothing.
    pub notes: Option<&'a str>,
    /// The folder it goes in, or nothing for the root.
    pub folder_id: Option<Uuid>,
    /// Whether it starts as a favourite.
    pub favorite: bool,
}

/// Writes an entry down.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] if the title is empty or any value is longer than
/// [`MAX_VALUE_BYTES`], [`DbError::Sealed`] if a value cannot be encrypted, and
/// [`DbError::Sqlite`] if the statement fails.
pub fn create_entry(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    entry: NewEntry<'_>,
    kind: EntryKind,
) -> Result<Entry, DbError> {
    check_value("the title of an entry", Some(entry.title))?;
    check_value("the user name of an entry", entry.username)?;
    check_value("the password of an entry", entry.password)?;
    check_value("the notes of an entry", entry.notes)?;
    if entry.title.is_empty() {
        return Err(DbError::TooMany {
            what: "the length of an entry title",
            value: 0,
            max: MAX_VALUE_BYTES as u64,
        });
    }

    let stamp = RowStamp::new(device, hlc, now_us)?;
    let sealed = codec.seal_row(
        RowKey {
            table: ENTRIES_TABLE,
            row_id: stamp.id,
            rev: stamp.rev,
        },
        ENTRIES_SEALED,
        &[
            ("title", Some(entry.title.as_bytes())),
            ("username", entry.username.map(str::as_bytes)),
            ("password", entry.password.map(str::as_bytes)),
            ("notes", entry.notes.map(str::as_bytes)),
        ],
    )?;

    connection
        .prepare_cached(
            "INSERT INTO vault_entries
                 (id, created_at, updated_at, device_id, deleted, hlc, rev,
                  title, username, password, notes, folder_id, favorite, last_used_at,
                  kind, trashed_at)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, NULL)",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            sealed.first().and_then(Option::as_ref),
            sealed.get(1).and_then(Option::as_ref),
            sealed.get(2).and_then(Option::as_ref),
            sealed.get(3).and_then(Option::as_ref),
            entry.folder_id.map(|id| id.as_bytes().to_vec()),
            i64::from(entry.favorite),
            kind_as_stored(kind),
        ])?;

    Ok(Entry {
        id: stamp.id,
        title: Zeroizing::new(entry.title.to_owned()),
        username: entry.username.map(|text| Zeroizing::new(text.to_owned())),
        password: entry.password.map(|text| Zeroizing::new(text.to_owned())),
        notes: entry.notes.map(|text| Zeroizing::new(text.to_owned())),
        folder_id: entry.folder_id,
        favorite: entry.favorite,
        last_used_at: None,
        kind,
        trashed_at: None,
        created_at: stamp.created_at,
        updated_at: stamp.updated_at,
        deleted: false,
        hlc: stamp.hlc,
    })
}

/// Reads one entry, answering `None` when it is not there or is a tombstone.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if the row is not the shape the schema describes or a value does
/// not decrypt, and [`DbError::Sqlite`] if the statement fails.
pub fn entry(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    id: Uuid,
) -> Result<Option<Entry>, DbError> {
    let found = connection
        .prepare_cached(&format!(
            "{ENTRY_PROJECTION} WHERE id = ?1 AND deleted = 0 {NOT_IN_THE_BIN} LIMIT 1"
        ))?
        .query_row([id.as_bytes().as_slice()], read_entry)
        .optional()?;

    found.map(|stored| decode_entry(codec, stored)).transpose()
}

/// A page of entries, in clock order, starting after a reading the caller already has.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] if more than [`MAX_PAGE`] rows are asked for, [`DbError::Sealed`]
/// if a row does not decode, and [`DbError::Sqlite`] if the statement fails.
pub fn entries(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    after: Option<Hlc>,
    limit: usize,
) -> Result<Vec<Entry>, DbError> {
    if limit == 0 || limit > MAX_PAGE {
        return Err(DbError::TooMany {
            what: "the size of a page of entries",
            value: limit as u64,
            max: MAX_PAGE as u64,
        });
    }

    let start = after.map_or([0_u8; 16], Hlc::to_bytes);
    let mut statement = connection.prepare_cached(&format!(
        "{ENTRY_PROJECTION} WHERE deleted = 0 {NOT_IN_THE_BIN} AND hlc > ?1 ORDER BY hlc LIMIT ?2"
    ))?;

    let rows = statement
        .query_map(
            params![start.as_slice(), i64::try_from(limit).unwrap_or(i64::MAX)],
            read_entry,
        )?
        .collect::<Result<Vec<_>, _>>()?;

    rows.into_iter()
        .map(|stored| decode_entry(codec, stored))
        .collect()
}

/// Everything about every live entry that can be searched, and whether that is all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Searchables {
    /// One per live entry, in clock order.
    pub entries: Vec<Searchable>,
    /// `false` if the file holds more than [`MAX_TITLES`] live entries and the rest were left
    /// unread, which is a search that cannot find them and has to be said out loud rather than
    /// discovered.
    pub complete: bool,
}

/// The most entries the in-memory index will hold.
///
/// A hundred thousand. The index is the only way this module can be searched at all, so a
/// ceiling here is a ceiling on searching, and it is set far above what a person accumulates in
/// a lifetime of accounts. It exists because the alternative is an unlock whose cost and memory
/// are decided by the size of the file rather than by this program.
pub const MAX_TITLES: usize = 100_000;

/// Everything searchable about every live entry, for the index that is held in memory.
///
/// Opens the title, the user name and the addresses. Not the notes, not the password and not the
/// value of any custom field: the notes are the longest and the most revealing thing an entry
/// holds, and keeping every one of them open so that somebody can search inside them would
/// multiply what a memory dump of this process shows.
///
/// Answers whether it read everything: `false` means the file holds more than [`MAX_TITLES`]
/// live entries and the rest were not read, which is a search that cannot find them and has to
/// be said out loud rather than discovered.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if a value does not open, and [`DbError::Sqlite`] if a statement
/// fails.
pub fn searchable(connection: &Connection, codec: &FieldCodec<'_>) -> Result<Searchables, DbError> {
    let mut statement = connection.prepare_cached(&format!(
        "SELECT id, rev, title, username FROM vault_entries
              WHERE deleted = 0 {NOT_IN_THE_BIN}
              ORDER BY hlc LIMIT ?1"
    ))?;

    // One more than the ceiling, so the answer to "was there more" comes from the same read
    // rather than from a second count that could disagree with it.
    let asked = i64::try_from(MAX_TITLES.saturating_add(1)).unwrap_or(i64::MAX);
    let rows = statement
        .query_map(params![asked], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let complete = rows.len() <= MAX_TITLES;
    let mut addresses = searchable_urls(connection, codec)?;
    let mut listed = Vec::with_capacity(rows.len().min(MAX_TITLES));

    for (id, rev, title, username) in rows.into_iter().take(MAX_TITLES) {
        let row = child_row(ENTRIES_TABLE, &id, rev)?;
        listed.push(Searchable {
            id: row.row_id,
            title: open_optional(codec, row, "title", title)?.ok_or_else(damaged)?,
            username: open_optional(codec, row, "username", username)?,
            urls: addresses.remove(&row.row_id).unwrap_or_default(),
        });
    }

    Ok(Searchables {
        entries: listed,
        complete,
    })
}

/// Every address of every live entry, grouped by the entry it belongs to.
///
/// One statement rather than one per entry. A hundred thousand entries would otherwise be a
/// hundred thousand statements on the unlock, which is the difference between a second and a
/// minute.
fn searchable_urls(
    connection: &Connection,
    codec: &FieldCodec<'_>,
) -> Result<HashMap<Uuid, Vec<Zeroizing<String>>>, DbError> {
    let mut statement = connection.prepare_cached(
        "SELECT u.id, u.rev, u.entry_id, u.value
           FROM vault_urls u
           JOIN vault_entries e ON e.id = u.entry_id
          WHERE u.deleted = 0 AND e.deleted = 0 AND e.trashed_at IS NULL
          ORDER BY u.entry_id, u.position, u.id",
    )?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut grouped: HashMap<Uuid, Vec<Zeroizing<String>>> = HashMap::new();
    for (id, rev, entry_id, value) in rows {
        let row = child_row(URLS_TABLE, &id, rev)?;
        let value = open_optional(codec, row, "value", value)?.ok_or_else(damaged)?;
        grouped
            .entry(Uuid::from_bytes(sixteen(&entry_id)?))
            .or_default()
            .push(value);
    }

    Ok(grouped)
}

/// Saves a whole draft over an entry that already exists.
///
/// Reseals every sealed column at the new revision, because sealing one would leave the others
/// authenticated under the old one. Does **not** touch `trashed_at`, `deleted` or the history:
/// the history is [`replace_password`]'s business and the bin is [`set_trashed`]'s.
///
/// Runs inside whatever transaction the caller has open, for the same reason as everything else
/// that writes an entry: a row saved without its addresses is a state no screen can draw.
///
/// # Errors
///
/// Returns [`DbError::NotFound`] if there is no live entry with that identifier or it is in the
/// bin, [`DbError::TooMany`] if the title is empty or a value is longer than [`MAX_VALUE_BYTES`],
/// [`DbError::Sealed`] if a value cannot be encrypted, and [`DbError::Sqlite`] if the statement
/// fails.
#[expect(
    clippy::too_many_arguments,
    reason = "one more than `create_entry`, which is the row being written over; grouping them would invent a type whose only purpose is to be taken apart again on the next line"
)]
pub fn update_entry(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    id: Uuid,
    entry: NewEntry<'_>,
    kind: EntryKind,
) -> Result<(), DbError> {
    check_value("the title of an entry", Some(entry.title))?;
    check_value("the user name of an entry", entry.username)?;
    check_value("the password of an entry", entry.password)?;
    check_value("the notes of an entry", entry.notes)?;
    if entry.title.is_empty() {
        return Err(DbError::TooMany {
            what: "the length of an entry title",
            value: 0,
            max: MAX_VALUE_BYTES as u64,
        });
    }

    // Through the reader that leaves the bin out, so that editing something somebody threw away
    // is the same answer as editing something that was never there.
    if entry_stamp_outside_the_bin(connection, id)?.is_none() {
        return Err(DbError::NotFound);
    }
    let Some(stored) = read_entry_stamp(connection, id)? else {
        return Err(DbError::NotFound);
    };
    let revised = stored.revised(hlc, now_us);

    let sealed = codec.seal_row(
        RowKey {
            table: ENTRIES_TABLE,
            row_id: revised.id,
            rev: revised.rev,
        },
        ENTRIES_SEALED,
        &[
            ("title", Some(entry.title.as_bytes())),
            ("username", entry.username.map(str::as_bytes)),
            ("password", entry.password.map(str::as_bytes)),
            ("notes", entry.notes.map(str::as_bytes)),
        ],
    )?;

    connection
        .prepare_cached(
            "UPDATE vault_entries
                SET updated_at = ?2, device_id = ?3, hlc = ?4, rev = ?5,
                    title = ?6, username = ?7, password = ?8, notes = ?9,
                    folder_id = ?10, favorite = ?11, kind = ?12
              WHERE id = ?1",
        )?
        .execute(params![
            revised.id.as_bytes().as_slice(),
            revised.updated_at,
            device.as_bytes().as_slice(),
            revised.hlc_as_stored().as_slice(),
            revised.rev_as_stored(),
            sealed.first().and_then(Option::as_ref),
            sealed.get(1).and_then(Option::as_ref),
            sealed.get(2).and_then(Option::as_ref),
            sealed.get(3).and_then(Option::as_ref),
            entry.folder_id.map(|folder| folder.as_bytes().to_vec()),
            i64::from(entry.favorite),
            kind_as_stored(kind),
        ])?;

    Ok(())
}

/// Replaces the password of an entry, keeping the old one in the history.
///
/// Three writes in one call, and they belong together: the entry is revised, the password it had
/// is written to the history, and the history is trimmed. A caller that did the first without the
/// second would lose a password with no way to get it back, which is the failure the history
/// exists for.
///
/// `None` is an entry that has no password from now on. Taking one away is changing it, so the
/// one it had goes to the history exactly as a replacement does; what the column holds afterwards
/// is a literal null rather than an encrypted empty string, because "no password" and "a password
/// with nothing in it" must not be two different rows that draw the same.
///
/// Runs inside whatever transaction the caller has open. It does not start one, because the
/// caller usually has more to do and two transactions around one change is a change that can be
/// half applied.
///
/// # Errors
///
/// Returns [`DbError::NotFound`] if there is no live entry with that identifier,
/// [`DbError::TooMany`] if the new password is longer than [`MAX_VALUE_BYTES`],
/// [`DbError::Sealed`] if a value does not decrypt or cannot be encrypted, and
/// [`DbError::Sqlite`] if a statement fails.
pub fn replace_password(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    id: Uuid,
    password: Option<&str>,
) -> Result<(), DbError> {
    check_value("the password of an entry", password)?;

    let Some(existing) = entry(connection, codec, id)? else {
        return Err(DbError::NotFound);
    };
    let Some(stored) = read_entry_stamp(connection, id)? else {
        return Err(DbError::NotFound);
    };
    let revised = stored.revised(hlc, now_us);

    // The whole row is resealed at the new revision, not just the column that changed.
    let sealed = reseal_entry(
        codec,
        &revised,
        &Entry {
            password: password.map(|text| Zeroizing::new(text.to_owned())),
            ..existing.clone()
        },
    )?;

    connection
        .prepare_cached(
            "UPDATE vault_entries
                SET updated_at = ?2, device_id = ?3, hlc = ?4, rev = ?5,
                    title = ?6, username = ?7, password = ?8, notes = ?9
              WHERE id = ?1",
        )?
        .execute(params![
            revised.id.as_bytes().as_slice(),
            revised.updated_at,
            revised.device.as_bytes().as_slice(),
            revised.hlc_as_stored().as_slice(),
            revised.rev_as_stored(),
            sealed.first().and_then(Option::as_ref),
            sealed.get(1).and_then(Option::as_ref),
            sealed.get(2).and_then(Option::as_ref),
            sealed.get(3).and_then(Option::as_ref),
        ])?;

    // Only if there was one. An entry that had no password has no previous password to keep, and
    // a history row holding nothing would be a row that says a password was replaced when none was.
    if let Some(previous) = existing.password.as_deref() {
        record_previous(connection, codec, device, hlc, now_us, id, previous)?;
        trim_history(connection, hlc, now_us, id)?;
    }

    Ok(())
}

/// How many old passwords an entry has that still hold their ciphertext.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if the statement fails.
pub fn history_len(connection: &Connection, entry_id: Uuid) -> Result<usize, DbError> {
    let counted: i64 = connection
        .prepare_cached(
            "SELECT count(*) FROM vault_password_history
              WHERE entry_id = ?1 AND deleted = 0",
        )?
        .query_row([entry_id.as_bytes().as_slice()], |row| row.get(0))?;

    Ok(usize::try_from(counted).unwrap_or(0))
}

/// One row of the history, without the password in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryMoment {
    /// The row's identifier, which is what asking for one of them takes.
    pub id: Uuid,
    /// When the password it holds stopped being the current one, in microseconds.
    pub replaced_at: i64,
}

/// Whose history to empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryScope {
    /// One entry's.
    Entry(Uuid),
    /// Every entry's, which is the button in the settings screen.
    All,
}

/// When each password of an entry was replaced, newest first. No passwords.
///
/// What the history screen is drawn from. A moment and an identifier are enough to list them and
/// to ask for one; the values stay where they are until somebody asks for one by name. The whole
/// point is what this does not take: there is no codec here, so however this function is called
/// and whatever is wrong inside it, it cannot hand back a password.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if the statement fails.
pub fn history_moments(
    connection: &Connection,
    entry_id: Uuid,
) -> Result<Vec<HistoryMoment>, DbError> {
    let mut statement = connection.prepare_cached(
        "SELECT id, replaced_at FROM vault_password_history
          WHERE entry_id = ?1 AND deleted = 0
          ORDER BY replaced_at DESC, id DESC
          LIMIT ?2",
    )?;

    let rows = statement
        .query_map(
            params![
                entry_id.as_bytes().as_slice(),
                i64::try_from(MAX_HISTORY).unwrap_or(i64::MAX)
            ],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;

    rows.into_iter()
        .map(|(id, replaced_at)| {
            Ok(HistoryMoment {
                id: Uuid::from_bytes(sixteen(&id)?),
                replaced_at,
            })
        })
        .collect()
}

/// One custom field's value, by the identifier a field carried, under the entry named.
///
/// Checks that the field belongs to that entry, and checks it in the `WHERE` rather than
/// afterwards, for the reason [`history_password`] does: filtering in Rust what could have been
/// filtered in SQL means the row was read and decrypted before anybody asked whether the caller
/// was entitled to it.
///
/// The bin is left out here as well. What somebody threw away is not consulted.
///
/// # Errors
///
/// [`DbError::NotFound`] if there is no live field with that identifier under a live entry that
/// is not in the bin, [`DbError::Sealed`] if it does not open, [`DbError::Sqlite`].
pub fn field_value(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    entry_id: Uuid,
    field_id: Uuid,
) -> Result<Zeroizing<String>, DbError> {
    let found: Option<(i64, Option<Vec<u8>>)> = connection
        .prepare_cached(
            "SELECT f.rev, f.value FROM vault_fields AS f
               JOIN vault_entries AS e ON e.id = f.entry_id
              WHERE f.id = ?1 AND f.entry_id = ?2 AND f.deleted = 0
                AND e.deleted = 0 AND e.trashed_at IS NULL
              LIMIT 1",
        )?
        .query_row(
            params![
                field_id.as_bytes().as_slice(),
                entry_id.as_bytes().as_slice()
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    // A row with no ciphertext is a tombstone that has not been compacted away yet. There is
    // nothing inside it, so it is the same answer as a row that is not there.
    let (rev, Some(stored)) = found.ok_or(DbError::NotFound)? else {
        return Err(DbError::NotFound);
    };

    codec.open_text(
        child_row(FIELDS_TABLE, field_id.as_bytes(), rev)?,
        "value",
        &stored,
    )
}

/// Writes down that somebody looked at an entry, and writes down nothing else.
///
/// One cleartext column and no revision. Not raising the revision is deliberate twice over:
/// every encrypted column of a row is authenticated against the revision it was written at, so a
/// write that raised it would have to reseal four values in order to record that somebody glanced
/// at one, and looking at an entry is not an edit that the other device needs to hear about.
///
/// This is the **only** trace a reveal leaves. There is no audit row, no counter and no table of
/// its own: a record of when each password is looked at is a record of somebody's own habits kept
/// inside their own vault, and it would be readable by anything that could read the vault.
///
/// # Errors
///
/// [`DbError::Sqlite`] if the statement fails. An entry that is not there, or is in the bin, is
/// not an error: there was nothing to write down.
pub fn mark_used(connection: &Connection, id: Uuid, now_us: i64) -> Result<(), DbError> {
    connection
        .prepare_cached(
            "UPDATE vault_entries SET last_used_at = ?2
              WHERE id = ?1 AND deleted = 0 AND trashed_at IS NULL",
        )?
        .execute(params![id.as_bytes().as_slice(), now_us])?;

    Ok(())
}

/// One old password, by the identifier a moment carried.
///
/// Checks that the row belongs to the entry named, and checks it in the `WHERE` rather than
/// afterwards. Filtering in Rust what could have been filtered in SQL means the row was read and
/// decrypted before anybody asked whether the caller was entitled to it.
///
/// The bin is left out for the same reason [`field_value`] leaves it out: what somebody threw
/// away is not consulted, and an old password of a thrown away entry is still its password.
///
/// # Errors
///
/// [`DbError::NotFound`] if there is no live row with that identifier under a live entry that is
/// not in the bin, [`DbError::Sealed`] if it does not open, [`DbError::Sqlite`].
pub fn history_password(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    entry_id: Uuid,
    history_id: Uuid,
) -> Result<Zeroizing<String>, DbError> {
    let found: Option<(i64, Option<Vec<u8>>)> = connection
        .prepare_cached(
            "SELECT h.rev, h.password FROM vault_password_history AS h
               JOIN vault_entries AS e ON e.id = h.entry_id
              WHERE h.id = ?1 AND h.entry_id = ?2 AND h.deleted = 0
                AND e.deleted = 0 AND e.trashed_at IS NULL
              LIMIT 1",
        )?
        .query_row(
            params![
                history_id.as_bytes().as_slice(),
                entry_id.as_bytes().as_slice()
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    // A row with no ciphertext is one the cap trimmed. It is a tombstone that has not been
    // compacted away yet and there is nothing inside it, so it is the same answer as a row that
    // is not there.
    let (rev, Some(stored)) = found.ok_or(DbError::NotFound)? else {
        return Err(DbError::NotFound);
    };

    codec.open_text(
        child_row(HISTORY_TABLE, history_id.as_bytes(), rev)?,
        "password",
        &stored,
    )
}

/// Empties the history of one entry, or of every entry.
///
/// Marks the rows deleted and empties their ciphertext, which is what every deletion in this
/// schema does and what makes this an actual erasure rather than a hidden list.
///
/// # Errors
///
/// [`DbError::Sqlite`].
pub fn clear_history(
    connection: &Connection,
    hlc: Hlc,
    now_us: i64,
    scope: HistoryScope,
) -> Result<u32, DbError> {
    // One statement either way. A loop over entries would run the same update once per entry to
    // reach exactly the rows this reaches in one pass.
    let cleared = match scope {
        HistoryScope::Entry(entry_id) => connection
            .prepare_cached(
                "UPDATE vault_password_history
                    SET deleted = 1, updated_at = ?2, hlc = ?3, rev = rev + 1, password = NULL
                  WHERE entry_id = ?1 AND deleted = 0",
            )?
            .execute(params![
                entry_id.as_bytes().as_slice(),
                now_us,
                hlc.to_bytes().as_slice(),
            ])?,
        HistoryScope::All => connection
            .prepare_cached(
                "UPDATE vault_password_history
                    SET deleted = 1, updated_at = ?1, hlc = ?2, rev = rev + 1, password = NULL
                  WHERE deleted = 0",
            )?
            .execute(params![now_us, hlc.to_bytes().as_slice()])?,
    };

    Ok(u32::try_from(cleared).unwrap_or(u32::MAX))
}

/// One address of an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    /// The row's identifier.
    pub id: Uuid,
    /// The address itself, which clears itself when it is dropped.
    pub value: Zeroizing<String>,
    /// Where it sits in the entry's list, counting from zero.
    pub position: i64,
}

/// One custom field of an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The row's identifier.
    pub id: Uuid,
    /// What the field is called.
    pub label: Zeroizing<String>,
    /// What it holds.
    pub value: Zeroizing<String>,
    /// Whether the interface hides it until somebody asks.
    pub secret: bool,
    /// Where it sits in the entry's list, counting from zero.
    pub position: i64,
}

/// What is needed to write one custom field down.
#[derive(Debug, Clone, Copy)]
pub struct NewField<'a> {
    /// What to call it.
    pub label: &'a str,
    /// What it holds.
    pub value: &'a str,
    /// Whether the interface hides it until somebody asks.
    pub secret: bool,
}

/// The addresses of one entry, in the order somebody put them in.
///
/// # Errors
///
/// [`DbError::Sealed`] if a row does not open, [`DbError::Sqlite`] if the statement fails.
pub fn urls(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    entry_id: Uuid,
) -> Result<Vec<Url>, DbError> {
    let mut statement = connection.prepare_cached(
        "SELECT id, rev, value, position FROM vault_urls
          WHERE entry_id = ?1 AND deleted = 0
          ORDER BY position, id",
    )?;

    let rows = statement
        .query_map([entry_id.as_bytes().as_slice()], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut addresses = Vec::with_capacity(rows.len());
    for (id, rev, stored, position) in rows {
        let row = child_row(URLS_TABLE, &id, rev)?;
        addresses.push(Url {
            id: row.row_id,
            value: open_optional(codec, row, "value", stored)?.ok_or_else(damaged)?,
            position,
        });
    }

    Ok(addresses)
}

/// The custom fields of one entry, in order.
///
/// # Errors
///
/// As above.
pub fn fields(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    entry_id: Uuid,
) -> Result<Vec<Field>, DbError> {
    let mut statement = connection.prepare_cached(
        "SELECT id, rev, label, value, secret, position FROM vault_fields
          WHERE entry_id = ?1 AND deleted = 0
          ORDER BY position, id",
    )?;

    let rows = statement
        .query_map([entry_id.as_bytes().as_slice()], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut custom = Vec::with_capacity(rows.len());
    for (id, rev, label, value, secret, position) in rows {
        let row = child_row(FIELDS_TABLE, &id, rev)?;
        custom.push(Field {
            id: row.row_id,
            label: open_optional(codec, row, "label", label)?.ok_or_else(damaged)?,
            value: open_optional(codec, row, "value", value)?.ok_or_else(damaged)?,
            secret: secret != 0,
            position,
        });
    }

    Ok(custom)
}

/// Replaces every address of an entry with the list given, in one go.
///
/// Rows that are no longer in the list are tombstoned the way everything in this schema is:
/// marked deleted and emptied of ciphertext. Rows that stay keep their identifier, so a merge
/// does not see an address leave and a different one arrive when all that happened is that
/// somebody moved it up.
///
/// Runs inside whatever transaction the caller has open, and starts none of its own: writing an
/// entry and its children is one logical write, and an entry with half its addresses is a state
/// no screen knows how to draw and the merge would carry to the other device.
///
/// # Errors
///
/// [`DbError::TooMany`] if there are more than [`cairn_domain::vault::MAX_URLS`] of them or one
/// is longer than it may be, [`DbError::Sealed`], [`DbError::Sqlite`].
pub fn replace_urls(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    entry_id: Uuid,
    values: &[&str],
) -> Result<Vec<Url>, DbError> {
    // Every one of them, before a single statement runs. A refusal halfway through would leave an
    // entry holding the first nineteen addresses of a list that was never acceptable.
    check_count(
        "the number of addresses of an entry",
        values.len(),
        MAX_URLS,
    )?;
    for value in values {
        check_chars("the length of an address", value, MAX_URL_CHARS)?;
    }

    let existing = live_children(
        connection,
        "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev FROM vault_urls
          WHERE entry_id = ?1 AND deleted = 0
          ORDER BY position, id",
        entry_id,
    )?;

    let mut written = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let position = i64::try_from(index).unwrap_or(i64::MAX);
        let previous = existing.get(index);
        let stamp = match previous {
            Some(kept) => kept.revised(hlc, now_us),
            None => RowStamp::new(device, hlc, now_us)?,
        };
        let sealed = codec.seal_row(
            RowKey {
                table: URLS_TABLE,
                row_id: stamp.id,
                rev: stamp.rev,
            },
            URLS_SEALED,
            &[("value", Some(value.as_bytes()))],
        )?;

        if previous.is_some() {
            connection
                .prepare_cached(
                    "UPDATE vault_urls
                        SET updated_at = ?2, device_id = ?3, hlc = ?4, rev = ?5,
                            value = ?6, position = ?7
                      WHERE id = ?1",
                )?
                .execute(params![
                    stamp.id.as_bytes().as_slice(),
                    stamp.updated_at,
                    stamp.device.as_bytes().as_slice(),
                    stamp.hlc_as_stored().as_slice(),
                    stamp.rev_as_stored(),
                    sealed.first().and_then(Option::as_ref),
                    position,
                ])?;
        } else {
            connection
                .prepare_cached(
                    "INSERT INTO vault_urls
                         (id, created_at, updated_at, device_id, deleted, hlc, rev,
                          entry_id, value, position)
                     VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8)",
                )?
                .execute(params![
                    stamp.id.as_bytes().as_slice(),
                    stamp.created_at,
                    stamp.device.as_bytes().as_slice(),
                    stamp.hlc_as_stored().as_slice(),
                    stamp.rev_as_stored(),
                    entry_id.as_bytes().as_slice(),
                    sealed.first().and_then(Option::as_ref),
                    position,
                ])?;
        }

        written.push(Url {
            id: stamp.id,
            value: Zeroizing::new((*value).to_owned()),
            position,
        });
    }

    for spare in existing.iter().skip(values.len()) {
        let gone = spare.tombstoned(hlc, now_us);
        connection
            .prepare_cached(
                "UPDATE vault_urls
                    SET deleted = 1, updated_at = ?2, hlc = ?3, rev = ?4, value = NULL
                  WHERE id = ?1",
            )?
            .execute(params![
                gone.id.as_bytes().as_slice(),
                gone.updated_at,
                gone.hlc_as_stored().as_slice(),
                gone.rev_as_stored(),
            ])?;
    }

    Ok(written)
}

/// Replaces every custom field of an entry, in one go. Same rules as above.
///
/// The `secret` flag is written in the clear beside the two sealed columns, on purpose: it
/// decides how every row is drawn, and a flag that has to be decrypted to know how to draw a row
/// is a decryption on every paint.
///
/// # Errors
///
/// As above, with [`cairn_domain::vault::MAX_FIELDS`].
pub fn replace_fields(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    entry_id: Uuid,
    fields: &[NewField<'_>],
) -> Result<Vec<Field>, DbError> {
    check_count(
        "the number of custom fields of an entry",
        fields.len(),
        MAX_FIELDS,
    )?;
    for field in fields {
        check_chars(
            "the length of the label of a custom field",
            field.label,
            MAX_FIELD_LABEL_CHARS,
        )?;
        check_count(
            "the length of the value of a custom field",
            field.value.len(),
            MAX_FIELD_VALUE_BYTES,
        )?;
    }

    let existing = live_children(
        connection,
        "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev FROM vault_fields
          WHERE entry_id = ?1 AND deleted = 0
          ORDER BY position, id",
        entry_id,
    )?;

    let mut written = Vec::with_capacity(fields.len());
    for (index, field) in fields.iter().enumerate() {
        let position = i64::try_from(index).unwrap_or(i64::MAX);
        let previous = existing.get(index);
        let stamp = match previous {
            Some(kept) => kept.revised(hlc, now_us),
            None => RowStamp::new(device, hlc, now_us)?,
        };

        write_field(
            connection,
            codec,
            &stamp,
            previous.is_some(),
            entry_id,
            field,
            position,
        )?;

        written.push(Field {
            id: stamp.id,
            label: Zeroizing::new(field.label.to_owned()),
            value: Zeroizing::new(field.value.to_owned()),
            secret: field.secret,
            position,
        });
    }

    for spare in existing.iter().skip(fields.len()) {
        let gone = spare.tombstoned(hlc, now_us);
        connection
            .prepare_cached(
                "UPDATE vault_fields
                    SET deleted = 1, updated_at = ?2, hlc = ?3, rev = ?4,
                        label = NULL, value = NULL
                  WHERE id = ?1",
            )?
            .execute(params![
                gone.id.as_bytes().as_slice(),
                gone.updated_at,
                gone.hlc_as_stored().as_slice(),
                gone.rev_as_stored(),
            ])?;
    }

    Ok(written)
}

/// Seals one custom field and writes it, either over the row it is reusing or as a new one.
///
/// Both columns are sealed together at the same revision, because sealing one of the two would
/// leave the other authenticated under the revision before it.
fn write_field(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    stamp: &RowStamp,
    reused: bool,
    entry_id: Uuid,
    field: &NewField<'_>,
    position: i64,
) -> Result<(), DbError> {
    let sealed = codec.seal_row(
        RowKey {
            table: FIELDS_TABLE,
            row_id: stamp.id,
            rev: stamp.rev,
        },
        FIELDS_SEALED,
        &[
            ("label", Some(field.label.as_bytes())),
            ("value", Some(field.value.as_bytes())),
        ],
    )?;

    if reused {
        connection
            .prepare_cached(
                "UPDATE vault_fields
                    SET updated_at = ?2, device_id = ?3, hlc = ?4, rev = ?5,
                        label = ?6, value = ?7, secret = ?8, position = ?9
                  WHERE id = ?1",
            )?
            .execute(params![
                stamp.id.as_bytes().as_slice(),
                stamp.updated_at,
                stamp.device.as_bytes().as_slice(),
                stamp.hlc_as_stored().as_slice(),
                stamp.rev_as_stored(),
                sealed.first().and_then(Option::as_ref),
                sealed.get(1).and_then(Option::as_ref),
                i64::from(field.secret),
                position,
            ])?;
    } else {
        connection
            .prepare_cached(
                "INSERT INTO vault_fields
                     (id, created_at, updated_at, device_id, deleted, hlc, rev,
                      entry_id, label, value, secret, position)
                 VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?
            .execute(params![
                stamp.id.as_bytes().as_slice(),
                stamp.created_at,
                stamp.device.as_bytes().as_slice(),
                stamp.hlc_as_stored().as_slice(),
                stamp.rev_as_stored(),
                entry_id.as_bytes().as_slice(),
                sealed.first().and_then(Option::as_ref),
                sealed.get(1).and_then(Option::as_ref),
                i64::from(field.secret),
                position,
            ])?;
    }

    Ok(())
}

/// One folder, with how many entries are in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// The row's identifier.
    pub id: Uuid,
    /// What it is called.
    pub name: Zeroizing<String>,
    /// Where it sits in the list somebody arranged.
    pub position: i64,
    /// Live entries inside it, not counting what is in the bin.
    pub entries: u32,
}

/// Every folder, in the order somebody arranged them.
///
/// The count beside each one leaves out what is in the bin. A folder that says three and opens
/// on one is worse than a folder that says one, because the first is a bug somebody has to go
/// looking for and the second is the truth.
///
/// # Errors
///
/// [`DbError::Sealed`] if a name does not open, [`DbError::Sqlite`].
pub fn folders(connection: &Connection, codec: &FieldCodec<'_>) -> Result<Vec<Folder>, DbError> {
    let mut statement = connection.prepare_cached(
        "SELECT f.id, f.rev, f.name, f.position,
                (SELECT count(*) FROM vault_entries e
                  WHERE e.folder_id = f.id AND e.deleted = 0 AND e.trashed_at IS NULL)
           FROM vault_folders f
          WHERE f.deleted = 0
          ORDER BY f.position, f.id",
    )?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut listed = Vec::with_capacity(rows.len());
    for (id, rev, name, position, entries) in rows {
        let row = child_row(FOLDERS_TABLE, &id, rev)?;
        listed.push(Folder {
            id: row.row_id,
            name: open_optional(codec, row, "name", name)?.ok_or_else(damaged)?,
            position,
            entries: u32::try_from(entries).unwrap_or(u32::MAX),
        });
    }

    Ok(listed)
}

/// Creates a folder, or renames one.
///
/// `id` names the folder to rename; `None` creates one at the end of the list.
///
/// `parent_id` is written null and there is no way to ask for anything else. The column stays
/// because migration 0003 created it and removing it would be a migration that buys nothing, but
/// this module's folders are a flat list: that is what the phase decided, and a repository that
/// merely happened never to write a parent would be one nesting could creep back into.
///
/// # Errors
///
/// [`DbError::NotFound`] if the folder to rename is not there, [`DbError::TooMany`] if the name
/// is empty or too long, [`DbError::Sealed`], [`DbError::Sqlite`].
pub fn save_folder(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    id: Option<Uuid>,
    name: &str,
) -> Result<Folder, DbError> {
    let name = validate_folder_name(name).map_err(|problem| refused_name(&problem))?;

    let (stamp, position, entries) = match id {
        Some(id) => {
            let Some(stored) = read_folder(connection, id)? else {
                return Err(DbError::NotFound);
            };
            // Renaming leaves a folder exactly where it was. Moving it is `reorder_folders`, and
            // a rename that also reshuffled the list would be a folder changing place because
            // somebody fixed a typo in it.
            (stored.0.revised(hlc, now_us), stored.1, stored.2)
        }
        None => (
            RowStamp::new(device, hlc, now_us)?,
            next_position(connection)?,
            0,
        ),
    };

    let sealed = codec.seal_row(
        RowKey {
            table: FOLDERS_TABLE,
            row_id: stamp.id,
            rev: stamp.rev,
        },
        FOLDERS_SEALED,
        &[("name", Some(name.as_bytes()))],
    )?;

    if id.is_some() {
        connection
            .prepare_cached(
                "UPDATE vault_folders
                    SET updated_at = ?2, hlc = ?3, rev = ?4, name = ?5
                  WHERE id = ?1",
            )?
            .execute(params![
                stamp.id.as_bytes().as_slice(),
                stamp.updated_at,
                stamp.hlc_as_stored().as_slice(),
                stamp.rev_as_stored(),
                sealed.first().and_then(Option::as_ref),
            ])?;
    } else {
        connection
            .prepare_cached(
                "INSERT INTO vault_folders
                     (id, created_at, updated_at, device_id, deleted, hlc, rev,
                      name, parent_id, position)
                 VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, NULL, ?7)",
            )?
            .execute(params![
                stamp.id.as_bytes().as_slice(),
                stamp.created_at,
                stamp.device.as_bytes().as_slice(),
                stamp.hlc_as_stored().as_slice(),
                stamp.rev_as_stored(),
                sealed.first().and_then(Option::as_ref),
                position,
            ])?;
    }

    Ok(Folder {
        id: stamp.id,
        name: Zeroizing::new(name),
        position,
        entries,
    })
}

/// Removes a folder and leaves its entries at the root.
///
/// Never touches an entry beyond its `folder_id`. Deleting a folder is filing, not destroying,
/// and a folder that took thirty passwords with it would be the worst button in the application.
///
/// Takes the codec for the reason [`set_trashed`] does: moving an entry to the root is a write on
/// that entry, a write raises its revision, and every encrypted column of a row is authenticated
/// against the revision it was written at.
///
/// # Errors
///
/// [`DbError::NotFound`], [`DbError::Sealed`], [`DbError::Sqlite`].
pub fn delete_folder(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    hlc: Hlc,
    now_us: i64,
    id: Uuid,
) -> Result<u32, DbError> {
    let Some((stamp, _position, _entries)) = read_folder(connection, id)? else {
        return Err(DbError::NotFound);
    };
    let gone = stamp.tombstoned(hlc, now_us);

    connection
        .prepare_cached(
            "UPDATE vault_folders
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = ?4, name = NULL
              WHERE id = ?1",
        )?
        .execute(params![
            gone.id.as_bytes().as_slice(),
            gone.updated_at,
            gone.hlc_as_stored().as_slice(),
            gone.rev_as_stored(),
        ])?;

    let inside: Vec<Uuid> = {
        let mut statement = connection
            .prepare_cached("SELECT id FROM vault_entries WHERE folder_id = ?1 AND deleted = 0")?;
        let rows = statement
            .query_map([id.as_bytes().as_slice()], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        rows.iter()
            .map(|bytes| sixteen(bytes).map(Uuid::from_bytes))
            .collect::<Result<Vec<_>, _>>()?
    };

    let mut moved = 0_u32;
    for entry_id in inside {
        let (Some(stored), Some(existing)) = (
            read_entry_stamp(connection, entry_id)?,
            read_any_entry(connection, codec, entry_id)?,
        ) else {
            continue;
        };
        let revised = stored.revised(hlc, now_us);
        let sealed = reseal_entry(codec, &revised, &existing)?;

        connection
            .prepare_cached(
                "UPDATE vault_entries
                    SET updated_at = ?2, hlc = ?3, rev = ?4, folder_id = NULL,
                        title = ?5, username = ?6, password = ?7, notes = ?8
                  WHERE id = ?1",
            )?
            .execute(params![
                revised.id.as_bytes().as_slice(),
                revised.updated_at,
                revised.hlc_as_stored().as_slice(),
                revised.rev_as_stored(),
                sealed.first().and_then(Option::as_ref),
                sealed.get(1).and_then(Option::as_ref),
                sealed.get(2).and_then(Option::as_ref),
                sealed.get(3).and_then(Option::as_ref),
            ])?;
        moved = moved.saturating_add(1);
    }

    Ok(moved)
}

/// Writes the whole order at once.
///
/// Refuses a list that is not exactly the set of live folders, which is how a list missing one is
/// caught instead of silently leaving it wherever it was. Nothing is written until the whole list
/// has been checked, so a refused order leaves the arrangement exactly as it was.
///
/// Takes the codec for the reason [`delete_folder`] does.
///
/// # Errors
///
/// [`DbError::IncompleteOrder`] if the list is not the whole set, [`DbError::Sealed`],
/// [`DbError::Sqlite`].
pub fn reorder_folders(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    hlc: Hlc,
    now_us: i64,
    ids: &[Uuid],
) -> Result<(), DbError> {
    let live = folders(connection, codec)?;
    if ids.len() != live.len() {
        return Err(DbError::IncompleteOrder);
    }

    // A repeated identifier passes the length check and leaves one folder unmentioned, which is
    // the same mistake as a missing one and gets the same answer.
    let mut named: Vec<Uuid> = ids.to_vec();
    named.sort_unstable();
    named.dedup();
    let mut known: Vec<Uuid> = live.iter().map(|folder| folder.id).collect();
    known.sort_unstable();
    if named != known {
        return Err(DbError::IncompleteOrder);
    }

    for (index, id) in ids.iter().enumerate() {
        let position = i64::try_from(index).unwrap_or(i64::MAX);
        let Some((stored, _position, _entries)) = read_folder(connection, *id)? else {
            return Err(DbError::IncompleteOrder);
        };
        let Some(folder) = live.iter().find(|folder| folder.id == *id) else {
            return Err(DbError::IncompleteOrder);
        };

        let revised = stored.revised(hlc, now_us);
        let sealed = codec.seal_row(
            RowKey {
                table: FOLDERS_TABLE,
                row_id: revised.id,
                rev: revised.rev,
            },
            FOLDERS_SEALED,
            &[("name", Some(folder.name.as_bytes()))],
        )?;

        connection
            .prepare_cached(
                "UPDATE vault_folders
                    SET updated_at = ?2, hlc = ?3, rev = ?4, name = ?5, position = ?6
                  WHERE id = ?1",
            )?
            .execute(params![
                revised.id.as_bytes().as_slice(),
                revised.updated_at,
                revised.hlc_as_stored().as_slice(),
                revised.rev_as_stored(),
                sealed.first().and_then(Option::as_ref),
                position,
            ])?;
    }

    Ok(())
}

/// The stamp, the position and the entry count of one live folder.
fn read_folder(connection: &Connection, id: Uuid) -> Result<Option<(RowStamp, i64, u32)>, DbError> {
    let found: Option<(StoredStamp, i64, i64)> = connection
        .prepare_cached(
            "SELECT f.id, f.created_at, f.updated_at, f.device_id, f.deleted, f.hlc, f.rev,
                    f.position,
                    (SELECT count(*) FROM vault_entries e
                      WHERE e.folder_id = f.id AND e.deleted = 0 AND e.trashed_at IS NULL)
               FROM vault_folders f
              WHERE f.id = ?1 AND f.deleted = 0
              LIMIT 1",
        )?
        .query_row([id.as_bytes().as_slice()], |row| {
            Ok((RowStamp::read_common(row)?, row.get(7)?, row.get(8)?))
        })
        .optional()?;

    found
        .map(|(stamp, position, entries)| {
            Ok((
                RowStamp::from_stored(stamp)?,
                position,
                u32::try_from(entries).unwrap_or(u32::MAX),
            ))
        })
        .transpose()
}

/// Where the next folder goes, which is after every one there is.
fn next_position(connection: &Connection) -> Result<i64, DbError> {
    let highest: Option<i64> = connection
        .prepare_cached("SELECT max(position) FROM vault_folders WHERE deleted = 0")?
        .query_row([], |row| row.get(0))?;

    Ok(highest.map_or(0, |last| last.saturating_add(1)))
}

/// Turns a refused folder name into the error this crate reports.
fn refused_name(problem: &cairn_domain::vault::FieldError) -> DbError {
    let (value, max) = match problem.problem {
        cairn_domain::vault::Problem::TooLong { limit, actual } => (actual as u64, limit as u64),
        _empty_or_control => (0, MAX_FOLDER_NAME_CHARS as u64),
    };

    DbError::TooMany {
        what: "the name of a folder",
        value,
        max,
    }
}

/// One entry in the bin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trashed {
    /// The row's identifier.
    pub id: Uuid,
    /// What the entry is called, which is all the bin screen draws of it.
    pub title: Zeroizing<String>,
    /// Whether it is an account or a note.
    pub kind: EntryKind,
    /// Where it stands, which is what the screen prints beside it.
    pub state: TrashState,
}

/// How much of the bin to empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    /// Only what is past its thirty days.
    Expired,
    /// The lot.
    All,
}

/// Throws an entry away, or takes it back out.
///
/// The bin is not the deletion. [`delete_entry`] empties every encrypted column of the row, so
/// there is no coming back from it; this only writes a moment into a column that is otherwise
/// null, and the row keeps every byte it had. That is what makes restoring possible at all.
///
/// Idempotent in both directions: throwing away something already in the bin leaves the moment
/// it went in, rather than resetting the thirty days every time somebody clicks twice.
///
/// Takes the codec because a write here is a revision like any other, and every encrypted column
/// of the row is authenticated against the revision it was written at. Raising the revision
/// without resealing them would leave a row whose title stops opening the next time anybody
/// looks at it.
///
/// # Errors
///
/// [`DbError::NotFound`] if there is no live entry with that identifier, [`DbError::Sealed`] if
/// a value does not open or cannot be sealed again, [`DbError::Sqlite`].
pub fn set_trashed(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    hlc: Hlc,
    now_us: i64,
    id: Uuid,
    trashed: bool,
) -> Result<(), DbError> {
    let Some(stored) = read_entry_stamp(connection, id)? else {
        return Err(DbError::NotFound);
    };
    let Some(existing) = read_any_entry(connection, codec, id)? else {
        return Err(DbError::NotFound);
    };

    let revised = stored.revised(hlc, now_us);
    let sealed = reseal_entry(codec, &revised, &existing)?;

    // `coalesce` is what makes the second click do nothing. Clearing is the other direction and
    // says so with a literal null rather than by leaving the column alone.
    let moment = if trashed {
        Some(now_us)
    } else {
        Option::<i64>::None
    };

    connection
        .prepare_cached(
            "UPDATE vault_entries
                SET updated_at = ?2, hlc = ?3, rev = ?4,
                    title = ?5, username = ?6, password = ?7, notes = ?8,
                    trashed_at = CASE WHEN ?9 IS NULL THEN NULL
                                      ELSE coalesce(trashed_at, ?9) END
              WHERE id = ?1",
        )?
        .execute(params![
            revised.id.as_bytes().as_slice(),
            revised.updated_at,
            revised.hlc_as_stored().as_slice(),
            revised.rev_as_stored(),
            sealed.first().and_then(Option::as_ref),
            sealed.get(1).and_then(Option::as_ref),
            sealed.get(2).and_then(Option::as_ref),
            sealed.get(3).and_then(Option::as_ref),
            moment,
        ])?;

    Ok(())
}

/// The entries in the bin, newest first, with how long each has left.
///
/// Opens the title and nothing else. The bin draws a name and a countdown, so the password of
/// something somebody threw away has no reason to be read at all.
///
/// # Errors
///
/// [`DbError::TooMany`] if more than [`MAX_PAGE`] rows are asked for, [`DbError::Sealed`] if a
/// row does not open, [`DbError::Sqlite`].
pub fn trashed(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    now_us: i64,
    limit: usize,
) -> Result<Vec<Trashed>, DbError> {
    check_count("the size of a page of the bin", limit, MAX_PAGE)?;

    let mut statement = connection.prepare_cached(
        "SELECT id, rev, title, kind, trashed_at FROM vault_entries
          WHERE deleted = 0 AND trashed_at IS NOT NULL
          ORDER BY trashed_at DESC, id DESC
          LIMIT ?1",
    )?;

    let rows = statement
        .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut bin = Vec::with_capacity(rows.len());
    for (id, rev, title, kind, trashed_at) in rows {
        let row = child_row(ENTRIES_TABLE, &id, rev)?;
        bin.push(Trashed {
            id: row.row_id,
            title: open_optional(codec, row, "title", title)?.ok_or_else(damaged)?,
            kind: read_kind(kind)?,
            state: trash::state(now_us, Some(trashed_at), false),
        });
    }

    Ok(bin)
}

/// Destroys everything in the bin that has been there longer than it may be, or all of it.
///
/// Called with [`Sweep::Expired`] when the vault opens and with [`Sweep::All`] when somebody
/// presses the button. Each entry destroyed goes through [`delete_entry`], so there is exactly
/// one place in this application where a row loses its contents, and adding a column to the
/// schema cannot leave a second copy of that statement behind still emptying the old six.
///
/// Runs inside whatever transaction the caller has open.
///
/// # Errors
///
/// [`DbError::Sqlite`].
pub fn empty_bin(
    connection: &Connection,
    hlc: Hlc,
    now_us: i64,
    sweep: Sweep,
) -> Result<u32, DbError> {
    // The cutoff comes from the domain, so the thirty days is written once, where the difference
    // between it and the hundred and eighty is explained.
    let cutoff = match sweep {
        Sweep::Expired => trash::bin_cutoff_us(now_us),
        Sweep::All => i64::MAX,
    };

    let doomed: Vec<Uuid> = {
        let mut statement = connection.prepare_cached(
            "SELECT id FROM vault_entries
              WHERE deleted = 0 AND trashed_at IS NOT NULL AND trashed_at <= ?1",
        )?;
        let rows = statement
            .query_map(params![cutoff], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        rows.iter()
            .map(|id| sixteen(id).map(Uuid::from_bytes))
            .collect::<Result<Vec<_>, _>>()?
    };

    let mut destroyed = 0_u32;
    for id in doomed {
        delete_entry(connection, hlc, now_us, id)?;
        destroyed = destroyed.saturating_add(1);
    }

    Ok(destroyed)
}

/// Marks an entry as deleted and empties every encrypted column it has.
///
/// All four, the title included. Decision nine says every encrypted column of the row, without an
/// exception for the one that would have been convenient to keep: a skeleton that still says which
/// bank it was is not a deleted entry, and the person who deleted it has no way to find that out.
/// What the merge gets instead is the identifier and the moment, which is enough to order two
/// versions of a deletion and is all it is owed.
///
/// The history goes with it. An entry whose skeleton survives with ten old passwords still inside
/// it is the same failure one level down.
///
/// # Errors
///
/// Returns [`DbError::NotFound`] if there is no live entry with that identifier, and
/// [`DbError::Sqlite`] if a statement fails.
pub fn delete_entry(
    connection: &Connection,
    hlc: Hlc,
    now_us: i64,
    id: Uuid,
) -> Result<(), DbError> {
    let Some(stamp) = read_entry_stamp(connection, id)? else {
        return Err(DbError::NotFound);
    };
    let gone = stamp.tombstoned(hlc, now_us);

    connection
        .prepare_cached(
            "UPDATE vault_entries
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = ?4,
                    title = NULL, username = NULL, password = NULL, notes = NULL
              WHERE id = ?1",
        )?
        .execute(params![
            gone.id.as_bytes().as_slice(),
            gone.updated_at,
            gone.hlc_as_stored().as_slice(),
            gone.rev_as_stored(),
        ])?;

    connection
        .prepare_cached(
            "UPDATE vault_password_history
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = rev + 1, password = NULL
              WHERE entry_id = ?1 AND deleted = 0",
        )?
        .execute(params![
            gone.id.as_bytes().as_slice(),
            gone.updated_at,
            gone.hlc_as_stored().as_slice(),
        ])?;

    // The addresses and the custom fields go the same way. An entry whose skeleton survives
    // holding the address of the bank and the label "PIN de la tarjeta" has had its title deleted
    // and everything the title was hiding left in place.
    connection
        .prepare_cached(
            "UPDATE vault_urls
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = rev + 1, value = NULL
              WHERE entry_id = ?1 AND deleted = 0",
        )?
        .execute(params![
            gone.id.as_bytes().as_slice(),
            gone.updated_at,
            gone.hlc_as_stored().as_slice(),
        ])?;

    connection
        .prepare_cached(
            "UPDATE vault_fields
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = rev + 1,
                    label = NULL, value = NULL
              WHERE entry_id = ?1 AND deleted = 0",
        )?
        .execute(params![
            gone.id.as_bytes().as_slice(),
            gone.updated_at,
            gone.hlc_as_stored().as_slice(),
        ])?;

    Ok(())
}

/// The columns every entry query reads, in the order [`read_entry`] expects them.
const ENTRY_PROJECTION: &str = "SELECT id, hlc, rev, deleted, title, username, password, notes, \
                                folder_id, favorite, last_used_at, kind, trashed_at, \
                                created_at, updated_at \
                                FROM vault_entries";

/// What every query of this module adds so that the bin stays out of every list but its own.
///
/// Written once and pasted into each statement, because an entry somebody threw away that still
/// turns up in a list is a bin that does nothing.
const NOT_IN_THE_BIN: &str = "AND trashed_at IS NULL";

/// One entry exactly as the projection hands it back.
type StoredEntry = (
    Vec<u8>,
    Vec<u8>,
    i64,
    i64,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    i64,
    Option<i64>,
    i64,
    Option<i64>,
    i64,
    i64,
);

/// Reads the projection above.
fn read_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredEntry> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
    ))
}

/// What the `kind` column holds.
///
/// A value the schema's check makes impossible is reported the way a value that does not open is:
/// the row was written by something that is not this program, and from here the two are the same.
fn read_kind(stored: i64) -> Result<EntryKind, DbError> {
    match stored {
        0 => Ok(EntryKind::Account),
        1 => Ok(EntryKind::Note),
        _other => Err(damaged()),
    }
}

/// What the `kind` column takes.
const fn kind_as_stored(kind: EntryKind) -> i64 {
    match kind {
        EntryKind::Account => 0,
        EntryKind::Note => 1,
    }
}

/// Checks a stored entry and opens its four encrypted columns.
fn decode_entry(codec: &FieldCodec<'_>, stored: StoredEntry) -> Result<Entry, DbError> {
    let (
        id,
        hlc,
        rev,
        deleted,
        title,
        username,
        password,
        notes,
        folder,
        favorite,
        last_used,
        kind,
        trashed_at,
        created_at,
        updated_at,
    ) = stored;

    let id = Uuid::from_bytes(sixteen(&id)?);
    let rev = Rev::from_number(u64::try_from(rev).map_err(|_negative| damaged())?);
    let row = RowKey {
        table: ENTRIES_TABLE,
        row_id: id,
        rev,
    };

    Ok(Entry {
        id,
        // A live row always has one. A null title on a row that is not a tombstone means the
        // file was written by something that is not this program, and it is reported the same
        // way as a value that does not open, because from here it is indistinguishable.
        title: open_optional(codec, row, "title", title)?.ok_or_else(damaged)?,
        username: open_optional(codec, row, "username", username)?,
        password: open_optional(codec, row, "password", password)?,
        notes: open_optional(codec, row, "notes", notes)?,
        folder_id: folder
            .map(|bytes| sixteen(&bytes).map(Uuid::from_bytes))
            .transpose()?,
        favorite: favorite != 0,
        last_used_at: last_used,
        kind: read_kind(kind)?,
        trashed_at,
        created_at,
        updated_at,
        deleted: deleted != 0,
        hlc: Hlc::from_bytes(sixteen(&hlc)?),
    })
}

/// Reads one entry whether or not it is in the bin.
///
/// For the callers that have to see what is in there: the one that takes something back out, the
/// one that reseals a row on its way in, and the one that has to refuse to destroy something that
/// never reached the bin. Every ordinary read of this module goes through [`entry`], which does
/// not see it.
///
/// # Errors
///
/// [`DbError::Sealed`] if a value does not open, [`DbError::Sqlite`] if the statement fails.
pub fn any_entry(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    id: Uuid,
) -> Result<Option<Entry>, DbError> {
    read_any_entry(connection, codec, id)
}

/// Reads one entry whether or not it is in the bin.
fn read_any_entry(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    id: Uuid,
) -> Result<Option<Entry>, DbError> {
    let found = connection
        .prepare_cached(&format!(
            "{ENTRY_PROJECTION} WHERE id = ?1 AND deleted = 0 LIMIT 1"
        ))?
        .query_row([id.as_bytes().as_slice()], read_entry)
        .optional()?;

    found.map(|stored| decode_entry(codec, stored)).transpose()
}

/// Seals all four encrypted columns of an entry at the revision it is being written at.
///
/// All four, every time, and never one of them. Sealing the column that changed would leave the
/// other three authenticated under the revision before it, and the next read of any of them fails
/// with an error that says a value did not decrypt and nothing else.
fn reseal_entry(
    codec: &FieldCodec<'_>,
    stamp: &RowStamp,
    entry: &Entry,
) -> Result<Vec<Option<Vec<u8>>>, DbError> {
    codec.seal_row(
        RowKey {
            table: ENTRIES_TABLE,
            row_id: stamp.id,
            rev: stamp.rev,
        },
        ENTRIES_SEALED,
        &[
            ("title", Some(entry.title.as_bytes())),
            ("username", entry.username.as_deref().map(String::as_bytes)),
            ("password", entry.password.as_deref().map(String::as_bytes)),
            ("notes", entry.notes.as_deref().map(String::as_bytes)),
        ],
    )
}

/// Opens a column that may be absent, keeping absent and empty apart.
fn open_optional(
    codec: &FieldCodec<'_>,
    row: RowKey<'_>,
    column: &str,
    stored: Option<Vec<u8>>,
) -> Result<Option<Zeroizing<String>>, DbError> {
    stored
        .map(|bytes| codec.open_text(row, column, &bytes))
        .transpose()
}

/// Answers with the identifier only if it names a live entry that is not in the bin.
///
/// A row and nothing else: the question is whether the entry may be written to, and reading its
/// sealed columns to find out would decrypt four values in order to throw them away.
fn entry_stamp_outside_the_bin(connection: &Connection, id: Uuid) -> Result<Option<Uuid>, DbError> {
    let found: Option<Vec<u8>> = connection
        .prepare_cached(&format!(
            "SELECT id FROM vault_entries WHERE id = ?1 AND deleted = 0 {NOT_IN_THE_BIN} LIMIT 1"
        ))?
        .query_row([id.as_bytes().as_slice()], |row| row.get(0))
        .optional()?;

    found
        .map(|bytes| Ok(Uuid::from_bytes(sixteen(&bytes)?)))
        .transpose()
}

/// Reads the common columns of a live entry.
fn read_entry_stamp(connection: &Connection, id: Uuid) -> Result<Option<RowStamp>, DbError> {
    let found = connection
        .prepare_cached(
            "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev
               FROM vault_entries
              WHERE id = ?1 AND deleted = 0
              LIMIT 1",
        )?
        .query_row([id.as_bytes().as_slice()], RowStamp::read_common)
        .optional()?;

    found.map(RowStamp::from_stored).transpose()
}

/// Writes one old password into the history.
fn record_previous(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    entry_id: Uuid,
    password: &str,
) -> Result<(), DbError> {
    let stamp = RowStamp::new(device, hlc, now_us)?;
    let sealed = codec.seal_row(
        RowKey {
            table: HISTORY_TABLE,
            row_id: stamp.id,
            rev: stamp.rev,
        },
        HISTORY_SEALED,
        &[("password", Some(password.as_bytes()))],
    )?;

    connection
        .prepare_cached(
            "INSERT INTO vault_password_history
                 (id, created_at, updated_at, device_id, deleted, hlc, rev,
                  entry_id, password, replaced_at)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?2)",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            entry_id.as_bytes().as_slice(),
            sealed.first().and_then(Option::as_ref),
        ])?;

    Ok(())
}

/// Marks everything past the newest [`MAX_HISTORY`] as deleted and empties its ciphertext.
///
/// The emptying is what makes the cap mean anything. A row that is merely marked still holds a
/// password somebody replaced, and a cap that keeps the ciphertext is a cap on what is listed
/// rather than a cap on what is stored.
fn trim_history(
    connection: &Connection,
    hlc: Hlc,
    now_us: i64,
    entry_id: Uuid,
) -> Result<(), DbError> {
    connection
        .prepare_cached(
            "UPDATE vault_password_history
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = rev + 1, password = NULL
              WHERE id IN (
                  SELECT id FROM vault_password_history
                   WHERE entry_id = ?1 AND deleted = 0
                   ORDER BY replaced_at DESC, id DESC
                   LIMIT -1 OFFSET ?4
              )",
        )?
        .execute(params![
            entry_id.as_bytes().as_slice(),
            now_us,
            hlc.to_bytes().as_slice(),
            i64::try_from(MAX_HISTORY).unwrap_or(i64::MAX),
        ])?;

    Ok(())
}

/// The identity of one child row of an entry, checked on the way out of the file.
///
/// Both the identifier and the revision come from columns this program wrote, so a length that
/// is not sixteen or a revision that is negative means the row was written by something else.
fn child_row<'a>(table: &'a str, id: &[u8], rev: i64) -> Result<RowKey<'a>, DbError> {
    Ok(RowKey {
        table,
        row_id: Uuid::from_bytes(sixteen(id)?),
        rev: Rev::from_number(u64::try_from(rev).map_err(|_negative| damaged())?),
    })
}

/// The stamps of the live child rows of an entry, in the order the statement asks for.
///
/// What the two replacements pair the incoming list against. The statement is a literal from this
/// file, never text that came from anywhere else.
fn live_children(
    connection: &Connection,
    statement: &str,
    entry_id: Uuid,
) -> Result<Vec<RowStamp>, DbError> {
    let mut prepared = connection.prepare_cached(statement)?;
    let rows = prepared
        .query_map([entry_id.as_bytes().as_slice()], RowStamp::read_common)?
        .collect::<Result<Vec<_>, _>>()?;

    rows.into_iter().map(RowStamp::from_stored).collect()
}

/// Refuses a count larger than one call may carry.
///
/// The ceilings are the domain's, checked again here. That is not duplication with a different
/// name: the domain protects whoever is typing from their own mistake, and this protects the file
/// from a caller written two years from now that never went through a form at all.
fn check_count(what: &'static str, value: usize, max: usize) -> Result<(), DbError> {
    if value > max {
        return Err(DbError::TooMany {
            what,
            value: value as u64,
            max: max as u64,
        });
    }

    Ok(())
}

/// Refuses a value longer than the number of characters it may be.
fn check_chars(what: &'static str, value: &str, max: usize) -> Result<(), DbError> {
    check_count(what, value.chars().count(), max)
}

/// Refuses a value larger than one row may hold.
fn check_value(what: &'static str, value: Option<&str>) -> Result<(), DbError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.len() > MAX_VALUE_BYTES {
        return Err(DbError::TooMany {
            what,
            value: value.len() as u64,
            max: MAX_VALUE_BYTES as u64,
        });
    }

    Ok(())
}

/// What a row this application did not write is reported as.
fn damaged() -> DbError {
    DbError::Sealed(cairn_crypto::CryptoError::Open)
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};
    use cairn_domain::Hlc;
    use cairn_domain::vault::{
        EntryKind, MAX_FIELDS, MAX_FOLDER_NAME_CHARS, MAX_URL_CHARS, MAX_URLS, TrashState,
    };
    use uuid::Uuid;

    use super::{
        HistoryScope, MAX_HISTORY, NewEntry, NewField, Sweep, clear_history, create_entry,
        delete_entry, delete_folder, empty_bin, entries, entry, fields, folders, history_len,
        history_moments, history_password, reorder_folders, replace_fields, replace_password,
        replace_urls, save_folder, searchable, set_trashed, trashed, update_entry, urls,
    };
    use crate::codec::FieldCodec;
    use crate::device::DeviceId;
    use crate::error::DbError;
    use crate::migrations;
    use crate::open::Database;
    use crate::test_support::Scratch;

    const NOW_US: i64 = 1_700_000_000_000_000;

    fn an_open_vault() -> UnlockedVault {
        let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");
        vault
    }

    fn a_database(scratch: &Scratch, vault: &UnlockedVault) -> Database {
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        database
    }

    fn at(step: u64) -> Hlc {
        Hlc::new(step, 0, [1; 6])
    }

    /// Writes an ordinary account, which is what all but one of these tests are about.
    ///
    /// The kind is the one argument none of them vary, so it is fixed here rather than repeated
    /// forty times in a position where a reader would have to check it every time.
    fn an_account(
        connection: &rusqlite::Connection,
        codec: &FieldCodec<'_>,
        device: DeviceId,
        hlc: Hlc,
        now_us: i64,
        entry: NewEntry<'_>,
    ) -> Result<super::Entry, DbError> {
        create_entry(
            connection,
            codec,
            device,
            hlc,
            now_us,
            entry,
            EntryKind::Account,
        )
    }

    fn an_entry(title: &str) -> NewEntry<'_> {
        NewEntry {
            title,
            username: Some("alguien@ejemplo"),
            password: Some("contraseña-uno"),
            notes: Some("una nota"),
            folder_id: None,
            favorite: false,
        }
    }

    #[test]
    fn what_is_written_comes_back() {
        let scratch = Scratch::new("vault-round-trip");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    DeviceId::generate()?,
                    at(1),
                    NOW_US,
                    an_entry("Banco"),
                )?;

                let read = entry(connection, &codec, written.id)?.expect("it was just written");
                assert_eq!(read.title.as_str(), "Banco");
                assert_eq!(
                    read.username.as_deref().map(String::as_str),
                    Some("alguien@ejemplo")
                );
                assert_eq!(
                    read.password.as_deref().map(String::as_str),
                    Some("contraseña-uno")
                );
                assert_eq!(read.notes.as_deref().map(String::as_str), Some("una nota"));
                assert!(!read.favorite);
                assert_eq!(read.last_used_at, None);
                Ok(())
            })
            .expect("the round trip works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn nothing_a_person_reads_is_in_the_file_in_the_clear() {
        // The claim this whole module rests on, checked against the bytes rather than trusted.
        // The title is in it too, unlike a habit name: in this module even the name is content.
        let scratch = Scratch::new("vault-opaque");
        let vault = an_open_vault();
        let path = scratch.database_path();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                an_account(
                    connection,
                    &codec,
                    DeviceId::generate()?,
                    at(1),
                    NOW_US,
                    NewEntry {
                        title: "cairn-canary-title",
                        username: Some("cairn-canary-user"),
                        password: Some("cairn-canary-password"),
                        notes: Some("cairn-canary-note"),
                        folder_id: None,
                        favorite: false,
                    },
                )?;
                Ok(())
            })
            .expect("the entry is written");
        database.close().expect("the connection closes");

        let raw = std::fs::read(&path).expect("the file is readable as bytes");
        for needle in [
            b"cairn-canary-title".as_slice(),
            b"cairn-canary-user",
            b"cairn-canary-password",
            b"cairn-canary-note",
        ] {
            assert!(
                !raw.windows(needle.len()).any(|window| window == needle),
                "something readable appeared in the file"
            );
        }
    }

    #[test]
    fn replacing_a_password_keeps_the_old_one_and_the_rest_of_the_row_still_opens() {
        let scratch = Scratch::new("vault-replace");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let written =
                    an_account(connection, &codec, device, at(1), NOW_US, an_entry("Banco"))?;

                replace_password(
                    connection,
                    &codec,
                    device,
                    at(2),
                    NOW_US + 1,
                    written.id,
                    Some("contraseña-dos"),
                )?;

                // The whole row is resealed, so the three columns that did not change have to
                // still open at the new revision. This is the assertion that catches a partial
                // reseal, and a partial reseal is the most likely way to break this module.
                let read = entry(connection, &codec, written.id)?.expect("it is still there");
                assert_eq!(
                    read.password.as_deref().map(String::as_str),
                    Some("contraseña-dos")
                );
                assert_eq!(read.title.as_str(), "Banco");
                assert_eq!(read.notes.as_deref().map(String::as_str), Some("una nota"));

                let kept = history_moments(connection, written.id)?;
                assert_eq!(kept.len(), 1);
                let was = kept.first().map(|moment| moment.id).expect("one moment");
                assert_eq!(
                    history_password(connection, &codec, written.id, was)?.as_str(),
                    "contraseña-uno"
                );
                Ok(())
            })
            .expect("the replacement works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_history_stops_at_ten_and_the_trimmed_ones_keep_no_ciphertext() {
        // Decision sixteen, in one test. The cap is not about what is listed, it is about what is
        // stored: a row that is merely marked still holds a password somebody replaced, and on a
        // machine somebody else later gets hold of that is the whole point of the cap lost.
        let scratch = Scratch::new("vault-history-cap");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let written =
                    an_account(connection, &codec, device, at(1), NOW_US, an_entry("Banco"))?;

                for step in 2..=20_u64 {
                    replace_password(
                        connection,
                        &codec,
                        device,
                        at(step),
                        NOW_US + i64::try_from(step).unwrap_or(0),
                        written.id,
                        Some(&format!("contraseña-{step}")),
                    )?;
                }

                assert_eq!(history_len(connection, written.id)?, MAX_HISTORY);
                assert_eq!(history_moments(connection, written.id)?.len(), MAX_HISTORY);

                let (trimmed, with_content): (i64, i64) = connection.query_row(
                    "SELECT count(*), count(password) FROM vault_password_history
                      WHERE entry_id = ?1 AND deleted = 1",
                    [written.id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                assert!(trimmed > 0, "nothing was trimmed");
                assert_eq!(with_content, 0, "a trimmed password kept its ciphertext");
                Ok(())
            })
            .expect("the cap holds");

        database.close().expect("the connection closes");
    }

    #[test]
    fn an_entry_with_no_password_gains_no_history_when_one_is_set() {
        let scratch = Scratch::new("vault-first-password");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    device,
                    at(1),
                    NOW_US,
                    NewEntry {
                        title: "Banco",
                        username: None,
                        password: None,
                        notes: None,
                        folder_id: None,
                        favorite: false,
                    },
                )?;

                replace_password(
                    connection,
                    &codec,
                    device,
                    at(2),
                    NOW_US + 1,
                    written.id,
                    Some("la primera"),
                )?;

                assert_eq!(
                    history_len(connection, written.id)?,
                    0,
                    "setting the first password wrote a history row for a password that never existed"
                );
                Ok(())
            })
            .expect("the first password is not a replacement");

        database.close().expect("the connection closes");
    }

    #[test]
    fn deleting_an_entry_empties_it_and_its_history() {
        let scratch = Scratch::new("vault-delete");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let written =
                    an_account(connection, &codec, device, at(1), NOW_US, an_entry("Banco"))?;
                replace_password(
                    connection,
                    &codec,
                    device,
                    at(2),
                    NOW_US + 1,
                    written.id,
                    Some("contraseña-dos"),
                )?;

                delete_entry(connection, at(3), NOW_US + 2, written.id)?;

                assert_eq!(entry(connection, &codec, written.id)?, None);
                assert_eq!(entries(connection, &codec, None, 10)?.len(), 0);

                // All four, the title included. Decision nine says every encrypted column of the
                // row, and the title is the one somebody would be tempted to make an exception
                // for, so it is the one this assertion exists to guard.
                let left: i64 = connection.query_row(
                    "SELECT count(title) + count(username) + count(password) + count(notes)
                       FROM vault_entries WHERE id = ?1",
                    [written.id.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;
                assert_eq!(left, 0, "a tombstone kept one of its encrypted columns");

                let (deleted, rev): (i64, i64) = connection.query_row(
                    "SELECT deleted, rev FROM vault_entries WHERE id = ?1",
                    [written.id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                assert_eq!(deleted, 1, "the row was removed rather than marked");
                assert_eq!(rev, 2, "the revision did not move on the deletion");

                let left: i64 = connection.query_row(
                    "SELECT count(password) FROM vault_password_history WHERE entry_id = ?1",
                    [written.id.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;
                assert_eq!(
                    left, 0,
                    "the history of a deleted entry kept its ciphertext"
                );
                Ok(())
            })
            .expect("the deletion works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_page_of_entries_walks_the_list_once_and_stops() {
        let scratch = Scratch::new("vault-page");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                for step in 1..=7_u64 {
                    an_account(
                        connection,
                        &codec,
                        device,
                        at(step),
                        NOW_US,
                        an_entry(&format!("Entrada {step}")),
                    )?;
                }

                let mut seen = 0_usize;
                let mut cursor: Option<Hlc> = None;
                loop {
                    let page = entries(connection, &codec, cursor, 3)?;
                    if page.is_empty() {
                        break;
                    }
                    seen += page.len();
                    cursor = page.last().map(|row| row.hlc);
                }

                assert_eq!(seen, 7);
                Ok(())
            })
            .expect("the walk finishes");

        database.close().expect("the connection closes");
    }

    /// Everything one test of the address and field tables needs, set up the same way each time.
    struct Bench {
        scratch: Scratch,
        vault: UnlockedVault,
        device: DeviceId,
    }

    impl Bench {
        fn new(label: &str) -> Self {
            Self {
                scratch: Scratch::new(label),
                vault: an_open_vault(),
                device: DeviceId::from_bytes([7; 16]),
            }
        }

        fn database(&self) -> Database {
            a_database(&self.scratch, &self.vault)
        }

        fn codec(&self) -> FieldCodec<'_> {
            FieldCodec::new(self.vault.data_key(), *self.vault.key_id())
        }
    }

    /// How many rows of a table belong to an entry, and how many of them still hold ciphertext.
    fn counted(connection: &rusqlite::Connection, statement: &str, entry_id: Uuid) -> (i64, i64) {
        connection
            .query_row(statement, [entry_id.as_bytes().as_slice()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("the rows can be counted")
    }

    /// The rows of `vault_urls` for an entry: how many there are, and how many still hold a value.
    const URL_CENSUS: &str = "SELECT count(*), count(value) FROM vault_urls WHERE entry_id = ?1";

    /// The same for `vault_fields`, counting a row as holding something if either column does.
    const FIELD_CENSUS: &str =
        "SELECT count(*), count(label) + count(value) FROM vault_fields WHERE entry_id = ?1";

    #[test]
    fn the_addresses_of_an_entry_are_written_in_the_order_they_arrived() {
        let bench = Bench::new("vault-urls-write");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Banco"),
                )?;
                let addresses = replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    written.id,
                    &["uno.es", "dos.es", "tres.es"],
                )?;

                assert_eq!(
                    addresses.iter().map(|url| url.position).collect::<Vec<_>>(),
                    vec![0, 1, 2]
                );
                assert_eq!(
                    urls(connection, &codec, written.id)?
                        .iter()
                        .map(|url| url.value.to_string())
                        .collect::<Vec<_>>(),
                    vec!["uno.es", "dos.es", "tres.es"]
                );
                Ok(())
            })
            .expect("the addresses are written");

        database.close().expect("the connection closes");
    }

    #[test]
    fn rewriting_the_same_addresses_in_another_order_reuses_the_same_rows() {
        // The reason the rows are paired up by position rather than replaced wholesale. Four
        // writes in the synchronisation log where two would do is four rows two devices have to
        // reconcile, for a change that was somebody dragging a line up a list.
        let bench = Bench::new("vault-urls-reorder");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Banco"),
                )?;
                let first = replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    written.id,
                    &["uno.es", "dos.es", "tres.es"],
                )?;
                let second = replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(3),
                    NOW_US + 1,
                    written.id,
                    &["tres.es", "uno.es", "dos.es"],
                )?;

                let before: Vec<Uuid> = first.iter().map(|url| url.id).collect();
                let after: Vec<Uuid> = second.iter().map(|url| url.id).collect();
                assert_eq!(
                    before, after,
                    "reordering created rows instead of moving them"
                );

                assert_eq!(
                    urls(connection, &codec, written.id)?
                        .iter()
                        .map(|url| url.value.to_string())
                        .collect::<Vec<_>>(),
                    vec!["tres.es", "uno.es", "dos.es"]
                );
                assert_eq!(counted(connection, URL_CENSUS, written.id), (3, 3));
                Ok(())
            })
            .expect("the reorder works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_shorter_list_tombstones_the_rows_that_are_left_over_and_empties_them() {
        let bench = Bench::new("vault-urls-shorter");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Banco"),
                )?;
                replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    written.id,
                    &["uno.es", "dos.es", "tres.es"],
                )?;
                replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(3),
                    NOW_US + 1,
                    written.id,
                    &["uno.es", "dos.es"],
                )?;

                assert_eq!(urls(connection, &codec, written.id)?.len(), 2);
                // Three rows, two of them still holding a value: the third kept its skeleton so a
                // merge can see it went, and lost its ciphertext so the service is not still named.
                assert_eq!(counted(connection, URL_CENSUS, written.id), (3, 2));
                Ok(())
            })
            .expect("the shorter list works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_longer_list_adds_rows_and_an_empty_one_takes_them_all_away() {
        let bench = Bench::new("vault-urls-longer");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Banco"),
                )?;
                let first = replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    written.id,
                    &["uno.es", "dos.es", "tres.es"],
                )?;
                let grown = replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(3),
                    NOW_US + 1,
                    written.id,
                    &["uno.es", "dos.es", "tres.es", "cuatro.es"],
                )?;

                assert_eq!(grown.len(), 4);
                let known: Vec<Uuid> = first.iter().map(|url| url.id).collect();
                assert!(
                    grown.last().is_some_and(|url| !known.contains(&url.id)),
                    "the fourth address reused an identifier instead of getting its own"
                );

                replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(4),
                    NOW_US + 2,
                    written.id,
                    &[],
                )?;

                assert!(urls(connection, &codec, written.id)?.is_empty());
                assert_eq!(counted(connection, URL_CENSUS, written.id), (4, 0));
                Ok(())
            })
            .expect("the longer list and the empty one both work");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_list_that_is_refused_writes_nothing_at_all() {
        // Checked before a statement runs, not while they are being written. A refusal halfway
        // through would leave an entry holding the first nineteen of a list nobody accepted.
        let bench = Bench::new("vault-urls-refused");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Banco"),
                )?;

                let many: Vec<String> = (0..=MAX_URLS).map(|n| format!("sitio{n}.es")).collect();
                let borrowed: Vec<&str> = many.iter().map(String::as_str).collect();
                let refused = replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    written.id,
                    &borrowed,
                )
                .expect_err("thirty-three addresses were accepted");
                assert!(matches!(refused, DbError::TooMany { .. }));
                assert_eq!(counted(connection, URL_CENSUS, written.id), (0, 0));

                let long: String = std::iter::repeat_n('a', MAX_URL_CHARS + 1).collect();
                let refused = replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(3),
                    NOW_US,
                    written.id,
                    &[long.as_str()],
                )
                .expect_err("an address past the ceiling was accepted");
                assert!(matches!(refused, DbError::TooMany { .. }));
                assert_eq!(counted(connection, URL_CENSUS, written.id), (0, 0));
                Ok(())
            })
            .expect("both refusals leave the file alone");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_reused_row_moves_on_a_revision_and_carries_the_reading_it_was_given() {
        let bench = Bench::new("vault-urls-revision");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Banco"),
                )?;
                replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    written.id,
                    &["uno.es"],
                )?;
                let (before, _hlc): (i64, Vec<u8>) = connection.query_row(
                    "SELECT rev, hlc FROM vault_urls WHERE entry_id = ?1",
                    [written.id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;

                replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(9),
                    NOW_US + 1,
                    written.id,
                    &["otro.es"],
                )?;
                let (after, hlc): (i64, Vec<u8>) = connection.query_row(
                    "SELECT rev, hlc FROM vault_urls WHERE entry_id = ?1",
                    [written.id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;

                assert!(after > before, "a reused row did not move on a revision");
                assert_eq!(hlc, at(9).to_bytes().to_vec());

                // And it still opens, which is the assertion a partial reseal would fail.
                assert_eq!(
                    urls(connection, &codec, written.id)?
                        .first()
                        .map(|url| url.value.to_string()),
                    Some("otro.es".to_owned())
                );
                Ok(())
            })
            .expect("the revision moves");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_addresses_of_one_entry_are_not_the_addresses_of_another() {
        let bench = Bench::new("vault-urls-isolated");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let one = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Uno"),
                )?;
                let two = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    an_entry("Dos"),
                )?;

                replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(3),
                    NOW_US,
                    one.id,
                    &["uno.es"],
                )?;
                replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(4),
                    NOW_US,
                    two.id,
                    &["dos.es"],
                )?;
                replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(5),
                    NOW_US + 1,
                    one.id,
                    &[],
                )?;

                assert!(urls(connection, &codec, one.id)?.is_empty());
                assert_eq!(urls(connection, &codec, two.id)?.len(), 1);
                Ok(())
            })
            .expect("one entry's addresses are its own");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_custom_fields_of_an_entry_behave_the_way_its_addresses_do() {
        let bench = Bench::new("vault-fields-write");
        let database = bench.database();
        let codec = bench.codec();

        let three = [
            NewField {
                label: "PIN",
                value: "1234",
                secret: true,
            },
            NewField {
                label: "Oficina",
                value: "Central",
                secret: false,
            },
            NewField {
                label: "Gestor",
                value: "Alguien",
                secret: false,
            },
        ];

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Banco"),
                )?;

                let first = replace_fields(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    written.id,
                    &three,
                )?;
                assert_eq!(
                    first.iter().map(|field| field.position).collect::<Vec<_>>(),
                    vec![0, 1, 2]
                );

                // The flag is in the clear beside the sealed columns, and comes back as it went.
                let read = fields(connection, &codec, written.id)?;
                assert_eq!(
                    read.iter().map(|field| field.secret).collect::<Vec<_>>(),
                    vec![true, false, false]
                );
                let (_rows, plain): (i64, i64) = connection.query_row(
                    "SELECT count(*), sum(secret) FROM vault_fields WHERE entry_id = ?1",
                    [written.id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                assert_eq!(plain, 1, "the flag is not readable without the key");

                // Shorter, then longer, then empty: the same three shapes as the addresses.
                let shorter = replace_fields(
                    connection,
                    &codec,
                    bench.device,
                    at(3),
                    NOW_US + 1,
                    written.id,
                    three.get(..2).unwrap_or_default(),
                )?;
                assert_eq!(
                    shorter.iter().map(|field| field.id).collect::<Vec<_>>(),
                    first
                        .iter()
                        .take(2)
                        .map(|field| field.id)
                        .collect::<Vec<_>>()
                );
                assert_eq!(counted(connection, FIELD_CENSUS, written.id), (3, 4));

                replace_fields(
                    connection,
                    &codec,
                    bench.device,
                    at(4),
                    NOW_US + 2,
                    written.id,
                    &[],
                )?;
                assert!(fields(connection, &codec, written.id)?.is_empty());
                assert_eq!(counted(connection, FIELD_CENSUS, written.id), (3, 0));
                Ok(())
            })
            .expect("the fields behave like the addresses");

        database.close().expect("the connection closes");
    }

    #[test]
    fn two_hundred_and_fifty_seven_custom_fields_are_refused_and_write_nothing() {
        let bench = Bench::new("vault-fields-ceiling");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Banco"),
                )?;

                let labels: Vec<String> = (0..=MAX_FIELDS).map(|n| format!("campo {n}")).collect();
                let many: Vec<NewField<'_>> = labels
                    .iter()
                    .map(|label| NewField {
                        label,
                        value: "x",
                        secret: false,
                    })
                    .collect();

                assert!(
                    replace_fields(
                        connection,
                        &codec,
                        bench.device,
                        at(2),
                        NOW_US,
                        written.id,
                        many.get(..MAX_FIELDS).unwrap_or_default(),
                    )
                    .is_ok()
                );

                let refused = replace_fields(
                    connection,
                    &codec,
                    bench.device,
                    at(3),
                    NOW_US + 1,
                    written.id,
                    &many,
                )
                .expect_err("one field past the ceiling was accepted");
                assert!(matches!(refused, DbError::TooMany { .. }));
                assert_eq!(fields(connection, &codec, written.id)?.len(), MAX_FIELDS);
                Ok(())
            })
            .expect("the ceiling holds");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_label_and_a_value_with_accents_and_an_emoji_come_back_byte_for_byte() {
        let bench = Bench::new("vault-fields-unicode");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let written = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Banco"),
                )?;
                replace_fields(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    written.id,
                    &[NewField {
                        label: "Contraseña del móvil 📱",
                        value: "ñandú-café-🔑",
                        secret: true,
                    }],
                )?;
                replace_urls(
                    connection,
                    &codec,
                    bench.device,
                    at(3),
                    NOW_US,
                    written.id,
                    &["https://señor.example/año?q=ñ#📌"],
                )?;

                let read = fields(connection, &codec, written.id)?;
                assert_eq!(
                    read.first().map(|field| field.label.to_string()),
                    Some("Contraseña del móvil 📱".to_owned())
                );
                assert_eq!(
                    read.first().map(|field| field.value.to_string()),
                    Some("ñandú-café-🔑".to_owned())
                );
                assert_eq!(
                    urls(connection, &codec, written.id)?
                        .first()
                        .map(|url| url.value.to_string()),
                    Some("https://señor.example/año?q=ñ#📌".to_owned())
                );
                Ok(())
            })
            .expect("nothing is rewritten on the way through");

        database.close().expect("the connection closes");
    }

    /// An entry whose password has been replaced that many times.
    fn with_replacements(
        connection: &rusqlite::Connection,
        codec: &FieldCodec<'_>,
        device: DeviceId,
        title: &str,
        times: u64,
    ) -> Result<Uuid, DbError> {
        let written = an_account(connection, codec, device, at(1), NOW_US, an_entry(title))?;
        for step in 1..=times {
            replace_password(
                connection,
                codec,
                device,
                at(step + 1),
                NOW_US + i64::try_from(step).unwrap_or(0),
                written.id,
                Some(&format!("contraseña-{step}")),
            )?;
        }

        Ok(written.id)
    }

    #[test]
    fn the_history_lists_moments_and_never_a_password() {
        let bench = Bench::new("vault-history-moments");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let bare = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    an_entry("Sin historial"),
                )?;
                assert!(history_moments(connection, bare.id)?.is_empty());

                let id = with_replacements(connection, &codec, bench.device, "Banco", 3)?;
                let moments = history_moments(connection, id)?;
                assert_eq!(moments.len(), 3);

                // Newest first, which is the order the screen draws them in.
                let times: Vec<i64> = moments.iter().map(|moment| moment.replaced_at).collect();
                let mut sorted = times.clone();
                sorted.sort_unstable_by(|left, right| right.cmp(left));
                assert_eq!(times, sorted);

                // The type carries an identifier and a moment, and there is nowhere in it for a
                // password to be. The assertion is the field list itself.
                for moment in &moments {
                    let super::HistoryMoment { id, replaced_at } = *moment;
                    assert_ne!(id, Uuid::nil());
                    assert!(replaced_at > 0);
                }
                Ok(())
            })
            .expect("the moments read");

        database.close().expect("the connection closes");
    }

    #[test]
    fn eleven_replacements_leave_ten_moments_and_the_oldest_with_nothing_in_it() {
        let bench = Bench::new("vault-history-cap-moments");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let id = with_replacements(connection, &codec, bench.device, "Banco", 11)?;

                assert_eq!(history_len(connection, id)?, MAX_HISTORY);
                assert_eq!(history_moments(connection, id)?.len(), MAX_HISTORY);

                let (trimmed, with_content): (i64, i64) = connection.query_row(
                    "SELECT count(*), count(password) FROM vault_password_history
                      WHERE entry_id = ?1 AND deleted = 1",
                    [id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                assert!(trimmed > 0, "nothing was trimmed");
                assert_eq!(with_content, 0, "a trimmed password kept its ciphertext");
                Ok(())
            })
            .expect("the cap still holds");

        database.close().expect("the connection closes");
    }

    #[test]
    fn one_old_password_is_handed_over_and_only_to_the_entry_it_belongs_to() {
        let bench = Bench::new("vault-history-one");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let mine = with_replacements(connection, &codec, bench.device, "Mía", 2)?;
                let theirs = an_account(
                    connection,
                    &codec,
                    bench.device,
                    at(50),
                    NOW_US,
                    an_entry("Ajena"),
                )?;

                let moments = history_moments(connection, mine)?;
                let newest = moments
                    .first()
                    .map(|moment| moment.id)
                    .expect("two moments");
                assert_eq!(
                    history_password(connection, &codec, mine, newest)?.as_str(),
                    "contraseña-1"
                );

                // The identifier is real and the entry is not. Checked in the statement, so the
                // row is never read, let alone decrypted.
                let stolen = history_password(connection, &codec, theirs.id, newest)
                    .expect_err("one entry read another's history");
                assert!(matches!(stolen, DbError::NotFound));

                let invented =
                    history_password(connection, &codec, mine, Uuid::from_bytes([9; 16]))
                        .expect_err("an invented identifier was accepted");
                assert!(matches!(invented, DbError::NotFound));
                Ok(())
            })
            .expect("only the right entry's history is readable");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_trimmed_row_is_the_same_answer_as_a_row_that_is_not_there() {
        let bench = Bench::new("vault-history-trimmed");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let id = with_replacements(connection, &codec, bench.device, "Banco", 3)?;
                let oldest = history_moments(connection, id)?
                    .last()
                    .map(|moment| moment.id)
                    .expect("three moments");

                // What the cap does to the oldest row, done here by hand so the test does not
                // need eleven replacements to reach the same state.
                connection.execute(
                    "UPDATE vault_password_history SET deleted = 1, password = NULL WHERE id = ?1",
                    [oldest.as_bytes().as_slice()],
                )?;

                let gone = history_password(connection, &codec, id, oldest)
                    .expect_err("a trimmed row handed something back");
                assert!(matches!(gone, DbError::NotFound));
                Ok(())
            })
            .expect("a trimmed row holds nothing");

        database.close().expect("the connection closes");
    }

    #[test]
    fn emptying_a_history_leaves_the_rows_without_their_ciphertext() {
        let bench = Bench::new("vault-history-clear");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let mine = with_replacements(connection, &codec, bench.device, "Mía", 3)?;
                let theirs = with_replacements(connection, &codec, bench.device, "Ajena", 2)?;

                assert_eq!(
                    clear_history(connection, at(60), NOW_US + 100, HistoryScope::Entry(mine))?,
                    3
                );
                assert!(history_moments(connection, mine)?.is_empty());
                assert_eq!(history_moments(connection, theirs)?.len(), 2);

                let left: i64 = connection.query_row(
                    "SELECT count(password) FROM vault_password_history WHERE entry_id = ?1",
                    [mine.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;
                assert_eq!(left, 0, "an emptied history kept a password");

                assert_eq!(
                    clear_history(connection, at(61), NOW_US + 101, HistoryScope::All)?,
                    2
                );
                assert!(history_moments(connection, theirs)?.is_empty());

                // And on nothing at all, which is the ordinary state of the button.
                assert_eq!(
                    clear_history(connection, at(62), NOW_US + 102, HistoryScope::All)?,
                    0
                );
                Ok(())
            })
            .expect("the history empties");

        database.close().expect("the connection closes");
    }

    /// Three folders, named in the order they were created.
    fn three_folders(
        connection: &rusqlite::Connection,
        codec: &FieldCodec<'_>,
        device: DeviceId,
    ) -> Result<Vec<Uuid>, DbError> {
        ["Bancos", "Trabajo", "Casa"]
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let step = u64::try_from(index).unwrap_or(0) + 1;
                save_folder(connection, codec, device, at(step), NOW_US, None, name)
                    .map(|folder| folder.id)
            })
            .collect()
    }

    /// How many folder rows carry a parent, which must be none of them, ever.
    fn nested(connection: &rusqlite::Connection) -> i64 {
        connection
            .query_row("SELECT count(parent_id) FROM vault_folders", [], |row| {
                row.get(0)
            })
            .expect("the parents can be counted")
    }

    #[test]
    fn folders_are_created_at_the_end_and_renamed_in_place() {
        let bench = Bench::new("vault-folders-create");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let made = three_folders(connection, &codec, bench.device)?;
                let listed = folders(connection, &codec)?;
                assert_eq!(
                    listed
                        .iter()
                        .map(|folder| folder.position)
                        .collect::<Vec<_>>(),
                    vec![0, 1, 2]
                );
                assert_eq!(
                    listed
                        .iter()
                        .map(|folder| folder.name.to_string())
                        .collect::<Vec<_>>(),
                    vec!["Bancos", "Trabajo", "Casa"]
                );

                let second = made.get(1).copied().expect("three folders");
                let before: i64 = connection.query_row(
                    "SELECT rev FROM vault_folders WHERE id = ?1",
                    [second.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;

                let renamed = save_folder(
                    connection,
                    &codec,
                    bench.device,
                    at(10),
                    NOW_US + 1,
                    Some(second),
                    "  Trabajo nuevo  ",
                )?;
                assert_eq!(renamed.id, second);
                assert_eq!(renamed.position, 1);
                assert_eq!(renamed.name.as_str(), "Trabajo nuevo");

                let after: i64 = connection.query_row(
                    "SELECT rev FROM vault_folders WHERE id = ?1",
                    [second.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;
                assert!(after > before, "a rename is invisible to a merge");
                assert_eq!(nested(connection), 0);
                Ok(())
            })
            .expect("folders are created and renamed");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_folder_with_no_name_or_too_long_a_one_is_refused_and_writes_nothing() {
        let bench = Bench::new("vault-folders-name");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let long: String = std::iter::repeat_n('a', MAX_FOLDER_NAME_CHARS + 1).collect();
                for name in ["   ", long.as_str()] {
                    let refused =
                        save_folder(connection, &codec, bench.device, at(1), NOW_US, None, name)
                            .expect_err("an unacceptable folder name was written");
                    assert!(matches!(refused, DbError::TooMany { .. }));
                }

                assert!(folders(connection, &codec)?.is_empty());

                let missing = save_folder(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    Some(Uuid::from_bytes([4; 16])),
                    "Existe",
                )
                .expect_err("a folder that is not there was renamed");
                assert!(matches!(missing, DbError::NotFound));
                Ok(())
            })
            .expect("bad names change nothing");

        database.close().expect("the connection closes");
    }

    #[test]
    fn there_is_no_way_to_put_one_folder_inside_another() {
        // The schema still has the column, because removing it would be a migration that buys
        // nothing. What is gone is every path that could write anything into it. The assertion is
        // over the rows rather than over a refusal, because there is no call left to refuse.
        let bench = Bench::new("vault-folders-flat");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let made = three_folders(connection, &codec, bench.device)?;
                let first = made.first().copied().expect("three folders");
                save_folder(
                    connection,
                    &codec,
                    bench.device,
                    at(10),
                    NOW_US,
                    Some(first),
                    "Bancos otra vez",
                )?;
                reorder_folders(connection, &codec, at(11), NOW_US, &made)?;
                delete_folder(connection, &codec, at(12), NOW_US, first)?;

                assert_eq!(
                    nested(connection),
                    0,
                    "something wrote a parent into a flat list of folders"
                );
                Ok(())
            })
            .expect("no path writes a parent");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_folder_counts_what_is_in_it_and_not_what_is_in_the_bin() {
        let bench = Bench::new("vault-folders-count");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let folder = save_folder(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    None,
                    "Bancos",
                )?;

                let mut inside = Vec::new();
                for number in 0..4_u64 {
                    let written = an_account(
                        connection,
                        &codec,
                        bench.device,
                        at(number + 2),
                        NOW_US,
                        NewEntry {
                            title: "Una cuenta",
                            username: None,
                            password: None,
                            notes: None,
                            folder_id: Some(folder.id),
                            favorite: false,
                        },
                    )?;
                    inside.push(written.id);
                }

                let binned = inside.last().copied().expect("four entries");
                set_trashed(connection, &codec, at(20), NOW_US + 1, binned, true)?;

                assert_eq!(
                    folders(connection, &codec)?.first().map(|one| one.entries),
                    Some(3),
                    "the count included what is in the bin"
                );
                Ok(())
            })
            .expect("the count is what the screen will open on");

        database.close().expect("the connection closes");
    }

    #[test]
    fn deleting_a_folder_files_its_entries_at_the_root_and_keeps_every_one_of_them() {
        let bench = Bench::new("vault-folders-delete");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let folder = save_folder(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    None,
                    "Bancos",
                )?;
                let empty = save_folder(
                    connection,
                    &codec,
                    bench.device,
                    at(2),
                    NOW_US,
                    None,
                    "Vacía",
                )?;

                let mut inside = Vec::new();
                for number in 0..5_u64 {
                    inside.push(
                        an_account(
                            connection,
                            &codec,
                            bench.device,
                            at(number + 3),
                            NOW_US,
                            NewEntry {
                                title: "Una cuenta",
                                username: Some("alguien@ejemplo"),
                                password: Some("contraseña"),
                                notes: None,
                                folder_id: Some(folder.id),
                                favorite: false,
                            },
                        )?
                        .id,
                    );
                }

                assert_eq!(
                    delete_folder(connection, &codec, at(20), NOW_US + 1, folder.id)?,
                    5
                );

                for id in inside {
                    let found = entry(connection, &codec, id)?.expect("an entry was destroyed");
                    assert_eq!(found.folder_id, None);
                    // The row still opens, which is what a revision raised without resealing
                    // would have broken.
                    assert_eq!(found.title.as_str(), "Una cuenta");
                    assert_eq!(
                        found.password.as_deref().map(String::as_str),
                        Some("contraseña")
                    );
                }

                assert_eq!(
                    delete_folder(connection, &codec, at(21), NOW_US + 2, empty.id)?,
                    0
                );
                assert!(folders(connection, &codec)?.is_empty());

                let missing = delete_folder(
                    connection,
                    &codec,
                    at(22),
                    NOW_US + 3,
                    Uuid::from_bytes([5; 16]),
                )
                .expect_err("a folder that is not there was deleted");
                assert!(matches!(missing, DbError::NotFound));
                Ok(())
            })
            .expect("deleting a folder is filing");

        database.close().expect("the connection closes");
    }

    #[test]
    fn an_order_that_is_not_the_whole_set_is_refused_and_moves_nothing() {
        let bench = Bench::new("vault-folders-reorder");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let made = three_folders(connection, &codec, bench.device)?;
                let (first, second, third) = (
                    made.first().copied().expect("three"),
                    made.get(1).copied().expect("three"),
                    made.get(2).copied().expect("three"),
                );

                let wanted = vec![third, first, second];
                reorder_folders(connection, &codec, at(10), NOW_US + 1, &wanted)?;
                assert_eq!(
                    folders(connection, &codec)?
                        .iter()
                        .map(|folder| folder.id)
                        .collect::<Vec<_>>(),
                    wanted
                );
                // And the names still open at their new revision.
                assert_eq!(
                    folders(connection, &codec)?
                        .iter()
                        .map(|folder| folder.name.to_string())
                        .collect::<Vec<_>>(),
                    vec!["Casa", "Bancos", "Trabajo"]
                );

                // Idempotent: the same list twice leaves the same arrangement.
                reorder_folders(connection, &codec, at(11), NOW_US + 2, &wanted)?;
                assert_eq!(
                    folders(connection, &codec)?
                        .iter()
                        .map(|folder| folder.id)
                        .collect::<Vec<_>>(),
                    wanted
                );

                for offered in [
                    vec![first, second],
                    vec![first, first, second],
                    vec![first, second, Uuid::from_bytes([6; 16])],
                ] {
                    let refused = reorder_folders(connection, &codec, at(12), NOW_US + 3, &offered)
                        .expect_err("a partial order was applied");
                    assert!(matches!(refused, DbError::IncompleteOrder));
                    assert_eq!(
                        folders(connection, &codec)?
                            .iter()
                            .map(|folder| folder.id)
                            .collect::<Vec<_>>(),
                        wanted,
                        "a refused order moved something anyway"
                    );
                }
                Ok(())
            })
            .expect("the order is all or nothing");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_folder_name_with_accents_and_an_emoji_comes_back_byte_for_byte() {
        let bench = Bench::new("vault-folders-unicode");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                save_folder(
                    connection,
                    &codec,
                    bench.device,
                    at(1),
                    NOW_US,
                    None,
                    "Año fiscal 📁 ñ",
                )?;

                assert_eq!(
                    folders(connection, &codec)?
                        .first()
                        .map(|folder| folder.name.to_string()),
                    Some("Año fiscal 📁 ñ".to_owned())
                );
                Ok(())
            })
            .expect("nothing is rewritten on the way through");

        database.close().expect("the connection closes");
    }

    /// Twenty-four hours in microseconds, for a bin measured in days.
    const A_DAY_US: i64 = 24 * 60 * 60 * 1_000_000;

    /// One entry with an address, a custom field and a replaced password behind it.
    fn a_furnished_entry(
        connection: &rusqlite::Connection,
        codec: &FieldCodec<'_>,
        device: DeviceId,
        title: &str,
    ) -> Result<Uuid, DbError> {
        let written = an_account(connection, codec, device, at(1), NOW_US, an_entry(title))?;
        replace_urls(
            connection,
            codec,
            device,
            at(2),
            NOW_US,
            written.id,
            &["banco.es", "banco.example"],
        )?;
        replace_fields(
            connection,
            codec,
            device,
            at(3),
            NOW_US,
            written.id,
            &[NewField {
                label: "PIN",
                value: "1234",
                secret: true,
            }],
        )?;
        replace_password(
            connection,
            codec,
            device,
            at(4),
            NOW_US + 1,
            written.id,
            Some("contraseña-dos"),
        )?;

        Ok(written.id)
    }

    /// Everything about an entry that a round trip through the bin has to leave untouched.
    ///
    /// The contents, and not the stamp. The clock reading and the revision are supposed to move:
    /// throwing something away and taking it back are two writes the other device has to see.
    /// What must come back byte for byte is what somebody typed.
    fn readable(
        connection: &rusqlite::Connection,
        codec: &FieldCodec<'_>,
        id: Uuid,
    ) -> Result<Option<Contents>, DbError> {
        let Some(found) = entry(connection, codec, id)? else {
            return Ok(None);
        };

        Ok(Some(Contents {
            entry: format!(
                "{}|{:?}|{:?}|{:?}|{:?}|{}|{:?}",
                found.title.as_str(),
                found.username.as_deref(),
                found.password.as_deref(),
                found.notes.as_deref(),
                found.folder_id,
                found.favorite,
                found.kind
            ),
            urls: urls(connection, codec, id)?
                .iter()
                .map(|url| url.value.to_string())
                .collect(),
            fields: fields(connection, codec, id)?
                .iter()
                .map(|field| format!("{}={}", field.label.as_str(), field.value.as_str()))
                .collect(),
        }))
    }

    /// What an entry holds, flattened to text so that one comparison covers all of it.
    #[derive(Debug, PartialEq, Eq)]
    struct Contents {
        entry: String,
        urls: Vec<String>,
        fields: Vec<String>,
    }

    #[test]
    fn something_thrown_away_leaves_every_list_and_comes_back_whole() {
        // The property the whole two-step deletion exists for. `delete_entry` empties every
        // encrypted column of the row, so a bin built on it could hand back a blank entry and
        // call it a restore.
        let bench = Bench::new("vault-trash-round-trip");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let id = a_furnished_entry(connection, &codec, bench.device, "Banco")?;
                let before = readable(connection, &codec, id)?;

                set_trashed(connection, &codec, at(5), NOW_US + 2, id, true)?;

                assert_eq!(entry(connection, &codec, id)?, None, "it is still on show");
                assert!(entries(connection, &codec, None, 10)?.is_empty());
                assert!(searchable(connection, &codec)?.entries.is_empty());

                let bin = trashed(connection, &codec, NOW_US + 2, 10)?;
                assert_eq!(bin.len(), 1);
                assert_eq!(
                    bin.first().map(|one| one.title.to_string()),
                    Some("Banco".to_owned())
                );
                assert_eq!(bin.first().map(|one| one.kind), Some(EntryKind::Account));

                set_trashed(connection, &codec, at(6), NOW_US + 3, id, false)?;

                assert_eq!(
                    readable(connection, &codec, id)?,
                    before,
                    "something came back from the bin changed"
                );
                Ok(())
            })
            .expect("the round trip works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn throwing_the_same_thing_away_twice_does_not_restart_its_thirty_days() {
        let bench = Bench::new("vault-trash-idempotent");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let id = a_furnished_entry(connection, &codec, bench.device, "Banco")?;

                set_trashed(connection, &codec, at(5), NOW_US, id, true)?;
                let (first, rev_after_one): (i64, i64) = connection.query_row(
                    "SELECT trashed_at, rev FROM vault_entries WHERE id = ?1",
                    [id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;

                set_trashed(connection, &codec, at(6), NOW_US + A_DAY_US, id, true)?;
                let (second, rev_after_two): (i64, i64) = connection.query_row(
                    "SELECT trashed_at, rev FROM vault_entries WHERE id = ?1",
                    [id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;

                assert_eq!(second, first, "the second click reset the countdown");
                assert!(
                    rev_after_two > rev_after_one,
                    "the second write is invisible to a merge"
                );
                Ok(())
            })
            .expect("throwing twice is throwing once");

        database.close().expect("the connection closes");
    }

    #[test]
    fn throwing_away_something_that_is_not_there_is_refused() {
        let bench = Bench::new("vault-trash-missing");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let absent = set_trashed(
                    connection,
                    &codec,
                    at(5),
                    NOW_US,
                    Uuid::from_bytes([3; 16]),
                    true,
                )
                .expect_err("something that does not exist was thrown away");
                assert!(matches!(absent, DbError::NotFound));

                let id = a_furnished_entry(connection, &codec, bench.device, "Banco")?;
                delete_entry(connection, at(5), NOW_US, id)?;

                let destroyed = set_trashed(connection, &codec, at(6), NOW_US + 1, id, true)
                    .expect_err("a skeleton was thrown away");
                assert!(matches!(destroyed, DbError::NotFound));
                Ok(())
            })
            .expect("both refusals are the same answer");

        database.close().expect("the connection closes");
    }

    #[test]
    fn emptying_the_bin_destroys_what_is_in_it_and_nothing_else() {
        let bench = Bench::new("vault-bin-all");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let mut binned = Vec::new();
                for number in 0..3 {
                    let id =
                        a_furnished_entry(connection, &codec, bench.device, &format!("En bin {number}"))?;
                    set_trashed(connection, &codec, at(5), NOW_US, id, true)?;
                    binned.push(id);
                }
                let kept = [
                    a_furnished_entry(connection, &codec, bench.device, "Fuera uno")?,
                    a_furnished_entry(connection, &codec, bench.device, "Fuera dos")?,
                ];

                assert_eq!(empty_bin(connection, at(6), NOW_US + 1, Sweep::All)?, 3);

                for id in kept {
                    assert!(entry(connection, &codec, id)?.is_some(), "an entry outside the bin was destroyed");
                    assert_eq!(urls(connection, &codec, id)?.len(), 2);
                }

                for id in binned {
                    let (sealed, deleted): (i64, i64) = connection.query_row(
                        "SELECT count(title) + count(username) + count(password) + count(notes),
                                deleted
                           FROM vault_entries WHERE id = ?1",
                        [id.as_bytes().as_slice()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )?;
                    assert_eq!(deleted, 1);
                    assert_eq!(sealed, 0, "a destroyed entry kept one of its columns");

                    let (urls_left, fields_left, history_left): (i64, i64, i64) = connection.query_row(
                        "SELECT (SELECT count(value) FROM vault_urls WHERE entry_id = ?1),
                                (SELECT count(label) + count(value) FROM vault_fields WHERE entry_id = ?1),
                                (SELECT count(password) FROM vault_password_history WHERE entry_id = ?1)",
                        [id.as_bytes().as_slice()],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )?;
                    assert_eq!(
                        (urls_left, fields_left, history_left),
                        (0, 0, 0),
                        "a destroyed entry kept its addresses, its fields or its history"
                    );
                }
                Ok(())
            })
            .expect("emptying the bin works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_sweep_takes_what_has_run_out_and_leaves_what_has_not() {
        let bench = Bench::new("vault-bin-expired");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let recent = a_furnished_entry(connection, &codec, bench.device, "Hace 29")?;
                let old = a_furnished_entry(connection, &codec, bench.device, "Hace 31")?;

                let now = NOW_US + 40 * A_DAY_US;
                set_trashed(connection, &codec, at(5), now - 29 * A_DAY_US, recent, true)?;
                set_trashed(connection, &codec, at(6), now - 31 * A_DAY_US, old, true)?;

                assert_eq!(empty_bin(connection, at(7), now, Sweep::Expired)?, 1);

                let left = trashed(connection, &codec, now, 10)?;
                assert_eq!(left.len(), 1);
                assert_eq!(left.first().map(|one| one.id), Some(recent));
                assert_eq!(
                    left.first().map(|one| one.state),
                    Some(TrashState::InBin {
                        days: 29,
                        days_left: 1
                    })
                );
                Ok(())
            })
            .expect("the sweep is selective");

        database.close().expect("the connection closes");
    }

    #[test]
    fn sweeping_an_empty_bin_destroys_nothing_and_does_not_fail() {
        let bench = Bench::new("vault-bin-empty");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                a_furnished_entry(connection, &codec, bench.device, "Banco")?;

                assert_eq!(empty_bin(connection, at(6), NOW_US, Sweep::All)?, 0);
                assert_eq!(empty_bin(connection, at(7), NOW_US, Sweep::Expired)?, 0);
                Ok(())
            })
            .expect("an empty bin is not a problem");

        database.close().expect("the connection closes");
    }

    #[test]
    fn something_thrown_away_by_a_clock_that_ran_ahead_still_comes_back() {
        let bench = Bench::new("vault-trash-future");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let id = a_furnished_entry(connection, &codec, bench.device, "Banco")?;
                let before = readable(connection, &codec, id)?;

                set_trashed(connection, &codec, at(5), NOW_US + 5 * A_DAY_US, id, true)?;
                assert_eq!(
                    trashed(connection, &codec, NOW_US, 10)?
                        .first()
                        .map(|one| one.state),
                    Some(TrashState::InBin {
                        days: 0,
                        days_left: 30
                    })
                );

                set_trashed(connection, &codec, at(6), NOW_US, id, false)?;
                assert_eq!(readable(connection, &codec, id)?, before);
                Ok(())
            })
            .expect("a moment from the future is restorable");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_hundred_entries_survive_a_round_trip_through_the_bin_unchanged() {
        // A property rather than an example, written as a loop rather than with a generator, so
        // the crate does not gain a dependency to say "for all of these".
        let bench = Bench::new("vault-trash-property");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                for number in 0_u64..100 {
                    let title = format!("Entrada ñ{number}·🔑");
                    let written = an_account(
                        connection,
                        &codec,
                        bench.device,
                        at(number * 10 + 1),
                        NOW_US,
                        NewEntry {
                            title: &title,
                            username: (number % 2 == 0).then_some("alguien@ejemplo"),
                            password: (number % 3 == 0).then_some("contraseña"),
                            notes: (number % 5 == 0).then_some("una nota\ncon dos líneas"),
                            folder_id: None,
                            favorite: number % 7 == 0,
                        },
                    )?;
                    let addresses: Vec<String> = (0..(number % 4))
                        .map(|which| format!("sitio{number}-{which}.es"))
                        .collect();
                    let borrowed: Vec<&str> = addresses.iter().map(String::as_str).collect();
                    replace_urls(
                        connection,
                        &codec,
                        bench.device,
                        at(number * 10 + 2),
                        NOW_US,
                        written.id,
                        &borrowed,
                    )?;

                    let before = readable(connection, &codec, written.id)?;
                    set_trashed(
                        connection,
                        &codec,
                        at(number * 10 + 3),
                        NOW_US + 1,
                        written.id,
                        true,
                    )?;
                    set_trashed(
                        connection,
                        &codec,
                        at(number * 10 + 4),
                        NOW_US + 2,
                        written.id,
                        false,
                    )?;

                    assert_eq!(
                        readable(connection, &codec, written.id)?,
                        before,
                        "entry {number} came back from the bin changed"
                    );
                }
                Ok(())
            })
            .expect("a hundred round trips change nothing");

        database.close().expect("the connection closes");
    }

    #[test]
    fn saving_a_whole_draft_over_an_entry_replaces_every_column_and_keeps_the_history() {
        let bench = Bench::new("vault-update-entry");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let id = a_furnished_entry(connection, &codec, bench.device, "Banco")?;
                let before = history_len(connection, id)?;

                update_entry(
                    connection,
                    &codec,
                    bench.device,
                    at(50),
                    NOW_US + 1,
                    id,
                    NewEntry {
                        title: "Caja",
                        username: None,
                        password: Some("contraseña-uno"),
                        notes: Some("otra nota"),
                        folder_id: None,
                        favorite: true,
                    },
                    EntryKind::Note,
                )?;

                // Read through the ordinary reader, which decrypts every column: a row resealed at
                // the new revision that had one column left at the old one would fail right here.
                let read = entry(connection, &codec, id)?.expect("it is still there");
                assert_eq!(read.title.as_str(), "Caja");
                assert_eq!(read.username, None);
                assert_eq!(read.notes.as_deref().map(String::as_str), Some("otra nota"));
                assert!(read.favorite);
                assert_eq!(read.kind, EntryKind::Note);

                // Untouched, both of them: the history belongs to `replace_password` and the bin
                // to `set_trashed`.
                assert_eq!(history_len(connection, id)?, before);
                assert_eq!(read.trashed_at, None);

                Ok(())
            })
            .expect("a whole draft can be saved over an entry");

        database.close().expect("the connection closes");
    }

    #[test]
    fn saving_over_something_in_the_bin_is_refused() {
        let bench = Bench::new("vault-update-binned");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let id = a_furnished_entry(connection, &codec, bench.device, "Banco")?;
                set_trashed(connection, &codec, at(60), NOW_US + 1, id, true)?;

                let refused = update_entry(
                    connection,
                    &codec,
                    bench.device,
                    at(61),
                    NOW_US + 2,
                    id,
                    an_entry("Otro nombre"),
                    EntryKind::Account,
                );

                assert!(matches!(refused, Err(DbError::NotFound)));
                Ok(())
            })
            .expect("the refusal is the answer, not a failure");

        database.close().expect("the connection closes");
    }

    #[test]
    fn taking_the_password_away_keeps_the_one_it_had() {
        let bench = Bench::new("vault-password-removed");
        let database = bench.database();
        let codec = bench.codec();

        database
            .with(|connection| {
                let id = a_furnished_entry(connection, &codec, bench.device, "Banco")?;
                let before = history_len(connection, id)?;

                replace_password(
                    connection,
                    &codec,
                    bench.device,
                    at(70),
                    NOW_US + 1,
                    id,
                    None,
                )?;

                let read = entry(connection, &codec, id)?.expect("it is still there");
                assert_eq!(
                    read.password, None,
                    "an entry with no password must hold a null, not an encrypted empty string"
                );
                assert_eq!(
                    history_len(connection, id)?,
                    before + 1,
                    "taking a password away is changing it, and the old one was not kept"
                );

                Ok(())
            })
            .expect("a password can be taken away");

        database.close().expect("the connection closes");
    }
}
