//! State that lives for as long as the application window does.
//!
//! Today it holds only the moment the process started. The session state that matters,
//! whether the vault is locked and when it locks itself again, arrives with the
//! cryptographic core and will live here too.

use std::time::Instant;

/// Application-wide state, managed by Tauri and handed to commands by reference.
#[derive(Debug)]
pub struct AppState {
    /// When the process started. Used to report uptime, never to seed anything.
    started_at: Instant,
}

impl AppState {
    /// Creates the state, taking the start of the process to be now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
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
