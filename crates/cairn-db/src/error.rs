//! The one error type this crate returns.
//!
//! One enum rather than one per module, for the same reason the cryptographic crate has one:
//! the layer above has to collapse several of these into a single message, and collapsing is
//! easier to get right when every case arrives through one door.
//!
//! Two things are deliberately not distinguished. A value that failed to decrypt says so and
//! nothing more, because the difference between a wrong key, associated data that does not
//! match and a flipped bit is an oracle. And a statement that SQLite refused keeps its cause
//! for a log and does not put it in the message, because the message reaches a screen.

use std::io;

use cairn_crypto::CryptoError;

/// Everything that can go wrong inside this crate.
///
/// Non-exhaustive on purpose: a variant added later must not break a caller that already
/// handles the ones it cares about.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DbError {
    /// SQLite refused the statement.
    ///
    /// The cause is kept as a source rather than put in the message. Whoever is repairing a
    /// machine can reach it; a person looking at a screen is told that storage failed.
    #[error("the database could not complete the operation")]
    Sqlite(#[source] rusqlite::Error),

    /// A stored value did not decrypt.
    ///
    /// Says nothing else, on purpose. The associated data binds a ciphertext to its table, its
    /// row, its column and its revision, so a value that fails here has been moved, rolled
    /// back or damaged, and which of those it was is not something a caller is told.
    #[error("a stored value did not decrypt")]
    Sealed(#[source] CryptoError),

    /// A row was offered to the sealer with a different set of columns than the table has.
    ///
    /// The check that makes "seal the whole row or seal nothing" real. Sealing half a row at a
    /// new revision would leave the other half authenticated under the old one, and the next
    /// read of those columns would fail with no way to tell why.
    #[error("the row was offered {found} encrypted columns and the table has {expected}")]
    RowColumns {
        /// How many were offered.
        found: usize,
        /// How many the table has.
        expected: usize,
    },

    /// An encrypted column of the table was not among the values offered, or was offered twice.
    ///
    /// The case the count alone cannot see: two values for one column and none for another
    /// satisfies a count and still leaves half the row written under the old revision. The name
    /// is one of the fixed column names of the schema, never text from a person.
    #[error("the encrypted column {column} was not offered exactly once")]
    ColumnNotOfferedOnce {
        /// The column, as the schema names it.
        column: &'static str,
    },

    /// A file beside the database could not be read or written.
    #[error("{what} could not be {operation}")]
    Io {
        /// Which file, named as the design names it rather than by its path.
        what: &'static str,
        /// What was being attempted, in a form that fits the sentence above.
        operation: &'static str,
        /// The underlying failure, kept so the cause is not lost on the way out.
        #[source]
        cause: io::Error,
    },

    /// The file on disk holds a schema this build does not know.
    ///
    /// Refused rather than opened hopefully. A newer build may have added a column this one
    /// does not write, and opening anyway would mean writing rows that the newer build then
    /// reads as incomplete.
    #[error("the database schema is version {found} and this build knows up to {expected}")]
    SchemaTooNew {
        /// What the file says.
        found: u32,
        /// The newest migration this build carries.
        expected: u32,
    },

    /// A migration in the ledger is not the migration this build carries under that number.
    ///
    /// Means somebody edited an applied migration instead of adding a new one. Carrying on
    /// would apply the remaining migrations to a schema that is not the one they were written
    /// against.
    #[error("migration {version} on disk is not the one this build carries")]
    LedgerMismatch {
        /// The version whose checksum did not match.
        version: u32,
    },

    /// The database is not open.
    ///
    /// The ordinary state while the vault is locked, and the reason every repository call can
    /// fail: the connection is closed when the vault closes, and nothing queues work for it.
    #[error("the database is not open")]
    Closed,

    /// A caller asked for more than one call may do.
    ///
    /// The bound is here and not in the caller because the caller is on the other side of the
    /// bridge. A number arriving from a WebView does not get to decide how much memory this
    /// process reserves.
    #[error("{what} is {value}, and at most {max} is allowed")]
    TooMany {
        /// What was being counted.
        what: &'static str,
        /// What was asked for.
        value: u64,
        /// The most that is allowed.
        max: u64,
    },

    /// There is no row with that identifier, or it is already a tombstone.
    #[error("there is no such row")]
    NotFound,
}

impl From<rusqlite::Error> for DbError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<CryptoError> for DbError {
    fn from(error: CryptoError) -> Self {
        Self::Sealed(error)
    }
}
