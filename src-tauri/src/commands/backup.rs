//! Writing the whole vault to one encrypted file, reading one back, and putting one in place.
//!
//! Seven commands: make a backup, check one, say how long it has been since the last, read one
//! into a staging database, put that staging database in place, throw it away, and write one
//! module out in the clear. Five decisions are worth knowing before reading any of them.
//!
//! **The path is never a parameter.** Every one of these opens a file dialog the operating
//! system draws, from Rust, and uses what the person picked. A path arriving from the WebView
//! would
//! be directory traversal handed over on a plate: the one thing an attacker who reaches a
//! WebView wants is to name a file, and nothing in this application lets them. The dialog
//! plugin is registered in the builder and its commands are never granted to the window, so
//! script inside the WebView cannot open one either.
//!
//! **No path comes back out.** A report carries the file's name and never the folder it is
//! in, because a folder carries the account name and very often the machine name, and this
//! application's rule is that a screenshot of any screen is safe to paste into a public
//! issue.
//!
//! **An export is not finished until it has been read back.** The file is written to a
//! temporary name, flushed to the platter, renamed, and then opened again and decrypted from
//! the first byte to the last using the password rather than the key still in memory. A
//! backup that has never been read is not a backup, and the day somebody needs it is the
//! worst possible day to discover that.
//!
//! **The long ones run off the drawing thread.** They are minutes of work on a large vault, and
//! they report progress as they go, so that somebody watching can tell slow from stuck.
//!
//! **A restore is two commands, and that is not an accident.** The first reads the whole file
//! into a database of its own and touches nothing; the second replaces the live vault. Split
//! that way, the confirmation is asked for by the core rather than decided in the WebView, and
//! it is asked *after* the file has proved it is worth the replacement. Nobody is asked to
//! destroy their data until the thing replacing it is known to be good.

use std::path::{Path, PathBuf};

use cairn_crypto::{Argon2Params, CryptoError};
use cairn_db::DbError;
use cairn_db::backup::export::write_backup;
use cairn_db::backup::history;
use cairn_db::backup::plaintext::Module;
use cairn_db::backup::verify::verify_backup;
use cairn_db::repositories::audit;
use cairn_domain::password::{self, PasswordProblem};
use cairn_domain::session::{backoff_remaining_s, locked_until_us};
use serde::{Deserialize, Serialize};
use tauri::{Emitter as _, Manager as _};
use tauri_plugin_dialog::DialogExt as _;
use zeroize::Zeroizing;

use crate::clock::{now_ms, now_us};
use crate::session::ImportTicket;
use crate::state::AppState;
use crate::storage::RestoreFailure;

/// The event a running export or verification reports its progress on.
///
/// One name for both, because the interface only ever has one of them in flight: the two
/// commands take the same permit, so a second one waits rather than interleaving its numbers
/// with the first.
pub const PROGRESS_EVENT: &str = "backup://progress";

/// What the file dialog calls the kind of file this writes.
const FILTER_NAME: &str = "Cairn backup";

/// The extension the dialog filters on and suggests.
const EXTENSION: &str = "cairn";

/// What the dialog calls a readable export.
const PLAINTEXT_FILTER_NAME: &str = "Hoja de cálculo";

/// The extension a readable export gets.
const PLAINTEXT_EXTENSION: &str = "csv";

/// How long a prepared import waits to be confirmed, in microseconds.
///
/// Ten minutes. Long enough to read the question and think about it, short enough that a
/// confirmation left on a screen somebody walked away from stops being one. The lock clears it
/// sooner in every case where the lock comes first.
const TICKET_LIFETIME_US: i64 = 10 * 60 * 1_000_000;

/// How many random bytes a confirmation word is made of.
const TOKEN_LEN: usize = 16;

/// What the copy of the vault being replaced is suggested as being called.
const SAFETY_COPY_NAME: &str = "cairn-antes-de-restaurar.db";

/// Which password an export is sealed with.
///
/// An enumeration rather than a flag, because these are two different operations and the
/// checks they get are different: the master password is checked against the vault, and a
/// separate one is checked against the password policy. A boolean parameter would have
/// hidden that behind a name that says neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PasswordSource {
    /// The password that opens the vault on this machine.
    Master,
    /// A password chosen for this file alone.
    Separate,
}

/// Why a backup operation did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[non_exhaustive]
pub enum BackupError {
    /// The vault is closed, and an export needs it open.
    #[error("the vault is locked")]
    Locked,

    /// The password given is not the one that opens this vault.
    ///
    /// Only ever produced by an export asked to use the master password, where getting it
    /// wrong would write a file whose password nobody knows. Reading a backup answers
    /// [`BackupError::WrongPassword`] for the same word with a different meaning, and that
    /// one is about the file rather than about this machine.
    #[error("that is not the password")]
    WrongPassword,

    /// Further attempts at the master password are refused for a while.
    ///
    /// The same tally and the same schedule as the unlock screen, because it is the same
    /// secret. A command that could be asked the master password without limit would be a way
    /// round the lockout, which is the whole reason that lockout exists.
    #[serde(rename_all = "camelCase")]
    #[error("further attempts are refused for {remaining_s} more seconds")]
    LockedOut {
        /// How many seconds are left before another attempt is allowed.
        remaining_s: u32,
    },

    /// A password chosen for the file alone was refused by the password policy.
    #[error("the password is not strong enough")]
    #[serde(rename_all = "camelCase")]
    WeakPassword {
        /// How many characters it has.
        chars: usize,
        /// How many are needed.
        min: usize,
    },

    /// The person closed the file dialog without choosing anything.
    #[error("no file was chosen")]
    Cancelled,

    /// An import has already been prepared and is waiting for a yes or a no.
    ///
    /// One at a time, because there is one staging database and it has one name. The screen
    /// that is showing the first question has to be answered before a second file is read.
    #[error("an import is already waiting to be confirmed")]
    AlreadyPreparing,

    /// There is no import waiting under that word, or the ten minutes have run out.
    ///
    /// One answer for both, because they are the same answer to whoever is asking: prepare it
    /// again. A separate "expired" would tell anything guessing at tokens that it had found
    /// one.
    #[error("there is no import waiting to be confirmed")]
    UnknownToken,

    /// The restore happened and the vault could not be opened again.
    ///
    /// Its own answer rather than [`BackupError::Io`], because it is the one failure of this
    /// phase that must never be described as nothing having happened. The restored vault is
    /// where the old one was, the old one is in the copy taken beside it, and what the person
    /// has to do is start the application again.
    #[error("the restore finished and the vault could not be opened again")]
    RestoredButNotOpen,

    /// The chosen file is not a Cairn backup at all.
    #[error("that file is not a Cairn backup")]
    NotABackup,

    /// It is a Cairn backup, from a version this build cannot read.
    #[error("that backup was written by a newer version of Cairn")]
    UnsupportedVersion,

    /// It is a Cairn backup and this build can read it, but not with that password.
    #[error("the backup did not open with that password")]
    BadPassword,

    /// The file is a Cairn backup, the password is right, and the contents do not hold up.
    #[error("the backup is damaged or incomplete")]
    Damaged,

    /// The disk refused. Deliberately without the reason, which names a path.
    #[error("the file could not be written or read")]
    Io,

    /// Something in the core refused for a reason that is not about this file.
    #[error("the operation could not be completed")]
    Crypto,
}

impl From<PasswordProblem> for BackupError {
    fn from(problem: PasswordProblem) -> Self {
        match problem {
            PasswordProblem::TooShort { chars, min } => Self::WeakPassword { chars, min },
            // Every other refusal the policy has is reported with the numbers of the one it
            // does have. A password the policy will not take is not exported under, and the
            // screen that asked for it already shows the strength meter that explains why.
            _other => Self::WeakPassword { chars: 0, min: 0 },
        }
    }
}

impl From<DbError> for BackupError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::NotABackup => Self::NotABackup,
            DbError::UnsupportedVersion => Self::UnsupportedVersion,
            DbError::WrongPassword => Self::BadPassword,
            // A file larger than one may be is refused the same way a damaged one is. It is
            // a statement about the file rather than about the machine, and telling the two
            // apart would be a fifth thing a reader says about somebody else's file.
            DbError::Malformed | DbError::TooMany { .. } => Self::Damaged,
            DbError::Io { .. } => Self::Io,
            _other => Self::Crypto,
        }
    }
}

/// What one export wrote.
///
/// The name of the file and two numbers. Not the folder: see the note at the top.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupExportReport {
    /// What the file is called, without the folder it is in.
    pub file_name: String,
    /// How large it is, in bytes.
    pub bytes: u64,
    /// How many chunks it holds.
    pub chunks: u64,
    /// How many rows it carries in total.
    pub records: u64,
}

/// What one verification found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupVerifyReport {
    /// What the file is called, without the folder it is in.
    pub file_name: String,
    /// How large it is, in bytes.
    pub bytes: u64,
    /// How many chunks it holds.
    pub chunks: u64,
    /// Which version of the file format it is in.
    pub format_version: u16,
    /// How many rows it carries in total.
    pub records: u64,
}

/// How long it has been since the last backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatusReport {
    /// How many whole days, or nothing if there has never been a backup.
    pub days_since_last: Option<u32>,
    /// Whether the screen should say so.
    pub remind: bool,
}

/// What an import found in the file, and the word that confirms it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreparedReport {
    /// The word [`backup_import_commit`] has to be given back.
    pub token: String,
    /// What the file is called, without the folder it is in.
    pub file_name: String,
    /// How many rows of each table it carries, in the order they appear.
    pub records_by_table: Vec<TableCount>,
    /// Whether the vault being replaced currently holds anything.
    ///
    /// The one thing the screen needs in order to phrase the question correctly. Restoring
    /// over an empty vault and restoring over a full one are the same operation and a very
    /// different sentence.
    pub has_existing_data: bool,
}

/// One table and how many rows of it a backup carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableCount {
    /// The table, as the schema names it.
    pub table: String,
    /// How many rows.
    pub rows: u64,
}

/// What a finished restore replaced, and where the vault it replaced went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCommittedReport {
    /// What the copy of the replaced vault is called, without the folder it is in.
    ///
    /// The name only, like everything else that crosses this bridge. The folder is the one the
    /// person chose a moment earlier in a dialog they were looking at.
    pub backup_copy_file_name: String,
    /// How many rows of each table were restored.
    pub records_by_table: Vec<TableCount>,
}

/// What one plaintext export wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaintextExportReport {
    /// What the file is called, without the folder it is in.
    pub file_name: String,
    /// How many rows it holds.
    pub records: u64,
}

/// How far along a running operation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupProgress {
    /// How many bytes have been processed so far.
    pub done: u64,
}

/// Writes the whole vault to one encrypted file and reads it back before saying it is done.
///
/// Asks for the destination in a dialog the operating system draws. The password is used
/// twice by the core: once to seal the file, and once more by the verification pass, which
/// derives the key again from what the finished file says rather than reusing what is in
/// memory, so the header it is checking is the header that was actually written.
///
/// # Errors
///
/// Returns [`BackupError::Locked`] if the vault is closed, [`BackupError::WrongPassword`] if
/// the master password was asked for and is not the right one, [`BackupError::WeakPassword`]
/// if a separate password is below the policy, [`BackupError::Cancelled`] if the dialog was
/// closed, [`BackupError::Io`] if the disk refuses and [`BackupError::Crypto`] if a stored
/// value does not decrypt. On any failure the temporary file is removed and no file appears
/// at the destination.
#[tauri::command]
pub async fn backup_export(
    app: tauri::AppHandle,
    password: String,
    source: PasswordSource,
) -> Result<BackupExportReport, BackupError> {
    let password = Zeroizing::new(password);

    run_off_the_drawing_thread(app, move |app| export(&app, &password, source)).await
}

/// Reads a backup end to end and reports what is in it, writing nothing.
///
/// Does not need the vault open, and that is the point of having it as a command of its own:
/// the moment somebody most wants to know whether their backup is any good is the moment
/// they cannot get into the application.
///
/// # Errors
///
/// Returns the four things a reader may say about somebody else's file and no more:
/// [`BackupError::NotABackup`], [`BackupError::UnsupportedVersion`],
/// [`BackupError::BadPassword`] and [`BackupError::Damaged`]. Also
/// [`BackupError::Cancelled`] if the dialog was closed and [`BackupError::Io`] if the disk
/// refuses, both of which are about this machine rather than about the file.
#[tauri::command]
pub async fn backup_verify(
    app: tauri::AppHandle,
    password: String,
) -> Result<BackupVerifyReport, BackupError> {
    let password = Zeroizing::new(password);

    run_off_the_drawing_thread(app, move |app| verify(&app, &password)).await
}

/// Says how long it has been since the last backup, and whether to mention it.
///
/// Days and a yes or no. Never the folder: the interface has no use for a path, and a path is
/// the one piece of this that carries somebody's name.
///
/// # Errors
///
/// Returns [`BackupError::Locked`] if the vault is closed, and [`BackupError::Crypto`] if the
/// stored value does not decrypt.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro deserialises its arguments into owned values and hands them over by value"
)]
pub fn backup_status(app: tauri::AppHandle) -> Result<BackupStatusReport, BackupError> {
    let state = app.try_state::<AppState>().ok_or(BackupError::Locked)?;
    let now = now_us();

    let status = state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage
                .database()
                .with(|connection| history::status(connection, &codec, now))
        })
        .ok_or(BackupError::Locked)??;

    Ok(BackupStatusReport {
        days_since_last: status.days_since_last,
        remind: status.remind,
    })
}

/// Reads a backup into a staging database beside the live one, touching nothing.
///
/// The long half of a restore, and the half that decides whether the file is any good. When
/// this comes back the whole file has been decrypted, decompressed, checked against every
/// limit and written row by row into a second database. The live vault has not been opened for
/// writing and not one byte from the file has reached it.
///
/// What comes back with it is a token and the counts per table, which is what the screen needs
/// to ask the only question worth asking: this file holds these rows, and you have these — do
/// you want yours replaced?
///
/// # Errors
///
/// The four a reader may say about a file — [`BackupError::NotABackup`],
/// [`BackupError::UnsupportedVersion`], [`BackupError::BadPassword`] and
/// [`BackupError::Damaged`] — plus [`BackupError::Locked`] if the vault is closed,
/// [`BackupError::Cancelled`] if the dialog was closed, [`BackupError::AlreadyPreparing`] if
/// an import is already waiting, and [`BackupError::Io`] if the disk refuses.
#[tauri::command]
pub async fn backup_import_begin(
    app: tauri::AppHandle,
    password: String,
) -> Result<ImportPreparedReport, BackupError> {
    let password = Zeroizing::new(password);

    run_off_the_drawing_thread(app, move |app| import_begin(&app, &password)).await
}

/// Replaces the live vault with the one that was prepared, after copying it aside.
///
/// The short half, and the irreversible one. Everything it does is on disk and in this order:
/// the live vault is copied to a file beside the one the person chose, the connection is
/// closed, the staging database takes the live one's place in a single system call, and the
/// vault is opened again.
///
/// # Errors
///
/// Returns [`BackupError::UnknownToken`] if the word is not the one waiting or has expired,
/// [`BackupError::Locked`] if the vault closed in between, and [`BackupError::Io`] if the disk
/// refuses. A failure before the replacement leaves the vault exactly as it was.
#[tauri::command]
pub async fn backup_import_commit(
    app: tauri::AppHandle,
    token: String,
) -> Result<ImportCommittedReport, BackupError> {
    run_off_the_drawing_thread(app, move |app| import_commit(&app, &token)).await
}

/// Throws away a prepared import and the staging database it wrote.
///
/// # Errors
///
/// Returns [`BackupError::UnknownToken`] if there is nothing waiting under that word, and
/// [`BackupError::Io`] if the staging database cannot be removed.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro deserialises its arguments into owned values and hands them over by value"
)]
pub fn backup_import_cancel(app: tauri::AppHandle, token: String) -> Result<(), BackupError> {
    let state = app.try_state::<AppState>().ok_or(BackupError::Locked)?;

    let waiting = state
        .session()
        .take_import(&token, now_us())
        .ok_or(BackupError::UnknownToken)?;

    cairn_db::backup::import::discard(&waiting.staging)?;

    Ok(())
}

/// Writes one module out as a file anybody can read, after checking the master password.
///
/// The password is asked for again and derived again, and that is the barrier. The word the
/// person types into the screen is a guard against absent-mindedness and lives in the
/// interface; this one is cryptographic and lives here.
///
/// # Errors
///
/// Returns [`BackupError::Locked`] if the vault is closed, [`BackupError::WrongPassword`] or
/// [`BackupError::LockedOut`] from the same tally the unlock screen uses,
/// [`BackupError::Cancelled`] if the dialog was closed, and [`BackupError::Io`] if the disk
/// refuses. Nothing is written unless the password was right.
#[tauri::command]
pub async fn export_plaintext(
    app: tauri::AppHandle,
    module: Module,
    password: String,
) -> Result<PlaintextExportReport, BackupError> {
    let password = Zeroizing::new(password);

    run_off_the_drawing_thread(app, move |app| plaintext(&app, module, &password)).await
}

/// Runs one of the two long operations on the blocking pool, one at a time.
///
/// The permit is the one that already serialises anything that can change the vault. An
/// export reads every row of every table and a verification runs Argon2id twice; either one
/// racing a password change would be reading a database whose key is being replaced.
async fn run_off_the_drawing_thread<T, F>(app: tauri::AppHandle, work: F) -> Result<T, BackupError>
where
    T: Send + 'static,
    F: FnOnce(tauri::AppHandle) -> Result<T, BackupError> + Send + 'static,
{
    // A second handle, because the first is moved onto the blocking thread and the permit
    // borrows the state that the handle reaches. Both name the same application.
    let held = app.clone();
    let state = held.try_state::<AppState>().ok_or(BackupError::Locked)?;
    let _permit = state.begin_vault_operation().await;

    tauri::async_runtime::spawn_blocking(move || work(app))
        .await
        .map_err(|_joining| BackupError::Io)?
}

/// The export itself, on a thread that is allowed to block.
fn export(
    app: &tauri::AppHandle,
    password: &str,
    source: PasswordSource,
) -> Result<BackupExportReport, BackupError> {
    let state = app.try_state::<AppState>().ok_or(BackupError::Locked)?;

    // Before anything is derived, and before the dialog opens. A closed vault is refused here
    // rather than sixty seconds later, and refusing it first is what stops this command being
    // a way to test master passwords against a locked vault: an export needs the vault open
    // anyway, so there is nothing lost by insisting on it before a password is looked at.
    if !state.session().is_unlocked() {
        return Err(BackupError::Locked);
    }

    // Asking somebody to choose a folder and name a file and only then telling them the
    // password was wrong is a worse apology than not opening the dialog at all.
    match source {
        PasswordSource::Master => check_master(&state, password, now_us())?,
        PasswordSource::Separate => password::validate(password)?,
    }

    let params = state.vault().params().unwrap_or(Argon2Params::DEFAULT);
    let destination = ask_where_to_save(app)?;

    let report = state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            let schema_version = storage.schema_version();

            storage.database().with(|connection| {
                write_backup(
                    connection,
                    &codec,
                    schema_version,
                    password,
                    params,
                    &destination,
                    &mut |done| report_progress(app, done),
                )
            })
        })
        .ok_or(BackupError::Locked)??;

    let (written, verified) = report;

    // After the file exists and has been read back, never before. A vault whose history says
    // it was backed up on a day when the export failed is a vault that lies about the one
    // thing this record is for.
    remember(&state, |connection, codec, device, hlc, now| {
        audit::record(
            connection,
            codec,
            device,
            hlc,
            now,
            audit::EventKind::BackupExported,
            Some(file_name_of(&destination).as_bytes()),
        )?;

        history::record_export(
            connection,
            codec,
            device,
            hlc,
            now,
            destination.parent().unwrap_or_else(|| Path::new(".")),
        )
    });

    Ok(BackupExportReport {
        file_name: file_name_of(&destination),
        bytes: written.bytes,
        chunks: written.chunks,
        records: verified.records.iter().map(|(_table, rows)| rows).sum(),
    })
}

/// The verification itself, on a thread that is allowed to block.
fn verify(app: &tauri::AppHandle, password: &str) -> Result<BackupVerifyReport, BackupError> {
    let chosen = ask_which_to_open(app)?;

    let report = verify_backup(&chosen, password, &mut |done| report_progress(app, done))?;

    if let Some(state) = app.try_state::<AppState>() {
        remember(&state, |connection, codec, device, hlc, now| {
            audit::record(
                connection,
                codec,
                device,
                hlc,
                now,
                audit::EventKind::BackupVerified,
                Some(file_name_of(&chosen).as_bytes()),
            )
            .map(|_written| ())
        });
    }

    Ok(BackupVerifyReport {
        file_name: file_name_of(&chosen),
        bytes: report.bytes,
        chunks: report.chunks,
        format_version: report.format_version,
        records: report.records.iter().map(|(_table, rows)| rows).sum(),
    })
}

/// The preparation itself, on a thread that is allowed to block.
fn import_begin(
    app: &tauri::AppHandle,
    password: &str,
) -> Result<ImportPreparedReport, BackupError> {
    let state = app.try_state::<AppState>().ok_or(BackupError::Locked)?;

    // Before the dialog opens. A restore replaces the vault this session has open, so a closed
    // one has nothing to replace, and asking somebody to find a file first would be a longer
    // way of saying the same thing.
    if !state.session().is_unlocked() {
        return Err(BackupError::Locked);
    }

    let chosen = ask_which_to_open(app)?;
    let directory = state.directory().path().to_path_buf();
    let now = now_us();

    let prepared = state
        .session()
        .with_open(|vault, storage| {
            storage.database().with(|live| {
                cairn_db::backup::import::prepare_import(
                    &directory,
                    live,
                    vault,
                    password,
                    &chosen,
                    now,
                    &mut |done| report_progress(app, done),
                )
            })
        })
        .ok_or(BackupError::Locked)??;

    let ticket = ImportTicket {
        token: fresh_token()?,
        staging: prepared.staging.clone(),
        records: prepared.report.records.clone(),
        expires_us: now.saturating_add(TICKET_LIFETIME_US),
    };
    let token = ticket.token.clone();

    match state.session().hold_import(ticket, now) {
        Ok(expired) => {
            // A ticket nobody answered inside ten minutes, whose staging database this run has
            // already replaced. Removing it is tidying up after the earlier attempt, and a
            // failure to tidy is not a reason to refuse this one.
            if let Some(stale) = expired {
                let _removed = cairn_db::backup::import::discard(&stale.staging);
            }
        }
        Err(_already) => {
            // Whatever this call wrote is thrown away, because the one that is waiting owns
            // that name and answering it is what the person has to do next.
            let _removed = cairn_db::backup::import::discard(&prepared.staging);

            return Err(BackupError::AlreadyPreparing);
        }
    }

    Ok(ImportPreparedReport {
        token,
        file_name: file_name_of(&chosen),
        records_by_table: counts_of(&prepared.report.records),
        has_existing_data: prepared.has_existing_data,
    })
}

/// The replacement itself, on a thread that is allowed to block.
fn import_commit(
    app: &tauri::AppHandle,
    token: &str,
) -> Result<ImportCommittedReport, BackupError> {
    let state = app.try_state::<AppState>().ok_or(BackupError::Locked)?;
    let now = now_us();

    let waiting = state
        .session()
        .take_import(token, now)
        .ok_or(BackupError::UnknownToken)?;

    let safety_copy = ask_where_to_put_the_copy(app)?;

    // Taken out rather than borrowed, because the file underneath it is about to be replaced
    // and a handle to it has to be closed for that to happen at all. Between here and the
    // attach below, the session has keys and no database, which everything else already reads
    // as closed.
    let storage = state.session().take_storage().ok_or(BackupError::Locked)?;

    let attempted = state
        .session()
        .with_vault(|vault| storage.restore_from(&waiting.staging, vault, &safety_copy, now));

    let Some(outcome) = attempted else {
        // The vault closed between taking the storage and asking for the keys. Nothing has been
        // replaced; the storage is dropped here, which closes it, and the session is already
        // locked as far as anything else is concerned.
        return Err(BackupError::Locked);
    };

    match outcome {
        Ok(opened) => {
            state.session().attach_storage(opened);

            // Into the restored vault, which is the only vault there is now. The note says
            // that this database is not the one it was, which is a question somebody asks
            // about their own history and would otherwise have no way to answer.
            remember(&state, |connection, codec, device, hlc, moment| {
                audit::record(
                    connection,
                    codec,
                    device,
                    hlc,
                    moment,
                    audit::EventKind::BackupImported,
                    Some(file_name_of(&safety_copy).as_bytes()),
                )
                .map(|_written| ())
            });

            Ok(ImportCommittedReport {
                backup_copy_file_name: file_name_of(&safety_copy),
                // The counts the file was read with, carried since the preparation. Counting
                // the restored vault instead would answer a different question: these are what
                // the backup said it held, and the point is that they arrived.
                records_by_table: counts_of(&waiting.records),
            })
        }
        Err(RestoreFailure::Refused { storage, cause }) => {
            // Nothing moved. The vault goes back where it was, open, and the person is told
            // why the restore did not start.
            state.session().attach_storage(*storage);

            Err(BackupError::from(cause))
        }
        Err(RestoreFailure::Closed(_cause)) => {
            // Nothing moved and the vault is not open here any more. Locking is the honest
            // thing: the person unlocks again into the vault they already had.
            let _was_open = state.session().lock();

            Err(BackupError::Io)
        }
        Err(RestoreFailure::Opened(_cause)) => {
            // The restore happened. This must not be reported as nothing having happened.
            let _was_open = state.session().lock();

            Err(BackupError::RestoredButNotOpen)
        }
    }
}

/// The plaintext export itself, on a thread that is allowed to block.
fn plaintext(
    app: &tauri::AppHandle,
    module: Module,
    password: &str,
) -> Result<PlaintextExportReport, BackupError> {
    let state = app.try_state::<AppState>().ok_or(BackupError::Locked)?;

    if !state.session().is_unlocked() {
        return Err(BackupError::Locked);
    }

    // Before the dialog, and charged to the same tally as the unlock screen. This is the
    // barrier: the word typed into the screen guards against absent-mindedness, and this
    // guards against somebody else at the keyboard.
    check_master(&state, password, now_us())?;

    let destination = ask_where_to_save_plaintext(app, module)?;

    let report = state
        .session()
        .with_open(|vault, storage| {
            let codec = storage.codec(vault);
            storage.database().with(|connection| {
                cairn_db::backup::plaintext::write_plaintext(
                    connection,
                    &codec,
                    module,
                    &destination,
                )
            })
        })
        .ok_or(BackupError::Locked)??;

    // The one export whose record matters most, because the file it made has nothing
    // protecting it. Somebody looking at their own history should be able to see that on such
    // a day a readable copy of this module left the vault.
    remember(&state, |connection, codec, device, hlc, moment| {
        audit::record(
            connection,
            codec,
            device,
            hlc,
            moment,
            audit::EventKind::PlaintextExported,
            Some(module.as_str().as_bytes()),
        )
        .map(|_written| ())
    });

    Ok(PlaintextExportReport {
        file_name: file_name_of(&destination),
        records: report.records,
    })
}

/// Writes something into the vault's own history, if the vault is open to write it into.
///
/// Best effort, and deliberately so. Everything that calls this has already succeeded at the
/// thing the person asked for; failing an export that is on the disk because the note about it
/// would not write would be reporting the wrong failure, and the note is the smaller loss.
///
/// The vault being closed is not a failure at all. A verification runs with the vault locked
/// by design — the moment somebody most wants to check a backup is the moment they cannot get
/// in — and there is nowhere to write a note in that case.
fn remember<F>(state: &tauri::State<'_, AppState>, write: F)
where
    F: FnOnce(
        &cairn_db::Connection,
        &cairn_db::codec::FieldCodec<'_>,
        cairn_db::DeviceId,
        cairn_domain::Hlc,
        i64,
    ) -> Result<(), DbError>,
{
    let now = now_us();
    let _outcome = state.session().with_open(|vault, storage| {
        let codec = storage.codec(vault);
        let device = storage.device();
        let hlc = storage.next_hlc(now_ms());

        storage
            .database()
            .with(|connection| write(connection, &codec, device, hlc, now))
    });
}

/// The counts a report carries, in the shape the interface reads.
fn counts_of(records: &[(String, u64)]) -> Vec<TableCount> {
    records
        .iter()
        .map(|(table, rows)| TableCount {
            table: table.clone(),
            rows: *rows,
        })
        .collect()
}

/// A word nobody can guess, for one confirmation.
///
/// Sixteen bytes from the system's own generator, written as hexadecimal. It is not a name and
/// it is not an index: it is what keeps anything else that reaches the bridge from confirming
/// a replacement it did not prepare.
///
/// # Errors
///
/// Returns [`BackupError::Crypto`] if the system will not give out random bytes, which is a
/// machine that should not be performing a restore either.
fn fresh_token() -> Result<String, BackupError> {
    use std::fmt::Write as _;

    let mut bytes = [0_u8; TOKEN_LEN];
    cairn_crypto::fill_random(&mut bytes).map_err(|_refused| BackupError::Crypto)?;

    Ok(bytes
        .iter()
        .fold(String::with_capacity(TOKEN_LEN * 2), |mut word, byte| {
            // The write cannot fail: a `String` is the one sink that never refuses.
            let _written = write!(word, "{byte:02x}");
            word
        }))
}

/// Checks that this is the password that opens this vault.
///
/// A full Argon2id derivation, thrown away. There is no cheaper way that is also honest: the
/// keys in memory were derived from the password that was typed at unlock, and comparing
/// against them would be comparing the password to itself. What this costs is a second of
/// the export, and what it buys is that nobody writes a year of their life into a file sealed
/// with a typo and finds out on the day they need it.
///
/// It is also, unavoidably, a place where a password can be tried, so it is charged the same
/// way an unlock is: a lockout already in force refuses it before anything is derived, and a
/// wrong answer counts against the same tally. The consequence is worth stating plainly —
/// getting this wrong several times locks the vault for a while, exactly as getting the
/// unlock screen wrong several times does. It is the same secret, and a door that is only
/// bolted on one side is not bolted.
fn check_master(
    state: &tauri::State<'_, AppState>,
    password: &str,
    now: i64,
) -> Result<(), BackupError> {
    let header = state.vault().header().cloned().ok_or(BackupError::Locked)?;

    // Decided before anything is derived, so the refusal says nothing about the password.
    let remaining_s = backoff_remaining_s(state.vault().locked_until_us(), now);
    if remaining_s > 0 {
        return Err(BackupError::LockedOut { remaining_s });
    }

    match cairn_crypto::unlock(&header, password) {
        Ok(_discarded) => {
            let _recorded = state.vault().record_attempt(0, 0);
            Ok(())
        }
        Err(CryptoError::Open) => {
            let failed = state.vault().failed_attempts().saturating_add(1);
            // Recorded before the failure is reported, so that closing the window between the
            // two does not hand back an attempt.
            let _recorded = state
                .vault()
                .record_attempt(failed, locked_until_us(failed, now));

            Err(BackupError::WrongPassword)
        }
        Err(_other) => Err(BackupError::Crypto),
    }
}

/// Opens the save dialog and answers where the person wants the file.
fn ask_where_to_save(app: &tauri::AppHandle) -> Result<PathBuf, BackupError> {
    app.dialog()
        .file()
        .add_filter(FILTER_NAME, &[EXTENSION])
        .set_file_name(suggested_name())
        .blocking_save_file()
        .ok_or(BackupError::Cancelled)?
        .into_path()
        .map_err(|_not_a_path| BackupError::Io)
}

/// Opens the open dialog and answers which file the person chose.
fn ask_which_to_open(app: &tauri::AppHandle) -> Result<PathBuf, BackupError> {
    app.dialog()
        .file()
        .add_filter(FILTER_NAME, &[EXTENSION])
        .blocking_pick_file()
        .ok_or(BackupError::Cancelled)?
        .into_path()
        .map_err(|_not_a_path| BackupError::Io)
}

/// Opens the save dialog for the copy of the vault that is about to be replaced.
///
/// A dialog rather than a folder this code picks, and the reason is the one thing a restore
/// has to get right: the person has to know where their old vault went, and the surest way for
/// them to know is for them to have chosen it. The suggestion names what it is.
fn ask_where_to_put_the_copy(app: &tauri::AppHandle) -> Result<PathBuf, BackupError> {
    app.dialog()
        .file()
        .set_title("Dónde guardar la copia de la bóveda actual")
        .set_file_name(SAFETY_COPY_NAME)
        .blocking_save_file()
        .ok_or(BackupError::Cancelled)?
        .into_path()
        .map_err(|_not_a_path| BackupError::Io)
}

/// Opens the save dialog for a readable export.
fn ask_where_to_save_plaintext(
    app: &tauri::AppHandle,
    module: Module,
) -> Result<PathBuf, BackupError> {
    app.dialog()
        .file()
        .add_filter(PLAINTEXT_FILTER_NAME, &[PLAINTEXT_EXTENSION])
        .set_file_name(format!("cairn-{}.{PLAINTEXT_EXTENSION}", module.as_str()))
        .blocking_save_file()
        .ok_or(BackupError::Cancelled)?
        .into_path()
        .map_err(|_not_a_path| BackupError::Io)
}

/// What the dialog offers as a name.
///
/// Deliberately without a date in it, which is the obvious thing to want. This application
/// has no civil calendar yet: the only clock it has answers in microseconds since the epoch
/// in UTC, and turning that into a year, a month and a day is arithmetic that belongs in the
/// domain crate beside `CivilDay` rather than invented here for a filename. Until then a
/// suggestion with a date would be a suggestion with the wrong date for anybody east or west
/// of Greenwich in the hours around midnight, and a wrong date on a backup is worse than no
/// date at all. The dialog is open and the name is selected; whoever is saving can type.
fn suggested_name() -> String {
    format!("cairn-backup.{EXTENSION}")
}

/// The name of a file, without the folder it is in.
///
/// A path that has no final component is not something any dialog returns, and the fallback
/// is there so that a report can still be built rather than the whole export being thrown
/// away over the label on it.
fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || format!("backup.{EXTENSION}"),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Tells the interface how far along the operation is.
///
/// A failure to emit is dropped. The event is a courtesy to somebody watching a progress
/// bar; abandoning a finished export because the window stopped listening would be the
/// wrong way round.
fn report_progress(app: &tauri::AppHandle, done: u64) {
    let _ignored = app.emit(PROGRESS_EVENT, BackupProgress { done });
}

#[cfg(test)]
mod tests {
    use super::{
        BackupError, BackupStatusReport, ImportPreparedReport, Module, PasswordSource, TableCount,
        file_name_of, fresh_token, suggested_name,
    };
    use cairn_db::DbError;
    use cairn_domain::password::PasswordProblem;
    use std::path::Path;

    #[test]
    fn the_suggested_name_carries_the_extension_the_dialog_filters_on() {
        let name = suggested_name();

        assert!(name.starts_with("cairn-"), "{name}");
        assert_eq!(
            Path::new(&name)
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("cairn"),
            "{name}"
        );
    }

    #[test]
    fn a_report_carries_the_file_name_and_never_the_folder() {
        // The whole reason this function exists. A folder carries the account name, and
        // every screen in this application has to be safe to put in a public issue.
        let name = file_name_of(
            Path::new("somewhere")
                .join("deep")
                .join("copia.cairn")
                .as_path(),
        );

        assert_eq!(name, "copia.cairn");
    }

    #[test]
    fn the_four_things_a_reader_may_say_are_the_four_it_says() {
        assert_eq!(
            BackupError::from(DbError::NotABackup),
            BackupError::NotABackup
        );
        assert_eq!(
            BackupError::from(DbError::UnsupportedVersion),
            BackupError::UnsupportedVersion
        );
        assert_eq!(
            BackupError::from(DbError::WrongPassword),
            BackupError::BadPassword
        );
        assert_eq!(BackupError::from(DbError::Malformed), BackupError::Damaged);
    }

    #[test]
    fn a_file_larger_than_one_may_be_is_refused_as_damage_rather_than_as_its_own_answer() {
        // A fifth answer about somebody else's file is a fifth thing that tells whoever is
        // editing it how close they got.
        let too_large = DbError::TooMany {
            what: "bytes in the backup file",
            value: 1,
            max: 0,
        };

        assert_eq!(BackupError::from(too_large), BackupError::Damaged);
    }

    #[test]
    fn a_short_password_comes_back_with_the_numbers_the_screen_needs() {
        let refused = BackupError::from(PasswordProblem::TooShort { chars: 4, min: 12 });

        assert_eq!(refused, BackupError::WeakPassword { chars: 4, min: 12 });
    }

    #[test]
    fn the_password_source_is_the_two_words_the_interface_sends() {
        let master: PasswordSource =
            serde_json::from_str("\"master\"").expect("the interface sends this");
        let separate: PasswordSource =
            serde_json::from_str("\"separate\"").expect("the interface sends this");

        assert_eq!(master, PasswordSource::Master);
        assert_eq!(separate, PasswordSource::Separate);
    }

    #[test]
    fn the_lockout_carries_the_number_the_countdown_needs() {
        // The same shape the unlock screen already gets, because it is the same tally. A
        // refusal without the seconds is a refusal somebody retries immediately.
        let encoded = serde_json::to_string(&BackupError::LockedOut { remaining_s: 8 })
            .expect("it serialises");

        assert_eq!(encoded, "{\"kind\":\"lockedOut\",\"remainingS\":8}");
    }

    #[test]
    fn a_confirmation_word_is_sixteen_bytes_of_randomness_and_never_the_same_twice() {
        // Not a name and not an index. It is what keeps anything else that reaches the bridge
        // from confirming a replacement it did not prepare.
        let first = fresh_token().expect("the system gives out random bytes");
        let second = fresh_token().expect("the system gives out random bytes");

        assert_eq!(first.len(), 32, "{first}");
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn a_missing_import_and_an_expired_one_are_the_same_answer() {
        // Two answers would tell whatever is guessing at words that it had found one.
        let encoded = serde_json::to_string(&BackupError::UnknownToken).expect("it serialises");

        assert_eq!(encoded, "{\"kind\":\"unknownToken\"}");
    }

    #[test]
    fn a_restore_that_happened_is_never_reported_as_nothing_happening() {
        // The one failure of this phase that must not be folded into a general one. The
        // restored vault is where the old one was, and the old one is in the copy beside it.
        assert_ne!(BackupError::RestoredButNotOpen, BackupError::Io);

        let encoded =
            serde_json::to_string(&BackupError::RestoredButNotOpen).expect("it serialises");

        assert_eq!(encoded, "{\"kind\":\"restoredButNotOpen\"}");
    }

    #[test]
    fn the_module_is_the_three_words_the_interface_sends() {
        for (word, module) in [
            ("\"habits\"", Module::Habits),
            ("\"vault\"", Module::Vault),
            ("\"finance\"", Module::Finance),
        ] {
            let sent: Module = serde_json::from_str(word).expect("the interface sends this");
            assert_eq!(sent, module);
        }

        assert!(
            serde_json::from_str::<Module>("\"audit_events\"").is_err(),
            "a table name crossed the bridge as a module"
        );
    }

    #[test]
    fn a_status_with_no_backup_ever_says_so_rather_than_saying_zero_days() {
        // Zero days since the last backup and no backup at all are opposite facts, and a
        // screen that showed the first for the second would reassure exactly the wrong person.
        let never = serde_json::to_string(&BackupStatusReport {
            days_since_last: None,
            remind: true,
        })
        .expect("it serialises");

        assert_eq!(never, "{\"daysSinceLast\":null,\"remind\":true}");
    }

    #[test]
    fn a_prepared_import_reports_counts_and_a_name_and_no_path() {
        let report = ImportPreparedReport {
            token: "la palabra".to_owned(),
            file_name: "copia.cairn".to_owned(),
            records_by_table: vec![TableCount {
                table: "settings".to_owned(),
                rows: 3,
            }],
            has_existing_data: true,
        };

        let encoded = serde_json::to_string(&report).expect("it serialises");

        assert!(
            encoded.contains("\"fileName\":\"copia.cairn\""),
            "{encoded}"
        );
        assert!(encoded.contains("\"hasExistingData\":true"), "{encoded}");
        assert!(
            !encoded.contains('\\'),
            "a path crossed the bridge: {encoded}"
        );
    }

    #[test]
    fn every_failure_is_tagged_so_the_interface_matches_rather_than_reads() {
        let encoded = serde_json::to_string(&BackupError::Cancelled).expect("it serialises");

        assert_eq!(encoded, "{\"kind\":\"cancelled\"}");
    }
}
