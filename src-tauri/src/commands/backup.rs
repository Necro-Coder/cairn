//! Writing the whole vault to one encrypted file, and reading one back to check it.
//!
//! Two commands, and four decisions worth knowing before reading them.
//!
//! **The path is never a parameter.** Both of these open a file dialog the operating system
//! draws, from Rust, and use what the person picked. A path arriving from the WebView would
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
//! **Both run off the drawing thread.** They are minutes of work on a large vault, and they
//! report progress as they go, so that somebody watching can tell slow from stuck.

use std::path::{Path, PathBuf};

use cairn_crypto::{Argon2Params, CryptoError};
use cairn_db::DbError;
use cairn_db::backup::export::write_backup;
use cairn_db::backup::verify::verify_backup;
use cairn_domain::password::{self, PasswordProblem};
use cairn_domain::session::{backoff_remaining_s, locked_until_us};
use serde::{Deserialize, Serialize};
use tauri::{Emitter as _, Manager as _};
use tauri_plugin_dialog::DialogExt as _;
use zeroize::Zeroizing;

use crate::clock::now_us;
use crate::state::AppState;

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

    Ok(BackupVerifyReport {
        file_name: file_name_of(&chosen),
        bytes: report.bytes,
        chunks: report.chunks,
        format_version: report.format_version,
        records: report.records.iter().map(|(_table, rows)| rows).sum(),
    })
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
    use super::{BackupError, PasswordSource, file_name_of, suggested_name};
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
    fn every_failure_is_tagged_so_the_interface_matches_rather_than_reads() {
        let encoded = serde_json::to_string(&BackupError::Cancelled).expect("it serialises");

        assert_eq!(encoded, "{\"kind\":\"cancelled\"}");
    }
}
