//! Identity of the running application.
//!
//! Everything here is a compile time constant, which is why the command cannot fail and
//! has no error type. It exists so that the window shows what is actually running rather
//! than what somebody typed into a template once.

use serde::Serialize;

/// How the application is written wherever a person reads it.
///
/// The crate, the binary and the paths all use the lowercase form; this is the only
/// place the display form is defined. `config_hardening` asserts that it matches
/// `productName` in the Tauri configuration, so the two cannot drift apart.
pub const DISPLAY_NAME: &str = "Cairn";

/// Which build profile produced this binary.
///
/// This is not cosmetic. A diagnostics screen that says `release` while the devtools open
/// would mean the hardening did not take effect, and the person looking at the screen
/// needs to be able to tell.
pub const BUILD_PROFILE: &str = if cfg!(debug_assertions) {
    "debug"
} else {
    "release"
};

/// Name, version and build profile of the running application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// The display name, always `Cairn`.
    pub name: String,
    /// The version, taken from the workspace manifest at compile time.
    pub version: String,
    /// Either `debug` or `release`.
    pub profile: &'static str,
}

impl AppInfo {
    /// Reads the identity of this build.
    ///
    /// Infallible by construction: every field is fixed when the binary is compiled.
    #[must_use]
    pub fn current() -> Self {
        Self {
            name: DISPLAY_NAME.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            profile: BUILD_PROFILE,
        }
    }
}

/// Returns the name, version and build profile of the running application.
#[tauri::command]
#[must_use]
pub fn app_info() -> AppInfo {
    AppInfo::current()
}

#[cfg(test)]
mod tests {
    use super::{AppInfo, BUILD_PROFILE, DISPLAY_NAME};

    #[test]
    fn version_comes_from_the_manifest_and_is_not_hand_written() {
        assert_eq!(AppInfo::current().version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn name_is_the_display_form() {
        assert_eq!(AppInfo::current().name, DISPLAY_NAME);
        assert_eq!(DISPLAY_NAME, "Cairn");
    }

    #[test]
    fn profile_is_one_of_the_two_known_values() {
        assert!(
            BUILD_PROFILE == "debug" || BUILD_PROFILE == "release",
            "unexpected build profile {BUILD_PROFILE}"
        );
    }

    #[test]
    fn tests_run_in_a_debug_build_so_the_profile_reports_debug() {
        // If this ever fails, the profile detection is reading something other than the
        // build it is compiled into, which would make the diagnostics screen lie.
        assert_eq!(BUILD_PROFILE, "debug");
    }
}
