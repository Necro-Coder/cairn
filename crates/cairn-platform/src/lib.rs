//! Operating system specific hardening: locking pages in memory, excluding the window
//! from screen capture, the clipboard, and the platform key stores behind Windows Hello
//! and Face ID.
//!
//! This is the only crate in the workspace permitted to contain `unsafe` code, which is
//! the entire reason it exists as a separate crate: the code that has to talk to raw
//! platform APIs lives in one place that can be audited on its own.
//!
//! The lint is set to `deny` at the workspace level rather than `forbid`, so that an
//! individual call can opt out at the point of use with a `// SAFETY:` comment
//! justifying every invariant it relies on. A blanket allow over the whole crate would
//! defeat the purpose: the exception has to be argued once per call site, not once.
// Deliberately no `#![forbid(unsafe_code)]` here. See the module documentation above.

pub mod secure_storage;

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
