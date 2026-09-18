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
//! Two walks come out of here, and they are not one question asked twice. [`current`] measures
//! the run that is alive now and says whether it is about to end; [`longest`] measures the
//! longest there has ever been, over the whole history rather than a window, and a record is
//! never at risk and has no week in progress, so it comes back as a bare number. Nothing here
//! remembers anything between calls, and nothing here reads a clock: `today` is a parameter,
//! because which day today is depends on a time zone and on the hour the person considers a day
//! to start at, and neither is a question this crate is allowed to ask.

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

/// The longest streak the history holds, whether or not it is the one running now.
///
/// Same slice contract as [`current`]: sorted oldest first, contiguous, one entry per calendar
/// day. Unlike `current`, this one is given the **whole** history, because a record cut to a
/// window stops being a record.
#[must_use]
pub fn longest(spec: &HabitSpec, days: &[(CivilDay, DayState)], today: CivilDay) -> u32 {
    match spec.period {
        Period::Daily => longest_daily(days),
        Period::Weekly => longest_weekly(spec, days, today),
    }
}

/// The longest run of days, walked forwards, keeping the best one seen.
///
/// Today needs none of the care [`daily`] takes over it. There, an unmarked today would end a
/// run the person can still save, so it is held back and reported as a risk instead. Here the
/// answer is a maximum, and a run that is closed and a run that is broken leave exactly the
/// same number behind: the one that reached yesterday was recorded when it grew. Writing the
/// exception anyway would add a branch no history could ever tell apart from its absence.
fn longest_daily(days: &[(CivilDay, DayState)]) -> u32 {
    let mut best: u32 = 0;
    let mut run: u32 = 0;

    for &(_day, state) in days {
        if state.breaks() {
            run = 0;
            continue;
        }

        if state.counts() {
            run = run.saturating_add(1);
            best = best.max(run);
        }
    }

    best
}

/// The longest run of met ISO weeks.
///
/// The two weeks [`weekly`] steps over are stepped over here as well, and it has to be the same
/// two: the week in progress is unfinished, and a week the habit did not exist for was never
/// asked of anybody. A record that judged either of them would come out below the run the
/// person is looking at, which is a record that reads as a bug.
fn longest_weekly(spec: &HabitSpec, days: &[(CivilDay, DayState)], today: CivilDay) -> u32 {
    let this_week = iso_week(today);
    let target = u32::from(spec.target_per_period);
    let mut best: u32 = 0;
    let mut run: u32 = 0;

    for tally in tally_weeks(days) {
        if tally.week == this_week || tally.newest < spec.started_on {
            continue;
        }

        if tally.done < target {
            run = 0;
            continue;
        }

        run = run.saturating_add(1);
        best = best.max(run);
    }

    best
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{CurrentStreak, WeekProgress, current, days_left_in_week, longest};
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

    #[test]
    fn case_16_a_run_of_ten_behind_a_miss_and_a_run_of_seven_is_a_record_of_ten() {
        let spec = spec(row());
        let missed = back(today(), 7);
        let days = window(&spec, back(today(), 17), today(), &|on| on != missed);

        assert_eq!(longest(&spec, &days, today()), 10);
    }

    #[test]
    fn case_17_a_run_of_ten_that_reaches_today_is_the_record_when_it_is_the_longer_one() {
        let spec = spec(row());
        let missed = back(today(), 10);
        let days = window(&spec, back(today(), 17), today(), &|on| on != missed);

        assert_eq!(
            longest(&spec, &days, today()),
            10,
            "the record is allowed to be the run still going"
        );
    }

    #[test]
    fn case_18_the_days_a_habit_never_asked_about_do_not_cut_its_record_in_two() {
        let spec = spec(HabitRow {
            schedule_mask: MONDAY_WEDNESDAY_FRIDAY,
            ..row()
        });
        let from = back(today(), 29);
        let days = window(&spec, from, today(), &|on| {
            spec.schedule.includes(weekday(on))
        });
        let scheduled = span(from, today())
            .into_iter()
            .filter(|&on| spec.schedule.includes(weekday(on)))
            .count();
        let expected =
            u32::try_from(scheduled).expect("thirty days hold fewer sessions than a number does");

        assert_eq!(
            longest(&spec, &days, today()),
            expected,
            "the Tuesdays and Thursdays split a record that was never interrupted"
        );
    }

    #[test]
    fn case_19_a_history_of_nothing_but_misses_is_a_record_of_nothing() {
        let spec = spec(row());
        let days = window(&spec, back(today(), 29), today(), &|_no_day| false);

        assert_eq!(longest(&spec, &days, today()), 0);
    }

    #[test]
    fn case_20_no_days_at_all_is_a_record_of_nothing() {
        assert_eq!(longest(&spec(row()), &[], today()), 0);
    }

    #[test]
    fn case_21_the_record_of_a_weekly_habit_is_its_best_run_of_weeks_not_its_last() {
        let spec = spec(weekly_row(3));
        let today = weekly_today();
        let short_week_start = day(2026, 2, 16);
        let days = window(&spec, day(2026, 1, 26), today, &|on| {
            if on >= short_week_start && on < day(2026, 2, 23) {
                // Two of the three days that week asked for, which ends the run of three.
                return matches!(weekday(on), Weekday::Monday | Weekday::Tuesday);
            }

            // One single day of the week in progress, which is not judged either way.
            if on >= day(2026, 3, 9) {
                return on == day(2026, 3, 9);
            }

            true
        });

        assert_eq!(longest(&spec, &days, today), 3);
        assert_eq!(
            current(&spec, &days, today).streak.days,
            2,
            "the case only says anything while the record beats the run in progress"
        );
    }

    #[test]
    fn case_22_the_weeks_before_a_weekly_habit_existed_are_not_part_of_its_record() {
        let start_of_the_third_week = day(2026, 2, 9);
        let spec = spec(HabitRow {
            started_on: start_of_the_third_week,
            ..weekly_row(3)
        });
        let today = weekly_today();
        // Everything marked up to and including the one day of the week in progress.
        let days = window(&spec, day(2026, 1, 26), today, &|on| on <= day(2026, 3, 9));

        assert_eq!(
            longest(&spec, &days, today),
            4,
            "a week the habit did not exist for was counted as one it met"
        );
    }

    /// The habits the properties below are checked against: two shapes of daily, two of weekly.
    ///
    /// All four are built rather than sampled, because a generated `HabitSpec` would be a second
    /// implementation of the validation in `spec.rs` and would drift from it. Four is enough:
    /// what the properties are about is the walk, and the walk only sees the period, the
    /// schedule and the target.
    fn any_spec() -> impl Strategy<Value = HabitSpec> {
        prop_oneof![
            Just(spec(row())),
            Just(spec(HabitRow {
                schedule_mask: MONDAY_WEDNESDAY_FRIDAY,
                ..row()
            })),
            Just(spec(weekly_row(3))),
            Just(spec(HabitRow {
                schedule_mask: MONDAY_WEDNESDAY_FRIDAY,
                ..weekly_row(2)
            })),
        ]
    }

    /// One decision per day of a history of up to five hundred days: was it marked.
    ///
    /// Four marks in five rather than a fair coin. A fair coin spends nearly every case on runs
    /// of one or two days, which are exactly the lengths the examples above already pin down,
    /// and almost never produces a run long enough for a walk to lose its place inside.
    fn any_marks() -> impl Strategy<Value = Vec<bool>> {
        proptest::collection::vec(prop_oneof![4 => Just(true), 1 => Just(false)], 1..=500)
    }

    /// The slice `current` asks for, built from a run of marks that ends on today.
    ///
    /// A day outside the schedule is never marked, which is what keeps a generated history
    /// coherent with the habit it belongs to. It also keeps [`DayState::Extra`] out of it, and
    /// that is deliberate: a day off that was done anyway carries the run forward without being
    /// one of the days the habit asked for, so it is the one state that puts the third property
    /// out of reach. It has its own example above.
    fn history(spec: &HabitSpec, marks: &[bool], today: CivilDay) -> Vec<(CivilDay, DayState)> {
        let length = i32::try_from(marks.len()).expect("a history of at most five hundred days");
        let from = back(today, length - 1);

        span(from, today)
            .into_iter()
            .zip(marks)
            .map(|(on, &marked)| {
                let entry = (marked && spec.schedule.includes(weekday(on))).then(|| mark(on));

                (on, classify(spec, on, entry, today))
            })
            .collect()
    }

    /// The same marks with the one at `at` turned on, without indexing into anything.
    fn marking(marks: &[bool], at: usize) -> Vec<bool> {
        marks
            .iter()
            .enumerate()
            .map(|(position, &marked)| marked || position == at)
            .collect()
    }

    /// The same marks with the one at `at` turned off.
    fn unmarking(marks: &[bool], at: usize) -> Vec<bool> {
        marks
            .iter()
            .enumerate()
            .map(|(position, &marked)| marked && position != at)
            .collect()
    }

    /// Where in a history the days that satisfy `wanted` are.
    fn positions(days: &[(CivilDay, DayState)], wanted: &dyn Fn(DayState) -> bool) -> Vec<usize> {
        days.iter()
            .enumerate()
            .filter_map(|(position, &(_day, state))| wanted(state).then_some(position))
            .collect()
    }

    proptest! {
        /// Catches a walk that treats a newly met day as the beginning of a run rather than as
        /// the joint between the two that surrounded the miss it replaced. Such a walk would
        /// answer one where it should answer eleven, so a person who filled in a day they had
        /// forgotten would watch their streak collapse for having done more. No example finds
        /// it: it needs a miss with a run of its own on either side, and the obvious examples
        /// put the miss at one end.
        #[test]
        fn marking_a_missed_day_never_shortens_the_run(
            spec in any_spec(),
            marks in any_marks(),
            pick in any::<prop::sample::Index>(),
        ) {
            let days = history(&spec, &marks, today());
            let missed = positions(&days, &DayState::breaks);

            if missed.is_empty() {
                return Ok(());
            }

            let before = current(&spec, &days, today()).streak.days;
            let filled = history(&spec, &marking(&marks, *pick.get(&missed)), today());
            let after = current(&spec, &filled, today()).streak.days;

            prop_assert!(
                after >= before,
                "marking one more day took the streak from {before} down to {after}"
            );
        }

        /// Catches anything remembered between calls: a cached tally, a lazily filled cell, a
        /// counter that lives outside the function. Every example above calls `current` once
        /// against a history it built itself, so a walk that folded its answer into state kept
        /// on the side would agree with all of them and only disagree the second time the same
        /// history is asked about, which is what the interface does every time a person marks a
        /// day and unmarks it again.
        #[test]
        fn unmarking_a_day_and_marking_it_again_changes_nothing(
            spec in any_spec(),
            marks in any_marks(),
            pick in any::<prop::sample::Index>(),
        ) {
            let days = history(&spec, &marks, today());
            let met = positions(&days, &DayState::counts);

            if met.is_empty() {
                return Ok(());
            }

            let at = *pick.get(&met);
            let before = current(&spec, &days, today());

            let undone = unmarking(&marks, at);
            let _ = current(&spec, &history(&spec, &undone, today()), today());
            let redone = history(&spec, &marking(&undone, at), today());

            prop_assert_eq!(current(&spec, &redone, today()), before);
        }

        /// Catches a walk that counts squares of the calendar instead of the habit's own days:
        /// one that added a day per entry it stepped over rather than per entry that counted
        /// would sail past this bound the moment the schedule leaves a weekday out, or the
        /// window opens before the habit did. The bound is the honest ceiling — a habit cannot
        /// have been kept more times than it was asked for.
        #[test]
        fn a_run_never_outlasts_the_days_the_habit_asked_for(
            spec in any_spec(),
            marks in any_marks(),
        ) {
            let days = history(&spec, &marks, today());
            let asked = days
                .iter()
                .filter(|&&(on, _state)| {
                    on >= spec.started_on && spec.schedule.includes(weekday(on))
                })
                .count();
            let asked = u32::try_from(asked).expect("five hundred days fit in a number");
            let run = current(&spec, &days, today()).streak.days;

            prop_assert!(
                run <= asked,
                "a run of {run} out of {asked} days the habit was ever expected on"
            );
        }

        /// Catches the two walks drifting apart. They apply the same three exceptions — the
        /// week in progress, the weeks before the habit existed, the days it never asked about
        /// — and either walk can lose one without any example noticing, because the examples
        /// check each walk on its own. The symptom on screen is the one a person would report
        /// as a bug: a record smaller than the streak printed above it.
        #[test]
        fn the_record_always_reaches_the_run_in_progress(
            spec in any_spec(),
            marks in any_marks(),
        ) {
            let days = history(&spec, &marks, today());
            let answer = current(&spec, &days, today());

            if answer.reached_window_start {
                // The run was still going at the oldest day given, so the history holds only
                // part of it, and a record measured inside the same history is not being
                // compared with a run the same history contains.
                return Ok(());
            }

            let record = longest(&spec, &days, today());

            prop_assert!(
                record >= answer.streak.days,
                "a record of {record} behind a run of {} that fits inside the history",
                answer.streak.days
            );
        }
    }
}
