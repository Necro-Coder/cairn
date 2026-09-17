# Data model

The schema, table by table, and why each column exists.

This page is about what is stored. For how the file is encrypted, read [storage](storage.md). For the key hierarchy, read [cryptography](cryptography.md).

## The seven columns every table has

Every table of user data carries the same seven columns, in the same order, before anything of its own.

| Column | Type | What it is |
| --- | --- | --- |
| `id` | `BLOB`, 16 bytes | A UUID v4. Never an autoincrementing integer: two devices generating rows while apart would collide on every number, and a merge that has to renumber rows is a merge that breaks every reference to them |
| `created_at` | `INTEGER` | Microseconds since the Unix epoch, UTC |
| `updated_at` | `INTEGER` | The same, for the last write. Indexed with `deleted`, because that pair is what a synchronisation asks for |
| `device_id` | `BLOB`, 16 bytes | Which installation last wrote the row |
| `deleted` | `INTEGER`, 0 or 1 | Whether it is a tombstone |
| `hlc` | `BLOB`, 16 bytes | The hybrid logical clock reading of the last write |
| `rev` | `INTEGER` | How many times the row has been written |

`schema_migrations` is the one exception. It is not user data, it is never synchronised, and giving it an `hlc` and a `device_id` would be an invitation to merge it. `PRAGMA user_version` mirrors its highest applied version, so the check "is this file newer than this build" costs a read and no parsing.

Every table is `STRICT`, so a column declared `INTEGER` cannot quietly hold text.

### Why the clock reading is sixteen bytes and not a number

`hlc` is `(wall_ms: u64, counter: u16, device: u48)` written big-endian, which makes the byte order of the blob the same as the logical order of the reading. SQLite can therefore index it, compare it and page by it without knowing what it is.

That is the whole reason for the layout. Paging is `WHERE hlc > ?` against that index, which costs the same for the last page as for the first. An `OFFSET` that grows makes the database count past everything it has already handed out, so a long list gets slower exactly where somebody is most likely to still be scrolling.

The counter is what makes two writes in the same millisecond orderable. When it overflows, the reading borrows the next millisecond rather than repeating itself: a reading that repeats is two rows no merge can order, and a merge that cannot order two rows picks one and loses the other.

A clock is resumed at open from the highest reading anywhere in the file, and it takes this device's identifier separately so that a reading seen from a peer never causes this device to adopt the peer's identity. Without the resume, a machine whose wall clock runs slow would hand out, after a restart, a reading a row already carries.

### Why nothing is ever physically deleted

A deletion marks the row, raises `rev`, writes a new `hlc` and **empties every encrypted column it has**. The skeleton stays so that two devices merging later can tell "this was deleted" apart from "this never arrived". The content does not stay, because a deletion that keeps the content for a hundred and eighty days is a delay and not a deletion.

The consequence is a file that only grows, which is why the count of tombstones is on the diagnostics screen rather than left for somebody to discover, and why there is a compaction that removes tombstones older than a hundred and eighty days. That compaction is the only `DELETE` in the application, and what it removes is a skeleton with nothing in it.

### Why every uniqueness rule is a partial index

Every `UNIQUE` in this schema is `WHERE deleted = 0`. A plain one would mean that a tombstone keeps holding the name, the day or the pair for ever.

The clearest case is `habit_entries`: a plain `UNIQUE(habit_id, day)` would mean a day somebody unmarked could never be marked again, and unmarking a day and marking it again is the single most common thing anybody does with a habit tracker. There is a test that does exactly that, and it fails against the ordinary index.

### Why there are no foreign keys

There is not one foreign key in the schema, and `PRAGMA foreign_keys` is on anyway so that adding one later behaves as written.

The reason is the merge. A synchronisation can deliver a child before its parent — an entry before its folder, a habit before its area, a movement before its account — and a foreign key would turn an ordinary out-of-order arrival into a failure. What a row points at is checked when it is read, where the answer can be "no folder" instead of an error.

## Migration 0001 — the tables that belong to no module

**`settings`** is preferences, and the first table with a sealed column. The `key` is in the clear so it can be looked up; the `value` is sealed, because a setting is as often a piece of content as it is a flag.

**`sync_state`** is one row per peer holding the watermark of what has been exchanged with it. Its `watermark` column is deliberately **not** sealed: the merge has to compare it in SQL, and a watermark nobody can compare is a synchronisation that starts from the beginning every time. There is a test whose only job is to fail if somebody seals it later.

## Migration 0002 — habits

Four tables: `habit_areas` group habits, `habits` describe what is tracked, `habit_entries` are the marks on the calendar, and `habit_pauses` are the stretches where a missed day is not a broken streak.

Only the notes are sealed — `habits.notes`, `habit_entries.note`, `habit_pauses.reason`. The name, the colour, the icon, the schedule and the position stay in the clear, and the reasoning, along with what it costs, is [ADR 0009](decisions/0009-what-the-database-encrypts.md).

Columns worth explaining:

- `kind` is 0 for a habit that is done or not done and 1 for one that counts a quantity.
- `schedule_mask` is seven bits, one per weekday, Monday first. Zero means no fixed schedule.
- `aggregation` says how the days of a period combine: sum, highest, or last.
- `direction` is 0 for "more is better" and 1 for "less is better". A habit somebody is cutting down is not a habit with a negative target; it is the same target read the other way round.
- `habit_entries.amount` is in the smallest unit the habit counts in, never a fraction. Eight glasses of water is eight; two and a half kilometres is 2500 metres.
- `started_on` and `day` are civil days, `YYYYMMDD`, not instants. [ADR 0008](decisions/0008-civil-days-and-instants.md) says why that distinction is a type and not a comment.

## Migration 0003 — the vault

Seven tables: `vault_folders`, `vault_entries`, `vault_urls`, `vault_fields`, `vault_password_history`, `vault_tags` and `vault_entry_tags`.

Here the rule is the opposite of the one habits took. Almost everything a person reads is content: the title of an entry says which bank somebody uses, a URL says which service, the label of a custom field says what kind of secret is beside it. So titles, user names, passwords, notes, URLs, field labels, field values, folder names and tag names are all sealed, and what stays readable is structure.

Two columns are in the clear on purpose and are worth naming:

- `vault_folders.parent_id`, because the depth of the tree is checked by following it, and a parent nobody can read is a tree nobody can walk. The check is a pure function over identifiers in `cairn-domain`, which is what lets it be proved against generated shapes — including the loop that two devices produce when each reparents a folder under the other's while they are apart.
- `vault_fields.secret`, because it decides whether the interface hides the value behind a reveal, and a flag that has to be decrypted to know how to draw a row is a flag decrypted on every row.

`vault_entries.title`, `.username`, `.password` and `.notes` are all nullable, and the null is the point: a tombstone keeps none of them. A skeleton that still says which bank it was is not a deleted entry. A live row always has a title, and the repository refuses one without; a live row missing one means the file was written by something that is not this program, and it is reported the same way as a value that does not decrypt.

The password history is capped at ten per entry. Trimming marks the oldest as deleted and empties its ciphertext, so the row survives for the merge to see and the password does not survive at all.

Because nothing in this module can be compared in SQL, searching is not a query. The titles are decrypted once when the vault unlocks, held in a list, searched by walking it, and the list is emptied when the vault locks. There is no plaintext index in the file and there will not be one: an index over titles is a copy of every title.

## Migration 0004 — finances

Four tables: `accounts`, `categories`, `transactions` and `budgets`.

Money is an integer in the smallest unit of its currency, with an ISO 4217 code beside it. Never a floating point number: a column of them adds up to a different total depending on the order it was added in, and a total that changes when nothing changed is not an accounting application.

Three rules are particular to this module.

**A movement is never edited.** An edit writes a new row that points back with `supersedes_id`, and the old row is marked deleted and gains `superseded_by_id`. A merge that overwrote money would lose a movement, and a lost movement is not noticed until somebody balances the month — by which point there is nothing left to compare against.

**A transfer is two rows and no category.** Both are `kind = 2`, each points at the other through `transfer_id`, and neither belongs to a category. That keeps a balance per account as the plain sum of that account's own rows, with no special case, and it keeps a transfer out of both sides of a report: moving money between two accounts somebody owns is neither spending it nor earning it. A `CHECK` in the table enforces the pairing, because it is the rule the reports depend on and a rule the reports depend on should be one the file cannot be written without.

**Categories are two levels deep and no more.** Checked by asking whether the intended parent has a parent of its own. Deeper trees of spending categories are a way of never finishing the categorising, and a report over three levels is a report nobody reads.

Other columns:

- `accounts.opening_balance` is what the account held before the first movement this application knows about. Without it a balance is the sum of the movements since installation, which is not the balance.
- `accounts.excluded` marks an account that is tracked but does not count towards the net worth.
- `transactions.kind` keeps expense, income and transfer apart at the column rather than inferring them from the sign, because a refund is a negative expense and not an income, and only the person entering it knows which.
- `transactions.cleared` says whether the movement has been seen on a statement. It is structural because filtering by it is the whole of reconciling a month.
- `budgets.period` is `YYYYMM`. A budget is per month by design; one with a start and an end is a second concept, and one that would make "how am I doing this month" a range query instead of a lookup.
