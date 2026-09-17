//! How far this device has got with each peer it has synchronised with.
//!
//! One row per peer, holding the highest clock reading received from it in the last exchange
//! that finished. The merge reads it to know where to start; nothing else reads it at all.
//!
//! Nothing here is encrypted, and that is deliberate. A watermark is a clock reading and a
//! device identifier, and the merge has to compare watermarks in SQL: sealing them would mean
//! decrypting every row of the table to answer a question the file's own encryption already
//! protects. The whole file is encrypted by SQLCipher; what the second layer adds is that a
//! value cannot be moved between rows, and moving a watermark between peers gains an attacker
//! a resynchronisation, not a secret.
//!
//! The watermark only moves forward. A peer that answers with an older reading than the one
//! already recorded does not roll this back, because the reading is what stops rows that have
//! already been merged from being asked for again, and lowering it is how a synchronisation
//! that never ends begins.

use rusqlite::{Connection, OptionalExtension as _, params};
use uuid::Uuid;

use crate::device::DeviceId;
use crate::error::DbError;
use crate::row::{RowStamp, sixteen};

/// The table, as the schema names it.
pub const TABLE: &str = "sync_state";

/// The most peers one listing will return.
///
/// A ceiling rather than an expectation. This is a personal vault with a handful of devices in
/// it; a query that could return an unbounded number of rows into memory is a query that only
/// behaves because nothing has gone wrong yet.
pub const MAX_PEERS: usize = 256;

/// How far this device has got with one peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerWatermark {
    /// The row's identifier.
    pub id: Uuid,
    /// The peer this is about.
    pub peer: DeviceId,
    /// The highest clock reading received from that peer, as sixteen ordered bytes.
    pub watermark: [u8; 16],
    /// When the last exchange with it finished, in microseconds since the epoch, UTC.
    pub synced_at: i64,
}

/// Records the result of a finished exchange with a peer.
///
/// Answers the watermark now in force, which is not always the one that was offered: a reading
/// that is not higher than the recorded one leaves the row alone and comes back unchanged.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if the identifier for a new row cannot be generated, and
/// [`DbError::Sqlite`] if the statement fails.
pub fn record(
    connection: &Connection,
    device: DeviceId,
    hlc: [u8; 16],
    now_us: i64,
    peer: DeviceId,
    watermark: [u8; 16],
) -> Result<PeerWatermark, DbError> {
    let existing = read(connection, peer)?;

    let (stamp, agreed) = match existing {
        Some((stamp, recorded)) => {
            // Byte comparison, and that is the point of the layout: sixteen bytes of hybrid
            // logical clock sort as bytes in the same order they sort as clocks, so this needs
            // no decoding and SQLite can do it in an index too.
            if watermark <= recorded {
                return Ok(PeerWatermark {
                    id: stamp.id,
                    peer,
                    watermark: recorded,
                    synced_at: stamp.updated_at,
                });
            }

            (stamp.revised(hlc, now_us), watermark)
        }
        None => (RowStamp::new(device, hlc, now_us)?, watermark),
    };

    connection
        .prepare_cached(
            "INSERT INTO sync_state
                 (id, created_at, updated_at, device_id, deleted, hlc, rev,
                  peer_device_id, watermark_hlc, synced_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, ?3)
             ON CONFLICT (id) DO UPDATE SET
                 updated_at    = excluded.updated_at,
                 device_id     = excluded.device_id,
                 deleted       = 0,
                 hlc           = excluded.hlc,
                 rev           = excluded.rev,
                 watermark_hlc = excluded.watermark_hlc,
                 synced_at     = excluded.synced_at",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.updated_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc.as_slice(),
            stamp.rev_as_stored(),
            peer.as_bytes().as_slice(),
            agreed.as_slice(),
        ])?;

    Ok(PeerWatermark {
        id: stamp.id,
        peer,
        watermark: agreed,
        synced_at: now_us,
    })
}

/// Reads how far this device has got with one peer, if it has ever finished an exchange with it.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if the stored row is not the shape the schema describes, and
/// [`DbError::Sqlite`] if the statement fails.
pub fn watermark(
    connection: &Connection,
    peer: DeviceId,
) -> Result<Option<PeerWatermark>, DbError> {
    Ok(
        read(connection, peer)?.map(|(stamp, recorded)| PeerWatermark {
            id: stamp.id,
            peer,
            watermark: recorded,
            synced_at: stamp.updated_at,
        }),
    )
}

/// Every peer this device has finished an exchange with, oldest identifier first.
///
/// Ordered by the peer identifier rather than by the moment, because the order has to be stable
/// between two calls that see the same rows, and two exchanges that finished in the same
/// microsecond would otherwise come back in whatever order the file happens to hold them.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] if there are more peers than [`MAX_PEERS`], which means
/// something is writing rows that this application does not write, [`DbError::Sealed`] if a
/// stored row is not the shape the schema describes, and [`DbError::Sqlite`] if the statement
/// fails.
pub fn peers(connection: &Connection) -> Result<Vec<PeerWatermark>, DbError> {
    let mut statement = connection.prepare_cached(
        "SELECT id, updated_at, peer_device_id, watermark_hlc, synced_at
           FROM sync_state
          WHERE deleted = 0
          ORDER BY peer_device_id
          LIMIT ?1",
    )?;

    // One more than the ceiling, so that too many rows is something this notices rather than
    // something it silently truncates and hands on as if it were the whole answer.
    let probe = i64::try_from(MAX_PEERS)
        .unwrap_or(i64::MAX)
        .saturating_add(1);
    let rows = statement
        .query_map([probe], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    if rows.len() > MAX_PEERS {
        return Err(DbError::TooMany {
            what: "the number of peers on record",
            value: rows.len() as u64,
            max: MAX_PEERS as u64,
        });
    }

    rows.into_iter()
        .map(|(id, updated_at, peer, recorded, synced_at)| {
            Ok(PeerWatermark {
                id: Uuid::from_bytes(sixteen(&id)?),
                peer: DeviceId::from_bytes(sixteen(&peer)?),
                watermark: sixteen(&recorded)?,
                // Equal in every row this program writes, and read separately anyway. A row
                // written by something else does not get to make the two disagree silently.
                synced_at: synced_at.min(updated_at),
            })
        })
        .collect()
}

/// Reads the live row for a peer, with the watermark it holds.
fn read(connection: &Connection, peer: DeviceId) -> Result<Option<(RowStamp, [u8; 16])>, DbError> {
    let found = connection
        .prepare_cached(
            "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev, watermark_hlc
               FROM sync_state
              WHERE peer_device_id = ?1 AND deleted = 0
              LIMIT 1",
        )?
        .query_row([peer.as_bytes().as_slice()], |row| {
            Ok((RowStamp::read_common(row)?, row.get::<_, Vec<u8>>(7)?))
        })
        .optional()?;

    let Some((stored, recorded)) = found else {
        return Ok(None);
    };

    Ok(Some((RowStamp::from_stored(stored)?, sixteen(&recorded)?)))
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};

    use super::{peers, record, watermark};
    use crate::device::DeviceId;
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

    fn a_database(scratch: &Scratch, vault: &UnlockedVault) -> Database {
        let database =
            Database::open(&scratch.database_path(), &vault.database_key()).expect("a new file");
        migrations::apply_all(&database, NOW_US).expect("the migrations apply");
        database
    }

    /// A watermark that is byte-wise higher than the one before it.
    fn reading(step: u8) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[0] = step;
        bytes
    }

    #[test]
    fn what_was_recorded_comes_back() {
        let scratch = Scratch::new("sync-round-trip");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let device = DeviceId::generate().unwrap();
        let peer = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let written = record(connection, device, HLC, NOW_US, peer, reading(3))?;
                assert_eq!(written.watermark, reading(3));

                let read = watermark(connection, peer)?.expect("it was just recorded");
                assert_eq!(read.peer, peer);
                assert_eq!(read.watermark, reading(3));
                assert_eq!(read.synced_at, NOW_US);
                Ok(())
            })
            .expect("the round trip works");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_peer_nobody_has_synchronised_with_has_no_watermark() {
        let scratch = Scratch::new("sync-unknown");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);

        database
            .with(|connection| {
                assert_eq!(watermark(connection, DeviceId::generate()?)?, None);
                assert!(peers(connection)?.is_empty());
                Ok(())
            })
            .expect("an empty table reads");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_higher_reading_moves_the_watermark_and_reuses_the_row() {
        let scratch = Scratch::new("sync-forward");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let device = DeviceId::generate().unwrap();
        let peer = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                let first = record(connection, device, HLC, NOW_US, peer, reading(3))?;
                let second = record(connection, device, LATER, NOW_US + 1, peer, reading(9))?;

                assert_eq!(first.id, second.id, "a second exchange made a second row");
                assert_eq!(second.watermark, reading(9));
                assert_eq!(second.synced_at, NOW_US + 1);

                let rows: i64 =
                    connection
                        .query_row("SELECT count(*) FROM sync_state", [], |row| row.get(0))?;
                assert_eq!(rows, 1);

                let rev: i64 =
                    connection.query_row("SELECT rev FROM sync_state", [], |row| row.get(0))?;
                assert_eq!(rev, 1, "the revision did not move");
                Ok(())
            })
            .expect("the watermark moves");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_lower_or_equal_reading_leaves_the_watermark_where_it_was() {
        // The invariant that keeps a synchronisation finite. A peer that answers with an older
        // reading than the one on record must not make this device ask for rows it has already
        // merged, and the shape that guarantees it is the watermark never going down.
        let scratch = Scratch::new("sync-backward");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let device = DeviceId::generate().unwrap();
        let peer = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                record(connection, device, HLC, NOW_US, peer, reading(9))?;

                for (step, moment) in [(3_u8, NOW_US + 1), (9, NOW_US + 2)] {
                    let answered = record(connection, device, LATER, moment, peer, reading(step))?;
                    assert_eq!(answered.watermark, reading(9));
                    assert_eq!(answered.synced_at, NOW_US, "the moment moved anyway");
                }

                let rev: i64 =
                    connection.query_row("SELECT rev FROM sync_state", [], |row| row.get(0))?;
                assert_eq!(rev, 0, "a refused reading still raised the revision");
                Ok(())
            })
            .expect("the watermark holds");

        database.close().expect("the connection closes");
    }

    #[test]
    fn two_peers_are_two_rows_and_do_not_see_each_other() {
        let scratch = Scratch::new("sync-two-peers");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let device = DeviceId::generate().unwrap();
        let first = DeviceId::generate().unwrap();
        let second = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                record(connection, device, HLC, NOW_US, first, reading(3))?;
                record(connection, device, HLC, NOW_US, second, reading(7))?;

                assert_eq!(
                    watermark(connection, first)?
                        .expect("the first peer")
                        .watermark,
                    reading(3)
                );
                assert_eq!(
                    watermark(connection, second)?
                        .expect("the second peer")
                        .watermark,
                    reading(7)
                );

                let listed = peers(connection)?;
                assert_eq!(listed.len(), 2);

                let mut expected = [first, second];
                expected.sort_by_key(|device| *device.as_bytes());
                assert_eq!(
                    listed.iter().map(|row| row.peer).collect::<Vec<_>>(),
                    expected.to_vec(),
                    "the listing is not ordered by peer identifier"
                );
                Ok(())
            })
            .expect("both peers are kept apart");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_watermark_is_stored_where_the_merge_can_compare_it() {
        // Unencrypted on purpose, and this test is the record of that decision. If somebody
        // seals the column later, this fails and they have to come and read the reasoning at
        // the top of this file rather than discovering the cost in a slow merge.
        let scratch = Scratch::new("sync-comparable");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let device = DeviceId::generate().unwrap();
        let peer = DeviceId::generate().unwrap();

        database
            .with(|connection| {
                record(connection, device, HLC, NOW_US, peer, reading(9))?;

                let above: i64 = connection.query_row(
                    "SELECT count(*) FROM sync_state WHERE watermark_hlc > ?1",
                    [reading(3).as_slice()],
                    |row| row.get(0),
                )?;
                assert_eq!(above, 1, "the watermark is not comparable in SQL");
                Ok(())
            })
            .expect("the comparison runs");

        database.close().expect("the connection closes");
    }
}
