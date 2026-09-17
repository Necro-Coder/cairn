//! The moment a restored database takes the place of the live one.
//!
//! Everything else about an import is careful and reversible. This is the one step that is
//! neither, and it is deliberately the smallest module in the crate: before it the person has
//! the vault they had, after it they have the one from the backup, and there is no third
//! state in between for a power cut to find.
//!
//! Four things happen in this order, and the order is the whole design.
//!
//! First the live database is checkpointed with `wal_checkpoint(TRUNCATE)`, which folds the
//! write-ahead log back into the main file and empties it. Without that, the main file on its
//! own is not the database — the last writes are in the log beside it — and the copy taken in
//! the next step would be a copy of a vault missing whatever was written most recently.
//!
//! Then a copy of the live database is taken. It is taken **before** anything is replaced,
//! because it is the only way back: a restore is the one operation in this application that
//! deliberately destroys data the person still has, and somebody who restores the wrong file
//! deserves better than an apology. The copy goes in the directory they chose, under a name
//! they can read.
//!
//! Then the connection is closed, because on Windows a file somebody still has open cannot be
//! replaced, and the replacement itself is one system call that either happened or did not.
//!
//! Opening the result again is the caller's job and not this module's. What has to be opened
//! after a restore is not a connection: it is the device identifier, the migrations, the
//! logical clock and the decrypted index of titles, all of which live a layer up. A function
//! here that handed back a connection would be handing back a quarter of a working vault.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::DbError;
use crate::open::Database;

/// What a completed swap leaves behind.
///
/// Nothing is open afterwards. The file at the live path is the restored one and the caller
/// opens it when it is ready to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swapped {
    /// Where the copy of the vault that was replaced ended up.
    pub safety_copy: PathBuf,
}

/// Why a swap did not finish, and what the caller is left holding.
///
/// Deliberately not `#[non_exhaustive]`, which every other error type in this crate is. The
/// whole value of this one is that a caller has to decide, for each case, whether the person's
/// vault is still theirs; a catch-all arm is a caller guessing at that, and a variant added
/// later ought to break the code that would have guessed.
#[derive(Debug)]
pub enum SwapError {
    /// Nothing was replaced. The live database is unchanged and is handed back open.
    ///
    /// Everything up to and including the copy is in this arm, which is to say every failure
    /// that can happen before the point of no return.
    Refused {
        /// The live database, still open and still the real one.
        database: Box<Database>,
        /// What went wrong.
        cause: DbError,
    },

    /// Nothing was replaced, and this process no longer holds the live database open.
    ///
    /// The close itself failed, the log files beside it could not be cleared, or the one
    /// system call that does the replacing refused. The vault on disk is the one the person
    /// had and is untouched; what has to happen next is that somebody opens it again.
    NotSwapped(DbError),
}

impl SwapError {
    /// What went wrong.
    #[must_use]
    pub fn cause(&self) -> &DbError {
        match self {
            Self::Refused { cause, .. } | Self::NotSwapped(cause) => cause,
        }
    }
}

/// Replaces the live database with a staging one, leaving a copy of what was there.
///
/// `live` is consumed because it is closed partway through: a handle to a file that has been
/// replaced is a handle to nothing, and one left in the caller's hands is one somebody will
/// keep using.
///
/// # Errors
///
/// Returns [`SwapError::Refused`] with the live database still open if anything fails while it
/// is still open, and [`SwapError::NotSwapped`] if it had already been closed. In either case
/// the vault on disk is the one the person had.
pub fn swap_in(live: Database, staging: &Path, safety_copy: &Path) -> Result<Swapped, SwapError> {
    let live_path = live.path().to_path_buf();

    // Asked before anything else, because everything else is harder to undo. A staging
    // database that is not there is a caller bug rather than a disk failure, and finding out
    // after the live connection has been closed would turn it into one.
    if !staging.is_file() {
        return Err(refused(
            live,
            DbError::Io {
                what: "the restored database",
                operation: "opened",
                cause: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "there is no staging database to swap in",
                ),
            },
        ));
    }

    // The log folded back in, so that the file about to be copied is the whole database and
    // not most of it.
    if let Err(cause) = checkpoint(&live) {
        return Err(refused(live, cause));
    }

    if let Err(cause) = copy_aside(&live_path, safety_copy) {
        return Err(refused(live, cause));
    }

    // Closed before the replacement, and checked. A close that SQLite refused means something
    // still holds a statement open, and on Windows the replacement would fail anyway; better
    // to stop here, where nothing has been touched, than one call later.
    if let Err(cause) = live.close() {
        // The handle is gone with the failed close, so there is nothing to hand back. The
        // file is untouched, which is what matters, and the caller opens it again.
        return Err(SwapError::NotSwapped(cause));
    }
    if let Err(cause) = remove_sidecars(&live_path) {
        return Err(SwapError::NotSwapped(cause));
    }

    // The point of no return, and one system call wide.
    if let Err(cause) = cairn_platform::replace::replace(staging, &live_path) {
        return Err(SwapError::NotSwapped(DbError::Io {
            what: "the restored database",
            operation: "moved into place",
            cause: std::io::Error::other(cause),
        }));
    }

    // Best effort, and after the swap on purpose. The staging log files belong to a database
    // that no longer exists under that name; failing the whole restore because one of them
    // could not be deleted would be refusing to report a restore that has already happened.
    remove_sidecars(staging).ok();

    Ok(Swapped {
        safety_copy: safety_copy.to_path_buf(),
    })
}

/// Folds the write-ahead log back into the main file and empties it.
fn checkpoint(live: &Database) -> Result<(), DbError> {
    live.with(|connection| {
        // Returns a row of three counters, which nothing here needs. What is needed is the
        // effect: after this the main file is the database on its own.
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_counters| Ok(()))?;

        Ok(())
    })
}

/// Copies the live database aside, refusing to write over anything that is already there.
fn copy_aside(live: &Path, destination: &Path) -> Result<(), DbError> {
    if destination.exists() {
        return Err(DbError::Io {
            what: "the copy of the vault being replaced",
            operation: "written",
            cause: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "there is already a file with that name",
            ),
        });
    }

    fs::copy(live, destination).map_err(|cause| DbError::Io {
        what: "the copy of the vault being replaced",
        operation: "written",
        cause,
    })?;

    Ok(())
}

/// Removes the two files SQLite keeps beside a database, if they are there.
fn remove_sidecars(database: &Path) -> Result<(), DbError> {
    for suffix in ["-wal", "-shm"] {
        let mut path = database.as_os_str().to_owned();
        path.push(suffix);
        let path = PathBuf::from(path);

        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {}
            Err(cause) => {
                return Err(DbError::Io {
                    what: "a database log file",
                    operation: "removed",
                    cause,
                });
            }
        }
    }

    Ok(())
}

/// The refusal arm, with the live database handed back.
fn refused(live: Database, cause: DbError) -> SwapError {
    SwapError::Refused {
        database: Box::new(live),
        cause,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use cairn_domain::Hlc;

    use super::{SwapError, swap_in};
    use crate::device::DeviceId;
    use crate::migrations;
    use crate::open::Database;
    use crate::repositories::settings;
    use crate::test_support::Scratch;

    const NOW_US: i64 = 1_700_000_000_000_000;
    const HLC: Hlc = Hlc::new(1_000, 0, [1; 6]);

    /// A vault, a directory, and two migrated databases in it under names of their own.
    struct Two {
        scratch: Scratch,
        vault: cairn_crypto::UnlockedVault,
    }

    impl Two {
        fn new(label: &str) -> Self {
            let params = cairn_crypto::Argon2Params::new(
                cairn_crypto::MIN_MEMORY_KIB,
                cairn_crypto::MIN_PASSES,
                1,
            )
            .expect("the lowest accepted parameters are accepted");
            let (_header, vault) =
                cairn_crypto::create("una frase larga para la prueba", params, 0)
                    .expect("a vault is created");

            Self {
                scratch: Scratch::new(label),
                vault,
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.scratch.directory().join(name)
        }

        /// A migrated database at that name, holding one setting with that value.
        fn database_with(&self, name: &str, value: &[u8]) -> Database {
            let database = Database::open(&self.path(name), &self.vault.database_key())
                .expect("the database opens");
            migrations::apply_all(&database, NOW_US).expect("the migrations apply");

            let device = DeviceId::generate().expect("random bytes");
            database
                .with(|connection| {
                    settings::put(
                        connection,
                        &self.codec(),
                        device,
                        HLC,
                        NOW_US,
                        "theme",
                        Some(value),
                    )
                })
                .expect("the setting writes");

            database
        }

        fn codec(&self) -> crate::codec::FieldCodec<'_> {
            crate::codec::FieldCodec::new(self.vault.data_key(), *self.vault.key_id())
        }

        /// What the `theme` setting says in an open database.
        fn theme_in(&self, database: &Database) -> Vec<u8> {
            database
                .with(|connection| {
                    let read = settings::get(connection, &self.codec(), "theme")?
                        .expect("the setting is there");
                    Ok(read.value.as_deref().map_or_else(Vec::new, Clone::clone))
                })
                .expect("the setting reads")
        }

        /// The same, for a database at a path that nothing has open.
        fn theme_of(&self, path: &Path) -> Vec<u8> {
            let database =
                Database::open(path, &self.vault.database_key()).expect("the database opens");
            let value = self.theme_in(&database);
            database.close().expect("the database closes");

            value
        }
    }

    #[test]
    fn the_staging_database_becomes_the_live_one() {
        let two = Two::new("swap-happy");
        let live = two.database_with("live.db", b"la vieja");
        let staging = two.database_with("staging.db", b"la nueva");
        staging.close().expect("the staging database closes");

        let _swapped = swap_in(live, &two.path("staging.db"), &two.path("copia.db"))
            .expect("the swap happens");
        let value = two.theme_of(&two.path("live.db"));

        assert_eq!(value, b"la nueva");
        assert!(
            !two.path("staging.db").exists(),
            "the staging database is still there afterwards"
        );
    }

    #[test]
    fn the_vault_that_was_replaced_is_still_readable_in_the_copy() {
        // The only way back, and the reason the copy is taken before anything is replaced
        // rather than after. A restore is the one operation here that destroys data the
        // person still has.
        let two = Two::new("swap-copy");
        let live = two.database_with("live.db", b"la vieja");
        let staging = two.database_with("staging.db", b"la nueva");
        staging.close().expect("the staging database closes");

        let _swapped = swap_in(live, &two.path("staging.db"), &two.path("copia.db"))
            .expect("the swap happens");

        assert_eq!(two.theme_of(&two.path("copia.db")), b"la vieja");
    }

    #[test]
    fn a_staging_database_that_is_not_there_leaves_the_live_one_alone() {
        // The failure that matters: a restore that cannot finish must leave the vault the
        // person had, open and unchanged, not a directory with nothing in it.
        let two = Two::new("swap-missing");
        let live = two.database_with("live.db", b"la vieja");

        let refused = swap_in(live, &two.path("not-here.db"), &two.path("copia.db"))
            .expect_err("a missing staging database is refused");

        let SwapError::Refused { database, .. } = refused else {
            panic!("the live database was not handed back");
        };
        database.close().expect("it closes");

        assert_eq!(two.theme_of(&two.path("live.db")), b"la vieja");
        assert!(
            !two.path("copia.db").exists(),
            "a copy was taken for a restore that never started"
        );
    }

    #[test]
    fn a_copy_that_would_write_over_something_is_refused_with_the_vault_still_open() {
        // Before the point of no return, which is what this arm is for: the caller gets the
        // live database back, open, and nothing on disk has moved.
        let two = Two::new("swap-clash");
        let live = two.database_with("live.db", b"la vieja");
        let staging = two.database_with("staging.db", b"la nueva");
        staging.close().expect("the staging database closes");

        std::fs::write(two.path("copia.db"), b"algo que ya estaba").expect("the file writes");

        let refused = swap_in(live, &two.path("staging.db"), &two.path("copia.db"))
            .expect_err("writing over an existing file is refused");

        let SwapError::Refused { database, .. } = refused else {
            panic!("the live database was not handed back");
        };

        let value = two.theme_in(&database);
        database.close().expect("it closes");

        assert_eq!(value, b"la vieja");
        assert!(
            two.path("staging.db").exists(),
            "the staging database moved"
        );
    }
}
