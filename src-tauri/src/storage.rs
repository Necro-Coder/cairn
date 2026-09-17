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
use std::sync::Mutex;

use cairn_crypto::UnlockedVault;
use cairn_db::codec::FieldCodec;
use cairn_db::{
    DATABASE_FILE, DEVICE_FILE, Database, DbError, DeviceId, clock, device, migrations,
};
use cairn_domain::{Clock, Hlc};

/// The database, the identifier of the device that writes to it, and its logical clock.
#[derive(Debug)]
pub struct Storage {
    database: Database,
    device: DeviceId,
    schema_version: u32,
    /// The clock this device stamps its writes with.
    ///
    /// Behind a lock of its own rather than inside the connection's, because two writes that
    /// arrive at the same moment must get two different readings, and that has to hold whether
    /// or not they are in the same transaction. Taken for the length of one increment and
    /// released, so it is never held while a statement runs.
    clock: Mutex<Clock>,
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

        // After the migrations, because the tables it reads have to exist, and before anything
        // is written, because a clock that starts below what is already in the file would hand
        // out a reading a row already carries.
        let resumed = database.with(|connection| clock::resume(connection, device))?;

        Ok(Self {
            database,
            device,
            schema_version: applied.to,
            clock: Mutex::new(resumed),
        })
    }

    /// The next clock reading, for one write.
    ///
    /// Takes the moment as an argument rather than reading a clock, like everything else in
    /// this workspace. What comes back is strictly greater than every reading this device has
    /// given out, whatever the argument says.
    ///
    /// A poisoned lock is taken anyway and the clock inside it is used as it stands. The
    /// invariant that matters is that readings never repeat, and that one is kept by the value
    /// itself: a panic between two ticks cannot lower it.
    pub fn next_hlc(&self, now_ms: u64) -> Hlc {
        let mut clock = match self.clock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                self.clock.clear_poison();
                poisoned.into_inner()
            }
        };

        clock.tick(now_ms)
    }

    /// The open database.
    #[must_use]
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// A codec for the encrypted columns, borrowing the key of an open vault.
    ///
    /// Built where it is used and dropped there. It borrows rather than holding, so it cannot
    /// outlive the closure the keys were read inside, which is what keeps the rule that no key
    /// is ever copied out of the one place that holds it.
    #[must_use]
    pub fn codec<'a>(&self, vault: &'a UnlockedVault) -> FieldCodec<'a> {
        FieldCodec::new(vault.data_key(), *vault.key_id())
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
