//! Habits, and the marks on the calendar that say they were done.
//!
//! Two tables in one module, because they are read together and because the pair is the whole
//! point: a habit without its entries is a row nobody looks at, and an entry without its habit is
//! a number on a day.
//!
//! Only the note is sealed. The name, the schedule, the colour and the position are in the clear,
//! and that is the rule from the design applied literally rather than case by case. Without them
//! in the clear there is no ordering in SQL, no paging by keyset and no heatmap inside its budget,
//! because each of those would become a full decrypt of the table in Rust. The consequence is
//! written down rather than glossed over: the name of a habit can say as much as its note, and
//! under this rule it is protected by the file's own encryption and nothing else.
//!
//! Paging is by clock reading, never by a growing offset. `WHERE hlc > ?` walks the index that
//! already exists for the merge, and costs the same for the last page as for the first; an offset
//! makes the database count past everything it has already handed out, so a long list gets slower
//! exactly where a person is most likely to still be scrolling.

use cairn_domain::{CivilDay, Hlc, Rev};
use rusqlite::{Connection, OptionalExtension as _, params};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::codec::{FieldCodec, RowKey, SealedColumns};
use crate::device::DeviceId;
use crate::error::DbError;
use crate::row::{RowStamp, sixteen};

/// The table habits live in, as the schema names it.
pub const TABLE: &str = "habits";

/// The encrypted columns of that table.
pub const SEALED: SealedColumns = SealedColumns::new(&["notes"]);

/// The table the marks live in.
pub const ENTRIES_TABLE: &str = "habit_entries";

/// The encrypted columns of that one.
pub const ENTRIES_SEALED: SealedColumns = SealedColumns::new(&["note"]);

/// The longest a habit name may be, in characters.
///
/// Matches the constraint in the migration, and is checked here as well. A value the database
/// refuses arrives as a constraint failure with nothing useful in it; a caller that is one
/// character over deserves to be told which limit it passed.
pub const MAX_NAME_LEN: usize = 120;

/// The most rows one page may hold.
///
/// A ceiling, not a default. The number that asks for a page arrives from the other side of the
/// bridge, and a number from a WebView does not get to decide how much memory this process
/// reserves.
pub const MAX_PAGE: usize = 200;

/// One habit, as it comes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Habit {
    /// The row's identifier.
    pub id: Uuid,
    /// What the habit is called. In the clear, and deliberately so.
    pub name: String,
    /// The note, if it has one and has not been deleted.
    ///
    /// Clears itself when it is dropped, because this is decrypted content.
    pub notes: Option<Zeroizing<Vec<u8>>>,
    /// The day the habit started being tracked.
    pub started_on: CivilDay,
    /// Where it sits in the person's own order.
    pub position: i64,
    /// Whether it is a tombstone.
    pub deleted: bool,
    /// The clock reading of the last write, which is also the key the next page starts after.
    pub hlc: Hlc,
}

/// What is needed to write a habit down for the first time.
///
/// A struct rather than seven arguments, because six of them are `Option`s and integers and a
/// caller that swaps two of them compiles perfectly.
#[derive(Debug, Clone, Copy)]
pub struct NewHabit<'a> {
    /// What to call it.
    pub name: &'a str,
    /// The note, or nothing.
    pub notes: Option<&'a [u8]>,
    /// The day it starts.
    pub started_on: CivilDay,
    /// Where it sits in the person's own order.
    pub position: i64,
}

/// One mark on the calendar, as it is offered.
///
/// A struct for the same reason [`NewHabit`] is one: an identifier, a day and a quantity in a
/// row is three values a caller can put in the wrong order and still compile.
#[derive(Debug, Clone, Copy)]
pub struct Mark<'a> {
    /// Which habit the mark belongs to.
    pub habit_id: Uuid,
    /// Which square on the calendar.
    pub day: CivilDay,
    /// How much, in the smallest unit the habit counts in. One, for a habit that is simply done.
    pub amount: i64,
    /// A note about that day, or nothing.
    pub note: Option<&'a [u8]>,
}

/// Writes a habit down.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] for a name longer than [`MAX_NAME_LEN`], [`DbError::Sealed`] if
/// the note cannot be encrypted, and [`DbError::Sqlite`] if the statement fails.
pub fn create(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    habit: NewHabit<'_>,
) -> Result<Habit, DbError> {
    check_name(habit.name)?;

    let stamp = RowStamp::new(device, hlc, now_us)?;
    let sealed = codec.seal_row(
        RowKey {
            table: TABLE,
            row_id: stamp.id,
            rev: stamp.rev,
        },
        SEALED,
        &[("notes", habit.notes)],
    )?;

    connection
        .prepare_cached(
            "INSERT INTO habits
                 (id, created_at, updated_at, device_id, deleted, hlc, rev,
                  name, notes, started_on, position)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            habit.name,
            sealed.first().and_then(Option::as_ref),
            habit.started_on.as_number(),
            habit.position,
        ])?;

    Ok(Habit {
        id: stamp.id,
        name: habit.name.to_owned(),
        notes: habit.notes.map(|bytes| Zeroizing::new(bytes.to_vec())),
        started_on: habit.started_on,
        position: habit.position,
        deleted: false,
        hlc: stamp.hlc,
    })
}

/// Reads one habit, answering `None` when it is not there or is a tombstone.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if the row is not the shape the schema describes or its note does
/// not decrypt, and [`DbError::Sqlite`] if the statement fails.
pub fn get(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    id: Uuid,
) -> Result<Option<Habit>, DbError> {
    let found = connection
        .prepare_cached(
            "SELECT id, hlc, rev, deleted, name, notes, started_on, position
               FROM habits
              WHERE id = ?1 AND deleted = 0
              LIMIT 1",
        )?
        .query_row([id.as_bytes().as_slice()], read_habit)
        .optional()?;

    found.map(|stored| decode(codec, stored)).transpose()
}

/// A page of habits, in clock order, starting after a reading the caller already has.
///
/// `None` starts at the beginning. The reading to pass next time is the one on the last habit of
/// the page, which is why it travels on the value rather than being counted by the caller.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] if more than [`MAX_PAGE`] rows are asked for, [`DbError::Sealed`]
/// if a row is not the shape the schema describes or a note does not decrypt, and
/// [`DbError::Sqlite`] if the statement fails.
pub fn page(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    after: Option<Hlc>,
    limit: usize,
) -> Result<Vec<Habit>, DbError> {
    if limit == 0 || limit > MAX_PAGE {
        return Err(DbError::TooMany {
            what: "the size of a page of habits",
            value: limit as u64,
            max: MAX_PAGE as u64,
        });
    }

    // Sixteen zero bytes are lower than every reading a clock can produce, so the first page and
    // every other page run the same statement against the same index. A separate statement for
    // the first page would be a second thing to keep correct.
    let start = after.map_or([0_u8; 16], Hlc::to_bytes);

    let mut statement = connection.prepare_cached(
        "SELECT id, hlc, rev, deleted, name, notes, started_on, position
           FROM habits
          WHERE deleted = 0 AND hlc > ?1
          ORDER BY hlc
          LIMIT ?2",
    )?;

    let rows = statement
        .query_map(
            params![start.as_slice(), i64::try_from(limit).unwrap_or(i64::MAX)],
            read_habit,
        )?
        .collect::<Result<Vec<_>, _>>()?;

    rows.into_iter()
        .map(|stored| decode(codec, stored))
        .collect()
}

/// Marks a habit as deleted and empties its encrypted column.
///
/// Answers the skeleton that is left: the identifier and the name stay, so a list can show that
/// something was removed, and the note does not, because a deletion that keeps the content for a
/// hundred and eighty days is a delay rather than a deletion.
///
/// # Errors
///
/// Returns [`DbError::NotFound`] if there is no live habit with that identifier, and
/// [`DbError::Sqlite`] if the statement fails.
pub fn delete(connection: &Connection, hlc: Hlc, now_us: i64, id: Uuid) -> Result<Habit, DbError> {
    let Some(stored) = connection
        .prepare_cached(
            "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev
               FROM habits
              WHERE id = ?1 AND deleted = 0
              LIMIT 1",
        )?
        .query_row([id.as_bytes().as_slice()], RowStamp::read_common)
        .optional()?
    else {
        return Err(DbError::NotFound);
    };

    let gone = RowStamp::from_stored(stored)?.tombstoned(hlc, now_us);

    connection
        .prepare_cached(
            "UPDATE habits
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = ?4, notes = NULL
              WHERE id = ?1",
        )?
        .execute(params![
            gone.id.as_bytes().as_slice(),
            gone.updated_at,
            gone.hlc_as_stored().as_slice(),
            gone.rev_as_stored(),
        ])?;

    let (name, started_on, position) = connection
        .prepare_cached("SELECT name, started_on, position FROM habits WHERE id = ?1")?
        .query_row([gone.id.as_bytes().as_slice()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;

    Ok(Habit {
        id: gone.id,
        name,
        notes: None,
        started_on: CivilDay::from_number(started_on)
            .map_err(|_not_a_day| DbError::Sealed(cairn_crypto::CryptoError::Open))?,
        position,
        deleted: true,
        hlc: gone.hlc,
    })
}

/// How many habits are not tombstones.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if the statement fails.
pub fn count_live(connection: &Connection) -> Result<u64, DbError> {
    let counted: i64 = connection
        .prepare_cached("SELECT count(*) FROM habits WHERE deleted = 0")?
        .query_row([], |row| row.get(0))?;

    Ok(u64::try_from(counted).unwrap_or(0))
}

/// Marks a day as done, or changes the amount already recorded for it.
///
/// One live row per habit and day, which the partial unique index enforces. Marking a day that is
/// already marked revises the row it found rather than adding a second one, because two marks on
/// one square is not a state the calendar can draw.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if the note cannot be encrypted, and [`DbError::Sqlite`] if the
/// statement fails.
pub fn mark(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    entry: Mark<'_>,
) -> Result<Uuid, DbError> {
    let Mark {
        habit_id,
        day,
        amount,
        note,
    } = entry;

    let existing = connection
        .prepare_cached(
            "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev
               FROM habit_entries
              WHERE habit_id = ?1 AND day = ?2 AND deleted = 0
              LIMIT 1",
        )?
        .query_row(
            params![habit_id.as_bytes().as_slice(), day.as_number()],
            RowStamp::read_common,
        )
        .optional()?;

    let stamp = match existing {
        Some(stored) => RowStamp::from_stored(stored)?.revised(hlc, now_us),
        None => RowStamp::new(device, hlc, now_us)?,
    };

    let sealed = codec.seal_row(
        RowKey {
            table: ENTRIES_TABLE,
            row_id: stamp.id,
            rev: stamp.rev,
        },
        ENTRIES_SEALED,
        &[("note", note)],
    )?;

    connection
        .prepare_cached(
            "INSERT INTO habit_entries
                 (id, created_at, updated_at, device_id, deleted, hlc, rev,
                  habit_id, day, amount, note)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (id) DO UPDATE SET
                 updated_at = excluded.updated_at,
                 device_id  = excluded.device_id,
                 deleted    = 0,
                 hlc        = excluded.hlc,
                 rev        = excluded.rev,
                 amount     = excluded.amount,
                 note       = excluded.note",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.updated_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            habit_id.as_bytes().as_slice(),
            day.as_number(),
            amount,
            sealed.first().and_then(Option::as_ref),
        ])?;

    Ok(stamp.id)
}

/// Unmarks a day, leaving a tombstone with nothing inside it.
///
/// # Errors
///
/// Returns [`DbError::NotFound`] if that day is not marked, and [`DbError::Sqlite`] if the
/// statement fails.
pub fn unmark(
    connection: &Connection,
    hlc: Hlc,
    now_us: i64,
    habit_id: Uuid,
    day: CivilDay,
) -> Result<(), DbError> {
    let Some(stored) = connection
        .prepare_cached(
            "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev
               FROM habit_entries
              WHERE habit_id = ?1 AND day = ?2 AND deleted = 0
              LIMIT 1",
        )?
        .query_row(
            params![habit_id.as_bytes().as_slice(), day.as_number()],
            RowStamp::read_common,
        )
        .optional()?
    else {
        return Err(DbError::NotFound);
    };

    let gone = RowStamp::from_stored(stored)?.tombstoned(hlc, now_us);

    connection
        .prepare_cached(
            "UPDATE habit_entries
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = ?4, note = NULL
              WHERE id = ?1",
        )?
        .execute(params![
            gone.id.as_bytes().as_slice(),
            gone.updated_at,
            gone.hlc_as_stored().as_slice(),
            gone.rev_as_stored(),
        ])?;

    Ok(())
}

/// Whether a day is marked for a habit.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if the statement fails.
pub fn is_marked(connection: &Connection, habit_id: Uuid, day: CivilDay) -> Result<bool, DbError> {
    let found: Option<i64> = connection
        .prepare_cached(
            "SELECT 1 FROM habit_entries
              WHERE habit_id = ?1 AND day = ?2 AND deleted = 0
              LIMIT 1",
        )?
        .query_row(
            params![habit_id.as_bytes().as_slice(), day.as_number()],
            |row| row.get(0),
        )
        .optional()?;

    Ok(found.is_some())
}

/// One habit exactly as the projection hands it back.
type StoredHabit = (
    Vec<u8>,
    Vec<u8>,
    i64,
    i64,
    String,
    Option<Vec<u8>>,
    u32,
    i64,
);

/// Reads the projection every habit query uses, in its order.
fn read_habit(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredHabit> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

/// Checks a stored habit and opens its note.
fn decode(codec: &FieldCodec<'_>, stored: StoredHabit) -> Result<Habit, DbError> {
    let (id, hlc, rev, deleted, name, notes, started_on, position) = stored;

    let id = Uuid::from_bytes(sixteen(&id)?);
    let rev = Rev::from_number(u64::try_from(rev).map_err(|_negative| damaged())?);
    let notes = notes
        .map(|bytes| {
            codec.open(
                RowKey {
                    table: TABLE,
                    row_id: id,
                    rev,
                },
                "notes",
                &bytes,
            )
        })
        .transpose()?;

    Ok(Habit {
        id,
        name,
        notes,
        started_on: CivilDay::from_number(started_on).map_err(|_not_a_day| damaged())?,
        position,
        deleted: deleted != 0,
        hlc: Hlc::from_bytes(sixteen(&hlc)?),
    })
}

/// Refuses a name the schema would refuse, with a message that says which limit.
fn check_name(name: &str) -> Result<(), DbError> {
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN {
        return Err(DbError::TooMany {
            what: "the length of a habit name",
            value: name.chars().count() as u64,
            max: MAX_NAME_LEN as u64,
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
    use cairn_domain::{CivilDay, Hlc};
    use uuid::Uuid;

    use super::{
        Habit, MAX_NAME_LEN, MAX_PAGE, Mark, NewHabit, count_live, create, delete, get, is_marked,
        mark, page, unmark,
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

    /// A reading higher than every reading built from a smaller step.
    fn at(step: u64) -> Hlc {
        Hlc::new(step, 0, [1; 6])
    }

    fn a_day() -> CivilDay {
        CivilDay::new(2026, 9, 17).expect("a day that exists")
    }

    fn a_mark(habit_id: Uuid, amount: i64) -> Mark<'static> {
        Mark {
            habit_id,
            day: a_day(),
            amount,
            note: None,
        }
    }

    fn a_habit(name: &str) -> NewHabit<'_> {
        NewHabit {
            name,
            notes: Some(b"cairn-canary-note"),
            started_on: a_day(),
            position: 0,
        }
    }

    #[test]
    fn what_is_written_comes_back() {
        let scratch = Scratch::new("habits-round-trip");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let written = create(
                    connection,
                    &codec,
                    device,
                    at(1),
                    NOW_US,
                    a_habit("Leer treinta minutos"),
                )?;

                let read = get(connection, &codec, written.id)?.expect("it was just written");
                assert_eq!(read.name, "Leer treinta minutos");
                assert_eq!(
                    read.notes.as_deref().map(Vec::as_slice),
                    Some(b"cairn-canary-note".as_slice())
                );
                assert_eq!(read.started_on, a_day());
                assert!(!read.deleted);
                assert_eq!(count_live(connection)?, 1);
                Ok(())
            })
            .expect("the round trip works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_note_is_not_in_the_file_in_the_clear_and_the_name_is() {
        // Both halves of the decision, in one test, so that neither can be changed quietly.
        // The name being readable in the raw file is not an accident, it is the price of
        // ordering and paging in SQL, and it is written down here as well as in the ADR.
        let scratch = Scratch::new("habits-opaque");
        let vault = an_open_vault();
        let path = scratch.database_path();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                create(
                    connection,
                    &codec,
                    DeviceId::generate()?,
                    at(1),
                    NOW_US,
                    a_habit("Leer treinta minutos"),
                )?;
                Ok(())
            })
            .expect("the habit is written");
        database.close().expect("the connection closes");

        let raw = std::fs::read(&path).expect("the file is readable as bytes");
        for needle in [b"cairn-canary-note".as_slice(), b"Leer treinta minutos"] {
            assert!(
                !raw.windows(needle.len()).any(|window| window == needle),
                "something readable appeared in the file, which SQLCipher should have covered"
            );
        }
    }

    #[test]
    fn a_deleted_habit_keeps_its_skeleton_and_loses_its_note() {
        let scratch = Scratch::new("habits-delete");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let written = create(
                    connection,
                    &codec,
                    DeviceId::generate()?,
                    at(1),
                    NOW_US,
                    a_habit("Leer treinta minutos"),
                )?;

                let gone = delete(connection, at(2), NOW_US + 1, written.id)?;
                assert!(gone.deleted);
                assert_eq!(gone.name, "Leer treinta minutos");
                assert_eq!(gone.notes, None);

                assert_eq!(get(connection, &codec, written.id)?, None);
                assert_eq!(count_live(connection)?, 0);

                let (deleted, rev, notes): (i64, i64, Option<Vec<u8>>) = connection.query_row(
                    "SELECT deleted, rev, notes FROM habits WHERE id = ?1",
                    [written.id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
                assert_eq!(deleted, 1, "the row was removed rather than marked");
                assert_eq!(rev, 1, "the revision did not move");
                assert_eq!(notes, None, "the tombstone kept its ciphertext");
                Ok(())
            })
            .expect("the deletion works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn deleting_something_that_is_not_there_says_so() {
        let scratch = Scratch::new("habits-missing");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);

        let refused = database
            .with(|connection| delete(connection, at(1), NOW_US, Uuid::nil()))
            .expect_err("deleting nothing was reported as success");
        assert!(matches!(refused, DbError::NotFound));

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_name_outside_the_limits_is_refused_with_the_limit_in_the_message() {
        let scratch = Scratch::new("habits-name-limit");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();
        let too_long = "h".repeat(MAX_NAME_LEN + 1);

        database
            .with(|connection| {
                for name in ["", too_long.as_str()] {
                    let refused = create(connection, &codec, device, at(1), NOW_US, a_habit(name))
                        .expect_err("a name outside the limits was accepted");
                    assert!(matches!(refused, DbError::TooMany { .. }));
                }
                Ok(())
            })
            .expect("both are refused");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_day_that_was_unmarked_can_be_marked_again() {
        // The partial unique index, and the most common thing anybody does with a habit
        // tracker. A plain UNIQUE(habit_id, day) would leave the tombstone holding the pair and
        // this would fail with a constraint error nobody could act on.
        let scratch = Scratch::new("habits-remark");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let habit = create(connection, &codec, device, at(1), NOW_US, a_habit("Andar"))?;

                mark(
                    connection,
                    &codec,
                    device,
                    at(2),
                    NOW_US,
                    a_mark(habit.id, 1),
                )?;
                assert!(is_marked(connection, habit.id, a_day())?);

                unmark(connection, at(3), NOW_US + 1, habit.id, a_day())?;
                assert!(!is_marked(connection, habit.id, a_day())?);

                mark(
                    connection,
                    &codec,
                    device,
                    at(4),
                    NOW_US + 2,
                    a_mark(habit.id, 1),
                )?;
                assert!(is_marked(connection, habit.id, a_day())?);

                let rows: i64 = connection.query_row(
                    "SELECT count(*) FROM habit_entries WHERE habit_id = ?1",
                    [habit.id.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;
                assert_eq!(rows, 2, "marking again did not make a new row");
                Ok(())
            })
            .expect("a day can be marked again");

        database.close().expect("the connection closes");
    }

    #[test]
    fn marking_a_day_twice_revises_the_mark_instead_of_adding_one() {
        let scratch = Scratch::new("habits-remark-twice");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let habit = create(connection, &codec, device, at(1), NOW_US, a_habit("Beber"))?;

                let first = mark(
                    connection,
                    &codec,
                    device,
                    at(2),
                    NOW_US,
                    a_mark(habit.id, 1),
                )?;
                let second = mark(
                    connection,
                    &codec,
                    device,
                    at(3),
                    NOW_US + 1,
                    a_mark(habit.id, 8),
                )?;

                assert_eq!(first, second, "a second mark made a second row");

                let (rows, amount, rev): (i64, i64, i64) = connection.query_row(
                    "SELECT count(*), max(amount), max(rev) FROM habit_entries WHERE habit_id = ?1",
                    [habit.id.as_bytes().as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
                assert_eq!(rows, 1);
                assert_eq!(amount, 8, "the amount was not replaced");
                assert_eq!(rev, 1, "the revision did not move");
                Ok(())
            })
            .expect("the second mark revises the first");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_page_walks_the_whole_list_once_and_stops() {
        let scratch = Scratch::new("habits-page");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                for step in 1..=7_u64 {
                    create(
                        connection,
                        &codec,
                        device,
                        at(step),
                        NOW_US,
                        a_habit(&format!("Hábito {step}")),
                    )?;
                }

                let mut seen: Vec<String> = Vec::new();
                let mut cursor: Option<Hlc> = None;
                loop {
                    let rows = page(connection, &codec, cursor, 3)?;
                    if rows.is_empty() {
                        break;
                    }
                    cursor = rows.last().map(|habit: &Habit| habit.hlc);
                    seen.extend(rows.into_iter().map(|habit| habit.name));
                }

                assert_eq!(
                    seen.len(),
                    7,
                    "the walk did not see every habit exactly once"
                );
                assert_eq!(
                    seen,
                    (1..=7)
                        .map(|step| format!("Hábito {step}"))
                        .collect::<Vec<_>>(),
                    "the page did not come back in clock order"
                );
                Ok(())
            })
            .expect("the walk finishes");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_page_larger_than_the_ceiling_is_refused_rather_than_truncated() {
        let scratch = Scratch::new("habits-page-limit");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                for limit in [0, MAX_PAGE + 1, usize::MAX] {
                    let refused = page(connection, &codec, None, limit)
                        .expect_err("a page outside the limits was accepted");
                    assert!(matches!(refused, DbError::TooMany { .. }));
                }
                Ok(())
            })
            .expect("both are refused");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_deleted_habit_does_not_appear_in_a_page() {
        let scratch = Scratch::new("habits-page-deleted");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let first = create(connection, &codec, device, at(1), NOW_US, a_habit("Uno"))?;
                create(connection, &codec, device, at(2), NOW_US, a_habit("Dos"))?;
                delete(connection, at(3), NOW_US + 1, first.id)?;

                let rows = page(connection, &codec, None, 10)?;
                assert_eq!(rows.len(), 1);
                assert_eq!(rows.first().map(|habit| habit.name.as_str()), Some("Dos"));
                Ok(())
            })
            .expect("the page skips the tombstone");

        database.close().expect("the connection closes");
    }
}
