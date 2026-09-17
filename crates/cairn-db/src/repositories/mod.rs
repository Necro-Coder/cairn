//! Reading and writing rows, one module per group of tables.
//!
//! Three rules hold in every one of them, and they are the reason this is a layer rather than
//! statements scattered through the application.
//!
//! Every statement is prepared and cached. A statement compiled once and reused is the
//! difference between a query that meets the budget and one that spends its time parsing.
//!
//! Every projection is written out. There is no `SELECT *` anywhere: a column added later must
//! not silently start arriving in a row a caller is decoding by position.
//!
//! Every page is a keyset page. Paging with a growing offset makes the database count past
//! everything it has already handed out, so the last page of a long list costs the most, which
//! is the opposite of what anybody wants.

pub mod finances;
pub mod habits;
pub mod settings;
pub mod sync_state;
pub mod vault;
