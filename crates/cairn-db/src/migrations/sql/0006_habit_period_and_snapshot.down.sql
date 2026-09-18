-- Undoes migration 0006.
--
-- Two statements, because the SQLite this build carries supports dropping a column and neither
-- of these two is indexed, named in a partial index or mentioned by any check but its own. The
-- targets each day was judged by go with the column, which is the honest consequence of
-- reverting the migration that started recording them: afterwards every day is judged by the
-- habit's current target again, as it was before.

ALTER TABLE habits DROP COLUMN period;

ALTER TABLE habit_entries DROP COLUMN target_snapshot;
