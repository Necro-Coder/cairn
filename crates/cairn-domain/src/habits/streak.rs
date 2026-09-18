//! How long the run is, and whether it is about to end.
//!
//! A streak is the one number a habit tracker is judged on, and it is the easiest one to get
//! subtly wrong, because the two interesting cases are not the run itself. They are today,
//! which is unfinished and must not be allowed to break anything, and the days the habit never
//! asked about, which must not break anything either. A habit scheduled on three days a week
//! would otherwise end its own streak four times every week, and a person opening the
//! application in the morning would watch a year of work reset before breakfast.
//!
//! So both live here, once, and they travel inside the answer. [`Streak::at_risk`] is part of
//! the value rather than something the interface works out, because the interface does not have
//! the calendar and would have to be handed one to ask the question again.
//!
//! What comes out is the *current* streak only. The record is a different walk over a different
//! window and lives elsewhere; nothing here remembers anything between calls, and nothing here
//! reads a clock: `today` is a parameter, because which day today is depends on a time zone and
//! on the hour the person considers a day to start at, and neither is a question this crate is
//! allowed to ask.

use crate::habits::calendar::{IsoWeek, Weekday, iso_week, weekday};
use crate::habits::day::DayState;
use crate::habits::spec::{HabitSpec, Period};
use crate::time::CivilDay;

/// A streak, and everything the interface needs to draw it without deducing anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streak {
    /// How long it is. Days for a daily habit, weeks for a weekly one.
    pub days: u32,
    /// Whether today can still be saved and has not been done yet.
    ///
    /// Part of the value on purpose. A streak that travels without saying whether it is about to
    /// break makes the interface work it out, and the interface does not have the calendar.
    pub at_risk: bool,
    /// For a weekly habit, how the week in progress is going. `None` for a daily one.
    pub week_progress: Option<WeekProgress>,
}

/// How many times the habit has been met this week, and how many it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeekProgress {
    /// Days met so far this week. Not capped: a person who did five of three sees five.
    pub done: u32,
    /// Days the week asks for.
    pub target: u32,
}

/// What [`current`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrentStreak {
    /// The run, as the interface will draw it.
    pub streak: Streak,
    /// The streak was still unbroken at the oldest day in the slice, so the real one may be longer.
    pub reached_window_start: bool,
}

/// The current streak, counted backwards from today.
///
/// `days` must be sorted oldest first, must end on `today`, and must be contiguous: one entry per
/// calendar day with no gaps. The caller builds it with `calendar::span` and `day::classify`.
///
/// `reached_window_start` is true when the streak was still running at the oldest day given,
/// which is how the caller knows to fetch an older window and ask again.
#[must_use]
pub fn current(spec: &HabitSpec, days: &[(CivilDay, DayState)], today: CivilDay) -> CurrentStreak {
    match spec.period {
        Period::Daily => daily(spec, days, today),
        Period::Weekly => weekly(spec, days, today),
    }
}

/// A daily habit's run, one square at a time.
///
/// Today is the only square allowed to be missed without ending anything, because a day being
/// lived is not a day that failed. It is reported as [`Streak::at_risk`] instead, which is the
/// difference between "you lost it" and "you have until midnight".
fn daily(spec: &HabitSpec, days: &[(CivilDay, DayState)], today: CivilDay) -> CurrentStreak {
    let mut length: u32 = 0;
    let mut at_risk = false;
    let mut unbroken = true;

    for &(day, state) in days.iter().rev() {
        if state.breaks() {
            // The only square that is still open. A habit is missed on the day it is being
            // lived only in the sense that it has not been done yet.
            if day == today {
                at_risk = true;
                continue;
            }

            unbroken = false;
            break;
        }

        if state.counts() {
            length = length.saturating_add(1);
        }
    }

    CurrentStreak {
        streak: Streak {
            days: length,
            at_risk,
            week_progress: None,
        },
        reached_window_start: reached_window_start(spec, days, unbroken),
    }
}

/// A weekly habit's run, one ISO week at a time.
///
/// The week in progress never breaks anything, for the same reason today never does in a daily
/// habit: it is unfinished. It is taken out of the count and reported on its own in
/// [`Streak::week_progress`], so an interface can draw "two of three" without asking the
/// calendar a second question.
fn weekly(spec: &HabitSpec, days: &[(CivilDay, DayState)], today: CivilDay) -> CurrentStreak {
    let tallies = tally_weeks(days);
    let this_week = iso_week(today);
    let target = u32::from(spec.target_per_period);
    let done = tallies
        .iter()
        .find(|tally| tally.week == this_week)
        .map_or(0, |tally| tally.done);

    let mut length: u32 = 0;
    let mut unbroken = true;

    for tally in tallies.iter().rev() {
        // A week the habit did not exist for cannot have been met, and judging it would end
        // every streak at the week the person signed up.
        if tally.week == this_week || tally.newest < spec.started_on {
            continue;
        }

        if tally.done < target {
            unbroken = false;
            break;
        }

        length = length.saturating_add(1);
    }

    CurrentStreak {
        streak: Streak {
            days: length,
            at_risk: at_risk_this_week(done, target, weekday(today)),
            week_progress: Some(WeekProgress { done, target }),
        },
        reached_window_start: reached_window_start(spec, days, unbroken),
    }
}

/// One ISO week of the slice, reduced to the two things the walk asks about.
#[derive(Debug, Clone, Copy)]
struct WeekTally {
    /// Which week this is.
    week: IsoWeek,
    /// How many of its days carried the run forward.
    done: u32,
    /// The latest day of it present in the slice, which is what dates the week against
    /// `started_on` without assuming the slice covers the whole of it.
    newest: CivilDay,
}

/// The slice grouped into weeks, oldest first.
///
/// The slice is contiguous and sorted by contract, so a week is closed by the first day that
/// belongs to the next one and never reopens. Grouping this way rather than into a map keeps
/// the result in calendar order, which is the order the walk needs.
fn tally_weeks(days: &[(CivilDay, DayState)]) -> Vec<WeekTally> {
    let mut tallies: Vec<WeekTally> = Vec::new();

    for &(day, state) in days {
        let week = iso_week(day);
        let met = u32::from(state.counts());

        match tallies.last_mut() {
            Some(last) if last.week == week => {
                last.done = last.done.saturating_add(met);
                last.newest = day;
            }
            _ => tallies.push(WeekTally {
                week,
                done: met,
                newest: day,
            }),
        }
    }

    tallies
}

/// Whether the week in progress can still reach its target.
///
/// Counted in whole days left including today, not in scheduled ones: a person can mark a habit
/// on a day it was not scheduled for, and that day counts, so refusing to count it here would
/// raise the alarm on a week that is still perfectly winnable.
fn at_risk_this_week(done: u32, target: u32, today: Weekday) -> bool {
    let needed = target.saturating_sub(done);

    needed > 0 && days_left_in_week(today) < needed
}

/// Days left in the ISO week counting today, which is a day the person can still act on.
const fn days_left_in_week(today: Weekday) -> u32 {
    match today {
        Weekday::Monday => 7,
        Weekday::Tuesday => 6,
        Weekday::Wednesday => 5,
        Weekday::Thursday => 4,
        Weekday::Friday => 3,
        Weekday::Saturday => 2,
        Weekday::Sunday => 1,
    }
}

/// Whether an older window would tell the caller anything new.
///
/// Only when the walk ran out of slice rather than out of streak, and only when there is older
/// calendar to ask for: a window that already reaches the day the habit started, or reaches
/// past it, has nothing behind it, and answering true there would send the caller back to the
/// database for a second slice of days that do not exist.
fn reached_window_start(spec: &HabitSpec, days: &[(CivilDay, DayState)], unbroken: bool) -> bool {
    unbroken
        && days
            .first()
            .is_some_and(|&(oldest, _)| oldest > spec.started_on)
}

#[cfg(test)]
mod tests {
    use super::{CurrentStreak, WeekProgress, current, days_left_in_week};
    use crate::habits::calendar::{Weekday, shift, span, weekday};
    use crate::habits::day::{DayState, Entry, classify};
    use crate::habits::spec::{HabitRow, HabitSpec};
    use crate::time::CivilDay;

    /// Mondays, Wednesdays and Fridays, which is the schedule the days-off cases need.
    const MONDAY_WEDNESDAY_FRIDAY: i64 = 0b001_0101;

    fn day(year: u16, month: u8, number: u8) -> CivilDay {
        CivilDay::new(year, month, number).expect("the test named a day that exists")
    }

    /// The day every habit below starts on, far enough back that no case reaches it by accident.
    fn started_on() -> CivilDay {
        day(2025, 1, 1)
    }

    /// Daily, done or not done, more is better, every day of the week.
    fn row() -> HabitRow {
        HabitRow {
            period: 0,
            kind: 0,
            direction: 0,
            aggregation: 0,
            schedule_mask: 0,
            unit: None,
            target_per_period: None,
            started_on: started_on(),
        }
    }

    /// Weekly, done or not done, expected on `target` days of the week.
    fn weekly_row(target: i64) -> HabitRow {
        HabitRow {
            period: 1,
            target_per_period: Some(target),
            ..row()
        }
    }

    fn spec(row: HabitRow) -> HabitSpec {
        HabitSpec::from_row(row).expect("the test named a row this product accepts")
    }

    /// A mark for that day, of one, with no snapshot of the target.
    fn mark(on: CivilDay) -> Entry {
        Entry {
            day: on,
            amount: 1,
            target_snapshot: None,
        }
    }

    /// The slice `current` asks for: one entry per day from `from` to `today`, classified.
    ///
    /// `marked` decides which days carry a mark, which is how each case describes itself
    /// without hand-building five variants of [`DayState`].
    fn window(
        spec: &HabitSpec,
        from: CivilDay,
        today: CivilDay,
        marked: &dyn Fn(CivilDay) -> bool,
    ) -> Vec<(CivilDay, DayState)> {
        span(from, today)
            .into_iter()
            .map(|on| {
                let entry = marked(on).then(|| mark(on));

                (on, classify(spec, on, entry, today))
            })
            .collect()
    }

    /// `count` days back from `today`, inclusive, which is where every daily case starts.
    fn back(today: CivilDay, count: i32) -> CivilDay {
        shift(today, -count).expect("the window fits in the calendar")
    }

    /// A Saturday, so that the daily cases run over a whole number of weeks and back.
    fn today() -> CivilDay {
        day(2026, 3, 14)
    }

    fn streak(answer: CurrentStreak) -> (u32, bool) {
        (answer.streak.days, answer.streak.at_risk)
    }

    #[test]
    fn case_01_thirty_days_met_in_a_row_up_to_today_is_a_streak_of_thirty() {
        let spec = spec(row());
        let days = window(&spec, back(today(), 29), today(), &|_every_day| true);

        let answer = current(&spec, &days, today());

        assert_eq!(streak(answer), (30, false));
        assert_eq!(answer.streak.week_progress, None);
    }

    #[test]
    fn case_02_twenty_nine_met_and_today_still_unmarked_is_twenty_nine_at_risk() {
        let spec = spec(row());
        let days = window(&spec, back(today(), 29), today(), &|on| on != today());

        assert_eq!(streak(current(&spec, &days, today())), (29, true));
    }

    #[test]
    fn case_03_a_missed_yesterday_ends_the_run_even_though_today_is_still_open() {
        let spec = spec(row());
        let yesterday = back(today(), 1);
        let days = window(&spec, back(today(), 29), today(), &|on| {
            on != today() && on != yesterday
        });

        assert_eq!(streak(current(&spec, &days, today())), (0, true));
    }

    #[test]
    fn case_04_the_days_a_habit_never_asked_about_do_not_end_its_run() {
        let spec = spec(HabitRow {
            schedule_mask: MONDAY_WEDNESDAY_FRIDAY,
            ..row()
        });
        let days = window(&spec, back(today(), 29), today(), &|on| {
            spec.schedule.includes(weekday(on))
        });

        let answer = current(&spec, &days, today());

        assert!(
            answer.streak.days > 0,
            "the Tuesdays and Thursdays ended a run they were never part of"
        );
        assert!(!answer.streak.at_risk);
    }

    #[test]
    fn case_05_a_today_outside_the_schedule_is_not_at_risk() {
        // Today is a Saturday, and the habit is only expected on Mondays, Wednesdays and Fridays.
        let spec = spec(HabitRow {
            schedule_mask: MONDAY_WEDNESDAY_FRIDAY,
            ..row()
        });
        let days = window(&spec, back(today(), 29), today(), &|on| {
            spec.schedule.includes(weekday(on))
        });

        assert!(!current(&spec, &days, today()).streak.at_risk);
    }

    #[test]
    fn case_06_a_window_met_end_to_end_that_starts_after_the_habit_did_asks_for_an_older_one() {
        let spec = spec(row());
        let days = window(&spec, back(today(), 29), today(), &|_every_day| true);

        assert!(current(&spec, &days, today()).reached_window_start);
    }

    #[test]
    fn case_07_a_window_met_end_to_end_that_starts_where_the_habit_did_asks_for_nothing() {
        let first_day = day(2026, 2, 13);
        let spec = spec(HabitRow {
            started_on: first_day,
            ..row()
        });
        let days = window(&spec, first_day, today(), &|_every_day| true);

        assert!(!current(&spec, &days, today()).reached_window_start);
    }

    #[test]
    fn case_08_no_days_at_all_is_a_streak_of_nothing_and_no_alarm() {
        let spec = spec(row());

        let answer = current(&spec, &[], today());

        assert_eq!(streak(answer), (0, false));
        assert!(!answer.reached_window_start);
    }

    #[test]
    fn case_09_today_alone_and_met_is_a_streak_of_one() {
        let spec = spec(row());
        let days = window(&spec, today(), today(), &|_every_day| true);

        assert_eq!(streak(current(&spec, &days, today())), (1, false));
    }

    /// A Wednesday, so that the week in progress has days both behind and ahead of it.
    fn weekly_today() -> CivilDay {
        day(2026, 3, 11)
    }

    /// The Monday four whole weeks before the week `weekly_today` falls in.
    fn weekly_window_start() -> CivilDay {
        day(2026, 2, 9)
    }

    /// The table of days left is checked against the calendar rather than against itself, so a
    /// transposed arm cannot agree with a transposed expectation.
    #[test]
    fn what_is_left_of_a_week_is_what_the_calendar_says_is_left_of_it() {
        let monday = day(2026, 3, 9);
        let sunday = day(2026, 3, 15);

        for on in span(monday, sunday) {
            let left = u32::try_from(span(on, sunday).len())
                .expect("a week has fewer days than a number holds");

            assert_eq!(
                days_left_in_week(weekday(on)),
                left,
                "the days left on {on:?} are not the ones between it and Sunday"
            );
        }
    }

    #[test]
    fn case_10_four_met_weeks_behind_a_week_in_progress_is_a_streak_of_four() {
        let spec = spec(weekly_row(3));
        let today = weekly_today();
        let days = window(&spec, weekly_window_start(), today, &|on| {
            // Every day of the four weeks behind, and one single day of the week in progress.
            on <= day(2026, 3, 9)
        });

        let answer = current(&spec, &days, today);

        assert_eq!(answer.streak.days, 4);
        assert_eq!(
            answer.streak.week_progress,
            Some(WeekProgress { done: 1, target: 3 })
        );
    }

    #[test]
    fn case_11_a_week_in_progress_with_nothing_done_on_a_monday_is_not_at_risk_yet() {
        let spec = spec(weekly_row(3));
        let monday = day(2026, 3, 9);
        let days = window(&spec, weekly_window_start(), monday, &|on| on < monday);

        let answer = current(&spec, &days, monday);

        assert_eq!(answer.streak.days, 4);
        assert!(!answer.streak.at_risk, "three whole days still fit");
    }

    #[test]
    fn case_12_a_week_in_progress_with_nothing_done_on_a_saturday_is_at_risk() {
        let spec = spec(weekly_row(3));
        let saturday = day(2026, 3, 14);
        let monday = day(2026, 3, 9);
        let days = window(&spec, weekly_window_start(), saturday, &|on| on < monday);

        assert!(current(&spec, &days, saturday).streak.at_risk);
    }

    #[test]
    fn case_13_an_earlier_week_two_of_three_short_ends_the_run_there() {
        let spec = spec(weekly_row(3));
        let today = weekly_today();
        let short_week_start = day(2026, 2, 23);
        let days = window(&spec, weekly_window_start(), today, &|on| {
            if on >= short_week_start && on < day(2026, 3, 2) {
                // Two of the three days that week asked for.
                return matches!(weekday(on), Weekday::Monday | Weekday::Tuesday);
            }

            on <= day(2026, 3, 9)
        });

        let answer = current(&spec, &days, today);

        assert_eq!(answer.streak.days, 1, "only the week after the short one");
        assert!(
            !answer.reached_window_start,
            "the run ended inside the window"
        );
    }

    #[test]
    fn case_14_a_week_in_progress_past_its_target_reports_what_was_really_done() {
        let spec = spec(weekly_row(3));
        let sunday = day(2026, 3, 15);
        let days = window(&spec, weekly_window_start(), sunday, &|_every_day| true);

        let answer = current(&spec, &days, sunday);

        assert_eq!(
            answer.streak.week_progress,
            Some(WeekProgress { done: 7, target: 3 }),
            "the week in progress is reported as lived, not capped at the target"
        );
        assert!(!answer.streak.at_risk);
    }

    #[test]
    fn case_15_a_day_outside_the_schedule_that_was_done_anyway_counts_towards_the_week() {
        let spec = spec(HabitRow {
            schedule_mask: MONDAY_WEDNESDAY_FRIDAY,
            ..weekly_row(3)
        });
        let sunday = day(2026, 3, 8);
        // Monday and Wednesday are scheduled; Sunday is not, and is the third day of the week.
        let days = window(&spec, day(2026, 3, 2), sunday, &|on| {
            matches!(
                weekday(on),
                Weekday::Monday | Weekday::Wednesday | Weekday::Sunday
            )
        });
        let sunday_state = days
            .iter()
            .find_map(|&(on, state)| (on == sunday).then_some(state));

        assert!(
            matches!(sunday_state, Some(DayState::Extra { .. })),
            "the case needs an extra day to be about anything"
        );
        assert_eq!(
            current(&spec, &days, sunday).streak.week_progress,
            Some(WeekProgress { done: 3, target: 3 }),
            "the extra day did not count towards the week"
        );
    }
}
