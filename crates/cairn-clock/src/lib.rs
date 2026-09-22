//! Which day a moment fell on, for the person holding the device.
//!
//! This is the only crate that asks the operating system where it is. It exists as a crate of
//! its own rather than as a module of another one because of the shape of the workspace:
//! `cairn-domain` owns the calendar and must keep knowing nothing about the machine it runs on,
//! and `cairn-platform` sits below `cairn-crypto`, which sits below the domain, so it cannot see
//! a calendar day. A crate that depends on the domain and on nothing else in the workspace is
//! the only place both halves can meet.
//!
//! Nothing here keeps a moment. Every function takes the instant as an argument, so a test can
//! pin the instant and the zone and get the same answer on every machine, in every season.
#![forbid(unsafe_code)]

pub mod clock;

pub use clock::{CivilClock, ClockError, DayStart, SystemZone, zone_named};

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
