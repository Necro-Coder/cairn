//! Proves that the SQLite this crate links against is SQLCipher, that it encrypts, and
//! that it was compiled with the options the project requires.
//!
//! None of this is testing SQLCipher itself. It is testing that the build produced what
//! the design assumes. Three separate things can go wrong without anybody noticing:
//! feature resolution can quietly hand back plain SQLite, the compile time options can
//! fail to reach the C compiler, and the encryption can be linked in but never actually
//! applied. Each one leaves a pipeline that is green and a promise that is false.
//!
//! No schema is created here beyond one throwaway table. The real schema arrives with the
//! phase that designs it, and starting it inside a test would be the worst possible place
//! for it to be born.
// Every function in an integration test file is test code, but the lint that forbids
// panicking constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]`
// functions. The helpers below are neither, and a helper that cannot panic would have to
// return a Result that every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

/// A test key, and nothing else. Thirty-two bytes written out by hand.
///
/// The real key is derived from the master password through Argon2id and never appears in
/// source. That machinery belongs to the phase that builds the key hierarchy; what is
/// under test here is the database layer, which only ever sees thirty-two bytes and has no
/// opinion about where they came from.
const TEST_KEY_HEX: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

/// A second test key, used to show that the wrong one fails the way the right one works.
const OTHER_TEST_KEY_HEX: &str = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";

/// A string written into the throwaway table so that it can be looked for in the raw file.
const CANARY: &str = "cairn-plaintext-canary";

/// Applies a raw key to a freshly opened connection.
///
/// The `x'...'` form is what tells SQLCipher that these are the key bytes themselves
/// rather than a passphrase to run through its own key derivation. That matters: the
/// project derives its key with Argon2id at parameters it chose, and letting SQLCipher
/// apply PBKDF2 on top of an already derived key would be work that buys nothing while
/// hiding which derivation actually protects the file.
///
/// The key is formatted into the statement rather than bound as a parameter because
/// `PRAGMA key` does not accept a bound parameter. The value is a constant in this file,
/// not input, so there is nothing here for a caller to inject.
fn apply_key(connection: &Connection, key_hex: &str) {
    connection
        .execute_batch(&format!("PRAGMA key = \"x'{key_hex}'\";"))
        .expect("applying a raw key to a fresh connection always succeeds");
}

/// Opens an encrypted database in memory, keyed and ready to use.
fn open_keyed_in_memory() -> Connection {
    let connection = Connection::open_in_memory().expect("an in-memory database always opens");
    apply_key(&connection, TEST_KEY_HEX);
    connection
}

/// A path in the system temporary directory, unique per test and per run.
///
/// Unique per run as well as per test, because a leftover file from a crashed run would
/// otherwise be opened by the next one and turn a real failure into a confusing one.
fn temporary_database_path(label: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the system clock is after 1970")
        .as_nanos();
    let ordinal = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "cairn-{label}-{}-{nanos}-{ordinal}.db",
        process::id()
    ))
}

/// Removes a test database and the journal files SQLite may have left beside it.
fn remove_database(path: &Path) {
    let _ = fs::remove_file(path);
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut companion = path.as_os_str().to_owned();
        companion.push(suffix);
        let _ = fs::remove_file(PathBuf::from(companion));
    }
}

/// Deletes the database it names when the test ends, whether or not the test passed.
struct TemporaryDatabase {
    path: PathBuf,
}

impl TemporaryDatabase {
    fn new(label: &str) -> Self {
        Self {
            path: temporary_database_path(label),
        }
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        remove_database(&self.path);
    }
}

/// Every `PRAGMA compile_options` entry of the library that got linked in.
fn compile_options() -> Vec<String> {
    let connection = Connection::open_in_memory().expect("an in-memory database always opens");
    let mut statement = connection
        .prepare("PRAGMA compile_options")
        .expect("compile_options is a pragma every SQLite build answers");
    let options = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("each row of compile_options is a single string")
        .collect::<Result<Vec<String>, _>>()
        .expect("reading the rows of compile_options does no I/O and cannot fail");
    assert!(
        !options.is_empty(),
        "compile_options returned nothing, which means the pragma did not run"
    );
    options
}

/// The compile time options the design depends on have to reach the C compiler.
///
/// They are requested through an environment variable in `.cargo/config.toml`, which is a
/// request rather than a guarantee: it is read by somebody else's build script, which
/// hands it to a compiler that is free to be configured differently. Reading them back out
/// of the built library is the only way to know they arrived.
#[test]
fn sqlite_was_compiled_with_the_options_the_project_requires() {
    let options = compile_options();

    for required in ["DQS=0", "THREADSAFE=1", "TEMP_STORE=3"] {
        assert!(
            options.iter().any(|option| option == required),
            "SQLite was built without {required}; compile options were {options:?}"
        );
    }

    // Loading a shared library named at runtime into the process that holds the vault key
    // is not something this application ever needs to do, and the bundled build turns it
    // on unless it is explicitly undefined.
    assert!(
        !options
            .iter()
            .any(|option| option == "ENABLE_LOAD_EXTENSION"),
        "SQLite was built with extension loading enabled; compile options were {options:?}"
    );
}

/// The linked library is SQLCipher, not plain SQLite.
///
/// `cipher_version` is the cheapest question only SQLCipher can answer. Plain SQLite does
/// not know the pragma and returns no rows at all, so this fails loudly rather than
/// quietly returning an empty string if feature resolution ever hands back the wrong
/// library.
#[test]
fn a_keyed_connection_reports_a_cipher_version() {
    let connection = open_keyed_in_memory();

    let version: String = connection
        .query_row("PRAGMA cipher_version", [], |row| row.get(0))
        .expect("SQLCipher answers cipher_version; plain SQLite returns no rows");

    assert!(
        !version.trim().is_empty(),
        "cipher_version answered with an empty string"
    );
}

/// A keyed database is unreadable without the key, and its contents are not in the file.
///
/// This is the test that matters. The two above can both pass against a build that links
/// SQLCipher and never encrypts anything, because linking and encrypting are different
/// facts. What is asserted here is the property the whole storage design rests on: the
/// bytes on disk are not the bytes that were written.
#[test]
fn a_database_created_with_a_key_cannot_be_read_without_it() {
    let database = TemporaryDatabase::new("keyed");

    {
        let connection = Connection::open(&database.path).expect("a new database file opens");
        apply_key(&connection, TEST_KEY_HEX);
        connection
            .execute_batch(&format!(
                "CREATE TABLE probe (value TEXT NOT NULL);
                 INSERT INTO probe (value) VALUES ('{CANARY}');"
            ))
            .expect("creating one table and inserting one row succeeds");
    }

    {
        // Opening succeeds even without the key: SQLCipher does not read a page until it
        // is asked for one. Reading is what fails.
        let connection = Connection::open(&database.path).expect("the file is still there");
        let result =
            connection.query_row("SELECT value FROM probe", [], |row| row.get::<_, String>(0));
        assert!(
            result.is_err(),
            "an unkeyed connection read an encrypted database"
        );
    }

    {
        let connection = Connection::open(&database.path).expect("the file is still there");
        apply_key(&connection, OTHER_TEST_KEY_HEX);
        let result =
            connection.query_row("SELECT value FROM probe", [], |row| row.get::<_, String>(0));
        assert!(result.is_err(), "the wrong key read an encrypted database");
    }

    {
        let connection = Connection::open(&database.path).expect("the file is still there");
        apply_key(&connection, TEST_KEY_HEX);
        let value: String = connection
            .query_row("SELECT value FROM probe", [], |row| row.get(0))
            .expect("the right key reads what it wrote");
        assert_eq!(value, CANARY, "the row came back changed");
    }

    let raw = fs::read(&database.path).expect("the database file is readable as bytes");
    let needle = CANARY.as_bytes();
    assert!(
        !raw.windows(needle.len()).any(|window| window == needle),
        "the inserted text appears verbatim in the database file"
    );
    // A plain SQLite file announces itself in its first sixteen bytes. An encrypted one
    // cannot, because that header is inside the encryption.
    assert!(
        !raw.starts_with(b"SQLite format 3\0"),
        "the database file carries a plain SQLite header"
    );
}
