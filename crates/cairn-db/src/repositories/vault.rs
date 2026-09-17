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
//! What is not here yet: reading and writing URLs, custom fields and tags. The tables exist,
//! because adding an empty table is the cheap kind of change and adding a column to one with
//! data in it is the expensive kind, and the repository for them arrives with the screen that
//! draws them.

use cairn_domain::{Hlc, Rev, tree};
use rusqlite::{Connection, OptionalExtension as _, params};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::codec::{FieldCodec, RowKey, SealedColumns};
use crate::device::DeviceId;
use crate::error::DbError;
use crate::row::{RowStamp, sixteen};

/// The table entries live in.
pub const ENTRIES_TABLE: &str = "vault_entries";

/// The encrypted columns of an entry, in the order the schema declares them.
pub const ENTRIES_SEALED: SealedColumns =
    SealedColumns::new(&["title", "username", "password", "notes"]);

/// The table folders live in.
pub const FOLDERS_TABLE: &str = "vault_folders";

/// The encrypted columns of a folder.
pub const FOLDERS_SEALED: SealedColumns = SealedColumns::new(&["name"]);

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
                  title, username, password, notes, folder_id, favorite, last_used_at)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)",
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
            "{ENTRY_PROJECTION} WHERE id = ?1 AND deleted = 0 LIMIT 1"
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
        "{ENTRY_PROJECTION} WHERE deleted = 0 AND hlc > ?1 ORDER BY hlc LIMIT ?2"
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

/// Every live title, and whether that is all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Titles {
    /// The identifier of each live entry beside its opened title, in clock order.
    pub entries: Vec<(Uuid, Zeroizing<String>)>,
    /// `false` if the file holds more than [`MAX_TITLES`] live entries and the rest were left
    /// unread, which is a search that cannot find them and has to be said out loud rather than
    /// discovered.
    pub complete: bool,
}

/// The most titles the in-memory index will hold.
///
/// A hundred thousand. The index is the only way this module can be searched at all, so a
/// ceiling here is a ceiling on searching, and it is set far above what a person accumulates in
/// a lifetime of accounts. It exists because the alternative is an unlock whose cost and memory
/// are decided by the size of the file rather than by this program.
pub const MAX_TITLES: usize = 100_000;

/// Every live entry's identifier and title, for the index that is held in memory.
///
/// Reads and opens the title alone. The other three sealed columns of an entry are not touched,
/// so building the index never brings a password into the process.
///
/// Answers whether it read everything: `false` means the file holds more than [`MAX_TITLES`]
/// live entries and the rest were not read, which is a search that cannot find them and has to
/// be said out loud rather than discovered.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if a title does not open, and [`DbError::Sqlite`] if the
/// statement fails.
pub fn titles(connection: &Connection, codec: &FieldCodec<'_>) -> Result<Titles, DbError> {
    let mut statement = connection.prepare_cached(
        "SELECT id, rev, title FROM vault_entries WHERE deleted = 0 ORDER BY hlc LIMIT ?1",
    )?;

    // One more than the ceiling, so the answer to "was there more" comes from the same read
    // rather than from a second count that could disagree with it.
    let asked = i64::try_from(MAX_TITLES.saturating_add(1)).unwrap_or(i64::MAX);
    let rows = statement
        .query_map(params![asked], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let complete = rows.len() <= MAX_TITLES;
    let mut titles = Vec::with_capacity(rows.len().min(MAX_TITLES));
    for (id, rev, title) in rows.into_iter().take(MAX_TITLES) {
        let id = Uuid::from_bytes(sixteen(&id)?);
        let rev = Rev::from_number(u64::try_from(rev).map_err(|_negative| damaged())?);
        let row = RowKey {
            table: ENTRIES_TABLE,
            row_id: id,
            rev,
        };
        let title = open_optional(codec, row, "title", title)?.ok_or_else(damaged)?;
        titles.push((id, title));
    }

    Ok(Titles {
        entries: titles,
        complete,
    })
}

/// Replaces the password of an entry, keeping the old one in the history.
///
/// Three writes in one call, and they belong together: the entry is revised, the password it had
/// is written to the history, and the history is trimmed. A caller that did the first without the
/// second would lose a password with no way to get it back, which is the failure the history
/// exists for.
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
    password: &str,
) -> Result<(), DbError> {
    check_value("the password of an entry", Some(password))?;

    let Some(existing) = entry(connection, codec, id)? else {
        return Err(DbError::NotFound);
    };
    let Some(stored) = read_entry_stamp(connection, id)? else {
        return Err(DbError::NotFound);
    };
    let revised = stored.revised(hlc, now_us);

    // The whole row is resealed at the new revision, not just the column that changed. Sealing
    // one column would leave the other three authenticated under the old revision, and the next
    // read of them would fail with an error that says a value did not decrypt and nothing else.
    let sealed = codec.seal_row(
        RowKey {
            table: ENTRIES_TABLE,
            row_id: revised.id,
            rev: revised.rev,
        },
        ENTRIES_SEALED,
        &[
            ("title", Some(existing.title.as_bytes())),
            (
                "username",
                existing.username.as_deref().map(String::as_bytes),
            ),
            ("password", Some(password.as_bytes())),
            ("notes", existing.notes.as_deref().map(String::as_bytes)),
        ],
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

/// The old passwords of an entry, newest first.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if a row does not decode, and [`DbError::Sqlite`] if the statement
/// fails.
pub fn history(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    entry_id: Uuid,
) -> Result<Vec<Zeroizing<String>>, DbError> {
    let mut statement = connection.prepare_cached(
        "SELECT id, rev, password FROM vault_password_history
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
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;

    let mut passwords = Vec::with_capacity(rows.len());
    for (id, rev, stored) in rows {
        let Some(bytes) = stored else {
            // A trimmed row that has not been compacted away yet. It is a tombstone with its
            // ciphertext emptied, which is the state this module puts them in on purpose, and it
            // has nothing to hand back.
            continue;
        };
        let id = Uuid::from_bytes(sixteen(&id)?);
        let rev = Rev::from_number(u64::try_from(rev).map_err(|_negative| damaged())?);
        passwords.push(codec.open_text(
            RowKey {
                table: HISTORY_TABLE,
                row_id: id,
                rev,
            },
            "password",
            &bytes,
        )?);
    }

    Ok(passwords)
}

/// Writes a folder down, refusing a placement that would break the depth rule.
///
/// The depth is checked by following the parent column, which is why that column is one of the
/// few things in this module that is not sealed. The rule itself lives in the domain crate, over
/// identifiers, where it can be checked against generated shapes rather than remembered ones.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] if the placement is too deep or the folders loop,
/// [`DbError::Sealed`] if the name cannot be encrypted, and [`DbError::Sqlite`] if the statement
/// fails.
pub fn create_folder(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    name: &str,
    parent_id: Option<Uuid>,
) -> Result<Uuid, DbError> {
    check_value("the name of a folder", Some(name))?;

    let parents = |folder: u128| parent_of(connection, Uuid::from_u128(folder));
    tree::may_place_under(parent_id.map(|folder| folder.as_u128()), &parents)
        .map_err(placement_refused)?;

    let stamp = RowStamp::new(device, hlc, now_us)?;
    let sealed = codec.seal_row(
        RowKey {
            table: FOLDERS_TABLE,
            row_id: stamp.id,
            rev: stamp.rev,
        },
        FOLDERS_SEALED,
        &[("name", Some(name.as_bytes()))],
    )?;

    connection
        .prepare_cached(
            "INSERT INTO vault_folders
                 (id, created_at, updated_at, device_id, deleted, hlc, rev, name, parent_id, position)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, 0)",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            sealed.first().and_then(Option::as_ref),
            parent_id.map(|id| id.as_bytes().to_vec()),
        ])?;

    Ok(stamp.id)
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

    Ok(())
}

/// The columns every entry query reads, in the order [`read_entry`] expects them.
const ENTRY_PROJECTION: &str = "SELECT id, hlc, rev, deleted, title, username, password, notes, \
                                folder_id, favorite, last_used_at FROM vault_entries";

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
    ))
}

/// Checks a stored entry and opens its four encrypted columns.
fn decode_entry(codec: &FieldCodec<'_>, stored: StoredEntry) -> Result<Entry, DbError> {
    let (id, hlc, rev, deleted, title, username, password, notes, folder, favorite, last_used) =
        stored;

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
        deleted: deleted != 0,
        hlc: Hlc::from_bytes(sixteen(&hlc)?),
    })
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

/// Reads the parent of a folder, for the depth check.
///
/// Answers `None` both for a folder at the root and for one that is not there. The two are the
/// same for this purpose: a chain that runs out is a chain that ends.
fn parent_of(connection: &Connection, folder: Uuid) -> Option<u128> {
    connection
        .prepare_cached("SELECT parent_id FROM vault_folders WHERE id = ?1 AND deleted = 0")
        .ok()?
        .query_row([folder.as_bytes().as_slice()], |row| {
            row.get::<_, Option<Vec<u8>>>(0)
        })
        .ok()
        .flatten()
        .and_then(|bytes| sixteen(&bytes).ok())
        .map(|bytes| Uuid::from_bytes(bytes).as_u128())
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

/// Turns a refused placement into the error this crate reports.
///
/// One `match` producing both halves, and a `_` arm because [`tree::TreeError`] is
/// `#[non_exhaustive]`: a variant added there later arrives here rather than stopping the build
/// in this crate, and is reported as a placement nobody can name, which is the honest answer
/// for a rule this module does not yet know about.
fn placement_refused(problem: tree::TreeError) -> DbError {
    let (what, value) = match problem {
        tree::TreeError::TooDeep { depth } => ("the depth of a folder", depth as u64),
        tree::TreeError::Cycle => ("the depth of a folder whose parents form a loop", u64::MAX),
        _ => ("the placement of a folder", u64::MAX),
    };

    DbError::TooMany {
        what,
        value,
        max: tree::MAX_DEPTH as u64,
    }
}

/// What a row this application did not write is reported as.
fn damaged() -> DbError {
    DbError::Sealed(cairn_crypto::CryptoError::Open)
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};
    use cairn_domain::{Hlc, tree};
    use uuid::Uuid;

    use super::{
        MAX_HISTORY, NewEntry, create_entry, create_folder, delete_entry, entries, entry, history,
        history_len, replace_password,
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
                let written = create_entry(
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
                create_entry(
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
                    create_entry(connection, &codec, device, at(1), NOW_US, an_entry("Banco"))?;

                replace_password(
                    connection,
                    &codec,
                    device,
                    at(2),
                    NOW_US + 1,
                    written.id,
                    "contraseña-dos",
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

                let kept = history(connection, &codec, written.id)?;
                assert_eq!(kept.len(), 1);
                assert_eq!(kept.first().map(|old| old.as_str()), Some("contraseña-uno"));
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
                    create_entry(connection, &codec, device, at(1), NOW_US, an_entry("Banco"))?;

                for step in 2..=20_u64 {
                    replace_password(
                        connection,
                        &codec,
                        device,
                        at(step),
                        NOW_US + i64::try_from(step).unwrap_or(0),
                        written.id,
                        &format!("contraseña-{step}"),
                    )?;
                }

                assert_eq!(history_len(connection, written.id)?, MAX_HISTORY);
                assert_eq!(history(connection, &codec, written.id)?.len(), MAX_HISTORY);

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
                let written = create_entry(
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
                    "la primera",
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
                    create_entry(connection, &codec, device, at(1), NOW_US, an_entry("Banco"))?;
                replace_password(
                    connection,
                    &codec,
                    device,
                    at(2),
                    NOW_US + 1,
                    written.id,
                    "contraseña-dos",
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
    fn a_folder_deeper_than_the_rule_allows_is_refused() {
        let scratch = Scratch::new("vault-folders");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let mut parent: Option<Uuid> = None;
                for level in 1..=tree::MAX_DEPTH {
                    let step = u64::try_from(level).unwrap_or(0);
                    parent = Some(create_folder(
                        connection,
                        &codec,
                        device,
                        at(step),
                        NOW_US,
                        &format!("Nivel {level}"),
                        parent,
                    )?);
                }

                let refused = create_folder(
                    connection,
                    &codec,
                    device,
                    at(99),
                    NOW_US,
                    "Uno de más",
                    parent,
                )
                .expect_err("a folder past the limit was accepted");
                assert!(matches!(refused, DbError::TooMany { .. }));
                Ok(())
            })
            .expect("the limit holds");

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
                    create_entry(
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
}
