-- Undoes migration 0004.
--
-- Children before parents, so that the order reads the same as the one a foreign key would have
-- forced if these tables had them. They do not, on purpose, but the reverse of a migration is
-- not the place to start relying on that.

DROP TABLE budgets;
DROP TABLE transactions;
DROP TABLE categories;
DROP TABLE accounts;
