-- Undoes migration 0002.
--
-- Children before parents, so that the order reads the same as the one a foreign key would have
-- forced if these tables had them. They do not, on purpose, but the reverse of a migration is
-- not the place to start relying on that.

DROP TABLE habit_pauses;
DROP TABLE habit_entries;
DROP TABLE habits;
DROP TABLE habit_areas;
