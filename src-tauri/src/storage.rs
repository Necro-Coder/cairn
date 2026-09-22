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
use cairn_db::backup::swap;
use cairn_db::codec::FieldCodec;
use cairn_db::search::{Results, SearchIndex};
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
    /// The titles of the vault, opened, for as long as this value exists.
    ///
    /// The one piece of plaintext this application keeps outside a single call, and it is here
    /// rather than beside the window for exactly that reason: this is what the lock destroys.
    /// Behind a lock of its own so a search does not wait on a write.
    titles: Mutex<SearchIndex>,
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

        // What the vault cannot search in SQL is opened here, once, while the key is already in
        // hand. A failure is **not** a failure to unlock. Refusing to open the vault because one
        // title does not decrypt turns a problem with one row into the loss of everything else,
        // which is the wrong half of that trade for somebody who came to read a different entry.
        // The index is left empty and says so through `is_complete`, and the interface reports
        // that the search is unavailable rather than letting somebody believe it found nothing.
        let mut titles = SearchIndex::empty();
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let _built = database.with(|connection| titles.build(connection, &codec));

        Ok(Self {
            database,
            device,
            schema_version: applied.to,
            clock: Mutex::new(resumed),
            titles: Mutex::new(titles),
        })
    }

    /// One page of the entries whose title, user name or address contains what was typed.
    ///
    /// Reads the index rather than the file. Nothing is decrypted here, because everything this
    /// answers with was decrypted once, on the unlock.
    #[must_use]
    pub fn search_entries(&self, needle: &str, page: u16) -> Results {
        self.with_titles(|index| index.search(needle, page))
    }

    /// Opens every live title again, replacing what the index held.
    ///
    /// Called after a write that changes a title. Rebuilding the whole list rather than patching
    /// one row is deliberate: a patch that misses a case leaves a title somebody can still find
    /// after deleting it, and the whole list is a few hundred kilobytes.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Sealed`] if a title does not open and [`DbError::Sqlite`] if the read
    /// fails. The index is left empty rather than stale.
    pub fn rebuild_titles(&self, vault: &UnlockedVault) -> Result<(), DbError> {
        let codec = self.codec(vault);
        self.with_titles(|index| {
            self.database
                .with(|connection| index.build(connection, &codec))
        })
    }

    /// How many titles the index holds, and whether it holds all of them.
    #[must_use]
    pub fn title_index_size(&self) -> (usize, bool) {
        self.with_titles(|index| (index.len(), index.is_complete()))
    }

    /// Takes the index, recovering from a poisoned lock the way the clock does.
    ///
    /// A panic elsewhere must not turn the search into a permanent failure, and there is no
    /// invariant to protect: the worst a half written index can be is out of date, and the next
    /// rebuild replaces it.
    fn with_titles<T>(&self, work: impl FnOnce(&mut SearchIndex) -> T) -> T {
        let mut index = match self.titles.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                self.titles.clear_poison();
                poisoned.into_inner()
            }
        };

        work(&mut index)
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
        // Before the file, and whether or not the file agrees to close. The index is plaintext,
        // and a database that refuses to let go is no reason to leave every title of the vault
        // readable in this process.
        self.with_titles(SearchIndex::clear);

        self.database.close()
    }

    /// Replaces the database underneath with a restored one and opens everything again.
    ///
    /// Consumes this storage, because the file it is holding is about to stop existing. What
    /// comes back is built the same way the one at unlock was: the device identifier is read
    /// from its own file, which a restore never touches; the migrations are applied, because a
    /// backup from an older schema arrives at an older schema; the logical clock is resumed
    /// from the rows that are now there; and the titles are decrypted again, because every one
    /// of them has just changed.
    ///
    /// # Errors
    ///
    /// Returns [`RestoreFailure::Refused`] with this storage handed back untouched if the swap
    /// did not start, [`RestoreFailure::Closed`] if the vault on disk is the one it was and
    /// this process no longer has it open, and [`RestoreFailure::Opened`] if the restore
    /// happened and the result could not be opened.
    pub fn restore_from(
        self,
        staging: &Path,
        vault: &UnlockedVault,
        safety_copy: &Path,
        now_us: i64,
    ) -> Result<Self, RestoreFailure> {
        // Every file of a vault lives in one directory, and this is the one the database was
        // opened from, so a restore cannot be made to write anywhere else by anything that
        // happened since.
        let directory = self
            .database
            .path()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        // Before the connection goes anywhere. The index is plaintext and it is about to be
        // wrong in any case, because the rows it was built from are being replaced.
        self.with_titles(SearchIndex::clear);

        match swap::swap_in(self.database, staging, safety_copy) {
            Ok(_swapped) => {
                Storage::open(&directory, vault, now_us).map_err(RestoreFailure::Opened)
            }
            Err(swap::SwapError::Refused { database, cause }) => Err(RestoreFailure::Refused {
                storage: Box::new(Self {
                    database: *database,
                    device: self.device,
                    schema_version: self.schema_version,
                    clock: self.clock,
                    titles: self.titles,
                }),
                cause,
            }),
            Err(swap::SwapError::NotSwapped(cause)) => Err(RestoreFailure::Closed(cause)),
        }
    }
}

/// Why a restore did not finish, and what the caller is left holding.
#[derive(Debug)]
#[non_exhaustive]
pub enum RestoreFailure {
    /// Nothing was replaced. The storage is handed back, open and still the real one.
    Refused {
        /// The vault as it was, still open.
        storage: Box<Storage>,
        /// What went wrong.
        cause: DbError,
    },

    /// Nothing was replaced, and the vault is no longer open in this process.
    ///
    /// The file on disk is the one the person had. What the caller has to do is lock, so the
    /// person unlocks again into the vault they already had.
    Closed(DbError),

    /// The restore happened and the result could not be opened.
    ///
    /// The file on disk is the restored one. This must never be described to anybody as
    /// nothing having happened: their old vault is in the copy taken beside it, and the new
    /// one is where the old one was.
    Opened(DbError),
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
