//! What the application can say about itself when something is wrong.
//!
//! This screen exists in release builds too, because the machine where a problem shows up
//! is usually not the machine with a debugger on it. That makes what it is allowed to
//! report a security question rather than a convenience one.
//!
//! The rule is that nothing here may identify the person or the machine. No absolute
//! paths, no user name, no host name, no serial numbers, and nothing derived from a key
//! or from the contents of the database. A screenshot of this screen has to be safe to
//! paste into a public issue. `tests/diagnostics_privacy.rs` enforces that rather than
//! trusting the reader of this comment.

use serde::Serialize;

use super::app_info::AppInfo;
use crate::state::AppState;

/// What state the encrypted database is in.
///
/// Four states, and none of them is the negation of another, which is why this is an
/// enumeration rather than a pair of flags. The numbers it carries are counts and versions:
/// nothing here is derived from a key or from the contents of a row, so the screen stays safe
/// to screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
#[non_exhaustive]
pub enum DatabaseStatus {
    /// There is no database file yet, because this machine has no vault.
    NotInitialized,

    /// The file exists and the vault is closed, so nothing is open.
    ///
    /// The ordinary state while the lock screen is showing on a machine that has a vault. Told
    /// apart from the one above because "no vault" and "vault not open" lead to different
    /// screens and different advice.
    Locked,

    /// The database is open.
    #[serde(rename_all = "camelCase")]
    Open {
        /// The schema version the file is at, after any migrations that ran on opening.
        schema_version: u32,
        /// How many rows are marked as deleted across every table.
        ///
        /// On the screen because nothing is ever physically removed, so this number only grows
        /// until it is compacted, and a person deserves to be able to see that rather than to
        /// discover it as a file that keeps getting bigger.
        tombstones: u64,
    },

    /// The file was written by a newer build than this one, and was not opened.
    #[serde(rename_all = "camelCase")]
    Unsupported {
        /// The version the file says it is at.
        found: u32,
        /// The newest version this build knows.
        expected: u32,
    },
}

/// A snapshot of everything the application will admit to about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    /// Name, version and build profile.
    pub app: AppInfo,
    /// The operating system family, as a compile time constant such as `windows`.
    pub os: String,
    /// The processor architecture, as a compile time constant such as `x86_64`.
    pub arch: String,
    /// The version of the system WebView, or `None` when it cannot be determined.
    ///
    /// A missing version is not a failure of the command. The WebView is running, since
    /// the screen asking the question is drawn by it; only the version string is
    /// unavailable, and the screen says so.
    pub webview_version: Option<String>,
    /// Whether the database has been opened.
    pub database: DatabaseStatus,
    /// Milliseconds since the application started.
    pub uptime_ms: u64,
}

impl Diagnostics {
    /// Assembles a snapshot from values the caller has already collected.
    ///
    /// Taking the uptime, the WebView version and the database state as arguments keeps this
    /// function pure, which is what lets the privacy test call it directly without starting a
    /// window or opening a file.
    #[must_use]
    pub fn assemble(
        uptime_ms: u64,
        webview_version: Option<String>,
        database: DatabaseStatus,
    ) -> Self {
        Self {
            app: AppInfo::current(),
            // `std::env::consts` are baked in at compile time. They describe the build,
            // not the machine, so they cannot identify anyone.
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            webview_version,
            database,
            uptime_ms,
        }
    }
}

/// Reads what state the database is in, without opening anything.
///
/// Answers from what the session already holds. A vault that is open has a schema version and a
/// count; a vault that is closed is reported as locked if this machine has a vault at all, and
/// as not initialised if it does not. Nothing here creates a file or takes a password.
///
/// A count that fails to be read reports zero rather than an error. This is the screen somebody
/// reaches when something is already wrong, and it refusing to draw because one of its numbers
/// is unavailable would be the worst possible moment for it to be strict.
fn database_status(state: &AppState) -> DatabaseStatus {
    let open = state.session().with_storage(|storage| {
        let tombstones = storage
            .database()
            .with(cairn_db::tombstones::census)
            .map(|counted| counted.total)
            .unwrap_or_default();

        DatabaseStatus::Open {
            schema_version: storage.schema_version(),
            tombstones,
        }
    });

    open.unwrap_or_else(|| {
        if state.vault().exists() {
            DatabaseStatus::Locked
        } else {
            DatabaseStatus::NotInitialized
        }
    })
}

/// Reads the version of the system WebView.
///
/// Returns `None` when the platform cannot report it, which is a normal outcome rather
/// than an error: the answer is simply unknown.
fn webview_version() -> Option<String> {
    tauri::webview_version().ok()
}

/// Returns a snapshot of the application state for the diagnostics screen.
#[tauri::command]
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn diagnostics(state: tauri::State<'_, AppState>) -> Diagnostics {
    Diagnostics::assemble(
        state.uptime_ms(),
        webview_version(),
        database_status(&state),
    )
}

#[cfg(test)]
mod tests {
    use super::{DatabaseStatus, Diagnostics};

    #[test]
    fn the_database_state_is_reported_as_it_was_handed_in() {
        let snapshot = Diagnostics::assemble(0, None, DatabaseStatus::NotInitialized);
        assert_eq!(snapshot.database, DatabaseStatus::NotInitialized);

        let open = Diagnostics::assemble(
            0,
            None,
            DatabaseStatus::Open {
                schema_version: 2,
                tombstones: 7,
            },
        );
        assert_eq!(
            open.database,
            DatabaseStatus::Open {
                schema_version: 2,
                tombstones: 7
            }
        );
    }

    #[test]
    fn an_unknown_webview_version_is_absence_rather_than_failure() {
        let snapshot = Diagnostics::assemble(1, None, DatabaseStatus::NotInitialized);
        assert_eq!(snapshot.webview_version, None);
    }

    #[test]
    fn os_and_arch_are_reported_and_never_empty() {
        let snapshot = Diagnostics::assemble(1, None, DatabaseStatus::NotInitialized);
        assert!(!snapshot.os.is_empty(), "os must be reported");
        assert!(!snapshot.arch.is_empty(), "arch must be reported");
    }

    #[test]
    fn uptime_is_passed_through_unchanged() {
        assert_eq!(
            Diagnostics::assemble(1_234, None, DatabaseStatus::NotInitialized).uptime_ms,
            1_234
        );
    }

    #[test]
    fn database_status_serialises_to_a_stable_name_the_frontend_can_match_on() {
        let encoded = serde_json::to_string(&DatabaseStatus::NotInitialized)
            .expect("a fieldless variant always serialises");
        assert_eq!(encoded, r#"{"state":"notInitialized"}"#);

        let encoded = serde_json::to_string(&DatabaseStatus::Open {
            schema_version: 2,
            tombstones: 7,
        })
        .expect("a variant with fields always serialises");
        assert_eq!(
            encoded,
            r#"{"state":"open","schemaVersion":2,"tombstones":7}"#
        );

        let encoded = serde_json::to_string(&DatabaseStatus::Unsupported {
            found: 9,
            expected: 2,
        })
        .expect("a variant with fields always serialises");
        assert_eq!(encoded, r#"{"state":"unsupported","found":9,"expected":2}"#);
    }
}
