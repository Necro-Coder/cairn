//! The open database and the identifier of this installation, for as long as the vault is open.
//!
//! Held beside the keys rather than beside the window, because the two have exactly the same
//! lifetime and the whole point of closing the vault is that nothing is left holding a handle
//! to the file with the key inside it. Closing the session closes this, in that order, in one
//! place.
//!
//! Nothing here decides anything. It is the pair of things that come into existence on the
//! first successful unlock and stop existing when the vault closes.

use std::path::{Path, PathBuf};

use cairn_crypto::UnlockedVault;
use cairn_db::{DATABASE_FILE, DEVICE_FILE, Database, DbError, DeviceId, device, migrations};

/// The database and the identifier of the device that writes to it.
#[derive(Debug)]
pub struct Storage {
    database: Database,
    device: DeviceId,
    schema_version: u32,
}

impl Storage {
    /// Opens the database for an open vault, creating the device identifier the first time.
    ///
    /// Takes the vault by reference inside the caller's closure, so no key is copied out to get
    /// here. The database key is derived on the spot and lives only as long as this call.
    ///
    /// The device identifier is read first. It is the cheaper of the two and the one whose
    /// failure means something is wrong with the vault rather than with the database, so
    /// finding out before a file is created keeps a broken machine from growing an empty
    /// database beside its problem.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Io`] or [`DbError::Sealed`] if the device identifier cannot be read or
    /// created, and [`DbError::Sqlite`] if the database cannot be opened with this key.
    pub fn open(directory: &Path, vault: &UnlockedVault, now_us: i64) -> Result<Self, DbError> {
        let device = device::load_or_create(
            &directory.join(DEVICE_FILE),
            vault.data_key(),
            vault.key_id(),
        )?;
        let database = Database::open(&directory.join(DATABASE_FILE), &vault.database_key())?;

        // Before anything reads a row. A database at an older schema than this build is brought
        // up here, once, while nothing else can be holding a statement against it; one that is
        // newer than this build is refused, and the unlock fails rather than the application
        // running against a shape it does not understand.
        let applied = migrations::apply_all(&database, now_us)?;

        Ok(Self {
            database,
            device,
            schema_version: applied.to,
        })
    }

    /// The open database.
    #[must_use]
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// The identifier this installation writes into every row.
    #[must_use]
    pub fn device(&self) -> DeviceId {
        self.device
    }

    /// The schema version the database is at, after any migrations that ran on opening.
    #[must_use]
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Closes the database, answering whether SQLite agreed to let the file go.
    ///
    /// Consumes the value. There is nothing left afterwards that could still be used, which is
    /// the property that matters: a close that hands back something usable is a close somebody
    /// carries on through with the key still in the process.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Sqlite`] if SQLite refuses because something still holds a statement.
    pub fn close(self) -> Result<(), DbError> {
        self.database.close()
    }
}

/// Where the four files of a vault live on this machine.
///
/// Read once at startup and kept, rather than asked for again on each unlock. Asking twice
/// would mean a profile variable changed halfway through a run could move the database out from
/// under an open vault.
#[derive(Debug, Clone)]
pub struct DataDirectory(PathBuf);

impl DataDirectory {
    /// Names a directory that has already been resolved and created.
    #[must_use]
    pub fn new(directory: PathBuf) -> Self {
        Self(directory)
    }

    /// The directory itself.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}
