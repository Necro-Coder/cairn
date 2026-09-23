//! Calendar arithmetic: everything a streak needs to ask about a run of days.
//!
//! A streak is a claim about consecutive days, and "consecutive" is a calendar word, not a
//! clock word. Counting one takes four questions — what day of the week is this, which week
//! does it belong to, how far apart are these two, and what is the day `n` days from here —
//! and every one of them has an edge a subtraction gets wrong: leap years, the ISO week at the
//! turn of the year, and the two ends of the range this application can name.
//!
//! So the answers come from `jiff`, and they come through [`CivilDay`] at both ends. The
//! calendar is the only thing borrowed: there is no time zone here, no reading of a clock and
//! no look at the environment. Where the person is standing is a question for the crate that
//! is allowed to ask the operating system, and the answer arrives here already turned into a
//! day.
//!
//! Anything that can fall off either end of the range says so in its return type. Nothing in
//! this module wraps around, and nothing panics.

use jiff::civil::{Date, ISOWeekDate};
use jiff::{Span, Unit};

use crate::time::{CivilDay, TimeError};

/// Which day of the week a civil day falls on. Monday first, because the schedule mask is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Weekday {
    /// Monday, the first day of the ISO week.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday, whose year decides which ISO year the whole week belongs to.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
    /// Sunday, the last day of the ISO week.
    Sunday,
}

impl Weekday {
    /// The bit this day occupies in a schedule mask. Monday is the lowest.
    ///
    /// Written out rather than shifted, so the mask a habit is stored with can be read
    /// straight off this function without working out which end the count started from.
    #[must_use]
    pub const fn bit(self) -> u8 {
        match self {
            Self::Monday => 0b000_0001,
            Self::Tuesday => 0b000_0010,
            Self::Wednesday => 0b000_0100,
            Self::Thursday => 0b000_1000,
            Self::Friday => 0b001_0000,
            Self::Saturday => 0b010_0000,
            Self::Sunday => 0b100_0000,
        }
    }
}

/// The ISO-8601 week a day belongs to, which is not always a week of its own year.
///
/// The last days of December and the first of January regularly fall in a week that belongs to
/// the other year, which is why the year is carried here rather than taken from the day. A
/// weekly habit that used the civil year would see two part-weeks every New Year and break a
/// streak that never broke.
///
/// The fields are in the order they are compared in, so the derived ordering is the order the
/// weeks happen in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IsoWeek {
    /// The ISO year, which is the civil year of the Thursday of this week.
    pub year: i16,
    /// The week number, one to fifty-three.
    pub week: u8,
}

/// The first of January of year zero, which is an answer no caller can ever get.
///
/// It stands in for the branch of [`to_jiff`] that the invariant of [`CivilDay`] makes
/// unreachable. Year zero is deliberately outside the range [`CivilDay`] can name, so a value
/// that somehow came from that branch cannot be turned back into a day: [`from_jiff`] refuses
/// it instead of handing back something that looks real.
const UNREACHABLE_DAY: Date = Date::ZERO;

/// The same day, as `jiff` names it.
///
/// Total, and it is the invariant of [`CivilDay`] that makes it total: every value of that type
/// is a day that exists, in a year between one and 9999, and `jiff` names every one of them.
/// There is no input to this function that has no answer, which is why it does not return a
/// `Result` that every caller would have to pretend to handle.
///
/// The branch that cannot be taken answers with [`UNREACHABLE_DAY`] rather than stopping the
/// process. A panic would turn a broken invariant in a type this crate owns into a dead
/// application; the property test at the bottom of this file walks the range instead, and
/// would fail the moment any day of it stopped converting.
fn to_jiff(day: CivilDay) -> Date {
    let (Ok(year), Ok(month), Ok(number)) = (
        i16::try_from(day.year()),
        i8::try_from(day.month()),
        i8::try_from(day.day()),
    ) else {
        return UNREACHABLE_DAY;
    };

    Date::new(year, month, number).unwrap_or(UNREACHABLE_DAY)
}

/// The same day, back in the type this application stores.
///
/// Fallible in the direction that matters: `jiff` names years this application does not, all
/// the way back past year zero, so a date that arrived by arithmetic has to be checked before
/// it can be a [`CivilDay`] again.
///
/// One error for both ends, and it is the same one [`CivilDay::previous`] gives when it runs
/// out of calendar. Every date `jiff` hands over is a day that exists, so the only thing the
/// constructor can object to is the year, and a caller that walked off the bottom of the range
/// wants the same answer as one that walked off the top rather than a different variant to
/// match on.
fn from_jiff(date: Date) -> Result<CivilDay, TimeError> {
    let (Ok(year), Ok(month), Ok(day)) = (
        u16::try_from(date.year()),
        u8::try_from(date.month()),
        u8::try_from(date.day()),
    ) else {
        return Err(TimeError::OutOfRange);
    };

    CivilDay::new(year, month, day).map_err(|_outside_the_range| TimeError::OutOfRange)
}

/// Which day of the week this is.
#[must_use]
pub fn weekday(day: CivilDay) -> Weekday {
    match to_jiff(day).weekday() {
        jiff::civil::Weekday::Monday => Weekday::Monday,
        jiff::civil::Weekday::Tuesday => Weekday::Tuesday,
        jiff::civil::Weekday::Wednesday => Weekday::Wednesday,
        jiff::civil::Weekday::Thursday => Weekday::Thursday,
        jiff::civil::Weekday::Friday => Weekday::Friday,
        jiff::civil::Weekday::Saturday => Weekday::Saturday,
        jiff::civil::Weekday::Sunday => Weekday::Sunday,
    }
}

/// The ISO-8601 week the day belongs to.
#[must_use]
pub fn iso_week(day: CivilDay) -> IsoWeek {
    let week_date = to_jiff(day).iso_week_date();

    IsoWeek {
        year: week_date.year(),
        // One to fifty-three by construction, so the narrowing always has an answer. Zero is
        // not a week number any calendar produces, which is what makes it the right value for
        // a branch that cannot be reached: visibly wrong rather than plausible.
        week: u8::try_from(week_date.week()).unwrap_or(0),
    }
}

/// The Monday of the day's ISO week.
///
/// # Errors
/// [`TimeError`] if the Monday falls outside the range `CivilDay` can name.
pub fn week_start(day: CivilDay) -> Result<CivilDay, TimeError> {
    let week_date = to_jiff(day).iso_week_date();
    let monday = ISOWeekDate::new(
        week_date.year(),
        week_date.week(),
        jiff::civil::Weekday::Monday,
    )
    .map_err(|_before_the_calendar_starts| TimeError::OutOfRange)?;

    from_jiff(monday.date())
}

/// How many days from `from` to `to`, negative when `to` is earlier.
#[must_use]
pub fn days_between(from: CivilDay, to: CivilDay) -> i32 {
    // The widest gap this can be asked for is the one between the two ends of the range, three
    // and a half million days, so the count is a whole number of days and it fits. `jiff`
    // states that a span can hold the difference between any two dates it names, so the
    // refusal is unreachable; it answers zero rather than panicking.
    to_jiff(from)
        .until((Unit::Day, to_jiff(to)))
        .map_or(0, |difference| difference.get_days())
}

/// The day `count` days after this one, negative counts going backwards.
///
/// # Errors
/// [`TimeError`] if the result falls outside the range `CivilDay` can name.
pub fn shift(day: CivilDay, count: i32) -> Result<CivilDay, TimeError> {
    let distance = Span::new()
        .try_days(count)
        .map_err(|_more_days_than_a_span_holds| TimeError::OutOfRange)?;
    let moved = to_jiff(day)
        .checked_add(distance)
        .map_err(|_off_the_end_of_the_calendar| TimeError::OutOfRange)?;

    from_jiff(moved)
}

/// Every day from `from` to `to`, both ends included, oldest first.
///
/// Empty when `to` is before `from`. Never allocates more than `days_between` plus one.
#[must_use]
pub fn span(from: CivilDay, to: CivilDay) -> Vec<CivilDay> {
    let distance = days_between(from, to);
    if distance.is_negative() {
        return Vec::new();
    }

    // Not negative, so the conversion always has an answer; and a guess of zero would only
    // cost a reallocation, never a wrong result.
    let length = usize::try_from(distance).unwrap_or(0).saturating_add(1);
    let mut days = Vec::with_capacity(length);
    let mut current = from;
    days.push(current);

    while current < to {
        // Unreachable: the only day with no successor is the last one this application can
        // name, and nothing is later than it, so the condition above is already false there.
        let Ok(next) = current.next() else { break };
        current = next;
        days.push(current);
    }

    days
}

/// How many days that month has, leap years included.
///
/// Answers zero for a year this application cannot name, or for a number that is not a month.
/// Neither has a length, and zero is a length no month has, so a caller that passes thirteen
/// finds out rather than being handed a plausible twenty-eight.
///
/// The answer is read out of [`CivilDay::new`] rather than from a second copy of the leap year
/// rule: asking for the thirty-first and looking at what the refusal says the month is worth
/// keeps one calendar in this crate instead of two that can drift apart.
#[must_use]
pub const fn month_length(year: u16, month: u8) -> u8 {
    match CivilDay::new(year, month, 31) {
        Ok(_a_month_with_thirty_one_days) => 31,
        Err(TimeError::Day { length, .. }) => length,
        Err(_not_a_year_or_not_a_month) => 0,
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        IsoWeek, Weekday, days_between, from_jiff, iso_week, month_length, shift, span, to_jiff,
        week_start, weekday,
    };
    use crate::time::{CivilDay, MAX_YEAR, MIN_YEAR, TimeError};

    /// A day, written the way the tests below read.
    fn day(year: u16, month: u8, number: u8) -> CivilDay {
        CivilDay::new(year, month, number).expect("the test named a day that exists")
    }

    /// Every day this application can name, drawn at random.
    fn any_day() -> impl Strategy<Value = CivilDay> {
        (MIN_YEAR..=MAX_YEAR, 1_u8..=12, 1_u8..=31).prop_filter_map(
            "the three parts are not a day that exists",
            |(year, month, number)| CivilDay::new(year, month, number).ok(),
        )
    }

    #[test]
    fn the_day_of_the_week_is_the_one_the_calendar_says() {
        assert_eq!(weekday(day(2026, 9, 14)), Weekday::Monday);
        assert_eq!(weekday(day(2026, 9, 20)), Weekday::Sunday);
        assert_eq!(weekday(day(2026, 9, 21)), Weekday::Monday);
    }

    #[test]
    fn the_seven_bits_of_a_schedule_mask_are_one_per_day_and_monday_is_the_lowest() {
        let whole_week = [
            Weekday::Monday,
            Weekday::Tuesday,
            Weekday::Wednesday,
            Weekday::Thursday,
            Weekday::Friday,
            Weekday::Saturday,
            Weekday::Sunday,
        ];

        let mask = whole_week.iter().fold(0_u8, |mask, day| mask | day.bit());

        assert_eq!(mask, 0b111_1111, "the seven bits do not cover the week");
        assert_eq!(Weekday::Monday.bit(), 0b000_0001);
        assert_eq!(Weekday::Sunday.bit(), 0b100_0000);
    }

    #[test]
    fn the_first_of_january_can_belong_to_the_week_of_the_year_before() {
        // The whole reason the ISO year is carried rather than taken from the day. A weekly
        // habit reading the civil year here would see a two-day week and break a live streak.
        assert_eq!(
            iso_week(day(2027, 1, 1)),
            IsoWeek {
                year: 2026,
                week: 53,
            }
        );
    }

    #[test]
    fn the_last_day_of_a_long_year_is_in_its_fifty_third_week() {
        assert_eq!(
            iso_week(day(2026, 12, 31)),
            IsoWeek {
                year: 2026,
                week: 53,
            }
        );
    }

    #[test]
    fn every_day_of_a_week_starts_at_the_same_monday_and_a_monday_starts_at_itself() {
        let monday = day(2026, 9, 14);

        for offset in 0..7 {
            let inside_the_week = shift(monday, offset).expect("still inside the range");

            assert_eq!(
                week_start(inside_the_week),
                Ok(monday),
                "{inside_the_week} did not start its week at {monday}"
            );
        }

        assert_eq!(
            week_start(monday),
            Ok(monday),
            "asking twice moved the start of the week"
        );
    }

    #[test]
    fn the_distance_between_two_days_is_the_same_number_either_way_round() {
        let earlier = day(2026, 1, 5);
        let later = day(2026, 3, 17);

        assert_eq!(days_between(earlier, later), 71);
        assert_eq!(days_between(later, earlier), -71);
        assert_eq!(days_between(earlier, earlier), 0);
    }

    #[test]
    fn a_distance_that_crosses_a_leap_day_counts_it() {
        assert_eq!(days_between(day(2024, 2, 28), day(2024, 3, 1)), 2);
        assert_eq!(days_between(day(2025, 2, 28), day(2025, 3, 1)), 1);
    }

    #[test]
    fn moving_a_day_lands_where_the_calendar_says_including_across_a_leap_year() {
        let new_year = day(2024, 1, 1);

        assert_eq!(shift(new_year, 0), Ok(new_year));
        assert_eq!(shift(new_year, 1), Ok(day(2024, 1, 2)));
        assert_eq!(shift(new_year, -1), Ok(day(2023, 12, 31)));
        // 2024 has three hundred and sixty-six days, so a year of moving lands a day short.
        assert_eq!(shift(new_year, 365), Ok(day(2024, 12, 31)));
        assert_eq!(shift(new_year, 366), Ok(day(2025, 1, 1)));
    }

    #[test]
    fn moving_past_either_end_of_the_range_refuses_instead_of_wrapping() {
        let last = day(MAX_YEAR, 12, 31);
        let first = day(MIN_YEAR, 1, 1);

        assert_eq!(shift(last, 1), Err(TimeError::OutOfRange));
        assert_eq!(shift(first, -1), Err(TimeError::OutOfRange));
        assert_eq!(shift(first, i32::MIN), Err(TimeError::OutOfRange));
        assert_eq!(shift(last, i32::MAX), Err(TimeError::OutOfRange));
    }

    #[test]
    fn a_run_of_one_day_is_that_day() {
        let only = day(2026, 9, 18);

        assert_eq!(span(only, only), vec![only]);
    }

    #[test]
    fn a_run_that_ends_before_it_starts_is_empty() {
        assert!(span(day(2026, 9, 18), day(2026, 9, 17)).is_empty());
    }

    #[test]
    fn a_run_of_four_hundred_days_is_four_hundred_days_in_order_and_each_one_once() {
        let first = day(2024, 1, 1);
        let last = shift(first, 399).expect("still inside the range");

        let run = span(first, last);

        assert_eq!(run.len(), 400);
        assert_eq!(run.first(), Some(&first));
        assert_eq!(run.last(), Some(&last));
        assert!(
            run.windows(2).all(|pair| match pair {
                [earlier, later] => later > earlier,
                _ => false,
            }),
            "the run is not strictly increasing, so it repeats a day or goes backwards"
        );
    }

    #[test]
    fn february_is_as_long_as_the_year_it_is_in() {
        assert_eq!(month_length(2024, 2), 29);
        assert_eq!(month_length(2025, 2), 28);
        assert_eq!(month_length(1900, 2), 28);
        assert_eq!(month_length(2000, 2), 29);
    }

    #[test]
    fn a_month_that_is_not_a_month_has_no_length() {
        assert_eq!(month_length(2026, 0), 0);
        assert_eq!(month_length(2026, 13), 0);
        assert_eq!(month_length(0, 1), 0);
        assert_eq!(month_length(10_000, 1), 0);
    }

    proptest! {
        /// The branch of `to_jiff` that everything above takes for granted stays unreachable.
        ///
        /// The rest of this module is built on the conversion being exact in both directions,
        /// and the failure it guards against is silent: a day that converted to the wrong date
        /// would still produce a weekday, a week and a distance, all of them wrong.
        #[test]
        fn a_day_survives_the_trip_through_the_calendar_library(day in any_day()) {
            prop_assert_eq!(from_jiff(to_jiff(day)), Ok(day));
        }

        /// Moving a day and moving it back returns the day it started as.
        #[test]
        fn moving_a_day_and_back_returns_it(day in any_day(), count in -400_000_i32..=400_000) {
            let Ok(moved) = shift(day, count) else {
                return Ok(());
            };

            prop_assert_eq!(shift(moved, -count), Ok(day));
        }

        /// The distance to a day reached by moving is the number of days it was moved.
        #[test]
        fn the_distance_to_a_moved_day_is_how_far_it_moved(
            day in any_day(),
            count in -400_000_i32..=400_000,
        ) {
            let Ok(moved) = shift(day, count) else {
                return Ok(());
            };

            prop_assert_eq!(days_between(day, moved), count);
        }

        /// A week starts on a Monday, within the seven days before the day it was asked about,
        /// and asking again from that Monday changes nothing.
        #[test]
        fn a_week_starts_on_a_monday_and_stays_there(day in any_day()) {
            let Ok(monday) = week_start(day) else {
                return Ok(());
            };

            prop_assert_eq!(weekday(monday), Weekday::Monday);
            prop_assert_eq!(week_start(monday), Ok(monday));
            prop_assert!(monday <= day);
            prop_assert!(days_between(monday, day) < 7);
        }

        /// A run holds one day for every day of the distance it covers, plus the one it starts
        /// on, and it ends on the day it was asked to end on.
        #[test]
        fn a_run_holds_the_days_between_its_ends_and_both_ends(
            day in any_day(),
            length in 0_i32..=500,
        ) {
            let Ok(last) = shift(day, length) else {
                return Ok(());
            };

            let run = span(day, last);

            prop_assert_eq!(run.len(), usize::try_from(length).unwrap_or(0) + 1);
            prop_assert_eq!(run.first(), Some(&day));
            prop_assert_eq!(run.last(), Some(&last));
        }
    }
}
