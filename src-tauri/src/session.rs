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

use std::sync::{Mutex, MutexGuard};

use cairn_crypto::{CryptoError, UnlockedVault};
use cairn_domain::session::{IdleDecision, InactivityTimeout, idle_decision};

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

/// Everything that has to change together when the vault opens or closes.
#[derive(Debug)]
struct SessionState {
    /// The keys, present exactly while the vault is open. Dropping this clears them.
    vault: Option<UnlockedVault>,
    /// The last moment there was keyboard or mouse activity inside the window.
    last_activity_us: i64,
    /// How long the vault may sit idle before it closes itself.
    timeout: InactivityTimeout,
}

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
    derivation: tauri::async_runtime::Mutex<()>,
}

impl Session {
    /// A locked session with the inactivity policy given.
    #[must_use]
    pub fn new(timeout: InactivityTimeout, now_us: i64) -> Self {
        Self {
            state: Mutex::new(SessionState {
                vault: None,
                last_activity_us: now_us,
                timeout,
            }),
            derivation: tauri::async_runtime::Mutex::new(()),
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

    /// Locks the vault if it has been idle long enough, answering whether it did.
    ///
    /// Answering rather than announcing, because the event that tells the interface belongs
    /// to the layer that has a window to send it to.
    pub fn lock_if_idle(&self, now_us: i64) -> bool {
        let mut state = self.state();
        if state.vault.is_none() {
            return false;
        }
        if idle_decision(state.timeout, state.last_activity_us, now_us) != IdleDecision::Lock {
            return false;
        }

        state.vault = None;
        true
    }

    /// Closes the vault, answering whether it was open.
    ///
    /// Closing is dropping. The key clears itself on the way out, and there is no copy of it
    /// anywhere else in this process to clear separately.
    pub fn lock(&self) -> bool {
        self.state().vault.take().is_some()
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

        let derived = tauri::async_runtime::spawn_blocking(derive)
            .await
            .map_err(|_joining| UnlockFailure::Interrupted)?
            .map_err(UnlockFailure::Refused)?;

        let mut state = self.state();
        state.vault = Some(derived);
        state.last_activity_us = now_us;

        Ok(UnlockOutcome::Opened)
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

    use super::{Session, UnlockFailure, UnlockOutcome};

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
        assert!(!session.lock_if_idle(NOW_US), "there was nothing to lock");
    }

    #[test]
    fn a_vault_left_alone_for_the_whole_period_locks_itself() {
        let session = an_unlocked_session(InactivityTimeout::default());

        assert!(!session.lock_if_idle(NOW_US + FIVE_MINUTES_US - 1));
        assert!(session.is_unlocked());

        assert!(session.lock_if_idle(NOW_US + FIVE_MINUTES_US));
        assert!(!session.is_unlocked());
    }

    #[test]
    fn activity_puts_the_timer_back_to_the_beginning() {
        let session = an_unlocked_session(InactivityTimeout::default());

        session.note_activity(NOW_US + FIVE_MINUTES_US - 1);

        assert!(!session.lock_if_idle(NOW_US + FIVE_MINUTES_US));
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
        assert!(
            session.lock_if_idle(NOW_US + FIVE_MINUTES_US),
            "the timer was measured from the beat that arrived while it was locked"
        );
    }

    #[test]
    fn never_means_the_vault_stays_open() {
        let session = an_unlocked_session(InactivityTimeout::Never);

        assert_eq!(session.idle_decision(i64::MAX), IdleDecision::NeverLocks);
        assert!(!session.lock_if_idle(i64::MAX));
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
        assert!(!session.lock_if_idle(four_minutes_later));
        assert!(session.lock_if_idle(four_minutes_later + 60 * 1_000_000));
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
