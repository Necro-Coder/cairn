-- Migration 0007: what kind of thing an entry is, and since when it has been in the bin.
--
-- `kind` is 0 for an account and 1 for a secure note. It is one column on one table rather than
-- a second table, because search, folders, the bin, the history and the merge are identical for
-- the two of them: a table of its own would duplicate five features to save nothing, and every
-- query that lists "everything" would become two queries and a union. What differs between the
-- two is which boxes a form draws, and that is a decision for the screen.
--
-- Nor is it deduced from the password being null. An account somebody has not typed a password
-- into yet is not a note, and that is the state every half filled form is in; a rule that reads
-- the absence of a value as a change of kind would turn "I will finish this later" into "this is
-- a different thing now", silently, on a row nobody looked at again.
--
-- `trashed_at` is the moment something was thrown away, in microseconds, and it is a state apart
-- from `deleted`. A row in the bin keeps every byte of its ciphertext and shows up in no list
-- but the bin's; a deleted row has `deleted = 1` and every sealed column emptied to null.
-- Deletion in this schema is irreversible by design — that is what makes a deleted password
-- actually gone — so without this column there would be nothing to restore from, and a bin that
-- cannot restore is not a bin.
--
-- The index is partial and covers what is alive and thrown away at once, which is exactly what
-- the bin screen reads and the only thing the thirty day sweep walks. A full index over
-- `trashed_at` would be an index over a column that is null in a hundred per cent of the rows a
-- hundred per cent of the time.

ALTER TABLE vault_entries ADD COLUMN kind INTEGER NOT NULL DEFAULT 0 CHECK (kind IN (0, 1));

ALTER TABLE vault_entries ADD COLUMN trashed_at INTEGER
    CHECK (trashed_at IS NULL OR trashed_at > 0);

CREATE INDEX vault_entries_trash ON vault_entries (trashed_at)
    WHERE deleted = 0 AND trashed_at IS NOT NULL;
