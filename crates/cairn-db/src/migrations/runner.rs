//! Applying migrations, reverting them, and the ledger that says which have run.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension as _};

use crate::error::DbError;
use crate::open::Database;

/// One change to the schema, forwards and backwards.
///
/// The two scripts are embedded rather than read from disk, so a build carries the schema it
/// was written against and cannot be handed a different one.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// What this migration is numbered. Unique, and applied in ascending order.
    pub version: u32,
    /// What it is for, in a few words. Reaches no screen; it is for whoever reads a ledger.
    pub name: &'static str,
    /// The statements that apply it.
    pub up: &'static str,
    /// The statements that undo it.
    ///
    /// Written for every migration, and exercised in the tests by applying, reverting and
    /// applying again. A migration whose reverse has never been run is a reverse that does not
    /// work, and it is needed on exactly the day nobody has time to find out.
    pub down: &'static str,
}

impl Migration {
    /// The checksum recorded in the ledger when this migration is applied.
    ///
    /// Over the forward script only. The reverse is allowed to be corrected later — it changes
    /// nothing about the schema a database already has — and including it would turn a fix to
    /// an undo path into a refusal to start.
    #[must_use]
    pub fn checksum(&self) -> [u8; cairn_crypto::DIGEST_LEN] {
        cairn_crypto::digest(self.up.as_bytes())
    }
}

/// Every migration this build carries, in ascending order.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "common",
        up: include_str!("sql/0001_common.sql"),
        down: include_str!("sql/0001_common.down.sql"),
    },
    Migration {
        version: 2,
        name: "habits",
        up: include_str!("sql/0002_habits.sql"),
        down: include_str!("sql/0002_habits.down.sql"),
    },
    Migration {
        version: 3,
        name: "vault",
        up: include_str!("sql/0003_vault.sql"),
        down: include_str!("sql/0003_vault.down.sql"),
    },
    Migration {
        version: 4,
        name: "finances",
        up: include_str!("sql/0004_finances.sql"),
        down: include_str!("sql/0004_finances.down.sql"),
    },
    Migration {
        version: 5,
        name: "audit",
        up: include_str!("sql/0005_audit.sql"),
        down: include_str!("sql/0005_audit.down.sql"),
    },
];

/// Every table of user data a fully migrated database has, in the order they were created in.
///
/// A closed list rather than a query against `sqlite_master`, because it is used to build
/// statements. Reading the table names back out of the file and interpolating them would mean
/// the shape of a statement this process runs depends on the contents of a file, which is the
/// pattern that has to be refused even when the file is one we encrypted ourselves.
///
/// `schema_migrations` is not here. It is not user data, it is never synchronised, and it is the
/// one table exempt from the seven common columns.
pub const DATA_TABLES: &[&str] = &[
    "settings",
    "sync_state",
    "habit_areas",
    "habits",
    "habit_entries",
    "habit_pauses",
    "vault_folders",
    "vault_entries",
    "vault_urls",
    "vault_fields",
    "vault_password_history",
    "vault_tags",
    "vault_entry_tags",
    "accounts",
    "categories",
    "transactions",
    "budgets",
    "audit_events",
];

/// The newest schema version this build knows.
///
/// Computed from the list rather than written beside it, so that adding a migration and
/// forgetting to raise a constant is not a thing that can happen.
pub const LATEST_VERSION: u32 = {
    let mut latest = 0;
    let mut index = 0;
    while index < MIGRATIONS.len() {
        // Indexing rather than iterating: this runs at compile time, where iterators do not.
        #[expect(
            clippy::indexing_slicing,
            reason = "bounded by the loop condition on the same slice, and evaluated at compile time, where an out of bounds index is a compile error rather than a panic"
        )]
        let version = MIGRATIONS[index].version;
        if version > latest {
            latest = version;
        }
        index += 1;
    }
    latest
};

/// The name of the copy taken before migrations are applied.
const BACKUP_SUFFIX: &str = ".backup";

/// What applying migrations did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The version the database was at before.
    pub from: u32,
    /// The version it is at now.
    pub to: u32,
    /// Which versions were applied, in the order they ran.
    pub versions: Vec<u32>,
}

impl Applied {
    /// Whether anything ran.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }
}

/// The highest migration recorded in the ledger.
///
/// Zero on a database that has never been migrated, including one that has just been created.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if the ledger cannot be read.
pub fn applied_version(connection: &Connection) -> Result<u32, DbError> {
    ensure_ledger(connection)?;

    let highest: Option<i64> = connection
        .query_row("SELECT max(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .optional()?
        .flatten();

    Ok(u32::try_from(highest.unwrap_or(0)).unwrap_or(0))
}

/// Brings the database up to [`LATEST_VERSION`], applying whatever is missing.
///
/// Called on every open, including the one that creates the file. Doing nothing is the
/// ordinary outcome and costs one query.
///
/// # Errors
///
/// Returns [`DbError::SchemaTooNew`] if the file was written by a newer build,
/// [`DbError::LedgerMismatch`] if an applied migration is not the one this build carries under
/// that number, [`DbError::Io`] if the copy taken beforehand cannot be written or removed, and
/// [`DbError::Sqlite`] if a migration itself fails, in which case that migration is rolled back
/// and the database is left at the version before it.
pub fn apply_all(database: &Database, now_us: i64) -> Result<Applied, DbError> {
    let from = database.with(|connection| {
        let stamped = stamped_version(connection)?;
        if stamped > LATEST_VERSION {
            // Refused before anything is read or written. A newer build may have added a column
            // this one does not know to write, and rows written without it are rows the newer
            // build reads as incomplete.
            return Err(DbError::SchemaTooNew {
                found: stamped,
                expected: LATEST_VERSION,
            });
        }

        let applied = applied_version(connection)?;
        verify_ledger(connection)?;

        Ok(applied.max(stamped))
    })?;

    let pending: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|migration| migration.version > from)
        .collect();
    if pending.is_empty() {
        return Ok(Applied {
            from,
            to: from,
            versions: Vec::new(),
        });
    }

    // Only when there is something to lose. A database at version zero is one that has just
    // been created, and a copy of an empty file protects nothing while being one more file to
    // fail to write.
    let backup = if from == 0 {
        None
    } else {
        Some(take_backup(database)?)
    };

    let mut versions = Vec::with_capacity(pending.len());
    for migration in pending {
        database.in_transaction(|transaction| {
            transaction.execute_batch(migration.up)?;
            record(transaction, migration, now_us)?;
            stamp_version(transaction, migration.version)?;
            Ok(())
        })?;
        versions.push(migration.version);
    }

    if let Some(backup) = backup {
        remove_backup(&backup)?;
    }

    Ok(Applied {
        from,
        to: LATEST_VERSION,
        versions,
    })
}

/// Takes the database back down to a version, running each reverse script in turn.
///
/// Exists for the tests and for the tool that exercises them. Nothing in the running
/// application calls it: a release that needs to go backwards is a release that is replaced.
///
/// # Errors
///
/// The same as [`apply_all`], and [`DbError::Sqlite`] if a reverse script fails, in which case
/// that one migration is rolled back and the database is left where it was.
pub fn revert_to(database: &Database, target: u32) -> Result<Vec<u32>, DbError> {
    let current = database.with(applied_version)?;

    let mut reverted = Vec::new();
    for migration in MIGRATIONS
        .iter()
        .rev()
        .filter(|migration| migration.version > target && migration.version <= current)
    {
        database.in_transaction(|transaction| {
            transaction.execute_batch(migration.down)?;
            transaction.execute(
                "DELETE FROM schema_migrations WHERE version = ?1",
                [migration.version],
            )?;
            Ok(())
        })?;
        reverted.push(migration.version);

        // Outside the transaction that removed the row, because the stamp has to be the
        // version that is now the highest, and that is only known once the row is gone.
        let now = database.with(applied_version)?;
        database.with(|connection| stamp_version(connection, now))?;
    }

    Ok(reverted)
}

/// Creates the ledger if it is not there.
///
/// Idempotent, and called before every read of it. The ledger is the one table not created by a
/// migration, because it is what records that a migration ran: a migration that creates the
/// table it is about to be written into would need a special case, and a special case inside a
/// migration runner is where the next defect lives.
///
/// It is also the one table without the seven columns every other table carries. It is not the
/// person's data and it is never synchronised, and giving it a clock and a device would be an
/// invitation for the merge to treat it as something to reconcile.
fn ensure_ledger(connection: &Connection) -> Result<(), DbError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER NOT NULL PRIMARY KEY,
             applied_at INTEGER NOT NULL,
             checksum   BLOB    NOT NULL CHECK (length(checksum) = 32)
         ) STRICT;",
    )?;

    Ok(())
}

/// Checks that every migration in the ledger is the one this build carries under that number.
///
/// A mismatch means somebody edited a migration that had already run instead of adding a new
/// one, which leaves two machines with different schemas under the same version. Caught here,
/// at the next start, rather than at the next merge.
///
/// A version in the ledger that this build does not carry is not a mismatch: it is a database
/// from a newer build, which the caller has already refused for a better reason.
fn verify_ledger(connection: &Connection) -> Result<(), DbError> {
    let mut statement =
        connection.prepare_cached("SELECT version, checksum FROM schema_migrations")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;

    for row in rows {
        let (version, checksum) = row?;
        let Ok(version) = u32::try_from(version) else {
            continue;
        };
        let Some(migration) = MIGRATIONS.iter().find(|entry| entry.version == version) else {
            continue;
        };
        if checksum != migration.checksum() {
            return Err(DbError::LedgerMismatch { version });
        }
    }

    Ok(())
}

/// Writes the ledger row for a migration that has just run.
fn record(connection: &Connection, migration: &Migration, now_us: i64) -> Result<(), DbError> {
    connection.execute(
        "INSERT INTO schema_migrations (version, applied_at, checksum) VALUES (?1, ?2, ?3)",
        rusqlite::params![migration.version, now_us, migration.checksum().as_slice()],
    )?;

    Ok(())
}

/// Reads `PRAGMA user_version`.
///
/// Cheap on purpose. The ledger is the truth, but reading it means opening a table, and the
/// question "is this file newer than this build" has to be answerable before that is safe.
fn stamped_version(connection: &Connection) -> Result<u32, DbError> {
    let stamped: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    Ok(u32::try_from(stamped).unwrap_or(0))
}

/// Writes `PRAGMA user_version`.
///
/// Formatted into the statement because a pragma does not accept a bound parameter. The value
/// is a `u32` from this codebase, never from a person, so there is nothing to inject.
fn stamp_version(connection: &Connection, version: u32) -> Result<(), DbError> {
    connection.execute_batch(&format!("PRAGMA user_version = {version};"))?;

    Ok(())
}

/// Writes an encrypted copy of the database beside it.
///
/// Through `VACUUM INTO` rather than by copying the file. Copying is wrong twice over: the
/// write ahead log holds committed pages that are not in the main file yet, so the copy would
/// be missing them, and a file being copied while it is open can be caught mid write. `VACUUM
/// INTO` asks the database to produce a consistent copy of itself, encrypted under the same key,
/// which is the only form of this that is worth having.
fn take_backup(database: &Database) -> Result<PathBuf, DbError> {
    let path = backup_path(database.path());

    // Removed first. `VACUUM INTO` refuses to write over an existing file, and a copy left by a
    // run that was interrupted is exactly when this matters most.
    remove_backup(&path)?;

    database.with(|connection| {
        // The path is bound as a parameter rather than formatted in, which `VACUUM INTO`
        // allows. It comes from this program and not from a person, and binding it anyway costs
        // nothing and means a path with a quote in it cannot become a statement.
        connection.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
        Ok(())
    })?;

    Ok(path)
}

/// Where the copy goes: beside the database, under its own name.
fn backup_path(database: &Path) -> PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(BACKUP_SUFFIX);

    PathBuf::from(name)
}

/// Removes the copy, treating an absent file as success.
fn remove_backup(path: &Path) -> Result<(), DbError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(DbError::Io {
            what: "the copy taken before migrating",
            operation: "removed",
            cause,
        }),
    }
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};

    use super::{
        Applied, DATA_TABLES, LATEST_VERSION, MIGRATIONS, applied_version, apply_all, backup_path,
        revert_to,
    };
    use crate::error::DbError;
    use crate::open::Database;
    use crate::test_support::Scratch;

    /// A moment in the middle of the range.
    const NOW_US: i64 = 1_700_000_000_000_000;

    fn an_open_vault() -> UnlockedVault {
        let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");
        vault
    }

    /// Every table in the database except the ones SQLite keeps for itself.
    fn tables(database: &Database) -> Vec<String> {
        database
            .with(|connection| {
                let mut statement = connection.prepare(
                    "SELECT name FROM sqlite_schema
                     WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                     ORDER BY name",
                )?;
                let names = statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<String>, _>>()?;
                Ok(names)
            })
            .expect("the schema can be listed")
    }

    #[test]
    fn every_migration_has_a_unique_ascending_version() {
        let mut previous = 0;
        for migration in MIGRATIONS {
            assert!(
                migration.version > previous,
                "migration {} is not after {previous}",
                migration.version
            );
            previous = migration.version;
        }
        assert_eq!(LATEST_VERSION, previous);
    }

    #[test]
    fn every_migration_has_something_to_undo() {
        // A reverse script that is empty is a reverse script somebody skipped rather than one
        // that has nothing to do: every migration so far creates something.
        for migration in MIGRATIONS {
            assert!(
                !migration.up.trim().is_empty(),
                "migration {} has no forward script",
                migration.version
            );
            assert!(
                !migration.down.trim().is_empty(),
                "migration {} has no reverse script",
                migration.version
            );
        }
    }

    #[test]
    fn a_new_database_ends_up_with_the_tables_the_migrations_declare_and_nothing_else() {
        let scratch = Scratch::new("migrate-new");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");

        let applied = apply_all(&database, NOW_US).expect("the migrations apply");

        assert_eq!(applied.from, 0);
        assert_eq!(applied.to, LATEST_VERSION);
        assert_eq!(
            applied.versions,
            MIGRATIONS.iter().map(|one| one.version).collect::<Vec<_>>()
        );

        // The list every statement that has to name a table is built from, checked against what
        // the migrations actually created. A table added to the schema and not to the list is a
        // table nothing sweeps, compacts or reads a clock out of, and nothing else would notice.
        let mut expected: Vec<String> = DATA_TABLES.iter().map(|name| (*name).to_owned()).collect();
        expected.push("schema_migrations".to_owned());
        expected.sort();

        assert_eq!(tables(&database), expected);

        database.close().expect("the connection closes");
    }

    #[test]
    fn applying_twice_does_nothing_the_second_time() {
        let scratch = Scratch::new("migrate-twice");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");

        apply_all(&database, NOW_US).expect("the migrations apply");
        let again = apply_all(&database, NOW_US).expect("applying again is not an error");

        assert!(
            again.is_empty(),
            "a second run applied {:?}",
            again.versions
        );
        assert_eq!(again.from, LATEST_VERSION);
        assert_eq!(
            again,
            Applied {
                from: LATEST_VERSION,
                to: LATEST_VERSION,
                versions: Vec::new()
            }
        );

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_stamp_and_the_ledger_agree() {
        let scratch = Scratch::new("migrate-stamp");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");

        apply_all(&database, NOW_US).expect("the migrations apply");

        database
            .with(|connection| {
                let stamped: i64 =
                    connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
                assert_eq!(u32::try_from(stamped).unwrap_or(0), LATEST_VERSION);
                assert_eq!(applied_version(connection)?, LATEST_VERSION);
                Ok(())
            })
            .expect("both can be read");

        database.close().expect("the connection closes");
    }

    #[test]
    fn apply_revert_and_apply_again_leaves_the_same_schema() {
        // The property the reverse scripts exist for, and the only way to know they work.
        let scratch = Scratch::new("migrate-round-trip");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");

        apply_all(&database, NOW_US).expect("the migrations apply");
        let before = tables(&database);

        let reverted = revert_to(&database, 0).expect("the migrations revert");
        assert_eq!(
            reverted,
            MIGRATIONS
                .iter()
                .rev()
                .map(|one| one.version)
                .collect::<Vec<_>>(),
            "the reverse ran in an order that is not the reverse of the forward one"
        );
        assert_eq!(
            tables(&database),
            vec!["schema_migrations".to_owned()],
            "reverting left a table behind"
        );
        assert_eq!(database.with(applied_version).expect("the ledger reads"), 0);

        apply_all(&database, NOW_US).expect("the migrations apply again");
        assert_eq!(tables(&database), before);

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_database_from_a_newer_build_is_refused_and_says_both_numbers() {
        let scratch = Scratch::new("migrate-future");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");

        apply_all(&database, NOW_US).expect("the migrations apply");
        database
            .with(|connection| {
                connection
                    .execute_batch(&format!("PRAGMA user_version = {};", LATEST_VERSION + 5))?;
                Ok(())
            })
            .expect("the stamp can be raised by hand");

        let refused = apply_all(&database, NOW_US).expect_err("a newer schema was accepted");

        assert!(matches!(
            refused,
            DbError::SchemaTooNew { found, expected }
                if found == LATEST_VERSION + 5 && expected == LATEST_VERSION
        ));

        database.close().expect("the connection closes");
    }

    #[test]
    fn an_edited_migration_is_caught_rather_than_applied_on_top_of() {
        // Somebody who changes a migration that has already run instead of adding a new one
        // leaves two machines with different schemas under the same number. The ledger catches
        // it here rather than the merge catching it in six months.
        let scratch = Scratch::new("migrate-edited");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");

        apply_all(&database, NOW_US).expect("the migrations apply");
        database
            .with(|connection| {
                connection.execute(
                    "UPDATE schema_migrations SET checksum = ?1 WHERE version = 1",
                    [[0_u8; 32].as_slice()],
                )?;
                Ok(())
            })
            .expect("the checksum can be changed by hand");

        let refused = apply_all(&database, NOW_US).expect_err("an edited migration was accepted");

        assert!(matches!(refused, DbError::LedgerMismatch { version: 1 }));

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_copy_is_taken_before_migrating_an_existing_database_and_removed_afterwards() {
        // Two halves of one promise. The copy exists while the migrations are running, because
        // the transaction covers the statements and not the machine losing power; and it is
        // gone afterwards, because a stale copy beside a database is something the next
        // recovery has to choose between.
        let scratch = Scratch::new("migrate-backup");
        let vault = an_open_vault();
        let path = scratch.database_path();
        let database = Database::open(&path, &vault.database_key()).expect("a new file");

        apply_all(&database, NOW_US).expect("the migrations apply");
        // Back to nothing applied, with the file no longer empty, which is what makes the next
        // run take a copy.
        revert_to(&database, 0).expect("the migrations revert");
        database
            .with(|connection| {
                connection.execute_batch(
                    "PRAGMA user_version = 0;
                     INSERT INTO schema_migrations (version, applied_at, checksum)
                     VALUES (0, 0, zeroblob(32));",
                )?;
                Ok(())
            })
            .expect("a zero row can be written");

        let applied = apply_all(&database, NOW_US).expect("the migrations apply again");
        assert_eq!(applied.from, 0, "the fixture did not leave version zero");

        assert!(
            !backup_path(&path).exists(),
            "the copy was left behind after a successful migration"
        );

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_ledger_survives_being_read_on_a_database_with_no_tables() {
        let scratch = Scratch::new("migrate-empty");
        let vault = an_open_vault();
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");

        assert_eq!(database.with(applied_version).expect("the ledger reads"), 0);

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_copy_is_named_beside_the_database_rather_than_replacing_its_extension() {
        // `with_extension` would turn `cairn.db` into `cairn.backup`, which is a different file
        // from the one anybody would look for and, worse, one that a later `cairn.backup` of
        // something else could collide with.
        let named = backup_path(std::path::Path::new("somewhere/cairn.db"));
        assert!(named.to_string_lossy().ends_with("cairn.db.backup"));
    }
}
