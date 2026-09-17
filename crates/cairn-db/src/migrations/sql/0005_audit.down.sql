-- Undoes migration 0005.
--
-- The indexes go with the table, so there is one statement. Dropping the table loses the history
-- it held, which is the honest consequence of reverting a migration that created it.

DROP TABLE audit_events;
