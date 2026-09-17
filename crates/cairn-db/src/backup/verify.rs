//! Reading a backup from the first byte to the last without writing anything.
//!
//! Two callers, one path. The export calls it on the file it has just written, because a
//! backup that has never been read is not a backup; and a person calls it on a file from
//! two years ago, because that is when they want to find out, not on the day they need it.
//! The import calls the same function with a sink that writes rows instead of dropping
//! them, and that sharing is not a saving of a hundred lines: it is the guarantee that what
//! an import accepts and what a verification accepts are the same thing.
//!
//! Nothing is held. The file is read a buffer at a time into a reader that decrypts chunk
//! by chunk, the decompressor pulls from that reader, and the lines come out of the
//! decompressor one at a time. At no point does the whole file, or the whole decompressed
//! body, exist in memory. That is what decides whether a half gibibyte backup can be opened
//! on a phone, and it is easy to lose by accident: collecting the decrypted bytes into a
//! buffer before decompressing them would pass every test in this file and use half a
//! gibibyte of memory doing it.
//!
//! The four things it may say are fixed, because a fifth would be an oracle for somebody
//! editing the file: this is not a Cairn backup, this version cannot be read, the password
//! is wrong, and the file is damaged or incomplete. Never how far it got, never which check
//! failed.
//!
//! One distinction is worth stating because it is the only one that could look like a leak.
//! A first chunk that does not open is reported as a wrong password; a later one is
//! reported as damage. That is honest rather than clever: the key is either right from the
//! first byte or wrong for the whole file, so a failure on chunk zero means the key, and a
//! failure after chunk zero has already proved the key right. It tells somebody holding the
//! file nothing they could not work out by deriving the key themselves, which they can, at
//! their leisure, because they are holding the file.

use std::fs::File;
use std::io::{self, BufReader, Read as _};
use std::path::Path;

use cairn_crypto::{
    BACKUP_HEADER_LEN, BackupHeader, ChunkOpener, CryptoError, TAG_LEN, derive_kek, export_key,
};
use zeroize::Zeroizing;

use crate::backup::compress::decompress_bounded;
use crate::backup::format::{
    FORMAT_NAME, Line, LineSplitter, MAX_FILE_BYTES, MAX_ROWS_PER_TABLE, RECORD_VERSION, RowValues,
};
use crate::backup::schema::table_named;
use crate::error::DbError;

/// How much of the file is read at a time, in bytes.
const READ_BUFFER_LEN: usize = 64 * 1024;

/// What one verification found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// How large the file is, in bytes.
    pub bytes: u64,
    /// How many chunks it holds.
    pub chunks: u64,
    /// Which version of the file format it is in.
    pub format_version: u16,
    /// Which version of the record layout it carries.
    pub record_version: u16,
    /// Which schema version the rows were read out of.
    pub schema_version: u32,
    /// How many rows of each table it carries, in the order they appear.
    pub records: Vec<(String, u64)>,
}

/// Reads a backup end to end and reports what it found, writing nothing.
///
/// # Errors
///
/// Says four things about the file and never more: [`DbError::NotABackup`],
/// [`DbError::UnsupportedVersion`], [`DbError::WrongPassword`] and [`DbError::Malformed`].
/// [`DbError::Io`] and [`DbError::TooMany`] can also come back, and both are about this
/// machine rather than about the file: a disk that will not read, and a file larger than
/// this will open at all.
pub fn verify_backup(
    path: &Path,
    password: &str,
    progress: &mut dyn FnMut(u64),
) -> Result<VerifyReport, DbError> {
    read_backup(path, password, progress, |_table, _values| Ok(()))
}

/// Reads a backup end to end, handing every row to a sink as it is read.
///
/// # Errors
///
/// The same as [`verify_backup`], plus whatever the sink returns.
pub fn read_backup(
    path: &Path,
    password: &str,
    progress: &mut dyn FnMut(u64),
    mut sink: impl FnMut(&str, RowValues) -> Result<(), DbError>,
) -> Result<VerifyReport, DbError> {
    let bytes = std::fs::metadata(path)
        .map_err(|cause| DbError::Io {
            what: "the backup file",
            operation: "opened",
            cause,
        })?
        .len();

    // Before a byte is read and before anything is reserved. A ceiling checked afterwards
    // is a ceiling that has already cost what it was there to save.
    if bytes > MAX_FILE_BYTES {
        return Err(DbError::TooMany {
            what: "bytes in the backup file",
            value: bytes,
            max: MAX_FILE_BYTES,
        });
    }
    if bytes < as_u64(BACKUP_HEADER_LEN + TAG_LEN) {
        return Err(DbError::NotABackup);
    }

    let file = File::open(path).map_err(|cause| DbError::Io {
        what: "the backup file",
        operation: "opened",
        cause,
    })?;
    let mut reader = BufReader::new(file);

    let mut header_bytes = [0_u8; BACKUP_HEADER_LEN];
    reader
        .read_exact(&mut header_bytes)
        .map_err(|_short| DbError::NotABackup)?;
    let header = BackupHeader::parse(&header_bytes).map_err(from_crypto)?;

    // The derivation runs over the salt and the parameters this file declares, which the
    // parser has already range checked, so nothing here reserves memory on a number
    // somebody else chose.
    let kek = derive_kek(password, header.kdf_salt(), header.params()).map_err(from_crypto)?;
    let key = export_key(&kek);

    let mut decrypting = Decrypting {
        source: reader,
        opener: Some(ChunkOpener::new(&key, &header)),
        ready: Zeroizing::new(Vec::new()),
        at: 0,
        buffer: vec![0_u8; READ_BUFFER_LEN],
        failure: None,
        read_so_far: as_u64(BACKUP_HEADER_LEN),
        chunks: 0,
        opened_something: false,
        progress,
    };

    let mut state = Reading::new(&header);
    let mut splitter = LineSplitter::new();

    let decompressed = decompress_bounded(&mut decrypting, |piece| {
        splitter.push(piece, |line| state.line(Line::parse(line)?, &mut sink))
    });

    // Whatever the reader stashed wins over whatever the decompressor said about it: the
    // decompressor can only report an input and output error, and the real reason is here.
    if let Some(failure) = decrypting.failure.take() {
        return Err(failure);
    }
    decompressed?;
    splitter.finish()?;

    // The decompressor stops at the end of the compressed frame, which is not necessarily
    // the end of the file. Without this, anything after that frame would never be decrypted
    // and the last chunk's tag — the one that says it is the last chunk — would never be
    // checked, so a truncated file would pass. This is the line that makes truncation fail.
    decrypting.drain()?;

    state.into_report(bytes, decrypting.chunks)
}

/// The file, decrypted chunk by chunk, as something the decompressor can read from.
///
/// Pull rather than push, because that is the shape the decompressor wants and because
/// pulling is what keeps the memory flat: it asks for as much as it needs and no more.
struct Decrypting<'key, 'progress, R: io::Read> {
    source: R,
    /// Taken once the last chunk has been opened, which is how "already finished" is
    /// represented without a separate flag that could disagree with it.
    opener: Option<ChunkOpener<'key>>,
    ready: Zeroizing<Vec<u8>>,
    at: usize,
    buffer: Vec<u8>,
    failure: Option<DbError>,
    read_so_far: u64,
    chunks: u64,
    opened_something: bool,
    progress: &'progress mut dyn FnMut(u64),
}

impl<R: io::Read> Decrypting<'_, '_, R> {
    /// Reads the rest of the file, so that every chunk is opened and the last one is
    /// checked against its mark.
    fn drain(&mut self) -> Result<(), DbError> {
        let mut discard = vec![0_u8; READ_BUFFER_LEN];

        loop {
            match self.read(&mut discard) {
                Ok(0) => break,
                Ok(_more) => {}
                Err(_wrapped) => {
                    return Err(self.failure.take().unwrap_or(DbError::Malformed));
                }
            }
        }

        Ok(())
    }

    /// Turns a failure to open a chunk into one of the four things this may say.
    fn explain(&self, failure: CryptoError) -> DbError {
        match failure {
            CryptoError::Open if !self.opened_something => DbError::WrongPassword,
            other => from_crypto(other),
        }
    }

    /// Stashes the real reason and hands the caller something an `io::Read` may return.
    fn stash(&mut self, failure: DbError) -> io::Error {
        self.failure = Some(failure);

        io::Error::other("the backup could not be read")
    }

    /// Fills [`Decrypting::ready`] with the next chunk's worth of plaintext, if there is one.
    ///
    /// The opener is taken out of its slot for the duration and put back, which is what lets
    /// the file, the buffer and the opener be touched in the same breath without borrowing
    /// the whole value. When the end of the file is reached it is not put back, and that
    /// absence is how "already finished" is recorded.
    fn refill(&mut self) -> io::Result<()> {
        self.ready.clear();
        self.at = 0;

        while self.ready.is_empty() {
            let Some(mut opener) = self.opener.take() else {
                return Ok(());
            };
            let mut buffer = core::mem::take(&mut self.buffer);
            let mut ready = core::mem::take(&mut self.ready);

            let taken = match self.source.read(&mut buffer) {
                Ok(taken) => taken,
                Err(cause) => {
                    self.buffer = buffer;
                    self.ready = ready;
                    return Err(self.stash(DbError::Io {
                        what: "the backup file",
                        operation: "read",
                        cause,
                    }));
                }
            };

            if taken == 0 {
                // The end of the file, which is where the last chunk is. Opening it checks
                // the mark that says it is the last one, which is what makes a file cut off
                // at a chunk boundary fail rather than restore a shorter vault.
                let outcome = opener.finish(&mut ready);
                self.buffer = buffer;
                self.ready = ready;

                return match outcome {
                    Ok(chunks) => {
                        self.chunks = chunks;
                        Ok(())
                    }
                    Err(failure) => {
                        let explained = self.explain(failure);
                        Err(self.stash(explained))
                    }
                };
            }

            self.read_so_far = self.read_so_far.saturating_add(as_u64(taken));
            (self.progress)(self.read_so_far);

            let outcome = opener.push(buffer.get(..taken).unwrap_or_default(), &mut ready);

            self.buffer = buffer;
            self.ready = ready;
            self.opener = Some(opener);

            if let Err(failure) = outcome {
                let explained = self.explain(failure);
                return Err(self.stash(explained));
            }
            if !self.ready.is_empty() {
                self.opened_something = true;
            }
        }

        Ok(())
    }
}

impl<R: io::Read> io::Read for Decrypting<'_, '_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.at >= self.ready.len() {
            self.refill()?;
        }

        let available = self.ready.get(self.at..).unwrap_or_default();
        if available.is_empty() {
            return Ok(0);
        }

        let taking = available.len().min(out.len());
        let Some(target) = out.get_mut(..taking) else {
            return Ok(0);
        };
        target.copy_from_slice(available.get(..taking).unwrap_or_default());
        self.at += taking;

        Ok(taking)
    }
}

/// What the reader has seen so far, and the rules about what may come next.
struct Reading {
    format_version: u16,
    record_version: u16,
    schema_version: u32,
    /// The table the rows currently arriving belong to, and how many it announced.
    current: Option<(String, u64)>,
    records: Vec<(String, u64)>,
    seen_manifest: bool,
}

impl Reading {
    fn new(header: &BackupHeader) -> Self {
        Self {
            format_version: header.format_version(),
            record_version: 0,
            schema_version: 0,
            current: None,
            records: Vec::new(),
            seen_manifest: false,
        }
    }

    /// Takes one line.
    fn line(
        &mut self,
        line: Line,
        sink: &mut impl FnMut(&str, RowValues) -> Result<(), DbError>,
    ) -> Result<(), DbError> {
        match line {
            Line::Manifest(manifest) => {
                // Exactly one, and first. A second manifest halfway through would be a file
                // describing itself twice, and a reader that took the later one would be
                // reading rows under a description that arrived after them.
                if self.seen_manifest || manifest.format != FORMAT_NAME {
                    return Err(DbError::Malformed);
                }
                if manifest.record_version > RECORD_VERSION {
                    return Err(DbError::UnsupportedVersion);
                }

                self.seen_manifest = true;
                self.record_version = manifest.record_version;
                self.schema_version = manifest.schema_version;
            }
            Line::Table(table) => {
                if !self.seen_manifest {
                    return Err(DbError::Malformed);
                }
                if table.rows > MAX_ROWS_PER_TABLE {
                    return Err(DbError::TooMany {
                        what: "rows in one table",
                        value: table.rows,
                        max: MAX_ROWS_PER_TABLE,
                    });
                }
                if table_named(&table.name).is_none() {
                    return Err(DbError::Malformed);
                }

                self.finish_table()?;
                self.records.push((table.name.clone(), 0));
                self.current = Some((table.name, table.rows));
            }
            Line::Row(values) => {
                let Some((name, expected)) = self.current.as_ref() else {
                    return Err(DbError::Malformed);
                };
                let name = name.clone();
                let expected = *expected;

                let Some(counted) = self.records.last_mut() else {
                    return Err(DbError::Malformed);
                };
                counted.1 = counted.1.saturating_add(1);

                // Counted before the row is handed on, so a table that announced ten rows
                // and carries eleven is refused on the eleventh rather than after all of
                // them have been written somewhere.
                if counted.1 > expected {
                    return Err(DbError::Malformed);
                }

                sink(&name, values)?;
            }
        }

        Ok(())
    }

    /// Checks that the table just finished carried the number of rows it announced.
    fn finish_table(&mut self) -> Result<(), DbError> {
        let Some((_name, expected)) = self.current.take() else {
            return Ok(());
        };
        let Some((_counted_name, counted)) = self.records.last() else {
            return Err(DbError::Malformed);
        };

        if *counted == expected {
            Ok(())
        } else {
            Err(DbError::Malformed)
        }
    }

    /// The report, once the file has been read to the end.
    fn into_report(mut self, bytes: u64, chunks: u64) -> Result<VerifyReport, DbError> {
        self.finish_table()?;

        if !self.seen_manifest {
            return Err(DbError::Malformed);
        }

        Ok(VerifyReport {
            bytes,
            chunks,
            format_version: self.format_version,
            record_version: self.record_version,
            schema_version: self.schema_version,
            records: self.records,
        })
    }
}

/// Turns a failure from the cryptography into one of the four things this may say.
fn from_crypto(failure: CryptoError) -> DbError {
    match failure {
        CryptoError::NotABackup | CryptoError::HeaderMagic => DbError::NotABackup,
        CryptoError::BackupVersion { .. } | CryptoError::BackupCompression { .. } => {
            DbError::UnsupportedVersion
        }
        CryptoError::Open => DbError::WrongPassword,
        CryptoError::Damaged
        | CryptoError::BackupReserved
        | CryptoError::MalformedSealed { .. }
        | CryptoError::ParamOutOfRange { .. }
        | CryptoError::ChunkCountExhausted => DbError::Malformed,
        // Anything left is about this machine rather than about the file: the operating
        // system refusing random bytes, or Argon2id refusing to run at parameters that are
        // inside the allowed range and still more than this device can give it.
        other => DbError::Sealed(other),
    }
}

/// A length as a count, saturating rather than truncating.
fn as_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use cairn_crypto::{Argon2Params, BACKUP_HEADER_LEN, MIN_MEMORY_KIB, MIN_PASSES, TAG_LEN};
    use cairn_domain::Hlc;

    use super::{read_backup, verify_backup};
    use crate::backup::export::write_backup;
    use crate::backup::format::RowValues;
    use crate::backup::schema::{TABLES, table_named};
    use crate::backup::tables::{read_table, write_row};
    use crate::device::DeviceId;
    use crate::error::DbError;
    use crate::repositories::settings;
    use crate::test_support::Sandbox;

    const NOW_US: i64 = 1_700_000_000_000_000;
    const PASSWORD: &str = "una frase larga para la copia";

    fn cheap() -> Argon2Params {
        Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, 1).expect("the floor is accepted")
    }

    /// Fills a sandbox with something recognisable.
    fn seeded(label: &str) -> Sandbox {
        let sandbox = Sandbox::new(label);
        let device = DeviceId::generate().expect("random bytes");

        sandbox
            .database()
            .with(|connection| {
                for index in 0..5_u64 {
                    settings::put(
                        connection,
                        &sandbox.codec(),
                        device,
                        Hlc::new(1_000 + index, 0, [1; 6]),
                        NOW_US,
                        &format!("preference-{index}"),
                        Some(format!("valor secreto {index}").as_bytes()),
                    )?;
                }
                Ok(())
            })
            .expect("the seed writes");

        sandbox
    }

    /// Exports a sandbox and gives back where the file is.
    fn exported(sandbox: &Sandbox, name: &str) -> PathBuf {
        let destination = sandbox.directory().join(name);
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
                    &mut |_so_far| {},
                )
                .map(|_report| ())
            })
            .expect("the export works");

        destination
    }

    /// Reads every table of a sandbox into memory, for comparing two databases.
    fn everything(sandbox: &Sandbox) -> Vec<(String, Vec<RowValues>)> {
        let codec = sandbox.codec();

        sandbox
            .database()
            .with(|connection| {
                let mut all = Vec::new();
                for table in TABLES {
                    let mut rows = Vec::new();
                    read_table(connection, &codec, table, |values| {
                        rows.push(values);
                        Ok(())
                    })?;
                    all.push((table.name.to_owned(), rows));
                }
                Ok(all)
            })
            .expect("the tables read")
    }

    /// Reads a backup into a sandbox, which is what an import does minus the staging
    /// database and the swap that the next branch adds.
    fn restored_into(sandbox: &Sandbox, path: &Path, password: &str) -> Result<(), DbError> {
        let codec = sandbox.codec();

        sandbox.database().with(|connection| {
            read_backup(path, password, &mut |_so_far| {}, |table, values| {
                let spec = table_named(table).ok_or(DbError::Malformed)?;
                write_row(connection, &codec, spec, &values)
            })
            .map(|_report| ())
        })
    }

    /// Writes a copy of a file with one byte changed.
    fn with_a_byte_changed(path: &Path, at: usize) -> PathBuf {
        let mut bytes = fs::read(path).expect("the file reads");
        if let Some(byte) = bytes.get_mut(at) {
            *byte ^= 0x01;
        }

        let damaged = path.with_extension("damaged");
        fs::write(&damaged, &bytes).expect("the copy writes");

        damaged
    }

    /// Writes a copy of a file cut to a length.
    fn truncated_to(path: &Path, len: usize) -> PathBuf {
        let bytes = fs::read(path).expect("the file reads");
        let cut = path.with_extension("cut");
        fs::write(&cut, bytes.get(..len).unwrap_or_default()).expect("the copy writes");

        cut
    }

    /// How long a file is, as a length.
    fn length_of(path: &Path) -> usize {
        usize::try_from(fs::metadata(path).expect("the file is there").len()).unwrap_or(0)
    }

    #[test]
    fn a_backup_restores_the_state_it_was_taken_from() {
        // The property the whole phase exists for, checked table by table rather than by
        // counting rows: two databases holding different things have equal counts.
        let source = seeded("verify-source");
        let destination = Sandbox::new("verify-destination");
        let path = exported(&source, "round-trip.cairn");

        restored_into(&destination, &path, PASSWORD).expect("the restore works");

        assert_eq!(everything(&destination), everything(&source));
    }

    #[test]
    fn the_report_says_what_the_file_holds() {
        let source = seeded("verify-report");
        let path = exported(&source, "report.cairn");

        let report = verify_backup(&path, PASSWORD, &mut |_so_far| {}).expect("it verifies");

        assert_eq!(report.bytes, fs::metadata(&path).unwrap().len());
        assert_eq!(report.format_version, cairn_crypto::BACKUP_FORMAT_VERSION);
        assert_eq!(report.record_version, crate::backup::format::RECORD_VERSION);
        assert_eq!(report.schema_version, crate::LATEST_VERSION);
        assert_eq!(
            report
                .records
                .iter()
                .find(|(name, _rows)| name == "settings"),
            Some(&("settings".to_owned(), 5))
        );
    }

    #[test]
    fn the_wrong_password_is_reported_as_the_wrong_password() {
        let source = seeded("verify-wrong-password");
        let path = exported(&source, "wrong.cairn");

        assert!(matches!(
            verify_backup(&path, "una frase distinta y larga", &mut |_so_far| {}),
            Err(DbError::WrongPassword)
        ));
    }

    #[test]
    fn a_backup_of_another_vault_is_reported_as_the_wrong_password() {
        // There is no installation identifier in the header, on purpose, so the honest
        // answer to "this is not your backup" is "this password does not open it", which is
        // exactly what it is. The user documentation says so in those words.
        let theirs = seeded("verify-other-vault");
        let mine = Sandbox::new("verify-mine");
        let path = exported(&theirs, "theirs.cairn");

        let refused = restored_into(&mine, &path, "la maestra de mi vault, que es otra");
        assert!(matches!(refused, Err(DbError::WrongPassword)));
        assert_eq!(everything(&mine), everything(&Sandbox::new("verify-fresh")));
    }

    #[test]
    fn something_that_is_not_a_backup_is_reported_as_such() {
        let sandbox = Sandbox::new("verify-not-a-backup");
        let path = sandbox.directory().join("nonsense.cairn");
        fs::write(&path, vec![0x41_u8; 4096]).expect("the file writes");

        assert!(matches!(
            verify_backup(&path, PASSWORD, &mut |_so_far| {}),
            Err(DbError::NotABackup)
        ));
    }

    #[test]
    fn a_file_too_short_to_be_a_backup_is_reported_as_not_a_backup() {
        let sandbox = Sandbox::new("verify-too-short");
        let path = sandbox.directory().join("stub.cairn");

        for len in [
            0_usize,
            1,
            BACKUP_HEADER_LEN,
            BACKUP_HEADER_LEN + TAG_LEN - 1,
        ] {
            fs::write(&path, vec![0x42_u8; len]).expect("the file writes");
            assert!(
                matches!(
                    verify_backup(&path, PASSWORD, &mut |_so_far| {}),
                    Err(DbError::NotABackup)
                ),
                "a file of {len} bytes was not refused"
            );
        }
    }

    #[test]
    fn a_file_from_a_later_format_version_is_refused() {
        // The refusal comes out of the header parser, before Argon2id runs. A build that
        // derived first and refused afterwards would spend a second of a phone's battery on
        // a file it was never going to read.
        let source = seeded("verify-future");
        let path = exported(&source, "future.cairn");

        let mut bytes = fs::read(&path).expect("the file reads");
        if let Some(target) = bytes.get_mut(8..10) {
            target.copy_from_slice(&(cairn_crypto::BACKUP_FORMAT_VERSION + 1).to_le_bytes());
        }
        let future = source.directory().join("future-version.cairn");
        fs::write(&future, &bytes).expect("the copy writes");

        assert!(matches!(
            verify_backup(&future, PASSWORD, &mut |_so_far| {}),
            Err(DbError::UnsupportedVersion)
        ));
    }

    #[test]
    fn a_truncated_file_is_refused_and_changes_nothing() {
        let source = seeded("verify-truncated");
        let destination = Sandbox::new("verify-truncated-destination");
        let before = everything(&destination);
        let path = exported(&source, "truncated.cairn");
        let whole = length_of(&path);

        for len in [
            BACKUP_HEADER_LEN + TAG_LEN,
            usize::midpoint(0, whole),
            whole - 1,
        ] {
            let cut = truncated_to(&path, len);
            let refused = restored_into(&destination, &cut, PASSWORD);

            assert!(
                refused.is_err(),
                "a file cut to {len} of {whole} bytes was accepted"
            );
        }

        assert_eq!(everything(&destination), before);
    }

    #[test]
    fn a_single_changed_byte_in_any_of_the_places_that_matter_is_noticed() {
        // One position per field of the header and one per zone of the body, rather than
        // every byte: each case costs a full Argon2id derivation, and the exhaustive walk
        // over every byte of a framed body already runs in the cryptography crate, where it
        // costs nothing. What is checked here is that the two halves are wired together —
        // that an edit to the salt, to the parameters, to the nonce base or to a tag all
        // reach a refusal rather than a restore.
        let source = seeded("verify-changed-byte");
        let path = exported(&source, "changed.cairn");
        let len = length_of(&path);

        let places = [
            ("the magic", 0),
            ("the format version", 8),
            ("the compression", 10),
            ("the salt", 12),
            ("the memory cost", 28),
            ("the passes", 32),
            ("the nonce base", 40),
            ("the chunk size", 56),
            ("the reserved field", 60),
            ("the first byte of the body", BACKUP_HEADER_LEN),
            (
                "the middle of the body",
                usize::midpoint(BACKUP_HEADER_LEN, len),
            ),
            ("the last byte of the final tag", len - 1),
        ];

        for (what, at) in places {
            let damaged = with_a_byte_changed(&path, at);
            assert!(
                verify_backup(&damaged, PASSWORD, &mut |_so_far| {}).is_err(),
                "a change to {what}, at byte {at} of {len}, went unnoticed"
            );
        }
    }

    #[test]
    fn a_file_that_fails_at_the_very_end_still_fails() {
        // The last tag, which is the one that says the file ends where it ends. This is the
        // reason an import writes into a staging database rather than the live one: by the
        // time this fails, most of the rows have already been read.
        let source = seeded("verify-last-byte");
        let destination = Sandbox::new("verify-last-byte-destination");
        let path = exported(&source, "last-byte.cairn");
        let len = length_of(&path);

        let damaged = with_a_byte_changed(&path, len - 1);
        assert!(restored_into(&destination, &damaged, PASSWORD).is_err());
    }

    #[test]
    fn progress_is_reported_and_never_goes_backwards() {
        let source = seeded("verify-progress");
        let path = exported(&source, "progress.cairn");
        let mut seen = Vec::new();

        verify_backup(&path, PASSWORD, &mut |so_far| seen.push(so_far)).expect("it verifies");

        assert!(!seen.is_empty(), "nothing was reported while it worked");
        assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]));
    }
}
