//! The two kinds of time this application has, and the reason they are two.
//!
//! A moment is not a day. The instant a transaction was recorded is a point on a line that every
//! machine in the world agrees about; the day a habit was completed is a square on a calendar that
//! only means anything in the place the person was standing. Applications that keep both in one
//! type are the ones where a habit marked at half past eleven at night shows up on tomorrow, or
//! where a streak breaks when somebody flies east.
//!
//! So there are two types, they do not convert into each other, and there is no arithmetic that
//! mixes them. Turning a [`Timestamp`] into a [`CivilDay`] needs a place, and a place is a
//! decision this crate does not have and will not guess: it arrives with the calendar work later.
//! Until then the impossibility is the feature.
//!
//! Both are stored as integers. An integer sorts, indexes and compares exactly as ISO text does
//! while taking a third of the room, and it cannot be half-written as `2026-9-1`.

use std::fmt;

/// The earliest year a civil day may name.
///
/// Not zero. A day in year zero is a typo, a sentinel somebody used instead of an option, or a
/// date arriving from a system with a different epoch, and none of those is a day a person marked
/// a habit on.
pub const MIN_YEAR: u16 = 1;

/// The latest year a civil day may name.
///
/// Four digits, so that the whole day fits in eight decimal digits and therefore in a `u32`. The
/// bound is also what makes the layout a fact rather than a convention.
pub const MAX_YEAR: u16 = 9999;

/// What a time value can be wrong about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TimeError {
    /// The year is outside [`MIN_YEAR`]..=[`MAX_YEAR`].
    #[error("the year {year} is outside {MIN_YEAR} to {MAX_YEAR}")]
    Year {
        /// What was given.
        year: u16,
    },

    /// The month is not one to twelve.
    #[error("the month {month} is not between 1 and 12")]
    Month {
        /// What was given.
        month: u8,
    },

    /// The day is not a day that month has.
    ///
    /// Carries the length of the month, because "31 is not a day of February" is a message
    /// somebody can act on and "invalid date" is not.
    #[error("the day {day} is not between 1 and {length} for that month")]
    Day {
        /// What was given.
        day: u8,
        /// How many days that month actually has, leap years included.
        length: u8,
    },

    /// The stored number is not eight digits in the shape `YYYYMMDD`.
    #[error("{value} is not a day in the form YYYYMMDD")]
    Shape {
        /// What was read back.
        value: u32,
    },

    /// The day is the last one this type can name, and something asked for the next.
    ///
    /// A refusal rather than a wrap, because a calendar that goes from the end of year 9999 back
    /// to the start of year 1 would make a streak calculation produce a number instead of a
    /// failure, and the number would be believed.
    #[error("there is no day after the last one this application can name")]
    OutOfRange,
}

/// A moment, in microseconds since the Unix epoch, UTC.
///
/// Microseconds because a millisecond is not enough to order two writes made by one person in one
/// gesture, and a nanosecond is more resolution than any clock this runs on actually has. Signed,
/// because the type SQLite stores is signed and a moment that comes back negative has to be
/// representable in order to be refused rather than silently reinterpreted as an enormous one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    /// The epoch itself.
    pub const EPOCH: Self = Self(0);

    /// A moment from a count of microseconds since the epoch.
    #[must_use]
    pub const fn from_micros(micros: i64) -> Self {
        Self(micros)
    }

    /// The count of microseconds since the epoch.
    #[must_use]
    pub const fn as_micros(self) -> i64 {
        self.0
    }

    /// The same moment rounded down to a whole millisecond.
    ///
    /// What the logical clock reads. Rounded down rather than to the nearest, so that the
    /// millisecond a moment belongs to never moves forward past a moment that has not happened.
    #[must_use]
    pub const fn as_millis(self) -> i64 {
        self.0.div_euclid(1_000)
    }

    /// This moment moved by a number of microseconds, saturating at the ends of the range.
    ///
    /// Saturating rather than wrapping. A moment that wrapped would be sixty thousand years in
    /// the past and would sort before everything, which is the shape of a bug that looks like
    /// data loss.
    #[must_use]
    pub const fn saturating_add_micros(self, micros: i64) -> Self {
        Self(self.0.saturating_add(micros))
    }

    /// How many microseconds separate two moments, saturating at the ends of the range.
    #[must_use]
    pub const fn micros_since(self, earlier: Self) -> i64 {
        self.0.saturating_sub(earlier.0)
    }
}

impl fmt::Display for Timestamp {
    /// Prints the count of microseconds, not a date.
    ///
    /// Formatting a moment as a date needs a place, and this type does not have one. A display
    /// that quietly chose UTC would be read as local time by whoever saw it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}us", self.0)
    }
}

/// A square on a calendar, stored as the number `YYYYMMDD`.
///
/// Every value of this type is a day that exists. There is no constructor that takes a number
/// without checking it, so the thirty-first of February cannot be built, stored or compared, and
/// a streak that counts days does not have to ask whether each one is real.
///
/// The three parts are kept as three fields, in the order they are compared in, rather than as
/// the packed number. Deriving the ordering then gives calendar order for free, and the only
/// place a number has to be split back into parts is the one function that reads a stored value,
/// where a part that does not fit is a row this application did not write and is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDay {
    year: u16,
    month: u8,
    day: u8,
}

impl CivilDay {
    /// The first of January 1970, which is the day the epoch falls on.
    ///
    /// A named value rather than a call that has to be checked. It exists so that code which
    /// needs a day it can always have does not reach for a constructor that returns a `Result`
    /// and then decide what to do when the answer it already knows is `Ok`.
    pub const UNIX_EPOCH: Self = Self {
        year: 1970,
        month: 1,
        day: 1,
    };

    /// A day from its three parts.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::Year`], [`TimeError::Month`] or [`TimeError::Day`] naming the part
    /// that is wrong, and for the day the length the month actually has.
    pub const fn new(year: u16, month: u8, day: u8) -> Result<Self, TimeError> {
        if year < MIN_YEAR || year > MAX_YEAR {
            return Err(TimeError::Year { year });
        }
        if month == 0 || month > 12 {
            return Err(TimeError::Month { month });
        }

        let length = days_in_month(year, month);
        if day == 0 || day > length {
            return Err(TimeError::Day { day, length });
        }

        Ok(Self { year, month, day })
    }

    /// A day read back from the number it is stored as.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::Shape`] if the number is not eight digits, and whatever [`Self::new`]
    /// returns if the three parts it splits into are not a day that exists. A row holding a
    /// number that is not a day was not written by this application.
    #[allow(
        clippy::integer_division,
        reason = "the lint is here to catch arithmetic that silently drops a remainder; these three divisions are digit extraction, and the remainder each one drops is the part the next line reads"
    )]
    pub fn from_number(value: u32) -> Result<Self, TimeError> {
        let shape = || TimeError::Shape { value };

        let year = u16::try_from(value / 10_000).map_err(|_too_large| shape())?;
        let month = u8::try_from(value / 100 % 100).map_err(|_too_large| shape())?;
        let day = u8::try_from(value % 100).map_err(|_too_large| shape())?;

        Self::new(year, month, day)
    }

    /// The number this day is stored as.
    ///
    /// Every part widens rather than narrows, so the arithmetic cannot lose anything: the
    /// largest day this type can hold is 99 991 231, which is a third of the way up a `u32`.
    #[must_use]
    pub fn as_number(self) -> u32 {
        u32::from(self.year) * 10_000 + u32::from(self.month) * 100 + u32::from(self.day)
    }

    /// The year.
    #[must_use]
    pub const fn year(self) -> u16 {
        self.year
    }

    /// The month, one to twelve.
    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }

    /// The day of the month, one to the length of that month.
    #[must_use]
    pub const fn day(self) -> u8 {
        self.day
    }

    /// The day after this one.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::OutOfRange`] on the last day this type can name.
    pub const fn next(self) -> Result<Self, TimeError> {
        let (year, month, day) = (self.year(), self.month(), self.day());

        if day < days_in_month(year, month) {
            return Self::new(year, month, day + 1);
        }
        if month < 12 {
            return Self::new(year, month + 1, 1);
        }
        if year < MAX_YEAR {
            return Self::new(year + 1, 1, 1);
        }

        Err(TimeError::OutOfRange)
    }

    /// The day before this one.
    ///
    /// # Errors
    ///
    /// Returns [`TimeError::OutOfRange`] on the first day this type can name.
    pub const fn previous(self) -> Result<Self, TimeError> {
        let (year, month, day) = (self.year(), self.month(), self.day());

        if day > 1 {
            return Self::new(year, month, day - 1);
        }
        if month > 1 {
            return Self::new(year, month - 1, days_in_month(year, month - 1));
        }
        if year > MIN_YEAR {
            return Self::new(year - 1, 12, 31);
        }

        Err(TimeError::OutOfRange)
    }
}

impl fmt::Display for CivilDay {
    /// Prints the day as `YYYY-MM-DD`.
    ///
    /// Unambiguous everywhere. The order the person's country writes dates in is a question for
    /// the interface, which has a language; this is for a log, which does not.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year(),
            self.month(),
            self.day()
        )
    }
}

/// How many days a month has, leap years included.
///
/// The proleptic Gregorian calendar, applied to every year in range. It disagrees with history
/// before 1582 and agrees with every other piece of software written since, which is the only
/// property that matters for a date somebody typed into a habit tracker.
const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        // February in an ordinary year, and also every number that is not a month. The second
        // case is reached only by a caller that has not checked, and it answers with the
        // shortest month rather than panicking: too small a length makes the day check refuse,
        // and a panic would take the process down over a value the caller can be told about.
        _ => 28,
    }
}

/// Whether a year has a twenty-ninth of February.
const fn is_leap(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{CivilDay, MAX_YEAR, MIN_YEAR, TimeError, Timestamp, is_leap};

    #[test]
    fn a_moment_keeps_the_number_it_was_given() {
        let moment = Timestamp::from_micros(1_700_000_000_123_456);

        assert_eq!(moment.as_micros(), 1_700_000_000_123_456);
        assert_eq!(moment.as_millis(), 1_700_000_000_123);
    }

    #[test]
    fn a_moment_before_the_epoch_rounds_down_rather_than_towards_zero() {
        // The whole reason this uses Euclidean division. Rounding towards zero would put the
        // microsecond before the epoch in the same millisecond as the one after it, and two
        // writes a person made in order would come back sharing a clock reading.
        assert_eq!(Timestamp::from_micros(-1).as_millis(), -1);
        assert_eq!(Timestamp::from_micros(-1_000).as_millis(), -1);
        assert_eq!(Timestamp::from_micros(-1_001).as_millis(), -2);
    }

    #[test]
    fn moving_a_moment_stops_at_the_ends_of_the_range_instead_of_wrapping() {
        let latest = Timestamp::from_micros(i64::MAX);
        let earliest = Timestamp::from_micros(i64::MIN);

        assert_eq!(latest.saturating_add_micros(1), latest);
        assert_eq!(earliest.saturating_add_micros(-1), earliest);
        assert_eq!(latest.micros_since(earliest), i64::MAX);
    }

    #[test]
    fn a_day_that_exists_is_accepted_and_reads_back_in_three_parts() {
        let day = CivilDay::new(2026, 9, 17).expect("a day that exists");

        assert_eq!(day.as_number(), 20_260_917);
        assert_eq!((day.year(), day.month(), day.day()), (2026, 9, 17));
        assert_eq!(day.to_string(), "2026-09-17");
    }

    #[test]
    fn the_twenty_ninth_of_february_depends_on_the_year() {
        assert!(CivilDay::new(2024, 2, 29).is_ok(), "2024 is a leap year");
        assert!(CivilDay::new(2000, 2, 29).is_ok(), "2000 is a leap year");

        for year in [2023_u16, 1900, 2100] {
            let refused = CivilDay::new(year, 2, 29)
                .expect_err("a leap day was accepted in a year that has none");
            assert_eq!(
                refused,
                TimeError::Day {
                    day: 29,
                    length: 28
                }
            );
        }
    }

    #[test]
    fn a_day_that_does_not_exist_says_which_part_is_wrong() {
        assert_eq!(CivilDay::new(0, 1, 1), Err(TimeError::Year { year: 0 }));
        assert_eq!(
            CivilDay::new(10_000, 1, 1),
            Err(TimeError::Year { year: 10_000 })
        );
        assert_eq!(
            CivilDay::new(2026, 0, 1),
            Err(TimeError::Month { month: 0 })
        );
        assert_eq!(
            CivilDay::new(2026, 13, 1),
            Err(TimeError::Month { month: 13 })
        );
        assert_eq!(
            CivilDay::new(2026, 4, 31),
            Err(TimeError::Day {
                day: 31,
                length: 30
            })
        );
        assert_eq!(
            CivilDay::new(2026, 4, 0),
            Err(TimeError::Day { day: 0, length: 30 })
        );
    }

    #[test]
    fn a_number_that_is_not_a_day_is_refused_rather_than_reinterpreted() {
        for value in [0_u32, 1, 202_609, 20_260_932, 99_991_232, 100_000_000] {
            assert!(
                CivilDay::from_number(value).is_err(),
                "{value} was accepted as a day"
            );
        }
    }

    #[test]
    fn the_day_after_the_end_of_a_month_a_year_and_the_range() {
        let end_of_month = CivilDay::new(2026, 1, 31).expect("a day that exists");
        assert_eq!(end_of_month.next(), CivilDay::new(2026, 2, 1));

        let end_of_year = CivilDay::new(2026, 12, 31).expect("a day that exists");
        assert_eq!(end_of_year.next(), CivilDay::new(2027, 1, 1));

        let last = CivilDay::new(MAX_YEAR, 12, 31).expect("a day that exists");
        assert_eq!(last.next(), Err(TimeError::OutOfRange));
    }

    #[test]
    fn the_day_before_the_start_of_a_month_a_year_and_the_range() {
        let start_of_month = CivilDay::new(2026, 3, 1).expect("a day that exists");
        assert_eq!(start_of_month.previous(), CivilDay::new(2026, 2, 28));

        let after_a_leap_day = CivilDay::new(2024, 3, 1).expect("a day that exists");
        assert_eq!(after_a_leap_day.previous(), CivilDay::new(2024, 2, 29));

        let start_of_year = CivilDay::new(2026, 1, 1).expect("a day that exists");
        assert_eq!(start_of_year.previous(), CivilDay::new(2025, 12, 31));

        let first = CivilDay::new(MIN_YEAR, 1, 1).expect("a day that exists");
        assert_eq!(first.previous(), Err(TimeError::OutOfRange));
    }

    proptest! {
        /// Every day that can be built survives being stored and read back.
        ///
        /// This is the property the schema depends on: the column holds the number, and the
        /// number has to come back as the same day or the heatmap is drawing something else.
        #[test]
        fn a_day_survives_being_stored_as_a_number(
            year in MIN_YEAR..=MAX_YEAR,
            month in 1_u8..=12,
            day in 1_u8..=31,
        ) {
            let Ok(built) = CivilDay::new(year, month, day) else {
                return Ok(());
            };

            prop_assert_eq!(CivilDay::from_number(built.as_number()), Ok(built));
        }

        /// Days sort in the order they happen.
        ///
        /// Not obvious from the layout alone: it holds because the parts are packed most
        /// significant first. A different order would still round trip and would sort the
        /// thirty-first of January after the first of February.
        #[test]
        fn the_next_day_is_always_greater_and_comes_back(
            year in MIN_YEAR..MAX_YEAR,
            month in 1_u8..=12,
            day in 1_u8..=31,
        ) {
            let Ok(today) = CivilDay::new(year, month, day) else {
                return Ok(());
            };
            let tomorrow = today.next().expect("there is a day after this one");

            prop_assert!(tomorrow > today);
            prop_assert_eq!(tomorrow.previous(), Ok(today));
        }

        /// A leap year has three hundred and sixty-six days, and no other year does.
        #[test]
        fn a_year_has_the_number_of_days_the_calendar_says(year in MIN_YEAR..=MAX_YEAR) {
            let mut day = CivilDay::new(year, 1, 1).expect("the first of January exists");
            let mut counted = 1_u32;

            while day.month() != 12 || day.day() != 31 {
                day = day.next().expect("there is a day after this one");
                counted += 1;
            }

            prop_assert_eq!(counted, if is_leap(year) { 366 } else { 365 });
        }
    }
}
