-- Undoes migration 0001.
--
-- The indexes go with their tables, so they are not named: dropping a table drops the indexes
-- on it. Naming them anyway would be two places to keep in step for no gain.

DROP TABLE sync_state;
DROP TABLE settings;
