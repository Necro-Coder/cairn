//! The open vault, for as long as it is open.
//!
//! One place holds the keys while the application is running, and this is it. Everything
//! about that is deliberate: the keys are behind a lock, they are reached through a closure
//! rather than handed out, dropping them is what clears them, and there is no accessor that
//! returns one. Nothing here can be reached from the WebView except through the commands.
//!
//! Two properties are worth naming because they are not obvious from the shape of the code.
//!
//! Only one derivation runs at a time. Argon2id is expensive on purpose, which means a
//! handful of concurrent unlocks is a way to make this machine unusable using nothing but
//! the interface it already offers. A second reason matters more: the second of two racing
//! unlocks finds the vault already open and returns without deriving anything, so a
//! frontend that sends the same request twice pays for it once.
//!
//! The derivation never runs on the thread that draws. It is handed to the blocking pool, so
//! a tenth of a second of hashing does not become a tenth of a second of frozen window.
//!
//! What is not here: the header, the file it lives in, and the count of failed attempts.
//! Those outlive the process and belong to the layer that owns the disk.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use cairn_crypto::{CryptoError, UnlockedVault};
use cairn_domain::session::{IdleDecision, InactivityTimeout, focus_grace_elapsed, idle_decision};

use crate::storage::Storage;

/// What an unlock did.
///
/// Two outcomes rather than one, because a caller that asked for an unlock and got one
/// somebody else had already performed should be able to tell. It is also what the
/// concurrency test asserts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockOutcome {
    /// This call derived the key and opened the vault.
    Opened,
    /// The vault was already open, so nothing was derived.
    AlreadyOpen,
}

/// Why an unlock did not happen.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UnlockFailure {
    /// The derivation ran and the vault did not open.
    ///
    /// The one the caller may count as a failed attempt.
    #[error("the vault did not open")]
    Refused(#[source] CryptoError),

    /// The derivation did not finish, because the thread running it went away.
    ///
    /// Distinct from a refusal on purpose. Counting this as a wrong password would punish
    /// somebody for a fault of the machine, and after enough of them lock them out of their
    /// own vault for five minutes at a time.
    #[error("the derivation did not finish")]
    Interrupted,
}

/// Why the vault closed.
///
/// Carried by the one event the interface listens for, so that the screen it draws can say
/// what happened rather than appearing for no visible reason. None of it is secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LockReason {
    /// Nobody touched the window for as long as the setting allows.
    Inactivity,
    /// The window was away from the front for longer than the grace period.
    FocusLost,
    /// The window was minimised, which locks at once.
    Minimised,
    /// Somebody asked for it.
    Requested,
}

/// Everything that has to change together when the vault opens or closes.
#[derive(Debug)]
struct SessionState {
    /// The keys, present exactly while the vault is open. Dropping this clears them.
    vault: Option<UnlockedVault>,
    /// The open database and the device identifier, present while the vault is.
    ///
    /// Beside the keys rather than beside the window, because the two have the same lifetime.
    /// A connection that outlived the keys would be a handle to the file with the key inside it
    /// after the vault was supposed to be shut, which is the one thing locking exists to stop.
    storage: Option<Storage>,
    /// The last moment there was keyboard or mouse activity inside the window.
    last_activity_us: i64,
    /// How long the vault may sit idle before it closes itself.
    timeout: InactivityTimeout,
    /// When the window lost focus, while it still has not come back.
    ///
    /// An option rather than a moment and a flag, because a window that is in front and a
    /// window that lost focus at the beginning of time are not two shades of the same thing.
    focus_lost_at_us: Option<i64>,
    /// The import that has been read and is waiting to be confirmed, if there is one.
    ///
    /// Here, and not in a store of its own, so that it dies when the vault does. A staging
    /// database is a complete copy of somebody's vault written from a file they were handed; a
    /// confirmation that survived a lock would be a way to replace the live vault of whoever
    /// unlocks next.
    import: Option<ImportTicket>,
}

/// An import that has been read and verified and is waiting for a yes or a no.
#[derive(Debug, Clone)]
pub struct ImportTicket {
    /// The random word the interface has to give back to confirm this one import.
    ///
    /// Not a name for the file and not an index. It is a secret handed to the screen that
    /// asked, so that a second window, or anything else that reaches the bridge, cannot
    /// confirm a replacement it did not prepare.
    pub token: String,
    /// Where the staging database is.
    pub staging: PathBuf,
    /// How many rows of each table the file held, in the order they appeared.
    ///
    /// Carried rather than counted again afterwards, because after the swap the same question
    /// has a different meaning: this is what the backup said it held, and the report exists to
    /// say that those are what arrived.
    pub records: Vec<(String, u64)>,
    /// The moment after which this is no longer good.
    pub expires_us: i64,
}

impl ImportTicket {
    /// Whether it is still good at that moment.
    #[must_use]
    pub fn is_live_at(&self, now_us: i64) -> bool {
        now_us < self.expires_us
    }
}

/// There is already an import waiting to be confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("an import is already waiting to be confirmed")]
pub struct ImportAlreadyWaiting;

/// The vault as this process holds it.
#[derive(Debug)]
pub struct Session {
    /// The keys and the timer, together, because a vault that is open and an activity
    /// moment that belongs to a previous one is a vault that locks at the wrong time.
    state: Mutex<SessionState>,
    /// The permit that makes derivation one at a time.
    ///
    /// Asynchronous rather than ordinary, because it is held across the await that runs the
    /// derivation on another thread, and a thread of the runtime blocked on an ordinary lock
    /// is a thread that is not drawing the window.
    derivation: tokio::sync::Mutex<()>,
}

impl Session {
    /// A locked session with the inactivity policy given.
    #[must_use]
    pub fn new(timeout: InactivityTimeout, now_us: i64) -> Self {
        Self {
            state: Mutex::new(SessionState {
                vault: None,
                storage: None,
                last_activity_us: now_us,
                timeout,
                focus_lost_at_us: None,
                import: None,
            }),
            derivation: tokio::sync::Mutex::new(()),
        }
    }

    /// Whether the vault is open.
    #[must_use]
    pub fn is_unlocked(&self) -> bool {
        self.state().vault.is_some()
    }

    /// How long the vault may sit idle.
    #[must_use]
    pub fn timeout(&self) -> InactivityTimeout {
        self.state().timeout
    }

    /// Changes how long the vault may sit idle.
    ///
    /// The timer starts again from now rather than from whenever the last key was pressed,
    /// because choosing a shorter period should not lock the vault in the same instant.
    pub fn set_timeout(&self, timeout: InactivityTimeout, now_us: i64) {
        let mut state = self.state();
        state.timeout = timeout;
        state.last_activity_us = now_us;
    }

    /// Records keyboard or mouse activity inside the window.
    ///
    /// Ignored while the vault is locked. There is nothing to keep open, and accepting it
    /// would let a frontend that keeps beating after a lock decide when the next timer
    /// starts.
    pub fn note_activity(&self, now_us: i64) {
        let mut state = self.state();
        if state.vault.is_some() {
            state.last_activity_us = now_us;
        }
    }

    /// What should happen to the session at `now_us`.
    ///
    /// A locked vault is reported as locking now, which is true and saves every caller a
    /// separate question.
    #[must_use]
    pub fn idle_decision(&self, now_us: i64) -> IdleDecision {
        let state = self.state();
        if state.vault.is_none() {
            return IdleDecision::Lock;
        }

        idle_decision(state.timeout, state.last_activity_us, now_us)
    }

    /// Records that the window is no longer in front.
    ///
    /// Only the first of a run is kept. A platform that reports losing focus twice without
    /// reporting it coming back must not restart the grace period each time, or a window that
    /// is never coming back would never lock.
    pub fn note_focus_lost(&self, now_us: i64) {
        let mut state = self.state();
        if state.focus_lost_at_us.is_none() {
            state.focus_lost_at_us = Some(now_us);
        }
    }

    /// Records that the window is in front again, cancelling the countdown.
    pub fn note_focus_gained(&self) {
        self.state().focus_lost_at_us = None;
    }

    /// Why the vault should close now, or `None` if it should stay open.
    ///
    /// Inactivity is checked before focus so that a session which is overdue on both is
    /// reported as the one the person can do something about.
    #[must_use]
    pub fn due_to_lock(&self, now_us: i64) -> Option<LockReason> {
        let state = self.state();
        state.vault.as_ref()?;

        if idle_decision(state.timeout, state.last_activity_us, now_us) == IdleDecision::Lock {
            return Some(LockReason::Inactivity);
        }
        if state
            .focus_lost_at_us
            .is_some_and(|lost_at| focus_grace_elapsed(lost_at, now_us))
        {
            return Some(LockReason::FocusLost);
        }

        None
    }

    /// Closes the vault if something says it should, answering with what that was.
    ///
    /// Answering rather than announcing, because the event that tells the interface belongs
    /// to the layer that has a window to send it to.
    pub fn lock_if_due(&self, now_us: i64) -> Option<LockReason> {
        let reason = self.due_to_lock(now_us)?;
        self.lock();

        Some(reason)
    }

    /// Closes the vault, answering whether it was open.
    ///
    /// Closing is dropping. The key clears itself on the way out, and there is no copy of it
    /// anywhere else in this process to clear separately. The focus countdown is cleared with
    /// it, so that opening the vault again does not inherit one from before.
    pub fn lock(&self) -> bool {
        let mut state = self.state();
        state.focus_lost_at_us = None;

        // The waiting import goes with it, and its staging database goes with that. A
        // confirmation that survived a lock would let whoever unlocks next have their vault
        // replaced by a file somebody else chose, and the file itself is a complete copy of a
        // vault that nobody is going to be asked about again.
        if let Some(waiting) = state.import.take() {
            let _removed = cairn_db::backup::import::discard(&waiting.staging);
        }

        // The database goes first. It is the thing that holds a file handle, and a failure to
        // let that handle go must not stop the keys being dropped: a vault that stayed open
        // because a statement was still alive would be the worst possible answer to a lock.
        if let Some(storage) = state.storage.take() {
            let _released = storage.close();
        }

        state.vault.take().is_some()
    }

    /// Puts the open database and the device identifier where the keys are.
    ///
    /// Replaces whatever was there, closing it. Reaching this with something already attached
    /// means a second unlock opened a second connection, and keeping the first would leave a
    /// handle nothing can ever close.
    pub fn attach_storage(&self, storage: Storage) {
        let mut state = self.state();
        if let Some(previous) = state.storage.replace(storage) {
            let _released = previous.close();
        }
    }

    /// Holds an import until somebody confirms it, refusing if one is already waiting.
    ///
    /// One at a time, because there is one staging database and its name is fixed. A second
    /// preparation while the first is still good is refused rather than allowed to overwrite
    /// it: two files, both half written, would be worse than either.
    ///
    /// An expired one is not in the way. It is handed back so the caller can remove the file
    /// it left behind, because nothing else is ever going to ask about it.
    ///
    /// # Errors
    ///
    /// Returns [`ImportAlreadyWaiting`] if one is, in which case it is left exactly where it
    /// was and nothing about it has changed.
    pub fn hold_import(
        &self,
        ticket: ImportTicket,
        now_us: i64,
    ) -> Result<Option<ImportTicket>, ImportAlreadyWaiting> {
        let mut state = self.state();

        match state.import.take() {
            Some(waiting) if waiting.is_live_at(now_us) => {
                state.import = Some(waiting);
                Err(ImportAlreadyWaiting)
            }
            expired => {
                state.import = Some(ticket);
                Ok(expired)
            }
        }
    }

    /// Takes the waiting import, if the word given is the one it is waiting for.
    ///
    /// Single use, and only on a match. A word that is not the right one leaves the ticket
    /// exactly where it was: taking it anyway would mean anything that can reach the bridge
    /// could throw away a restore somebody spent two minutes preparing, by guessing once.
    #[must_use]
    pub fn take_import(&self, token: &str, now_us: i64) -> Option<ImportTicket> {
        let mut state = self.state();

        let matches = state
            .import
            .as_ref()
            .is_some_and(|waiting| waiting.token == token && waiting.is_live_at(now_us));

        if matches { state.import.take() } else { None }
    }

    /// Takes the waiting import whatever its word is, for the caller that is giving up on it.
    #[must_use]
    pub fn abandon_import(&self) -> Option<ImportTicket> {
        self.state().import.take()
    }

    /// Takes the open database out, leaving the keys where they are.
    ///
    /// For one caller: the restore, which has to close the file so it can be replaced and then
    /// open a new one in its place. Between the two the session has keys and no database, which
    /// [`Session::with_open`] already reports as closed, so nothing can read or write in that
    /// gap — which is exactly what should happen while the file underneath is being swapped.
    ///
    /// Whoever takes it owns it. A caller that drops it on the floor leaves a handle nothing
    /// can close, so the restore either puts one back or locks the session.
    #[must_use]
    pub fn take_storage(&self) -> Option<Storage> {
        self.state().storage.take()
    }

    /// Reads something out of the open database without the database leaving the lock.
    ///
    /// Answers `None` while the vault is closed, which is the same shape as
    /// [`Session::with_vault`] and for the same reason: a borrow that ends with the closure
    /// cannot be kept somewhere that outlives the lock.
    #[must_use]
    pub fn with_storage<T>(&self, read: impl FnOnce(&Storage) -> T) -> Option<T> {
        self.state().storage.as_ref().map(read)
    }

    /// Runs something that needs both the keys and the open database, under one lock.
    ///
    /// Every repository call needs the pair: the database to run the statement against, and the
    /// key to seal or open the encrypted columns with. Taking them in two calls would leave a
    /// gap where the vault could close between the two, and the second half would then be a
    /// statement against a connection that has just been shut.
    ///
    /// Answers `None` while the vault is closed, which is the shape the other two have.
    #[must_use]
    pub fn with_open<T>(&self, work: impl FnOnce(&UnlockedVault, &Storage) -> T) -> Option<T> {
        let state = self.state();
        match (state.vault.as_ref(), state.storage.as_ref()) {
            (Some(vault), Some(storage)) => Some(work(vault, storage)),
            // Half of a session, which is the state between the keys arriving and the database
            // opening, and the state a failed attach leaves for the moment before it locks.
            // Reported as closed, because nothing can be read or written in it.
            _ => None,
        }
    }

    /// Runs the derivation and opens the vault with what it produced.
    ///
    /// The derivation is a closure rather than a call to the cryptographic crate so that
    /// this file owns the concurrency and nothing else: what a derivation is stays the
    /// business of the caller, and a test can count how many times one happened.
    ///
    /// # Errors
    ///
    /// Returns [`UnlockFailure::Refused`] carrying whatever the derivation reported, and
    /// [`UnlockFailure::Interrupted`] if the thread running it did not come back.
    pub async fn unlock_with<F>(
        &self,
        derive: F,
        now_us: i64,
    ) -> Result<UnlockOutcome, UnlockFailure>
    where
        F: FnOnce() -> Result<UnlockedVault, CryptoError> + Send + 'static,
    {
        // Taken before the vault is looked at rather than after, so that the second of two
        // racing unlocks waits here and then finds the work already done. Checking first and
        // then taking the permit would let both decide to derive.
        let _permit = self.derivation.lock().await;

        if self.is_unlocked() {
            return Ok(UnlockOutcome::AlreadyOpen);
        }

        self.install(|| derive().map(|vault| (vault, ())), now_us)
            .await?;

        Ok(UnlockOutcome::Opened)
    }

    /// Runs a derivation that produces new keys and something else, and installs both.
    ///
    /// What creating a vault and rewriting its header both need. They differ from an unlock
    /// in two ways: the vault being open already is no reason to skip the work, since the
    /// work is what produces the header that has to be written, and there is a second value
    /// to carry back out, which is that header.
    ///
    /// The keys are replaced rather than compared. The password was checked by the
    /// derivation itself: a rewrap that produced keys at all produced them from the right
    /// one.
    ///
    /// # Errors
    ///
    /// The same two as [`Session::unlock_with`].
    pub async fn replace_with<F, T>(&self, derive: F, now_us: i64) -> Result<T, UnlockFailure>
    where
        F: FnOnce() -> Result<(UnlockedVault, T), CryptoError> + Send + 'static,
        T: Send + 'static,
    {
        let _permit = self.derivation.lock().await;

        self.install(derive, now_us).await
    }

    /// Runs the derivation off the drawing thread and puts what it produced in the state.
    ///
    /// Private, and expects the permit to be held by the caller: the two public entry points
    /// differ only in what they do before this, and sharing the body is what keeps the
    /// blocking pool and the moment the timer restarts from being decided twice.
    async fn install<F, T>(&self, derive: F, now_us: i64) -> Result<T, UnlockFailure>
    where
        F: FnOnce() -> Result<(UnlockedVault, T), CryptoError> + Send + 'static,
        T: Send + 'static,
    {
        let (vault, carried) = tauri::async_runtime::spawn_blocking(derive)
            .await
            .map_err(|_joining| UnlockFailure::Interrupted)?
            .map_err(UnlockFailure::Refused)?;

        let mut state = self.state();
        state.vault = Some(vault);
        state.last_activity_us = now_us;

        Ok(carried)
    }

    /// Reads something out of the open vault without the vault leaving the lock.
    ///
    /// Answers `None` while the vault is closed. There is no accessor that hands back the
    /// keys, and this is the reason: a borrow that ends with the closure cannot be stored
    /// somewhere that outlives the lock.
    #[must_use]
    pub fn with_vault<T>(&self, read: impl FnOnce(&UnlockedVault) -> T) -> Option<T> {
        self.state().vault.as_ref().map(read)
    }

    /// The guarded state, treating a poisoned lock as a reason to close the vault.
    ///
    /// A panic while the keys were reachable leaves state nobody can vouch for. Continuing
    /// with it is the wrong half of the choice, and refusing every later lock would mean one
    /// panic anywhere makes the application useless until it is restarted. So the vault is
    /// emptied, the poison is cleared, and whoever asks next is told it is locked, which is
    /// the truth by then.
    fn state(&self) -> MutexGuard<'_, SessionState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                guard.vault = None;
                if let Some(storage) = guard.storage.take() {
                    let _released = storage.close();
                }
                self.state.clear_poison();
                guard
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use cairn_crypto::{Argon2Params, CryptoError, UnlockedVault};
    use cairn_domain::session::{IdleDecision, InactivityMinutes, InactivityTimeout};

    use super::{ImportTicket, LockReason, PathBuf, Session, UnlockFailure, UnlockOutcome};

    /// A moment in the middle of the range, so the arithmetic either side of it is ordinary.
    const NOW_US: i64 = 1_700_000_000_000_000;

    /// Five minutes in microseconds, the default inactivity period.
    const FIVE_MINUTES_US: i64 = 5 * 60 * 1_000_000;

    /// An open vault to put in a session.
    ///
    /// Made by creating a real one at the cheapest parameters the crate will accept. Building
    /// the type by hand is not possible from outside its crate, and it should not be: a test
    /// that fabricated one would stop testing what the application actually holds.
    fn an_open_vault() -> UnlockedVault {
        let params = Argon2Params::new(
            cairn_crypto::MIN_MEMORY_KIB,
            cairn_crypto::MIN_PASSES,
            cairn_crypto::MAX_LANES,
        )
        .expect("the lowest accepted parameters are accepted");

        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");
        vault
    }

    /// A session that is already open, for the tests that are about what happens afterwards.
    fn an_unlocked_session(timeout: InactivityTimeout) -> Session {
        let session = Session::new(timeout, NOW_US);
        let opened =
            tauri::async_runtime::block_on(session.unlock_with(|| Ok(an_open_vault()), NOW_US))
                .expect("a derivation that succeeds opens the vault");

        assert_eq!(opened, UnlockOutcome::Opened);
        session
    }

    /// A ticket with that word, expiring ten minutes after [`NOW_US`].
    fn a_ticket(token: &str) -> ImportTicket {
        ImportTicket {
            token: token.to_owned(),
            staging: PathBuf::from("no-existe").join("cairn.import.db"),
            records: vec![("settings".to_owned(), 3)],
            expires_us: NOW_US + 10 * 60 * 1_000_000,
        }
    }

    #[test]
    fn an_import_is_confirmed_by_the_word_it_was_given_and_by_no_other() {
        let session = Session::new(InactivityTimeout::default(), NOW_US);
        session
            .hold_import(a_ticket("la palabra"), NOW_US)
            .expect("nothing was waiting");

        assert!(session.take_import("otra palabra", NOW_US).is_none());
        // And the wrong word did not take it away, which is what stops one guess from
        // throwing away a restore somebody spent two minutes preparing.
        let taken = session
            .take_import("la palabra", NOW_US)
            .expect("the right word confirms it");

        assert_eq!(taken.records, vec![("settings".to_owned(), 3)]);
    }

    #[test]
    fn an_import_is_confirmed_once_and_not_twice() {
        let session = Session::new(InactivityTimeout::default(), NOW_US);
        session
            .hold_import(a_ticket("la palabra"), NOW_US)
            .expect("nothing was waiting");

        assert!(session.take_import("la palabra", NOW_US).is_some());
        assert!(
            session.take_import("la palabra", NOW_US).is_none(),
            "a replacement could have happened twice"
        );
    }

    #[test]
    fn an_import_stops_being_good_after_ten_minutes() {
        let session = Session::new(InactivityTimeout::default(), NOW_US);
        session
            .hold_import(a_ticket("la palabra"), NOW_US)
            .expect("nothing was waiting");

        let just_before = NOW_US + 10 * 60 * 1_000_000 - 1;
        assert!(session.take_import("la palabra", just_before + 1).is_none());
        assert!(session.take_import("la palabra", just_before).is_some());
    }

    #[test]
    fn a_second_import_is_refused_while_the_first_is_still_waiting() {
        // There is one staging database and it has one name. Two preparations at once would
        // be two half written files under it.
        let session = Session::new(InactivityTimeout::default(), NOW_US);
        session
            .hold_import(a_ticket("la primera"), NOW_US)
            .expect("nothing was waiting");

        assert!(session.hold_import(a_ticket("la segunda"), NOW_US).is_err());
        assert!(
            session.take_import("la primera", NOW_US).is_some(),
            "the refused second one disturbed the first"
        );
    }

    #[test]
    fn an_expired_import_is_out_of_the_way_and_handed_back_to_be_cleaned_up() {
        let session = Session::new(InactivityTimeout::default(), NOW_US);
        session
            .hold_import(a_ticket("la vieja"), NOW_US)
            .expect("nothing was waiting");

        let later = NOW_US + 11 * 60 * 1_000_000;
        let displaced = session
            .hold_import(a_ticket("la nueva"), later)
            .expect("an expired ticket is not in the way");

        assert_eq!(
            displaced.map(|ticket| ticket.token),
            Some("la vieja".to_owned()),
            "the file the expired one left behind would never be removed"
        );
    }

    #[test]
    fn locking_throws_away_the_import_that_was_waiting() {
        // A confirmation that survived a lock would let whoever unlocks next have their vault
        // replaced by a file somebody else chose.
        let session = an_unlocked_session(InactivityTimeout::default());
        session
            .hold_import(a_ticket("la palabra"), NOW_US)
            .expect("nothing was waiting");

        assert!(session.lock());
        assert!(session.take_import("la palabra", NOW_US).is_none());
    }

    #[test]
    fn a_new_session_is_locked() {
        let session = Session::new(InactivityTimeout::default(), NOW_US);

        assert!(!session.is_unlocked());
        assert!(session.with_vault(|_vault| ()).is_none());
    }

    #[test]
    fn a_derivation_that_succeeds_opens_the_vault() {
        let session = an_unlocked_session(InactivityTimeout::default());

        assert!(session.is_unlocked());
        assert!(session.with_vault(|vault| *vault.key_id()).is_some());
    }

    #[test]
    fn a_derivation_that_fails_leaves_the_vault_closed() {
        let session = Session::new(InactivityTimeout::default(), NOW_US);

        let failure =
            tauri::async_runtime::block_on(session.unlock_with(|| Err(CryptoError::Open), NOW_US))
                .expect_err("a derivation that fails cannot open the vault");

        assert!(matches!(failure, UnlockFailure::Refused(CryptoError::Open)));
        assert!(!session.is_unlocked());
    }

    #[test]
    fn a_derivation_that_panics_is_not_a_wrong_password() {
        // The distinction that keeps a machine fault from spending somebody's attempts. A
        // panic in the blocking pool arrives as a join failure, and folding it into the
        // refusal would make the caller count it.
        let session = Session::new(InactivityTimeout::default(), NOW_US);

        let failure = tauri::async_runtime::block_on(
            session.unlock_with(|| panic!("the derivation thread went away"), NOW_US),
        )
        .expect_err("a derivation that panics cannot open the vault");

        assert!(matches!(failure, UnlockFailure::Interrupted));
        assert!(!session.is_unlocked());
    }

    #[test]
    fn two_concurrent_unlocks_derive_exactly_once() {
        // The reason the permit is taken before the vault is looked at. Argon2id is
        // expensive by design, so a frontend that sends the request twice, or an attacker
        // who sends it a hundred times, must not turn that into a hundred derivations.
        //
        // Deterministic rather than timed: the first derivation blocks inside itself until
        // this test lets it go, so the second is definitely waiting for the permit by then.
        let session = Arc::new(Session::new(InactivityTimeout::default(), NOW_US));
        let derivations = Arc::new(AtomicUsize::new(0));

        let (started, has_started) = mpsc::channel::<()>();
        let (release, may_finish) = mpsc::channel::<()>();

        let first = {
            let session = Arc::clone(&session);
            let derivations = Arc::clone(&derivations);
            tauri::async_runtime::spawn(async move {
                session
                    .unlock_with(
                        move || {
                            derivations.fetch_add(1, Ordering::SeqCst);
                            started.send(()).expect("the test is still listening");
                            may_finish.recv().expect("the test releases this");
                            Ok(an_open_vault())
                        },
                        NOW_US,
                    )
                    .await
            })
        };

        has_started
            .recv()
            .expect("the first derivation reports that it started");

        let second = {
            let session = Arc::clone(&session);
            let derivations = Arc::clone(&derivations);
            tauri::async_runtime::spawn(async move {
                session
                    .unlock_with(
                        move || {
                            derivations.fetch_add(1, Ordering::SeqCst);
                            Ok(an_open_vault())
                        },
                        NOW_US,
                    )
                    .await
            })
        };

        release.send(()).expect("the first derivation is waiting");

        let (first, second) = tauri::async_runtime::block_on(async move {
            (
                first.await.expect("the first task finished"),
                second.await.expect("the second task finished"),
            )
        });

        assert_eq!(
            first.expect("the first unlock opened the vault"),
            UnlockOutcome::Opened
        );
        assert_eq!(
            second.expect("the second unlock found it open"),
            UnlockOutcome::AlreadyOpen
        );
        assert_eq!(
            derivations.load(Ordering::SeqCst),
            1,
            "two concurrent unlocks ran more than one derivation"
        );
    }

    #[test]
    fn replacing_derives_even_though_the_vault_is_already_open() {
        // Changing the password or the parameters happens with the vault open, and the work
        // is what produces the header that then has to be written. Skipping it the way an
        // unlock does would mean the operation silently did nothing.
        let session = an_unlocked_session(InactivityTimeout::default());
        let derivations = Arc::new(AtomicUsize::new(0));

        let carried = {
            let derivations = Arc::clone(&derivations);
            tauri::async_runtime::block_on(session.replace_with(
                move || {
                    derivations.fetch_add(1, Ordering::SeqCst);
                    Ok((an_open_vault(), "the new header"))
                },
                NOW_US,
            ))
        }
        .expect("the derivation succeeded");

        assert_eq!(carried, "the new header");
        assert_eq!(derivations.load(Ordering::SeqCst), 1);
        assert!(session.is_unlocked());
    }

    #[test]
    fn a_replacement_that_fails_leaves_the_vault_open() {
        // A password change that could not be completed must not lock somebody out of a
        // vault they already had open. Nothing on disk changed either, so the keys in memory
        // are still the right ones.
        let session = an_unlocked_session(InactivityTimeout::default());
        let before = session
            .with_vault(|vault| *vault.key_id())
            .expect("the vault is open");

        let failure = tauri::async_runtime::block_on(
            session.replace_with(|| Err::<(UnlockedVault, ()), _>(CryptoError::Open), NOW_US),
        )
        .expect_err("the derivation failed");

        assert!(matches!(failure, UnlockFailure::Refused(CryptoError::Open)));
        assert_eq!(session.with_vault(|vault| *vault.key_id()), Some(before));
    }

    #[test]
    fn locking_closes_the_vault_and_locking_again_says_so() {
        let session = an_unlocked_session(InactivityTimeout::default());

        assert!(session.lock(), "the first lock closed an open vault");
        assert!(!session.is_unlocked());
        assert!(session.with_vault(|_vault| ()).is_none());
        assert!(!session.lock(), "the second lock had nothing to close");
    }

    #[test]
    fn a_locked_vault_is_reported_as_locking_now() {
        let session = Session::new(InactivityTimeout::Never, NOW_US);

        // Even under never, because never describes when an open vault closes itself and
        // this one is already closed.
        assert_eq!(session.idle_decision(NOW_US), IdleDecision::Lock);
        assert_eq!(
            session.lock_if_due(NOW_US),
            None,
            "there was nothing to lock"
        );
    }

    #[test]
    fn a_vault_left_alone_for_the_whole_period_locks_itself() {
        let session = an_unlocked_session(InactivityTimeout::default());

        assert_eq!(session.lock_if_due(NOW_US + FIVE_MINUTES_US - 1), None);
        assert!(session.is_unlocked());

        assert_eq!(
            session.lock_if_due(NOW_US + FIVE_MINUTES_US),
            Some(LockReason::Inactivity)
        );
        assert!(!session.is_unlocked());
    }

    #[test]
    fn activity_puts_the_timer_back_to_the_beginning() {
        let session = an_unlocked_session(InactivityTimeout::default());

        session.note_activity(NOW_US + FIVE_MINUTES_US - 1);

        assert_eq!(session.lock_if_due(NOW_US + FIVE_MINUTES_US), None);
        assert!(session.is_unlocked());
    }

    #[test]
    fn activity_after_a_lock_does_not_decide_when_the_next_timer_starts() {
        // A frontend that keeps beating after the vault closed would otherwise set the
        // moment the next session is measured from, before that session exists.
        let session = an_unlocked_session(InactivityTimeout::default());
        session.lock();

        session.note_activity(NOW_US + FIVE_MINUTES_US);
        let reopened =
            tauri::async_runtime::block_on(session.unlock_with(|| Ok(an_open_vault()), NOW_US))
                .expect("the vault opens again");

        assert_eq!(reopened, UnlockOutcome::Opened);
        assert_eq!(
            session.lock_if_due(NOW_US + FIVE_MINUTES_US),
            Some(LockReason::Inactivity),
            "the timer was measured from the beat that arrived while it was locked"
        );
    }

    #[test]
    fn a_window_that_lost_focus_locks_once_the_grace_period_has_passed() {
        let session = an_unlocked_session(InactivityTimeout::default());
        let grace = 30 * 1_000_000;

        session.note_focus_lost(NOW_US);

        assert_eq!(session.due_to_lock(NOW_US + grace - 1), None);
        assert_eq!(
            session.lock_if_due(NOW_US + grace),
            Some(LockReason::FocusLost)
        );
        assert!(!session.is_unlocked());
    }

    #[test]
    fn a_window_that_came_back_cancels_the_countdown() {
        let session = an_unlocked_session(InactivityTimeout::default());

        session.note_focus_lost(NOW_US);
        session.note_focus_gained();

        assert_eq!(session.due_to_lock(NOW_US + 60 * 1_000_000), None);
        assert!(session.is_unlocked());
    }

    #[test]
    fn losing_focus_twice_without_coming_back_does_not_restart_the_countdown() {
        // A platform that reports the same thing twice must not be able to hold the vault
        // open forever by repeating itself.
        let session = an_unlocked_session(InactivityTimeout::default());
        let grace = 30 * 1_000_000;

        session.note_focus_lost(NOW_US);
        session.note_focus_lost(NOW_US + grace - 1);

        assert_eq!(
            session.lock_if_due(NOW_US + grace),
            Some(LockReason::FocusLost)
        );
    }

    #[test]
    fn opening_the_vault_again_does_not_inherit_a_countdown_from_before() {
        // Closing clears it. Otherwise a vault locked while the window was away would lock
        // again the moment it was opened, with no way to tell why.
        let session = an_unlocked_session(InactivityTimeout::default());

        session.note_focus_lost(NOW_US);
        session.lock();

        let reopened =
            tauri::async_runtime::block_on(session.unlock_with(|| Ok(an_open_vault()), NOW_US))
                .expect("the vault opens again");

        assert_eq!(reopened, UnlockOutcome::Opened);
        assert_eq!(session.due_to_lock(NOW_US + 60 * 1_000_000), None);
    }

    #[test]
    fn a_session_overdue_on_both_counts_reports_the_one_somebody_can_act_on() {
        // Inactivity is a setting somebody chose and can change. Focus is not, so reporting
        // focus here would send them looking for a setting that would not have helped.
        let session = an_unlocked_session(InactivityTimeout::default());

        session.note_focus_lost(NOW_US);

        assert_eq!(
            session.lock_if_due(NOW_US + FIVE_MINUTES_US),
            Some(LockReason::Inactivity)
        );
    }

    #[test]
    fn never_means_the_vault_stays_open() {
        let session = an_unlocked_session(InactivityTimeout::Never);

        assert_eq!(session.idle_decision(i64::MAX), IdleDecision::NeverLocks);
        assert_eq!(session.lock_if_due(i64::MAX), None);
        assert!(session.is_unlocked());
    }

    #[test]
    fn choosing_a_shorter_period_does_not_lock_the_vault_in_the_same_instant() {
        // Somebody who has been reading for four minutes and then picks one minute should
        // get a minute, not an immediate lock.
        let session = an_unlocked_session(InactivityTimeout::default());
        let four_minutes_later = NOW_US + 4 * 60 * 1_000_000;

        session.set_timeout(
            InactivityTimeout::After(InactivityMinutes::One),
            four_minutes_later,
        );

        assert_eq!(
            session.timeout(),
            InactivityTimeout::After(InactivityMinutes::One)
        );
        assert_eq!(session.lock_if_due(four_minutes_later), None);
        assert_eq!(
            session.lock_if_due(four_minutes_later + 60 * 1_000_000),
            Some(LockReason::Inactivity)
        );
    }

    #[test]
    fn a_panic_while_the_keys_were_reachable_closes_the_vault_rather_than_bricking_it() {
        // Two things at once. The vault must not survive a panic nobody can account for, and
        // the application must still work afterwards: refusing every later lock would turn
        // one panic anywhere into a program that needs restarting.
        let session = an_unlocked_session(InactivityTimeout::default());

        let panicked = catch_unwind(AssertUnwindSafe(|| {
            session.with_vault(|_vault| panic!("something went wrong while the keys were held"))
        }));

        assert!(panicked.is_err(), "the panic was supposed to escape");
        assert!(
            !session.is_unlocked(),
            "the vault survived a panic nobody can account for"
        );

        let reopened =
            tauri::async_runtime::block_on(session.unlock_with(|| Ok(an_open_vault()), NOW_US))
                .expect("the session still works after a panic");
        assert_eq!(reopened, UnlockOutcome::Opened);
    }
}
