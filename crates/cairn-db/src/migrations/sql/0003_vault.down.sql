-- Undoes migration 0003.
--
-- Children before parents, so the order reads the way a foreign key would have forced it. There
-- are none, deliberately, but the reverse of a migration is not the place to start relying on
-- an absence.

DROP TABLE vault_entry_tags;
DROP TABLE vault_tags;
DROP TABLE vault_password_history;
DROP TABLE vault_fields;
DROP TABLE vault_urls;
DROP TABLE vault_entries;
DROP TABLE vault_folders;
