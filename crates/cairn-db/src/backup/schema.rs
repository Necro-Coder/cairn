//! What a backup carries, table by table and column by column.
//!
//! This is a second description of the schema, and a second description of anything is a
//! thing that drifts. It exists anyway, for a reason that is worth stating: a backup has to
//! move every table, including the six that have no repository yet because the module that
//! will use them has not been built. Going through the repositories would mean a backup
//! that quietly leaves out whatever has not been given one, which is the sort of gap nobody
//! notices until a restore.
//!
//! The drift is dealt with rather than hoped away. A test asks SQLite for the columns of
//! every table in a freshly migrated database and compares them against this list, in
//! order, and it fails on a column added to a migration and not added here — and, just as
//! importantly, on one added here and not to a migration. There is no way to add a column
//! to the schema and have it silently left out of every backup.
//!
//! [`ColumnKind`] says how a value crosses the file. The distinction that matters is
//! [`ColumnKind::Sealed`]: those columns hold a ciphertext whose associated data names the
//! table, the row, the column and the revision it was written at, and none of that survives
//! the trip into another database. So a sealed value is decrypted on the way out and sealed
//! again, under the receiving vault's own key and at its own revision, on the way in. The
//! consequence is the one the security documentation has to state in plain words: inside a
//! decrypted backup there is no second layer, and the protection of the content is entirely
//! the password the file was made with.

/// How one column travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// A whole number. Travels as a JSON number.
    Integer,
    /// Text the schema stores in the clear, such as a currency code.
    Text,
    /// Bytes the schema stores in the clear: an identifier, a device, a clock reading.
    Blob,
    /// A value encrypted for this vault. Travels as its plaintext, base64 encoded.
    Sealed,
}

/// One column of one table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnSpec {
    /// The column, as the schema names it.
    pub name: &'static str,
    /// How it travels.
    pub kind: ColumnKind,
}

/// Declares an ordinary column.
const fn column(name: &'static str, kind: ColumnKind) -> ColumnSpec {
    ColumnSpec { name, kind }
}

/// One table of the backup.
///
/// Holds only the columns that belong to this table. The seven every table carries are in
/// [`COMMON`] and are joined on by [`TableSpec::columns`], rather than copied into each
/// declaration: one list to keep right instead of sixteen, and no way for one table to
/// disagree with the others about what a common column is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSpec {
    /// The table, as the schema names it.
    pub name: &'static str,
    /// The columns that are this table's own, after the seven common ones.
    own: &'static [ColumnSpec],
}

impl TableSpec {
    /// Every column, in the order the schema declares them: the seven common ones first.
    pub fn columns(&self) -> impl Iterator<Item = ColumnSpec> + use<'_> {
        COMMON.iter().chain(self.own).copied()
    }

    /// How many columns the table has.
    #[must_use]
    pub const fn column_count(&self) -> usize {
        COMMON.len() + self.own.len()
    }

    /// The encrypted columns of this table, in order.
    pub fn sealed_columns(&self) -> impl Iterator<Item = &'static str> + use<'_> {
        self.columns()
            .filter(|column| column.kind == ColumnKind::Sealed)
            .map(|column| column.name)
    }

    /// Looks a column up by name.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<ColumnSpec> {
        self.columns().find(|column| column.name == name)
    }
}

/// The seven columns every table carries, in the order the schema declares them.
///
/// Written out here as well as in [`crate::row::COMMON_COLUMNS`] because this list carries
/// the kind of each one, and the test below checks the two agree.
const COMMON: &[ColumnSpec] = &[
    column("id", ColumnKind::Blob),
    column("created_at", ColumnKind::Integer),
    column("updated_at", ColumnKind::Integer),
    column("device_id", ColumnKind::Blob),
    column("deleted", ColumnKind::Integer),
    column("hlc", ColumnKind::Blob),
    column("rev", ColumnKind::Integer),
];

/// Declares a table by its own columns.
macro_rules! table {
    ($name:literal, [$($column:expr),* $(,)?]) => {
        TableSpec {
            name: $name,
            own: &[$($column),*],
        }
    };
}

/// The tables a backup carries, in the order they are written and read.
///
/// The order is not cosmetic. It is the order rows are inserted into the staging database,
/// so a table that is pointed at comes before the tables that point at it, which keeps the
/// foreign key settings of the schema satisfiable even though the schema deliberately
/// declares almost none of them.
///
/// `sync_state` is not here, and its absence is a decision rather than an oversight. The
/// watermarks in it are what this device has seen of its peers. Restoring them on another
/// machine would make the merge skip records that machine has never been shown, which is
/// data loss that looks like nothing at all.
pub const TABLES: &[TableSpec] = &[
    table!(
        "settings",
        [
            column("key", ColumnKind::Text),
            column("value", ColumnKind::Sealed),
        ]
    ),
    table!(
        "habit_areas",
        [
            column("name", ColumnKind::Text),
            column("color", ColumnKind::Text),
            column("position", ColumnKind::Integer),
        ]
    ),
    table!(
        "habits",
        [
            column("name", ColumnKind::Text),
            column("notes", ColumnKind::Sealed),
            column("icon", ColumnKind::Text),
            column("color", ColumnKind::Text),
            column("area_id", ColumnKind::Blob),
            column("kind", ColumnKind::Integer),
            column("schedule_mask", ColumnKind::Integer),
            column("target_per_period", ColumnKind::Integer),
            column("unit", ColumnKind::Text),
            column("aggregation", ColumnKind::Integer),
            column("direction", ColumnKind::Integer),
            column("started_on", ColumnKind::Integer),
            column("archived_at", ColumnKind::Integer),
            column("position", ColumnKind::Integer),
        ]
    ),
    table!(
        "habit_entries",
        [
            column("habit_id", ColumnKind::Blob),
            column("day", ColumnKind::Integer),
            column("amount", ColumnKind::Integer),
            column("note", ColumnKind::Sealed),
        ]
    ),
    table!(
        "habit_pauses",
        [
            column("habit_id", ColumnKind::Blob),
            column("starts_on", ColumnKind::Integer),
            column("ends_on", ColumnKind::Integer),
            column("reason", ColumnKind::Sealed),
        ]
    ),
    table!(
        "vault_folders",
        [
            column("name", ColumnKind::Sealed),
            column("parent_id", ColumnKind::Blob),
            column("position", ColumnKind::Integer),
        ]
    ),
    table!(
        "vault_entries",
        [
            column("title", ColumnKind::Sealed),
            column("username", ColumnKind::Sealed),
            column("password", ColumnKind::Sealed),
            column("notes", ColumnKind::Sealed),
            column("folder_id", ColumnKind::Blob),
            column("favorite", ColumnKind::Integer),
            column("last_used_at", ColumnKind::Integer),
        ]
    ),
    table!(
        "vault_urls",
        [
            column("entry_id", ColumnKind::Blob),
            column("value", ColumnKind::Sealed),
            column("position", ColumnKind::Integer),
        ]
    ),
    table!(
        "vault_fields",
        [
            column("entry_id", ColumnKind::Blob),
            column("label", ColumnKind::Sealed),
            column("value", ColumnKind::Sealed),
            column("secret", ColumnKind::Integer),
            column("position", ColumnKind::Integer),
        ]
    ),
    table!(
        "vault_password_history",
        [
            column("entry_id", ColumnKind::Blob),
            column("password", ColumnKind::Sealed),
            column("replaced_at", ColumnKind::Integer),
        ]
    ),
    table!(
        "vault_tags",
        [
            column("name", ColumnKind::Sealed),
            column("color", ColumnKind::Text),
        ]
    ),
    table!(
        "vault_entry_tags",
        [
            column("entry_id", ColumnKind::Blob),
            column("tag_id", ColumnKind::Blob),
        ]
    ),
    table!(
        "accounts",
        [
            column("name", ColumnKind::Text),
            column("kind", ColumnKind::Integer),
            column("currency", ColumnKind::Text),
            column("opening_balance", ColumnKind::Integer),
            column("excluded", ColumnKind::Integer),
            column("color", ColumnKind::Text),
            column("icon", ColumnKind::Text),
            column("archived_at", ColumnKind::Integer),
            column("position", ColumnKind::Integer),
        ]
    ),
    table!(
        "categories",
        [
            column("name", ColumnKind::Text),
            column("kind", ColumnKind::Integer),
            column("parent_id", ColumnKind::Blob),
            column("color", ColumnKind::Text),
            column("icon", ColumnKind::Text),
            column("archived_at", ColumnKind::Integer),
            column("position", ColumnKind::Integer),
        ]
    ),
    table!(
        "transactions",
        [
            column("account_id", ColumnKind::Blob),
            column("category_id", ColumnKind::Blob),
            column("kind", ColumnKind::Integer),
            column("amount", ColumnKind::Integer),
            column("currency", ColumnKind::Text),
            column("occurred_on", ColumnKind::Integer),
            column("note", ColumnKind::Sealed),
            column("transfer_id", ColumnKind::Blob),
            column("supersedes_id", ColumnKind::Blob),
            column("superseded_by_id", ColumnKind::Blob),
            column("cleared", ColumnKind::Integer),
        ]
    ),
    table!(
        "budgets",
        [
            column("category_id", ColumnKind::Blob),
            column("period", ColumnKind::Integer),
            column("amount", ColumnKind::Integer),
            column("currency", ColumnKind::Text),
            column("rolls_over", ColumnKind::Integer),
            column("note", ColumnKind::Sealed),
        ]
    ),
];

/// The tables the schema has that a backup deliberately does not carry.
///
/// Named so that the test below can tell "left out on purpose" from "forgotten", which is
/// the whole difference between a decision and a bug.
pub const NOT_CARRIED: &[&str] = &["sync_state", "schema_migrations"];

/// Looks a table up by name.
#[must_use]
pub fn table_named(name: &str) -> Option<&'static TableSpec> {
    TABLES.iter().find(|table| table.name == name)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{ColumnKind, NOT_CARRIED, TABLES, table_named};
    use crate::row::COMMON_COLUMNS;
    use crate::test_support::migrated;

    #[test]
    fn every_table_starts_with_the_seven_common_columns() {
        for table in TABLES {
            let front: Vec<&str> = table
                .columns()
                .take(COMMON_COLUMNS.len())
                .map(|column| column.name)
                .collect();

            assert_eq!(
                front, COMMON_COLUMNS,
                "{} does not start with the common columns",
                table.name
            );
        }
    }

    #[test]
    fn no_table_is_listed_twice_and_no_column_is_listed_twice() {
        let mut seen_tables = std::collections::HashSet::new();
        for table in TABLES {
            assert!(
                seen_tables.insert(table.name),
                "{} is listed twice",
                table.name
            );

            let mut seen_columns = std::collections::HashSet::new();
            for column in table.columns() {
                assert!(
                    seen_columns.insert(column.name),
                    "{}.{} is listed twice",
                    table.name,
                    column.name
                );
            }
        }
    }

    /// Asks SQLite which columns a table really has, in declaration order.
    fn columns_of(connection: &Connection, table: &str) -> Vec<(String, String)> {
        let mut statement = connection
            .prepare("SELECT name, type FROM pragma_table_info(?1) ORDER BY cid")
            .unwrap();

        statement
            .query_map([table], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn the_list_of_tables_is_the_list_the_schema_has() {
        // The half that catches a table added to a migration and forgotten here, which
        // would be a table silently left out of every backup anybody ever makes.
        let sandbox = migrated();
        let found: Vec<String> = sandbox
            .database()
            .with(|connection| {
                let mut statement = connection
                    .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")?;
                let names = statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<String>, _>>()?;

                Ok(names)
            })
            .unwrap();

        for name in &found {
            if NOT_CARRIED.contains(&name.as_str()) || name.starts_with("sqlite_") {
                continue;
            }
            assert!(
                table_named(name).is_some(),
                "the schema has a table named {name} that no backup carries"
            );
        }

        for table in TABLES {
            assert!(
                found.iter().any(|name| name == table.name),
                "a backup carries {}, and the schema has no such table",
                table.name
            );
        }
    }

    #[test]
    fn every_column_of_every_table_is_described_the_way_the_schema_declares_it() {
        // The other half, one level down: a column added to a migration and not added here
        // travels as nothing, and a column described here with the wrong kind travels as
        // the wrong thing. Order is checked too, because the readers project by position.
        let sandbox = migrated();

        sandbox
            .database()
            .with(|connection| {
                for table in TABLES {
                    let declared = columns_of(connection, table.name);
                    let described: Vec<&str> = table.columns().map(|column| column.name).collect();
                    let actual: Vec<&str> =
                        declared.iter().map(|(name, _type)| name.as_str()).collect();

                    assert_eq!(
                        described, actual,
                        "the columns described for {} are not the ones it has",
                        table.name
                    );

                    for (column, (name, declared_type)) in table.columns().zip(&declared) {
                        let expected = match column.kind {
                            ColumnKind::Integer => "INTEGER",
                            ColumnKind::Text => "TEXT",
                            ColumnKind::Blob | ColumnKind::Sealed => "BLOB",
                        };
                        assert_eq!(
                            declared_type, expected,
                            "{}.{name} is declared {declared_type} and described as {expected}",
                            table.name
                        );
                    }
                }

                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn the_sealed_columns_are_the_ones_the_repositories_seal() {
        // The repositories are the other place that knows which columns are encrypted. Two
        // lists that disagree would mean a value sealed on the way in and copied out raw,
        // or the reverse, and either way the backup would carry something nobody can read.
        use crate::repositories::{finances, habits, settings, vault};

        for (table, sealed) in [
            ("settings", settings::SEALED),
            ("habits", habits::SEALED),
            ("habit_entries", habits::ENTRIES_SEALED),
            ("vault_entries", vault::ENTRIES_SEALED),
            ("vault_folders", vault::FOLDERS_SEALED),
            ("vault_password_history", vault::HISTORY_SEALED),
            ("transactions", finances::TRANSACTIONS_SEALED),
            ("budgets", finances::BUDGETS_SEALED),
        ] {
            let described: Vec<&str> = table_named(table).unwrap().sealed_columns().collect();
            assert_eq!(
                described,
                sealed.names(),
                "the sealed columns of {table} are described differently here"
            );
        }
    }

    #[test]
    fn synchronisation_state_is_left_out_on_purpose() {
        assert!(NOT_CARRIED.contains(&"sync_state"));
        assert!(table_named("sync_state").is_none());
    }

    #[test]
    fn a_table_that_is_pointed_at_comes_before_the_tables_that_point_at_it() {
        // Insertion order in the staging database. Habits before their entries, entries
        // before their fields, accounts before their movements.
        let position = |name: &str| {
            TABLES
                .iter()
                .position(|table| table.name == name)
                .unwrap_or(usize::MAX)
        };

        for (pointed_at, pointer) in [
            ("habit_areas", "habits"),
            ("habits", "habit_entries"),
            ("habits", "habit_pauses"),
            ("vault_folders", "vault_entries"),
            ("vault_entries", "vault_urls"),
            ("vault_entries", "vault_fields"),
            ("vault_entries", "vault_password_history"),
            ("vault_tags", "vault_entry_tags"),
            ("accounts", "transactions"),
            ("categories", "transactions"),
            ("categories", "budgets"),
        ] {
            assert!(
                position(pointed_at) < position(pointer),
                "{pointer} is written before {pointed_at}"
            );
        }
    }
}
