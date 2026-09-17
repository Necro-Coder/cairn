//! A directory that exists for one test and is removed when it ends.
//!
//! Unique per test and per run. Per run as well as per test because a file left behind by a
//! crashed run would otherwise be opened by the next one, and an encrypted file from a
//! different key turns a real failure into a confusing one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A temporary directory, removed when the value is dropped.
pub(crate) struct Scratch {
    directory: PathBuf,
}

impl Scratch {
    /// A directory of its own for the test that names it.
    pub(crate) fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let ordinal = COUNTER.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "cairn-db-{label}-{}-{nanos}-{ordinal}",
            process::id()
        ));

        fs::create_dir_all(&directory).expect("a temporary directory can be created");

        Self { directory }
    }

    /// The directory itself.
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    /// Where the database file goes, under the name the application uses.
    pub(crate) fn database_path(&self) -> PathBuf {
        self.directory.join(crate::DATABASE_FILE)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// A migrated database in a directory of its own, with the vault that opens it.
///
/// The three things every test of this crate needs, kept together because they have to
/// outlive each other in the right order: the keys open the file, the file lives in the
/// directory, and the directory is removed last. Assembling them separately in every test
/// module meant five copies of the same twenty lines, and five places for the Argon2id
/// parameters of a test to drift apart.
pub(crate) struct Sandbox {
    /// Named rather than ignored so the directory is removed when this is dropped.
    scratch: Scratch,
    vault: cairn_crypto::UnlockedVault,
    database: crate::open::Database,
}

impl Sandbox {
    /// A new vault and a freshly migrated database, at the cheapest parameters that are
    /// still a real Argon2id run.
    ///
    /// The floor rather than the default: sixty-four mebibytes and three passes take most
    /// of a second each, and a suite that takes a minute is a suite people stop running.
    pub(crate) fn new(label: &str) -> Self {
        Self::at(label, 1_700_000_000_000_000)
    }

    /// The same, with the moment the migrations are stamped with chosen by the caller.
    pub(crate) fn at(label: &str, now_us: i64) -> Self {
        let params = cairn_crypto::Argon2Params::new(
            cairn_crypto::MIN_MEMORY_KIB,
            cairn_crypto::MIN_PASSES,
            1,
        )
        .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");

        let scratch = Scratch::new(label);
        let database = crate::open::Database::open(&scratch.database_path(), &vault.database_key())
            .expect("a new database file can be created");
        crate::migrations::apply_all(&database, now_us).expect("the migrations apply");

        Self {
            scratch,
            vault,
            database,
        }
    }

    /// The open database.
    pub(crate) fn database(&self) -> &crate::open::Database {
        &self.database
    }

    /// The keys that open it.
    pub(crate) fn vault(&self) -> &cairn_crypto::UnlockedVault {
        &self.vault
    }

    /// A codec over those keys.
    pub(crate) fn codec(&self) -> crate::codec::FieldCodec<'_> {
        crate::codec::FieldCodec::new(self.vault.data_key(), *self.vault.key_id())
    }

    /// The directory the database file is in.
    pub(crate) fn directory(&self) -> &Path {
        self.scratch.directory()
    }
}

/// A migrated database and nothing else, for a test that only reads the schema.
pub(crate) fn migrated() -> Sandbox {
    Sandbox::new("schema")
}
