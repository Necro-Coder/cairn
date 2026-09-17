//! When the last backup was made, and where it was put.
//!
//! Two facts, both kept in `settings` and both sealed, and the reason they are sealed is worth
//! the sentence. The moment of the last export is not especially private on its own; the folder
//! it went to almost always carries the name of whoever is logged in, and often the name of the
//! machine. It is the one setting in this phase that is personal data, so it is encrypted like
//! any note, it never leaves the core, and it is never written to a log or to a diagnostic
//! dump. The date is sealed beside it because a pair of settings where one is readable and one
//! is not is a pair somebody will eventually get the wrong way round.
//!
//! The reminder is the other half. Fourteen days after the last backup — or from the start, if
//! there has never been one — the main screen says so, quietly and once. It is not a nag and it
//! is not a modal: the person decides when to make a copy, and the only thing this can usefully
//! do is make sure they are deciding rather than forgetting.
//!
//! A clock that has gone backwards is treated as no time having passed. Somebody whose machine
//! thinks it is last year should not be told their backup is from the future, and should not be
//! reminded either; the next honest tick of the clock will do it.

use std::path::{Path, PathBuf};

use cairn_domain::Hlc;
use rusqlite::Connection;

use crate::codec::FieldCodec;
use crate::device::DeviceId;
use crate::error::DbError;
use crate::repositories::settings;

/// The setting that holds the moment of the last export, in microseconds.
pub const LAST_EXPORT_AT: &str = "backup.last_export_at";

/// The setting that holds the folder the last export went to.
pub const LAST_DIRECTORY: &str = "backup.last_directory";

/// How many days without a backup before the reminder appears.
pub const REMIND_AFTER_DAYS: u32 = 14;

/// Microseconds in a day.
const DAY_US: i64 = 24 * 60 * 60 * 1_000_000;

/// What the main screen needs to know about backups.
///
/// Days and a yes or no, and nothing else. In particular not the folder: the interface has no
/// use for a path, and a path is the one piece of this that is personal data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupStatus {
    /// How many whole days since the last export, or `None` if there has never been one.
    pub days_since_last: Option<u32>,
    /// Whether the person should be reminded.
    pub remind: bool,
}

/// Writes down that an export happened, and where it went.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if a value cannot be encrypted and [`DbError::Sqlite`] if a
/// statement fails.
pub fn record_export(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    directory: &Path,
) -> Result<(), DbError> {
    settings::put(
        connection,
        codec,
        device,
        hlc,
        now_us,
        LAST_EXPORT_AT,
        Some(now_us.to_string().as_bytes()),
    )?;

    // Lossy on a path that is not valid Unicode, which on Windows means one built from
    // surrogates no dialog produces. Remembering a slightly wrong folder is a dialog that
    // opens in the wrong place; refusing the whole export over it would be worse.
    settings::put(
        connection,
        codec,
        device,
        hlc,
        now_us,
        LAST_DIRECTORY,
        Some(directory.to_string_lossy().as_bytes()),
    )?;

    Ok(())
}

/// The folder the last export went to, if one is remembered.
///
/// Used to open the system dialog where the person last was. Never reported to the interface
/// and never logged.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if the stored value does not decrypt and [`DbError::Sqlite`] if
/// the statement fails.
pub fn last_directory(
    connection: &Connection,
    codec: &FieldCodec<'_>,
) -> Result<Option<PathBuf>, DbError> {
    let Some(setting) = settings::get(connection, codec, LAST_DIRECTORY)? else {
        return Ok(None);
    };
    let Some(bytes) = setting.value else {
        return Ok(None);
    };
    let Ok(text) = String::from_utf8(bytes.to_vec()) else {
        return Ok(None);
    };

    Ok(Some(PathBuf::from(text)))
}

/// How long it has been, and whether to say so.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if the stored value does not decrypt and [`DbError::Sqlite`] if
/// the statement fails.
pub fn status(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    now_us: i64,
) -> Result<BackupStatus, DbError> {
    let Some(last_us) = last_export_at(connection, codec)? else {
        // Never exported. Reminded from the first day, because the person with no backup at
        // all is exactly the person this exists for.
        return Ok(BackupStatus {
            days_since_last: None,
            remind: true,
        });
    };

    let days = days_between(last_us, now_us);

    Ok(BackupStatus {
        days_since_last: Some(days),
        remind: days >= REMIND_AFTER_DAYS,
    })
}

/// The moment of the last export, if one was written down and still parses.
fn last_export_at(connection: &Connection, codec: &FieldCodec<'_>) -> Result<Option<i64>, DbError> {
    let Some(setting) = settings::get(connection, codec, LAST_EXPORT_AT)? else {
        return Ok(None);
    };
    let Some(bytes) = setting.value else {
        return Ok(None);
    };
    let Ok(text) = String::from_utf8(bytes.to_vec()) else {
        return Ok(None);
    };

    // A value that does not parse is treated as no value. It can only have got there by
    // somebody editing a decrypted backup, and the worst this can do is remind them.
    Ok(text.parse::<i64>().ok())
}

/// Whole days from one moment to another, and none if the second is not after the first.
fn days_between(from_us: i64, to_us: i64) -> u32 {
    let elapsed = to_us.saturating_sub(from_us);
    if elapsed <= 0 {
        return 0;
    }

    u32::try_from(elapsed.div_euclid(DAY_US)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use cairn_domain::Hlc;

    use super::{
        BackupStatus, LAST_DIRECTORY, REMIND_AFTER_DAYS, days_between, last_directory,
        record_export, status,
    };
    use crate::device::DeviceId;
    use crate::repositories::settings;
    use crate::test_support::Sandbox;

    const DAY_US: i64 = 24 * 60 * 60 * 1_000_000;
    const EXPORTED_AT: i64 = 1_700_000_000_000_000;

    /// A sandbox that has exported once, at [`EXPORTED_AT`].
    fn exported_once(label: &str) -> Sandbox {
        let sandbox = Sandbox::new(label);
        let device = DeviceId::generate().expect("random bytes");

        sandbox
            .database()
            .with(|connection| {
                record_export(
                    connection,
                    &sandbox.codec(),
                    device,
                    Hlc::new(1_000, 0, [1; 6]),
                    EXPORTED_AT,
                    Path::new("una carpeta"),
                )
            })
            .expect("the export is written down");

        sandbox
    }

    fn status_at(sandbox: &Sandbox, now_us: i64) -> BackupStatus {
        sandbox
            .database()
            .with(|connection| status(connection, &sandbox.codec(), now_us))
            .expect("the status reads")
    }

    #[test]
    fn a_vault_that_has_never_been_exported_is_reminded_from_the_start() {
        let sandbox = Sandbox::new("history-never");

        assert_eq!(
            status_at(&sandbox, EXPORTED_AT),
            BackupStatus {
                days_since_last: None,
                remind: true,
            }
        );
    }

    #[test]
    fn thirteen_days_is_not_yet_a_reminder() {
        let sandbox = exported_once("history-thirteen");

        assert_eq!(
            status_at(&sandbox, EXPORTED_AT + 13 * DAY_US),
            BackupStatus {
                days_since_last: Some(13),
                remind: false,
            }
        );
    }

    #[test]
    fn fifteen_days_is_a_reminder() {
        let sandbox = exported_once("history-fifteen");

        assert_eq!(
            status_at(&sandbox, EXPORTED_AT + 15 * DAY_US),
            BackupStatus {
                days_since_last: Some(15),
                remind: true,
            }
        );
    }

    #[test]
    fn the_fourteenth_day_is_the_first_one_that_reminds() {
        // The boundary itself, both sides of it, because "after fourteen days" and "on the
        // fourteenth day" are a day apart and only one of them is what the screen says.
        let sandbox = exported_once("history-boundary");

        assert!(!status_at(&sandbox, EXPORTED_AT + 13 * DAY_US + DAY_US - 1).remind);
        assert!(
            status_at(
                &sandbox,
                EXPORTED_AT + i64::from(REMIND_AFTER_DAYS) * DAY_US
            )
            .remind
        );
    }

    #[test]
    fn a_clock_that_has_gone_backwards_is_no_time_at_all() {
        // Somebody whose machine thinks it is last year should not be told their backup is
        // from the future, and should not be reminded either.
        let sandbox = exported_once("history-backwards");

        assert_eq!(
            status_at(&sandbox, EXPORTED_AT - 400 * DAY_US),
            BackupStatus {
                days_since_last: Some(0),
                remind: false,
            }
        );
    }

    #[test]
    fn days_are_whole_days_and_never_negative() {
        assert_eq!(days_between(0, 0), 0);
        assert_eq!(days_between(0, DAY_US - 1), 0);
        assert_eq!(days_between(0, DAY_US), 1);
        assert_eq!(days_between(DAY_US, 0), 0);
        // The widest gap two microsecond stamps can name, which saturates to `i64::MAX`
        // microseconds and is about a hundred and seven million days. It fits in a `u32`, so
        // nothing is lost; the conversion is still written to saturate rather than to wrap,
        // because a reminder that says minus three days is worse than one that says never.
        assert_eq!(days_between(i64::MIN, i64::MAX), 106_751_991);
    }

    #[test]
    fn the_folder_comes_back_the_way_it_went_in() {
        let sandbox = exported_once("history-folder");

        let remembered = sandbox
            .database()
            .with(|connection| last_directory(connection, &sandbox.codec()))
            .expect("the folder reads");

        assert_eq!(remembered.as_deref(), Some(Path::new("una carpeta")));
    }

    #[test]
    fn the_folder_is_not_readable_in_the_database_without_the_key() {
        // The one setting of this phase that is personal data: a path carries the name of
        // whoever is logged in, and very often the name of the machine.
        let sandbox = exported_once("history-sealed");

        let stored = sandbox
            .database()
            .with(|connection| {
                let bytes = connection.query_row(
                    "SELECT value FROM settings WHERE key = ?1",
                    [LAST_DIRECTORY],
                    |row| row.get::<_, Vec<u8>>(0),
                )?;
                Ok(bytes)
            })
            .expect("the row is there");

        assert!(
            !stored.windows(3).any(|window| window == b"una"),
            "the folder is in the clear in the database"
        );
    }

    #[test]
    fn a_value_that_does_not_parse_is_treated_as_never_exported() {
        // It can only have got there through a decrypted backup somebody edited, and the
        // worst this may do about it is remind them.
        let sandbox = exported_once("history-rubbish");
        let device = DeviceId::generate().expect("random bytes");

        sandbox
            .database()
            .with(|connection| {
                settings::put(
                    connection,
                    &sandbox.codec(),
                    device,
                    Hlc::new(2_000, 0, [1; 6]),
                    EXPORTED_AT,
                    super::LAST_EXPORT_AT,
                    Some(b"el jueves pasado"),
                )?;
                Ok(())
            })
            .expect("the setting writes");

        assert_eq!(
            status_at(&sandbox, EXPORTED_AT),
            BackupStatus {
                days_since_last: None,
                remind: true,
            }
        );
    }
}
