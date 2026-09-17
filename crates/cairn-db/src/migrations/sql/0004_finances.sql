-- Migration 0004: accounts, categories, the movements between them, and the budgets.
--
-- Money is an integer in the smallest unit of its currency, never a fraction. Ninety nine euros
-- and ninety nine cents is 9999 with `EUR` beside it. Floating point cannot represent a tenth,
-- so a column of them adds up to a different total depending on the order they were added in,
-- and a total that changes when nothing changed is not an accounting application.
--
-- The currency travels with the amount rather than sitting on the account alone. An account is
-- in one currency, but a movement that was converted carries what it originally was, and the
-- pair is meaningless split up.
--
-- What is sealed follows the same rule as the rest of the schema, applied literally: the note of
-- a movement and the note of a budget are content and are sealed; names, amounts, dates, kinds
-- and positions are structural, because a monthly report that had to decrypt every row in Rust
-- to add two numbers would not meet any budget worth writing down. The consequence is the same
-- one habits took, and it is recorded in an ADR rather than in a comment nobody reads: the name
-- of an account can say as much as a note, and it is protected by the file's own encryption.
--
-- The one rule here that is not in any other module is that a movement is never edited. Editing
-- writes a new row that points back at the old one with `supersedes_id`, and the old one is
-- marked with `superseded_by_id` and with `deleted`. A merge that overwrites money loses a
-- movement, and a lost movement is not noticed until somebody balances the month.

CREATE TABLE accounts (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    name        TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    -- 0 cash, 1 current account, 2 savings, 3 card, 4 investment. An integer so a value outside
    -- the five is refused here rather than by whoever reads it next.
    kind        INTEGER NOT NULL DEFAULT 1 CHECK (kind IN (0, 1, 2, 3, 4)),
    -- ISO 4217, three letters upper case. Stored as text because that is what the code is, and
    -- checked for length because a currency of two letters is a typo that would otherwise only
    -- surface as a report that silently excludes an account.
    currency    TEXT    NOT NULL CHECK (length(currency) = 3),
    -- What the account held before the first movement this application knows about. Without it
    -- a balance is the sum of the movements since installation, which is not the balance.
    opening_balance INTEGER NOT NULL DEFAULT 0,
    -- 0 counts towards the net worth, 1 does not. A card somebody tracks for its statements but
    -- does not want double counted is the ordinary case.
    excluded    INTEGER NOT NULL DEFAULT 0 CHECK (excluded IN (0, 1)),
    color       TEXT    CHECK (color IS NULL OR length(color) BETWEEN 1 AND 32),
    icon        TEXT    CHECK (icon IS NULL OR length(icon) BETWEEN 1 AND 64),
    archived_at INTEGER,
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX accounts_sync ON accounts (deleted, updated_at);
CREATE INDEX accounts_hlc ON accounts (hlc);
CREATE INDEX accounts_live ON accounts (position) WHERE deleted = 0;

CREATE TABLE categories (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    name        TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    -- 0 expense, 1 income. A category belongs to one side or the other; a category that is both
    -- is two categories, and a report that has to guess which one a movement meant is a report
    -- nobody can check.
    kind        INTEGER NOT NULL DEFAULT 0 CHECK (kind IN (0, 1)),
    -- Two levels, and only two. No foreign key, for the same reason as everywhere else: a merge
    -- can deliver a child before its parent. That the parent is itself a root is checked in
    -- `cairn-domain`, where it can be proved over generated shapes rather than over the three
    -- somebody thought of.
    parent_id   BLOB    CHECK (parent_id IS NULL OR length(parent_id) = 16),
    color       TEXT    CHECK (color IS NULL OR length(color) BETWEEN 1 AND 32),
    icon        TEXT    CHECK (icon IS NULL OR length(icon) BETWEEN 1 AND 64),
    archived_at INTEGER,
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX categories_parent ON categories (parent_id, position) WHERE deleted = 0;
CREATE INDEX categories_sync ON categories (deleted, updated_at);
CREATE INDEX categories_hlc ON categories (hlc);

CREATE TABLE transactions (
    id              BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    device_id       BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted         INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc             BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev             INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    account_id      BLOB    NOT NULL CHECK (length(account_id) = 16),
    -- Null for a transfer, which belongs to no category on purpose: moving money between two
    -- accounts somebody owns is neither spending it nor earning it, and a transfer that lands in
    -- a category is a month that reports twice what was spent.
    category_id     BLOB    CHECK (category_id IS NULL OR length(category_id) = 16),
    -- 0 expense, 1 income, 2 transfer. The three are kept apart at the column rather than
    -- inferred from the sign, because a refund is a negative expense and not an income, and only
    -- the person entering it knows which.
    kind            INTEGER NOT NULL CHECK (kind IN (0, 1, 2)),
    -- In the smallest unit of `currency`, and always positive. Which way it moves is `kind`,
    -- not the sign: a sign and a kind that disagree is a row no report can read twice the same.
    --
    -- The ceiling is a trillion minor units, which is ten thousand million euros, and it is not
    -- about what anybody would type. SQLite raises an integer overflow rather than returning a
    -- wrong total, so one row carrying a number near the limit of the type would turn every
    -- monthly report into an error until somebody found it. A file that arrives by
    -- synchronisation is not a file this program wrote.
    amount          INTEGER NOT NULL CHECK (amount >= 0 AND amount <= 1000000000000),
    currency        TEXT    NOT NULL CHECK (length(currency) = 3),
    -- The day it happened, YYYYMMDD, in the place the person was standing. A square on a
    -- calendar and not an instant: a purchase at half past eleven at night belongs to that day
    -- wherever the file is later opened.
    occurred_on     INTEGER NOT NULL CHECK (occurred_on BETWEEN 10101 AND 99991231),
    -- Sealed. What somebody writes beside a movement is the one part of it that reads like a
    -- diary: who it was for, what it was about, why it was more than usual.
    note            BLOB,
    -- The other side of a transfer, which is the row in the destination account. Both rows
    -- exist, both are kind 2, and each points at the other, so a balance per account is still
    -- the sum of its own rows and needs no special case.
    transfer_id     BLOB    CHECK (transfer_id IS NULL OR length(transfer_id) = 16),
    -- What this row replaces, and what replaced it. A movement is never edited in place: an edit
    -- writes a new row pointing back with `supersedes_id`, and the old row is marked deleted and
    -- gains `superseded_by_id`, so the chain survives a merge that saw only one of the two.
    supersedes_id   BLOB    CHECK (supersedes_id IS NULL OR length(supersedes_id) = 16),
    superseded_by_id BLOB   CHECK (superseded_by_id IS NULL OR length(superseded_by_id) = 16),
    -- Whether the movement has been seen on a statement. Structural, because filtering by it is
    -- the whole of reconciling a month.
    cleared         INTEGER NOT NULL DEFAULT 0 CHECK (cleared IN (0, 1)),

    -- A transfer has no category and everything else has to have somewhere to go. Written here
    -- rather than left to the repository because it is the rule the reports depend on, and a
    -- rule the reports depend on should be one the file cannot be written without.
    CHECK ((kind = 2 AND category_id IS NULL) OR (kind <> 2 AND transfer_id IS NULL))
) STRICT;

-- The index every report uses: one account, a range of days, live rows only.
CREATE INDEX transactions_ledger ON transactions (account_id, occurred_on) WHERE deleted = 0;
CREATE INDEX transactions_by_category ON transactions (category_id, occurred_on) WHERE deleted = 0;
CREATE INDEX transactions_month ON transactions (occurred_on) WHERE deleted = 0;
CREATE INDEX transactions_chain ON transactions (supersedes_id) WHERE supersedes_id IS NOT NULL;
CREATE INDEX transactions_sync ON transactions (deleted, updated_at);
CREATE INDEX transactions_hlc ON transactions (hlc);

CREATE TABLE budgets (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    category_id BLOB    NOT NULL CHECK (length(category_id) = 16),
    -- The month the budget is for, as YYYYMM. A budget is per month by design; a budget with a
    -- start and an end is a second concept, and one that would make "how am I doing this month"
    -- a range query instead of a lookup.
    period      INTEGER NOT NULL CHECK (period BETWEEN 101 AND 999912),
    amount      INTEGER NOT NULL CHECK (amount >= 0 AND amount <= 1000000000000),
    currency    TEXT    NOT NULL CHECK (length(currency) = 3),
    -- 0 does not carry, 1 carries what was left over into the next month.
    rolls_over  INTEGER NOT NULL DEFAULT 0 CHECK (rolls_over IN (0, 1)),
    note        BLOB
) STRICT;

-- Partial, like every uniqueness rule in this schema. One budget per category per month while it
-- is alive; a budget deleted and set again for the same month is an ordinary thing to do.
CREATE UNIQUE INDEX budgets_period_live ON budgets (category_id, period) WHERE deleted = 0;
CREATE INDEX budgets_month ON budgets (period) WHERE deleted = 0;
CREATE INDEX budgets_sync ON budgets (deleted, updated_at);
CREATE INDEX budgets_hlc ON budgets (hlc);
