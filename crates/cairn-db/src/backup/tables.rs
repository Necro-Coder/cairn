//! Turning rows into backup records and back again.
//!
//! The one decision worth understanding here is what happens to an encrypted column. Its
//! ciphertext is authenticated against the table, the row, the column and the revision it
//! was written at, and against the identifier of the key that sealed it. None of that
//! survives a move into another database: another installation has another key, and the
//! same row imported twice is at a different revision each time. So a ciphertext copied
//! across as-is would be a ciphertext that never opens again.
//!
//! What travels is therefore the plaintext, protected by the backup's own encryption and by
//! nothing else, and sealed again on the way in under the receiving vault's key at the
//! revision the row actually lands on. The consequence is stated in the security
//! documentation in exactly these words: inside a decrypted backup, the passwords are in
//! the clear. The whole protection of the file is Argon2id over the password it was made
//! with.
//!
//! Reading is paged by identifier rather than by offset, so the last page of a long table
//! costs what the first one did. Writing goes row by row into a database that is not the
//! live one, so nothing here can damage anything: the caller decides much later, and only
//! after the whole file has been read, whether that database ever becomes the real one.

use rusqlite::types::Value;
use rusqlite::{Connection, ToSql};
use uuid::Uuid;
use zeroize::Zeroizing;

use cairn_domain::Rev;

use crate::backup::base64;
use crate::backup::format::{MAX_FIELD_BYTES, MAX_ROWS_PER_TABLE, RowValues};
use crate::backup::schema::{ColumnKind, TableSpec};
use crate::codec::{FieldCodec, RowKey};
use crate::error::DbError;
use crate::row::sixteen;

/// How many rows are read at a time.
///
/// Five hundred. Large enough that the statement overhead disappears, small enough that one
/// page of the widest table is well under a mebibyte, which is what keeps the memory of an
/// export flat regardless of how much there is to export.
const PAGE: i64 = 500;

/// Reads every row of one table, handing each to `emit` as it is read.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] if the table holds more rows than a backup may carry,
/// [`DbError::Sealed`] if a stored value does not decrypt where it was found, and
/// [`DbError::Sqlite`] if a statement fails.
pub fn read_table(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    table: &TableSpec,
    mut emit: impl FnMut(RowValues) -> Result<(), DbError>,
) -> Result<u64, DbError> {
    let names: Vec<&str> = table.columns().map(|column| column.name).collect();
    let projection = names.join(", ");
    // Both halves come from `TABLES`, which is a constant of this crate checked against the
    // schema by a test. Nothing a caller supplies reaches this string, which is the only
    // form of dynamic SQL this project allows.
    let statement = format!(
        "SELECT {projection} FROM {} WHERE id > ?1 ORDER BY id LIMIT ?2",
        table.name
    );

    let mut cursor = vec![0_u8; 16];
    let mut written = 0_u64;

    loop {
        let mut prepared = connection.prepare_cached(&statement)?;
        let mut rows = prepared.query(rusqlite::params![cursor, PAGE])?;

        let mut in_page = 0_i64;
        while let Some(row) = rows.next()? {
            let stored: Vec<Value> = (0..names.len())
                .map(|index| row.get::<_, Value>(index))
                .collect::<rusqlite::Result<Vec<Value>>>()?;

            let Some(Value::Blob(id)) = stored.first() else {
                return Err(damaged());
            };
            cursor.clone_from(id);

            written = written.saturating_add(1);
            if written > MAX_ROWS_PER_TABLE {
                return Err(DbError::TooMany {
                    what: "rows in one table",
                    value: written,
                    max: MAX_ROWS_PER_TABLE,
                });
            }

            emit(encode_row(codec, table, &stored)?)?;
            in_page += 1;
        }

        if in_page < PAGE {
            break;
        }
    }

    Ok(written)
}

/// Turns one stored row into the values a backup carries.
fn encode_row(
    codec: &FieldCodec<'_>,
    table: &TableSpec,
    stored: &[Value],
) -> Result<RowValues, DbError> {
    let key = row_key(table, stored)?;
    let mut values = RowValues::new();

    for (column, value) in table.columns().zip(stored) {
        let encoded = match (column.kind, value) {
            (_any, Value::Null) => serde_json::Value::Null,
            (ColumnKind::Integer, Value::Integer(number)) => serde_json::json!(number),
            (ColumnKind::Text, Value::Text(text)) => {
                check_field_len(text.len())?;
                serde_json::json!(text)
            }
            (ColumnKind::Blob, Value::Blob(bytes)) => {
                check_field_len(bytes.len())?;
                serde_json::json!(base64::encode(bytes))
            }
            (ColumnKind::Sealed, Value::Blob(bytes)) => {
                let plaintext = codec.open(key, column.name, bytes)?;
                check_field_len(plaintext.len())?;
                serde_json::json!(base64::encode(&plaintext))
            }
            // A column holding something the schema says it cannot hold means the file was
            // written by something that is not this program, which is the same class of
            // problem as a value that does not decrypt and gets the same answer.
            _mismatched => return Err(damaged()),
        };

        values.insert(column.name.to_owned(), encoded);
    }

    Ok(values)
}

/// Writes one row into a database, sealing its encrypted columns under that database's key.
///
/// # Errors
///
/// Returns [`DbError::Malformed`] if the row is not exactly the set of columns the table
/// has, or if any value is not the shape its column takes, [`DbError::TooMany`] if a value
/// is longer than one may be, [`DbError::Sealed`] if a value cannot be encrypted, and
/// [`DbError::Sqlite`] if the insert fails.
pub fn write_row(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    table: &TableSpec,
    values: &RowValues,
) -> Result<(), DbError> {
    // Every column exactly once, and nothing else. A row missing a column would be written
    // with a default nobody chose; a row carrying one the table does not have is a row from
    // a schema this build does not know.
    if values.len() != table.column_count() {
        return Err(DbError::Malformed);
    }
    for name in values.keys() {
        if table.column(name).is_none() {
            return Err(DbError::Malformed);
        }
    }

    let decoded = decode_row(table, values)?;
    let key = row_key(table, &decoded)?;

    let mut bound: Vec<Value> = Vec::with_capacity(decoded.len());
    for (column, value) in table.columns().zip(&decoded) {
        match (column.kind, value) {
            (ColumnKind::Sealed, Value::Blob(plaintext)) => {
                // Zeroized before it is handed to the sealer: this is a password on its way
                // from the backup into the staging database, and it is in the clear here.
                let guarded = Zeroizing::new(plaintext.clone());
                bound.push(Value::Blob(codec.seal(key, column.name, &guarded)?));
            }
            _other => bound.push(value.clone()),
        }
    }

    let names: Vec<&str> = table.columns().map(|column| column.name).collect();
    let placeholders: Vec<String> = (1..=names.len()).map(|index| format!("?{index}")).collect();
    let statement = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        table.name,
        names.join(", "),
        placeholders.join(", ")
    );

    let parameters: Vec<&dyn ToSql> = bound.iter().map(|value| value as &dyn ToSql).collect();
    connection
        .prepare_cached(&statement)?
        .execute(parameters.as_slice())?;

    Ok(())
}

/// Turns the values of one backup record into what SQLite stores.
fn decode_row(table: &TableSpec, values: &RowValues) -> Result<Vec<Value>, DbError> {
    let mut decoded = Vec::with_capacity(table.column_count());

    for column in table.columns() {
        let Some(value) = values.get(column.name) else {
            return Err(DbError::Malformed);
        };

        decoded.push(match (column.kind, value) {
            (_any, serde_json::Value::Null) => Value::Null,
            (ColumnKind::Integer, serde_json::Value::Number(number)) => {
                // Only whole numbers that fit. A floating point value here would be a
                // quantity of money or a moment that cannot be represented, and rounding it
                // silently is how a ledger stops adding up.
                let Some(whole) = number.as_i64() else {
                    return Err(DbError::Malformed);
                };
                Value::Integer(whole)
            }
            (ColumnKind::Text, serde_json::Value::String(text)) => {
                check_field_len(text.len())?;
                Value::Text(text.clone())
            }
            (ColumnKind::Blob | ColumnKind::Sealed, serde_json::Value::String(text)) => {
                // Two checks, on two different lengths, and the first one is not the limit.
                // It bounds what is decoded before a buffer is reserved for it, so it has to
                // be the encoded size of the limit rather than the limit itself: base64 is
                // four characters for every three bytes, and checking the limit against the
                // text would refuse a field this same code is happy to write. It did, and
                // the export's own verification pass is what caught it.
                check_encoded_field_len(text.len())?;
                let Some(bytes) = base64::decode(text) else {
                    return Err(DbError::Malformed);
                };
                check_field_len(bytes.len())?;
                Value::Blob(bytes)
            }
            _mismatched => return Err(DbError::Malformed),
        });
    }

    Ok(decoded)
}

/// The identity a row's encrypted columns are authenticated against.
///
/// Read out of the row's own first and seventh columns rather than passed in, because those
/// are what the associated data was built from when the value was sealed. Taking them from
/// anywhere else would be taking them from something that can disagree.
fn row_key<'a>(table: &'a TableSpec, stored: &[Value]) -> Result<RowKey<'a>, DbError> {
    let Some(Value::Blob(id)) = stored.first() else {
        return Err(damaged());
    };
    let Some(Value::Integer(rev)) = stored.get(6) else {
        return Err(damaged());
    };

    Ok(RowKey {
        table: table.name,
        row_id: Uuid::from_bytes(sixteen(id)?),
        rev: Rev::from_number(u64::try_from(*rev).map_err(|_negative| damaged())?),
    })
}

/// Refuses a value longer than one may be.
fn check_field_len(len: usize) -> Result<(), DbError> {
    if len > MAX_FIELD_BYTES {
        return Err(DbError::TooMany {
            what: "bytes in one field",
            value: u64::try_from(len).unwrap_or(u64::MAX),
            max: u64::try_from(MAX_FIELD_BYTES).unwrap_or(u64::MAX),
        });
    }

    Ok(())
}

/// Refuses base64 text longer than the longest a field within the limit could produce.
///
/// The bound that runs before anything is decoded, so that a hostile file cannot ask for a
/// large buffer by sending a large string. It is the encoded size of [`MAX_FIELD_BYTES`]:
/// four characters for every three bytes, rounded up to the next whole group of four.
fn check_encoded_field_len(len: usize) -> Result<(), DbError> {
    let max = MAX_FIELD_BYTES.div_ceil(3) * 4;

    if len > max {
        return Err(DbError::TooMany {
            what: "base64 characters in one field",
            value: u64::try_from(len).unwrap_or(u64::MAX),
            max: u64::try_from(max).unwrap_or(u64::MAX),
        });
    }

    Ok(())
}

/// What a row this program did not write is reported as.
fn damaged() -> DbError {
    DbError::Sealed(cairn_crypto::CryptoError::Open)
}

#[cfg(test)]
mod tests {
    use cairn_domain::Hlc;

    use super::{read_table, write_row};
    use crate::backup::format::{MAX_FIELD_BYTES, RowValues};
    use crate::backup::schema::table_named;
    use crate::device::DeviceId;
    use crate::error::DbError;
    use crate::repositories::settings;
    use crate::test_support::Sandbox;

    const NOW_US: i64 = 1_700_000_000_000_000;
    const HLC: Hlc = Hlc::new(1_000, 0, [1; 6]);

    /// Reads a whole table into memory, which is what a test wants and an export never does.
    fn rows_of(sandbox: &Sandbox, table: &str) -> Vec<RowValues> {
        let codec = sandbox.codec();
        let spec = table_named(table).expect("the table is carried");

        sandbox
            .database()
            .with(|connection| {
                let mut rows = Vec::new();
                read_table(connection, &codec, spec, |values| {
                    rows.push(values);
                    Ok(())
                })?;
                Ok(rows)
            })
            .expect("the table reads")
    }

    #[test]
    fn an_empty_table_reads_as_no_rows() {
        let sandbox = Sandbox::new("tables-empty");
        assert!(rows_of(&sandbox, "habits").is_empty());
    }

    #[test]
    fn a_sealed_column_travels_as_its_plaintext() {
        // The decision this module exists to make concrete, and the one worth a test that
        // asserts the uncomfortable half: what leaves the database is readable.
        let sandbox = Sandbox::new("tables-sealed");
        let codec = sandbox.codec();
        let device = DeviceId::generate().unwrap();

        sandbox
            .database()
            .with(|connection| {
                settings::put(
                    connection,
                    &codec,
                    device,
                    HLC,
                    NOW_US,
                    "backup.last_directory",
                    Some(b"una carpeta"),
                )?;
                Ok(())
            })
            .unwrap();

        let rows = rows_of(&sandbox, "settings");
        assert_eq!(rows.len(), 1);

        let value = rows[0].get("value").expect("the column travels");
        let encoded = value.as_str().expect("a sealed value travels as text");
        assert_eq!(
            crate::backup::base64::decode(encoded).unwrap(),
            b"una carpeta"
        );
    }

    #[test]
    fn a_row_written_into_another_database_comes_back_the_same() {
        // The end to end property of this file: a row read out of one vault and written
        // into another is the same row, even though every ciphertext in it is different,
        // because the sealing happens again under the second vault's key.
        let source = Sandbox::new("tables-source");
        let destination = Sandbox::new("tables-destination");
        let device = DeviceId::generate().unwrap();

        source
            .database()
            .with(|connection| {
                settings::put(
                    connection,
                    &source.codec(),
                    device,
                    HLC,
                    NOW_US,
                    "theme",
                    Some(b"ink"),
                )?;
                Ok(())
            })
            .unwrap();

        let rows = rows_of(&source, "settings");
        let spec = table_named("settings").unwrap();

        destination
            .database()
            .with(|connection| {
                for row in &rows {
                    write_row(connection, &destination.codec(), spec, row)?;
                }
                Ok(())
            })
            .unwrap();

        assert_eq!(rows_of(&destination, "settings"), rows);

        // And the repository, which knows nothing about backups, can read it.
        destination
            .database()
            .with(|connection| {
                let read = settings::get(connection, &destination.codec(), "theme")?
                    .expect("the setting was imported");
                assert_eq!(
                    read.value.as_deref().map(Vec::as_slice),
                    Some(b"ink".as_slice())
                );
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn two_sandboxes_are_two_different_vaults() {
        // Guards the test above against passing for the wrong reason. If two sandboxes
        // shared a key, re-sealing on the way in would be a no-op and the round trip would
        // prove nothing about the thing it exists to prove.
        let source = Sandbox::new("tables-keys-source");
        let destination = Sandbox::new("tables-keys-destination");

        assert_ne!(source.vault().key_id(), destination.vault().key_id());
    }

    #[test]
    fn the_same_value_stores_as_different_bytes_in_the_two_vaults() {
        // The other half of that guard, at the level that matters: the ciphertext in the
        // destination is not the ciphertext that was in the source, which is what "sealed
        // again" means.
        let source = Sandbox::new("tables-cipher-source");
        let destination = Sandbox::new("tables-cipher-destination");
        let device = DeviceId::generate().unwrap();

        let stored_value = |sandbox: &Sandbox| -> Vec<u8> {
            sandbox
                .database()
                .with(|connection| {
                    let bytes = connection.query_row(
                        "SELECT value FROM settings WHERE key = 'theme'",
                        [],
                        |row| row.get::<_, Vec<u8>>(0),
                    )?;
                    Ok(bytes)
                })
                .expect("the row is there")
        };

        source
            .database()
            .with(|connection| {
                settings::put(
                    connection,
                    &source.codec(),
                    device,
                    HLC,
                    NOW_US,
                    "theme",
                    Some(b"ink"),
                )?;
                Ok(())
            })
            .unwrap();

        let rows = rows_of(&source, "settings");
        let spec = table_named("settings").unwrap();
        destination
            .database()
            .with(|connection| {
                for row in &rows {
                    write_row(connection, &destination.codec(), spec, row)?;
                }
                Ok(())
            })
            .unwrap();

        assert_ne!(stored_value(&source), stored_value(&destination));
    }

    #[test]
    fn a_field_that_export_accepts_is_a_field_import_accepts() {
        // A regression, and a cheap one to have shipped. The limit was checked against the
        // raw bytes on the way out and against the base64 text on the way in, and base64 is
        // four characters for every three bytes: anything over three quarters of the limit
        // exported without complaint and refused to come back. It went unnoticed because
        // every test until this one used values of a few dozen bytes.
        let source = Sandbox::new("tables-field-band-source");
        let device = DeviceId::generate().unwrap();

        // Comfortably inside the limit, and comfortably past three quarters of it.
        let value = vec![0x5a_u8; MAX_FIELD_BYTES - 1];

        source
            .database()
            .with(|connection| {
                settings::put(
                    connection,
                    &source.codec(),
                    device,
                    HLC,
                    NOW_US,
                    "grande",
                    Some(&value),
                )?;
                Ok(())
            })
            .unwrap();

        let travelling = rows_of(&source, "settings").remove(0);

        let target = Sandbox::new("tables-field-band-target");
        let spec = table_named("settings").unwrap();
        target
            .database()
            .with(|connection| write_row(connection, &target.codec(), spec, &travelling))
            .expect("a field this same code exported has to import");

        let arrived = rows_of(&target, "settings").remove(0);
        assert_eq!(arrived.get("value"), travelling.get("value"));
    }

    #[test]
    fn a_row_with_a_column_missing_is_refused() {
        let sandbox = Sandbox::new("tables-missing-column");
        let spec = table_named("settings").unwrap();
        let mut values = RowValues::new();
        values.insert(
            "id".to_owned(),
            serde_json::json!("AAAAAAAAAAAAAAAAAAAAAA=="),
        );

        let refused = sandbox
            .database()
            .with(|connection| write_row(connection, &sandbox.codec(), spec, &values));

        assert!(matches!(refused, Err(DbError::Malformed)));
    }

    #[test]
    fn a_row_with_a_column_the_table_does_not_have_is_refused() {
        let sandbox = Sandbox::new("tables-extra-column");
        let spec = table_named("settings").unwrap();
        let source = Sandbox::new("tables-extra-source");
        let device = DeviceId::generate().unwrap();

        source
            .database()
            .with(|connection| {
                settings::put(
                    connection,
                    &source.codec(),
                    device,
                    HLC,
                    NOW_US,
                    "theme",
                    Some(b"ink"),
                )?;
                Ok(())
            })
            .unwrap();

        let mut values = rows_of(&source, "settings").remove(0);
        values.insert("invented".to_owned(), serde_json::json!(1));

        let refused = sandbox
            .database()
            .with(|connection| write_row(connection, &sandbox.codec(), spec, &values));

        assert!(matches!(refused, Err(DbError::Malformed)));
    }

    #[test]
    fn a_value_of_the_wrong_shape_is_refused() {
        let sandbox = Sandbox::new("tables-wrong-shape");
        let spec = table_named("settings").unwrap();

        for (column, wrong) in [
            ("rev", serde_json::json!("uno")),
            ("key", serde_json::json!(7)),
            ("id", serde_json::json!(7)),
            ("created_at", serde_json::json!(1.5)),
            ("value", serde_json::json!(["AA=="])),
        ] {
            let mut values = RowValues::new();
            for spec_column in spec.columns() {
                values.insert(spec_column.name.to_owned(), serde_json::Value::Null);
            }
            values.insert(column.to_owned(), wrong);

            let refused = sandbox
                .database()
                .with(|connection| write_row(connection, &sandbox.codec(), spec, &values));

            assert!(
                matches!(refused, Err(DbError::Malformed)),
                "{column} accepted a value of the wrong shape"
            );
        }
    }

    #[test]
    fn a_field_one_byte_over_the_limit_is_refused() {
        // The limit, broken by exactly one. A test that feeds it something enormous passes
        // for any limit at all.
        let sandbox = Sandbox::new("tables-field-limit");
        let spec = table_named("settings").unwrap();

        let mut values = RowValues::new();
        for column in spec.columns() {
            values.insert(column.name.to_owned(), serde_json::Value::Null);
        }
        values.insert(
            "key".to_owned(),
            serde_json::json!("k".repeat(MAX_FIELD_BYTES + 1)),
        );

        let refused = sandbox
            .database()
            .with(|connection| write_row(connection, &sandbox.codec(), spec, &values));

        assert!(matches!(
            refused,
            Err(DbError::TooMany {
                what: "bytes in one field",
                ..
            })
        ));
    }

    #[test]
    fn a_blob_that_is_not_base64_is_refused() {
        let sandbox = Sandbox::new("tables-bad-base64");
        let spec = table_named("settings").unwrap();

        let mut values = RowValues::new();
        for column in spec.columns() {
            values.insert(column.name.to_owned(), serde_json::Value::Null);
        }
        values.insert("id".to_owned(), serde_json::json!("no es base64!"));

        let refused = sandbox
            .database()
            .with(|connection| write_row(connection, &sandbox.codec(), spec, &values));

        assert!(matches!(refused, Err(DbError::Malformed)));
    }
}
