-- Migration 0008: let the addresses, the custom fields and the folders be emptied when they go.
--
-- Migration 0003 declared `vault_entries.title` and the three columns beside it nullable, with a
-- comment saying why: a tombstone keeps none of them, because a skeleton that still says which
-- bank it was is not a deleted entry. It declared `vault_password_history.password` nullable for
-- the same reason. And then it declared `vault_urls.value`, `vault_fields.label`,
-- `vault_fields.value` and `vault_folders.name` NOT NULL, which is the same decision reached the
-- other way round on four columns that say exactly as much: an address names the service, "PIN de
-- la tarjeta" beside an opaque blob is most of the answer already, and a folder called "Cuentas
-- de Hacienda" says as much as anything that was ever filed in it.
--
-- Nothing had noticed, because no repository wrote to the first two tables and nothing deleted a
-- folder. The code that does cannot mark a row deleted and empty it, so either the ciphertext of
-- every address, label and folder name somebody ever removed stays in the file for ever, or a
-- deletion in this schema stops meaning what it means everywhere else in it. Neither is
-- acceptable, so the three tables are rebuilt with the column constraint the rest of the schema
-- already uses.
--
-- SQLite has no way to drop a NOT NULL, so this is a rebuild: a new table, a copy, a drop, a
-- rename, and every index put back. The indexes are the part worth reading twice — a rebuild that
-- restores five of six is a change nothing fails on and that quietly stops a query meeting its
-- budget a year later — so they are recreated here exactly as 0003 declared them, and a test
-- compares the list before and after the round trip.
--
-- The tables are empty on every database that exists today, which is why this is a rebuild
-- nobody pays for. The copy is written anyway, because a migration that is correct only while a
-- table happens to be empty is a migration waiting for the day it is not.

CREATE TABLE vault_urls_rebuilt (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    entry_id    BLOB    NOT NULL CHECK (length(entry_id) = 16),
    -- Nullable now, and the null is the point: a removed address keeps its row so a merge can
    -- see that it went, and loses its ciphertext so the service somebody stopped using is not
    -- still named in the file. A live row always has one; the repository refuses one without.
    value       BLOB,
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

INSERT INTO vault_urls_rebuilt
    (id, created_at, updated_at, device_id, deleted, hlc, rev, entry_id, value, position)
SELECT id, created_at, updated_at, device_id, deleted, hlc, rev, entry_id, value, position
  FROM vault_urls;

DROP TABLE vault_urls;

ALTER TABLE vault_urls_rebuilt RENAME TO vault_urls;

CREATE INDEX vault_urls_entry ON vault_urls (entry_id, position) WHERE deleted = 0;
CREATE INDEX vault_urls_sync ON vault_urls (deleted, updated_at);
CREATE INDEX vault_urls_hlc ON vault_urls (hlc);

CREATE TABLE vault_fields_rebuilt (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    entry_id    BLOB    NOT NULL CHECK (length(entry_id) = 16),
    -- Both nullable, and both emptied together. Emptying the value and keeping the label would
    -- leave a row that says somebody kept a card PIN here and declines to say which card.
    label       BLOB,
    value       BLOB,
    -- Still in the clear, unchanged from 0003: it decides whether the interface hides the value
    -- behind a reveal, and a flag that has to be decrypted to know how to draw a row is a
    -- decryption on every paint.
    secret      INTEGER NOT NULL DEFAULT 0 CHECK (secret IN (0, 1)),
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

INSERT INTO vault_fields_rebuilt
    (id, created_at, updated_at, device_id, deleted, hlc, rev,
     entry_id, label, value, secret, position)
SELECT id, created_at, updated_at, device_id, deleted, hlc, rev,
       entry_id, label, value, secret, position
  FROM vault_fields;

DROP TABLE vault_fields;

ALTER TABLE vault_fields_rebuilt RENAME TO vault_fields;

CREATE INDEX vault_fields_entry ON vault_fields (entry_id, position) WHERE deleted = 0;
CREATE INDEX vault_fields_sync ON vault_fields (deleted, updated_at);
CREATE INDEX vault_fields_hlc ON vault_fields (hlc);

CREATE TABLE vault_folders_rebuilt (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    -- Nullable now. Deleting a folder is filing rather than destroying — the entries inside it go
    -- to the root and every one of them survives — but the folder's own row is a tombstone like
    -- any other, and a tombstone that still says "Cuentas de Hacienda" is a deletion that deleted
    -- the list and kept the label on it.
    name        BLOB,
    -- Unchanged from 0003, and still written null by everything: the column stays because
    -- removing it would be a migration that buys nothing, and this module's folders are flat.
    parent_id   BLOB    CHECK (parent_id IS NULL OR length(parent_id) = 16),
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

INSERT INTO vault_folders_rebuilt
    (id, created_at, updated_at, device_id, deleted, hlc, rev, name, parent_id, position)
SELECT id, created_at, updated_at, device_id, deleted, hlc, rev, name, parent_id, position
  FROM vault_folders;

DROP TABLE vault_folders;

ALTER TABLE vault_folders_rebuilt RENAME TO vault_folders;

CREATE INDEX vault_folders_parent ON vault_folders (parent_id, position) WHERE deleted = 0;
CREATE INDEX vault_folders_sync ON vault_folders (deleted, updated_at);
CREATE INDEX vault_folders_hlc ON vault_folders (hlc);
