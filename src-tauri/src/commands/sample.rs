//! The four commands the diagnostics screen uses to prove that storage works.
//!
//! They exist for one manual test: create a habit, list it, delete it, restart, and see that it
//! is still there as a tombstone with nothing inside it. That test is worth having because it is
//! the only one that exercises the whole path — the WebView, the command boundary, the session
//! lock, the codec, SQLCipher and the file — on a real machine with a real window.
//!
//! They write to `habits`, which is a real table, not a table invented for testing. A test table
//! would prove that a test table works. The name is a constant of this module, so nothing a
//! person types reaches the row, and the note is a constant too.
//!
//! Nothing here returns content. A sample habit comes back as an identifier, a name this module
//! chose and a flag, and the note never crosses the bridge in either direction. That keeps the
//! diagnostics screen inside the rule the rest of it follows: a screenshot of it has to be safe
//! to paste into a public issue.

use cairn_crypto::UnlockedVault;
use cairn_db::DbError;
use cairn_db::repositories::habits::{self, Habit, MAX_PAGE, NewHabit};
use cairn_db::tombstones;
use cairn_domain::{CivilDay, Hlc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::clock::{now_ms, now_us};
use crate::state::AppState;
use crate::storage::Storage;

/// What every sample habit is called.
///
/// A constant rather than an argument. The command takes no input at all, so there is nothing
/// arriving from the WebView that can reach a row, and the manual test does not depend on
/// whoever runs it typing the same thing twice.
pub const SAMPLE_NAME: &str = "Hábito de prueba";

/// The note every sample habit carries, which never comes back out.
const SAMPLE_NOTE: &[u8] = "Escrito por la pantalla de diagnóstico.".as_bytes();

/// The most rows one seeding call may write per table.
///
/// A hard ceiling on the number that arrives from the WebView. A generator is exactly the shape
/// of thing somebody points at a slider, and a number from the other side of the bridge does not
/// get to decide how long this process spends in a transaction.
pub const MAX_SEED_ROWS: u32 = 10_000;

/// Why a sample operation did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[non_exhaustive]
pub enum SampleError {
    /// The vault is closed, and all four of these need it open.
    #[error("the vault is locked")]
    Locked,

    /// There is no sample habit with that identifier, or it has already been deleted.
    #[error("there is no such sample habit")]
    NotFound,

    /// A number arrived that is larger than one call may do.
    #[error("{what} is {value}, and at most {max} is allowed")]
    #[serde(rename_all = "camelCase")]
    TooMany {
        /// What was being counted.
        what: &'static str,
        /// What was asked for.
        value: u64,
        /// The most that is allowed.
        max: u64,
    },

    /// The database refused. Deliberately without the reason.
    ///
    /// Whoever is repairing a machine reaches the cause through the log; what reaches the screen
    /// is that storage failed, for the same reason an unlock says the vault did not open and
    /// nothing else.
    #[error("the database could not complete the operation")]
    Storage,
}

impl From<DbError> for SampleError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::NotFound => Self::NotFound,
            DbError::Closed => Self::Locked,
            DbError::TooMany { what, value, max } => Self::TooMany { what, value, max },
            _other => Self::Storage,
        }
    }
}

/// One sample habit, as the screen shows it.
///
/// Three facts and a cursor. No note, no moment, no device: the note is content and the other
/// two are about this machine, and neither belongs on a screen whose whole purpose is to be
/// safe to screenshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleHabit {
    /// The row's identifier, as a hyphenated UUID.
    pub id: String,
    /// What it is called, which is always [`SAMPLE_NAME`].
    pub name: String,
    /// Whether it is a tombstone.
    pub deleted: bool,
    /// Where the next page starts, as the thirty-two hexadecimal characters of its clock reading.
    pub cursor: String,
}

impl SampleHabit {
    /// Describes a habit the repository handed back.
    fn of(habit: &Habit) -> Self {
        Self {
            id: habit.id.to_string(),
            name: habit.name.clone(),
            deleted: habit.deleted,
            cursor: hex(&habit.hlc.to_bytes()),
        }
    }
}

/// Where a page starts and how big it is.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysetPage {
    /// The cursor of the last row of the previous page, or nothing for the first page.
    pub after: Option<String>,
    /// How many rows to return.
    pub limit: u32,
}

/// What a seeding run wrote, and how long it took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedReport {
    /// One entry per table written to, in the order they were written.
    pub tables: Vec<SeededTable>,
    /// How long the whole run took, in milliseconds.
    pub elapsed_ms: u64,
}

/// What one table gained during a seeding run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeededTable {
    /// The table, as the schema names it.
    pub table: &'static str,
    /// How many rows were written into it.
    pub rows: u32,
}

/// What a compaction removed, for the diagnostics screen.
///
/// Counts and table names, and nothing else. The tables are named by the schema, so this is safe
/// to put in a screenshot for the same reason the rest of that screen is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionReport {
    /// What went, per table, skipping the ones nothing went from.
    pub tables: Vec<SeededTable>,
    /// The total across every table.
    pub removed: u64,
    /// How many tombstones are left.
    pub remaining: u64,
    /// How long it took.
    pub elapsed_ms: u64,
}

/// Removes the tombstones that are older than the retention window.
///
/// The one operation in the application that runs a `DELETE`, offered here because it is the one
/// number on the diagnostics screen a person cannot otherwise move. What it removes is a
/// skeleton: every encrypted column of those rows was emptied when they were marked, so there is
/// nothing left in them to lose, and a row marked yesterday is not a candidate.
///
/// Takes no argument. The window is a constant of the core, and a retention period arriving from
/// a WebView would be a way to ask this process to empty the file.
///
/// # Errors
///
/// Returns [`SampleError::Locked`] if the vault is closed and [`SampleError::Storage`] if the
/// database refuses.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn diagnostics_compact_tombstones(
    state: tauri::State<'_, AppState>,
) -> Result<CompactionReport, SampleError> {
    let micros = now_us();
    let started = std::time::Instant::now();

    let (removed, remaining) = state
        .session()
        .with_open(|_vault, storage| {
            storage.database().with(|connection| {
                let removed = tombstones::compact(connection, micros)?;
                let remaining = tombstones::census(connection)?;
                Ok((removed, remaining))
            })
        })
        .ok_or(SampleError::Locked)??;

    Ok(CompactionReport {
        tables: removed
            .by_table
            .into_iter()
            .map(|(table, rows)| SeededTable {
                table,
                rows: u32::try_from(rows).unwrap_or(u32::MAX),
            })
            .collect(),
        removed: removed.total,
        remaining: remaining.total,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// Writes one sample habit and answers what it wrote.
///
/// # Errors
///
/// Returns [`SampleError::Locked`] if the vault is closed and [`SampleError::Storage`] if the
/// database refuses.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn diagnostics_insert_sample_habit(
    state: tauri::State<'_, AppState>,
) -> Result<SampleHabit, SampleError> {
    let micros = now_us();
    let millis = now_ms();

    state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            let hlc = storage.next_hlc(millis);

            storage.database().with(|connection| {
                habits::create(
                    connection,
                    &codec,
                    storage.device(),
                    hlc,
                    micros,
                    NewHabit {
                        name: SAMPLE_NAME,
                        notes: Some(SAMPLE_NOTE),
                        started_on: today(),
                        position: 0,
                    },
                )
            })
        })
        .ok_or(SampleError::Locked)?
        .map(|habit| SampleHabit::of(&habit))
        .map_err(SampleError::from)
}

/// Reads a page of sample habits, in clock order.
///
/// # Errors
///
/// Returns [`SampleError::Locked`] if the vault is closed, [`SampleError::TooMany`] if the page
/// asked for is larger than the repository allows, and [`SampleError::Storage`] if the database
/// refuses or a stored row does not decode.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn diagnostics_list_sample_habits(
    state: tauri::State<'_, AppState>,
    page: KeysetPage,
) -> Result<Vec<SampleHabit>, SampleError> {
    let limit = usize::try_from(page.limit).unwrap_or(usize::MAX);
    if limit == 0 || limit > MAX_PAGE {
        return Err(SampleError::TooMany {
            what: "the size of a page of habits",
            value: u64::from(page.limit),
            max: MAX_PAGE as u64,
        });
    }

    // Parsed before the lock is taken, and a cursor that is not thirty-two hexadecimal
    // characters starts from the beginning rather than being refused. It is an opaque token this
    // application handed out; a caller that damages it gets the first page, not an error screen.
    let after = page.after.as_deref().and_then(cursor);

    state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage
                .database()
                .with(|connection| habits::page(connection, &codec, after, limit))
        })
        .ok_or(SampleError::Locked)?
        .map(|habits| habits.iter().map(SampleHabit::of).collect())
        .map_err(SampleError::from)
}

/// Marks a sample habit as deleted and empties its encrypted column.
///
/// # Errors
///
/// Returns [`SampleError::Locked`] if the vault is closed, [`SampleError::NotFound`] if the
/// identifier does not name a live habit, and [`SampleError::Storage`] if the database refuses.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn diagnostics_delete_sample_habit(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<SampleHabit, SampleError> {
    // An identifier that is not one cannot name a row, so it is the same answer as an identifier
    // that names nothing. Nothing is built out of it before it has been parsed.
    let id = Uuid::parse_str(&id).map_err(|_not_a_uuid| SampleError::NotFound)?;
    let micros = now_us();
    let millis = now_ms();

    state
        .session()
        .with_open(|_vault, storage| {
            let hlc = storage.next_hlc(millis);
            storage
                .database()
                .with(|connection| habits::delete(connection, hlc, micros, id))
        })
        .ok_or(SampleError::Locked)?
        .map(|habit| SampleHabit::of(&habit))
        .map_err(SampleError::from)
}

/// Writes a number of rows into every table that has a generator, for measuring.
///
/// One transaction for the whole run. Ten thousand separate commits is a measurement of the
/// disk's flush behaviour rather than of anything this application does.
///
/// # Errors
///
/// Returns [`SampleError::Locked`] if the vault is closed, [`SampleError::TooMany`] if more than
/// [`MAX_SEED_ROWS`] rows per table are asked for, and [`SampleError::Storage`] if the database
/// refuses.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn diagnostics_seed_data(
    state: tauri::State<'_, AppState>,
    rows_per_table: u32,
) -> Result<SeedReport, SampleError> {
    if rows_per_table == 0 || rows_per_table > MAX_SEED_ROWS {
        return Err(SampleError::TooMany {
            what: "the number of rows per table",
            value: u64::from(rows_per_table),
            max: u64::from(MAX_SEED_ROWS),
        });
    }

    let micros = now_us();
    let millis = now_ms();
    let started = std::time::Instant::now();

    let written = state
        .session()
        .with_open(|vault, storage| seed(storage, vault, rows_per_table, millis, micros))
        .ok_or(SampleError::Locked)??;

    Ok(SeedReport {
        tables: vec![SeededTable {
            table: "habits",
            rows: written,
        }],
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// Writes the rows of one seeding run, inside one transaction.
fn seed(
    storage: &Storage,
    vault: &UnlockedVault,
    rows: u32,
    millis: u64,
    micros: i64,
) -> Result<u32, SampleError> {
    let codec = storage.codec(vault);
    let device = storage.device();

    storage.database().in_transaction(|transaction| {
        for ordinal in 0..rows {
            let name = format!("{SAMPLE_NAME} {ordinal}");
            habits::create(
                transaction,
                &codec,
                device,
                storage.next_hlc(millis),
                micros,
                NewHabit {
                    name: &name,
                    notes: Some(SAMPLE_NOTE),
                    started_on: today(),
                    position: i64::from(ordinal),
                },
            )?;
        }

        Ok(rows)
    })?;

    Ok(rows)
}

/// The day every sample row is started on.
///
/// A fixed day rather than today's. Turning a moment into a day needs a place, and the place is
/// a decision the calendar work makes later; a sample row does not need the right answer to a
/// question nothing else in this build has answered yet.
fn today() -> CivilDay {
    // Checked by the type, with a fallback that needs no checking. Neither branch can fail, and
    // neither reaches a constructor whose answer somebody has to decide what to do about.
    CivilDay::new(2026, 1, 1).unwrap_or(CivilDay::UNIX_EPOCH)
}

/// Reads a cursor, answering `None` for anything that is not one.
fn cursor(text: &str) -> Option<Hlc> {
    // The length and the alphabet are checked before parsing, because `from_str_radix` accepts
    // a leading sign and would read "+000â¦" as a number. A cursor is an opaque token this
    // application handed out, and the only shape it is ever handed out in is thirty-two
    // hexadecimal digits.
    if text.len() != 32 || !text.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }

    Some(Hlc::from_bytes(
        u128::from_str_radix(text, 16).ok()?.to_be_bytes(),
    ))
}

/// Formats bytes as lower case hexadecimal.
fn hex(bytes: &[u8; 16]) -> String {
    format!("{:032x}", u128::from_be_bytes(*bytes))
}

#[cfg(test)]
mod tests {
    use cairn_domain::Hlc;

    use super::{KeysetPage, SampleError, SampleHabit, cursor, hex};

    #[test]
    fn a_cursor_survives_being_written_out_and_read_back() {
        let reading = Hlc::new(1_700_000_000_123, 7, [1, 2, 3, 4, 5, 6]);
        let text = hex(&reading.to_bytes());

        assert_eq!(text.len(), 32);
        assert_eq!(cursor(&text), Some(reading));
    }

    #[test]
    fn a_cursor_that_is_not_one_starts_from_the_beginning_rather_than_failing() {
        // It is an opaque token this application handed out. A caller that damages it gets the
        // first page, which is a page; an error screen would be a worse answer to a worse
        // question.
        for text in ["", "zz", &"g".repeat(32), &"0".repeat(31), &"0".repeat(33)] {
            assert_eq!(cursor(text), None, "{text} was accepted as a cursor");
        }
    }

    #[test]
    fn the_error_serialises_as_a_tagged_object_the_interface_can_match_on() {
        let encoded = serde_json::to_string(&SampleError::Locked).expect("it serialises");
        assert_eq!(encoded, r#"{"kind":"locked"}"#);

        let encoded = serde_json::to_string(&SampleError::TooMany {
            what: "the number of rows per table",
            value: 20_000,
            max: 10_000,
        })
        .expect("it serialises");
        assert!(encoded.contains(r#""kind":"tooMany""#), "{encoded}");
        assert!(encoded.contains(r#""value":20000"#), "{encoded}");
    }

    #[test]
    fn a_sample_habit_carries_nothing_that_is_not_meant_to_leave_the_core() {
        let encoded = serde_json::to_string(&SampleHabit {
            id: "00000000-0000-4000-8000-000000000000".to_owned(),
            name: "Hábito de prueba".to_owned(),
            deleted: false,
            cursor: "0".repeat(32),
        })
        .expect("it serialises");

        for forbidden in ["note", "notes", "device", "createdAt", "updatedAt"] {
            assert!(
                !encoded.contains(forbidden),
                "{forbidden} reached the interface"
            );
        }
    }

    #[test]
    fn a_page_is_read_from_the_shape_the_interface_sends() {
        let page: KeysetPage = serde_json::from_str(r#"{"after":null,"limit":25}"#)
            .expect("the interface's shape is accepted");

        assert_eq!(page.after, None);
        assert_eq!(page.limit, 25);
    }
}
