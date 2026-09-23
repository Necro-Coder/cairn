# 0012 — What a streak means, and where each part of it is decided

Date: 2026-09-22 · Status: accepted

## Context

A streak is the one number a habit tracker is judged on, and it is the one that is easiest to get subtly wrong. Wrong by one day looks exactly like a person who missed a day, so it is a defect that never reports itself: nobody files a bug saying their streak is off by one, they conclude they broke it and stop opening the application.

Four questions have to be answered before a run of days can be counted, and each of them has an answer that is obvious and wrong.

**Which day is today?** Read from a clock, in whichever zone the machine happens to be in, at whatever hour the reader considers midnight. A person awake at one in the morning is living the previous day by any reasonable account of it, and a file opened from a second machine in another zone must not disagree with the first about which squares are filled.

**Does a day the habit never asked for break the run?** A habit scheduled on Mondays, Wednesdays and Fridays has four days a week that it asked nothing of. If those break the streak, such a habit can never have one.

**Does today break it?** A day being lived is not a day that failed. If today breaks the run, a person who opens the application before breakfast watches a year of work reset in front of them, for the crime of being early.

**What does a weekly habit count?** Three runs out of a target of three is a finished week, not three days out of seven missed. And the week in progress is not a week that failed; it is a week that has not finished.

Before this decision each of these was answered wherever it was needed. A streak, a month percentage and a heat map are three readings of the same underlying question — was this day good, bad, or none of the application's business — and asking it three times in three features is precisely how the three end up disagreeing about the same square.

## Decision

**The question is asked once.** `cairn-domain::habits::day::classify` takes a habit's rules, the mark that may or may not exist for a day, and today, and returns one of five variants: `Done`, `Missed`, `NotScheduled`, `Extra`, `NoData`. Everything downstream matches on one of those five and never looks at an entry again. The function is total — every combination has an answer — so there is no error case for a caller to invent a display for.

**The two questions a streak asks are methods on that answer,** not conditions rewritten at each call site. `DayState::counts` is true for `Done` and `Extra`. `DayState::breaks` is true for `Missed` alone. A day that neither counts nor breaks is the reason a schedule of three days a week has a streak at all, and it is easy to get wrong twice in two different ways.

**`Extra` counts.** A day outside the schedule that was met carries the run forward. Refusing it would mean a person who trains on Mondays and trains on a Tuesday as well is punished for the extra session. `Extra` only exists for a habit being built: not smoking on a day the habit never asked about is an ordinary day, not an extra session, and awarding a streak for it would hand one out for doing nothing at all.

**Today never breaks a run.** In the daily walk, a `Missed` whose day is today is skipped and reported as `Streak::at_risk` instead. That flag is part of the value rather than something the interface works out, because the interface does not have the calendar and would have to be handed one to ask the question again. The difference it draws is between "you lost it" and "you have until midnight".

**The week in progress never breaks one either,** for the same reason, and is reported on its own as `Streak::week_progress` — done and target, uncapped, so a person who did five of three sees five. Weeks are ISO weeks, Monday to Sunday. A week the habit did not yet exist for is skipped rather than judged, or every streak would end at the week its owner signed up.

**Polarity is a direction on the target, not a negative target.** `Direction::AtLeast` is met when there is a mark and its amount reaches the target; `Direction::AtMost` is met when the amount stays at or under it, _including when there is no mark at all_. The asymmetry is the whole point: a habit being built needs a mark, because an absent row means nothing happened; a habit being cut down is met by the absence itself, which is what "did not smoke today" is.

**Nothing in the domain reads a clock.** `today` is a parameter everywhere — `classify`, `current`, `longest`, the month ratio. Which day is today depends on a time zone and on the hour a person considers their day to start at, and neither is a question this crate is allowed to ask.

**The answer to it is assembled in one place.** The command layer reads the system zone on every call, reads the `time.day_start_offset_minutes` preference, and turns the two into a `CivilDay`. A preference that is present but is not a number of minutes is a `Storage` error, not silently midnight: a damaged preference that quietly becomes the default is a streak counted against the wrong day with nothing on screen to say so. No zone at all is its own error with its own sentence, because it is the one failure in the module a person can fix themselves.

**The day travels with the row.** Every habit the interface receives carries `todayDay`, the `YYYYMMDD` the core judged its square against, and marking sends that number back. The browser's clock is never consulted, and cannot be: it does not know about the preference and it may not be in the same zone.

The split between the layers is therefore:

| Layer          | What it decides                                                                |
| -------------- | ------------------------------------------------------------------------------ |
| SQL            | Which rows fall in a window of days. Nothing about whether a day is good       |
| `cairn-domain` | Everything a day, a run, a record and a month mean, from values handed in      |
| Command layer  | What today is: the zone, the day-start preference, and the size of the windows |
| Interface      | How a number is worded in Spanish, and nothing else                            |

## Alternatives considered (and why not)

**Classifying in SQL.** A `CASE` over the entries table answers the whole question in one statement, and it was measured: aggregating a whole year in Rust costs 0,02 ms of the 0,14 ms the year takes, and 0,21 ms of the 1,23 ms a ten-year history takes. The cost is the query, not the classification. Moving the rules into SQL would buy nothing measurable and would put the definition of a streak in a place with no types, no tests that run without a database, and no way to be read by somebody auditing the application.

**A grace period, or a freeze somebody can spend.** The kindest-looking feature on the list, and the one that makes the number meaningless. A streak that can be repaired is not a record of anything. Pauses — a declared stretch where a missed day does not break a run, for an illness or a holiday — are a different thing and have a table waiting for them; they are declared in advance and visible on the calendar, which is what separates them from a retroactive excuse.

**Letting the interface work out whether a streak is at risk.** It has the number and it has the state of today, so it looks like it could. It cannot: at risk for a weekly habit depends on how many days are left in the ISO week, and giving the WebView a calendar to answer that is giving a second implementation of the calendar to something that must never disagree with the first.

**Reading today from the browser.** One line, and wrong in three ways: the WebView does not know about the day-start preference, it has its own idea of the zone, and it makes the answer depend on which side of the bridge asked.

**Counting weeks Sunday to Saturday, or from the day the habit started.** ISO weeks are what a calendar on a wall shows in the places this application is used, and a week that starts on the day somebody created a habit is a week nobody can point at.

## Consequences (good and bad)

Good: one definition, in a crate with no input or output, tested as ordinary functions over ordinary values. The streak, the month and the heat map cannot disagree about a square, because they are three readings of one answer. A streak survives a flight: the file opened in another zone shows the same filled squares, because a marked day is a `CivilDay` and not an instant. The interface cannot get the rules wrong, because it is handed conclusions rather than the material to draw them from.

Bad: the command layer fetches a window of four hundred days to answer a question about one number, and fetches a second window when the run is still alive at the oldest day of the first. That is two statements for a long streak where a clever `CASE` would be one. It also means a streak longer than eight hundred days is reported as eight hundred, which nobody has yet and which is a bound preferred to an unbounded walk.

Bad: `at_risk` and `week_progress` travel on every habit in the list, whether the screen draws them or not. The alternative is the interface asking again per habit, which is worse in every way that matters.

Bad: the day-start preference is read but no screen sets it. The code path is exercised only by tests until there is one, which is the kind of gap that rots. It is named in the user documentation as not yet settable rather than left to be discovered.
