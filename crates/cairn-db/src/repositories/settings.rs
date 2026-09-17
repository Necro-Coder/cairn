//! Preferences that have to survive a restart.
//!
//! Small and boring on purpose, and the first table with an encrypted column, which makes it the
//! place the pattern every other repository follows is easiest to read.
//!
//! The value is sealed. A preference sounds harmless until it is the name of the folder somebody
//! keeps their bank details in, or the last account they looked at, and deciding case by case
//! which preference is sensitive is a decision somebody eventually gets wrong.

use rusqlite::{Connection, OptionalExtension as _, params};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::codec::{FieldCodec, RowKey, SealedColumns};
use crate::device::DeviceId;
use crate::error::DbError;
use crate::row::{RowStamp, sixteen};

/// The table, as the schema names it.
pub const TABLE: &str = "settings";

/// The encrypted columns of this table.
pub const SEALED: SealedColumns = SealedColumns::new(&["value"]);

/// The longest a key may be, in characters.
///
/// Matches the constraint in the migration. Checked here as well, because a value refused by the
/// database arrives as a constraint failure with nothing useful in it, and a caller that is one
/// character over deserves to be told which limit it passed.
pub const MAX_KEY_LEN: usize = 64;

/// One preference, as it comes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    /// The row's identifier.
    pub id: Uuid,
    /// What the preference is called.
    pub key: String,
    /// What it is set to, or `None` if it has been cleared.
    ///
    /// Cleared and never written are different facts, and the schema keeps them apart. The
    /// buffer clears itself when it is dropped, because this is decrypted content.
    pub value: Option<Zeroizing<Vec<u8>>>,
}

/// Writes a preference, replacing whatever it was set to.
///
/// One row per key, reused rather than added to. A preference with a history is a preference that
/// answers differently depending on which row somebody reads, and there is no question anybody
/// asks that needs the previous value of a setting.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] for a key longer than [`MAX_KEY_LEN`], [`DbError::Sealed`] if the
/// value cannot be encrypted, and [`DbError::Sqlite`] if the statement fails.
pub fn put(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: [u8; 16],
    now_us: i64,
    key: &str,
    value: Option<&[u8]>,
) -> Result<Setting, DbError> {
    check_key(key)?;

    let existing = read_stamp(connection, key)?;
    let stamp = match existing {
        Some(stamp) => stamp.revised(hlc, now_us),
        None => RowStamp::new(device, hlc, now_us)?,
    };

    let row = RowKey {
        table: TABLE,
        row_id: stamp.id,
        rev: stamp.rev,
    };
    let sealed = codec.seal_row(row, SEALED, &[("value", value)])?;
    let stored = sealed.first().and_then(Option::as_ref);

    connection
        .prepare_cached(
            "INSERT INTO settings
                 (id, created_at, updated_at, device_id, deleted, hlc, rev, key, value)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8)
             ON CONFLICT (id) DO UPDATE SET
                 updated_at = excluded.updated_at,
                 device_id  = excluded.device_id,
                 deleted    = 0,
                 hlc        = excluded.hlc,
                 rev        = excluded.rev,
                 value      = excluded.value",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.updated_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc.as_slice(),
            stamp.rev_as_stored(),
            key,
            stored,
        ])?;

    Ok(Setting {
        id: stamp.id,
        key: key.to_owned(),
        value: value.map(|bytes| Zeroizing::new(bytes.to_vec())),
    })
}

/// Reads a preference, answering `None` when it has never been set or has been cleared away.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if the stored value does not decrypt where it was found, and
/// [`DbError::Sqlite`] if the statement fails.
pub fn get(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    key: &str,
) -> Result<Option<Setting>, DbError> {
    let found = connection
        .prepare_cached(
            "SELECT id, rev, value FROM settings WHERE key = ?1 AND deleted = 0 LIMIT 1",
        )?
        .query_row([key], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
            ))
        })
        .optional()?;

    let Some((id, rev, stored)) = found else {
        return Ok(None);
    };

    let id = Uuid::from_bytes(sixteen(&id)?);
    let rev = u64::try_from(rev).map_err(|_negative| {
        // A revision the schema says cannot exist. Reported as a value that did not open,
        // because opening it at any revision this side can name would fail anyway.
        DbError::Sealed(cairn_crypto::CryptoError::Open)
    })?;
    let value = stored
        .map(|bytes| {
            codec.open(
                RowKey {
                    table: TABLE,
                    row_id: id,
                    rev,
                },
                "value",
                &bytes,
            )
        })
        .transpose()?;

    Ok(Some(Setting {
        id,
        key: key.to_owned(),
        value,
    }))
}

/// Marks a preference as deleted and empties its encrypted column.
///
/// Emptying is the point. A tombstone that keeps its ciphertext for a hundred and eighty days is
/// not a deletion, it is a delay, and the person who cleared it has no way to know that.
///
/// # Errors
///
/// Returns [`DbError::NotFound`] if there is no live row with that key, and [`DbError::Sqlite`]
/// if the statement fails.
pub fn remove(
    connection: &Connection,
    hlc: [u8; 16],
    now_us: i64,
    key: &str,
) -> Result<(), DbError> {
    let Some(stamp) = read_stamp(connection, key)? else {
        return Err(DbError::NotFound);
    };
    let gone = stamp.tombstoned(hlc, now_us);

    connection
        .prepare_cached(
            "UPDATE settings
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = ?4, value = NULL
              WHERE id = ?1",
        )?
        .execute(params![
            gone.id.as_bytes().as_slice(),
            gone.updated_at,
            gone.hlc.as_slice(),
            gone.rev_as_stored(),
        ])?;

    Ok(())
}

/// Reads the common columns of the live row for a key.
fn read_stamp(connection: &Connection, key: &str) -> Result<Option<RowStamp>, DbError> {
    let found = connection
        .prepare_cached(
            "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev
               FROM settings
              WHERE key = ?1 AND deleted = 0
              LIMIT 1",
        )?
        .query_row([key], RowStamp::read_common)
        .optional()?;

    found.map(RowStamp::from_stored).transpose()
}

/// Refuses a key the schema would refuse, with a message that says which limit.
fn check_key(key: &str) -> Result<(), DbError> {
    if key.is_empty() || key.chars().count() > MAX_KEY_LEN {
        return Err(DbError::TooMany {
            what: "the length of a setting key",
            value: key.chars().count() as u64,
            max: MAX_KEY_LEN as u64,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};

    use super::{MAX_KEY_LEN, get, put, remove};
    use crate::codec::FieldCodec;
    use crate::device::DeviceId;
    use crate::error::DbError;
    use crate::migrations;
    use crate::open::Database;
    use crate::test_support::Scratch;

    const NOW_US: i64 = 1_700_000_000_000_000;
    const HLC: [u8; 16] = [1; 16];
    const LATER: [u8; 16] = [2; 16];

    fn an_open_vault() -> UnlockedVault {
        let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");
        vault
    }

    /// A migrated database in a directory of its own.
    fn a_database(scratch: &Scratch, vault: &UnlockedVault) -> Database {
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        database
    }

    #[test]
    fn what_is_written_comes_back() {
        let scratch = Scratch::new("settings-round-trip");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                put(
                    connection,
                    &codec,
                    device,
                    HLC,
                    NOW_US,
                    "theme",
                    Some(b"ink"),
                )?;
                let read = get(connection, &codec, "theme")?.expect("it was just written");
                assert_eq!(read.key, "theme");
                assert_eq!(
                    read.value.as_deref().map(Vec::as_slice),
                    Some(b"ink".as_slice())
                );
                Ok(())
            })
            .expect("the round trip works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_value_is_not_in_the_file_in_the_clear() {
        let scratch = Scratch::new("settings-opaque");
        let vault = an_open_vault();
        let path = scratch.database_path();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                put(
                    connection,
                    &codec,
                    DeviceId::generate()?,
                    HLC,
                    NOW_US,
                    "last-folder",
                    Some(b"cairn-canary-folder"),
                )?;
                Ok(())
            })
            .expect("the setting is written");
        database.close().expect("the connection closes");

        let raw = std::fs::read(&path).expect("the file is readable as bytes");
        let needle = b"cairn-canary-folder";
        assert!(
            !raw.windows(needle.len()).any(|window| window == needle),
            "the value appears verbatim in the file"
        );
    }

    #[test]
    fn writing_the_same_key_twice_replaces_it_and_raises_the_revision() {
        let scratch = Scratch::new("settings-replace");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let first = put(
                    connection,
                    &codec,
                    device,
                    HLC,
                    NOW_US,
                    "theme",
                    Some(b"ink"),
                )?;
                let second = put(
                    connection,
                    &codec,
                    device,
                    LATER,
                    NOW_US + 1,
                    "theme",
                    Some(b"paper"),
                )?;

                assert_eq!(first.id, second.id, "a second write made a second row");

                let rows: i64 =
                    connection.query_row("SELECT count(*) FROM settings", [], |row| row.get(0))?;
                assert_eq!(rows, 1);

                let rev: i64 = connection.query_row(
                    "SELECT rev FROM settings WHERE key = 'theme'",
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(rev, 1, "the revision did not move");

                let read = get(connection, &codec, "theme")?.expect("it is there");
                assert_eq!(
                    read.value.as_deref().map(Vec::as_slice),
                    Some(b"paper".as_slice()),
                    "the value read back at the new revision was the old one"
                );
                Ok(())
            })
            .expect("the second write works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_removed_setting_is_a_tombstone_with_nothing_inside_it() {
        // The whole of decision nine in one test. The row stays, so a merge can see that it was
        // deleted rather than never existing; the ciphertext does not, so a password cleared
        // today is not still in the file in six months.
        let scratch = Scratch::new("settings-remove");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                put(
                    connection,
                    &codec,
                    DeviceId::generate()?,
                    HLC,
                    NOW_US,
                    "theme",
                    Some(b"ink"),
                )?;
                remove(connection, LATER, NOW_US + 1, "theme")?;

                assert_eq!(get(connection, &codec, "theme")?, None);

                let (deleted, rev, value): (i64, i64, Option<Vec<u8>>) = connection.query_row(
                    "SELECT deleted, rev, value FROM settings WHERE key = 'theme'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
                assert_eq!(deleted, 1, "the row was removed rather than marked");
                assert_eq!(rev, 1, "the revision did not move");
                assert_eq!(value, None, "the tombstone kept its ciphertext");
                Ok(())
            })
            .expect("the removal works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_key_can_be_used_again_after_being_removed() {
        // What the partial unique index is for. A plain one would leave the tombstone holding
        // the name for ever, and setting the preference again would fail with a constraint
        // error nobody could act on.
        let scratch = Scratch::new("settings-reuse");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                put(
                    connection,
                    &codec,
                    device,
                    HLC,
                    NOW_US,
                    "theme",
                    Some(b"ink"),
                )?;
                remove(connection, LATER, NOW_US + 1, "theme")?;
                put(
                    connection,
                    &codec,
                    device,
                    LATER,
                    NOW_US + 2,
                    "theme",
                    Some(b"paper"),
                )?;

                let read = get(connection, &codec, "theme")?.expect("it is there again");
                assert_eq!(
                    read.value.as_deref().map(Vec::as_slice),
                    Some(b"paper".as_slice())
                );
                Ok(())
            })
            .expect("the key can be used again");

        database.close().expect("the connection closes");
    }

    #[test]
    fn removing_something_that_is_not_there_says_so() {
        let scratch = Scratch::new("settings-missing");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);

        let refused = database
            .with(|connection| remove(connection, HLC, NOW_US, "never-set"))
            .expect_err("removing nothing was reported as success");
        assert!(matches!(refused, DbError::NotFound));

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_cleared_value_and_a_value_that_was_never_written_are_different() {
        let scratch = Scratch::new("settings-null");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                put(
                    connection,
                    &codec,
                    DeviceId::generate()?,
                    HLC,
                    NOW_US,
                    "cleared",
                    None,
                )?;

                let read = get(connection, &codec, "cleared")?.expect("the row is there");
                assert_eq!(read.value, None, "an absent value came back as something");
                assert_eq!(
                    get(connection, &codec, "never-written")?,
                    None,
                    "a key nobody wrote came back as a row"
                );
                Ok(())
            })
            .expect("both cases read");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_key_outside_the_limits_is_refused_with_the_limit_in_the_message() {
        let scratch = Scratch::new("settings-key-limit");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());
        let device = DeviceId::generate().unwrap();
        let too_long = "k".repeat(MAX_KEY_LEN + 1);

        database
            .with(|connection| {
                for key in ["", too_long.as_str()] {
                    let refused = put(connection, &codec, device, HLC, NOW_US, key, Some(b"x"))
                        .expect_err("a key outside the limits was accepted");
                    assert!(matches!(refused, DbError::TooMany { .. }));
                }
                Ok(())
            })
            .expect("both are refused");

        database.close().expect("the connection closes");
    }
}
