-- Migration 0003: the password vault.
--
-- Seven tables, and the rule about what is sealed is the opposite of the one for habits. Here
-- almost everything is content: the title of an entry says which bank somebody uses, a URL says
-- which service, the label of a custom field says what kind of secret is beside it. So the
-- title, the user name, the password, the notes, every URL, every custom label and value, every
-- folder name and every tag name are sealed, and what is left in the clear is structure:
-- identifiers, positions, flags and moments.
--
-- That has a cost, and it is paid deliberately. Nothing in this module can be searched or sorted
-- in SQL by anything a person reads. The titles are decrypted into memory when the vault opens
-- and searched there — a few thousand entries is a few megabytes — and the memory is emptied
-- when the vault closes. There is no plain text index, and there will not be one: a search index
-- over titles is a copy of every title, and a copy that is not encrypted is the thing this whole
-- module exists to prevent.
--
-- The password history is capped per entry, and the cap is enforced by the repository rather
-- than by a constraint here. Trimming means marking the oldest as deleted and emptying its
-- ciphertext, which is a write and not a rule, and a trigger that did it would be logic living
-- where nobody looks for it.

CREATE TABLE vault_folders (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    -- Sealed. A folder called "Cuentas de Hacienda" says as much as anything inside it.
    name        BLOB    NOT NULL,
    -- In the clear, and it has to be: the depth of the tree is checked by following this column,
    -- and a parent nobody can read is a tree nobody can walk. No foreign key, because a merge
    -- can deliver a child before its parent and a constraint would turn that into a failure.
    parent_id   BLOB    CHECK (parent_id IS NULL OR length(parent_id) = 16),
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX vault_folders_parent ON vault_folders (parent_id, position) WHERE deleted = 0;
CREATE INDEX vault_folders_sync ON vault_folders (deleted, updated_at);
CREATE INDEX vault_folders_hlc ON vault_folders (hlc);

CREATE TABLE vault_entries (
    id            BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    device_id     BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted       INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc           BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev           INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    -- All four nullable, because a tombstone keeps none of them. Deleting empties every
    -- encrypted column of the row, the title included: a skeleton that still says which bank it
    -- was is not a deleted entry, and the person who deleted it has no way to find that out.
    -- A live row always has a title; the repository refuses one without.
    title         BLOB,
    username      BLOB,
    password      BLOB,
    notes         BLOB,
    folder_id     BLOB    CHECK (folder_id IS NULL OR length(folder_id) = 16),
    favorite      INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    -- When it was last copied or opened. A moment, not a day: it is a point on a line that every
    -- machine agrees about, and it exists so a list can be ordered by what somebody actually uses.
    last_used_at  INTEGER
) STRICT;

CREATE INDEX vault_entries_folder ON vault_entries (folder_id) WHERE deleted = 0;
CREATE INDEX vault_entries_recent ON vault_entries (last_used_at) WHERE deleted = 0;
CREATE INDEX vault_entries_sync ON vault_entries (deleted, updated_at);
CREATE INDEX vault_entries_hlc ON vault_entries (hlc);

CREATE TABLE vault_urls (
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

CREATE INDEX vault_urls_entry ON vault_urls (entry_id, position) WHERE deleted = 0;
CREATE INDEX vault_urls_sync ON vault_urls (deleted, updated_at);
CREATE INDEX vault_urls_hlc ON vault_urls (hlc);

CREATE TABLE vault_fields (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    entry_id    BLOB    NOT NULL CHECK (length(entry_id) = 16),
    -- Both sealed. The label of a custom field is as telling as its value: "PIN de la tarjeta"
    -- beside an opaque blob is most of the answer already.
    label       BLOB    NOT NULL,
    value       BLOB    NOT NULL,
    -- In the clear, because it decides whether the interface hides the value behind a reveal,
    -- and a flag that has to be decrypted to know how to draw a row is a flag read on every row.
    secret      INTEGER NOT NULL DEFAULT 0 CHECK (secret IN (0, 1)),
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX vault_fields_entry ON vault_fields (entry_id, position) WHERE deleted = 0;
CREATE INDEX vault_fields_sync ON vault_fields (deleted, updated_at);
CREATE INDEX vault_fields_hlc ON vault_fields (hlc);

CREATE TABLE vault_password_history (
    id           BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    device_id    BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted      INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc          BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev          INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    entry_id     BLOB    NOT NULL CHECK (length(entry_id) = 16),
    -- Nullable, and the null is the point: a trimmed entry keeps its row so a merge can see it
    -- was trimmed, and loses its ciphertext so an old password is not still in the file.
    password     BLOB,
    replaced_at  INTEGER NOT NULL
) STRICT;

CREATE INDEX vault_password_history_entry
    ON vault_password_history (entry_id, replaced_at) WHERE deleted = 0;
CREATE INDEX vault_password_history_sync ON vault_password_history (deleted, updated_at);
CREATE INDEX vault_password_history_hlc ON vault_password_history (hlc);

CREATE TABLE vault_tags (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    name        BLOB    NOT NULL,
    color       TEXT    CHECK (color IS NULL OR length(color) BETWEEN 1 AND 32)
) STRICT;

CREATE INDEX vault_tags_sync ON vault_tags (deleted, updated_at);
CREATE INDEX vault_tags_hlc ON vault_tags (hlc);

CREATE TABLE vault_entry_tags (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    entry_id    BLOB    NOT NULL CHECK (length(entry_id) = 16),
    tag_id      BLOB    NOT NULL CHECK (length(tag_id) = 16)
) STRICT;

-- Partial, like every other uniqueness rule in this schema. A tag taken off an entry and put
-- back on it is an ordinary thing to do, and a plain unique index would refuse the second half.
CREATE UNIQUE INDEX vault_entry_tags_pair_live
    ON vault_entry_tags (entry_id, tag_id) WHERE deleted = 0;
CREATE INDEX vault_entry_tags_tag ON vault_entry_tags (tag_id) WHERE deleted = 0;
CREATE INDEX vault_entry_tags_sync ON vault_entry_tags (deleted, updated_at);
CREATE INDEX vault_entry_tags_hlc ON vault_entry_tags (hlc);
