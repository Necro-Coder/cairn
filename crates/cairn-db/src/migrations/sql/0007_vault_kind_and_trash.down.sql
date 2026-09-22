-- Undoes migration 0007.
--
-- The index goes first, and the order is not a preference: SQLite refuses to drop a column that
-- an index names, partial or not, and `vault_entries_trash` names `trashed_at`. Dropping the
-- column first fails with a message about the index, halfway through a reverse script.
--
-- Everything in the bin comes back into the lists when this runs, which is the honest
-- consequence of reverting the migration that introduced the bin: before it, there was nowhere
-- for a thrown away entry to be, and no row lost a single byte on the way.

DROP INDEX vault_entries_trash;

ALTER TABLE vault_entries DROP COLUMN trashed_at;

ALTER TABLE vault_entries DROP COLUMN kind;
