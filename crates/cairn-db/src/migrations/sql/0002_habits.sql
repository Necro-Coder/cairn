-- Migration 0002: habits, the days they were done on, and the pauses in between.
--
-- Four tables. Areas group habits, habits describe what is being tracked, entries are the marks
-- on the calendar, and pauses are the stretches where a missed day is not a broken streak.
--
-- What is sealed and what is not follows the list in the design, literally and without
-- exceptions invented here. Notes and reasons are content and are sealed. Names, colours,
-- icons, schedules and positions are structural: without them in the clear there is no ordering
-- in SQL, no paging by keyset and no heatmap that meets its budget, because every one of those
-- would become a full decrypt of the table in Rust.
--
-- The consequence is written down rather than hidden: the name of a habit can say as much as
-- its note, and under this rule it is protected by the file's encryption alone. That is the
-- decision the phase took with its eyes open, and it is recorded in an ADR rather than in a
-- comment nobody reads.
--
-- A day is an integer in the shape YYYYMMDD, not a moment and not text. It is a square on a
-- calendar, which only means anything in the place the person was standing, and keeping it in
-- the same type as an instant is what makes a habit marked late at night land on tomorrow.

CREATE TABLE habit_areas (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    name        TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    color       TEXT    CHECK (color IS NULL OR length(color) BETWEEN 1 AND 32),
    position    INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX habit_areas_sync ON habit_areas (deleted, updated_at);
CREATE INDEX habit_areas_hlc ON habit_areas (hlc);

CREATE TABLE habits (
    id                BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    device_id         BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted           INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc               BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev               INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    name              TEXT    NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    notes             BLOB,
    icon              TEXT    CHECK (icon IS NULL OR length(icon) BETWEEN 1 AND 64),
    color             TEXT    CHECK (color IS NULL OR length(color) BETWEEN 1 AND 32),
    -- No foreign key. The area may arrive from another device after the habit does, and a
    -- constraint that refuses the row would turn an ordinary out of order merge into a failure.
    -- What it points at is checked when it is read, where the answer can be "no area" instead.
    area_id           BLOB    CHECK (area_id IS NULL OR length(area_id) = 16),
    -- 0 does or does not, 1 counts a quantity. An integer rather than text so that a value
    -- outside the two is refused by the check rather than by whoever reads it next.
    kind              INTEGER NOT NULL DEFAULT 0 CHECK (kind IN (0, 1)),
    -- Seven bits, one per day of the week, Monday first. Zero means no fixed schedule.
    schedule_mask     INTEGER NOT NULL DEFAULT 0 CHECK (schedule_mask BETWEEN 0 AND 127),
    target_per_period INTEGER CHECK (target_per_period IS NULL OR target_per_period > 0),
    unit              TEXT    CHECK (unit IS NULL OR length(unit) BETWEEN 1 AND 32),
    -- How the days of a period combine: 0 sum, 1 the highest, 2 the last.
    aggregation       INTEGER NOT NULL DEFAULT 0 CHECK (aggregation IN (0, 1, 2)),
    -- 0 more is better, 1 less is better. A habit somebody is cutting down is not a habit with
    -- a negative target; it is the same target read the other way round.
    direction         INTEGER NOT NULL DEFAULT 0 CHECK (direction IN (0, 1)),
    started_on        INTEGER NOT NULL CHECK (started_on BETWEEN 10101 AND 99991231),
    archived_at       INTEGER,
    position          INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX habits_sync ON habits (deleted, updated_at);
CREATE INDEX habits_hlc ON habits (hlc);
CREATE INDEX habits_area ON habits (area_id, position) WHERE deleted = 0;

CREATE TABLE habit_entries (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    habit_id    BLOB    NOT NULL CHECK (length(habit_id) = 16),
    day         INTEGER NOT NULL CHECK (day BETWEEN 10101 AND 99991231),
    -- In the smallest unit the habit counts in, never a fraction. Eight glasses of water is
    -- eight; two and a half kilometres is 2500 metres. Floating point is not a quantity anybody
    -- can add up twice and get the same answer for.
    amount      INTEGER NOT NULL DEFAULT 1,
    note        BLOB
) STRICT;

-- Partial, and this one is the reason the rule exists at all. A plain UNIQUE(habit_id, day)
-- would mean a day unmarked once could never be marked again, because the tombstone would still
-- be holding the pair. That is the single most common thing a person does with a habit tracker.
CREATE UNIQUE INDEX habit_entries_day_live ON habit_entries (habit_id, day) WHERE deleted = 0;
CREATE INDEX habit_entries_sync ON habit_entries (deleted, updated_at);
CREATE INDEX habit_entries_hlc ON habit_entries (hlc);

CREATE TABLE habit_pauses (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   BLOB    NOT NULL CHECK (length(device_id) = 16),
    deleted     INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    hlc         BLOB    NOT NULL CHECK (length(hlc) = 16),
    rev         INTEGER NOT NULL DEFAULT 0 CHECK (rev >= 0),

    habit_id    BLOB    NOT NULL CHECK (length(habit_id) = 16),
    starts_on   INTEGER NOT NULL CHECK (starts_on BETWEEN 10101 AND 99991231),
    -- Null means the pause is still open. A pause that ended before it started is refused here
    -- rather than discovered by a streak calculation that quietly returns zero.
    ends_on     INTEGER CHECK (ends_on IS NULL OR ends_on BETWEEN 10101 AND 99991231),
    reason      BLOB,

    CHECK (ends_on IS NULL OR ends_on >= starts_on)
) STRICT;

CREATE INDEX habit_pauses_habit ON habit_pauses (habit_id, starts_on) WHERE deleted = 0;
CREATE INDEX habit_pauses_sync ON habit_pauses (deleted, updated_at);
CREATE INDEX habit_pauses_hlc ON habit_pauses (hlc);
