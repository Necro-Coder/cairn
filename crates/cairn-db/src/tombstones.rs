//! Counting the rows that are marked as deleted, across every table that has any.
//!
//! Nothing in this application removes a row. A deletion marks it, raises its revision, writes a
//! new clock reading and empties every encrypted column it has, and the skeleton stays so that
//! two devices merging later can tell "this was deleted" apart from "this never arrived". The
//! cost of that is a file that only grows, which is why the number is on the diagnostics screen
//! rather than left for somebody to discover.
//!
//! Every statement here names its table from [`DATA_TABLES`], which is a list written in this
//! crate. Reading the names back out of the file and interpolating them would make the shape of a
//! statement depend on the contents of a file, and that has to be refused even when the file is
//! one we encrypted ourselves.

use rusqlite::Connection;

use crate::error::DbError;
use crate::migrations::DATA_TABLES;

/// How many tombstones there are, and which tables they are in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Census {
    /// The total across every table.
    pub total: u64,
    /// The count per table, in the order [`DATA_TABLES`] names them, skipping the empty ones.
    pub by_table: Vec<(&'static str, u64)>,
}

/// Counts the tombstones in every table of the current schema.
///
/// Tables the current schema does not have yet are skipped rather than reported as an error: a
/// database at an older version than this build is a database that is about to be migrated, and
/// counting is not the operation that should refuse it.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if a statement fails for a reason other than the table being
/// absent.
pub fn census(connection: &Connection) -> Result<Census, DbError> {
    let mut census = Census::default();

    for table in DATA_TABLES {
        let Some(counted) = count_in(connection, table)? else {
            continue;
        };
        if counted > 0 {
            census.by_table.push((table, counted));
            census.total = census.total.saturating_add(counted);
        }
    }

    Ok(census)
}
/// How old a tombstone has to be before it can go.
///
/// A hundred and eighty days. The number is not about disk: it is about the merge. A device that
/// has been off for six months and comes back with a row this one removed would, if the tombstone
/// were gone, look to the merge like a row that had never arrived, and the deletion would undo
/// itself. Six months is longer than any gap a person who owns both machines is likely to leave,
/// and the cost of being wrong is a row reappearing rather than a row disappearing.
pub const RETENTION_DAYS: i64 = 180;

/// How many microseconds that is.
const RETENTION_US: i64 = RETENTION_DAYS * 24 * 60 * 60 * 1_000_000;

/// What a compaction removed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Compacted {
    /// The total across every table.
    pub total: u64,
    /// The count per table, in the order [`DATA_TABLES`] names them, skipping the untouched ones.
    pub by_table: Vec<(&'static str, u64)>,
}

/// Removes the tombstones that are older than [`RETENTION_DAYS`], and leaves the rest.
///
/// This is the one place in the application that runs a `DELETE`, and it is allowed to because
/// what it deletes is a skeleton: every encrypted column of these rows was emptied when they were
/// marked, so there is nothing left in them to lose. A row marked yesterday is not touched, and a
/// row that is not marked at all is never a candidate.
///
/// Running it twice in a row removes nothing the second time, which is the property that makes it
/// safe to offer as a button.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if a statement fails for a reason other than the table being
/// absent.
pub fn compact(connection: &Connection, now_us: i64) -> Result<Compacted, DbError> {
    // Saturating, so a clock that has been set to the beginning of time does not wrap into a
    // cutoff in the far future and take every tombstone in the file with it.
    let cutoff = now_us.saturating_sub(RETENTION_US);

    let mut compacted = Compacted::default();
    for table in DATA_TABLES {
        if !table_exists(connection, table)? {
            continue;
        }

        // The only interpolation in this crate, and the name comes from a constant slice in this
        // crate rather than from anything that arrived from outside the process.
        let statement = format!("DELETE FROM \"{table}\" WHERE deleted = 1 AND updated_at < ?1");
        let removed = connection.prepare_cached(&statement)?.execute([cutoff])?;
        let removed = u64::try_from(removed).unwrap_or(0);

        if removed > 0 {
            compacted.by_table.push((table, removed));
            compacted.total = compacted.total.saturating_add(removed);
        }
    }

    Ok(compacted)
}

/// Whether a table of the current schema is in this file yet.
///
/// A database at an older version than this build is a database that is about to be migrated,
/// and neither counting nor compacting is the operation that should refuse it.
fn table_exists(connection: &Connection, table: &'static str) -> Result<bool, DbError> {
    let found: Option<i64> = connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1")?
        .query_row([table], |row| row.get(0))
        .ok();

    Ok(found.is_some())
}

/// Counts the tombstones in one table, answering `None` when the table is not there.
fn count_in(connection: &Connection, table: &'static str) -> Result<Option<u64>, DbError> {
    if !table_exists(connection, table)? {
        return Ok(None);
    }

    // The only interpolation in this crate, and the name comes from a constant slice in this
    // file's own dependency rather than from anything that arrived from outside the process.
    let statement = format!("SELECT count(*) FROM \"{table}\" WHERE deleted = 1");
    let counted: i64 = connection
        .prepare_cached(&statement)?
        .query_row([], |row| row.get(0))?;

    Ok(Some(u64::try_from(counted).unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};
    use cairn_domain::{CivilDay, Hlc};

    use super::census;
    use crate::codec::FieldCodec;
    use crate::device::DeviceId;
    use crate::migrations;
    use crate::open::Database;
    use crate::repositories::habits::{self, NewHabit};
    use crate::test_support::Scratch;

    const NOW_US: i64 = 1_700_000_000_000_000;

    fn an_open_vault() -> UnlockedVault {
        let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");
        vault
    }

    #[test]
    fn an_empty_database_has_no_tombstones_and_a_deletion_makes_one() {
        let scratch = Scratch::new("tombstones");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let counted = census(connection)?;
                assert_eq!(counted.total, 0);
                assert!(counted.by_table.is_empty());

                let habit = habits::create(
                    connection,
                    &codec,
                    DeviceId::generate()?,
                    Hlc::new(1, 0, [1; 6]),
                    NOW_US,
                    NewHabit::plain(
                        "Andar",
                        CivilDay::new(2026, 9, 17).expect("a day that exists"),
                        0,
                    ),
                )?;
                habits::delete(connection, Hlc::new(2, 0, [1; 6]), NOW_US + 1, habit.id)?;

                let counted = census(connection)?;
                assert_eq!(counted.total, 1);
                assert_eq!(counted.by_table, vec![("habits", 1)]);
                Ok(())
            })
            .expect("the census runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_database_with_no_tables_yet_counts_nothing_rather_than_failing() {
        // The state a file is in for the moment between being created and being migrated. A
        // count that refused here would turn the ordinary first run into an error screen.
        let scratch = Scratch::new("tombstones-empty");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");

        let counted = database.with(census).expect("the census runs");
        assert_eq!(counted.total, 0);

        database.close().expect("the connection closes");
    }
}

#[cfg(test)]
mod compaction_tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};
    use cairn_domain::{CivilDay, Hlc};

    use super::{RETENTION_DAYS, census, compact};
    use crate::codec::FieldCodec;
    use crate::device::DeviceId;
    use crate::migrations;
    use crate::open::Database;
    use crate::repositories::habits::{self, NewHabit};
    use crate::test_support::Scratch;

    const NOW_US: i64 = 1_700_000_000_000_000;

    /// A day, in microseconds.
    const DAY_US: i64 = 24 * 60 * 60 * 1_000_000;

    fn an_open_vault() -> UnlockedVault {
        let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");
        vault
    }

    /// Writes a habit and marks it deleted, both stamped at `moment`.
    fn a_tombstone(
        connection: &rusqlite::Connection,
        codec: &FieldCodec<'_>,
        step: u64,
        moment: i64,
    ) -> Result<(), crate::error::DbError> {
        let habit = habits::create(
            connection,
            codec,
            DeviceId::from_bytes([7; 16]),
            Hlc::new(step, 0, [1; 6]),
            moment,
            NewHabit::plain(
                &format!("Andar {step}"),
                CivilDay::new(2026, 9, 17).expect("a day that exists"),
                0,
            ),
        )?;
        habits::delete(connection, Hlc::new(step + 1, 0, [1; 6]), moment, habit.id)?;

        Ok(())
    }

    /// The whole reason the retention window exists, and the test that keeps it honest.
    #[test]
    fn a_tombstone_younger_than_the_window_is_left_alone() {
        let scratch = Scratch::new("compact-young");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                // One marked yesterday, one marked a day past the window.
                a_tombstone(connection, &codec, 1, NOW_US - DAY_US)?;
                a_tombstone(
                    connection,
                    &codec,
                    3,
                    NOW_US - (RETENTION_DAYS + 1) * DAY_US,
                )?;
                assert_eq!(census(connection)?.total, 2);

                let removed = compact(connection, NOW_US)?;
                assert_eq!(removed.total, 1, "the wrong number of tombstones went");
                assert_eq!(removed.by_table, vec![("habits", 1)]);

                // The young one is still there, which is the point: a device that has been away
                // for a month and comes back with a row this one removed has to be able to see
                // that it was removed rather than that it never arrived.
                assert_eq!(census(connection)?.total, 1);
                Ok(())
            })
            .expect("the compaction runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_tombstone_exactly_at_the_window_is_kept_rather_than_removed() {
        // The boundary, written down rather than left to whichever comparison somebody typed.
        // Strictly older goes; exactly as old as the window stays.
        let scratch = Scratch::new("compact-boundary");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                a_tombstone(connection, &codec, 1, NOW_US - RETENTION_DAYS * DAY_US)?;

                assert_eq!(compact(connection, NOW_US)?.total, 0);
                assert_eq!(census(connection)?.total, 1);
                Ok(())
            })
            .expect("the compaction runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn compacting_twice_removes_nothing_the_second_time() {
        // The property that makes it safe to offer as a button.
        let scratch = Scratch::new("compact-twice");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                a_tombstone(
                    connection,
                    &codec,
                    1,
                    NOW_US - (RETENTION_DAYS + 1) * DAY_US,
                )?;

                assert_eq!(compact(connection, NOW_US)?.total, 1);
                assert_eq!(compact(connection, NOW_US)?.total, 0);
                Ok(())
            })
            .expect("the compaction runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_row_that_is_not_a_tombstone_is_never_a_candidate() {
        let scratch = Scratch::new("compact-live");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                // Written long before the window and never deleted. Age alone must not be enough.
                habits::create(
                    connection,
                    &codec,
                    DeviceId::from_bytes([7; 16]),
                    Hlc::new(1, 0, [1; 6]),
                    NOW_US - (RETENTION_DAYS + 100) * DAY_US,
                    NewHabit::plain(
                        "Andar",
                        CivilDay::new(2026, 9, 17).expect("a day that exists"),
                        0,
                    ),
                )?;

                assert_eq!(compact(connection, NOW_US)?.total, 0);
                assert_eq!(habits::count_live(connection)?, 1);
                Ok(())
            })
            .expect("the compaction runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_clock_set_to_the_beginning_of_time_does_not_take_the_whole_file_with_it() {
        // Saturating rather than wrapping. A cutoff that wrapped into the far future would make
        // every tombstone in the file a candidate, which is the one failure here that cannot be
        // undone.
        let scratch = Scratch::new("compact-clock");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                a_tombstone(connection, &codec, 1, NOW_US)?;

                assert_eq!(compact(connection, i64::MIN)?.total, 0);
                assert_eq!(census(connection)?.total, 1);
                Ok(())
            })
            .expect("the compaction runs");

        database.close().expect("the connection closes");
    }
}
