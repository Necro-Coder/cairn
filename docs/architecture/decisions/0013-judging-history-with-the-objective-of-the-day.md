# 0013 — A day is judged by the target it was marked under

Date: 2026-09-22 · Status: accepted

## Context

A habit that counts a quantity carries a target: two litres, thirty pages, five kilometres. The target is a column on the habit, and the habit is a row somebody edits.

So the obvious implementation judges every day of the history against the target as it is now. It is one column, one comparison, and it is wrong in a way that only shows up months later, on the exact day somebody decides to ask more of themselves.

Raise the water goal from two litres to three, and every two-litre day of the past year turns red at once. A calendar somebody actually lived changes colour because of a decision taken this morning. The streak that was running yesterday is gone, and nothing on the screen explains why, because from the application's point of view nothing happened: the same rows are being compared against a different number.

A record that a later edit can rewrite is not a record. It is also the opposite of what raising a goal is for: the one moment a person is most likely to be doing well is the moment this turns their history into a wall of failures.

The same question could be asked about every other part of a habit's definition — the schedule, the direction, the period, the aggregation — and answering it the same way for all of them is a different and much larger decision.

## Decision

**The target is snapshotted per entry. Nothing else is.**

`habit_entries.target_snapshot` is written next to the mark, holding the target that was in force the day it was written. When a day is classified, that number wins over the habit's current target. It is nullable, and null in two cases that are the same case: a habit that is done or not done has no target to remember, and a row written before the column existed has none stored. Both fall back to the habit's current target, which is exactly what was happening to them the day before the migration ran.

Everything else about a habit is read as it is now. Change the schedule and the whole calendar is redrawn against the new one; change the direction and every day means the opposite; change the period and the run is counted in weeks instead of days. Those are not silently absorbed: the form asks the core what a change would mean before saving it, and a change that alters the meaning of the history shows the streak as it is and as it would become, with nothing written until it is accepted.

Days that fall outside a new schedule keep their marks. They stop counting and are drawn as days the habit no longer asks for. Narrowing a schedule tells you how many marks it puts outside, before saving.

## Alternatives considered (and why not)

**Judge everything by the habit as it is now.** The one-column implementation. It is what was there before this decision, and the failure above is not hypothetical: it is the manual check that exists because it is the cheapest possible way to destroy somebody's year.

**Version the habit, and judge each day against the version in force.** The complete answer, and the honest one. Every edit writes a new row; every day looks up the version covering it. It makes all five parts of the definition behave the way the target does, with no asymmetry to explain.

It was rejected on cost, not on correctness. It turns one join into a temporal join in every query that touches a day, it puts a second kind of history into a file that a merge has to reconcile, and it makes "what is this habit" a question with a date in it. It buys correctness for four fields that nobody edits in the middle of a run, at the price of complexity in the one part of the module that has to stay fast. If somebody later demonstrates that people do change schedules and directions mid-run often enough to matter, this is the decision to revisit, and this record is the reason it would be a new number rather than an edit here.

**Snapshot the whole definition per entry.** Versioning's cost without its structure: the schedule, the direction, the period and the aggregation copied onto every row of every day. A ten-year habit is three and a half thousand copies of four values that changed twice, and a merge has no way to tell a stale copy from a deliberate one.

**Refuse to let the target be edited, and make people create a new habit.** Keeps the history perfect and throws away the thing habits are for. Raising a goal is progress, and the application should not answer it by making somebody start again at zero.

**Store the target snapshot on the habit with a date range instead of on the entry.** Fewer rows, and it reintroduces the temporal join this decision exists to avoid, for the one field it applies to.

## Consequences (good and bad)

Good: a day's verdict is fixed the moment it is marked and no later edit reaches it. Raising a goal costs nothing and breaks nothing. The heat map of a past year is stable, which is what makes it worth drawing. The lookup is a column on a row that has already been fetched, so it costs nothing in a query.

Good: the column is nullable, so the migration is additive and no existing row had to be rewritten to a value somebody guessed.

Bad: the asymmetry has to be explained, and it is the kind that looks like an oversight. A person who changes a schedule and sees the whole calendar redraw, having just been told that changing a target does not, has been surprised by an inconsistency that is deliberate. The warning before saving exists partly to say so at the moment it matters.

Bad: a habit's history can be judged by a target that no longer appears anywhere on the screen. Two litres is written on a row from March and three is written on the habit, and only the calendar knows. The detail screen does not yet show what a given day was judged against; a square that could say "2 of 2 litres, which was the goal then" would close that gap, and does not exist.

Bad: the fallback for null is the current target, which means rows written before migration 0006 do move if the target is edited. That is a bounded and knowingly accepted inconsistency — it is the old behaviour, applied only to rows that predate the fix — rather than a backfill of a value that was never recorded.
