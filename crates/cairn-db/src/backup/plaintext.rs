//! Writing one module's data out with no protection on it at all.
//!
//! This is the one thing in Cairn that deliberately produces a file anybody can read. It
//! exists because data somebody cannot get out of an application is data held hostage, and a
//! vault whose only exit is its own format is a vault with a lock on the inside as well.
//!
//! Everything about it is therefore arranged to be hard to do by accident and impossible to do
//! quietly. The master password is asked for again and checked before this function is
//! reached; the screen says in plain Spanish what is about to happen; and the event is written
//! to `audit_events`, so the vault's own history records that on such a day, someone took a
//! readable copy of this module out of it.
//!
//! The format is CSV, and it goes one way. Nothing in this project reads it back: a plaintext
//! file has no version, no integrity and no way to tell a value that was empty from one that
//! was missing, and accepting it as input would be accepting all three of those as facts about
//! somebody's vault. What comes back in is a `.cairn` backup and nothing else.
//!
//! A module's file holds one block per table: a line naming the table, a header row of column
//! names, and then the rows. Blocks are separated by a blank line. That is not quite a
//! spreadsheet's idea of a CSV file, and it is what makes the file readable by the person who
//! asked for it, which is the only reason it exists.
//!
//! Rows marked deleted are left out. They are tombstones that the merge needs and that a
//! person reading their own habits does not, and a file full of rows that are not there any
//! more would make the export useless for the thing it is for.

use std::fs::{self, File};
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::backup::base64;
use crate::backup::format::RowValues;
use crate::backup::schema::{ColumnKind, ColumnSpec, TableSpec, table_named};
use crate::backup::tables::read_table;
use crate::codec::FieldCodec;
use crate::error::DbError;

/// What the half-written file is called while it is being written.
const PART_SUFFIX: &str = ".part";

/// A part of the application, as a person thinks of it.
///
/// Closed on purpose. This is what a caller may ask to have written out in the clear, and a
/// free-form table name arriving from the interface would be a way to ask for any table at
/// all, including the audit log that records these exports. That is also why it crosses the
/// bridge as an enumeration: what arrives is one of three words or nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Module {
    /// Habits, their areas, their entries and their pauses.
    Habits,
    /// The password vault: folders, entries, addresses, fields, history and tags.
    Vault,
    /// Accounts, categories, transactions and budgets.
    Finance,
}

impl Module {
    /// How the module is named in stored data and across the bridge.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Habits => "habits",
            Self::Vault => "vault",
            Self::Finance => "finance",
        }
    }

    /// The module a stored name refers to, if it is one this build knows.
    #[must_use]
    pub fn from_stored(text: &str) -> Option<Self> {
        match text {
            "habits" => Some(Self::Habits),
            "vault" => Some(Self::Vault),
            "finance" => Some(Self::Finance),
            _unknown => None,
        }
    }

    /// The tables this module is made of, in the order they are written.
    ///
    /// `settings` and `audit_events` belong to no module and are in none of these lists.
    /// Settings are the application's own state rather than the person's data, and the audit
    /// log is the record of exports like this one: a plaintext export that carried it would
    /// be an export of the evidence that exports happen.
    #[must_use]
    pub const fn tables(self) -> &'static [&'static str] {
        match self {
            Self::Habits => &["habit_areas", "habits", "habit_entries", "habit_pauses"],
            Self::Vault => &[
                "vault_folders",
                "vault_entries",
                "vault_urls",
                "vault_fields",
                "vault_password_history",
                "vault_tags",
                "vault_entry_tags",
            ],
            Self::Finance => &["accounts", "categories", "transactions", "budgets"],
        }
    }
}

/// What one plaintext export wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaintextReport {
    /// How many rows were written, across every table of the module.
    pub records: u64,
}

/// Writes one module out as readable CSV, replacing nothing.
///
/// The file is written under a temporary name beside the destination and renamed at the end,
/// so a failure halfway leaves no half-written file for somebody to mistake for a complete
/// one. The destination is refused if something is already there: this writes in the clear,
/// and writing over a file somebody chose by name is not a thing to do silently.
///
/// # Errors
///
/// Returns [`DbError::Io`] if the file cannot be written, [`DbError::Sealed`] if a stored
/// value does not decrypt where it was found, and [`DbError::Sqlite`] if a statement fails.
pub fn write_plaintext(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    module: Module,
    destination: &Path,
) -> Result<PlaintextReport, DbError> {
    if destination.exists() {
        return Err(DbError::Io {
            what: "the plaintext export",
            operation: "written",
            cause: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "there is already a file with that name",
            ),
        });
    }

    let part = part_path(destination);
    let outcome = write_all_tables(connection, codec, module, &part);

    match outcome {
        Ok(report) => {
            fs::rename(&part, destination).map_err(|cause| DbError::Io {
                what: "the plaintext export",
                operation: "renamed into place",
                cause,
            })?;

            Ok(report)
        }
        Err(failure) => {
            // Best effort, and deliberately not reported. The caller is about to be told why
            // the export failed, and a second message about the leftovers would replace it.
            fs::remove_file(&part).ok();

            Err(failure)
        }
    }
}

/// Writes every table of the module into one file.
fn write_all_tables(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    module: Module,
    part: &Path,
) -> Result<PlaintextReport, DbError> {
    let file = File::create(part).map_err(|cause| DbError::Io {
        what: "the plaintext export",
        operation: "created",
        cause,
    })?;
    let mut writer = BufWriter::new(file);
    let mut records = 0_u64;

    for (index, name) in module.tables().iter().enumerate() {
        // Every name comes from `Module::tables`, which is a constant of this crate, so a
        // table this build does not have is a schema that changed without this list changing
        // with it — the same class of problem the backup schema test exists to catch.
        let spec = table_named(name).ok_or(DbError::Malformed)?;

        if index > 0 {
            write_line(&mut writer, b"")?;
        }
        write_line(&mut writer, format!("# {name}").as_bytes())?;
        write_line(&mut writer, header_row(spec).as_bytes())?;

        let mut written = 0_u64;
        read_table(connection, codec, spec, |values| {
            if is_deleted(&values) {
                return Ok(());
            }

            write_line(&mut writer, row(spec, &values).as_bytes())?;
            written = written.saturating_add(1);

            Ok(())
        })?;

        records = records.saturating_add(written);
    }

    writer.flush().map_err(|cause| DbError::Io {
        what: "the plaintext export",
        operation: "written",
        cause,
    })?;
    writer
        .into_inner()
        .map_err(|cause| DbError::Io {
            what: "the plaintext export",
            operation: "written",
            cause: std::io::Error::other(cause.to_string()),
        })?
        // Before the rename, never after, for the same reason an encrypted export does it:
        // a name that lands before the bytes do is a file that looks complete and is not.
        .sync_all()
        .map_err(|cause| DbError::Io {
            what: "the plaintext export",
            operation: "flushed to disk",
            cause,
        })?;

    Ok(PlaintextReport { records })
}

/// Whether a row is a tombstone.
fn is_deleted(values: &RowValues) -> bool {
    values
        .get("deleted")
        .and_then(serde_json::Value::as_i64)
        .is_some_and(|flag| flag != 0)
}

/// The header line: every column of the table, in order.
fn header_row(spec: &TableSpec) -> String {
    let names: Vec<String> = spec.columns().map(|column| quoted(column.name)).collect();

    names.join(",")
}

/// One row, as its columns in the table's order.
fn row(spec: &TableSpec, values: &RowValues) -> String {
    let cells: Vec<String> = spec
        .columns()
        .map(|column| quoted(&cell(column, values)))
        .collect();

    cells.join(",")
}

/// One cell, as the text a person should see.
///
/// The three interesting cases are all about what a value is rather than what it holds. A
/// sealed column carries somebody's own words and is shown as words: it arrives base64
/// encoded because that is how a backup record carries bytes, and it is decoded back. A blob
/// column carries an identifier or a clock reading and stays base64, because there is nothing
/// readable in it to show. Everything else is already text or a number.
fn cell(column: ColumnSpec, values: &RowValues) -> String {
    let Some(value) = values.get(column.name) else {
        return String::new();
    };

    match (column.kind, value) {
        (_any, serde_json::Value::Null) => String::new(),
        (ColumnKind::Sealed, serde_json::Value::String(encoded)) => readable(encoded),
        (_other, serde_json::Value::String(text)) => text.clone(),
        (_other, other) => other.to_string(),
    }
}

/// A sealed value as text, or as it arrived if it is not text.
///
/// Falls back rather than guessing. A sealed column holds whatever the application put in it,
/// which is text everywhere in this schema today; a value that is not valid UTF-8 is either a
/// column that will hold bytes one day or a row that is wrong, and showing the base64 says so
/// without inventing characters that were never there.
fn readable(encoded: &str) -> String {
    let Some(bytes) = base64::decode(encoded) else {
        return encoded.to_owned();
    };
    let bytes = Zeroizing::new(bytes);

    String::from_utf8(bytes.to_vec()).unwrap_or_else(|_not_text| encoded.to_owned())
}

/// One field, quoted the way every CSV reader agrees on.
///
/// Always quoted, rather than only when it has to be. A rule with no exceptions is a rule
/// with nowhere for a comma, a newline or a quotation mark inside somebody's note to change
/// what the rest of the line means.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        if character == '"' {
            out.push('"');
        }
        out.push(character);
    }
    out.push('"');

    out
}

/// Writes one line, with the ending every spreadsheet reads.
fn write_line(writer: &mut impl std::io::Write, line: &[u8]) -> Result<(), DbError> {
    writer
        .write_all(line)
        .and_then(|()| writer.write_all(b"\r\n"))
        .map_err(|cause| DbError::Io {
            what: "the plaintext export",
            operation: "written",
            cause,
        })
}

/// The temporary name beside a destination.
fn part_path(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_os_string();
    name.push(PART_SUFFIX);

    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use cairn_domain::{CivilDay, Hlc};

    use super::{Module, part_path, quoted, write_plaintext};
    use crate::backup::schema::TABLES;
    use crate::device::DeviceId;
    use crate::error::DbError;
    use crate::repositories::habits;
    use crate::test_support::Sandbox;

    const NOW_US: i64 = 1_700_000_000_000_000;

    /// A sandbox with two habits in it, one of them a tombstone.
    fn seeded(label: &str) -> Sandbox {
        let sandbox = Sandbox::new(label);
        let device = DeviceId::generate().expect("random bytes");
        let day = CivilDay::new(2026, 9, 17).expect("a day that exists");

        sandbox
            .database()
            .with(|connection| {
                habits::create(
                    connection,
                    &sandbox.codec(),
                    device,
                    Hlc::new(1_000, 0, [1; 6]),
                    NOW_US,
                    habits::NewHabit {
                        name: "Correr, \"la de verdad\"",
                        notes: Some("cinco kilómetros,\ncada mañana".as_bytes()),
                        started_on: day,
                        position: 0,
                    },
                )?;

                let gone = habits::create(
                    connection,
                    &sandbox.codec(),
                    device,
                    Hlc::new(1_001, 0, [1; 6]),
                    NOW_US,
                    habits::NewHabit {
                        name: "Un hábito borrado",
                        notes: None,
                        started_on: day,
                        position: 1,
                    },
                )?;
                habits::delete(connection, Hlc::new(1_002, 0, [1; 6]), NOW_US, gone.id)?;

                Ok(())
            })
            .expect("the seed writes");

        sandbox
    }

    #[test]
    fn a_field_is_always_quoted_and_a_quotation_mark_is_doubled() {
        assert_eq!(quoted("hola"), "\"hola\"");
        assert_eq!(quoted("di \"hola\""), "\"di \"\"hola\"\"\"");
        assert_eq!(quoted("una,coma"), "\"una,coma\"");
        assert_eq!(quoted("un\nsalto"), "\"un\nsalto\"");
    }

    #[test]
    fn the_file_holds_the_words_somebody_typed() {
        // The whole point, and the uncomfortable half of it: what comes out is readable.
        let sandbox = seeded("plaintext-readable");
        let destination = sandbox.directory().join("habitos.csv");

        let report = sandbox
            .database()
            .with(|connection| {
                write_plaintext(connection, &sandbox.codec(), Module::Habits, &destination)
            })
            .expect("the export works");

        let text = std::fs::read_to_string(&destination).expect("the file is readable");

        assert!(
            text.contains("cinco kilómetros"),
            "the note is not in it: {text}"
        );
        assert!(
            text.contains("\"Correr, \"\"la de verdad\"\"\""),
            "a quotation mark was not doubled: {text}"
        );
        assert!(
            !text.contains("Un hábito borrado"),
            "a deleted row was written out: {text}"
        );
        assert_eq!(report.records, 1);
    }

    #[test]
    fn every_table_of_the_module_gets_a_block_of_its_own() {
        let sandbox = seeded("plaintext-blocks");
        let destination = sandbox.directory().join("habitos.csv");

        sandbox
            .database()
            .with(|connection| {
                write_plaintext(connection, &sandbox.codec(), Module::Habits, &destination)
            })
            .expect("the export works");

        let text = std::fs::read_to_string(&destination).expect("the file is readable");

        for table in Module::Habits.tables() {
            assert!(text.contains(&format!("# {table}")), "{table} is missing");
        }
    }

    #[test]
    fn nothing_of_another_module_is_in_the_file() {
        // The promise the screen makes: this exports one module, not the vault.
        let sandbox = seeded("plaintext-one-module");
        let destination = sandbox.directory().join("habitos.csv");

        sandbox
            .database()
            .with(|connection| {
                write_plaintext(connection, &sandbox.codec(), Module::Habits, &destination)
            })
            .expect("the export works");

        let text = std::fs::read_to_string(&destination).expect("the file is readable");

        for table in Module::Vault
            .tables()
            .iter()
            .chain(Module::Finance.tables())
        {
            assert!(!text.contains(&format!("# {table}")), "{table} is in it");
        }
        assert!(!text.contains("# settings"), "the settings are in it");
        assert!(!text.contains("# audit_events"), "the audit log is in it");
    }

    #[test]
    fn a_destination_that_is_already_there_is_refused_and_left_alone() {
        let sandbox = seeded("plaintext-clash");
        let destination = sandbox.directory().join("habitos.csv");
        std::fs::write(&destination, b"algo que ya estaba").expect("the file writes");

        let refused = sandbox
            .database()
            .with(|connection| {
                write_plaintext(connection, &sandbox.codec(), Module::Habits, &destination)
            })
            .expect_err("writing over a file is refused");

        assert!(matches!(refused, DbError::Io { .. }), "{refused:?}");
        assert_eq!(
            std::fs::read(&destination).expect("the file is readable"),
            b"algo que ya estaba"
        );
        assert!(!part_path(&destination).exists());
    }

    #[test]
    fn every_table_belongs_to_at_most_one_module() {
        // Guards the three lists against the mistake that would make an export of one module
        // quietly carry another's rows.
        let mut seen: Vec<&str> = Vec::new();
        for module in [Module::Habits, Module::Vault, Module::Finance] {
            for table in module.tables() {
                assert!(!seen.contains(table), "{table} is in two modules");
                seen.push(table);
            }
        }
    }

    #[test]
    fn every_table_a_module_names_is_one_the_schema_has() {
        for module in [Module::Habits, Module::Vault, Module::Finance] {
            for name in module.tables() {
                assert!(
                    TABLES.iter().any(|table| table.name == *name),
                    "{name} is not a table this build has"
                );
            }
        }
    }

    #[test]
    fn a_module_survives_being_written_down_and_read_back() {
        for module in [Module::Habits, Module::Vault, Module::Finance] {
            assert_eq!(Module::from_stored(module.as_str()), Some(module));
        }
        assert_eq!(Module::from_stored("audit_events"), None);
        assert_eq!(Module::from_stored(""), None);
    }
}
