//! Storage for Cairn: the schema, its migrations, the repositories built on top of it
//! and the code that opens the encrypted database file.
//!
//! No database is opened yet. The schema is defined in one place, versioned, and every
//! migration is reversible, so this crate is where that contract will live.
//!
//! What it does already have is the dependency that will open it: SQLite built from
//! source as SQLCipher, with its own copy of OpenSSL linked in. It is here this early on
//! purpose. Cross compiling that C to the phone is the one assumption the storage design
//! rests on that could turn out to be false, and a pipeline that compiles an empty crate
//! for iOS proves nothing while looking exactly like one that proves everything.
//!
//! The crate exposes no API for it yet, and deliberately so. What it has instead is
//! `tests/sqlcipher.rs`, which checks that the library linked in is SQLCipher rather than
//! plain SQLite, that it was compiled with the options the design requires, and that a
//! keyed file is genuinely unreadable without its key.
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
