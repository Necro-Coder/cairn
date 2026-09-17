-- Migration 0001: the tables that belong to no module.
--
-- Two of them. Settings is where a preference goes that has to survive a restart and has no
-- home in the vault header, which is authenticated cryptographic material with no room for
-- anything else. Sync state is the high water mark per peer that the merge will read, and it
-- is written now rather than later because adding a column to a table with data in it is the
-- expensive kind of change and adding an empty table is the cheap kind.
--
-- The ledger is not here. `schema_migrations` is created by the runner before any migration
-- is applied, because it is what records that this one was applied; a migration that creates
-- the table it is about to be written into would have to be special-cased anyway, and a
-- special case in a migration runner is where the next bug lives.
--
-- Every data table carries the same seven columns, without exception:
--
--   id          sixteen bytes of version four UUID. Never an autoincrementing number: two
--               devices generating rows offline would produce the same one.
--   created_at  microseconds since the epoch, UTC. An integer because it sorts, indexes and
--               compares the same as text while taking a third of the room.
--   updated_at  the same, for the last write.
--   device_id   which installation wrote it, for the merge and for nothing else.
--   deleted     nothing is ever removed. A row is marked, and its encrypted columns emptied.
--   hlc         sixteen bytes of hybrid logical clock, which is what actually orders writes
--               across two machines whose wall clocks disagree.
--   rev         a per-row counter, and part of what every encrypted value in the row is
--               authenticated against, so an old ciphertext cannot be put back.
--
-- The tables are STRICT, so a column declared INTEGER refuses text. Ordinary SQLite would
-- store whatever it was handed and the mistake would surface as an ordering that is wrong in
-- one place.

CREATE TABLE settings (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    key         TEXT    NOT NULL CHECK (length(key) BETWEEN 1 AND 64),
    -- Sealed, because a preference can name a folder or a person. Nullable, because a
    -- setting that has been cleared and a setting that was never written are different facts.
    value       BLOB
) STRICT;

-- Partial, and that is the whole point. A plain unique index would mean a setting deleted
-- once could never be set again, because the tombstone would still be holding its name.
CREATE UNIQUE INDEX settings_key_live ON settings (key) WHERE deleted = 0;
CREATE INDEX settings_sync ON settings (deleted, updated_at);
CREATE INDEX settings_hlc ON settings (hlc);

CREATE TABLE sync_state (
    id              BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    device_id       BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted         INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc             BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev             INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    peer_device_id  BLOB    NOT NULL CHECK (length(peer_device_id) = 16),
    -- The highest clock value received from that peer in the last successful synchronisation.
    -- In the clear: it is a clock reading, and the merge has to compare it in SQL.
    watermark_hlc   BLOB    NOT NULL CHECK (length(watermark_hlc) = 16),
    synced_at       INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX sync_state_peer_live ON sync_state (peer_device_id) WHERE deleted = 0;
CREATE INDEX sync_state_sync ON sync_state (deleted, updated_at);
CREATE INDEX sync_state_hlc ON sync_state (hlc);
