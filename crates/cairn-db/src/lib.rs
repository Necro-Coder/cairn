//! Storage for Cairn: opening the encrypted database, the schema, its migrations and the
//! repositories built on top of them.
//!
//! Two layers of encryption, and they defend against different things. SQLCipher encrypts the
//! whole file, which is the only thing that can hide table names, index contents, the write
//! ahead log and the space a deleted row used to occupy. On top of it, every sensitive value is
//! sealed on its own with associated data that names the table, the row, the column and the
//! revision, which is the only thing that stops somebody who has the file from rearranging it.
//! Neither layer is sufficient and neither is decoration.
//!
//! What lives where. [`codec`] is the second layer. [`open`] is the first, and the only place
//! that knows the order the settings have to be applied in. [`device`] is the identifier this
//! installation writes into every row. Everything else is built on those three.
//!
//! Nothing here reads a clock or invents a moment. The caller supplies the instant, as it does
//! everywhere else in this workspace, so that a test can pin it and a row written on two
//! machines can be reasoned about.
#![forbid(unsafe_code)]

pub mod clock;
pub mod codec;
pub mod device;
pub mod error;
pub mod migrations;
pub mod open;
pub mod repositories;
pub mod row;
pub mod tombstones;

#[cfg(test)]
mod test_support;

pub use codec::{FieldCodec, RECORD_FORMAT_VERSION, RowKey, SealedColumns};
pub use device::DeviceId;
pub use error::DbError;
pub use migrations::{LATEST_VERSION, Migration};
pub use open::Database;
pub use row::{COMMON_COLUMNS, RowStamp};

/// The name of the encrypted database file.
///
/// The four files of a vault share a prefix so that a listing of the data directory shows them
/// together and a backup script that matches on it cannot pick up three of the four.
pub const DATABASE_FILE: &str = "cairn.db";

/// The name of the file holding the sealed identifier of this installation.
pub const DEVICE_FILE: &str = "cairn.device";

/// The name of the vault header file.
///
/// Named here rather than beside the code that writes it, because two things need it and they
/// have to agree: the application, which rewrites it when the password changes, and the
/// migration tool, which reads it to open a vault with no window running. A second spelling of
/// this name somewhere else is a tool that looks in the wrong place and reports that there is no
/// vault.
pub const HEADER_FILE: &str = "cairn.header";

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
