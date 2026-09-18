//! Reading a backup into a database of its own, beside the live one and never into it.
//!
//! The rule this module exists to keep is one sentence long: **not one byte from a file
//! somebody else could have written ever reaches the database this application is using.**
//!
//! The obvious way to import is to open a transaction on the live database, write the rows,
//! and roll back if anything is wrong. It is what the project's own storage rules say for
//! every other write, and here it is not good enough for two separate reasons. The first is
//! size: a backup may be half a gibibyte, and a transaction that large is a rollback journal
//! that large on a device that may not have the room. The second is worse — a transaction
//! protects the contents of the database, not the process. A parser that is handed a hostile
//! file and goes wrong inside a transaction has already gone wrong with the live key in
//! memory and the live file open.
//!
//! So an import builds a whole second database, `cairn.import.db`, in the same directory,
//! with this installation's schema and encrypted with this installation's own database key.
//! It is written from the first row to the last and checked from the first byte to the last
//! before anybody is asked anything. Only then does the caller get to decide whether it
//! becomes the real one, and that decision is a separate step in a separate module.
//!
//! Two things follow that are worth stating rather than discovering.
//!
//! The staging database is encrypted with the local key, not with anything from the file.
//! What arrives from the backup is content; the protection around it on this disk is this
//! machine's, so a staging file left behind by a crash is no more readable than the live one.
//!
//! Nothing here touches `cairn.device`. The identifier of this installation is a fact about
//! the machine, not about the data, and a restore that rewrote it would make this device
//! claim to be the one the backup came from.

use std::fs;
use std::path::{Path, PathBuf};

use cairn_crypto::UnlockedVault;
use rusqlite::Connection;

use crate::backup::schema::{TABLES, table_named};
use crate::backup::tables::write_row;
use crate::backup::verify::{VerifyReport, read_backup};
use crate::codec::FieldCodec;
use crate::error::DbError;
use crate::migrations;
use crate::open::Database;

/// What the staging database is called, in the same directory as the live one.
///
/// Beside the real database rather than in a system temporary directory, because the swap
/// that may follow is a rename, and a rename is only atomic within one filesystem. Visible
/// rather than hidden, because a file left behind by a crash should be something somebody can
/// find and delete rather than something that quietly occupies half a gibibyte.
pub const STAGING_FILE: &str = "cairn.import.db";

/// What an import prepared, and what the caller has to decide about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPrepared {
    /// Where the staging database is.
    pub staging: PathBuf,
    /// What reading the file found: sizes, versions and the count per table.
    pub report: VerifyReport,
    /// Whether the live database currently holds anything at all.
    ///
    /// The one thing the caller needs in order to phrase the question correctly. Restoring
    /// over an empty vault and restoring over a full one are the same operation and a very
    /// different sentence.
    pub has_existing_data: bool,
}

/// Reads a backup into a staging database beside the live one, and verifies it on the way.
///
/// Leaves the live database completely alone. On any failure the staging database is removed,
/// so a refused import leaves the directory exactly as it found it.
///
/// `progress` is called with the number of bytes of the file read so far.
///
/// # Errors
///
/// The four a reader may say about a file — [`DbError::NotABackup`],
/// [`DbError::UnsupportedVersion`], [`DbError::WrongPassword`] and [`DbError::Malformed`] —
/// plus [`DbError::Io`] if the disk refuses, [`DbError::TooMany`] if the file or a table is
/// larger than one may be, and [`DbError::Sqlite`] if a statement fails.
pub fn prepare_import(
    directory: &Path,
    live: &Connection,
    vault: &UnlockedVault,
    password: &str,
    backup: &Path,
    now_us: i64,
    progress: &mut dyn FnMut(u64),
) -> Result<ImportPrepared, DbError> {
    let staging = directory.join(STAGING_FILE);

    // Before anything is opened. A staging database left behind by a crash is from a run
    // nobody finished, and carrying on into it would mix two imports.
    discard(&staging)?;

    let outcome = build(&staging, live, vault, password, backup, now_us, progress);

    if outcome.is_err() {
        // Best effort, and deliberately not reported. The import has already failed for a
        // reason the caller is about to be told, and a second message about the leftovers
        // would replace the one that says what actually went wrong.
        discard(&staging).ok();
    }

    outcome
}

/// Removes a staging database and the two files SQLite keeps beside it.
///
/// # Errors
///
/// Returns [`DbError::Io`] if one of them is there and cannot be removed, which usually means
/// another copy of this application still has it open.
pub fn discard(staging: &Path) -> Result<(), DbError> {
    for suffix in ["", "-wal", "-shm"] {
        let mut path = staging.as_os_str().to_owned();
        path.push(suffix);
        let path = PathBuf::from(path);

        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {}
            Err(cause) => {
                return Err(DbError::Io {
                    what: "the staging database",
                    operation: "removed",
                    cause,
                });
            }
        }
    }

    Ok(())
}

/// The work, with the cleanup left to the caller above.
fn build(
    staging: &Path,
    live: &Connection,
    vault: &UnlockedVault,
    password: &str,
    backup: &Path,
    now_us: i64,
    progress: &mut dyn FnMut(u64),
) -> Result<ImportPrepared, DbError> {
    let database = Database::open(staging, &vault.database_key())?;
    migrations::apply_all(&database, now_us)?;

    let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

    let report = database.in_transaction(|transaction| {
        read_backup(backup, password, progress, |table, values| {
            // The reader has already refused a table this build does not know, so this is
            // belt and braces rather than the check. It is here because the alternative to a
            // second lookup is a row silently going nowhere.
            let spec = table_named(table).ok_or(DbError::Malformed)?;

            write_row(transaction, &codec, spec, &values)
        })
    })?;

    let has_existing_data = holds_anything(live)?;

    // Closed before the caller is told anything. The next step is a rename over the live
    // file, and on Windows a rename over a file somebody still has open does not happen.
    database.close()?;

    Ok(ImportPrepared {
        staging: staging.to_path_buf(),
        report,
        has_existing_data,
    })
}

/// Whether a database holds a single row that is not a tombstone.
///
/// Asked per table with `EXISTS` rather than counted, so it stops at the first row it finds
/// instead of walking a vault somebody has been using for years to answer a yes or no.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if a statement fails.
fn holds_anything(connection: &Connection) -> Result<bool, DbError> {
    for table in TABLES {
        let statement = format!(
            "SELECT EXISTS(SELECT 1 FROM {} WHERE deleted = 0)",
            table.name
        );
        let found: i64 = connection
            .prepare_cached(&statement)?
            .query_row([], |row| row.get(0))?;

        if found != 0 {
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ImportPrepared, STAGING_FILE, discard, prepare_import};
    use crate::backup::export::write_backup;
    use crate::backup::format::RowValues;
    use crate::backup::schema::table_named;
    use crate::backup::tables::{read_table, write_row};
    use crate::error::DbError;
    use crate::migrations::LATEST_VERSION;
    use crate::test_support::Sandbox;
    use cairn_crypto::{Argon2Params, MIN_MEMORY_KIB, MIN_PASSES};

    /// The password every backup in this module's tests is sealed with.
    const PASSWORD: &str = "una frase larga para la prueba de importacion";

    /// The cheapest parameters that are still a real Argon2id run.
    fn params() -> Argon2Params {
        Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, 1).expect("the floor is accepted")
    }

    /// Writes one habit area into a database, which is enough to tell two vaults apart.
    fn seed(sandbox: &Sandbox, name: &str, id: u8) {
        let spec = table_named("habit_areas").expect("the table is carried");
        let codec = sandbox.codec();
        let stamped = 1_700_000_000_000_000_i64;

        let mut values = RowValues::new();
        values.insert(
            "id".to_owned(),
            serde_json::Value::from(crate::backup::base64::encode(&[id; 16])),
        );
        values.insert("created_at".to_owned(), serde_json::Value::from(stamped));
        values.insert("updated_at".to_owned(), serde_json::Value::from(stamped));
        values.insert(
            "device_id".to_owned(),
            serde_json::Value::from(crate::backup::base64::encode(&[0xde; 16])),
        );
        values.insert("deleted".to_owned(), serde_json::Value::from(0));
        values.insert(
            "hlc".to_owned(),
            serde_json::Value::from(crate::backup::base64::encode(&[0xc1; 16])),
        );
        values.insert("rev".to_owned(), serde_json::Value::from(1));
        values.insert("name".to_owned(), serde_json::Value::from(name));
        values.insert("color".to_owned(), serde_json::Value::Null);
        values.insert("position".to_owned(), serde_json::Value::from(0));

        sandbox
            .database()
            .with(|connection| write_row(connection, &codec, spec, &values))
            .expect("the row is written");
    }

    /// Writes enough incompressible content that the export spans several chunks.
    ///
    /// Needed by the two tests about damage. A tiny backup is a single chunk, and a single
    /// chunk that fails its tag is reported as a wrong password, because a file whose first
    /// chunk never opened has told the reader nothing that distinguishes the two. Damage that
    /// is reported as damage is damage to a chunk that is not the first one, so these tests
    /// have to produce a file with more than one.
    ///
    /// Pseudo-random rather than repeated, because zstd turns a megabyte of the same byte
    /// into a few hundred of them and the file would be one chunk again.
    fn seed_large(sandbox: &Sandbox) {
        let spec = table_named("settings").expect("the table is carried");
        let codec = sandbox.codec();
        let stamped = 1_700_000_000_000_000_i64;

        sandbox
            .database()
            .with(|connection| {
                for row in 0..8_u32 {
                    // Forty kibibytes, because the limit is on the encoded field and base64 is four
                    // characters for every three bytes: sixty would be eighty encoded, over the
                    // sixty-four the format allows.
                    let mut noise = vec![0_u8; 40 * 1024];
                    let mut state = u32::from(u16::try_from(row).unwrap_or(0)).wrapping_add(1);
                    for byte in &mut noise {
                        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        *byte = u8::try_from(state >> 24).unwrap_or(0);
                    }

                    let mut values = RowValues::new();
                    // Row zero's identifier is sixteen zero bytes, and it stays that way. That
                    // is the row the paging cursor used to skip, and having it here means the
                    // whole import path walks over the case rather than only the unit test in
                    // `tables.rs` that names it.
                    values.insert(
                        "id".to_owned(),
                        serde_json::Value::from(crate::backup::base64::encode(
                            &[u8::try_from(row).unwrap_or(0); 16],
                        )),
                    );
                    values.insert("created_at".to_owned(), serde_json::Value::from(stamped));
                    values.insert("updated_at".to_owned(), serde_json::Value::from(stamped));
                    values.insert(
                        "device_id".to_owned(),
                        serde_json::Value::from(crate::backup::base64::encode(&[0xde; 16])),
                    );
                    values.insert("deleted".to_owned(), serde_json::Value::from(0));
                    values.insert(
                        "hlc".to_owned(),
                        serde_json::Value::from(crate::backup::base64::encode(&[0xc1; 16])),
                    );
                    values.insert("rev".to_owned(), serde_json::Value::from(1));
                    values.insert(
                        "key".to_owned(),
                        serde_json::Value::from(format!("ruido-{row}")),
                    );
                    values.insert(
                        "value".to_owned(),
                        serde_json::Value::from(crate::backup::base64::encode(&noise)),
                    );

                    write_row(connection, &codec, spec, &values)?;
                }

                Ok(())
            })
            .expect("the rows are written");
    }

    /// Exports a sandbox to a file beside it and answers where it went.
    fn export(sandbox: &Sandbox) -> std::path::PathBuf {
        let destination = sandbox.directory().join("copia.cairn");
        let codec = sandbox.codec();

        sandbox
            .database()
            .with(|connection| {
                write_backup(
                    connection,
                    &codec,
                    LATEST_VERSION,
                    PASSWORD,
                    params(),
                    &destination,
                    &mut |_done| {},
                )
            })
            .expect("the export works");

        destination
    }

    /// Reads back the names in `habit_areas`, which is what the round trip is checked on.
    fn area_names(sandbox: &Sandbox) -> Vec<String> {
        let spec = table_named("habit_areas").expect("the table is carried");
        let codec = sandbox.codec();
        let mut names = Vec::new();

        sandbox
            .database()
            .with(|connection| {
                read_table(connection, &codec, spec, |values| {
                    if let Some(name) = values.get("name").and_then(serde_json::Value::as_str) {
                        names.push(name.to_owned());
                    }
                    Ok(())
                })
                .map(|_counted| ())
            })
            .expect("the read works");

        names.sort();
        names
    }

    fn prepare(sandbox: &Sandbox, file: &std::path::Path) -> Result<ImportPrepared, DbError> {
        sandbox.database().with(|connection| {
            prepare_import(
                sandbox.directory(),
                connection,
                sandbox.vault(),
                PASSWORD,
                file,
                1_700_000_000_000_000,
                &mut |_read| {},
            )
        })
    }

    #[test]
    fn a_backup_reads_into_a_staging_database_and_the_live_one_is_untouched() {
        let source = Sandbox::new("import-source");
        seed(&source, "Salud", 0x11);
        let file = export(&source);

        let target = Sandbox::new("import-target");
        seed(&target, "Lo que ya habia", 0x22);

        let prepared = prepare(&target, &file).expect("the import prepares");

        assert!(prepared.has_existing_data);
        assert!(prepared.staging.exists(), "no staging database was written");
        assert_eq!(
            prepared.staging.file_name().and_then(|n| n.to_str()),
            Some(STAGING_FILE)
        );
        // The whole point. Preparing an import changes nothing about the live database.
        assert_eq!(area_names(&target), vec!["Lo que ya habia".to_owned()]);
    }

    #[test]
    fn an_empty_vault_is_reported_as_empty_so_the_question_can_be_phrased_right() {
        let source = Sandbox::new("import-empty-source");
        seed(&source, "Salud", 0x11);
        let file = export(&source);

        let target = Sandbox::new("import-empty-target");
        let prepared = prepare(&target, &file).expect("the import prepares");

        assert!(!prepared.has_existing_data);
    }

    #[test]
    fn the_wrong_password_leaves_no_staging_database_behind() {
        let source = Sandbox::new("import-wrong-source");
        seed(&source, "Salud", 0x11);
        let file = export(&source);

        let target = Sandbox::new("import-wrong-target");
        let refused = target
            .database()
            .with(|connection| {
                prepare_import(
                    target.directory(),
                    connection,
                    target.vault(),
                    "not the password",
                    &file,
                    1_700_000_000_000_000,
                    &mut |_read| {},
                )
            })
            .expect_err("the wrong password is refused");

        assert!(matches!(refused, DbError::WrongPassword), "{refused:?}");
        assert!(
            !target.directory().join(STAGING_FILE).exists(),
            "a refused import left a staging database behind"
        );
    }

    #[test]
    fn a_file_that_is_not_a_backup_leaves_no_staging_database_behind() {
        let target = Sandbox::new("import-garbage");
        let file = target.directory().join("no-es-una-copia.cairn");
        fs::write(&file, vec![0x7f; 4096]).expect("the file can be written");

        let refused = prepare(&target, &file).expect_err("rubbish is refused");

        assert!(matches!(refused, DbError::NotABackup), "{refused:?}");
        assert!(!target.directory().join(STAGING_FILE).exists());
    }

    #[test]
    fn a_truncated_backup_leaves_no_staging_database_behind() {
        let source = Sandbox::new("import-cut-source");
        seed(&source, "Salud", 0x11);
        seed_large(&source);
        let file = export(&source);

        let whole = fs::read(&file).expect("the backup is readable");
        let cut = whole.len().saturating_sub(64);
        fs::write(&file, &whole[..cut]).expect("the file can be shortened");

        let target = Sandbox::new("import-cut-target");
        let refused = prepare(&target, &file).expect_err("a truncated file is refused");

        assert!(matches!(refused, DbError::Malformed), "{refused:?}");
        assert!(!target.directory().join(STAGING_FILE).exists());
    }

    #[test]
    fn a_backup_with_one_byte_changed_leaves_no_staging_database_behind() {
        let source = Sandbox::new("import-bit-source");
        seed(&source, "Salud", 0x11);
        seed_large(&source);
        let file = export(&source);

        let mut whole = fs::read(&file).expect("the backup is readable");
        // Well past the first chunk, so the reader has already proved the password and the
        // only honest thing left to say is that the file is damaged.
        let late = whole.len().saturating_sub(1_024);
        if let Some(byte) = whole.get_mut(late) {
            *byte ^= 0x01;
        }
        fs::write(&file, &whole).expect("the file can be rewritten");

        let target = Sandbox::new("import-bit-target");
        let refused = prepare(&target, &file).expect_err("an edited file is refused");

        assert!(matches!(refused, DbError::Malformed), "{refused:?}");
        assert!(!target.directory().join(STAGING_FILE).exists());
    }

    #[test]
    fn damage_to_the_very_first_chunk_is_reported_as_a_wrong_password() {
        // Not a defect, and written down so it is not mistaken for one. A file whose first
        // chunk never opened has said nothing that tells a wrong password apart from an
        // edited byte, and a reader that guessed between them would be guessing. It only
        // shows on a backup small enough to be a single chunk; a real one is many.
        let source = Sandbox::new("import-first-chunk-source");
        seed(&source, "Salud", 0x11);
        let file = export(&source);

        let mut whole = fs::read(&file).expect("the backup is readable");
        let middle = whole.len().div_euclid(2);
        if let Some(byte) = whole.get_mut(middle) {
            *byte ^= 0x01;
        }
        fs::write(&file, &whole).expect("the file can be rewritten");

        let target = Sandbox::new("import-first-chunk-target");
        let refused = prepare(&target, &file).expect_err("an edited file is refused");

        assert!(matches!(refused, DbError::WrongPassword), "{refused:?}");
        assert!(!target.directory().join(STAGING_FILE).exists());
    }

    #[test]
    fn a_backup_from_another_vault_is_refused_as_a_wrong_password() {
        // There is nothing in the file that says which vault it came from, on purpose: that
        // would be a fact about its owner available to anybody who finds it. The cost is this
        // message, and it is written into the user documentation so it does not look like a
        // fault.
        let other = Sandbox::new("import-other-vault");
        seed(&other, "De otro sitio", 0x33);

        let destination = other.directory().join("de-otro.cairn");
        let codec = other.codec();
        other
            .database()
            .with(|connection| {
                write_backup(
                    connection,
                    &codec,
                    LATEST_VERSION,
                    "la contrasena de la otra caja fuerte",
                    params(),
                    &destination,
                    &mut |_done| {},
                )
            })
            .expect("the other export works");

        let target = Sandbox::new("import-other-target");
        let refused = prepare(&target, &destination).expect_err("another vault is refused");

        assert!(matches!(refused, DbError::WrongPassword), "{refused:?}");
    }

    #[test]
    fn a_staging_database_left_by_an_earlier_run_is_removed_rather_than_carried_on_into() {
        let source = Sandbox::new("import-leftover-source");
        seed(&source, "Salud", 0x11);
        let file = export(&source);

        let target = Sandbox::new("import-leftover-target");
        let leftover = target.directory().join(STAGING_FILE);
        fs::write(&leftover, b"from a run nobody finished").expect("the file can be written");

        prepare(&target, &file).expect("the import prepares over the leftover");

        // It is a database now, rather than the rubbish that was there.
        let bytes = fs::read(&leftover).expect("the staging database is readable");
        assert_ne!(bytes.as_slice(), b"from a run nobody finished");
    }

    #[test]
    fn discarding_a_staging_database_that_is_not_there_is_not_an_error() {
        let target = Sandbox::new("import-discard");

        discard(&target.directory().join(STAGING_FILE)).expect("discarding nothing is fine");
    }
}
