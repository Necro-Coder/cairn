//! The seven columns every table of this schema carries, and the values that go in them.
//!
//! Written once, here, rather than spelled out in each repository. Not for brevity: for the
//! property that every table treats them the same way. A table where `rev` is raised in one
//! place and forgotten in another is a table whose encrypted columns stop opening, and the
//! error it produces says a value did not decrypt and nothing about why.
//!
//! Nothing here reads a clock or invents an identifier. Both are arguments, as they are
//! everywhere else in this workspace, so that a row written in a test is a row whose every byte
//! the test chose.

use cairn_domain::{Hlc, Rev};
use rusqlite::Row;
use uuid::Uuid;

use crate::device::DeviceId;
use crate::error::DbError;

/// The columns every data table has, in the order the schema declares them.
///
/// A constant rather than a comment, so that a statement built from it and a table created from
/// the migration cannot drift apart without something failing.
pub const COMMON_COLUMNS: &[&str] = &[
    "id",
    "created_at",
    "updated_at",
    "device_id",
    "deleted",
    "hlc",
    "rev",
];

/// What the common columns hold for one row.
///
/// Carried together because they change together. Raising `rev` without writing a new `hlc`, or
/// writing either without touching `updated_at`, produces a row that two devices will order
/// differently, and the disagreement does not show up until they are merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowStamp {
    /// The row's identifier. A version four UUID, generated once and never reused.
    pub id: Uuid,
    /// When the row was first written, in microseconds since the epoch, UTC.
    pub created_at: i64,
    /// When it was last written.
    pub updated_at: i64,
    /// Which installation wrote it.
    pub device: DeviceId,
    /// Whether it is a tombstone.
    pub deleted: bool,
    /// The hybrid logical clock of the write.
    pub hlc: Hlc,
    /// The revision of the row, which every encrypted value in it is authenticated against.
    pub rev: Rev,
}

impl RowStamp {
    /// A stamp for a row being written for the first time.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Sealed`] if the operating system will not provide random bytes for the
    /// identifier. There is no fallback: an identifier somebody can predict is an identifier two
    /// devices can collide on.
    pub fn new(device: DeviceId, hlc: Hlc, now_us: i64) -> Result<Self, DbError> {
        let mut bytes = [0_u8; 16];
        cairn_crypto::fill_random(&mut bytes)?;

        Ok(Self {
            id: uuid::Builder::from_random_bytes(bytes).into_uuid(),
            created_at: now_us,
            updated_at: now_us,
            device,
            deleted: false,
            hlc,
            rev: Rev::FIRST,
        })
    }

    /// The stamp the same row carries after being written again.
    ///
    /// Raises the revision, replaces the clock, moves the moment and keeps everything else.
    #[must_use]
    pub fn revised(&self, hlc: Hlc, now_us: i64) -> Self {
        Self {
            updated_at: now_us,
            hlc,
            rev: self.rev.next(),
            ..*self
        }
    }

    /// The stamp the same row carries once it has been marked as deleted.
    #[must_use]
    pub fn tombstoned(&self, hlc: Hlc, now_us: i64) -> Self {
        Self {
            deleted: true,
            ..self.revised(hlc, now_us)
        }
    }

    /// The seven common columns as SQLite hands them back, before anything is checked.
    ///
    /// Read by position, in the order [`COMMON_COLUMNS`] names them, so every repository that
    /// projects those seven first can share this. Separate from [`RowStamp::from_stored`]
    /// because the two fail differently: this one fails when the statement is wrong, and that
    /// one fails when what is in the file is.
    ///
    /// # Errors
    ///
    /// Whatever `rusqlite` returns when a column is absent or holds another type.
    pub(crate) fn read_common(row: &Row<'_>) -> rusqlite::Result<StoredStamp> {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
        ))
    }

    /// Checks the seven stored values and builds the stamp they describe.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Sealed`] if an identifier, a device or a clock is not sixteen bytes
    /// long, or if the revision came back negative. All of those mean the row was written by
    /// something that is not this program, which is the same class of problem as a value that
    /// does not decrypt, and is answered the same way without saying which check failed.
    pub(crate) fn from_stored(stored: StoredStamp) -> Result<Self, DbError> {
        let (id, created_at, updated_at, device, deleted, hlc, rev) = stored;

        Ok(Self {
            id: Uuid::from_bytes(sixteen(&id)?),
            created_at,
            updated_at,
            device: DeviceId::from_bytes(sixteen(&device)?),
            deleted: deleted != 0,
            hlc: Hlc::from_bytes(sixteen(&hlc)?),
            rev: Rev::from_number(u64::try_from(rev).map_err(|_negative| damaged())?),
        })
    }

    /// The revision as SQLite stores it.
    #[must_use]
    pub fn rev_as_stored(&self) -> i64 {
        self.rev.as_stored()
    }

    /// The clock reading as SQLite stores it.
    #[must_use]
    pub fn hlc_as_stored(&self) -> [u8; 16] {
        self.hlc.to_bytes()
    }
}

/// The seven common columns exactly as SQLite hands them back.
///
/// A named alias rather than a bare tuple in three signatures, because the order is the order of
/// [`COMMON_COLUMNS`] and a reader has to be able to find that out from somewhere.
pub(crate) type StoredStamp = (Vec<u8>, i64, i64, Vec<u8>, i64, Vec<u8>, i64);

/// Reads sixteen bytes back out of a stored blob.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if there are not exactly sixteen.
pub(crate) fn sixteen(bytes: &[u8]) -> Result<[u8; 16], DbError> {
    bytes.try_into().map_err(|_wrong_length| damaged())
}

/// What a row this program did not write is reported as.
///
/// Says a value did not open, because from here that is indistinguishable from one that did
/// not, and telling the two apart gains a caller nothing.
fn damaged() -> DbError {
    DbError::Sealed(cairn_crypto::CryptoError::Open)
}

#[cfg(test)]
mod tests {
    use cairn_domain::{Hlc, Rev};

    use super::{COMMON_COLUMNS, RowStamp};
    use crate::device::DeviceId;

    const NOW_US: i64 = 1_700_000_000_000_000;
    const HLC: Hlc = Hlc::new(1_000, 0, [7; 6]);
    const LATER_HLC: Hlc = Hlc::new(1_001, 0, [7; 6]);

    fn a_stamp() -> RowStamp {
        RowStamp::new(DeviceId::generate().unwrap(), HLC, NOW_US).unwrap()
    }

    #[test]
    fn the_seven_columns_are_the_seven_the_schema_declares() {
        assert_eq!(COMMON_COLUMNS.len(), 7);
    }

    #[test]
    fn a_new_row_starts_at_revision_zero_and_is_not_a_tombstone() {
        let stamp = a_stamp();

        assert_eq!(stamp.rev, Rev::FIRST);
        assert!(!stamp.deleted);
        assert_eq!(stamp.created_at, stamp.updated_at);
        assert_eq!(stamp.id.get_version_num(), 4);
    }

    #[test]
    fn two_new_rows_have_different_identifiers() {
        assert_ne!(a_stamp().id, a_stamp().id);
    }

    #[test]
    fn revising_raises_the_revision_and_keeps_the_identity() {
        let first = a_stamp();
        let second = first.revised(LATER_HLC, NOW_US + 1);

        assert_eq!(second.id, first.id);
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(second.rev, Rev::FIRST.next());
        assert_eq!(second.updated_at, NOW_US + 1);
        assert_eq!(second.hlc, LATER_HLC);
        assert!(!second.deleted);
    }

    #[test]
    fn a_tombstone_is_a_revision_like_any_other() {
        // It has to be. A deletion that did not raise the revision would leave the encrypted
        // columns of the row still opening at the revision they were written at, which is
        // exactly the state the emptying is supposed to end.
        let first = a_stamp();
        let gone = first.tombstoned(LATER_HLC, NOW_US + 1);

        assert!(gone.deleted);
        assert_eq!(gone.rev, Rev::FIRST.next());
        assert_eq!(gone.id, first.id);
        assert_eq!(gone.hlc, LATER_HLC);
    }

    #[test]
    fn the_revision_never_goes_backwards_even_at_the_top_of_the_range() {
        let mut stamp = a_stamp();
        stamp.rev = Rev::from_number(u64::MAX);

        assert_eq!(
            stamp.revised(LATER_HLC, NOW_US).rev,
            Rev::from_number(u64::MAX)
        );
        assert_eq!(stamp.rev_as_stored(), i64::MAX);
    }
}
