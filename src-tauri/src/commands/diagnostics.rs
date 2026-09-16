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

/// Whether the encrypted database has been opened.
///
/// Only one value exists today. It is an enumeration rather than a boolean because the
/// states that are coming, such as a database that exists but is still locked, are not
/// the negation of anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DatabaseStatus {
    /// No database file has been created or opened. This is the only possible value
    /// until storage exists.
    NotInitialized,
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
    /// Taking the uptime and the WebView version as arguments keeps this function pure,
    /// which is what lets the privacy test call it directly without starting a window.
    #[must_use]
    pub fn assemble(uptime_ms: u64, webview_version: Option<String>) -> Self {
        Self {
            app: AppInfo::current(),
            // `std::env::consts` are baked in at compile time. They describe the build,
            // not the machine, so they cannot identify anyone.
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            webview_version,
            database: DatabaseStatus::NotInitialized,
            uptime_ms,
        }
    }
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
    Diagnostics::assemble(state.uptime_ms(), webview_version())
}

#[cfg(test)]
mod tests {
    use super::{DatabaseStatus, Diagnostics};

    #[test]
    fn database_is_not_initialised_while_there_is_no_storage() {
        let snapshot = Diagnostics::assemble(0, None);
        assert_eq!(snapshot.database, DatabaseStatus::NotInitialized);
    }

    #[test]
    fn an_unknown_webview_version_is_absence_rather_than_failure() {
        let snapshot = Diagnostics::assemble(1, None);
        assert_eq!(snapshot.webview_version, None);
    }

    #[test]
    fn os_and_arch_are_reported_and_never_empty() {
        let snapshot = Diagnostics::assemble(1, None);
        assert!(!snapshot.os.is_empty(), "os must be reported");
        assert!(!snapshot.arch.is_empty(), "arch must be reported");
    }

    #[test]
    fn uptime_is_passed_through_unchanged() {
        assert_eq!(Diagnostics::assemble(1_234, None).uptime_ms, 1_234);
    }

    #[test]
    fn database_status_serialises_to_a_stable_name_the_frontend_can_match_on() {
        let encoded = serde_json::to_string(&DatabaseStatus::NotInitialized)
            .expect("a fieldless enum always serialises");
        assert_eq!(encoded, "\"notInitialized\"");
    }
}
