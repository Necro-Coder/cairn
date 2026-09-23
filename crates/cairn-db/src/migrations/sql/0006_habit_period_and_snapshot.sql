-- Migration 0006: the period a habit is judged over, and the target an entry was judged by.
--
-- Migration 0002 gave a habit an `aggregation` saying how the days of a period combine, and a
-- `target_per_period` naming one, and then left no column anywhere saying what the period is.
-- Everything that has read those two since has had to assume a day. That assumption is right
-- for most habits and quietly wrong for the ones somebody counts by the week, where three runs
-- out of a target of three is a week finished and not three days out of seven missed. The
-- column says it instead, so that the two that were already there have something to refer to.
--
-- Two values, 0 daily and 1 weekly, and the check is the whole point of the column being an
-- integer: it makes a monthly habit impossible rather than something whoever reads the row has
-- to notice and refuse. A month is not a period this application judges a streak over. "The
-- month before this one" has twelve lengths, two of which depend on the year, and every one of
-- those is somewhere a streak goes wrong without anybody being told.
--
-- `target_snapshot` is the target a day was judged by, written next to the mark rather than
-- looked up on the habit when the calendar is drawn. Without it, raising a target from five to
-- ten repaints as failed every day that was a success under five, and a year somebody actually
-- lived changes colour because of a decision taken this morning. A record that a later edit can
-- rewrite is not a record.
--
-- It is null on the habits that are done or not done, which have no target to remember, and
-- null on every row written before this column existed. Both are judged by the habit's current
-- target, which is exactly what was happening to them the day before this migration ran.

ALTER TABLE habits ADD COLUMN period INTEGER NOT NULL DEFAULT 0 CHECK (period IN (0, 1));

ALTER TABLE habit_entries ADD COLUMN target_snapshot INTEGER
    CHECK (target_snapshot IS NULL OR target_snapshot > 0);
