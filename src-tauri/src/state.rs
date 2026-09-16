//! State that lives for as long as the application window does.
//!
//! Two things, and they measure different kinds of time on purpose. The uptime comes from a
//! monotonic instant, because it is a duration and must not move when somebody adjusts the
//! clock. The session measures against the wall clock, because the moments it compares
//! against are written into a file and read back by a later process.

use std::time::Instant;

use cairn_domain::session::InactivityTimeout;

use crate::clock::now_us;
use crate::session::Session;

/// Application-wide state, managed by Tauri and handed to commands by reference.
#[derive(Debug)]
pub struct AppState {
    /// When the process started. Used to report uptime, never to seed anything.
    started_at: Instant,
    /// The vault, for as long as it is open.
    session: Session,
}

impl AppState {
    /// Creates the state, taking the start of the process to be now.
    ///
    /// The vault starts locked. There is no path by which a process begins with keys in it:
    /// opening one always goes through a password.
    #[must_use]
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            session: Session::new(InactivityTimeout::default(), now_us()),
        }
    }

    /// The vault, for as long as it is open.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
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

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;

    #[test]
    fn uptime_starts_near_zero() {
        let state = AppState::new();
        assert!(
            state.uptime_ms() < 1_000,
            "a state created just now should not report a second of uptime"
        );
    }

    #[test]
    fn uptime_does_not_go_backwards() {
        let state = AppState::new();
        let first = state.uptime_ms();
        let second = state.uptime_ms();
        assert!(second >= first, "{second} should not be before {first}");
    }
}
