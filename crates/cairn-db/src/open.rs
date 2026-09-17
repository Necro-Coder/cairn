//! Opening the encrypted database file, and closing it so that it is really closed.
//!
//! One connection, behind one lock. A pool was considered and rejected: it buys nothing
//! against a local file with one writer, and it multiplies the number of places a connection
//! can still be alive, with the key inside it, at the moment the vault is supposed to be shut.
//!
//! The order of the statements below is not a preference. `PRAGMA key` has to be the first
//! thing said on the connection, and the cipher settings have to be said before the first page
//! is read, because after that the file has already been interpreted. The rest follow in the
//! order the project fixed for them so that a reader can compare this file against the design
//! line by line.
//!
//! Closing is the part that is easy to get wrong. Dropping a connection usually closes it, and
//! usually is not good enough for something that holds a key: [`Database::close`] consumes the
//! value and asks SQLite to close, and if SQLite refuses because a statement is still alive it
//! says so rather than leaving a handle open and reporting success.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use cairn_crypto::DatabaseKey;
use rusqlite::Connection;

use crate::error::DbError;

/// How long a statement waits for the lock before giving up, in milliseconds.
const BUSY_TIMEOUT_MS: u32 = 5_000;

/// The page cache, in kibibytes, expressed the way SQLite wants it.
///
/// Negative means kibibytes rather than pages, which is the only form worth using: a count of
/// pages means a different amount of memory the moment the page size changes.
///
/// Sixty-four mebibytes, which for this application's database is all of it.
const CACHE_SIZE_KIB: i32 = -65_536;

/// The page size the file is created with.
const CIPHER_PAGE_SIZE: u32 = 4_096;

/// The encrypted database, open.
///
/// Holds the connection rather than handing it out. Every caller passes a closure, so a borrow
/// of the connection cannot be stored anywhere that outlives the lock, which is the same shape
/// the session uses for the keys and for the same reason.
pub struct Database {
    path: PathBuf,
    connection: Mutex<Connection>,
}

impl fmt::Debug for Database {
    /// Names the type and nothing else.
    ///
    /// Not even the path. It contains the account name of whoever is logged in, and this value
    /// is reachable from the application state that a diagnostic dump would print.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Database(open)")
    }
}

impl Database {
    /// Opens, or creates, the database at a path with the key derived for it.
    ///
    /// The key is the one derived from the data key for this purpose and nothing else. It is
    /// given in the `x'...'` form, which tells SQLCipher that these are the key bytes rather
    /// than a passphrase: Argon2id has already been run, at parameters this project chose, and
    /// letting SQLCipher run its own derivation on top would be work that buys nothing while
    /// obscuring which derivation actually protects the file.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Sqlite`] if the file cannot be opened, if a setting is refused, or if
    /// the key does not open it. The last of those is found by reading the schema, because
    /// opening a connection reads no page and therefore succeeds against any file at all.
    pub fn open(path: &Path, key: &DatabaseKey) -> Result<Self, DbError> {
        allow_resident_memory_once();

        let connection = Connection::open(path)?;

        apply_settings(&connection, key)?;
        // Forces a page to be read. Without it a wrong key is discovered by the first query
        // that happens to run, which could be minutes later and somewhere that reports it as
        // something else.
        connection.query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })?;

        Ok(Self {
            path: path.to_path_buf(),
            connection: Mutex::new(connection),
        })
    }

    /// Where the file is.
    ///
    /// Used by the code that takes a copy before a migration. Not reported to anybody.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Runs something against the connection, holding the lock for exactly that long.
    ///
    /// # Errors
    ///
    /// Whatever the closure returns.
    pub fn with<T>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        work(&self.guard())
    }

    /// Runs something against the connection in a transaction, committing if it succeeds.
    ///
    /// # Errors
    ///
    /// Whatever the closure returns, in which case nothing it did is kept, and
    /// [`DbError::Sqlite`] if the transaction cannot be started or committed.
    pub fn in_transaction<T>(
        &self,
        work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        let mut guard = self.guard();
        let transaction = guard.transaction()?;
        let produced = work(&transaction)?;
        transaction.commit()?;

        Ok(produced)
    }

    /// Closes the connection, answering whether SQLite agreed.
    ///
    /// Consumes the value, so there is nothing left to use afterwards. That is the point: a
    /// close that leaves a usable handle behind is a close that somebody will call and then
    /// carry on through, with the key still in the process.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Sqlite`] if SQLite refuses, which means something else still holds a
    /// statement open. Reported rather than ignored: the caller asked for the file to be let
    /// go, and it has not been.
    pub fn close(self) -> Result<(), DbError> {
        let connection = self
            .connection
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        connection.close().map_err(|(_connection, error)| {
            // The connection handed back by a refused close is dropped here, which closes it if
            // it can. The error is still reported, because the caller has to know the file was
            // not let go when it asked.
            DbError::Sqlite(error)
        })
    }

    /// The connection, for the duration of the returned guard.
    ///
    /// A lock poisoned by a panic is taken anyway. What it guards is a handle to a file whose
    /// contents are on disk; refusing every later lock would turn one panic into an application
    /// that has to be restarted, and the vault closes on a panic in any case, which closes this.
    fn guard(&self) -> MutexGuard<'_, Connection> {
        match self.connection.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                self.connection.clear_poison();
                poisoned.into_inner()
            }
        }
    }
}

/// Raises the residency allowance once per process, before the first connection exists.
///
/// `cipher_memory_security` asks the system to pin the buffers the cipher works in, and the
/// default allowance on Windows is about a megabyte and a half. Left alone, every one of those
/// requests is refused: the setting is applied, the log fills with warnings nobody reads, and
/// the protection it is configured for does not happen. Past that, a process that keeps asking
/// eventually cannot commit the guard page of a thread stack, and Windows reports that as a
/// stack overflow with no recursion anywhere near it, which is how this was found.
///
/// The application raises it at startup as well, and earlier, because the keys are allocated
/// before any database is opened. This is here for everything else that opens one: the tests,
/// the migration tool, and any future entry point that forgets.
///
/// A refusal is not an error. The application still opens, with a cipher whose buffers may reach
/// the page file, which is the same honest position the rest of the project takes about locking.
fn allow_resident_memory_once() {
    use std::sync::Once;

    static RAISED: Once = Once::new();
    RAISED.call_once(|| {
        let _raised = cairn_platform::memory::allow_resident(
            cairn_platform::memory::RECOMMENDED_RESIDENT_BYTES,
        );
    });
}

/// Applies the key and every setting, in the order the design fixes.
///
/// Written as separate statements rather than one batch so that a refusal names which one. A
/// batch reports the first failure with the text of the whole batch, and the setting that
/// failed is the thing worth knowing.
fn apply_settings(connection: &Connection, key: &DatabaseKey) -> Result<(), DbError> {
    // First, before anything else is said on this connection. The value is formatted into the
    // statement because `PRAGMA key` does not accept a bound parameter; what goes in is
    // sixty-four characters of hexadecimal produced here from thirty-two bytes, so there is
    // nothing a caller could inject even if a caller chose the bytes.
    connection.execute_batch(&format!("PRAGMA key = \"x'{}'\";", hex(key.expose())))?;

    // The cipher settings, before a page is read. After that the file has been interpreted and
    // changing how it is interpreted is too late.
    connection.execute_batch("PRAGMA cipher_memory_security = ON;")?;
    connection.execute_batch(&format!("PRAGMA cipher_page_size = {CIPHER_PAGE_SIZE};"))?;

    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    // Answers with the mode it ended up in, which `execute_batch` discards. Read back below
    // rather than trusted, because a file on a network share silently stays in its old mode.
    connection.execute_batch("PRAGMA journal_mode = WAL;")?;
    connection.execute_batch("PRAGMA synchronous = NORMAL;")?;
    connection.execute_batch(&format!("PRAGMA busy_timeout = {BUSY_TIMEOUT_MS};"))?;
    // No temporary table and no sort buffer reaches the disk in the clear.
    connection.execute_batch("PRAGMA temp_store = MEMORY;")?;
    connection.execute_batch(&format!("PRAGMA cache_size = {CACHE_SIZE_KIB};"))?;
    // SQLCipher and memory mapped reads are incompatible. Written explicitly so that nobody
    // turns it on later as an optimisation and gets plaintext pages in the page cache.
    connection.execute_batch("PRAGMA mmap_size = 0;")?;
    connection.execute_batch("PRAGMA trusted_schema = OFF;")?;

    Ok(())
}

/// Sixty-four lower case hexadecimal characters for thirty-two bytes of key.
///
/// Written by hand rather than pulled in, because the alternative is a dependency for one loop,
/// and the one that formats key material is a dependency worth not having.
fn hex(bytes: &[u8; 32]) -> String {
    use fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Cannot fail: writing to a `String` has no failure mode, and the width is fixed.
        let _written = write!(encoded, "{byte:02x}");
    }

    encoded
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};

    use super::{Database, hex};
    use crate::error::DbError;
    use crate::test_support::Scratch;

    fn an_open_vault(password: &str) -> UnlockedVault {
        let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create(password, params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");
        vault
    }

    #[test]
    fn hex_is_two_lower_case_characters_per_byte() {
        let mut bytes = [0_u8; 32];
        bytes[0] = 0x00;
        bytes[1] = 0x0f;
        bytes[2] = 0xff;

        let encoded = hex(&bytes);
        assert_eq!(encoded.len(), 64);
        assert!(encoded.starts_with("000fff"));
        assert!(
            encoded
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn the_settings_the_design_fixes_are_the_ones_in_force() {
        let scratch = Scratch::new("settings");
        let vault = an_open_vault("una frase larga para la prueba");
        let database = Database::open(&scratch.database_path(), &vault.database_key())
            .expect("a new database file opens");

        database
            .with(|connection| {
                let journal: String = connection
                    .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                    .expect("journal_mode answers");
                assert_eq!(journal.to_lowercase(), "wal");

                let foreign_keys: i64 = connection
                    .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                    .expect("foreign_keys answers");
                assert_eq!(foreign_keys, 1, "foreign keys are not being enforced");

                // Read as text and parsed. SQLCipher answers this one with a string, and asking
                // for an integer gets a type error rather than a number, which is the kind of
                // detail that makes an assertion look like it passed when it never ran.
                let page_size: String = connection
                    .query_row("PRAGMA cipher_page_size", [], |row| row.get(0))
                    .expect("cipher_page_size answers");
                assert_eq!(page_size.trim(), "4096");

                let mmap: i64 = connection
                    .query_row("PRAGMA mmap_size", [], |row| row.get(0))
                    .expect("mmap_size answers");
                assert_eq!(
                    mmap, 0,
                    "memory mapped reads are incompatible with the cipher"
                );

                let temp_store: i64 = connection
                    .query_row("PRAGMA temp_store", [], |row| row.get(0))
                    .expect("temp_store answers");
                assert_eq!(
                    temp_store, 2,
                    "temporary data would reach the disk in the clear"
                );

                let cipher: String = connection
                    .query_row("PRAGMA cipher_version", [], |row| row.get(0))
                    .expect("SQLCipher answers cipher_version; plain SQLite returns no rows");
                assert!(!cipher.trim().is_empty());

                Ok(())
            })
            .expect("the settings can be read back");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_file_written_with_one_key_cannot_be_opened_with_another() {
        let scratch = Scratch::new("wrong-key");
        let path = scratch.database_path();

        let mine = an_open_vault("una frase larga para la prueba");
        let theirs = an_open_vault("otra frase completamente distinta");

        let database = Database::open(&path, &mine.database_key()).expect("a new file opens");
        database
            .with(|connection| {
                connection.execute_batch("CREATE TABLE probe (value TEXT NOT NULL)")?;
                Ok(())
            })
            .expect("one table can be created");
        database.close().expect("the connection closes");

        assert!(
            matches!(
                Database::open(&path, &theirs.database_key()),
                Err(DbError::Sqlite(_))
            ),
            "the wrong key opened an encrypted database"
        );
        // And the right one still does.
        Database::open(&path, &mine.database_key())
            .expect("the right key opens it")
            .close()
            .expect("the connection closes");
    }

    #[test]
    fn what_was_written_is_not_in_the_file_in_the_clear() {
        let scratch = Scratch::new("opaque");
        let path = scratch.database_path();
        let vault = an_open_vault("una frase larga para la prueba");
        let canary = "cairn-plaintext-canary";

        let database = Database::open(&path, &vault.database_key()).expect("a new file opens");
        database
            .with(|connection| {
                connection.execute_batch(&format!(
                    "CREATE TABLE probe (value TEXT NOT NULL);
                     INSERT INTO probe (value) VALUES ('{canary}');"
                ))?;
                Ok(())
            })
            .expect("one row can be written");
        database.close().expect("the connection closes");

        let raw = std::fs::read(&path).expect("the file is readable as bytes");
        assert!(
            !raw.windows(canary.len())
                .any(|window| window == canary.as_bytes()),
            "the inserted text appears verbatim in the file"
        );
        assert!(
            !raw.starts_with(b"SQLite format 3\0"),
            "the file carries a plain SQLite header"
        );
        // The table name is metadata, which is the half the per-record layer cannot protect and
        // the whole reason the file is encrypted as well.
        assert!(
            !raw.windows(5).any(|window| window == b"probe"),
            "the table name appears verbatim in the file"
        );
    }

    #[test]
    fn closing_lets_go_of_the_file() {
        // Asserted by renaming it. A handle that is still open holds the file on Windows, so a
        // rename that succeeds is the operating system agreeing that nothing has it.
        let scratch = Scratch::new("close");
        let path = scratch.database_path();
        let vault = an_open_vault("una frase larga para la prueba");

        let database = Database::open(&path, &vault.database_key()).expect("a new file opens");
        database.close().expect("the connection closes");

        let moved = path.with_extension("moved");
        std::fs::rename(&path, &moved).expect("a closed database can be renamed");
        std::fs::rename(&moved, &path).expect("and put back");
    }

    #[test]
    fn a_transaction_that_fails_keeps_nothing() {
        let scratch = Scratch::new("rollback");
        let vault = an_open_vault("una frase larga para la prueba");
        let database = Database::open(&scratch.database_path(), &vault.database_key())
            .expect("a new file opens");

        database
            .with(|connection| {
                connection.execute_batch("CREATE TABLE probe (value TEXT NOT NULL)")?;
                Ok(())
            })
            .expect("one table can be created");

        let failed: Result<(), DbError> = database.in_transaction(|transaction| {
            transaction.execute("INSERT INTO probe (value) VALUES ('one')", [])?;
            Err(DbError::NotFound)
        });
        assert!(matches!(failed, Err(DbError::NotFound)));

        let rows: i64 = database
            .with(|connection| {
                Ok(connection.query_row("SELECT count(*) FROM probe", [], |row| row.get(0))?)
            })
            .expect("the count can be read");
        assert_eq!(rows, 0, "a transaction that failed left a row behind");

        database.close().expect("the connection closes");
    }
}
