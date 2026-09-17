//! Where the logical clock picks up from when the application starts.
//!
//! The clock itself lives in the domain crate and knows nothing about SQLite. What it needs on
//! every start is one number: the highest reading anything in this database has already been
//! written at. Without it, a device that restarts begins again from whatever the operating
//! system's clock says, and on a machine whose clock is a little slow that means the next write
//! carries a reading that has already been used for a different row.
//!
//! Every table is asked, not just the one about to be written to. A reading has to be greater
//! than everything, not greater than everything in its own table, or two rows in different tables
//! end up ordered against each other by nothing.
//!
//! Every name comes from the list this crate declares. Reading the table names back out of the
//! file and putting them into a statement would make the shape of what this process runs depend
//! on the contents of a file, and that is refused even when the file is one we encrypted.

use cairn_domain::{Clock, Hlc};
use rusqlite::{Connection, OptionalExtension as _};

use crate::device::DeviceId;
use crate::error::DbError;
use crate::migrations::DATA_TABLES;

/// The clock this device should carry on from, given what is already in the database.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if a statement fails for a reason other than the table not being
/// there yet.
pub fn resume(connection: &Connection, device: DeviceId) -> Result<Clock, DbError> {
    let tie_break = device.tie_break();

    Ok(match highest(connection)? {
        Some(reading) => Clock::resuming(reading, tie_break),
        None => Clock::starting(tie_break),
    })
}

/// The highest clock reading in the database, across every table of the current schema.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if a statement fails for a reason other than the table being
/// absent.
pub fn highest(connection: &Connection) -> Result<Option<Hlc>, DbError> {
    let mut highest: Option<Hlc> = None;

    for table in DATA_TABLES {
        // The index on `hlc` makes this one row, not a scan, which is what keeps it affordable on
        // every unlock rather than only on a small database.
        let statement = format!("SELECT hlc FROM \"{table}\" ORDER BY hlc DESC LIMIT 1");
        let Ok(mut prepared) = connection.prepare_cached(&statement) else {
            // The table is not there, which is the ordinary state of a file between being
            // created and being migrated. Not an error, and not a reason to stop asking about
            // the others.
            continue;
        };

        let found: Option<Vec<u8>> = prepared.query_row([], |row| row.get(0)).optional()?;
        let Some(bytes) = found else {
            continue;
        };
        let Ok(bytes) = <[u8; 16]>::try_from(bytes.as_slice()) else {
            // A reading that is not sixteen bytes was written by something that is not this
            // program. Skipped rather than refused: the clock only needs a lower bound, and a
            // damaged row is something the read of that row will report with its own message.
            continue;
        };

        let reading = Hlc::from_bytes(bytes);
        if highest.is_none_or(|current| reading > current) {
            highest = Some(reading);
        }
    }

    Ok(highest)
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};
    use cairn_domain::{CivilDay, Hlc};

    use super::{highest, resume};
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
    fn an_empty_database_has_no_reading_and_the_clock_starts_from_nothing() {
        let scratch = Scratch::new("clock-empty");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                assert_eq!(highest(connection)?, None);
                assert_eq!(resume(connection, device)?.last().wall_ms(), 0);
                Ok(())
            })
            .expect("the clock resumes");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_clock_that_resumes_never_gives_out_a_reading_already_in_the_file() {
        // The property the whole module exists for, written as the failure it prevents: a
        // restart on a machine whose clock is a little slow must not produce a reading that a
        // row already carries.
        let scratch = Scratch::new("clock-resume");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();
        let written = Hlc::new(9_000, 4, [7; 6]);

        database
            .with(|connection| {
                habits::create(
                    connection,
                    &codec,
                    device,
                    written,
                    NOW_US,
                    NewHabit {
                        name: "Andar",
                        notes: None,
                        started_on: CivilDay::new(2026, 9, 17).expect("a day that exists"),
                        position: 0,
                    },
                )?;

                assert_eq!(highest(connection)?, Some(written));

                let mut clock = resume(connection, device)?;
                let next = clock.tick(1_000);

                assert!(
                    next > written,
                    "the clock gave out a reading already in use"
                );
                assert_eq!(
                    next.device(),
                    device.tie_break(),
                    "it signed as another device"
                );
                Ok(())
            })
            .expect("the clock resumes past what is there");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_highest_reading_is_the_highest_across_tables_and_not_within_one() {
        let scratch = Scratch::new("clock-across-tables");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                crate::repositories::settings::put(
                    connection,
                    &codec,
                    device,
                    Hlc::new(20_000, 0, [1; 6]),
                    NOW_US,
                    "theme",
                    Some(b"ink"),
                )?;
                habits::create(
                    connection,
                    &codec,
                    device,
                    Hlc::new(9_000, 0, [1; 6]),
                    NOW_US,
                    NewHabit {
                        name: "Andar",
                        notes: None,
                        started_on: CivilDay::new(2026, 9, 17).expect("a day that exists"),
                        position: 0,
                    },
                )?;

                assert_eq!(highest(connection)?, Some(Hlc::new(20_000, 0, [1; 6])));
                Ok(())
            })
            .expect("the highest is found");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_database_with_no_tables_yet_answers_rather_than_failing() {
        let scratch = Scratch::new("clock-unmigrated");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");

        assert_eq!(database.with(highest).expect("it answers"), None);

        database.close().expect("the connection closes");
    }
}
