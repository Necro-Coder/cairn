//! What one square of the calendar means.
//!
//! A streak, a percentage and a heat map are three readings of the same underlying question,
//! asked once per day: was this day good, bad, or none of the application's business. Answering
//! it takes the habit's rules, the mark that may or may not exist for that day, and today's
//! date; answering it three times, once inside each of those three features, is how the three
//! end up disagreeing about the same square.
//!
//! So it is answered once, here, and what comes out is a small closed set of five variants.
//! Everything downstream matches on one of those five and never looks at an entry again. That
//! is also why the two questions a streak asks — does this square carry the run forward, does
//! it end it — are methods on the answer rather than conditions rewritten at each call site: a
//! day that neither counts nor breaks is the reason a habit with a schedule of three days a
//! week has a streak at all, and it is easy to get wrong twice in two different ways.
//!
//! Nothing here reads a clock. `today` is a parameter, because which day is today depends on
//! the person's time zone and on the hour they consider a day to start at, and neither of those
//! is a question this crate is allowed to ask. Nothing here is fallible either: every
//! combination of a habit, a day, a mark and a today has one of the five answers, so there is
//! no error for a caller to invent a display for.

use crate::habits::calendar::weekday;
use crate::habits::spec::{Direction, HabitSpec, Measure};
use crate::time::CivilDay;

/// One mark on the calendar, as the domain sees one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The day the mark was written for.
    pub day: CivilDay,
    /// In the smallest unit the habit counts in. One, for a habit that is simply done.
    pub amount: i64,
    /// The target that was in force the day this was written.
    ///
    /// `None` for a habit that is only done or not done, and for rows written before the column
    /// existed, which are judged with the habit's current target.
    pub target_snapshot: Option<i64>,
}

/// What one square of the calendar says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayState {
    /// Expected that day, and met.
    Done {
        /// What was actually done, in the habit's unit.
        amount: i64,
        /// What was asked of that day.
        target: i64,
    },
    /// Expected that day, and not met.
    Missed {
        /// What was actually done, in the habit's unit.
        amount: i64,
        /// What was asked of that day.
        target: i64,
    },
    /// Not expected that day. A mark on it is kept and shown, and counts for nothing.
    NotScheduled {
        /// What was done anyway, which is zero when there is no mark at all.
        amount: i64,
    },
    /// Not expected that day, but met anyway.
    Extra {
        /// What was done, in the habit's unit.
        amount: i64,
        /// What would have been asked, had the day been one of the habit's.
        target: i64,
    },
    /// Before the habit existed, or after today. Nothing to say.
    NoData,
}

impl DayState {
    /// Whether this square carries a streak forward.
    ///
    /// A day outside the schedule that was done anyway counts, because refusing it would mean a
    /// person who trains on Mondays and trains on a Tuesday as well is punished for the extra
    /// session.
    #[must_use]
    pub const fn counts(self) -> bool {
        matches!(self, Self::Done { .. } | Self::Extra { .. })
    }

    /// Whether this square ends one.
    ///
    /// Only a day that was asked for and not delivered. A day the habit never asked about, and
    /// a day outside the habit's life, leave the run exactly as they found it: a schedule of
    /// three days a week would otherwise break its own streak four times every week.
    #[must_use]
    pub const fn breaks(self) -> bool {
        matches!(self, Self::Missed { .. })
    }
}

/// Classifies one day.
///
/// `today` arrives from the caller and is never read from a clock here.
///
/// `entry` is the mark for `day`, which the caller has already paired with it; the day carried
/// inside it is what the row stores, not a second opinion this function weighs against its
/// argument.
#[must_use]
pub fn classify(
    spec: &HabitSpec,
    day: CivilDay,
    entry: Option<Entry>,
    today: CivilDay,
) -> DayState {
    // Outside the habit's life, and that includes a day carrying a mark: an entry written
    // before the habit started, or dated in the future, describes a square the person has no
    // business being judged on. Today itself is inside, which is what makes a habit failable
    // on the day it is being lived rather than only in hindsight.
    if day < spec.started_on || day > today {
        return DayState::NoData;
    }

    let target = target_for(spec, entry);
    let amount = entry.map_or(0, |mark| mark.amount);
    let met = is_met(spec.direction, amount, target, entry.is_some());

    if spec.schedule.includes(weekday(day)) {
        return if met {
            DayState::Done { amount, target }
        } else {
            DayState::Missed { amount, target }
        };
    }

    match spec.direction {
        // Only a habit somebody is building can be overshot on a day off. Not smoking on a day
        // the habit never asked about is not an extra session; it is an ordinary day, and
        // counting it would hand out a streak for doing nothing at all.
        Direction::AtLeast if met => DayState::Extra { amount, target },
        Direction::AtLeast | Direction::AtMost => DayState::NotScheduled { amount },
    }
}

/// What that day asked for.
///
/// The snapshot wins where there is one, because a day judged against a target the person only
/// set last week is a day whose verdict changes under them: raising a goal from two litres to
/// three would turn every two-litre day of the past year red at once.
fn target_for(spec: &HabitSpec, entry: Option<Entry>) -> i64 {
    match &spec.measure {
        // A habit with no quantity still has a number behind it, and which number depends on
        // which way it is read: one thing done, or nothing done at all.
        Measure::DoneOrNot => match spec.direction {
            Direction::AtLeast => 1,
            Direction::AtMost => 0,
        },
        Measure::Quantity { target, .. } => entry
            .and_then(|mark| mark.target_snapshot)
            .unwrap_or(*target),
    }
}

/// Whether the day did what was asked of it.
///
/// The two directions are not mirror images, and the asymmetry is the whole point. A habit
/// being built needs a mark to have been met: an absent row means nothing happened. A habit
/// being cut down is met by the absence itself, which is what "did not smoke today" is.
///
/// Comparisons only, on numbers that came from a database and from a WebView. Nothing here
/// adds, subtracts or scales them, so there is no width for either of those sources to push a
/// value past.
const fn is_met(direction: Direction, amount: i64, target: i64, marked: bool) -> bool {
    match direction {
        Direction::AtLeast => marked && amount >= target,
        Direction::AtMost => amount <= target,
    }
}

#[cfg(test)]
mod tests {
    use super::{DayState, Entry, classify};
    use crate::habits::spec::{HabitRow, HabitSpec};
    use crate::time::CivilDay;

    /// Only Thursday, so that a day in the schedule and a day outside it are one apart.
    const THURSDAYS_ONLY: i64 = 0b000_1000;

    /// The day the habits below start on, which is a Thursday.
    fn started_on() -> CivilDay {
        day(2026, 1, 1)
    }

    /// A Thursday well inside the habit's life, which is the day most cases are about.
    fn thursday() -> CivilDay {
        day(2026, 1, 15)
    }

    /// The Friday after it, which is the day off in a schedule of Thursdays.
    fn friday() -> CivilDay {
        day(2026, 1, 16)
    }

    /// Today, for every case that is not about the edge of today itself.
    fn today() -> CivilDay {
        day(2026, 1, 31)
    }

    fn day(year: u16, month: u8, number: u8) -> CivilDay {
        CivilDay::new(year, month, number).expect("the test named a day that exists")
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

    fn spec(row: HabitRow) -> HabitSpec {
        HabitSpec::from_row(row).expect("the test named a row this product accepts")
    }

    /// A habit that counts millilitres towards `target`.
    fn quantity(target: i64) -> HabitRow {
        HabitRow {
            kind: 1,
            unit: Some("ml".to_owned()),
            target_per_period: Some(target),
            ..row()
        }
    }

    /// A mark for that day with no snapshot of the target.
    fn entry(on: CivilDay, amount: i64) -> Entry {
        Entry {
            day: on,
            amount,
            target_snapshot: None,
        }
    }

    #[test]
    fn case_01_a_scheduled_day_with_a_mark_is_done() {
        let state = classify(
            &spec(row()),
            thursday(),
            Some(entry(thursday(), 1)),
            today(),
        );

        assert_eq!(
            state,
            DayState::Done {
                amount: 1,
                target: 1
            }
        );
    }

    #[test]
    fn case_02_a_scheduled_day_with_no_mark_is_missed() {
        let state = classify(&spec(row()), thursday(), None, today());

        assert_eq!(
            state,
            DayState::Missed {
                amount: 0,
                target: 1
            }
        );
    }

    #[test]
    fn case_03_a_day_off_with_no_mark_is_not_scheduled() {
        let state = classify(
            &spec(HabitRow {
                schedule_mask: THURSDAYS_ONLY,
                ..row()
            }),
            friday(),
            None,
            today(),
        );

        assert_eq!(state, DayState::NotScheduled { amount: 0 });
    }

    #[test]
    fn case_04_a_day_off_that_was_done_anyway_is_extra() {
        let state = classify(
            &spec(HabitRow {
                schedule_mask: THURSDAYS_ONLY,
                ..row()
            }),
            friday(),
            Some(entry(friday(), 1)),
            today(),
        );

        assert_eq!(
            state,
            DayState::Extra {
                amount: 1,
                target: 1
            }
        );
    }

    #[test]
    fn case_05_a_day_before_the_habit_started_says_nothing_even_with_a_mark() {
        let before = day(2025, 12, 31);
        let state = classify(&spec(row()), before, Some(entry(before, 1)), today());

        assert_eq!(state, DayState::NoData);
    }

    #[test]
    fn case_06_a_day_after_today_says_nothing() {
        let state = classify(&spec(row()), day(2026, 2, 1), None, today());

        assert_eq!(state, DayState::NoData);
    }

    #[test]
    fn case_07_today_itself_is_a_day_that_can_already_be_missed() {
        let state = classify(&spec(row()), thursday(), None, thursday());

        assert_eq!(
            state,
            DayState::Missed {
                amount: 0,
                target: 1
            },
            "today is inside the habit's life, not beyond its end"
        );
    }

    #[test]
    fn case_08_a_quantity_that_reaches_its_target_exactly_is_done() {
        let state = classify(
            &spec(quantity(2000)),
            thursday(),
            Some(entry(thursday(), 2000)),
            today(),
        );

        assert_eq!(
            state,
            DayState::Done {
                amount: 2000,
                target: 2000
            }
        );
    }

    #[test]
    fn case_09_a_quantity_one_short_of_its_target_is_missed() {
        let state = classify(
            &spec(quantity(2000)),
            thursday(),
            Some(entry(thursday(), 1999)),
            today(),
        );

        assert_eq!(
            state,
            DayState::Missed {
                amount: 1999,
                target: 2000
            }
        );
    }

    #[test]
    fn case_10_a_quantity_well_past_its_target_keeps_the_amount_it_had() {
        let state = classify(
            &spec(quantity(2000)),
            thursday(),
            Some(entry(thursday(), 5000)),
            today(),
        );

        assert_eq!(
            state,
            DayState::Done {
                amount: 5000,
                target: 2000
            }
        );
    }

    #[test]
    fn case_11_a_mark_that_remembers_its_target_is_judged_by_that_one() {
        let state = classify(
            &spec(quantity(3000)),
            thursday(),
            Some(Entry {
                day: thursday(),
                amount: 2000,
                target_snapshot: Some(2000),
            }),
            today(),
        );

        assert_eq!(
            state,
            DayState::Done {
                amount: 2000,
                target: 2000
            },
            "raising the goal must not turn a day that was met into one that was not"
        );
    }

    #[test]
    fn case_12_a_mark_with_no_snapshot_is_judged_by_the_habit_as_it_stands_now() {
        let state = classify(
            &spec(quantity(3000)),
            thursday(),
            Some(entry(thursday(), 2000)),
            today(),
        );

        assert_eq!(
            state,
            DayState::Missed {
                amount: 2000,
                target: 3000
            }
        );
    }

    #[test]
    fn case_13_a_day_somebody_did_not_smoke_is_done_without_any_mark_at_all() {
        let state = classify(
            &spec(HabitRow {
                direction: 1,
                ..row()
            }),
            thursday(),
            None,
            today(),
        );

        assert_eq!(
            state,
            DayState::Done {
                amount: 0,
                target: 0
            }
        );
    }

    #[test]
    fn case_14_a_day_somebody_did_smoke_is_missed() {
        let state = classify(
            &spec(HabitRow {
                direction: 1,
                ..row()
            }),
            thursday(),
            Some(entry(thursday(), 1)),
            today(),
        );

        assert_eq!(
            state,
            DayState::Missed {
                amount: 1,
                target: 0
            }
        );
    }

    #[test]
    fn case_15_a_quantity_being_cut_down_is_met_at_its_ceiling() {
        let state = classify(
            &spec(HabitRow {
                direction: 1,
                ..quantity(1)
            }),
            thursday(),
            Some(entry(thursday(), 1)),
            today(),
        );

        assert_eq!(
            state,
            DayState::Done {
                amount: 1,
                target: 1
            }
        );
    }

    #[test]
    fn case_16_a_quantity_being_cut_down_is_missed_one_over_it() {
        let state = classify(
            &spec(HabitRow {
                direction: 1,
                ..quantity(1)
            }),
            thursday(),
            Some(entry(thursday(), 2)),
            today(),
        );

        assert_eq!(
            state,
            DayState::Missed {
                amount: 2,
                target: 1
            }
        );
    }

    #[test]
    fn case_17_a_day_off_from_a_habit_being_cut_down_is_never_an_extra() {
        let state = classify(
            &spec(HabitRow {
                direction: 1,
                schedule_mask: THURSDAYS_ONLY,
                ..row()
            }),
            friday(),
            Some(entry(friday(), 0)),
            today(),
        );

        assert_eq!(
            state,
            DayState::NotScheduled { amount: 0 },
            "not doing something on a day it was never asked for is not an achievement"
        );
    }

    #[test]
    fn case_18_only_the_two_kinds_of_success_count_and_only_a_miss_breaks() {
        let states = [
            DayState::Done {
                amount: 1,
                target: 1,
            },
            DayState::Missed {
                amount: 0,
                target: 1,
            },
            DayState::NotScheduled { amount: 0 },
            DayState::Extra {
                amount: 1,
                target: 1,
            },
            DayState::NoData,
        ];
        let expected = [
            (true, false),
            (false, true),
            (false, false),
            (true, false),
            (false, false),
        ];

        for (state, (counts, breaks)) in states.into_iter().zip(expected) {
            assert_eq!(state.counts(), counts, "{state:?} counts the wrong way");
            assert_eq!(state.breaks(), breaks, "{state:?} breaks the wrong way");
        }
    }
}
