//! The record of what was done with the data as a whole.
//!
//! Four operations move everything somebody owns at once: writing a backup, checking one,
//! restoring one, and exporting a module in the clear. Each one writes a row here. They are the
//! four a person needs to be able to look back at and ask "did I do that, and when", because a
//! vault that cannot answer that question is a vault where an export nobody remembers making
//! looks exactly like an export that did not happen.
//!
//! The kind is a closed enumeration of this crate's own constants, so nothing a person typed
//! reaches the clear column. Whatever is worth recording beyond that — a file name, a module
//! name — goes in `detail`, sealed, because a file name is usually a folder name and a folder
//! name is usually somebody's own name.
//!
//! Nothing here deletes. `record` writes and `recent` reads, and there is no third function on
//! purpose: a history with a way to remove one entry from it is a history nobody can rely on.

use cairn_domain::{Hlc, Rev};
use rusqlite::{Connection, params};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::codec::{FieldCodec, RowKey, SealedColumns};
use crate::device::DeviceId;
use crate::error::DbError;
use crate::row::{RowStamp, sixteen};

/// The table, as the schema names it.
pub const TABLE: &str = "audit_events";

/// The encrypted columns of this table.
pub const SEALED: SealedColumns = SealedColumns::new(&["detail"]);

/// The most rows one read may ask for.
///
/// A ceiling on what a caller can make this process hold at once, rather than a matter of taste.
/// A history screen shows a page; nobody reads ten thousand rows of it.
pub const MAX_PAGE: u32 = 500;

/// What happened.
///
/// An enumeration rather than a string, so the set of things that can be written here is decided
/// by this file and by nothing that arrives from anywhere else. The strings it maps to are the
/// stored form and are part of the schema: changing one would orphan every row already written
/// with the old spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventKind {
    /// A backup was written and verified.
    BackupExported,
    /// A backup was read end to end and found good.
    BackupVerified,
    /// A backup was restored over this database.
    BackupImported,
    /// One module was written out unencrypted.
    PlaintextExported,
}

impl EventKind {
    /// The stored spelling. Part of the schema, not a label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BackupExported => "backup.exported",
            Self::BackupVerified => "backup.verified",
            Self::BackupImported => "backup.imported",
            Self::PlaintextExported => "plaintext.exported",
        }
    }

    /// Reads a stored spelling back.
    ///
    /// Named `from_stored` rather than `from_str` because the standard trait of that name is
    /// fallible with an error type, and this is deliberately an `Option`: an unrecognised kind
    /// is not a parse failure, it is a row written by a newer build, and the caller skips it.
    ///
    /// Answers `None` for anything else, which is what a row written by a newer build looks
    /// like. The caller skips it rather than guessing: a history that renders an event it does
    /// not understand as one it does is worse than a history with a gap.
    #[must_use]
    pub fn from_stored(stored: &str) -> Option<Self> {
        match stored {
            "backup.exported" => Some(Self::BackupExported),
            "backup.verified" => Some(Self::BackupVerified),
            "backup.imported" => Some(Self::BackupImported),
            "plaintext.exported" => Some(Self::PlaintextExported),
            _unknown => None,
        }
    }
}

/// One thing that happened, as it comes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The row's identifier.
    pub id: Uuid,
    /// What happened.
    pub kind: EventKind,
    /// When it happened, in microseconds since the epoch, UTC.
    pub occurred_at: i64,
    /// Whatever was worth recording beyond the kind, decrypted, or `None` if there was nothing.
    ///
    /// The buffer clears itself when it is dropped, because this is decrypted content and it is
    /// very often a path.
    pub detail: Option<Zeroizing<Vec<u8>>>,
}

/// Writes one event.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if the detail cannot be encrypted and [`DbError::Sqlite`] if the
/// insert fails.
pub fn record(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    kind: EventKind,
    detail: Option<&[u8]>,
) -> Result<Event, DbError> {
    let stamp = RowStamp::new(device, hlc, now_us)?;
    let row = RowKey {
        table: TABLE,
        row_id: stamp.id,
        rev: stamp.rev,
    };

    let sealed = codec.seal_row(row, SEALED, &[("detail", detail)])?;
    let stored = sealed.first().and_then(Option::as_ref);

    connection
        .prepare_cached(
            "INSERT INTO audit_events
                 (id, created_at, updated_at, device_id, deleted, hlc, rev, kind, occurred_at, detail)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, ?9)",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.updated_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            kind.as_str(),
            now_us,
            stored,
        ])?;

    Ok(Event {
        id: stamp.id,
        kind,
        occurred_at: now_us,
        detail: detail.map(|bytes| Zeroizing::new(bytes.to_vec())),
    })
}

/// Reads the most recent events, newest first.
///
/// A row whose kind this build does not recognise is skipped rather than guessed at, so a
/// database written by a newer version reads as a history with a gap rather than as a history
/// with a wrong entry in it.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] if more than [`MAX_PAGE`] rows are asked for, [`DbError::Sealed`]
/// if a stored detail does not decrypt, and [`DbError::Sqlite`] if the statement fails.
pub fn recent(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    limit: u32,
) -> Result<Vec<Event>, DbError> {
    if limit > MAX_PAGE {
        return Err(DbError::TooMany {
            what: "audit events in one page",
            value: u64::from(limit),
            max: u64::from(MAX_PAGE),
        });
    }

    let mut statement = connection.prepare_cached(
        "SELECT id, kind, occurred_at, detail, rev FROM audit_events \
         WHERE deleted = 0 ORDER BY occurred_at DESC, id DESC LIMIT ?1",
    )?;

    let rows = statement.query_map(params![i64::from(limit)], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<Vec<u8>>>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;

    let mut found = Vec::new();
    for row in rows {
        let (id_bytes, stored_kind, occurred_at, sealed, rev) = row?;

        let Some(kind) = EventKind::from_stored(&stored_kind) else {
            continue;
        };

        let id = Uuid::from_bytes(sixteen(&id_bytes)?);
        // A revision the schema says cannot exist. Reported as a value that did not open,
        // because opening it at any revision this side can name would fail anyway.
        let rev = Rev::from_number(
            u64::try_from(rev)
                .map_err(|_negative| DbError::Sealed(cairn_crypto::CryptoError::Open))?,
        );
        let key = RowKey {
            table: TABLE,
            row_id: id,
            rev,
        };

        let detail = match sealed {
            Some(bytes) => Some(codec.open(key, "detail", &bytes)?),
            None => None,
        };

        found.push(Event {
            id,
            kind,
            occurred_at,
            detail,
        });
    }

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::{EventKind, MAX_PAGE, TABLE, recent, record};
    use crate::device::DeviceId;
    use crate::error::DbError;
    use crate::test_support::Sandbox;
    use cairn_domain::Hlc;

    /// The one installation these tests pretend to be.
    fn device() -> DeviceId {
        DeviceId::from_bytes([0xde; 16])
    }

    /// A clock reading that is enough for a test that writes one row at a time.
    fn reading(counter: u16) -> Hlc {
        Hlc::new(1_700_000_000_000, counter, device().tie_break())
    }

    #[test]
    fn an_event_comes_back_with_its_detail_readable() {
        let sandbox = Sandbox::new("audit-round-trip");
        let codec = sandbox.codec();

        sandbox
            .database()
            .with(|connection| {
                record(
                    connection,
                    &codec,
                    device(),
                    reading(1),
                    1_700_000_000_000_000,
                    EventKind::BackupExported,
                    Some(b"copia-2026.cairn"),
                )?;

                let found = recent(connection, &codec, 10)?;
                assert_eq!(found.len(), 1);
                assert_eq!(found[0].kind, EventKind::BackupExported);
                assert_eq!(
                    found[0].detail.as_deref().map(Vec::as_slice),
                    Some(&b"copia-2026.cairn"[..])
                );

                Ok(())
            })
            .expect("the round trip works");
    }

    #[test]
    fn an_event_with_nothing_to_add_is_different_from_one_with_an_empty_detail() {
        // A null and a zero length blob are different facts, and the difference survives.
        let sandbox = Sandbox::new("audit-null");
        let codec = sandbox.codec();

        sandbox
            .database()
            .with(|connection| {
                record(
                    connection,
                    &codec,
                    device(),
                    reading(1),
                    1_700_000_000_000_000,
                    EventKind::BackupVerified,
                    None,
                )?;
                record(
                    connection,
                    &codec,
                    device(),
                    reading(2),
                    1_700_000_000_000_001,
                    EventKind::BackupVerified,
                    Some(b""),
                )?;

                let found = recent(connection, &codec, 10)?;
                assert_eq!(found.len(), 2);
                assert!(found.iter().any(|event| event.detail.is_none()));
                assert!(
                    found
                        .iter()
                        .any(|event| event.detail.as_deref().map(Vec::as_slice) == Some(&b""[..]))
                );

                Ok(())
            })
            .expect("both shapes survive");
    }

    #[test]
    fn the_detail_is_not_readable_without_the_key() {
        // What the sealed column is for. The kind is in the file in the clear on purpose; the
        // detail is where a file name goes, and it must not be.
        let sandbox = Sandbox::new("audit-sealed");
        let codec = sandbox.codec();

        sandbox
            .database()
            .with(|connection| {
                record(
                    connection,
                    &codec,
                    device(),
                    reading(1),
                    1_700_000_000_000_000,
                    EventKind::PlaintextExported,
                    Some(b"finanzas-en-claro.csv"),
                )?;

                let stored: Vec<u8> =
                    connection.query_row("SELECT detail FROM audit_events LIMIT 1", [], |row| {
                        row.get(0)
                    })?;

                assert!(
                    !stored
                        .windows(b"finanzas".len())
                        .any(|window| window == b"finanzas"),
                    "the detail is readable in the stored bytes"
                );

                Ok(())
            })
            .expect("the read works");
    }

    #[test]
    fn a_kind_this_build_does_not_know_is_skipped_rather_than_guessed_at() {
        // What a database written by a newer version looks like. A history with a gap is worse
        // than no gap and much better than a history that renders an unknown event as a known
        // one.
        let sandbox = Sandbox::new("audit-unknown");
        let codec = sandbox.codec();

        sandbox
            .database()
            .with(|connection| {
                record(
                    connection,
                    &codec,
                    device(),
                    reading(1),
                    1_700_000_000_000_000,
                    EventKind::BackupImported,
                    None,
                )?;
                connection.execute("UPDATE audit_events SET kind = 'something.newer'", [])?;

                assert!(recent(connection, &codec, 10)?.is_empty());

                Ok(())
            })
            .expect("the read works");
    }

    #[test]
    fn a_page_larger_than_the_ceiling_is_refused() {
        let sandbox = Sandbox::new("audit-page");
        let codec = sandbox.codec();

        let refused = sandbox
            .database()
            .with(|connection| recent(connection, &codec, MAX_PAGE + 1))
            .expect_err("a page past the ceiling is refused");

        assert!(matches!(refused, DbError::TooMany { .. }), "{refused:?}");
    }

    #[test]
    fn the_stored_spellings_are_the_ones_the_schema_carries() {
        // Frozen. Changing one of these orphans every row already written with the old one, so
        // the test exists to make that a deliberate act rather than a rename.
        assert_eq!(EventKind::BackupExported.as_str(), "backup.exported");
        assert_eq!(EventKind::BackupVerified.as_str(), "backup.verified");
        assert_eq!(EventKind::BackupImported.as_str(), "backup.imported");
        assert_eq!(EventKind::PlaintextExported.as_str(), "plaintext.exported");

        for kind in [
            EventKind::BackupExported,
            EventKind::BackupVerified,
            EventKind::BackupImported,
            EventKind::PlaintextExported,
        ] {
            assert_eq!(EventKind::from_stored(kind.as_str()), Some(kind));
        }

        assert_eq!(EventKind::from_stored("backup.Exported"), None);
        assert_eq!(TABLE, "audit_events");
    }
}
