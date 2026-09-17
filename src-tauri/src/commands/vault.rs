//! The things the WebView may ask about the vault.
//!
//! Everything a person can do with a master password happens through one of these, and
//! nothing else in the application is reachable from the other side of the bridge. Four
//! rules hold across all of them.
//!
//! No key and no plaintext ever crosses back. The return types here are booleans, counts and
//! enumerations; the keys stay in [`crate::session`] and are read through a closure that
//! cannot outlive its lock.
//!
//! Every failure to open the vault is the same failure. A wrong password, a header somebody
//! edited and a flipped bit on a disk all come back as [`VaultError::NotOpened`], because a
//! caller who can tell them apart knows which half of the problem to attack. The one refusal
//! reported separately is the lockout, and that is decided before anything is derived, so it
//! says nothing about the password.
//!
//! A password is used and cleared. It arrives as an owned string, because that is what
//! crossing the bridge produces, and it is wrapped so that the allocation is zeroed when the
//! command returns rather than left in the heap for whatever lands there next.
//!
//! Anything that can change the vault runs one at a time. Two creations racing would each
//! write a header, and the second would make every byte encrypted under the first unreadable
//! with no warning and no way back. The permit that prevents it is taken first and held for
//! the whole command; the order is always that permit, then the derivation, then the two
//! ordinary locks, and never the other way round.

use cairn_crypto::{Argon2Params, CryptoError, VaultHeader};
use cairn_domain::password::{self, PasswordProblem, Strength};
use cairn_domain::session::{
    IdleDecision, InactivityMinutes, InactivityTimeout, backoff_remaining_s, locked_until_us,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::clock::now_us;
use crate::session::{LockReason, UnlockFailure, UnlockOutcome};
use crate::state::AppState;
use crate::storage::Storage;
use crate::vault::VaultCondition;
use crate::vault_file::VaultFileError;

/// Why an operation on the vault did not happen.
///
/// Serialised as a tagged object so that the interface matches on `kind` and reads the
/// numbers beside it, rather than parsing a sentence that will one day be translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[non_exhaustive]
pub enum VaultError {
    /// The vault did not open. Every reason, deliberately indistinguishable.
    #[error("the vault did not open")]
    NotOpened,

    /// Further attempts are refused for a while after repeated failures.
    #[serde(rename_all = "camelCase")]
    #[error("further attempts are refused for {remaining_s} more seconds")]
    LockedOut {
        /// How many seconds are left before another attempt is allowed.
        remaining_s: u32,
    },

    /// There is no vault on this machine.
    #[error("there is no vault on this machine")]
    NoVault,

    /// There is already a vault on this machine, and a second would destroy the first.
    #[error("this machine already has a vault")]
    AlreadyExists,

    /// The vault is closed, and this operation needs it open.
    #[error("the vault is locked")]
    Locked,

    /// The password is shorter than the application accepts.
    #[error("the password has {chars} characters, and at least {min} are needed")]
    PasswordTooShort {
        /// How many characters it has.
        chars: usize,
        /// How many are needed.
        min: usize,
    },

    /// The password is longer than the key derivation accepts.
    #[error("the password is {bytes} bytes, and at most {max} are accepted")]
    PasswordTooLong {
        /// How many bytes of UTF-8 it takes.
        bytes: usize,
        /// How many are accepted.
        max: usize,
    },

    /// The password was refused for a reason this build cannot put into numbers.
    ///
    /// Exists so that a reason added to the policy later cannot become a silent success
    /// here. Nothing produces it today.
    #[error("the password was not accepted")]
    PasswordRejected,

    /// A derivation parameter was outside the range a vault may ask for.
    #[error("{field} is {value}, and the allowed range is {min} to {max}")]
    ParamOutOfRange {
        /// Which field, named as it is in the header layout.
        field: &'static str,
        /// What was asked for.
        value: u32,
        /// The lowest value allowed.
        min: u32,
        /// The highest value allowed.
        max: u32,
    },

    /// The key derivation could not be run on this machine at these parameters.
    ///
    /// A fact about the machine rather than about the password, so it is reported as itself.
    #[error("the key derivation could not be run on this machine")]
    DerivationRefused,

    /// The header could not be written, so nothing was changed.
    #[error("the vault header could not be written, and nothing was changed")]
    Storage,
}

impl From<PasswordProblem> for VaultError {
    fn from(problem: PasswordProblem) -> Self {
        match problem {
            PasswordProblem::TooShort { chars, min } => Self::PasswordTooShort { chars, min },
            PasswordProblem::TooLong { bytes, max } => Self::PasswordTooLong { bytes, max },
            _ => Self::PasswordRejected,
        }
    }
}

impl From<cairn_db::DbError> for VaultError {
    fn from(_error: cairn_db::DbError) -> Self {
        // Collapsed for the same reason as the file errors below. Whether the file could not be
        // created, the schema is newer than this build, or a stored value failed its tag, the
        // answer to the interface is that storage did not work; the difference is in the log,
        // where somebody repairing a machine can reach it.
        Self::Storage
    }
}

impl From<VaultFileError> for VaultError {
    fn from(_error: VaultFileError) -> Self {
        // Collapsed on purpose. The difference between a full disk and a header that would
        // not parse is worth keeping where somebody repairing a machine can reach it, and is
        // an oracle in a value the interface can read.
        Self::Storage
    }
}

/// Maps a failure that happened while deriving, keeping the two that are not about secrets.
///
/// Everything else becomes the single unlock error. The two exceptions say nothing about
/// whether the password was right: one is the length of the string the caller just typed
/// into a field they can see, the other is this machine refusing to allocate.
fn from_derivation(failure: &UnlockFailure) -> VaultError {
    match failure {
        UnlockFailure::Refused(CryptoError::PasswordTooLong { len, max }) => {
            VaultError::PasswordTooLong {
                bytes: *len,
                max: *max,
            }
        }
        UnlockFailure::Refused(CryptoError::Kdf) => VaultError::DerivationRefused,
        _ => VaultError::NotOpened,
    }
}

/// Maps whatever [`Argon2Params::new`] refused.
///
/// It reports nothing but a parameter out of range today. The other arm exists so that a
/// reason added later is refused rather than quietly accepted as valid parameters.
fn from_params(error: &CryptoError) -> VaultError {
    match error {
        CryptoError::ParamOutOfRange {
            field,
            value,
            min,
            max,
        } => VaultError::ParamOutOfRange {
            field,
            value: *value,
            min: *min,
            max: *max,
        },
        _ => VaultError::DerivationRefused,
    }
}

/// What reading the header at startup found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConditionReport {
    /// No header and no copy. This machine has no vault yet.
    NoVaultYet,
    /// The header was readable and nothing had to be done.
    Readable,
    /// The header was unreadable and the copy beside it was used instead.
    RestoredFromBackup,
    /// There is a header and neither it nor the copy beside it can be read.
    ///
    /// The interface must not offer to create a vault here. It offers to put a copy back.
    Unreadable,
}

impl From<VaultCondition> for ConditionReport {
    fn from(condition: VaultCondition) -> Self {
        match condition {
            VaultCondition::NoVaultYet => Self::NoVaultYet,
            VaultCondition::Readable => Self::Readable,
            VaultCondition::RestoredFromBackup => Self::RestoredFromBackup,
            VaultCondition::Unreadable => Self::Unreadable,
        }
    }
}

/// The derivation parameters currently in force.
///
/// Reported because the diagnostics screen shows them and because somebody deciding whether
/// to raise them has to see what they are. None of it is secret: the same three numbers sit
/// in the header of a file an attacker would already have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KdfReport {
    /// Memory in kibibytes.
    pub memory_kib: u32,
    /// Passes over that memory.
    pub passes: u32,
    /// Lanes of parallelism.
    pub lanes: u32,
    /// When these parameters were written, in microseconds since the epoch, UTC.
    pub written_at_us: i64,
}

impl KdfReport {
    /// Describes the parameters a header carries.
    #[must_use]
    pub fn of(header: &VaultHeader) -> Self {
        let params = header.params();

        Self {
            memory_kib: params.memory_kib(),
            passes: params.passes(),
            lanes: params.lanes(),
            written_at_us: header.params_written_at_us(),
        }
    }
}

/// How long the vault may sit idle, in the shape the interface offers.
///
/// A closed set on both sides of the bridge. A number would let a value nobody designed for
/// arrive from the WebView and become the lock policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InactivityChoice {
    /// One minute.
    One,
    /// Five minutes, which is what a new installation uses.
    Five,
    /// Fifteen minutes.
    Fifteen,
    /// Thirty minutes.
    Thirty,
    /// Never locks itself.
    Never,
}

impl From<InactivityTimeout> for InactivityChoice {
    fn from(timeout: InactivityTimeout) -> Self {
        match timeout {
            InactivityTimeout::After(InactivityMinutes::One) => Self::One,
            InactivityTimeout::After(InactivityMinutes::Five) => Self::Five,
            InactivityTimeout::After(InactivityMinutes::Fifteen) => Self::Fifteen,
            InactivityTimeout::After(InactivityMinutes::Thirty) => Self::Thirty,
            InactivityTimeout::Never => Self::Never,
        }
    }
}

impl From<InactivityChoice> for InactivityTimeout {
    fn from(choice: InactivityChoice) -> Self {
        match choice {
            InactivityChoice::One => Self::After(InactivityMinutes::One),
            InactivityChoice::Five => Self::After(InactivityMinutes::Five),
            InactivityChoice::Fifteen => Self::After(InactivityMinutes::Fifteen),
            InactivityChoice::Thirty => Self::After(InactivityMinutes::Thirty),
            InactivityChoice::Never => Self::Never,
        }
    }
}

/// Everything the status is computed from, read while the locks are held.
///
/// Gathered into one value so that the arithmetic below is a pure function of it, and so
/// that the whole status comes from a single look at the state rather than from six, each
/// seeing a slightly different moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusInputs {
    /// Whether this machine has a vault at all.
    pub exists: bool,
    /// Whether it is open right now.
    pub unlocked: bool,
    /// What reading the header at startup found.
    pub condition: VaultCondition,
    /// The parameters in force, absent on a machine with no vault.
    pub kdf: Option<KdfReport>,
    /// How many unlock attempts have failed since the last successful one.
    pub failed_attempts: u32,
    /// Until when attempts are refused, in microseconds since the epoch, UTC.
    pub locked_until_us: i64,
    /// How long the vault may sit idle.
    pub inactivity: InactivityTimeout,
    /// What should happen to the session now.
    pub idle: IdleDecision,
}

/// Everything the interface needs to decide what to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    /// Whether this machine has a vault at all.
    pub exists: bool,
    /// Whether it is open right now.
    pub unlocked: bool,
    /// What reading the header at startup found.
    pub condition: ConditionReport,
    /// The derivation parameters, absent on a machine with no vault.
    pub kdf: Option<KdfReport>,
    /// How many unlock attempts have failed since the last successful one.
    pub failed_attempts: u32,
    /// How many seconds attempts are still refused for. Zero when they are not.
    pub locked_out_for_s: u32,
    /// How long the vault may sit idle.
    pub inactivity: InactivityChoice,
    /// Seconds left before it locks itself, absent when it is closed or never locks.
    pub idle_remaining_s: Option<u32>,
}

impl VaultStatus {
    /// Turns what was read into what the interface is told.
    ///
    /// Pure, so the whole shape of the screen can be tested without a window.
    #[must_use]
    pub fn assemble(inputs: StatusInputs, now_us: i64) -> Self {
        Self {
            exists: inputs.exists,
            unlocked: inputs.unlocked,
            condition: inputs.condition.into(),
            kdf: inputs.kdf,
            failed_attempts: inputs.failed_attempts,
            locked_out_for_s: backoff_remaining_s(inputs.locked_until_us, now_us),
            inactivity: inputs.inactivity.into(),
            // Absence rather than zero, because a number means draw a countdown and a very
            // large number would be drawn.
            idle_remaining_s: match inputs.idle {
                IdleDecision::Wait { remaining_s } => Some(remaining_s),
                IdleDecision::Lock | IdleDecision::NeverLocks => None,
            },
        }
    }
}

/// How strong a password looks, as the bar can honestly show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StrengthReport {
    /// Would not survive an attacker with the file and a word list.
    Weak,
    /// Better than most, and still not much.
    Fair,
    /// Reasonable against somebody who has the file.
    Good,
    /// More than this design needs.
    Strong,
}

impl From<Strength> for StrengthReport {
    fn from(strength: Strength) -> Self {
        match strength {
            Strength::Weak => Self::Weak,
            Strength::Fair => Self::Fair,
            Strength::Good => Self::Good,
            Strength::Strong => Self::Strong,
        }
    }
}

/// Opens the database for a vault that has just been opened, and puts it beside the keys.
///
/// Called at the one moment it can be: after a derivation succeeded, because the key the file is
/// encrypted with hangs off the data key and does not exist before then.
///
/// A failure here closes the vault again rather than leaving it open without storage. Half an
/// open vault is a state every screen would have to ask about, and the honest answer to somebody
/// whose database will not open is that the application did not open.
fn attach_storage(state: &AppState) -> Result<(), VaultError> {
    // The path is copied out first so that the directory is not borrowed across the closure that
    // holds the session lock.
    let directory = state.directory().path().to_path_buf();

    let opened = state
        .session()
        .with_vault(|vault| Storage::open(&directory, vault))
        .ok_or(VaultError::Locked)?;

    match opened {
        Ok(storage) => {
            state.session().attach_storage(storage);
            Ok(())
        }
        Err(error) => {
            state.session().lock();
            Err(error.into())
        }
    }
}

/// Reads the whole status in one pass over the state.
///
/// One pass rather than six, so that every field of the answer describes the same moment.
fn status_of(state: &AppState, now: i64) -> VaultStatus {
    let vault = state.vault();
    let session = state.session();

    VaultStatus::assemble(
        StatusInputs {
            exists: vault.exists(),
            unlocked: session.is_unlocked(),
            condition: vault.condition(),
            kdf: vault.header().map(KdfReport::of),
            failed_attempts: vault.failed_attempts(),
            locked_until_us: vault.locked_until_us(),
            inactivity: session.timeout(),
            idle: session.idle_decision(now),
        },
        now,
    )
}

/// Creates the vault and opens it.
///
/// Separate from the command so that the whole sequence can be driven by a test without a
/// window. The command below is the three lines that turn a Tauri state guard into this.
///
/// # Errors
///
/// Returns [`VaultError::AlreadyExists`] on a machine that already has one, the password
/// length errors if the password is not acceptable, [`VaultError::ParamOutOfRange`] for
/// parameters a vault may not ask for, and [`VaultError::Storage`] if the header could not be
/// written, in which case no vault was created and no key was kept.
pub async fn create(
    state: &AppState,
    password: Zeroizing<String>,
    params: Argon2Params,
    now: i64,
) -> Result<VaultStatus, VaultError> {
    password::validate(&password)?;

    // Held for the whole operation. Two creations racing would each write a header, and the
    // second would make everything encrypted under the first unreadable.
    let _operation = state.begin_vault_operation().await;

    if state.vault().exists() {
        return Err(VaultError::AlreadyExists);
    }

    let header = state
        .session()
        .replace_with(
            move || {
                cairn_crypto::create(&password, params, now).map(|(header, vault)| (vault, header))
            },
            now,
        )
        .await
        .map_err(|failure| from_derivation(&failure))?;

    if let Err(error) = state.vault().install(header) {
        // A machine that could not write its header does not have a vault, so it must not be
        // left holding the keys to one.
        state.session().lock();
        return Err(error.into());
    }

    attach_storage(state)?;

    Ok(status_of(state, now))
}

/// Opens the vault with its master password.
///
/// # Errors
///
/// Returns [`VaultError::NoVault`] on a machine with no vault, [`VaultError::LockedOut`] while
/// attempts are still being refused, and [`VaultError::NotOpened`] for every reason the vault
/// did not open.
pub async fn unlock(
    state: &AppState,
    password: Zeroizing<String>,
    now: i64,
) -> Result<VaultStatus, VaultError> {
    // Held so that reading the count of failed attempts, deriving, and writing the new count
    // are one operation. Without it two attempts could read the same count and record it
    // twice, which hands back an attempt every time.
    let _operation = state.begin_vault_operation().await;

    let Some(header) = state.vault().header().cloned() else {
        return Err(VaultError::NoVault);
    };

    // Decided before anything is derived, so the refusal says nothing about the password.
    let remaining_s = backoff_remaining_s(state.vault().locked_until_us(), now);
    if remaining_s > 0 {
        return Err(VaultError::LockedOut { remaining_s });
    }

    let outcome = state
        .session()
        .unlock_with(move || cairn_crypto::unlock(&header, &password), now)
        .await;

    match outcome {
        Ok(UnlockOutcome::Opened) => {
            state.vault().record_attempt(0, 0)?;
            attach_storage(state)?;
            Ok(status_of(state, now))
        }
        // The vault is open, and it is open for everything in this process: there is no
        // second identity here to authenticate, so there is nothing left to check. The count
        // is not reset, because this call proved nothing about its own password.
        Ok(UnlockOutcome::AlreadyOpen) => Ok(status_of(state, now)),
        // A thread of this machine going away is not somebody guessing. Charging it against
        // the attempts would eventually lock an owner out of their own vault.
        Err(UnlockFailure::Interrupted) => Err(VaultError::NotOpened),
        Err(failure) => {
            let failed = state.vault().failed_attempts().saturating_add(1);
            // Recorded before the failure is reported, so that closing the window between the
            // two does not hand back an attempt. A write that fails costs an attacker nothing
            // they did not already have: this count lives in the part of the header anybody
            // holding the file can edit anyway.
            let _recorded = state
                .vault()
                .record_attempt(failed, locked_until_us(failed, now));

            Err(from_derivation(&failure))
        }
    }
}

/// Changes the master password, keeping every stored byte as it is.
///
/// # Errors
///
/// Returns [`VaultError::NoVault`], the password length errors for the new password, and
/// [`VaultError::NotOpened`] if the current password is not the right one.
/// [`VaultError::Storage`] means the header was not replaced and the old password still
/// opens the vault.
pub async fn change_password(
    state: &AppState,
    current: Zeroizing<String>,
    new: Zeroizing<String>,
    now: i64,
) -> Result<VaultStatus, VaultError> {
    password::validate(&new)?;

    let _operation = state.begin_vault_operation().await;

    let Some(header) = state.vault().header().cloned() else {
        return Err(VaultError::NoVault);
    };

    let replacement = state
        .session()
        .replace_with(
            move || {
                cairn_crypto::change_password(&header, &current, &new, now)
                    .map(|(header, vault)| (vault, header))
            },
            now,
        )
        .await
        .map_err(|failure| from_derivation(&failure))?;

    state.vault().rewrite(replacement)?;

    Ok(status_of(state, now))
}

/// Changes the Argon2id parameters, keeping the password and every stored byte as they are.
///
/// # Errors
///
/// The same as [`change_password`], plus [`VaultError::ParamOutOfRange`] for parameters a
/// vault may not ask for.
pub async fn change_kdf_params(
    state: &AppState,
    password: Zeroizing<String>,
    params: Argon2Params,
    now: i64,
) -> Result<VaultStatus, VaultError> {
    let _operation = state.begin_vault_operation().await;

    let Some(header) = state.vault().header().cloned() else {
        return Err(VaultError::NoVault);
    };

    let replacement = state
        .session()
        .replace_with(
            move || {
                cairn_crypto::change_kdf_params(&header, &password, params, now)
                    .map(|(header, vault)| (vault, header))
            },
            now,
        )
        .await
        .map_err(|failure| from_derivation(&failure))?;

    state.vault().rewrite(replacement)?;

    Ok(status_of(state, now))
}

/// Changes how long the vault may sit idle before it closes itself.
///
/// Takes effect at once and lasts as long as the process does. There is nowhere to keep it
/// yet: the header is authenticated cryptographic material and has no room for a preference,
/// and inventing a settings file here would mean inventing a format that the phase which
/// brings storage is going to replace. So it returns to five minutes on the next start, which
/// is the safe direction for a setting to forget itself in.
pub fn set_inactivity(state: &AppState, choice: InactivityChoice, now: i64) -> VaultStatus {
    state.session().set_timeout(choice.into(), now);

    status_of(state, now)
}

/// Reports activity inside the window, and says how long is left.
///
/// # Errors
///
/// Returns [`VaultError::Locked`] if the vault is closed.
pub fn heartbeat(state: &AppState, now: i64) -> Result<VaultStatus, VaultError> {
    if !state.session().is_unlocked() {
        return Err(VaultError::Locked);
    }
    state.session().note_activity(now);

    Ok(status_of(state, now))
}

/// Everything the interface needs to decide what to draw.
#[tauri::command]
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn vault_status(state: tauri::State<'_, AppState>) -> VaultStatus {
    status_of(&state, now_us())
}

/// How strong a password looks, without it being stored or used for anything.
///
/// A command rather than an estimate computed in the WebView, because the alternative puts
/// several megabytes of word list into the bundle and evaluates the master password in
/// JavaScript. The string is cleared before this returns.
#[tauri::command]
#[must_use]
pub fn password_strength(password: String) -> StrengthReport {
    let password = Zeroizing::new(password);

    password::strength(&password).into()
}

/// Creates the vault and opens it.
///
/// # Errors
///
/// See [`create`].
#[tauri::command]
pub async fn vault_create(
    state: tauri::State<'_, AppState>,
    password: String,
    memory_kib: u32,
    passes: u32,
    lanes: u32,
) -> Result<VaultStatus, VaultError> {
    // Refused before the permit is taken, because nothing about a parameter out of range
    // needs to wait for whatever else is running.
    let params =
        Argon2Params::new(memory_kib, passes, lanes).map_err(|error| from_params(&error))?;

    create(&state, Zeroizing::new(password), params, now_us()).await
}

/// Opens the vault with its master password.
///
/// # Errors
///
/// See [`unlock`].
#[tauri::command]
pub async fn vault_unlock(
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<VaultStatus, VaultError> {
    unlock(&state, Zeroizing::new(password), now_us()).await
}

/// Closes the vault, clearing every key in this process.
#[tauri::command]
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn vault_lock(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> VaultStatus {
    // Through the window module rather than through the session, because closing the vault
    // and telling the interface are one thing: a lock nobody was told about leaves the screen
    // drawing what it had.
    crate::window::lock_and_announce(&app, LockReason::Requested);

    status_of(&state, now_us())
}

/// Changes the master password, keeping every stored byte as it is.
///
/// # Errors
///
/// See [`change_password`].
#[tauri::command]
pub async fn vault_change_password(
    state: tauri::State<'_, AppState>,
    current: String,
    new: String,
) -> Result<VaultStatus, VaultError> {
    change_password(
        &state,
        Zeroizing::new(current),
        Zeroizing::new(new),
        now_us(),
    )
    .await
}

/// Changes the Argon2id parameters, keeping the password and every stored byte as they are.
///
/// # Errors
///
/// See [`change_kdf_params`].
#[tauri::command]
pub async fn vault_change_kdf_params(
    state: tauri::State<'_, AppState>,
    password: String,
    memory_kib: u32,
    passes: u32,
    lanes: u32,
) -> Result<VaultStatus, VaultError> {
    let params =
        Argon2Params::new(memory_kib, passes, lanes).map_err(|error| from_params(&error))?;

    change_kdf_params(&state, Zeroizing::new(password), params, now_us()).await
}

/// Reports keyboard or mouse activity inside the window, and says how long is left.
///
/// The only thing that puts the inactivity timer back to the beginning. System activity does
/// not count and is never asked about: somebody typing in another application is not somebody
/// using this one, and a vault that stays open because a different program is busy is a vault
/// that stays open all day.
///
/// # Errors
///
/// Returns [`VaultError::Locked`] if the vault is closed, so that a frontend which kept
/// beating after a lock is told to stop rather than quietly ignored.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn session_heartbeat(state: tauri::State<'_, AppState>) -> Result<VaultStatus, VaultError> {
    heartbeat(&state, now_us())
}

/// Changes how long the vault may sit idle before it closes itself.
///
/// The choice is a closed set on both sides of the bridge, so a period nobody designed for
/// cannot arrive from the WebView and become the lock policy.
#[tauri::command]
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn session_set_inactivity(
    state: tauri::State<'_, AppState>,
    inactivity: InactivityChoice,
) -> VaultStatus {
    set_inactivity(&state, inactivity, now_us())
}

#[cfg(test)]
mod tests {
    use cairn_crypto::CryptoError;
    use cairn_domain::password::{MIN_PASSWORD_CHARS, PasswordProblem};
    use cairn_domain::session::{IdleDecision, InactivityMinutes, InactivityTimeout};

    use super::{
        ConditionReport, InactivityChoice, KdfReport, StatusInputs, StrengthReport, VaultError,
        VaultStatus, from_derivation, from_params,
    };
    use crate::session::UnlockFailure;
    use crate::vault::VaultCondition;
    use crate::vault_file::VaultFileError;

    /// A moment in the middle of the range.
    const NOW_US: i64 = 1_700_000_000_000_000;

    fn a_kdf_report() -> KdfReport {
        KdfReport {
            memory_kib: 65_536,
            passes: 3,
            lanes: 1,
            written_at_us: NOW_US,
        }
    }

    /// A machine with no vault, as the starting point for the cases below.
    fn nothing_there() -> StatusInputs {
        StatusInputs {
            exists: false,
            unlocked: false,
            condition: VaultCondition::NoVaultYet,
            kdf: None,
            failed_attempts: 0,
            locked_until_us: 0,
            inactivity: InactivityTimeout::default(),
            idle: IdleDecision::Lock,
        }
    }

    /// A vault that exists and is open.
    fn open_vault() -> StatusInputs {
        StatusInputs {
            exists: true,
            unlocked: true,
            condition: VaultCondition::Readable,
            kdf: Some(a_kdf_report()),
            ..nothing_there()
        }
    }

    #[test]
    fn every_way_a_vault_fails_to_open_is_the_same_way() {
        // The assertion this whole file exists to make true. A caller who can tell a wrong
        // password from an edited header knows which half of the problem to work on.
        for failure in [
            UnlockFailure::Refused(CryptoError::Open),
            UnlockFailure::Refused(CryptoError::HeaderMagic),
            UnlockFailure::Refused(CryptoError::HeaderSize { len: 7 }),
            UnlockFailure::Refused(CryptoError::Entropy),
            UnlockFailure::Interrupted,
        ] {
            assert_eq!(
                from_derivation(&failure),
                VaultError::NotOpened,
                "{failure:?} was distinguishable from a wrong password"
            );
        }
    }

    #[test]
    fn the_two_failures_that_say_nothing_about_the_password_are_reported_as_themselves() {
        // Neither is an oracle. The length is of the string the caller just typed into a
        // field they can see, and the refusal to allocate is a fact about this machine.
        assert_eq!(
            from_derivation(&UnlockFailure::Refused(CryptoError::PasswordTooLong {
                len: 9_000,
                max: 1_024,
            })),
            VaultError::PasswordTooLong {
                bytes: 9_000,
                max: 1_024,
            }
        );
        assert_eq!(
            from_derivation(&UnlockFailure::Refused(CryptoError::Kdf)),
            VaultError::DerivationRefused
        );
    }

    #[test]
    fn a_storage_failure_never_says_which_storage_failure_it_was() {
        // A full disk and a header that would not parse are worth telling apart when repairing
        // a machine, and are an
        // oracle in a return value.
        assert_eq!(
            VaultError::from(VaultFileError::BackupNotVerified),
            VaultError::Storage
        );
        assert_eq!(
            VaultError::from(VaultFileError::Malformed(CryptoError::HeaderMagic)),
            VaultError::Storage
        );
    }

    #[test]
    fn a_parameter_out_of_range_says_which_one_and_what_the_bounds_are() {
        // Not a secret: the same three numbers sit in the header of a file an attacker would
        // already have, and somebody who mistyped needs to know what to type instead.
        assert_eq!(
            from_params(&CryptoError::ParamOutOfRange {
                field: "memory_kib",
                value: 1,
                min: 32_768,
                max: 4_194_304,
            }),
            VaultError::ParamOutOfRange {
                field: "memory_kib",
                value: 1,
                min: 32_768,
                max: 4_194_304,
            }
        );
        assert_eq!(
            from_params(&CryptoError::Open),
            VaultError::DerivationRefused,
            "a reason added to the parameter check later became valid parameters"
        );
    }

    #[test]
    fn a_password_problem_keeps_its_numbers_so_the_interface_can_say_what_is_missing() {
        assert_eq!(
            VaultError::from(PasswordProblem::TooShort {
                chars: 4,
                min: MIN_PASSWORD_CHARS,
            }),
            VaultError::PasswordTooShort {
                chars: 4,
                min: MIN_PASSWORD_CHARS,
            }
        );
        assert_eq!(
            VaultError::from(PasswordProblem::TooLong {
                bytes: 9_000,
                max: 1_024,
            }),
            VaultError::PasswordTooLong {
                bytes: 9_000,
                max: 1_024,
            }
        );
    }

    #[test]
    fn a_machine_with_no_vault_reports_nothing_rather_than_zeroes_that_look_like_facts() {
        let status = VaultStatus::assemble(nothing_there(), NOW_US);

        assert!(!status.exists);
        assert!(!status.unlocked);
        assert_eq!(status.condition, ConditionReport::NoVaultYet);
        assert_eq!(status.kdf, None);
        assert_eq!(status.locked_out_for_s, 0);
        assert_eq!(status.idle_remaining_s, None);
        assert_eq!(status.inactivity, InactivityChoice::Five);
    }

    #[test]
    fn an_open_vault_reports_how_long_it_has_before_it_closes_itself() {
        let status = VaultStatus::assemble(
            StatusInputs {
                idle: IdleDecision::Wait { remaining_s: 42 },
                ..open_vault()
            },
            NOW_US,
        );

        assert!(status.exists);
        assert!(status.unlocked);
        assert_eq!(status.idle_remaining_s, Some(42));
        assert_eq!(status.kdf, Some(a_kdf_report()));
    }

    #[test]
    fn a_vault_that_never_locks_reports_no_countdown_rather_than_a_large_one() {
        // The difference matters to the interface: a number means draw a countdown, and
        // absence means do not. A very large number would be drawn.
        let status = VaultStatus::assemble(
            StatusInputs {
                inactivity: InactivityTimeout::Never,
                idle: IdleDecision::NeverLocks,
                ..open_vault()
            },
            NOW_US,
        );

        assert_eq!(status.idle_remaining_s, None);
        assert_eq!(status.inactivity, InactivityChoice::Never);
    }

    #[test]
    fn a_lockout_still_running_is_reported_as_the_seconds_that_are_left() {
        let status = VaultStatus::assemble(
            StatusInputs {
                unlocked: false,
                failed_attempts: 4,
                locked_until_us: NOW_US + 8_000_000,
                ..open_vault()
            },
            NOW_US,
        );

        assert_eq!(status.failed_attempts, 4);
        assert_eq!(status.locked_out_for_s, 8);
    }

    #[test]
    fn a_lockout_that_has_passed_is_not_reported_at_all() {
        let status = VaultStatus::assemble(
            StatusInputs {
                unlocked: false,
                failed_attempts: 4,
                locked_until_us: NOW_US - 1,
                ..open_vault()
            },
            NOW_US,
        );

        assert_eq!(status.locked_out_for_s, 0);
        assert_eq!(
            status.failed_attempts, 4,
            "the count is kept after the wait ends, because the next failure doubles from it"
        );
    }

    #[test]
    fn a_restored_copy_is_reported_so_somebody_can_be_told_about_it() {
        let status = VaultStatus::assemble(
            StatusInputs {
                condition: VaultCondition::RestoredFromBackup,
                ..open_vault()
            },
            NOW_US,
        );

        assert_eq!(status.condition, ConditionReport::RestoredFromBackup);
    }

    #[test]
    fn every_inactivity_choice_survives_the_round_trip_in_both_directions() {
        for choice in [
            InactivityChoice::One,
            InactivityChoice::Five,
            InactivityChoice::Fifteen,
            InactivityChoice::Thirty,
            InactivityChoice::Never,
        ] {
            let timeout: InactivityTimeout = choice.into();
            assert_eq!(InactivityChoice::from(timeout), choice);
        }

        for timeout in [
            InactivityTimeout::After(InactivityMinutes::One),
            InactivityTimeout::After(InactivityMinutes::Five),
            InactivityTimeout::After(InactivityMinutes::Fifteen),
            InactivityTimeout::After(InactivityMinutes::Thirty),
            InactivityTimeout::Never,
        ] {
            let choice: InactivityChoice = timeout.into();
            assert_eq!(InactivityTimeout::from(choice), timeout);
        }
    }

    #[test]
    fn the_error_serialises_to_a_tagged_object_the_interface_can_match_on() {
        // A sentence would be translated one day and the interface would stop recognising
        // it. The tag is what it matches on, and the numbers sit beside it.
        assert_eq!(
            serde_json::to_string(&VaultError::LockedOut { remaining_s: 8 })
                .expect("the error serialises"),
            r#"{"kind":"lockedOut","remainingS":8}"#
        );
        assert_eq!(
            serde_json::to_string(&VaultError::NotOpened).expect("the error serialises"),
            r#"{"kind":"notOpened"}"#
        );
    }

    #[test]
    fn the_status_serialises_with_the_names_the_interface_expects() {
        let encoded = serde_json::to_value(VaultStatus::assemble(open_vault(), NOW_US))
            .expect("the status serialises");

        for name in [
            "exists",
            "unlocked",
            "condition",
            "kdf",
            "failedAttempts",
            "lockedOutForS",
            "inactivity",
            "idleRemainingS",
        ] {
            assert!(
                encoded.get(name).is_some(),
                "the status has no field called {name}"
            );
        }
    }

    #[test]
    fn strength_serialises_to_the_four_names_the_bar_draws() {
        for (report, name) in [
            (StrengthReport::Weak, r#""weak""#),
            (StrengthReport::Fair, r#""fair""#),
            (StrengthReport::Good, r#""good""#),
            (StrengthReport::Strong, r#""strong""#),
        ] {
            assert_eq!(
                serde_json::to_string(&report).expect("the report serialises"),
                name
            );
        }
    }
}
