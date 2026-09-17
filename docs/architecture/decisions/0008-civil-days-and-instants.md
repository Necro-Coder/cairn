# 0008 — Civil days and instants are different types

Date: 2026-09-17 · Status: accepted

## Context

Cairn records two kinds of date, and until this decision they were the same integer.

The first is a moment: when a row was written, when a password was replaced, when the window last saw activity. It is a point on a line that every machine agrees about, it is compared across devices during a merge, and it is only meaningful in UTC.

The second is a square on a calendar: the day a habit was completed, the day a purchase happened, the month a budget covers. It only means anything in the place the person was standing. A habit marked at half past eleven at night in Madrid belongs to that day, and it must still belong to that day when the same file is opened from a laptop whose clock is set to another zone.

Keeping both as "a number of microseconds since the epoch" works until the two are mixed, and mixing them is silent. Converting an instant to a day needs a zone, and the zone that is available at the point of conversion is the zone of whichever machine happens to be reading. So the streak of a habit changes depending on where the file is opened, and there is no error anywhere to notice.

The same confusion runs the other way. A day converted to an instant becomes midnight in some zone, and that instant then takes part in a comparison it has no business in: it is not a moment anything happened, it is a label.

## Decision

Two types in `cairn-domain`, and the compiler keeps them apart.

`Timestamp` is a count of microseconds since the Unix epoch, always UTC. It is what `created_at`, `updated_at`, `last_used_at`, `replaced_at` and `synced_at` hold. Arithmetic on it is saturating, so a clock somebody set to the year 1601 produces a bounded wrong answer rather than a wrapped one.

`CivilDay` is a year, a month and a day, with the year between 1 and 9999. It is stored as an integer in the shape `YYYYMMDD` — `20260917` for this date — and read back with a constructor that refuses the thirty first of February. Its derived ordering is calendar order, because the digits are in descending significance, so SQL can sort and range-scan the column without knowing what it means. A budget's month is the same idea one digit shorter: `YYYYMM`.

There is no conversion between the two anywhere in the workspace. A day is chosen by the person, or derived from a moment at the edge where a timezone is a known input, and never inside the domain or the database.

## Alternatives considered (and why not)

**One integer for both, with a comment.** This is what the code had. The comment does not survive the third caller, and the failure is invisible: a streak that is wrong by one day looks like a person who missed a day.

**A calendar crate (`jiff`, `time`, `chrono`).** A dependency, its transitive closure, and a parser this application does not need. The only operations Cairn performs on a civil day are: construct it, compare it, ask for the next or previous one, and store it. Those are forty lines and a leap year rule that has not changed since 1582. The crate becomes worth its cost when there is formatting and parsing of dates entered as text, which is a user interface problem for a later phase; until then it would be a dependency taken on to subtract two integers.

**Storing a day as text, `"2026-09-17"`.** Sorts correctly and reads well in a database viewer, and costs a parse on every comparison plus three bytes a row of nothing. Integers compare in one instruction.

**Storing a day as a count of days since the epoch.** Compact, sorts correctly, and unreadable. Every inspection of the file, every bug report and every test fixture would need a conversion. `20260917` is readable by a human looking at a hex dump, which matters for a file format somebody may one day have to recover by hand.

## Consequences (good and bad)

Good: a function that takes a `CivilDay` cannot be handed an instant, and a merge that compares instants cannot accidentally compare labels. The habit calendar is stable wherever the file is opened. SQL ranges over days work directly on the stored integer, so a year of a heatmap is one index scan.

Good: no dependency, and a leap year rule that is tested exhaustively over the range the type allows rather than trusted.

Bad: the workspace carries its own small date type, which is code to maintain and a thing a newcomer has to learn instead of recognising. The mitigation is that it is small and does one thing.

Bad: `CivilDay` has no notion of a timezone at all, so the edge that decides "what day is it for this person right now" has to be written deliberately, in one place, and passed in. That is more work than reading a clock, and it is the work that makes the rest of it testable.
