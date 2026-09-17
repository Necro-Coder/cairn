//! State that lives for as long as the application window does.
//!
//! Three things. The uptime comes from a monotonic instant, because it is a duration and
//! must not move when somebody adjusts the clock. The session and the vault measure against
//! the wall clock instead, because the moments they compare are written into a file and read
//! back by a later process.
//!
//! The permit is the fourth field and the only one that is not data. Anything that changes
//! the vault takes it first and holds it until the command returns, so that two creations
//! cannot both decide there is no vault yet and two failed unlocks cannot both read the same
//! count of attempts. The order everywhere is that permit, then the derivation inside the
//! session, then the two ordinary locks; nothing takes them the other way round.

use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use cairn_domain::session::InactivityTimeout;
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

use crate::clock::now_us;
use crate::session::Session;
use crate::storage::DataDirectory;
use crate::vault::Vault;

/// Application-wide state, managed by Tauri and handed to commands by reference.
#[derive(Debug)]
pub struct AppState {
    /// When the process started. Used to report uptime, never to seed anything.
    started_at: Instant,
    /// The keys, for as long as the vault is open.
    session: Session,
    /// The header, and where it lives.
    vault: Mutex<Vault>,
    /// Where the four files of the vault live, resolved once at startup.
    directory: DataDirectory,
    /// Taken for the whole of any command that can change the vault.
    operation: AsyncMutex<()>,
}

impl AppState {
    /// Creates the state around a vault that has already been read from disk.
    ///
    /// The vault starts locked. There is no path by which a process begins with keys in it:
    /// opening one always goes through a password.
    #[must_use]
    pub fn new(vault: Vault, directory: DataDirectory) -> Self {
        Self {
            started_at: Instant::now(),
            session: Session::new(InactivityTimeout::default(), now_us()),
            vault: Mutex::new(vault),
            directory,
            operation: AsyncMutex::new(()),
        }
    }

    /// Where the four files of the vault live.
    ///
    /// Resolved once at startup and kept. Asking the environment again on each unlock would mean
    /// a profile variable changed halfway through a run could move the database out from under
    /// an open vault.
    #[must_use]
    pub fn directory(&self) -> &DataDirectory {
        &self.directory
    }

    /// The keys, for as long as the vault is open.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The header, for the duration of the returned guard.
    ///
    /// Never held across an await. Every caller reads or writes and lets it go in the same
    /// statement, which is what keeps this lock and the derivation permit from ever being
    /// wanted at the same time in opposite orders.
    ///
    /// A lock poisoned by a panic is taken anyway. What it guards is a copy of bytes that are
    /// on the disk, so the worst a half finished mutation can leave behind is a header that
    /// the next read replaces; refusing every later lock would turn one panic into an
    /// application that needs restarting.
    pub fn vault(&self) -> MutexGuard<'_, Vault> {
        match self.vault.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                self.vault.clear_poison();
                poisoned.into_inner()
            }
        }
    }

    /// Takes the permit that makes vault changing commands run one at a time.
    ///
    /// Held for the whole command rather than for each step. The thing being protected is not
    /// a field, it is the sequence: read what is there, derive, write what replaces it.
    pub async fn begin_vault_operation(&self) -> AsyncMutexGuard<'_, ()> {
        self.operation.lock().await
    }

    /// Milliseconds elapsed since the application started.
    ///
    /// `Instant` is monotonic, so this cannot go backwards when the system clock is
    /// adjusted. The conversion saturates rather than wrapping: a process that has been
    /// running for longer than `u64::MAX` milliseconds is not a situation worth
    /// modelling, but silently reporting a small number for it would be a lie.
    #[must_use]
    pub fn uptime_ms(&self) -> u64 {
        u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::AppState;
    use crate::storage::DataDirectory;
    use crate::vault::Vault;

    struct Scratch {
        directory: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);

            Self {
                directory: std::env::temp_dir().join(format!(
                    "cairn-state-{name}-{}-{unique}",
                    std::process::id()
                )),
            }
        }

        fn state(&self) -> AppState {
            AppState::new(
                Vault::open_at(&self.directory).expect("the directory can be read"),
                DataDirectory::new(self.directory.clone()),
            )
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn uptime_starts_near_zero() {
        let scratch = Scratch::new("uptime");
        let state = scratch.state();

        assert!(
            state.uptime_ms() < 1_000,
            "a state created just now should not report a second of uptime"
        );
    }

    #[test]
    fn uptime_tracks_a_real_clock_rather_than_reporting_a_number() {
        // Pins that the value is a measurement. A constant would satisfy both of the tests
        // either side of this one, and a constant is exactly what the cold start figure on
        // the diagnostics screen must never be.
        //
        // Spinning rather than sleeping, because what is being waited for is that time has
        // genuinely passed, and the loop guarantees it has before the second reading.
        use std::time::{Duration, Instant};

        let state = Scratch::new("grows").state();
        let before = state.uptime_ms();

        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(5) {
            std::hint::spin_loop();
        }

        assert!(
            state.uptime_ms() > before,
            "the uptime did not move after five milliseconds of real time"
        );
    }

    #[test]
    fn uptime_does_not_go_backwards() {
        let scratch = Scratch::new("monotonic");
        let state = scratch.state();

        let first = state.uptime_ms();
        let second = state.uptime_ms();

        assert!(second >= first, "{second} should not be before {first}");
    }

    #[test]
    fn a_process_starts_with_the_vault_closed() {
        // There is no path that begins with keys in memory. Opening one always goes through
        // a password, even on the machine that just created the vault.
        let scratch = Scratch::new("starts-locked");

        assert!(!scratch.state().session().is_unlocked());
    }

    #[test]
    fn a_lock_poisoned_by_a_panic_does_not_leave_the_application_unusable() {
        // What it guards is a copy of bytes that are on the disk, so the recovery is to carry
        // on rather than to refuse everything from then on.
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let scratch = Scratch::new("poison");
        let state = scratch.state();

        let panicked = catch_unwind(AssertUnwindSafe(|| {
            let _guard = state.vault();
            panic!("something went wrong while the header was held");
        }));

        assert!(panicked.is_err(), "the panic was supposed to escape");
        assert!(!state.vault().exists(), "the header could not be read back");
    }
}
