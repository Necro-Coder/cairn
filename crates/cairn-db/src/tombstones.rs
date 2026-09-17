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

/// Counts the tombstones in one table, answering `None` when the table is not there.
fn count_in(connection: &Connection, table: &'static str) -> Result<Option<u64>, DbError> {
    let exists: Option<i64> = connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1")?
        .query_row([table], |row| row.get(0))
        .ok();
    if exists.is_none() {
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
                    NewHabit {
                        name: "Andar",
                        notes: None,
                        started_on: CivilDay::new(2026, 9, 17).expect("a day that exists"),
                        position: 0,
                    },
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
