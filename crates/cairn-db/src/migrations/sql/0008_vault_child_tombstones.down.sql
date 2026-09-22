-- Undoes migration 0008.
--
-- Another rebuild, back to the NOT NULL the tables were created with. The one thing this cannot
-- do is put back a ciphertext that was emptied on purpose, so the emptied columns come back as a
-- blob of zero length: it satisfies the constraint, it is not a value, and it decrypts to nothing
-- if anything ever asks. The alternative would be to drop those rows, and dropping a tombstone is
-- how a deletion undoes itself the next time two devices meet.

CREATE TABLE vault_urls_restored (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    entry_id    BLOB    NOT NULL CHECK (length(entry_id) = 16),
    value       BLOB    NOT NULL,
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

INSERT INTO vault_urls_restored
    (id, created_at, updated_at, device_id, deleted, hlc, rev, entry_id, value, position)
SELECT id, created_at, updated_at, device_id, deleted, hlc, rev, entry_id,
       coalesce(value, x''), position
  FROM vault_urls;

DROP TABLE vault_urls;

ALTER TABLE vault_urls_restored RENAME TO vault_urls;

CREATE INDEX vault_urls_entry ON vault_urls (entry_id, position) WHERE deleted = 0;
CREATE INDEX vault_urls_sync ON vault_urls (deleted, updated_at);
CREATE INDEX vault_urls_hlc ON vault_urls (hlc);

CREATE TABLE vault_fields_restored (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    entry_id    BLOB    NOT NULL CHECK (length(entry_id) = 16),
    label       BLOB    NOT NULL,
    value       BLOB    NOT NULL,
    secret      INTEGER NOT NULL DEFAULT 0 CHECK (secret IN (0, 1)),
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

INSERT INTO vault_fields_restored
    (id, created_at, updated_at, device_id, deleted, hlc, rev,
     entry_id, label, value, secret, position)
SELECT id, created_at, updated_at, device_id, deleted, hlc, rev, entry_id,
       coalesce(label, x''), coalesce(value, x''), secret, position
  FROM vault_fields;

DROP TABLE vault_fields;

ALTER TABLE vault_fields_restored RENAME TO vault_fields;

CREATE INDEX vault_fields_entry ON vault_fields (entry_id, position) WHERE deleted = 0;
CREATE INDEX vault_fields_sync ON vault_fields (deleted, updated_at);
CREATE INDEX vault_fields_hlc ON vault_fields (hlc);

CREATE TABLE vault_folders_restored (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    name        BLOB    NOT NULL,
    parent_id   BLOB    CHECK (parent_id IS NULL OR length(parent_id) = 16),
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

INSERT INTO vault_folders_restored
    (id, created_at, updated_at, device_id, deleted, hlc, rev, name, parent_id, position)
SELECT id, created_at, updated_at, device_id, deleted, hlc, rev,
       coalesce(name, x''), parent_id, position
  FROM vault_folders;

DROP TABLE vault_folders;

ALTER TABLE vault_folders_restored RENAME TO vault_folders;

CREATE INDEX vault_folders_parent ON vault_folders (parent_id, position) WHERE deleted = 0;
CREATE INDEX vault_folders_sync ON vault_folders (deleted, updated_at);
CREATE INDEX vault_folders_hlc ON vault_folders (hlc);
