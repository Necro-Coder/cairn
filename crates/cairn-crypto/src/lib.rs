//! Cryptography for Cairn: key derivation, the key hierarchy, authenticated
//! encryption and the framing used by encrypted exports.
//!
//! This crate performs no input or output and knows nothing about the database, the
//! window or the transport. Everything it does is a pure function of its arguments,
//! which is what makes it testable with property tests and checkable under Miri.
//!
//! Nothing in here is implemented yet. The key hierarchy arrives with the phase that
//! introduces it, so that it can be reviewed on its own rather than buried in a commit
//! that also moves scaffolding around.
#![forbid(unsafe_code)]

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

#[cfg(target_os = "ios")]
compile_error!("throwaway: deliberately breaking the iOS build to prove the gate goes red");
