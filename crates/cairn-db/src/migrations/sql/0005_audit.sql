-- Migration 0005: the record of what was done with the data as a whole.
--
-- One table, and it exists because four operations in this application move everything somebody
-- owns at once: writing a backup, checking one, restoring one, and exporting a module in the
-- clear. Those are the four that a person needs to be able to look back at and ask "did I do
-- that, and when". A vault that cannot answer that question is a vault where an export nobody
-- remembers making looks exactly like an export that did not happen.
--
-- It is a table rather than a log file beside the database, and that is the whole decision. A
-- text file next to the database would be the only place in this project where something is
-- written without being encrypted, and what would go in it is the name of a module and the name
-- of a file. Inside the database it already has both layers.
--
-- `kind` is in the clear. It is a closed enumeration of four strings that this code writes and
-- nothing else does — `backup.exported`, `backup.verified`, `backup.imported`,
-- `plaintext.exported` — so it says nothing about the person that the existence of the row does
-- not already say, and keeping it readable is what lets the index below do its job.
--
-- `detail` is sealed, because that is where a file name or a module name ends up, and a file
-- name is very often a folder name, and a folder name is very often somebody's own name.
--
-- It carries the same seven columns as every other data table, so it synchronises with the rest
-- in the phase that brings synchronisation, and so a row is marked rather than removed. An audit
-- record that can be deleted is an audit record that will be, at exactly the moment it matters.

CREATE TABLE audit_events (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    -- What happened. In the clear, and bounded, because it is one of a handful of constants in
    -- this repository and never anything a person typed.
    kind        TEXT    NOT NULL CHECK (length(kind) BETWEEN 1 AND 64),
    -- When it happened, in microseconds since the epoch, UTC. Separate from `created_at`,
    -- which is when the row was written: they are the same number today and they stop being
    -- the same number the first time a record arrives from another device.
    occurred_at INTEGER NOT NULL,
    -- Sealed. A file name, a module name, a count. Nullable, because some events have nothing
    -- to add beyond having happened.
    detail      BLOB
) STRICT;

-- What the history screen reads: the events of one kind, newest first.
CREATE INDEX audit_events_kind ON audit_events (kind, occurred_at);

-- The two every table in this schema carries, for the merge and for finding a row by its clock.
CREATE INDEX audit_events_sync ON audit_events (deleted, updated_at);
CREATE INDEX audit_events_hlc ON audit_events (hlc);
