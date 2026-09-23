//! Business logic for Cairn: habit streaks, budget arithmetic, validation rules and
//! the decisions the merge algorithm makes.
//!
//! Deterministic by construction. The clock and the source of randomness are injected by
//! the caller rather than read from the environment, so a test can pin both and get the
//! same answer on every machine.
//!
//! This crate knows nothing about SQLite, about Tauri or about the serialisation format
//! used on the wire. Dependencies point inwards, and this is as far in as they go.
#![forbid(unsafe_code)]

pub mod habits;
pub mod hlc;
pub mod password;
pub mod session;
pub mod time;
pub mod tree;

pub use hlc::{Clock, Hlc, Rev};
pub use time::{CivilDay, TimeError, Timestamp};
pub use tree::{MAX_DEPTH, TreeError};

/// The version of this crate, taken from its manifest at compile time.
///
/// Every crate in the workspace inherits the same version from `[workspace.package]`, so
/// this is also the version of the application as a whole.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    /// Guards against a malformed version in the manifest. The value reaches the
    /// diagnostics screen and the release artefacts, so a typo here is not cosmetic.
    #[test]
    fn version_is_three_numeric_components() {
        let components: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(
            components.len(),
            3,
            "version must be major.minor.patch, got {VERSION}"
        );
        for component in components {
            assert!(
                component.parse::<u32>().is_ok(),
                "non numeric component in version {VERSION}"
            );
        }
    }
}
