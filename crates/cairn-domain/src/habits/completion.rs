//! How much of the month the habit was actually met.
//!
//! A percentage looks like the simplest number on the screen and is the one with the most ways
//! of being dishonest. Counting the whole month makes a habit look like a failure on the second
//! of the month, because the twenty-nine days nobody has lived yet are counted as missed.
//! Counting only the days that carry a mark makes every habit sit at a hundred per cent
//! forever. Counting today as a miss punishes a person for opening the application before
//! breakfast.
//!
//! So the denominator is neither the month nor the marks: it is the days the habit asked for
//! and that are already over. Today joins them only once it has been met, which is the same
//! rule [`crate::habits::streak`] uses for a day that is still being lived — a day in progress
//! is not a day that failed, and it is not a day that succeeded either, so it stays out of the
//! division until it is one of the two.
//!
//! The answer leaves here as the pair of numbers it was counted from rather than as a
//! percentage, because "three of four" is what the screen shows and a number that arrives
//! already divided cannot say it. There is no floating point anywhere in this module, on the
//! way in or on the way out: a proportion of whole days is two whole numbers, and turning it
//! into a fraction of one would only add a rounding nobody asked for.
//!
//! Nothing here reads a clock. `today` is a parameter, for the reason it is a parameter
//! everywhere else in this module: which day today is depends on a time zone and on the hour
//! the person considers a day to start at, and neither is a question this crate may ask.

use crate::habits::calendar::{month_length, weekday};
use crate::habits::day::DayState;
use crate::habits::spec::HabitSpec;
use crate::time::CivilDay;

/// A proportion, kept as the two numbers it came from.
///
/// Never a float. A percentage that arrives already divided cannot say "three of four", and
/// "three of four" is what the screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio {
    /// How many days were met.
    pub done: u32,
    /// How many days were asked for and are already over.
    ///
    /// Not an upper bound on `done`: a day outside the schedule that was done anyway is a day
    /// nobody asked for, so it adds to the numerator and to nothing else. A month can read
    /// above a hundred per cent, and that is the honest reading of having done more than was
    /// asked.
    pub of: u32,
}

impl Ratio {
    /// Rounded to the nearest whole percent. Zero when there is nothing to divide.
    ///
    /// Half a point rounds up, so a habit met one day out of eight reads thirteen rather than
    /// twelve. The rounding is done by adding half the denominator before dividing, which is
    /// the same decision in whole numbers and never leaves the width of a `u32`: the largest
    /// numerator a month can produce is thirty-one.
    #[must_use]
    #[expect(
        clippy::integer_division,
        reason = "the lint is here to catch arithmetic that silently drops a remainder; this division is the rounding itself, and the remainder it drops is the half a point the addition above has already accounted for"
    )]
    pub const fn percent(self) -> u32 {
        if self.of == 0 {
            return 0;
        }

        // Saturating rather than checked because neither bound is reachable from a month:
        // thirty-one times a hundred is four orders of magnitude short of the top of a `u32`.
        // Saturating says what should happen anyway if a caller ever handed this a ratio that
        // did not come from one — a pinned maximum, not a wrap to nearly nothing.
        self.done.saturating_mul(100).saturating_add(self.of / 2) / self.of
    }
}

/// How much of the month has been met.
///
/// The denominator is the active days of the month that have **already finished**. Today counts
/// in it only when it is met, or when it is not today any more.
///
/// The month is walked day by day rather than read off `days`, so a day the caller has no row
/// for is still a day the habit asked for: a month nobody opened the application in reads zero
/// per cent, not a hundred. `days` supplies the verdicts, and a day missing from it is a day
/// that was not met.
///
/// `year` and `month` that are not a month on the calendar answer with an empty ratio, which is
/// the same answer an empty month gives. There is no month to be wrong about.
#[must_use]
pub fn month(
    spec: &HabitSpec,
    days: &[(CivilDay, DayState)],
    year: u16,
    month: u8,
    today: CivilDay,
) -> Ratio {
    let mut done: u32 = 0;
    let mut of: u32 = 0;

    for number in 1..=month_length(year, month) {
        // Unreachable: `month_length` answered with a length, so the year and the month are
        // both ones this application can name and every number up to that length is a day it
        // can name too. Skipping rather than stopping keeps a disagreement between the two
        // calendars in this crate from turning a percentage into a dead process.
        let Ok(day) = CivilDay::new(year, month, number) else {
            continue;
        };

        // Before the habit existed, or not lived yet. Neither is a day anybody can be judged
        // on, in either direction.
        if day < spec.started_on || day > today {
            continue;
        }

        let met = is_met_on(days, day);
        if met {
            done = done.saturating_add(1);
        }

        // Asked for, and over. Today is over for this purpose only once it has been met:
        // until then it is a day still being lived, and putting it in the denominator would
        // mean the percentage falls every midnight and climbs back during the day.
        if spec.schedule.includes(weekday(day)) && (day < today || met) {
            of = of.saturating_add(1);
        }
    }

    Ratio { done, of }
}

/// Whether the slice says that day was met.
///
/// A linear scan, deliberately: the slice this is asked about is a month or a year of squares,
/// the walk above asks at most thirty-one times, and the alternative is an index that assumes
/// the caller sorted the slice and holds exactly one entry per day. False for a day the slice
/// does not mention, which is the honest reading of an absent row.
fn is_met_on(days: &[(CivilDay, DayState)], day: CivilDay) -> bool {
    days.iter()
        .any(|&(marked, state)| marked == day && state.counts())
}

#[cfg(test)]
mod tests {
    use super::{Ratio, month};
    use crate::habits::calendar::month_length;
    use crate::habits::day::{DayState, Entry, classify};
    use crate::habits::spec::{HabitRow, HabitSpec};
    use crate::time::CivilDay;

    /// Monday, Wednesday and Friday, which is the schedule with the most interesting month.
    const MON_WED_FRI: i64 = 0b001_0101;

    /// Mondays only, so that a day outside the schedule is one day away from a day inside it.
    const MONDAYS_ONLY: i64 = 0b000_0001;

    fn day(year: u16, month: u8, number: u8) -> CivilDay {
        CivilDay::new(year, month, number).expect("the test named a day that exists")
    }

    /// Daily, done or not done, more is better, every day of the week, started long ago.
    fn row() -> HabitRow {
        HabitRow {
            period: 0,
            kind: 0,
            direction: 0,
            aggregation: 0,
            schedule_mask: 0,
            unit: None,
            target_per_period: None,
            started_on: day(2026, 1, 1),
        }
    }

    fn spec(row: HabitRow) -> HabitSpec {
        HabitSpec::from_row(row).expect("the test named a row this product accepts")
    }

    /// The month as the caller will hand it over: one square per day, classified, with a mark
    /// on the days the test names by their number.
    fn squares(
        spec: &HabitSpec,
        year: u16,
        number: u8,
        marked: &[u8],
        today: CivilDay,
    ) -> Vec<(CivilDay, DayState)> {
        (1..=month_length(year, number))
            .filter_map(|of_the_month| CivilDay::new(year, number, of_the_month).ok())
            .map(|square| {
                let entry = marked.contains(&square.day()).then_some(Entry {
                    day: square,
                    amount: 1,
                    target_snapshot: None,
                });

                (square, classify(spec, square, entry, today))
            })
            .collect()
    }

    #[test]
    fn case_01_the_first_of_the_month_is_today_and_unmarked_so_there_is_nothing_to_divide() {
        let habit = spec(row());
        let today = day(2026, 2, 1);

        let ratio = month(
            &habit,
            &squares(&habit, 2026, 2, &[], today),
            2026,
            2,
            today,
        );

        assert_eq!(ratio, Ratio { done: 0, of: 0 });
        assert_eq!(ratio.percent(), 0);
    }

    #[test]
    fn case_02_the_first_of_the_month_is_today_and_marked_so_it_is_one_of_one() {
        let habit = spec(row());
        let today = day(2026, 2, 1);

        let ratio = month(
            &habit,
            &squares(&habit, 2026, 2, &[1], today),
            2026,
            2,
            today,
        );

        assert_eq!(ratio, Ratio { done: 1, of: 1 });
        assert_eq!(ratio.percent(), 100);
    }

    #[test]
    fn case_03_an_unmarked_today_stays_out_of_the_denominator() {
        let habit = spec(row());
        let today = day(2026, 2, 2);

        let ratio = month(
            &habit,
            &squares(&habit, 2026, 2, &[1], today),
            2026,
            2,
            today,
        );

        assert_eq!(
            ratio,
            Ratio { done: 1, of: 1 },
            "today is unfinished, so it belongs to neither number"
        );
        assert_eq!(ratio.percent(), 100);
    }

    #[test]
    fn case_04_a_month_that_is_over_divides_by_all_of_its_days() {
        let habit = spec(row());
        let today = day(2026, 5, 1);
        let met: Vec<u8> = (1..=27).collect();

        let ratio = month(
            &habit,
            &squares(&habit, 2026, 4, &met, today),
            2026,
            4,
            today,
        );

        assert_eq!(ratio, Ratio { done: 27, of: 30 });
        assert_eq!(ratio.percent(), 90);
    }

    #[test]
    fn case_05_the_days_before_the_habit_started_are_not_the_habits_business() {
        let habit = spec(HabitRow {
            started_on: day(2026, 4, 15),
            ..row()
        });
        let today = day(2026, 5, 1);
        let every_day: Vec<u8> = (1..=30).collect();

        let ratio = month(
            &habit,
            &squares(&habit, 2026, 4, &every_day, today),
            2026,
            4,
            today,
        );

        assert_eq!(
            ratio,
            Ratio { done: 16, of: 16 },
            "the fourteen days before it started were counted, marked or not"
        );
        assert_eq!(ratio.percent(), 100);
    }

    #[test]
    fn case_06_a_schedule_of_three_days_a_week_divides_by_those_days_alone() {
        let habit = spec(HabitRow {
            schedule_mask: MON_WED_FRI,
            started_on: day(2025, 1, 1),
            ..row()
        });
        // A Thursday, which is not one of the habit's days. The Mondays, Wednesdays and Fridays
        // already over are the third, fifth, seventh, tenth, twelfth, fourteenth, seventeenth
        // and nineteenth; the twenty-first onwards have not happened.
        let today = day(2025, 2, 20);
        let met = [3, 5, 7, 10, 12, 14];

        let ratio = month(
            &habit,
            &squares(&habit, 2025, 2, &met, today),
            2025,
            2,
            today,
        );

        assert_eq!(ratio, Ratio { done: 6, of: 8 });
        assert_eq!(ratio.percent(), 75);
    }

    #[test]
    fn case_07_a_month_with_no_active_day_is_not_a_division_by_zero() {
        let habit = spec(HabitRow {
            started_on: day(2026, 3, 1),
            ..row()
        });
        let today = day(2026, 4, 1);

        let ratio = month(
            &habit,
            &squares(&habit, 2026, 2, &[], today),
            2026,
            2,
            today,
        );

        assert_eq!(ratio, Ratio { done: 0, of: 0 });
        assert_eq!(ratio.percent(), 0);
    }

    #[test]
    fn case_08_a_percentage_rounds_to_the_nearest_whole_point_and_a_half_goes_up() {
        assert_eq!(Ratio { done: 1, of: 3 }.percent(), 33);
        assert_eq!(Ratio { done: 2, of: 3 }.percent(), 67);
        assert_eq!(Ratio { done: 1, of: 8 }.percent(), 13);
        assert_eq!(Ratio { done: 3, of: 8 }.percent(), 38);
        assert_eq!(Ratio { done: 0, of: 0 }.percent(), 0);
    }

    #[test]
    fn case_09_a_day_done_outside_the_schedule_adds_to_what_was_done_and_to_nothing_else() {
        let habit = spec(HabitRow {
            schedule_mask: MONDAYS_ONLY,
            ..row()
        });
        let today = day(2026, 5, 1);
        // The sixth and the thirteenth are Mondays; the seventh is the Tuesday after the first
        // of them, which the habit never asked about.
        let met = [6, 7, 13];

        let ratio = month(
            &habit,
            &squares(&habit, 2026, 4, &met, today),
            2026,
            4,
            today,
        );

        assert_eq!(
            ratio,
            Ratio { done: 3, of: 4 },
            "the extra Tuesday was counted as a day the habit asked for"
        );
        assert_eq!(ratio.percent(), 75);
    }

    #[test]
    fn case_10_a_month_nobody_has_lived_yet_is_empty_rather_than_missed() {
        let habit = spec(row());
        let today = day(2026, 5, 10);

        let ratio = month(
            &habit,
            &squares(&habit, 2026, 6, &[], today),
            2026,
            6,
            today,
        );

        assert_eq!(ratio, Ratio { done: 0, of: 0 });
        assert_eq!(ratio.percent(), 0);
    }

    #[test]
    fn a_month_number_that_is_not_a_month_answers_empty_instead_of_panicking() {
        let habit = spec(row());
        let today = day(2026, 5, 10);

        for not_a_month in [0, 13, u8::MAX] {
            assert_eq!(
                month(&habit, &[], 2026, not_a_month, today),
                Ratio { done: 0, of: 0 },
                "month {not_a_month} produced days"
            );
        }
    }

    #[test]
    fn a_day_the_slice_never_mentions_is_a_day_that_was_not_met() {
        let habit = spec(row());
        let today = day(2026, 3, 1);

        // February 2026 in full, and not one square handed over: a month the person never
        // opened the application in reads zero, not a hundred.
        let ratio = month(&habit, &[], 2026, 2, today);

        assert_eq!(ratio, Ratio { done: 0, of: 28 });
        assert_eq!(ratio.percent(), 0);
    }
}
