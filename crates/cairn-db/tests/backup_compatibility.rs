//! Proves that a backup written by an older build still opens.
//!
//! The file in `tests/fixtures/` was written once and is never written again. That is the
//! entire value of it: every other test in this crate exports and imports with the same
//! code in the same process, which proves the two halves agree with each other and proves
//! nothing at all about the bytes. This one has bytes that no current code produced.
//!
//! A backup is the thing people reach for on the worst day they have with this program. If
//! a change to the header layout, to the chunk framing, to the record stream or to the
//! compression settings makes last year's file unreadable, the only acceptable time to find
//! out is now, in a test, and not then.
//!
//! **Regenerating the fixture destroys the only compatibility proof this project has.** The
//! test that writes it is `#[ignore]`d so that it cannot run by accident, and the README
//! beside the file says the same thing to whoever finds it first. If this test fails, the
//! answer is almost always to fix the reader, not to rewrite the file. The one honest
//! reason to add a fixture is a deliberate new format version, and then it is added beside
//! this one, with this one still passing.
// Every function in an integration test file is test code, but the lint that forbids
// panicking constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]`
// functions. The helpers below are neither.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use uuid::Uuid;

use cairn_crypto::{Argon2Params, BACKUP_FORMAT_VERSION, MIN_MEMORY_KIB, MIN_PASSES};
use cairn_db::backup::base64;
use cairn_db::backup::export::write_backup;
use cairn_db::backup::format::{RECORD_VERSION, RowValues};
use cairn_db::backup::schema::{TABLES, table_named};
use cairn_db::backup::tables::write_row;
use cairn_db::backup::verify::{read_backup, verify_backup};
use cairn_db::{Database, DbError, FieldCodec};

/// The password the frozen file was made with.
///
/// In the source on purpose, and not a secret by any reading of the word: it protects six
/// invented rows in a file that is published in a public repository. A fixture whose
/// password lived somewhere else would be a fixture nobody could open, which is a fixture
/// that proves nothing.
const FIXTURE_PASSWORD: &str = "the frozen fixture opens with this";

/// What the file is called.
const FIXTURE_NAME: &str = "v1-schema-4.cairn";

/// The schema version the rows in the frozen file were read out of.
///
/// Frozen along with the file. It does not follow `LATEST_VERSION`, and the day those two
/// numbers differ is the day this test starts earning its keep.
const FIXTURE_SCHEMA_VERSION: u32 = 4;

/// How many rows of each table the frozen file carries, in the order it carries them.
///
/// Every table appears, including the empty ones. A table that is present with no rows and
/// a table that is missing are different files, and only one of them is this one.
const FIXTURE_RECORDS: &[(&str, u64)] = &[
    ("settings", 2),
    ("habit_areas", 1),
    ("habits", 1),
    ("habit_entries", 2),
    ("habit_pauses", 0),
    ("vault_folders", 1),
    ("vault_entries", 1),
    ("vault_urls", 0),
    ("vault_fields", 0),
    ("vault_password_history", 0),
    ("vault_tags", 0),
    ("vault_entry_tags", 0),
    ("accounts", 0),
    ("categories", 0),
    ("transactions", 0),
    ("budgets", 0),
];

/// The password stored in the one vault entry, as it must come back out.
///
/// Deliberately not ASCII. A change to how text is encoded on the way into the file would
/// otherwise pass on every string a programmer happens to type.
const FIXTURE_ENTRY_PASSWORD: &str = "contraseña-ñandú-€-clave";

/// Where the frozen file lives.
fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(FIXTURE_NAME)
}

#[test]
fn the_frozen_backup_still_opens_and_holds_what_it_held() {
    let report = verify_backup(&fixture_path(), FIXTURE_PASSWORD, &mut |_read| {})
        .expect("the frozen backup opens");

    assert_eq!(report.format_version, BACKUP_FORMAT_VERSION);
    assert_eq!(report.record_version, RECORD_VERSION);
    assert_eq!(report.schema_version, FIXTURE_SCHEMA_VERSION);

    let expected: Vec<(String, u64)> = FIXTURE_RECORDS
        .iter()
        .map(|(table, rows)| ((*table).to_owned(), *rows))
        .collect();
    assert_eq!(report.records, expected);
}

#[test]
fn the_frozen_backup_carries_every_table_this_build_knows_about() {
    // Not the same assertion as the one above. That one says the file is unchanged; this
    // one says the build has not grown a table the file cannot describe. Both can be true
    // on their own, and only together do they mean a restore loses nothing.
    let in_file: Vec<&str> = FIXTURE_RECORDS
        .iter()
        .map(|(table, _rows)| *table)
        .collect();
    let in_build: Vec<&str> = TABLES.iter().map(|table| table.name).collect();

    assert_eq!(
        in_file, in_build,
        "the frozen fixture predates a change to the table list; add a fixture, do not replace this one"
    );
}

#[test]
fn the_values_inside_the_frozen_backup_are_the_ones_that_went_in() {
    let mut passwords = Vec::new();
    let mut settings = BTreeMap::new();

    read_backup(
        &fixture_path(),
        FIXTURE_PASSWORD,
        &mut |_read| {},
        |table, values| {
            match table {
                "vault_entries" => passwords.push(decoded(&values, "password")),
                "settings" => {
                    let key = values
                        .get("key")
                        .and_then(Value::as_str)
                        .expect("a setting has a key")
                        .to_owned();
                    settings.insert(key, values.get("value").cloned());
                }
                _other => {}
            }
            Ok(())
        },
    )
    .expect("the frozen backup opens");

    assert_eq!(passwords, vec![FIXTURE_ENTRY_PASSWORD.as_bytes().to_vec()]);
    assert_eq!(settings.len(), 2);
    assert_eq!(
        settings
            .get("locale")
            .and_then(Option::as_ref)
            .and_then(Value::as_str)
            .map(|text| base64::decode(text).expect("a sealed value is base64")),
        Some(b"es-ES".to_vec())
    );
    // A cleared setting and a setting that was never written are different facts, and the
    // null has to survive the trip for the difference to mean anything.
    assert_eq!(settings.get("last_opened"), Some(&Some(Value::Null)));
}

#[test]
fn the_frozen_backup_uses_the_parameters_written_in_its_own_header() {
    // The point of putting the Argon2id parameters in the file rather than in a constant.
    // The fixture was made at the floor, the default has been higher since before it was
    // written, and it opens anyway. That is what says the default can be raised again
    // without stranding anybody's old backups.
    let bytes = fs::read(fixture_path()).expect("the frozen backup can be read");
    let header = cairn_crypto::BackupHeader::parse(
        bytes
            .get(..cairn_crypto::BACKUP_HEADER_LEN)
            .expect("the frozen backup is at least a header long"),
    )
    .expect("its header parses");

    assert_eq!(header.params().memory_kib(), MIN_MEMORY_KIB);
    assert_eq!(header.params().passes(), MIN_PASSES);
    assert_ne!(header.params(), Argon2Params::DEFAULT);
}

#[test]
fn the_wrong_password_does_not_open_the_frozen_backup() {
    let failure = verify_backup(&fixture_path(), "not the fixture password", &mut |_read| {})
        .expect_err("the wrong password is refused");

    assert!(
        matches!(failure, DbError::WrongPassword),
        "expected the wrong password, got {failure:?}"
    );
}

#[test]
#[ignore = "writes the frozen fixture; running it destroys the only compatibility proof this project has"]
fn regenerate_the_frozen_backup() {
    let scratch = Scratch::new("fixture");
    let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, 1).expect("the floor is accepted");
    let (_header, vault) =
        cairn_crypto::create(FIXTURE_PASSWORD, params, 0).expect("a vault can be created");

    let database = Database::open(
        &scratch.path().join(cairn_db::DATABASE_FILE),
        &vault.database_key(),
    )
    .expect("a database can be created");
    cairn_db::migrations::apply_all(&database, 1_700_000_000_000_000)
        .expect("the migrations apply");

    let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
    database
        .with(|connection| {
            seed(connection, &codec);
            Ok(())
        })
        .expect("the fixture rows are written");

    let destination = fixture_path();
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("the fixtures directory exists");
    }
    let _ignored = fs::remove_file(&destination);

    let (_export, verified) = database
        .with(|connection| {
            write_backup(
                connection,
                &codec,
                FIXTURE_SCHEMA_VERSION,
                FIXTURE_PASSWORD,
                params,
                &destination,
                &mut |_written| {},
            )
        })
        .expect("the fixture can be exported");

    assert_eq!(verified.schema_version, FIXTURE_SCHEMA_VERSION);
}

/// Writes the rows the frozen file carries.
///
/// Small on purpose. The fixture is about the shape of the file, not about volume: one row
/// of each interesting kind, a null where a null is meaningful, text that is not ASCII, and
/// a row that points at another row so the write order matters.
fn seed(connection: &rusqlite::Connection, codec: &FieldCodec<'_>) {
    let area = Uuid::from_bytes([0x11; 16]);
    let habit = Uuid::from_bytes([0x22; 16]);
    let folder = Uuid::from_bytes([0x33; 16]);
    let entry = Uuid::from_bytes([0x44; 16]);

    insert(
        connection,
        codec,
        "settings",
        Uuid::from_bytes([0x01; 16]),
        &[("key", text("locale")), ("value", blob(b"es-ES"))],
    );
    insert(
        connection,
        codec,
        "settings",
        Uuid::from_bytes([0x02; 16]),
        &[("key", text("last_opened")), ("value", Value::Null)],
    );

    insert(
        connection,
        codec,
        "habit_areas",
        area,
        &[
            ("name", text("Salud")),
            ("color", Value::Null),
            ("position", Value::from(0)),
        ],
    );

    insert(
        connection,
        codec,
        "habits",
        habit,
        &[
            ("name", text("Caminar")),
            ("notes", blob("treinta minutos, sin excusas".as_bytes())),
            ("icon", Value::Null),
            ("color", Value::Null),
            ("area_id", identifier(area)),
            ("kind", Value::from(0)),
            ("schedule_mask", Value::from(127)),
            ("target_per_period", Value::from(1)),
            ("unit", Value::Null),
            ("aggregation", Value::from(0)),
            ("direction", Value::from(0)),
            ("started_on", Value::from(20_240_101)),
            ("archived_at", Value::Null),
            ("position", Value::from(0)),
        ],
    );

    for (index, day) in [20_240_102_i64, 20_240_103].into_iter().enumerate() {
        let ordinal = u8::try_from(index).unwrap_or(0);
        insert(
            connection,
            codec,
            "habit_entries",
            Uuid::from_bytes([0x50 + ordinal; 16]),
            &[
                ("habit_id", identifier(habit)),
                ("day", Value::from(day)),
                ("amount", Value::from(1)),
                (
                    "note",
                    if index == 0 {
                        blob("llovía".as_bytes())
                    } else {
                        Value::Null
                    },
                ),
            ],
        );
    }

    insert(
        connection,
        codec,
        "vault_folders",
        folder,
        &[
            ("name", blob("Correo".as_bytes())),
            ("parent_id", Value::Null),
            ("position", Value::from(0)),
        ],
    );

    insert(
        connection,
        codec,
        "vault_entries",
        entry,
        &[
            ("title", blob("Cuenta de prueba".as_bytes())),
            ("username", blob("alguien".as_bytes())),
            ("password", blob(FIXTURE_ENTRY_PASSWORD.as_bytes())),
            ("notes", Value::Null),
            ("folder_id", identifier(folder)),
            ("favorite", Value::from(1)),
            ("last_used_at", Value::Null),
        ],
    );
}

/// Writes one row, filling the seven common columns the same way every time.
fn insert(
    connection: &rusqlite::Connection,
    codec: &FieldCodec<'_>,
    table: &str,
    row_id: Uuid,
    own: &[(&str, Value)],
) {
    let spec = table_named(table).expect("the table is one a backup carries");
    let stamped = 1_700_000_000_000_000_i64;

    let mut values = RowValues::new();
    values.insert("id".to_owned(), identifier(row_id));
    values.insert("created_at".to_owned(), Value::from(stamped));
    values.insert("updated_at".to_owned(), Value::from(stamped));
    values.insert(
        "device_id".to_owned(),
        identifier(Uuid::from_bytes([0xde; 16])),
    );
    values.insert("deleted".to_owned(), Value::from(0));
    values.insert("hlc".to_owned(), identifier(Uuid::from_bytes([0xc1; 16])));
    values.insert("rev".to_owned(), Value::from(1));

    for (name, value) in own {
        values.insert((*name).to_owned(), value.clone());
    }

    write_row(connection, codec, spec, &values).expect("the fixture row is written");
}

/// A blob column holding an identifier, as the record stream spells one.
fn identifier(value: Uuid) -> Value {
    Value::from(base64::encode(value.as_bytes()))
}

/// A blob or sealed column holding these bytes.
fn blob(bytes: &[u8]) -> Value {
    Value::from(base64::encode(bytes))
}

/// A text column.
fn text(value: &str) -> Value {
    Value::from(value)
}

/// The bytes behind one base64 column of a row that came out of the file.
fn decoded(values: &RowValues, column: &str) -> Vec<u8> {
    let encoded = values
        .get(column)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{column} is present and is text"));

    base64::decode(encoded).unwrap_or_else(|| panic!("{column} is base64"))
}

/// A temporary directory, removed when the value is dropped.
struct Scratch {
    directory: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("cairn-db-{label}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&directory).expect("a temporary directory can be created");

        Self { directory }
    }

    fn path(&self) -> &Path {
        &self.directory
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
