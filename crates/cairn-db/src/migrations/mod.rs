//! Versioned, reversible changes to the schema, and the ledger that records them.
//!
//! Every migration is embedded in the binary rather than read from a directory beside it. A
//! build that carries its own schema cannot be handed a different one, and an installation
//! cannot lose the file that explains what its own database means.
//!
//! Four things make this safe rather than merely working.
//!
//! Each migration runs in a transaction, with its ledger row and the version stamp inside it.
//! A migration that fails halfway leaves a database at the version it was at, not at a version
//! that half exists.
//!
//! A copy of the file is taken before anything is applied, and removed when everything has
//! been. The transaction covers the statements; it does not cover the machine losing power in
//! the middle of a long backfill, and the two failures need different answers.
//!
//! A database from the future is refused. If the file says version nine and this build knows
//! up to seven, opening it would mean writing rows that a newer build reads as incomplete, so
//! it is not opened at all and the two numbers are reported.
//!
//! An applied migration is checksummed. Editing a migration that has already run, instead of
//! adding a new one, leaves every other machine with a different schema under the same number;
//! the ledger catches it at the next start rather than at the next merge.
//!
//! `PRAGMA user_version` mirrors the highest applied version. The ledger is the truth; the
//! pragma is what lets the "is this file newer than this build" question be answered without
//! parsing anything, which is what makes it cheap enough to ask before every open.

mod runner;

pub use runner::{
    Applied, DATA_TABLES, LATEST_VERSION, MIGRATIONS, Migration, applied_version, apply_all,
    revert_to,
};
