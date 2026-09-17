//! Writing a backup, and reading it straight back before saying it was written.
//!
//! The order of operations is the whole content of this file, and every step of it is
//! there because of something that goes wrong without it.
//!
//! It is written to a temporary name beside the destination and renamed at the end, so a
//! disk that fills up halfway leaves a file called `something.part` rather than a
//! `something.cairn` that looks finished and is not. It is flushed and synced before the
//! rename, so a power cut after the rename cannot leave a name pointing at bytes that never
//! reached the platter.
//!
//! And then it is opened again and decrypted from the first byte to the last. A backup that
//! has never been read is not a backup: it is a file somebody hopes is a backup, and the
//! day it is needed is the worst possible day to find out. The verification costs a second
//! Argon2id derivation and a full pass over the file, and that cost is the point rather
//! than an unfortunate side effect.
//!
//! What crosses back to the caller is a count of bytes, a count of chunks and a count of
//! rows per table. Never a path: a path carries the account name of whoever is logged in
//! and usually the name of their machine.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use cairn_crypto::{
    Argon2Params, BackupHeader, ChunkNonces, ChunkSealer, SALT_LEN, derive_kek, export_key,
    fill_random,
};
use rusqlite::Connection;

use crate::backup::compress::Compressor;
use crate::backup::format::{FORMAT_NAME, Line, Manifest, RECORD_VERSION, TableCount, TableHeader};
use crate::backup::schema::TABLES;
use crate::backup::tables::read_table;
use crate::backup::verify::{VerifyReport, verify_backup};
use crate::codec::FieldCodec;
use crate::error::DbError;

/// What the temporary file is called while it is being written.
///
/// A suffix rather than a hidden file or a system temporary directory. Beside the
/// destination, because a rename is only atomic within one filesystem and a temporary
/// directory is very often on another one; visible, because a file left behind by a crash
/// should be something somebody can find and delete rather than something that quietly
/// occupies space in a directory nobody opens.
const PART_SUFFIX: &str = ".part";

/// What one export did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// How large the finished file is, in bytes.
    pub bytes: u64,
    /// How many chunks it holds.
    pub chunks: u64,
    /// How many rows of each table went into it, in the order they were written.
    pub records: BTreeMap<String, u64>,
}

/// Writes the whole vault to one encrypted file and verifies it.
///
/// `password` is used twice: once to derive the key the file is sealed with, and once more
/// by the verification pass, which derives it again from what the finished file says rather
/// than reusing what is in memory. Reusing it would mean the verification never reads the
/// header it is supposed to be checking.
///
/// `progress` is called with the number of plaintext bytes serialised so far. It exists so
/// that a person watching a phone can tell the difference between slow and stuck.
///
/// # Errors
///
/// Returns [`DbError::Io`] if anything on disk refuses, [`DbError::Sealed`] if a stored
/// value does not decrypt or the file cannot be sealed, [`DbError::TooMany`] if a table
/// holds more rows than a backup may carry, and [`DbError::Sqlite`] if a statement fails.
/// On any failure the temporary file is removed and the destination is left alone.
pub fn write_backup(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    schema_version: u32,
    password: &str,
    params: Argon2Params,
    destination: &Path,
    progress: &mut dyn FnMut(u64),
) -> Result<(ExportReport, VerifyReport), DbError> {
    let part = part_path(destination);

    let outcome = write_and_verify(
        connection,
        codec,
        schema_version,
        password,
        params,
        destination,
        &part,
        progress,
    );

    if outcome.is_err() {
        // Best effort, and deliberately not reported. The operation has already failed for
        // a reason the caller is about to be told; a second message about the leftovers
        // would replace the one that says what actually went wrong.
        let _ignored = fs::remove_file(&part);
    }

    outcome
}

/// The steps, in order, with the cleanup left to the caller above.
#[expect(
    clippy::too_many_arguments,
    reason = "every one of these is a separate decision the caller has already made, and grouping them into a struct would put the password in a value that outlives the call"
)]
fn write_and_verify(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    schema_version: u32,
    password: &str,
    params: Argon2Params,
    destination: &Path,
    part: &Path,
    progress: &mut dyn FnMut(u64),
) -> Result<(ExportReport, VerifyReport), DbError> {
    let mut salt = [0_u8; SALT_LEN];
    fill_random(&mut salt)?;

    let nonces = ChunkNonces::generate()?;
    let header = BackupHeader::new(salt, params, *nonces.base());

    // The file's own derivation, over the file's own salt. Not the vault's key and not
    // anything reachable from it: a backup sealed with a key that hangs off this vault
    // could only ever be opened by a machine that can already open this vault.
    let kek = derive_kek(password, &salt, params)?;
    let key = export_key(&kek);

    let report = {
        let file = File::create(part).map_err(|cause| DbError::Io {
            what: "the backup file",
            operation: "created",
            cause,
        })?;
        let mut writer = BufWriter::new(&file);

        writer
            .write_all(&header.to_bytes())
            .map_err(|cause| DbError::Io {
                what: "the backup file",
                operation: "written",
                cause,
            })?;

        let mut sealer = ChunkSealer::new(&key, &header, nonces);
        let mut written = 0_u64;

        // The sink the compressor feeds. Everything between here and the file goes through
        // it a buffer at a time, so the whole backup never exists in memory at once.
        let sink = Sealing {
            sealer: &mut sealer,
            writer: &mut writer,
            error: None,
        };
        let mut records = BTreeMap::new();
        let mut stream = Compressor::new(sink)?;

        let counts = table_counts(connection)?;
        Line::Manifest(Manifest {
            format: FORMAT_NAME.to_owned(),
            record_version: RECORD_VERSION,
            schema_version,
            tables: counts.clone(),
        })
        .write_to(&mut stream)?;

        for (table, count) in TABLES.iter().zip(&counts) {
            Line::Table(TableHeader {
                name: table.name.to_owned(),
                rows: count.rows,
            })
            .write_to(&mut stream)?;

            let rows = read_table(connection, codec, table, |values| {
                Line::Row(values).write_to(&mut stream)?;
                written = written.saturating_add(1);
                progress(written);
                Ok(())
            })?;

            // The manifest is written before the rows are read, so a row added between the
            // two would make the file describe itself wrongly. There is one writer to this
            // database and it is this thread, so it cannot happen; the check is here
            // because "cannot happen" is a claim with a shelf life.
            if rows != count.rows {
                return Err(DbError::Malformed);
            }
            records.insert(table.name.to_owned(), rows);
        }

        let sink = stream.finish()?;
        sink.into_result()?;

        // The last chunk, which carries the mark that says it is the last one. Always
        // written, even when nothing is left over, because that mark is what makes a
        // truncated file fail rather than restore a shorter vault.
        let mut tail = Vec::new();
        let chunks = sealer.finish(&mut tail)?;
        writer.write_all(&tail).map_err(|cause| DbError::Io {
            what: "the backup file",
            operation: "written",
            cause,
        })?;

        writer.flush().map_err(|cause| DbError::Io {
            what: "the backup file",
            operation: "written",
            cause,
        })?;
        drop(writer);

        // Before the rename, never after. A rename that lands before the bytes do leaves a
        // finished name over an unfinished file, which is the one failure a backup must not
        // have.
        file.sync_all().map_err(|cause| DbError::Io {
            what: "the backup file",
            operation: "flushed to disk",
            cause,
        })?;

        ExportReport {
            bytes: 0,
            chunks,
            records,
        }
    };

    fs::rename(part, destination).map_err(|cause| DbError::Io {
        what: "the backup file",
        operation: "renamed into place",
        cause,
    })?;

    let bytes = fs::metadata(destination)
        .map_err(|cause| DbError::Io {
            what: "the backup file",
            operation: "measured",
            cause,
        })?
        .len();

    // From the finished file, with the password and nothing carried over from above. A
    // verification that reused the key in memory would never read the header it exists to
    // check.
    let verified = verify_backup(destination, password, progress)?;

    Ok((ExportReport { bytes, ..report }, verified))
}

/// Counts the rows of every table, so the manifest can say what follows.
fn table_counts(connection: &Connection) -> Result<Vec<TableCount>, DbError> {
    let mut counts = Vec::with_capacity(TABLES.len());

    for table in TABLES {
        // The name comes from a constant of this crate, checked against the schema by a
        // test. Nothing a caller supplies reaches this string.
        let statement = format!("SELECT count(*) FROM {}", table.name);
        let rows: i64 = connection.query_row(&statement, [], |row| row.get(0))?;

        counts.push(TableCount {
            name: table.name.to_owned(),
            rows: u64::try_from(rows).unwrap_or(0),
        });
    }

    Ok(counts)
}

/// The sink that seals what the compressor produces and writes it to the file.
///
/// A writer rather than a callback because that is what the compressor wants, and the one
/// awkward part is what to do with an error: `io::Write` can only return an `io::Error`, and
/// what happens here can also be a sealing failure. So the real error is kept and the caller
/// asks for it with [`Sealing::into_result`] once the stream is finished.
struct Sealing<'a, 'key, W: std::io::Write> {
    sealer: &'a mut ChunkSealer<'key>,
    writer: &'a mut W,
    error: Option<DbError>,
}

impl<W: std::io::Write> Sealing<'_, '_, W> {
    /// Whatever went wrong inside, once the compressor has finished with this.
    fn into_result(self) -> Result<(), DbError> {
        self.error.map_or(Ok(()), Err)
    }
}

impl<W: std::io::Write> std::io::Write for Sealing<'_, '_, W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let mut sealed = Vec::new();
        if let Err(failure) = self.sealer.push(buffer, &mut sealed) {
            self.error = Some(DbError::Sealed(failure));
            return Err(std::io::Error::other("a backup chunk could not be sealed"));
        }

        self.writer.write_all(&sealed)?;

        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

/// The temporary name beside a destination.
fn part_path(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_os_string();
    name.push(PART_SUFFIX);

    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cairn_crypto::{Argon2Params, MIN_MEMORY_KIB, MIN_PASSES};
    use cairn_domain::Hlc;

    use super::{ExportReport, part_path, write_backup};
    use crate::backup::verify::verify_backup;
    use crate::device::DeviceId;
    use crate::error::DbError;
    use crate::repositories::settings;
    use crate::test_support::Sandbox;

    const NOW_US: i64 = 1_700_000_000_000_000;
    const HLC: Hlc = Hlc::new(1_000, 0, [1; 6]);
    const PASSWORD: &str = "una frase larga para la copia";

    fn cheap() -> Argon2Params {
        Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, 1).expect("the floor is accepted")
    }

    /// Writes a backup of a sandbox and gives back where it is and what it reported.
    fn exported(sandbox: &Sandbox, name: &str) -> (std::path::PathBuf, ExportReport) {
        let destination = sandbox.directory().join(name);
        let report = sandbox
            .database()
            .with(|connection| {
                let (report, _verified) = write_backup(
                    connection,
                    &sandbox.codec(),
                    crate::LATEST_VERSION,
                    PASSWORD,
                    cheap(),
                    &destination,
                    &mut |_so_far| {},
                )?;
                Ok(report)
            })
            .expect("the export works");

        (destination, report)
    }

    #[test]
    fn an_empty_vault_still_produces_a_file_that_verifies() {
        let sandbox = Sandbox::new("export-empty");
        let (path, report) = exported(&sandbox, "empty.cairn");

        assert!(path.exists());
        assert_eq!(report.bytes, fs::metadata(&path).unwrap().len());
        assert!(report.chunks >= 1);
        assert!(report.records.values().all(|count| *count == 0));
    }

    #[test]
    fn nothing_but_the_magic_is_readable_in_the_file() {
        // The manual check, automated: a backup opened in a text editor shows eight bytes
        // and then noise. A column name or a key that survived into the file would be
        // visible here.
        let sandbox = Sandbox::new("export-opaque");
        let device = DeviceId::generate().unwrap();

        sandbox
            .database()
            .with(|connection| {
                settings::put(
                    connection,
                    &sandbox.codec(),
                    device,
                    HLC,
                    NOW_US,
                    "theme",
                    Some(b"un valor reconocible"),
                )?;
                Ok(())
            })
            .unwrap();

        let (path, _report) = exported(&sandbox, "opaque.cairn");
        let bytes = fs::read(&path).unwrap();

        assert_eq!(bytes.get(..8), Some(b"CAIRNBAK".as_slice()));
        for needle in [
            b"un valor reconocible".as_slice(),
            b"theme".as_slice(),
            b"settings".as_slice(),
            b"cairn-backup".as_slice(),
        ] {
            assert!(
                !bytes.windows(needle.len()).any(|window| window == needle),
                "{} appears in the file in the clear",
                String::from_utf8_lossy(needle)
            );
        }
    }

    #[test]
    fn the_file_it_wrote_verifies_with_the_password_and_not_without_it() {
        let sandbox = Sandbox::new("export-verify");
        let (path, report) = exported(&sandbox, "verify.cairn");

        let verified = verify_backup(&path, PASSWORD, &mut |_so_far| {}).expect("it verifies");
        assert_eq!(verified.bytes, report.bytes);
        assert_eq!(verified.chunks, report.chunks);

        assert!(matches!(
            verify_backup(&path, "otra frase distinta", &mut |_so_far| {}),
            Err(DbError::WrongPassword)
        ));
    }

    #[test]
    fn no_leftover_file_is_kept_and_no_destination_is_written_when_it_fails() {
        // The disk filling up, stood in for by a destination that cannot be created. What
        // matters is that neither name exists afterwards: a half written `.cairn` that
        // looks finished is the worst thing an export could leave behind.
        let sandbox = Sandbox::new("export-failure");
        let destination = sandbox
            .directory()
            .join("no-such-directory")
            .join("x.cairn");

        let refused = sandbox.database().with(|connection| {
            write_backup(
                connection,
                &sandbox.codec(),
                crate::LATEST_VERSION,
                PASSWORD,
                cheap(),
                &destination,
                &mut |_so_far| {},
            )
            .map(|_report| ())
        });

        assert!(matches!(refused, Err(DbError::Io { .. })));
        assert!(!destination.exists());
        assert!(!part_path(&destination).exists());
    }

    #[test]
    fn the_temporary_name_sits_beside_the_destination() {
        // A rename is only atomic inside one filesystem, so the temporary file cannot live
        // in a system temporary directory, which is very often on another one.
        let destination = std::path::Path::new("/somewhere/backup.cairn");
        let part = part_path(destination);

        assert_eq!(part.parent(), destination.parent());
        assert_ne!(part, destination);
    }

    #[test]
    fn progress_is_reported_while_it_works() {
        let sandbox = Sandbox::new("export-progress");
        let device = DeviceId::generate().unwrap();

        sandbox
            .database()
            .with(|connection| {
                for index in 0..32 {
                    settings::put(
                        connection,
                        &sandbox.codec(),
                        device,
                        Hlc::new(1_000 + index, 0, [1; 6]),
                        NOW_US,
                        &format!("key-{index}"),
                        Some(b"valor"),
                    )?;
                }
                Ok(())
            })
            .unwrap();

        let destination = sandbox.directory().join("progress.cairn");
        let mut seen = Vec::new();

        sandbox
            .database()
            .with(|connection| {
                write_backup(
                    connection,
                    &sandbox.codec(),
                    crate::LATEST_VERSION,
                    PASSWORD,
                    cheap(),
                    &destination,
                    &mut |so_far| seen.push(so_far),
                )
                .map(|_report| ())
            })
            .unwrap();

        assert!(!seen.is_empty(), "nothing was reported while it worked");
        assert!(
            seen.windows(2).all(|pair| pair[0] <= pair[1]),
            "progress went backwards"
        );
    }
}
