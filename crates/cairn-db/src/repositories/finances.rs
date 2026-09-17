//! Accounts, categories, the movements between them, and the budgets they are measured against.
//!
//! Money is an integer in the smallest unit of its currency and never a fraction. Floating point
//! cannot represent a tenth, so a column of them adds up to a different total depending on the
//! order it was added in, and a total that changes when nothing changed is not an accounting
//! application.
//!
//! Only the note is sealed, here as in habits, and for the same reason written down the same way:
//! a monthly report that had to decrypt every row in Rust to add two numbers would not meet any
//! budget worth stating. The consequence is that the name of an account is protected by the
//! file's own encryption and nothing else.
//!
//! The rule that is particular to this module is that a movement is never edited. An edit writes
//! a new row pointing back at the old one, and the old one is marked deleted and gains a pointer
//! forward. A merge that overwrote money would lose a movement, and a lost movement is not
//! noticed until somebody balances the month — by which point there is nothing left to compare
//! against. Reading a movement therefore means reading the newest row of a chain, and the chain
//! survives a merge that only ever saw one end of it.
//!
//! Transfers are two rows, both of kind [`Kind::Transfer`], each pointing at the other, and
//! neither in a category. That keeps a balance per account as the plain sum of that account's own
//! rows, with no special case, and it keeps a transfer out of both sides of a report: moving money
//! between two accounts somebody owns is neither spending it nor earning it.

use cairn_domain::{CivilDay, Hlc, Rev};
use rusqlite::{Connection, OptionalExtension as _, params};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::codec::{FieldCodec, RowKey, SealedColumns};
use crate::device::DeviceId;
use crate::error::DbError;
use crate::row::{RowStamp, sixteen};

/// The table accounts live in.
pub const ACCOUNTS_TABLE: &str = "accounts";

/// The table categories live in.
pub const CATEGORIES_TABLE: &str = "categories";

/// The table movements live in.
pub const TRANSACTIONS_TABLE: &str = "transactions";

/// The encrypted columns of a movement.
pub const TRANSACTIONS_SEALED: SealedColumns = SealedColumns::new(&["note"]);

/// The table budgets live in.
pub const BUDGETS_TABLE: &str = "budgets";

/// The encrypted columns of a budget.
pub const BUDGETS_SEALED: SealedColumns = SealedColumns::new(&["note"]);

/// The longest a name may be, in characters, matching the constraint in the migration.
pub const MAX_NAME_LEN: usize = 120;

/// The largest a single amount may be, in the smallest unit of its currency.
///
/// A trillion minor units is ten thousand million euros, which is not a number anybody types by
/// hand. The ceiling is not about typing: [`totals`] adds amounts inside SQLite, and SQLite
/// answers an addition that leaves the range of a signed 64-bit integer with an error rather than
/// a wrong number. One row near the limit of the type would therefore turn every report that
/// covers its month into a refusal, and a file that arrives by synchronisation is not a file this
/// application wrote. The same number is a `CHECK` in the migration, so a row that comes in around
/// this code is refused by the file itself.
pub const MAX_AMOUNT: i64 = 1_000_000_000_000;

/// The most rows one page may hold.
///
/// A ceiling, not a default. The number that asks for a page arrives from the other side of the
/// bridge, and a number from a WebView does not get to decide how much memory this process
/// reserves.
pub const MAX_PAGE: usize = 200;

/// How many levels of category the design allows.
///
/// Two. A category and its subcategory, and nothing below that. Deeper trees of spending
/// categories are a way of never finishing the categorising, and a report over three levels is a
/// report nobody reads.
pub const MAX_CATEGORY_DEPTH: usize = 2;

/// What a movement does to an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Money left the account.
    Expense,
    /// Money arrived in the account.
    Income,
    /// Money moved to or from another account of the same person.
    ///
    /// Counts as neither spending nor earning. A transfer that landed in a category would make a
    /// month report twice what was spent.
    Transfer,
}

impl Kind {
    /// The number the column holds.
    #[must_use]
    pub const fn as_stored(self) -> i64 {
        match self {
            Self::Expense => 0,
            Self::Income => 1,
            Self::Transfer => 2,
        }
    }

    /// What a stored number means, or `None` for one this build does not know.
    #[must_use]
    pub const fn from_stored(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Expense),
            1 => Some(Self::Income),
            2 => Some(Self::Transfer),
            _ => None,
        }
    }
}

/// One movement, as it comes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    /// The row's identifier.
    pub id: Uuid,
    /// The account it belongs to.
    pub account_id: Uuid,
    /// The category, or `None` for a transfer.
    pub category_id: Option<Uuid>,
    /// Which way the money went.
    pub kind: Kind,
    /// How much, in the smallest unit of [`Self::currency`], always positive.
    pub amount: i64,
    /// The ISO 4217 code of the amount.
    pub currency: String,
    /// The day it happened, where the person was standing.
    pub occurred_on: CivilDay,
    /// The note, if it has one and has not been deleted.
    pub note: Option<Zeroizing<Vec<u8>>>,
    /// The other side of a transfer.
    pub transfer_id: Option<Uuid>,
    /// The row this one replaces.
    pub supersedes_id: Option<Uuid>,
    /// The row that replaced this one, which is set exactly when it is no longer current.
    pub superseded_by_id: Option<Uuid>,
    /// Whether it has been seen on a statement.
    pub cleared: bool,
    /// Whether it is a tombstone.
    pub deleted: bool,
    /// The clock reading of the last write, which is also where the next page starts.
    pub hlc: Hlc,
}

/// What is needed to write a movement down.
///
/// A struct rather than nine arguments, because most of them are identifiers and integers and a
/// caller that swaps two of them compiles perfectly.
#[derive(Debug, Clone, Copy)]
pub struct NewTransaction<'a> {
    /// Which account.
    pub account_id: Uuid,
    /// Which category, or nothing for a transfer.
    pub category_id: Option<Uuid>,
    /// Which way the money went.
    pub kind: Kind,
    /// How much, in the smallest unit of the currency, always positive.
    pub amount: i64,
    /// The ISO 4217 code.
    pub currency: &'a str,
    /// The day it happened.
    pub occurred_on: CivilDay,
    /// A note, or nothing.
    pub note: Option<&'a [u8]>,
    /// Whether it has already been seen on a statement.
    pub cleared: bool,
}

/// One movement of money between two accounts of the same person.
///
/// A struct for the same reason [`NewTransaction`] is one, and with an extra reason of its own:
/// the two clock readings are the same type and swapping them would put the outgoing half in the
/// destination account. Named, that is impossible to do silently.
#[derive(Debug, Clone, Copy)]
pub struct Transfer<'a> {
    /// The movement as it leaves the source account. Its kind and category are overridden.
    pub movement: NewTransaction<'a>,
    /// The account the money arrives in.
    pub destination: Uuid,
    /// The clock reading of the outgoing half.
    pub from_hlc: Hlc,
    /// The clock reading of the incoming half.
    pub to_hlc: Hlc,
}

/// What is needed to set a budget.
#[derive(Debug, Clone, Copy)]
pub struct NewBudget<'a> {
    /// Which category it limits.
    pub category_id: Uuid,
    /// Which month, as `YYYYMM`.
    pub period: u32,
    /// How much, in the smallest unit of the currency.
    pub amount: i64,
    /// The ISO 4217 code.
    pub currency: &'a str,
}

/// What is needed to open an account.
#[derive(Debug, Clone, Copy)]
pub struct NewAccount<'a> {
    /// What to call it.
    pub name: &'a str,
    /// What kind of account it is, as the schema numbers them.
    pub kind: i64,
    /// The ISO 4217 code it is held in.
    pub currency: &'a str,
    /// What it held before the first movement this application knows about.
    pub opening_balance: i64,
    /// Where it sits in the person's own order.
    pub position: i64,
}

/// One account, as it comes back. Nothing here is sealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// The row's identifier.
    pub id: Uuid,
    /// What it is called.
    pub name: String,
    /// What kind of account it is.
    pub kind: i64,
    /// The ISO 4217 code it is held in.
    pub currency: String,
    /// What it held before the first movement recorded here.
    pub opening_balance: i64,
    /// Where it sits in the person's own order.
    pub position: i64,
    /// Whether it is a tombstone.
    pub deleted: bool,
    /// The clock reading of the last write.
    pub hlc: Hlc,
}

/// Opens an account.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] for a name longer than [`MAX_NAME_LEN`] or a currency code that
/// is not three characters, and [`DbError::Sqlite`] if the statement fails.
pub fn create_account(
    connection: &Connection,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    account: NewAccount<'_>,
) -> Result<Account, DbError> {
    check_name("the name of an account", account.name)?;
    check_currency(account.currency)?;

    let stamp = RowStamp::new(device, hlc, now_us)?;
    connection
        .prepare_cached(
            "INSERT INTO accounts
                 (id, created_at, updated_at, device_id, deleted, hlc, rev,
                  name, kind, currency, opening_balance, position)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            account.name,
            account.kind,
            account.currency,
            account.opening_balance,
            account.position,
        ])?;

    Ok(Account {
        id: stamp.id,
        name: account.name.to_owned(),
        kind: account.kind,
        currency: account.currency.to_owned(),
        opening_balance: account.opening_balance,
        position: account.position,
        deleted: false,
        hlc: stamp.hlc,
    })
}

/// Writes a category down, refusing one deeper than [`MAX_CATEGORY_DEPTH`].
///
/// # Errors
///
/// Returns [`DbError::TooMany`] for a name too long or a parent that is itself a child,
/// [`DbError::NotFound`] if the parent is not there, and [`DbError::Sqlite`] if the statement
/// fails.
pub fn create_category(
    connection: &Connection,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    name: &str,
    kind: i64,
    parent_id: Option<Uuid>,
) -> Result<Uuid, DbError> {
    check_name("the name of a category", name)?;

    if let Some(parent) = parent_id {
        // Two levels, and only two. Checked by asking whether the parent has a parent, which is
        // one statement, rather than by walking a tree that by construction cannot be deep.
        let grandparent: Option<Option<Vec<u8>>> = connection
            .prepare_cached("SELECT parent_id FROM categories WHERE id = ?1 AND deleted = 0")?
            .query_row([parent.as_bytes().as_slice()], |row| row.get(0))
            .optional()?;

        match grandparent {
            None => return Err(DbError::NotFound),
            Some(Some(_deeper)) => {
                return Err(DbError::TooMany {
                    what: "the depth of a category",
                    value: MAX_CATEGORY_DEPTH as u64 + 1,
                    max: MAX_CATEGORY_DEPTH as u64,
                });
            }
            Some(None) => {}
        }
    }

    let stamp = RowStamp::new(device, hlc, now_us)?;
    connection
        .prepare_cached(
            "INSERT INTO categories
                 (id, created_at, updated_at, device_id, deleted, hlc, rev, name, kind, parent_id)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8)",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            name,
            kind,
            parent_id.map(|id| id.as_bytes().to_vec()),
        ])?;

    Ok(stamp.id)
}

/// Writes a movement down.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] for an amount outside zero to [`MAX_AMOUNT`] or a currency code
/// that is not three characters, [`DbError::Sealed`] if the note cannot be encrypted, and
/// [`DbError::Sqlite`] if the statement fails.
pub fn record(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    movement: NewTransaction<'_>,
) -> Result<Transaction, DbError> {
    insert_transaction(connection, codec, device, hlc, now_us, movement, None)
}

/// Replaces a movement with a corrected version of it, keeping both.
///
/// The old row is marked deleted, gains a pointer to the new one, and loses its note. The new row
/// points back. Nothing is overwritten, which is the whole rule: an edit that wrote over the
/// numbers would be a movement that cannot be recovered after a merge picked the wrong side.
///
/// # Errors
///
/// Returns [`DbError::NotFound`] if there is no current movement with that identifier, and
/// otherwise what [`record`] returns.
pub fn amend(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    id: Uuid,
    movement: NewTransaction<'_>,
) -> Result<Transaction, DbError> {
    let Some(stored) = connection
        .prepare_cached(
            "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev
               FROM transactions
              WHERE id = ?1 AND deleted = 0
              LIMIT 1",
        )?
        .query_row([id.as_bytes().as_slice()], RowStamp::read_common)
        .optional()?
    else {
        return Err(DbError::NotFound);
    };

    let replacement =
        insert_transaction(connection, codec, device, hlc, now_us, movement, Some(id))?;

    let gone = RowStamp::from_stored(stored)?.tombstoned(hlc, now_us);
    connection
        .prepare_cached(
            "UPDATE transactions
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = ?4,
                    note = NULL, superseded_by_id = ?5
              WHERE id = ?1",
        )?
        .execute(params![
            id.as_bytes().as_slice(),
            gone.updated_at,
            gone.hlc_as_stored().as_slice(),
            gone.rev_as_stored(),
            replacement.id.as_bytes().as_slice(),
        ])?;

    Ok(replacement)
}

/// Records a movement of money between two accounts, as the two rows it is.
///
/// Answers both identifiers, source first. Neither row has a category, and each points at the
/// other, so a balance per account stays the sum of that account's own rows.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] if the two accounts are the same, if the amount is outside zero
/// to [`MAX_AMOUNT`] or if the currency code is not three characters, [`DbError::Sealed`] if the
/// note cannot be encrypted, and [`DbError::Sqlite`] if a statement fails.
pub fn transfer(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    now_us: i64,
    transfer: Transfer<'_>,
) -> Result<(Uuid, Uuid), DbError> {
    let Transfer {
        movement,
        destination,
        from_hlc,
        to_hlc,
    } = transfer;

    if movement.account_id == destination {
        return Err(DbError::TooMany {
            what: "the number of distinct accounts a transfer needs",
            value: 1,
            max: 2,
        });
    }

    let out = insert_transaction(
        connection,
        codec,
        device,
        from_hlc,
        now_us,
        NewTransaction {
            kind: Kind::Transfer,
            category_id: None,
            ..movement
        },
        None,
    )?;
    let back = insert_transaction(
        connection,
        codec,
        device,
        to_hlc,
        now_us,
        NewTransaction {
            account_id: destination,
            kind: Kind::Transfer,
            category_id: None,
            ..movement
        },
        None,
    )?;

    point_at(connection, out.id, back.id)?;
    point_at(connection, back.id, out.id)?;

    Ok((out.id, back.id))
}

/// Reads one movement, answering `None` when it is not there or is no longer current.
///
/// # Errors
///
/// Returns [`DbError::Sealed`] if the row is not the shape the schema describes or its note does
/// not decrypt, and [`DbError::Sqlite`] if the statement fails.
pub fn transaction(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    id: Uuid,
) -> Result<Option<Transaction>, DbError> {
    let found = connection
        .prepare_cached(&format!(
            "{TRANSACTION_PROJECTION} WHERE id = ?1 AND deleted = 0 LIMIT 1"
        ))?
        .query_row([id.as_bytes().as_slice()], read_transaction)
        .optional()?;

    found.map(|stored| decode(codec, stored)).transpose()
}

/// A page of the movements of one account, oldest day first.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] if more than [`MAX_PAGE`] rows are asked for, [`DbError::Sealed`]
/// if a row does not decode, and [`DbError::Sqlite`] if the statement fails.
pub fn ledger(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    account_id: Uuid,
    after: Option<Hlc>,
    limit: usize,
) -> Result<Vec<Transaction>, DbError> {
    if limit == 0 || limit > MAX_PAGE {
        return Err(DbError::TooMany {
            what: "the size of a page of movements",
            value: limit as u64,
            max: MAX_PAGE as u64,
        });
    }

    let start = after.map_or([0_u8; 16], Hlc::to_bytes);
    let mut statement = connection.prepare_cached(&format!(
        "{TRANSACTION_PROJECTION}
          WHERE account_id = ?1 AND deleted = 0 AND hlc > ?2
          ORDER BY hlc
          LIMIT ?3"
    ))?;

    let rows = statement
        .query_map(
            params![
                account_id.as_bytes().as_slice(),
                start.as_slice(),
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            read_transaction,
        )?
        .collect::<Result<Vec<_>, _>>()?;

    rows.into_iter()
        .map(|stored| decode(codec, stored))
        .collect()
}

/// What was spent and what came in over a range of days, in the smallest unit.
///
/// Transfers are excluded from both halves, which is the point of counting them as their own
/// kind: a report that included them would say a month spent twice what it did.
///
/// # Errors
///
/// Returns [`DbError::Sqlite`] if the statement fails.
pub fn totals(
    connection: &Connection,
    from: CivilDay,
    to: CivilDay,
) -> Result<(i64, i64), DbError> {
    let mut statement = connection.prepare_cached(
        "SELECT
             coalesce(sum(CASE WHEN kind = 0 THEN amount ELSE 0 END), 0),
             coalesce(sum(CASE WHEN kind = 1 THEN amount ELSE 0 END), 0)
           FROM transactions
          WHERE deleted = 0 AND kind <> 2 AND occurred_on BETWEEN ?1 AND ?2",
    )?;

    let totals = statement.query_row(params![from.as_number(), to.as_number()], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;

    Ok(totals)
}

/// Marks a movement as deleted and empties its note.
///
/// # Errors
///
/// Returns [`DbError::NotFound`] if there is no current movement with that identifier, and
/// [`DbError::Sqlite`] if the statement fails.
pub fn delete_transaction(
    connection: &Connection,
    hlc: Hlc,
    now_us: i64,
    id: Uuid,
) -> Result<(), DbError> {
    let Some(stored) = connection
        .prepare_cached(
            "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev
               FROM transactions
              WHERE id = ?1 AND deleted = 0
              LIMIT 1",
        )?
        .query_row([id.as_bytes().as_slice()], RowStamp::read_common)
        .optional()?
    else {
        return Err(DbError::NotFound);
    };

    let gone = RowStamp::from_stored(stored)?.tombstoned(hlc, now_us);
    connection
        .prepare_cached(
            "UPDATE transactions
                SET deleted = 1, updated_at = ?2, hlc = ?3, rev = ?4, note = NULL
              WHERE id = ?1",
        )?
        .execute(params![
            id.as_bytes().as_slice(),
            gone.updated_at,
            gone.hlc_as_stored().as_slice(),
            gone.rev_as_stored(),
        ])?;

    Ok(())
}

/// Sets the budget of a category for one month, replacing whatever was there.
///
/// # Errors
///
/// Returns [`DbError::TooMany`] for an amount outside zero to [`MAX_AMOUNT`] or a currency code
/// that is not three characters, [`DbError::Sealed`] if the note cannot be encrypted, and
/// [`DbError::Sqlite`] if the statement fails.
pub fn set_budget(
    connection: &Connection,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    budget: NewBudget<'_>,
) -> Result<Uuid, DbError> {
    let NewBudget {
        category_id,
        period,
        amount,
        currency,
    } = budget;

    check_amount(amount)?;
    check_currency(currency)?;

    // The partial unique index would refuse a second live row for the same month, so the one
    // already there is retired first. Retiring rather than updating keeps the shape every other
    // table has, and keeps a merge able to see that the old figure existed.
    if let Some(previous) = connection
        .prepare_cached(
            "SELECT id, created_at, updated_at, device_id, deleted, hlc, rev
               FROM budgets
              WHERE category_id = ?1 AND period = ?2 AND deleted = 0
              LIMIT 1",
        )?
        .query_row(
            params![category_id.as_bytes().as_slice(), period],
            RowStamp::read_common,
        )
        .optional()?
    {
        let gone = RowStamp::from_stored(previous)?.tombstoned(hlc, now_us);
        connection
            .prepare_cached(
                "UPDATE budgets
                    SET deleted = 1, updated_at = ?2, hlc = ?3, rev = ?4, note = NULL
                  WHERE id = ?1",
            )?
            .execute(params![
                gone.id.as_bytes().as_slice(),
                gone.updated_at,
                gone.hlc_as_stored().as_slice(),
                gone.rev_as_stored(),
            ])?;
    }

    let stamp = RowStamp::new(device, hlc, now_us)?;
    connection
        .prepare_cached(
            "INSERT INTO budgets
                 (id, created_at, updated_at, device_id, deleted, hlc, rev,
                  category_id, period, amount, currency)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            category_id.as_bytes().as_slice(),
            period,
            amount,
            currency,
        ])?;

    Ok(stamp.id)
}

/// The columns every movement query reads, in the order [`read_transaction`] expects them.
const TRANSACTION_PROJECTION: &str = "SELECT id, hlc, rev, deleted, account_id, category_id, \
                                      kind, amount, currency, occurred_on, note, transfer_id, \
                                      supersedes_id, superseded_by_id, cleared FROM transactions";

/// One movement exactly as the projection hands it back.
type StoredTransaction = (
    Vec<u8>,
    Vec<u8>,
    i64,
    i64,
    Vec<u8>,
    Option<Vec<u8>>,
    i64,
    i64,
    String,
    u32,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    i64,
);

/// Reads the projection above.
fn read_transaction(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredTransaction> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
    ))
}

/// Checks a stored movement and opens its one encrypted column.
fn decode(codec: &FieldCodec<'_>, stored: StoredTransaction) -> Result<Transaction, DbError> {
    let (
        id,
        hlc,
        rev,
        deleted,
        account,
        category,
        kind,
        amount,
        currency,
        occurred_on,
        note,
        transfer,
        supersedes,
        superseded_by,
        cleared,
    ) = stored;

    let id = Uuid::from_bytes(sixteen(&id)?);
    let rev = Rev::from_number(u64::try_from(rev).map_err(|_negative| damaged())?);
    let row = RowKey {
        table: TRANSACTIONS_TABLE,
        row_id: id,
        rev,
    };

    Ok(Transaction {
        id,
        account_id: Uuid::from_bytes(sixteen(&account)?),
        category_id: identifier(category)?,
        kind: Kind::from_stored(kind).ok_or_else(damaged)?,
        amount,
        currency,
        occurred_on: CivilDay::from_number(occurred_on).map_err(|_not_a_day| damaged())?,
        note: note
            .map(|bytes| codec.open(row, "note", &bytes))
            .transpose()?,
        transfer_id: identifier(transfer)?,
        supersedes_id: identifier(supersedes)?,
        superseded_by_id: identifier(superseded_by)?,
        cleared: cleared != 0,
        deleted: deleted != 0,
        hlc: Hlc::from_bytes(sixteen(&hlc)?),
    })
}

/// Writes one movement, with or without a row it replaces.
fn insert_transaction(
    connection: &Connection,
    codec: &FieldCodec<'_>,
    device: DeviceId,
    hlc: Hlc,
    now_us: i64,
    movement: NewTransaction<'_>,
    supersedes: Option<Uuid>,
) -> Result<Transaction, DbError> {
    check_amount(movement.amount)?;
    check_currency(movement.currency)?;

    let stamp = RowStamp::new(device, hlc, now_us)?;
    let sealed = codec.seal_row(
        RowKey {
            table: TRANSACTIONS_TABLE,
            row_id: stamp.id,
            rev: stamp.rev,
        },
        TRANSACTIONS_SEALED,
        &[("note", movement.note)],
    )?;

    // A transfer carries no category whatever the caller passed. The schema refuses the
    // combination too, but a constraint failure says nothing about which of the two was wrong.
    let category = if movement.kind == Kind::Transfer {
        None
    } else {
        movement.category_id
    };

    connection
        .prepare_cached(
            "INSERT INTO transactions
                 (id, created_at, updated_at, device_id, deleted, hlc, rev,
                  account_id, category_id, kind, amount, currency, occurred_on, note,
                  supersedes_id, cleared)
             VALUES (?1, ?2, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )?
        .execute(params![
            stamp.id.as_bytes().as_slice(),
            stamp.created_at,
            stamp.device.as_bytes().as_slice(),
            stamp.hlc_as_stored().as_slice(),
            stamp.rev_as_stored(),
            movement.account_id.as_bytes().as_slice(),
            category.map(|id| id.as_bytes().to_vec()),
            movement.kind.as_stored(),
            movement.amount,
            movement.currency,
            movement.occurred_on.as_number(),
            sealed.first().and_then(Option::as_ref),
            supersedes.map(|id| id.as_bytes().to_vec()),
            i64::from(movement.cleared),
        ])?;

    Ok(Transaction {
        id: stamp.id,
        account_id: movement.account_id,
        category_id: category,
        kind: movement.kind,
        amount: movement.amount,
        currency: movement.currency.to_owned(),
        occurred_on: movement.occurred_on,
        note: movement.note.map(|bytes| Zeroizing::new(bytes.to_vec())),
        transfer_id: None,
        supersedes_id: supersedes,
        superseded_by_id: None,
        cleared: movement.cleared,
        deleted: false,
        hlc: stamp.hlc,
    })
}

/// Records which row is the other half of a transfer.
fn point_at(connection: &Connection, row: Uuid, other: Uuid) -> Result<(), DbError> {
    connection
        .prepare_cached("UPDATE transactions SET transfer_id = ?2 WHERE id = ?1")?
        .execute(params![
            row.as_bytes().as_slice(),
            other.as_bytes().as_slice()
        ])?;

    Ok(())
}

/// Turns sixteen stored bytes into an identifier, keeping absent as absent.
fn identifier(stored: Option<Vec<u8>>) -> Result<Option<Uuid>, DbError> {
    stored
        .map(|bytes| sixteen(&bytes).map(Uuid::from_bytes))
        .transpose()
}

/// Refuses a name the column would refuse, with a message that says which limit was passed.
fn check_name(what: &'static str, name: &str) -> Result<(), DbError> {
    let length = name.chars().count();
    if length == 0 || length > MAX_NAME_LEN {
        return Err(DbError::TooMany {
            what,
            value: length as u64,
            max: MAX_NAME_LEN as u64,
        });
    }

    Ok(())
}

/// Refuses anything that is not a three letter currency code.
fn check_currency(currency: &str) -> Result<(), DbError> {
    if currency.chars().count() != 3 {
        return Err(DbError::TooMany {
            what: "the length of a currency code",
            value: currency.chars().count() as u64,
            max: 3,
        });
    }

    Ok(())
}

/// Refuses a negative amount, because which way money moved is the kind and not the sign, and an
/// amount above [`MAX_AMOUNT`], because a number that large makes a report impossible to add up.
fn check_amount(amount: i64) -> Result<(), DbError> {
    if amount < 0 {
        return Err(DbError::TooMany {
            what: "the sign of an amount, which is carried by the kind and not by the number",
            value: 0,
            max: 0,
        });
    }

    if amount > MAX_AMOUNT {
        return Err(DbError::TooMany {
            what: "the size of an amount",
            value: amount.unsigned_abs(),
            max: MAX_AMOUNT.unsigned_abs(),
        });
    }

    Ok(())
}

/// What a row this application did not write is reported as.
fn damaged() -> DbError {
    DbError::Sealed(cairn_crypto::CryptoError::Open)
}
#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};
    use cairn_domain::{CivilDay, Hlc};
    use uuid::Uuid;

    use super::{
        Kind, MAX_AMOUNT, NewAccount, NewBudget, NewTransaction, Transfer, amend, create_account,
        create_category, delete_transaction, ledger, record, set_budget, totals, transaction,
    };
    use crate::codec::FieldCodec;
    use crate::device::DeviceId;
    use crate::error::DbError;
    use crate::migrations;
    use crate::open::Database;
    use crate::test_support::Scratch;

    const NOW_US: i64 = 1_700_000_000_000_000;

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

    fn at(step: u64) -> Hlc {
        Hlc::new(step, 0, [1; 6])
    }

    fn a_device() -> DeviceId {
        DeviceId::from_bytes([7; 16])
    }

    fn a_day() -> CivilDay {
        CivilDay::new(2026, 9, 17).expect("a day that exists")
    }

    fn an_account(name: &str) -> NewAccount<'_> {
        NewAccount {
            name,
            kind: 1,
            currency: "EUR",
            opening_balance: 0,
            position: 0,
        }
    }

    fn a_movement(account: Uuid, kind: Kind, amount: i64) -> NewTransaction<'static> {
        NewTransaction {
            account_id: account,
            category_id: None,
            kind,
            amount,
            currency: "EUR",
            occurred_on: CivilDay::new(2026, 9, 17).expect("a day that exists"),
            note: Some(b"la compra del mes"),
            cleared: false,
        }
    }

    #[test]
    fn what_is_written_comes_back() {
        let scratch = Scratch::new("finances-roundtrip");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let account = create_account(
                    connection,
                    a_device(),
                    at(1),
                    NOW_US,
                    an_account("Corriente"),
                )?;
                let written = record(
                    connection,
                    &codec,
                    a_device(),
                    at(2),
                    NOW_US,
                    a_movement(account.id, Kind::Expense, 9999),
                )?;

                let read = transaction(connection, &codec, written.id)?.expect("it is there");
                assert_eq!(read.amount, 9999);
                assert_eq!(read.currency, "EUR");
                assert_eq!(read.kind, Kind::Expense);
                assert_eq!(read.occurred_on, a_day());
                assert_eq!(
                    read.note.as_deref().map(Vec::as_slice),
                    Some(b"la compra del mes".as_slice())
                );
                Ok(())
            })
            .expect("the movement survives a round trip");

        database.close().expect("the connection closes");
    }

    /// Decision 14 of the phase, and the reason it is a decision: a merge that overwrites money
    /// loses a movement, and a lost movement is not noticed until somebody balances the month.
    #[test]
    fn editing_a_movement_writes_a_new_one_and_keeps_the_old_as_a_skeleton() {
        let scratch = Scratch::new("finances-amend");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let account =
                    create_account(connection, a_device(), at(1), NOW_US, an_account("Corriente"))?;
                let first = record(
                    connection,
                    &codec,
                    a_device(),
                    at(2),
                    NOW_US,
                    a_movement(account.id, Kind::Expense, 9999),
                )?;

                let corrected = amend(
                    connection,
                    &codec,
                    a_device(),
                    at(3),
                    NOW_US + 1,
                    first.id,
                    a_movement(account.id, Kind::Expense, 1234),
                )?;

                assert_ne!(corrected.id, first.id, "the edit reused the row");
                assert_eq!(corrected.supersedes_id, Some(first.id));
                assert_eq!(corrected.amount, 1234);

                // The old row is still there, still readable as a row, and no longer current.
                assert!(
                    transaction(connection, &codec, first.id)?.is_none(),
                    "the superseded row is still being handed out as current"
                );

                let (deleted, superseded_by, note): (i64, Option<Vec<u8>>, Option<Vec<u8>>) =
                    connection
                        .prepare_cached(
                            "SELECT deleted, superseded_by_id, note FROM transactions WHERE id = ?1",
                        )?
                        .query_row([first.id.as_bytes().as_slice()], |row| {
                            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                        })?;

                assert_eq!(deleted, 1);
                assert_eq!(
                    superseded_by.as_deref(),
                    Some(corrected.id.as_bytes().as_slice()),
                    "the old row does not point at what replaced it"
                );
                assert!(note.is_none(), "the superseded row kept its note");
                Ok(())
            })
            .expect("the amendment runs");

        database.close().expect("the connection closes");
    }

    /// The other rule this module has that no other module needs: a transfer is not spending.
    #[test]
    fn a_transfer_counts_as_neither_an_expense_nor_an_income() {
        let scratch = Scratch::new("finances-transfer");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let current = create_account(
                    connection,
                    a_device(),
                    at(1),
                    NOW_US,
                    an_account("Corriente"),
                )?;
                let savings =
                    create_account(connection, a_device(), at(2), NOW_US, an_account("Ahorro"))?;

                record(
                    connection,
                    &codec,
                    a_device(),
                    at(3),
                    NOW_US,
                    a_movement(current.id, Kind::Expense, 5000),
                )?;
                record(
                    connection,
                    &codec,
                    a_device(),
                    at(4),
                    NOW_US,
                    a_movement(current.id, Kind::Income, 200_000),
                )?;

                let (out, back) = super::transfer(
                    connection,
                    &codec,
                    a_device(),
                    NOW_US,
                    Transfer {
                        movement: a_movement(current.id, Kind::Transfer, 100_000),
                        destination: savings.id,
                        from_hlc: at(5),
                        to_hlc: at(6),
                    },
                )?;

                let (spent, earned) = totals(connection, a_day(), a_day())?;
                assert_eq!(spent, 5000, "the transfer was counted as spending");
                assert_eq!(earned, 200_000, "the transfer was counted as income");

                // Each half points at the other, so a balance per account is still the plain sum
                // of that account's own rows.
                let out_row = transaction(connection, &codec, out)?.expect("the outgoing half");
                let back_row = transaction(connection, &codec, back)?.expect("the incoming half");
                assert_eq!(out_row.transfer_id, Some(back));
                assert_eq!(back_row.transfer_id, Some(out));
                assert_eq!(back_row.account_id, savings.id);
                assert!(out_row.category_id.is_none());
                assert!(back_row.category_id.is_none());
                Ok(())
            })
            .expect("the transfer runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_transfer_to_the_same_account_is_refused() {
        let scratch = Scratch::new("finances-self-transfer");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let account = create_account(
                    connection,
                    a_device(),
                    at(1),
                    NOW_US,
                    an_account("Corriente"),
                )?;

                let refused = super::transfer(
                    connection,
                    &codec,
                    a_device(),
                    NOW_US,
                    Transfer {
                        movement: a_movement(account.id, Kind::Transfer, 100),
                        destination: account.id,
                        from_hlc: at(2),
                        to_hlc: at(3),
                    },
                );

                assert!(
                    matches!(refused, Err(DbError::TooMany { .. })),
                    "a transfer with one account was accepted"
                );
                Ok(())
            })
            .expect("the check runs");

        database.close().expect("the connection closes");
    }

    /// The ceiling is not about what anybody would type. SQLite answers an addition that leaves
    /// the range of a signed 64-bit integer with an error rather than a wrong number, so one row
    /// carrying a number near the limit of the type would turn every report that covers its month
    /// into a refusal, and there would be nothing in the report to say which row did it. The row
    /// is refused when it is written instead.
    #[test]
    fn an_amount_too_large_to_add_up_is_refused_rather_than_stored() {
        let scratch = Scratch::new("finances-ceiling");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let account = create_account(
                    connection,
                    a_device(),
                    at(1),
                    NOW_US,
                    an_account("Corriente"),
                )?;

                let refused = record(
                    connection,
                    &codec,
                    a_device(),
                    at(2),
                    NOW_US,
                    a_movement(account.id, Kind::Expense, MAX_AMOUNT + 1),
                );
                assert!(
                    matches!(refused, Err(DbError::TooMany { .. })),
                    "an amount past the ceiling was accepted"
                );

                // And the ceiling itself is allowed, several times over, because the point of the
                // number is that a month made of them still adds up.
                for step in 0..8 {
                    record(
                        connection,
                        &codec,
                        a_device(),
                        at(3 + step),
                        NOW_US,
                        a_movement(account.id, Kind::Expense, MAX_AMOUNT),
                    )?;
                }

                let (spent, earned) = totals(
                    connection,
                    CivilDay::new(2026, 9, 1).expect("a day that exists"),
                    CivilDay::new(2026, 9, 30).expect("a day that exists"),
                )?;
                assert_eq!(spent, MAX_AMOUNT * 8);
                assert_eq!(earned, 0);
                Ok(())
            })
            .expect("the check runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_category_three_levels_deep_is_refused() {
        let scratch = Scratch::new("finances-depth");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);

        database
            .with(|connection| {
                let top = create_category(connection, a_device(), at(1), NOW_US, "Casa", 0, None)?;
                let second = create_category(
                    connection,
                    a_device(),
                    at(2),
                    NOW_US,
                    "Suministros",
                    0,
                    Some(top),
                )?;

                let refused = create_category(
                    connection,
                    a_device(),
                    at(3),
                    NOW_US,
                    "Luz",
                    0,
                    Some(second),
                );
                assert!(
                    matches!(refused, Err(DbError::TooMany { .. })),
                    "a third level of category was accepted"
                );
                Ok(())
            })
            .expect("the check runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn deleting_a_movement_empties_its_note() {
        let scratch = Scratch::new("finances-delete");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let account = create_account(
                    connection,
                    a_device(),
                    at(1),
                    NOW_US,
                    an_account("Corriente"),
                )?;
                let written = record(
                    connection,
                    &codec,
                    a_device(),
                    at(2),
                    NOW_US,
                    a_movement(account.id, Kind::Expense, 500),
                )?;

                delete_transaction(connection, at(3), NOW_US + 1, written.id)?;

                let note: Option<Vec<u8>> = connection
                    .prepare_cached("SELECT note FROM transactions WHERE id = ?1")?
                    .query_row([written.id.as_bytes().as_slice()], |row| row.get(0))?;
                assert!(note.is_none(), "the tombstone kept its note");

                let (spent, _earned) = totals(connection, a_day(), a_day())?;
                assert_eq!(spent, 0, "a deleted movement is still being counted");
                Ok(())
            })
            .expect("the deletion runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_budget_set_twice_for_one_month_keeps_only_the_newer_one_alive() {
        let scratch = Scratch::new("finances-budget");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);

        database
            .with(|connection| {
                let category =
                    create_category(connection, a_device(), at(1), NOW_US, "Comida", 0, None)?;

                let first = set_budget(
                    connection,
                    a_device(),
                    at(2),
                    NOW_US,
                    NewBudget {
                        category_id: category,
                        period: 202_609,
                        amount: 30_000,
                        currency: "EUR",
                    },
                )?;
                let second = set_budget(
                    connection,
                    a_device(),
                    at(3),
                    NOW_US + 1,
                    NewBudget {
                        category_id: category,
                        period: 202_609,
                        amount: 40_000,
                        currency: "EUR",
                    },
                )?;
                assert_ne!(first, second);

                let live: i64 = connection
                    .prepare_cached(
                        "SELECT count(*) FROM budgets WHERE category_id = ?1 AND period = ?2 AND deleted = 0",
                    )?
                    .query_row(rusqlite::params![category.as_bytes().as_slice(), 202_609], |row| {
                        row.get(0)
                    })?;
                assert_eq!(live, 1, "the partial unique index is not doing its job");
                Ok(())
            })
            .expect("the budgets are written");

        database.close().expect("the connection closes");
    }

    #[test]
    fn a_page_of_a_ledger_walks_the_list_once_and_stops() {
        let scratch = Scratch::new("finances-page");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let account = create_account(
                    connection,
                    a_device(),
                    at(1),
                    NOW_US,
                    an_account("Corriente"),
                )?;
                for step in 0..5_u64 {
                    record(
                        connection,
                        &codec,
                        a_device(),
                        at(step + 2),
                        NOW_US,
                        a_movement(account.id, Kind::Expense, 100),
                    )?;
                }

                let first = ledger(connection, &codec, account.id, None, 2)?;
                assert_eq!(first.len(), 2);

                let last = first.last().expect("a page with two rows has a last one");
                let second = ledger(connection, &codec, account.id, Some(last.hlc), 2)?;
                assert_eq!(second.len(), 2);
                assert!(
                    second.iter().all(|row| row.hlc > last.hlc),
                    "the second page handed back something the first page had already given"
                );

                let refused = ledger(connection, &codec, account.id, None, super::MAX_PAGE + 1);
                assert!(
                    matches!(refused, Err(DbError::TooMany { .. })),
                    "a page bigger than the ceiling was accepted"
                );
                Ok(())
            })
            .expect("the paging runs");

        database.close().expect("the connection closes");
    }

    #[test]
    fn the_note_of_a_movement_is_not_in_the_file_in_the_clear() {
        let scratch = Scratch::new("finances-ciphertext");
        let vault = an_open_vault();
        let database = a_database(&scratch, &vault);
        let codec = FieldCodec::new(vault.data_key(), *vault.key_id());

        database
            .with(|connection| {
                let account = create_account(
                    connection,
                    a_device(),
                    at(1),
                    NOW_US,
                    an_account("Corriente"),
                )?;
                record(
                    connection,
                    &codec,
                    a_device(),
                    at(2),
                    NOW_US,
                    NewTransaction {
                        note: Some(b"una nota que nadie mas debe leer"),
                        ..a_movement(account.id, Kind::Expense, 100)
                    },
                )?;
                Ok(())
            })
            .expect("the movement is written");

        database.close().expect("the connection closes");

        let bytes = std::fs::read(scratch.database_path()).expect("the file can be read");
        assert!(
            !bytes
                .windows(b"una nota que nadie mas debe leer".len())
                .any(|window| window == b"una nota que nadie mas debe leer"),
            "the note is in the file in the clear"
        );
        // SQLCipher covers the name as well, and this asserts that rather than assuming it.
        assert!(
            !bytes
                .windows(b"Corriente".len())
                .any(|window| window == b"Corriente"),
            "the name of the account is in the file in the clear"
        );
    }
}
