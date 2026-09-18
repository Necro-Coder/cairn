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

use std::collections::HashSet;

use cairn_domain::{CivilDay, Clock, Hlc, Rev};
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

/// The longest an icon may be, in characters.
///
/// The same reasoning as [`MAX_NAME_LEN`], and the same number as the migration.
pub const MAX_ICON_LEN: usize = 64;

/// The longest a colour may be, in characters.
pub const MAX_COLOR_LEN: usize = 32;

/// The longest a unit may be, in characters.
pub const MAX_UNIT_LEN: usize = 32;

/// The most rows one page may hold.
///
/// A ceiling, not a default. The number that asks for a page arrives from the other side of the
/// bridge, and a number from a WebView does not get to decide how much memory this process
/// reserves.
pub const MAX_PAGE: usize = 200;

/// A statement that reads habits, built from the one list of columns there is.
///
/// Every query that produces a [`Habit`] goes through here, and the caller supplies only what
/// comes after the table. The list is written once because two lists that have to agree end up
/// not agreeing, and the way that failure shows up is a column read by position landing in the
/// wrong field: a colour in a unit, a target in a mask. Assembled by the compiler from literals,
/// so nothing is concatenated at runtime and nothing a caller supplies reaches the statement.
macro_rules! select_habits {
    ($tail:literal) => {
        concat!(
            "SELECT id, hlc, rev, deleted, name, notes, icon, color, period, kind, direction, \
             aggregation, schedule_mask, unit, target_per_period, started_on, archived_at, \
             position FROM habits ",
            $tail
        )
    };
}

/// The seven common columns of one live habit, which every write to one reads first.
///
/// Written once for the same reason [`select_habits`] is: four copies of a column list are four
/// chances for one of them to drift, and a stamp read in the wrong order is a revision that
/// lands in `created_at`. Sharing the text also shares the prepared statement, because the
/// cache is keyed on it.
const HABIT_STAMP: &str = "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev
       FROM habits
      WHERE id = ?1 AND deleted = 0
      LIMIT 1";

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
    /// What it is drawn with, if anything was chosen.
    pub icon: Option<String>,
    /// What colour it is drawn in, if anything was chosen.
    pub color: Option<String>,
    /// How often it is judged: zero daily, one weekly.
    ///
    /// The columns from here down are the ones the domain turns into a `HabitSpec`, untouched
    /// and unvalidated. Raw on purpose. This crate reads rows; deciding whether a row is a habit
    /// this product has is the domain's job, and doing it in both places is two definitions that
    /// drift.
    pub period: i64,
    /// What is counted: zero done-or-not, one a quantity.
    pub kind: i64,
    /// Which way the target is read: zero more is better, one less is better.
    pub direction: i64,
    /// How the days of a period combine: zero the sum, one the highest, two the last.
    pub aggregation: i64,
    /// Seven bits, one per day of the week, Monday lowest.
    pub schedule_mask: i64,
    /// What the amounts are counted in, for a habit that counts a quantity.
    pub unit: Option<String>,
    /// The quantity a period aims at, if it aims at one.
    pub target_per_period: Option<i64>,
    /// The day the habit started being tracked.
    pub started_on: CivilDay,
    /// When it was archived, in microseconds since the epoch. `None` while it is active.
    pub archived_at: Option<i64>,
    /// Where it sits in the person's own order.
    pub position: i64,
    /// Whether it is a tombstone.
    pub deleted: bool,
    /// The clock reading of the last write, which is also the key the next page starts after.
    pub hlc: Hlc,
}

/// What is needed to write a habit down for the first time.
///
/// A struct rather than thirteen arguments, because nine of them are `Option`s and integers and
/// a caller that swaps two of them compiles perfectly.
#[derive(Debug, Clone, Copy)]
pub struct NewHabit<'a> {
    /// What to call it.
    pub name: &'a str,
    /// The note, or nothing.
    pub notes: Option<&'a [u8]>,
    /// What to draw it with, or nothing.
    pub icon: Option<&'a str>,
    /// What colour to draw it in, or nothing.
    pub color: Option<&'a str>,
    /// How often it is judged: zero daily, one weekly.
    pub period: i64,
    /// What is counted: zero done-or-not, one a quantity.
    pub kind: i64,
    /// Which way the target is read: zero more is better, one less is better.
    pub direction: i64,
    /// How the days of a period combine: zero the sum, one the highest, two the last.
    pub aggregation: i64,
    /// Seven bits, one per day of the week, Monday lowest.
    pub schedule_mask: i64,
    /// What the amounts are counted in, or nothing.
    pub unit: Option<&'a str>,
    /// The quantity a period aims at, or nothing.
    pub target_per_period: Option<i64>,
    /// The day it starts.
    pub started_on: CivilDay,
    /// Where it sits in the person's own order.
    pub position: i64,
}

impl<'a> NewHabit<'a> {
    /// A habit that is simply done or not, every day, with nothing else set.
    ///
    /// The shape almost every caller wants, and the one the seeding and the tests wanted before
    /// the other columns existed. Everything else is set on the value afterwards, by name, so
    /// that nine positional arguments never happen.
    #[must_use]
    pub const fn plain(name: &'a str, started_on: CivilDay, position: i64) -> Self {
        Self {
            name,
            notes: None,
            icon: None,
            color: None,
            period: 0,
            kind: 0,
            direction: 0,
            aggregation: 0,
            schedule_mask: 0,
            unit: None,
            target_per_period: None,
            started_on,
            position,
        }
    }
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
/// Returns [`DbError::TooMany`] for a name, an icon, a colour or a unit longer than its limit,
/// [`DbError::Sealed`] if the note cannot be encrypted, and [`DbError::Sqlite`] if the statement
/// fails.
pub fn create(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    habit: NewHabit<'_>,
) -> Result<Habit, DbError> {
    check_lengths(habit)?;

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

    // `archived_at` is absent from the list on purpose: a habit is not born archived, and the
    // column's default is the only value it may hold at this point. Naming it here would be a
    // second place that has to agree with `archive`, in the next task, about what an unarchived
    // habit looks like.
    connection
        .prepare_cached(
            "INSERT INTO habits
                 (id, created_at, updated_at, device_id, deleted, hlc, rev,
                  name, notes, icon, color, period, kind, direction, aggregation,
                  schedule_mask, unit, target_per_period, started_on, position)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18)",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            habit.name,
            sealed.first().and_then(Option::as_ref),
            habit.icon,
            habit.color,
            habit.period,
            habit.kind,
            habit.direction,
            habit.aggregation,
            habit.schedule_mask,
            habit.unit,
            habit.target_per_period,
            habit.started_on.as_number(),
            habit.position,
        ])?;

    Ok(Habit {
        id: stamp.id,
        name: habit.name.to_owned(),
        notes: habit.notes.map(|bytes| Zeroizing::new(bytes.to_vec())),
        icon: habit.icon.map(str::to_owned),
        color: habit.color.map(str::to_owned),
        period: habit.period,
        kind: habit.kind,
        direction: habit.direction,
        aggregation: habit.aggregation,
        schedule_mask: habit.schedule_mask,
        unit: habit.unit.map(str::to_owned),
        target_per_period: habit.target_per_period,
        started_on: habit.started_on,
        archived_at: None,
        position: habit.position,
        deleted: false,
        hlc: stamp.hlc,
    })
}

/// Changes a habit, raising its revision and resealing its note.
///
/// Does **not** touch `archived_at` or `position`. Archiving is `archive` and ordering is
/// `reorder`, both in the next task, and hiding either inside an edit form is how a habit
/// disappears from a screen because somebody changed its colour. `position` is on [`NewHabit`]
/// because writing one for the first time has to say where it goes; this reads every other field
/// of the value and leaves that one alone.
///
/// # Errors
///
/// [`DbError::NotFound`] if there is no live habit with that identifier, [`DbError::TooMany`] for
/// a name over [`MAX_NAME_LEN`], [`DbError::Sealed`] if the note cannot be encrypted, and
/// [`DbError::Sqlite`] if the statement fails.
pub fn update(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    hlc: Hlc,
    now_us: i64,
    id: Uuid,
    habit: NewHabit<'_>,
) -> Result<Habit, DbError> {
    check_lengths(habit)?;

    let Some(stored) = connection
        .prepare_cached(HABIT_STAMP)?
        .query_row([id.as_bytes().as_slice()], RowStamp::read_common)
        .optional()?
    else {
        return Err(DbError::NotFound);
    };

    let stamp = RowStamp::from_stored(stored)?.revised(hlc, now_us);

    // Sealed again under the revision this write produces, never reused from the one before.
    // Every ciphertext in this schema is authenticated against its row's revision, so a note
    // carried across an edit would stop opening; and reaching for the previous sealed bytes to
    // avoid the work would be one key encrypting two things under the same nonce, which is the
    // one mistake this design makes structurally impossible rather than merely discouraged.
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
            "UPDATE habits
                SET updated_at = ?2, hlc = ?3, rev = ?4,
                    name = ?5, notes = ?6, icon = ?7, color = ?8, period = ?9, kind = ?10,
                    direction = ?11, aggregation = ?12, schedule_mask = ?13, unit = ?14,
                    target_per_period = ?15, started_on = ?16
              WHERE id = ?1",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.updated_at,
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            habit.name,
            sealed.first().and_then(Option::as_ref),
            habit.icon,
            habit.color,
            habit.period,
            habit.kind,
            habit.direction,
            habit.aggregation,
            habit.schedule_mask,
            habit.unit,
            habit.target_per_period,
            habit.started_on.as_number(),
        ])?;

    // Read back rather than assembled here, so that the two fields this deliberately does not
    // write come from the row instead of from an assumption about what they still hold.
    get(connection, codec, id)?.ok_or(DbError::NotFound)
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
        .prepare_cached(select_habits!("WHERE id = ?1 AND deleted = 0 LIMIT 1"))?
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

    let mut statement = connection.prepare_cached(select_habits!(
        "WHERE deleted = 0 AND hlc > ?1 ORDER BY hlc LIMIT ?2"
    ))?;

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

/// Archives or unarchives a habit. The same call does both.
///
/// Idempotent: archiving one that is already archived leaves the timestamp it had and still
/// raises the revision, because the write happened and the merge has to see it.
///
/// Takes the codec although nothing here is about the note, and that is not an oversight. Every
/// encrypted value in this schema is authenticated against its row's revision, so a write that
/// raises the revision and leaves the old ciphertext in place produces a note that will never
/// open again. Archiving raises the revision; therefore archiving reseals. The same applies to
/// [`reorder`], and not to [`delete`], which empties the column instead of keeping it.
///
/// # Errors
///
/// [`DbError::NotFound`] if there is no live habit with that identifier, [`DbError::Sealed`] if
/// its note does not open or cannot be sealed again, and [`DbError::Sqlite`] if the statement
/// fails.
pub fn archive(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    hlc: Hlc,
    now_us: i64,
    id: Uuid,
    archived: bool,
) -> Result<Habit, DbError> {
    let Some(stored) = connection
        .prepare_cached(HABIT_STAMP)?
        .query_row([id.as_bytes().as_slice()], RowStamp::read_common)
        .optional()?
    else {
        return Err(DbError::NotFound);
    };

    let stamp = RowStamp::from_stored(stored)?.revised(hlc, now_us);
    let current = get(connection, codec, id)?.ok_or(DbError::NotFound)?;

    // The moment it was archived is the moment it was *first* archived. Rewriting it on every
    // call would make a list ordered by it reshuffle itself every time somebody archived
    // something else, and the date a habit was put away is a fact about that habit.
    let archived_at = archived.then(|| current.archived_at.unwrap_or(now_us));

    let sealed = codec.seal_row(
        RowKey {
            table: TABLE,
            row_id: stamp.id,
            rev: stamp.rev,
        },
        SEALED,
        &[("notes", current.notes.as_deref().map(Vec::as_slice))],
    )?;

    connection
        .prepare_cached(
            "UPDATE habits
                SET updated_at = ?2, hlc = ?3, rev = ?4, notes = ?5, archived_at = ?6
              WHERE id = ?1",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.updated_at,
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            sealed.first().and_then(Option::as_ref),
            archived_at,
        ])?;

    get(connection, codec, id)?.ok_or(DbError::NotFound)
}

/// Marks a habit and every day it was ever marked on as deleted, and empties what was sealed.
///
/// Answers the skeleton the habit leaves: the identifier and the name stay, so a list can show
/// that something was removed, and the note does not, because a deletion that keeps the content
/// for a hundred and eighty days is a delay rather than a deletion.
///
/// The entries go with it, in the same transaction. A habit whose marks outlived it is a year of
/// somebody's calendar left in the file with nothing pointing at it, and it would come back the
/// first time the habit's tombstone lost a merge.
///
/// # Errors
///
/// Returns [`DbError::NotFound`] if there is no live habit with that identifier, and
/// [`DbError::Sqlite`] if any statement fails, in which case nothing at all is written.
pub fn delete(connection: &Connection, hlc: Hlc, now_us: i64, id: Uuid) -> Result<Habit, DbError> {
    let Some(stored) = connection
        .prepare_cached(HABIT_STAMP)?
        .query_row([id.as_bytes().as_slice()], RowStamp::read_common)
        .optional()?
    else {
        return Err(DbError::NotFound);
    };

    let gone = RowStamp::from_stored(stored)?.tombstoned(hlc, now_us);
    let mut clock = following(hlc);

    // Unchecked because the signature takes a shared connection, which is what `Database::with`
    // hands out and therefore what every caller has. The check it gives up is the one that
    // refuses a nested transaction, and this call opens exactly one and returns.
    let transaction = connection.unchecked_transaction()?;

    transaction
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

    // Read in full before any of them is written, rather than stepped through while the same
    // statement writes: a cursor over rows a sibling statement is changing is a shape SQLite
    // does not promise anything about.
    let entries = transaction
        .prepare_cached(
            "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev
               FROM habit_entries
              WHERE habit_id = ?1 AND deleted = 0",
        )?
        .query_map([id.as_bytes().as_slice()], RowStamp::read_common)?
        .collect::<Result<Vec<_>, _>>()?;

    for stored in entries {
        let gone = RowStamp::from_stored(stored)?.tombstoned(clock.tick(hlc.wall_ms()), now_us);

        transaction
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
    }

    transaction.commit()?;

    // The same projection as every other read, because the skeleton that comes back is a
    // `Habit` like any other and a shorter list here would be a second thing to keep in step.
    // Its note comes back as `None` because the statement above emptied the column, not because
    // this hides it.
    let stored = connection
        .prepare_cached(select_habits!("WHERE id = ?1 LIMIT 1"))?
        .query_row([gone.id.as_bytes().as_slice()], read_habit)?;

    // Assembled with no note rather than decoded with one, and it needs no key to do it: the
    // statement above emptied the column a moment ago, inside the same call.
    assemble(stored, None)
}

/// Sets the order of every live, unarchived habit in one transaction.
///
/// The list has to be the whole set, exactly: same length, same identifiers, no repeats. A
/// partial list cannot tell a habit that moved from a habit that was dropped by a bug on the
/// other side of the bridge, and the difference between those two is a habit that quietly ends
/// up at position zero on every device that merges the result.
///
/// Takes the codec for the reason given on [`archive`]: the position is not sealed, but writing
/// it raises the revision, and a revision raised without resealing is a note that stops opening.
///
/// # Errors
///
/// [`DbError::IncompleteOrder`] when the list is not exactly that set, [`DbError::Sealed`] if a
/// note does not open or cannot be sealed again, and [`DbError::Sqlite`] if the transaction
/// fails. Nothing is written when it is refused.
pub fn reorder(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    hlc: Hlc,
    now_us: i64,
    ids: &[Uuid],
) -> Result<(), DbError> {
    // Read and checked before the transaction opens, so that a list that was never going to be
    // accepted does not take a write lock on the file on its way to being refused.
    let live = connection
        .prepare_cached(select_habits!("WHERE deleted = 0 AND archived_at IS NULL"))?
        .query_map([], read_habit)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|stored| decode(codec, stored))
        .collect::<Result<Vec<_>, DbError>>()?;

    let offered: HashSet<Uuid> = ids.iter().copied().collect();

    // Three checks and not one, because each catches something the others let through: the
    // length catches a missing identifier, the set size catches a repeat, and the containment
    // catches one that belongs to another file or to a habit that is archived. Together they
    // mean the two sets are equal, which is the only thing that makes an index a position.
    if ids.len() != live.len() || offered.len() != ids.len() {
        return Err(DbError::IncompleteOrder);
    }
    if !live.iter().all(|habit| offered.contains(&habit.id)) {
        return Err(DbError::IncompleteOrder);
    }

    let mut clock = following(hlc);
    let transaction = connection.unchecked_transaction()?;

    for (index, id) in ids.iter().enumerate() {
        let Some(stored) = transaction
            .prepare_cached(HABIT_STAMP)?
            .query_row([id.as_bytes().as_slice()], RowStamp::read_common)
            .optional()?
        else {
            // Checked a moment ago against the same connection, which no other writer can hold
            // at the same time. Refused rather than assumed away all the same: this is the one
            // place where being wrong would write a position onto a row nobody looked at.
            return Err(DbError::IncompleteOrder);
        };

        let stamp = RowStamp::from_stored(stored)?.revised(clock.tick(hlc.wall_ms()), now_us);
        let notes = live
            .iter()
            .find(|habit| habit.id == *id)
            .and_then(|habit| habit.notes.as_deref())
            .map(Vec::as_slice);

        let sealed = codec.seal_row(
            RowKey {
                table: TABLE,
                row_id: stamp.id,
                rev: stamp.rev,
            },
            SEALED,
            &[("notes", notes)],
        )?;

        transaction
            .prepare_cached(
                "UPDATE habits
                    SET updated_at = ?2, hlc = ?3, rev = ?4, notes = ?5, position = ?6
                  WHERE id = ?1",
            )?
            .execute(params![
                stamp.id.as_bytes().as_slice(),
                stamp.updated_at,
                stamp.hlc_as_stored().as_slice(),
                stamp.rev_as_stored(),
                sealed.first().and_then(Option::as_ref),
                // The list is exactly as long as the habits in the file, so an index that does
                // not fit in a signed integer is a database with more rows than addressable
                // memory. Saturating rather than panicking, as `page` does with its limit.
                i64::try_from(index).unwrap_or(i64::MAX),
            ])?;
    }

    transaction.commit()?;

    Ok(())
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

/// One habit exactly as the projection hands it back, before anything in it is checked.
///
/// A struct rather than a tuple of eighteen, because the only thing keeping a tuple that long
/// honest is counting, and a colour and a unit are both `Option<String>`.
struct StoredHabit {
    id: Vec<u8>,
    hlc: Vec<u8>,
    rev: i64,
    deleted: i64,
    name: String,
    notes: Option<Vec<u8>>,
    icon: Option<String>,
    color: Option<String>,
    period: i64,
    kind: i64,
    direction: i64,
    aggregation: i64,
    schedule_mask: i64,
    unit: Option<String>,
    target_per_period: Option<i64>,
    started_on: u32,
    archived_at: Option<i64>,
    position: i64,
}

/// Reads the projection every habit query uses, in its order.
fn read_habit(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredHabit> {
    Ok(StoredHabit {
        id: row.get(0)?,
        hlc: row.get(1)?,
        rev: row.get(2)?,
        deleted: row.get(3)?,
        name: row.get(4)?,
        notes: row.get(5)?,
        icon: row.get(6)?,
        color: row.get(7)?,
        period: row.get(8)?,
        kind: row.get(9)?,
        direction: row.get(10)?,
        aggregation: row.get(11)?,
        schedule_mask: row.get(12)?,
        unit: row.get(13)?,
        target_per_period: row.get(14)?,
        started_on: row.get(15)?,
        archived_at: row.get(16)?,
        position: row.get(17)?,
    })
}

/// Checks a stored habit and opens its note.
fn decode(codec: &FieldCodec<'_>, stored: StoredHabit) -> Result<Habit, DbError> {
    let id = Uuid::from_bytes(sixteen(&stored.id)?);
    let rev = Rev::from_number(u64::try_from(stored.rev).map_err(|_negative| damaged())?);
    let notes = stored
        .notes
        .as_deref()
        .map(|bytes| {
            codec.open(
                RowKey {
                    table: TABLE,
                    row_id: id,
                    rev,
                },
                "notes",
                bytes,
            )
        })
        .transpose()?;

    assemble(stored, notes)
}

/// Builds the habit a stored row describes, given whatever its note turned out to be.
///
/// Separate from [`decode`] because opening the note needs a key and the rest of the row does
/// not, and the one caller that has no key is the one that has just emptied the column.
fn assemble(stored: StoredHabit, notes: Option<Zeroizing<Vec<u8>>>) -> Result<Habit, DbError> {
    Ok(Habit {
        id: Uuid::from_bytes(sixteen(&stored.id)?),
        name: stored.name,
        notes,
        icon: stored.icon,
        color: stored.color,
        period: stored.period,
        kind: stored.kind,
        direction: stored.direction,
        aggregation: stored.aggregation,
        schedule_mask: stored.schedule_mask,
        unit: stored.unit,
        target_per_period: stored.target_per_period,
        started_on: CivilDay::from_number(stored.started_on).map_err(|_not_a_day| damaged())?,
        archived_at: stored.archived_at,
        position: stored.position,
        deleted: stored.deleted != 0,
        hlc: Hlc::from_bytes(sixteen(&stored.hlc)?),
    })
}

/// Refuses the four text values the schema bounds, before the schema gets the chance to.
fn check_lengths(habit: NewHabit<'_>) -> Result<(), DbError> {
    check_name(habit.name)?;
    check_text(habit.icon, "the length of a habit icon", MAX_ICON_LEN)?;
    check_text(habit.color, "the length of a habit colour", MAX_COLOR_LEN)?;
    check_text(habit.unit, "the length of a habit unit", MAX_UNIT_LEN)
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

/// The same check for a value the schema allows to be absent.
///
/// Absent passes. Present and empty does not, because the column's check is a range starting at
/// one: a habit with an icon of no characters is a value SQLite refuses, and refusing it here is
/// the difference between a message naming the field and a constraint failure naming nothing.
fn check_text(value: Option<&str>, what: &'static str, max: usize) -> Result<(), DbError> {
    let Some(value) = value else {
        return Ok(());
    };

    let length = value.chars().count();
    if value.is_empty() || length > max {
        return Err(DbError::TooMany {
            what,
            value: length as u64,
            max: max as u64,
        });
    }

    Ok(())
}

/// A clock that carries on from one reading, for a call that writes more than one row.
///
/// Two rows written under the same reading are two rows no merge can order, so a call that
/// tombstones a habit and five of its days needs six readings and not one. It carries on from
/// the reading it was given and keeps that device's identifier, so everything it produces is
/// above what the caller already used and still signed by this machine.
///
/// The caller's own clock does not learn about these. It does not have to for the file to be
/// consistent — the clock is resumed from the highest reading in the database on every unlock,
/// which is exactly what `clock::resume` is for — but until then that clock can still hand out a
/// reading inside the same millisecond that one of these already took. The command layer closes
/// that by moving its clock past what a call like this used, and that is written here rather
/// than left to be discovered.
fn following(hlc: Hlc) -> Clock {
    Clock::resuming(hlc, hlc.device())
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

    use std::collections::HashSet;

    use rusqlite::Connection;

    use super::{
        Habit, MAX_COLOR_LEN, MAX_ICON_LEN, MAX_NAME_LEN, MAX_PAGE, MAX_UNIT_LEN, Mark, NewHabit,
        archive, count_live, create, delete, get, is_marked, mark, page, reorder, unmark, update,
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
            notes: Some(b"cairn-canary-note"),
            ..NewHabit::plain(name, a_day(), 0)
        }
    }

    /// A habit with every column set to something other than its default.
    ///
    /// The point of the value is that no two fields hold the same number, so a projection that
    /// reads two columns in the wrong order fails instead of passing by coincidence.
    fn a_full_habit(name: &str) -> NewHabit<'_> {
        NewHabit {
            name,
            notes: Some(b"cairn-canary-note"),
            icon: Some("mountain"),
            color: Some("#3b6ea5"),
            period: 1,
            kind: 1,
            direction: 1,
            aggregation: 2,
            schedule_mask: 0b001_0101,
            unit: Some("metros"),
            target_per_period: Some(5_000),
            started_on: a_day(),
            position: 7,
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

    /// The revision of a row, read straight from the column.
    fn rev_of(connection: &rusqlite::Connection, id: Uuid) -> rusqlite::Result<i64> {
        connection.query_row(
            "SELECT rev FROM habits WHERE id = ?1",
            [id.as_bytes().as_slice()],
            |row| row.get(0),
        )
    }

    #[test]
    fn every_column_a_habit_has_comes_back_the_way_it_went_in() {
        let scratch = Scratch::new("habits-every-column");
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
                    a_full_habit("Correr"),
                )?;
                let read = get(connection, &codec, written.id)?.expect("it was just written");

                assert_eq!(read, written, "reading it back is not what writing it said");
                assert_eq!(read.name, "Correr");
                assert_eq!(
                    read.notes.as_deref().map(Vec::as_slice),
                    Some(b"cairn-canary-note".as_slice())
                );
                assert_eq!(read.icon.as_deref(), Some("mountain"));
                assert_eq!(read.color.as_deref(), Some("#3b6ea5"));
                assert_eq!(read.period, 1);
                assert_eq!(read.kind, 1);
                assert_eq!(read.direction, 1);
                assert_eq!(read.aggregation, 2);
                assert_eq!(read.schedule_mask, 0b001_0101);
                assert_eq!(read.unit.as_deref(), Some("metros"));
                assert_eq!(read.target_per_period, Some(5_000));
                assert_eq!(read.started_on, a_day());
                assert_eq!(read.archived_at, None);
                assert_eq!(read.position, 7);
                Ok(())
            })
            .expect("the round trip works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn what_was_left_out_comes_back_as_nothing_rather_than_as_something_empty() {
        let scratch = Scratch::new("habits-nothing-set");
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
                    NewHabit::plain("Andar", a_day(), 0),
                )?;
                let read = get(connection, &codec, written.id)?.expect("it was just written");

                assert_eq!(read.notes, None);
                assert_eq!(read.icon, None);
                assert_eq!(read.color, None);
                assert_eq!(read.unit, None);
                assert_eq!(read.target_per_period, None);
                Ok(())
            })
            .expect("the round trip works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn an_update_changes_what_it_was_given_and_moves_the_row_forward() {
        let scratch = Scratch::new("habits-update");
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
                    a_full_habit("Correr"),
                )?;
                let before = rev_of(connection, written.id)?;

                let changed = update(
                    connection,
                    &codec,
                    at(2),
                    NOW_US + 1,
                    written.id,
                    NewHabit {
                        name: "Correr más",
                        notes: Some(b"otra nota"),
                        target_per_period: Some(7_500),
                        ..a_full_habit("Correr")
                    },
                )?;

                assert_eq!(changed.name, "Correr más");
                assert_eq!(changed.target_per_period, Some(7_500));
                assert_eq!(
                    changed.notes.as_deref().map(Vec::as_slice),
                    Some(b"otra nota".as_slice())
                );
                assert_eq!(
                    rev_of(connection, written.id)?,
                    before + 1,
                    "the revision did not move by one"
                );
                assert_ne!(changed.hlc, written.hlc, "the clock reading did not change");

                let read = get(connection, &codec, written.id)?.expect("it is still there");
                assert_eq!(read, changed, "the answer is not what the row holds");
                Ok(())
            })
            .expect("the update works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn updating_something_that_is_not_there_says_so() {
        let scratch = Scratch::new("habits-update-missing");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        let refused = database
            .with(|connection| {
                update(
                    connection,
                    &codec,
                    at(1),
                    NOW_US,
                    Uuid::nil(),
                    a_habit("Andar"),
                )
            })
            .expect_err("updating nothing was reported as success");
        assert!(matches!(refused, DbError::NotFound));

        database.close().expect("the connection closes");
    }

    #[test]
    fn an_update_leaves_archiving_and_ordering_alone() {
        // The two fields an edit form must not be able to move. A habit that vanishes from a
        // screen because somebody changed its colour is the failure this test exists to stop,
        // and it is cheaper to catch here than in the interface that would show it.
        let scratch = Scratch::new("habits-update-untouched");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();
        let archived_at = NOW_US - 1_000;

        database
            .with(|connection| {
                let written = create(
                    connection,
                    &codec,
                    device,
                    at(1),
                    NOW_US,
                    a_full_habit("Correr"),
                )?;
                assert_eq!(
                    written.position, 7,
                    "the fixture stopped setting a position"
                );

                // Archived by hand because `archive` is the next task. What is being tested is
                // that an update leaves the column where it found it, not how it got there.
                connection.execute(
                    "UPDATE habits SET archived_at = ?2 WHERE id = ?1",
                    rusqlite::params![written.id.as_bytes().as_slice(), archived_at],
                )?;

                let changed = update(
                    connection,
                    &codec,
                    at(2),
                    NOW_US + 1,
                    written.id,
                    NewHabit {
                        name: "Correr menos",
                        position: 99,
                        ..a_full_habit("Correr")
                    },
                )?;

                assert_eq!(changed.name, "Correr menos", "the edit did not happen");
                assert_eq!(
                    changed.archived_at,
                    Some(archived_at),
                    "the update unarchived a habit nobody asked it to"
                );
                assert_eq!(changed.position, 7, "the update reordered the list");
                Ok(())
            })
            .expect("the update works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_note_still_opens_after_being_updated_twice() {
        // The reseal, and the reason skipping it is not an optimisation. Every ciphertext is
        // authenticated against the revision of its row, so a note carried across an edit stops
        // opening at the next one; and reusing the sealed bytes to avoid the work would be one
        // key encrypting two things under the same nonce.
        let scratch = Scratch::new("habits-update-twice");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let written = create(connection, &codec, device, at(1), NOW_US, a_habit("Andar"))?;

                for (step, note) in [(2_u64, b"primera".as_slice()), (3, b"segunda")] {
                    let changed = update(
                        connection,
                        &codec,
                        at(step),
                        NOW_US + 1,
                        written.id,
                        NewHabit {
                            notes: Some(note),
                            ..NewHabit::plain("Andar", a_day(), 0)
                        },
                    )?;
                    assert_eq!(changed.notes.as_deref().map(Vec::as_slice), Some(note));

                    let read = get(connection, &codec, written.id)?.expect("it is still there");
                    assert_eq!(
                        read.notes.as_deref().map(Vec::as_slice),
                        Some(note),
                        "the note did not open after update number {step}"
                    );
                }
                Ok(())
            })
            .expect("both updates work");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_text_column_outside_its_limit_is_refused_and_the_message_names_it() {
        let scratch = Scratch::new("habits-text-limits");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        let long_name = "h".repeat(MAX_NAME_LEN + 1);
        let long_icon = "i".repeat(MAX_ICON_LEN + 1);
        let long_color = "c".repeat(MAX_COLOR_LEN + 1);
        let long_unit = "u".repeat(MAX_UNIT_LEN + 1);

        let cases: [(NewHabit<'_>, &str, u64); 4] = [
            (
                NewHabit::plain(&long_name, a_day(), 0),
                "name",
                MAX_NAME_LEN as u64,
            ),
            (
                NewHabit {
                    icon: Some(&long_icon),
                    ..NewHabit::plain("Andar", a_day(), 0)
                },
                "icon",
                MAX_ICON_LEN as u64,
            ),
            (
                NewHabit {
                    color: Some(&long_color),
                    ..NewHabit::plain("Andar", a_day(), 0)
                },
                "colour",
                MAX_COLOR_LEN as u64,
            ),
            (
                NewHabit {
                    unit: Some(&long_unit),
                    ..NewHabit::plain("Andar", a_day(), 0)
                },
                "unit",
                MAX_UNIT_LEN as u64,
            ),
        ];

        database
            .with(|connection| {
                let live = create(connection, &codec, device, at(1), NOW_US, a_habit("Andar"))?;

                for (offered, field, limit) in cases {
                    for refused in [
                        create(connection, &codec, device, at(2), NOW_US, offered)
                            .expect_err("a value over its limit was written"),
                        update(connection, &codec, at(2), NOW_US, live.id, offered)
                            .expect_err("a value over its limit was written"),
                    ] {
                        let DbError::TooMany { what, value, max } = refused else {
                            panic!("{field} was refused for some other reason: {refused:?}")
                        };
                        assert!(
                            what.contains(field),
                            "the message does not name the field: {what}"
                        );
                        assert_eq!(value, limit + 1);
                        assert_eq!(max, limit);
                    }
                }
                Ok(())
            })
            .expect("every one is refused");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_page_brings_the_new_columns_of_every_row_in_clock_order() {
        let scratch = Scratch::new("habits-page-columns");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                for step in 1..=3_u64 {
                    create(
                        connection,
                        &codec,
                        device,
                        at(step),
                        NOW_US,
                        NewHabit {
                            target_per_period: Some(i64::try_from(step).unwrap_or(0) * 100),
                            ..a_full_habit("Correr")
                        },
                    )?;
                }

                let rows = page(connection, &codec, None, 10)?;
                assert_eq!(rows.len(), 3);
                assert_eq!(
                    rows.iter()
                        .map(|habit| habit.target_per_period)
                        .collect::<Vec<_>>(),
                    vec![Some(100), Some(200), Some(300)],
                    "the page did not come back in clock order"
                );
                for habit in &rows {
                    assert_eq!(habit.icon.as_deref(), Some("mountain"));
                    assert_eq!(habit.color.as_deref(), Some("#3b6ea5"));
                    assert_eq!(habit.period, 1);
                    assert_eq!(habit.kind, 1);
                    assert_eq!(habit.direction, 1);
                    assert_eq!(habit.aggregation, 2);
                    assert_eq!(habit.schedule_mask, 0b001_0101);
                    assert_eq!(habit.unit.as_deref(), Some("metros"));
                    assert_eq!(habit.archived_at, None);
                    assert_eq!(habit.position, 7);
                }
                Ok(())
            })
            .expect("the page works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_first_day_and_the_last_day_the_schema_allows_survive_the_round_trip() {
        // The two ends of the column's own range check. A day stored as a number in the shape
        // YYYYMMDD has to come back as the same day at both extremes, because the conversion
        // there is where an off-by-one in the arithmetic would show first.
        let scratch = Scratch::new("habits-day-extremes");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                for (step, day) in [
                    (
                        1_u64,
                        CivilDay::new(1, 1, 1).expect("the first day there is"),
                    ),
                    (
                        2,
                        CivilDay::new(9999, 12, 31).expect("the last day there is"),
                    ),
                ] {
                    let written = create(
                        connection,
                        &codec,
                        device,
                        at(step),
                        NOW_US,
                        NewHabit::plain("Andar", day, 0),
                    )?;
                    let read = get(connection, &codec, written.id)?.expect("it was just written");
                    assert_eq!(read.started_on, day);
                }
                Ok(())
            })
            .expect("both ends are accepted");

        database.close().expect("the connection closes");
    }

    /// The position an index in a list becomes.
    ///
    /// Named rather than cast, because a cast that silently wraps is exactly what the lint
    /// refuses and a test that orders three habits has nothing to wrap.
    fn nth(index: usize) -> i64 {
        i64::try_from(index).expect("a test never orders more habits than an integer holds")
    }

    /// What a habit row holds in the three columns the state changes touch.
    ///
    /// Read straight out of the table rather than through [`get`], because two of the three are
    /// the columns a caller is told about and the point of the assertions is what is on disk.
    fn state_of(connection: &Connection, id: Uuid) -> Result<(i64, Option<i64>, i64), DbError> {
        Ok(connection.query_row(
            "SELECT rev, archived_at, position FROM habits WHERE id = ?1",
            [id.as_bytes().as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?)
    }

    /// A habit with a note and five consecutive days marked on it, each with a note of its own.
    fn a_habit_with_five_days(
        connection: &Connection,
        codec: &FieldCodec<'_>,
        device: DeviceId,
    ) -> Result<(Uuid, Vec<Uuid>), DbError> {
        let habit = create(
            connection,
            codec,
            device,
            at(1),
            NOW_US,
            a_habit("Leer treinta minutos"),
        )?;

        let mut entries = Vec::new();
        for (step, day) in (13..18).enumerate() {
            let entry = mark(
                connection,
                codec,
                device,
                at(10 + step as u64),
                NOW_US,
                Mark {
                    habit_id: habit.id,
                    day: CivilDay::new(2026, 9, day).expect("a day of September that exists"),
                    amount: 1,
                    note: Some(b"cairn-canary-entry"),
                },
            )?;
            entries.push(entry);
        }

        Ok((habit.id, entries))
    }

    /// Three habits in a known order, each carrying a note so a write that forgets to reseal
    /// one is caught rather than passing because nothing was encrypted.
    fn three_habits(
        connection: &Connection,
        codec: &FieldCodec<'_>,
        device: DeviceId,
    ) -> Result<Vec<Uuid>, DbError> {
        let mut ids = Vec::new();
        for (index, name) in ["Uno", "Dos", "Tres"].into_iter().enumerate() {
            let written = create(
                connection,
                codec,
                device,
                at(index as u64 + 1),
                NOW_US,
                NewHabit {
                    notes: Some(b"cairn-canary-note"),
                    ..NewHabit::plain(name, a_day(), nth(index))
                },
            )?;
            ids.push(written.id);
        }

        Ok(ids)
    }

    #[test]
    fn archiving_writes_the_moment_and_raises_the_revision() {
        let scratch = Scratch::new("habits-archive");
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

                let put_away = archive(connection, &codec, at(2), NOW_US + 1, written.id, true)?;
                assert_eq!(put_away.archived_at, Some(NOW_US + 1));

                let (rev, archived_at, _position) = state_of(connection, written.id)?;
                assert_eq!(rev, 1, "the revision did not move");
                assert_eq!(archived_at, Some(NOW_US + 1));

                // The whole reason this call takes the codec. The revision moved, so the note
                // had to be sealed again, and a note that was carried across would not open.
                assert_eq!(
                    put_away.notes.as_deref().map(Vec::as_slice),
                    Some(b"cairn-canary-note".as_slice()),
                    "the note did not survive the revision the archiving raised"
                );
                Ok(())
            })
            .expect("archiving works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn archiving_something_already_archived_keeps_the_moment_it_had() {
        let scratch = Scratch::new("habits-archive-twice");
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

                archive(connection, &codec, at(2), NOW_US + 1, written.id, true)?;
                let again = archive(connection, &codec, at(3), NOW_US + 2, written.id, true)?;

                assert_eq!(
                    again.archived_at,
                    Some(NOW_US + 1),
                    "the second call rewrote the date the habit was put away"
                );
                let (rev, _archived_at, _position) = state_of(connection, written.id)?;
                assert_eq!(rev, 2, "the second write did not raise the revision");
                Ok(())
            })
            .expect("archiving twice works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn unarchiving_puts_the_moment_back_to_nothing() {
        let scratch = Scratch::new("habits-unarchive");
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

                archive(connection, &codec, at(2), NOW_US + 1, written.id, true)?;
                let back = archive(connection, &codec, at(3), NOW_US + 2, written.id, false)?;

                assert_eq!(back.archived_at, None);
                assert_eq!(
                    back.notes.as_deref().map(Vec::as_slice),
                    Some(b"cairn-canary-note".as_slice())
                );
                let (rev, archived_at, _position) = state_of(connection, written.id)?;
                assert_eq!(rev, 2);
                assert_eq!(archived_at, None);
                Ok(())
            })
            .expect("unarchiving works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn archiving_something_that_is_not_there_says_so() {
        let scratch = Scratch::new("habits-archive-missing");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let refused = archive(
                    connection,
                    &codec,
                    at(1),
                    NOW_US,
                    Uuid::from_bytes([9; 16]),
                    true,
                );
                assert!(matches!(refused, Err(DbError::NotFound)));
                Ok(())
            })
            .expect("the check runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn deleting_a_habit_deletes_the_days_it_was_marked_on() {
        let scratch = Scratch::new("habits-delete-cascade");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let (habit_id, entries) =
                    a_habit_with_five_days(connection, &codec, DeviceId::generate()?)?;
                assert_eq!(entries.len(), 5);

                delete(connection, at(20), NOW_US + 1, habit_id)?;

                let still_live: i64 = connection.query_row(
                    "SELECT count(*) FROM habit_entries WHERE habit_id = ?1 AND deleted = 0",
                    [habit_id.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;
                let tombstoned: i64 = connection.query_row(
                    "SELECT count(*) FROM habit_entries WHERE habit_id = ?1 AND deleted = 1",
                    [habit_id.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;

                assert_eq!(still_live, 0, "a day outlived the habit it belonged to");
                assert_eq!(tombstoned, 5, "the days were removed rather than marked");
                assert_eq!(count_live(connection)?, 0);
                Ok(())
            })
            .expect("the cascade works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_deletion_empties_the_note_of_the_habit_and_of_every_day() {
        let scratch = Scratch::new("habits-delete-notes");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let (habit_id, _entries) =
                    a_habit_with_five_days(connection, &codec, DeviceId::generate()?)?;

                delete(connection, at(20), NOW_US + 1, habit_id)?;

                let notes: Option<Vec<u8>> = connection.query_row(
                    "SELECT notes FROM habits WHERE id = ?1",
                    [habit_id.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;
                assert_eq!(notes, None, "the habit's tombstone kept its ciphertext");

                let kept: i64 = connection.query_row(
                    "SELECT count(*) FROM habit_entries
                      WHERE habit_id = ?1 AND note IS NOT NULL",
                    [habit_id.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;
                assert_eq!(kept, 0, "a day's tombstone kept its ciphertext");
                Ok(())
            })
            .expect("the emptying works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn every_row_the_deletion_touched_gets_its_own_revision_and_its_own_reading() {
        let scratch = Scratch::new("habits-delete-stamps");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let (habit_id, _entries) =
                    a_habit_with_five_days(connection, &codec, DeviceId::generate()?)?;

                delete(connection, at(20), NOW_US + 1, habit_id)?;

                let (rev, _archived_at, _position) = state_of(connection, habit_id)?;
                assert_eq!(rev, 1, "the habit's revision did not move");

                let mut statement = connection.prepare(
                    "SELECT rev, hlc FROM habit_entries WHERE habit_id = ?1 ORDER BY day",
                )?;
                let rows = statement
                    .query_map([habit_id.as_bytes().as_slice()], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;

                assert_eq!(rows.len(), 5);
                for (rev, _hlc) in &rows {
                    assert_eq!(*rev, 1, "a day's revision did not move");
                }

                // Six rows, six readings. Two rows written under the same reading are two rows
                // the merge of phase 10 has no way to order against each other.
                let mut readings: HashSet<Vec<u8>> =
                    rows.into_iter().map(|(_rev, hlc)| hlc).collect();
                readings.insert(at(20).to_bytes().to_vec());
                assert_eq!(readings.len(), 6, "two rows share a clock reading");
                Ok(())
            })
            .expect("the stamps are right");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_deletion_that_cannot_write_the_days_writes_nothing_at_all() {
        // The test that proves the transaction. The failure is forced with a temporary trigger
        // that refuses every update of `habit_entries`: a temporary trigger is the only kind
        // SQLite lets reach into another database, it is gone when this connection is, and it
        // breaks the statement in the same way a constraint would without leaving a
        // deliberately malformed row behind for the next test to trip over.
        let scratch = Scratch::new("habits-delete-rollback");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let (habit_id, _entries) =
                    a_habit_with_five_days(connection, &codec, DeviceId::generate()?)?;

                connection.execute_batch(
                    "CREATE TEMP TRIGGER refuse_entry_updates
                     BEFORE UPDATE ON habit_entries
                     BEGIN
                         SELECT RAISE(ABORT, 'refused so the rollback can be observed');
                     END",
                )?;

                let refused = delete(connection, at(20), NOW_US + 1, habit_id);
                assert!(
                    matches!(refused, Err(DbError::Sqlite(_))),
                    "the deletion was not refused"
                );

                connection.execute_batch("DROP TRIGGER temp.refuse_entry_updates")?;

                let (rev, _archived_at, _position) = state_of(connection, habit_id)?;
                assert_eq!(rev, 0, "the habit was written although the call failed");
                assert!(
                    get(connection, &codec, habit_id)?.is_some(),
                    "the habit was tombstoned although the call failed"
                );

                let untouched: i64 = connection.query_row(
                    "SELECT count(*) FROM habit_entries
                      WHERE habit_id = ?1 AND deleted = 0 AND rev = 0 AND note IS NOT NULL",
                    [habit_id.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;
                assert_eq!(untouched, 5, "a day was written although the call failed");
                Ok(())
            })
            .expect("the rollback is observed");

        database.close().expect("the connection closes");
    }

    #[test]
    fn reordering_with_the_whole_set_writes_the_index_as_the_position() {
        let scratch = Scratch::new("habits-reorder");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let ids = three_habits(connection, &codec, DeviceId::generate()?)?;
                let wanted = vec![ids[2], ids[0], ids[1]];

                reorder(connection, &codec, at(10), NOW_US + 1, &wanted)?;

                for (index, id) in wanted.iter().enumerate() {
                    let (rev, _archived_at, position) = state_of(connection, *id)?;
                    assert_eq!(position, nth(index), "a habit is not where it was put");
                    assert_eq!(rev, 1, "the revision did not move");

                    // Resealed, like the archiving. A reordering that raised the revision and
                    // left the ciphertext alone would silently destroy every note in the list.
                    let read = get(connection, &codec, *id)?.expect("it is still there");
                    assert_eq!(
                        read.notes.as_deref().map(Vec::as_slice),
                        Some(b"cairn-canary-note".as_slice()),
                        "a note did not survive the reordering"
                    );
                }
                Ok(())
            })
            .expect("the reordering works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_reordering_missing_one_habit_is_refused_and_moves_nothing() {
        let scratch = Scratch::new("habits-reorder-short");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let ids = three_habits(connection, &codec, DeviceId::generate()?)?;

                let refused = reorder(connection, &codec, at(10), NOW_US + 1, &[ids[2], ids[0]]);
                assert!(matches!(refused, Err(DbError::IncompleteOrder)));

                for (index, id) in ids.iter().enumerate() {
                    let (rev, _archived_at, position) = state_of(connection, *id)?;
                    assert_eq!(position, nth(index), "a position moved on a refused call");
                    assert_eq!(rev, 0, "a revision moved on a refused call");
                }
                Ok(())
            })
            .expect("the check runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_reordering_that_names_one_habit_twice_is_refused() {
        let scratch = Scratch::new("habits-reorder-repeat");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let ids = three_habits(connection, &codec, DeviceId::generate()?)?;

                // The same length as the set, which is exactly why counting is not enough.
                let refused = reorder(
                    connection,
                    &codec,
                    at(10),
                    NOW_US + 1,
                    &[ids[0], ids[0], ids[1]],
                );
                assert!(matches!(refused, Err(DbError::IncompleteOrder)));

                let (_rev, _archived_at, position) = state_of(connection, ids[2])?;
                assert_eq!(position, 2, "a position moved on a refused call");
                Ok(())
            })
            .expect("the check runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_reordering_naming_something_that_is_not_in_this_file_is_refused() {
        let scratch = Scratch::new("habits-reorder-stranger");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let ids = three_habits(connection, &codec, DeviceId::generate()?)?;

                let refused = reorder(
                    connection,
                    &codec,
                    at(10),
                    NOW_US + 1,
                    &[ids[0], ids[1], Uuid::from_bytes([9; 16])],
                );
                assert!(matches!(refused, Err(DbError::IncompleteOrder)));

                for (index, id) in ids.iter().enumerate() {
                    let (_rev, _archived_at, position) = state_of(connection, *id)?;
                    assert_eq!(position, nth(index), "a position moved on a refused call");
                }
                Ok(())
            })
            .expect("the check runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn reordering_twice_with_the_same_list_leaves_the_same_positions_and_two_revisions() {
        // Idempotent in what a person sees and deliberately not in what the merge sees. The
        // second call wrote, so it has to leave a revision and a reading behind saying so.
        let scratch = Scratch::new("habits-reorder-twice");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let ids = three_habits(connection, &codec, DeviceId::generate()?)?;
                let wanted = vec![ids[1], ids[2], ids[0]];

                reorder(connection, &codec, at(10), NOW_US + 1, &wanted)?;
                reorder(connection, &codec, at(20), NOW_US + 2, &wanted)?;

                for (index, id) in wanted.iter().enumerate() {
                    let (rev, _archived_at, position) = state_of(connection, *id)?;
                    assert_eq!(position, nth(index), "the second call moved something");
                    assert_eq!(rev, 2, "the second write did not raise the revision");
                }
                Ok(())
            })
            .expect("reordering twice works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn an_archived_habit_is_not_in_the_set_a_reordering_has_to_name() {
        let scratch = Scratch::new("habits-reorder-archived");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let ids = three_habits(connection, &codec, DeviceId::generate()?)?;
                archive(connection, &codec, at(10), NOW_US + 1, ids[1], true)?;

                let refused = reorder(
                    connection,
                    &codec,
                    at(20),
                    NOW_US + 2,
                    &[ids[0], ids[1], ids[2]],
                );
                assert!(
                    matches!(refused, Err(DbError::IncompleteOrder)),
                    "an archived habit was accepted as part of the order"
                );

                // And the list without it is the whole set, so it is accepted.
                reorder(connection, &codec, at(30), NOW_US + 3, &[ids[2], ids[0]])?;
                let (_rev, _archived_at, position) = state_of(connection, ids[2])?;
                assert_eq!(position, 0);
                Ok(())
            })
            .expect("the check runs");

        database.close().expect("the connection closes");
    }
}
