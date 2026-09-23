//! Which day an instant falls on, for the person holding the device.
//!
//! [`cairn_domain::Timestamp`] is a point on a line every machine agrees about, and
//! [`cairn_domain::CivilDay`] is a square on a calendar that only means something in the place
//! somebody was standing. Turning the first into the second needs a place, and a place is read
//! from the operating system, which is why the conversion lives here and not in the domain.
//!
//! Two decisions are worth the words:
//!
//! The zone is never guessed. If the system does not say where it is, this answers
//! [`ClockError::NoZone`] and the screen says something is wrong. A streak counted in the wrong
//! zone looks exactly like a correct one, so a silent fallback to UTC would be a defect nobody
//! could see and everybody would trust.
//!
//! The day may start somewhere other than midnight, because somebody who reads at one in the
//! morning is still having last night. That shift is a civil-time shift, applied to the wall
//! clock after the zone has been resolved, not an instant-arithmetic one: on the morning the
//! clocks go forward, shifting the instant would move the boundary by an hour, and the day a
//! habit was marked on would depend on the weather of the calendar.

use cairn_domain::{CivilDay, Timestamp};
use jiff::{SignedDuration, tz::TimeZone};

/// How far from midnight a day may start, in minutes, in either direction.
///
/// Twelve hours. Past that the shift stops being "my day starts late" and starts being a
/// different day, and every streak the person has ever seen would renumber itself.
const MAX_OFFSET_MINUTES: i32 = 720;

/// What can go wrong turning an instant into a day.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ClockError {
    /// The operating system did not say what time zone it is in.
    ///
    /// Never falls back to UTC. A streak counted in the wrong zone is worse than a screen that
    /// says something is wrong, because nobody can tell it is wrong by looking at it.
    #[error("the system did not report a time zone")]
    NoZone,
    /// The instant is outside the range a civil day can name.
    #[error("that instant is not a day this calendar can name")]
    OutOfRange,
    /// The start-of-day offset is not between -720 and 720 minutes.
    #[error("a day may not start {minutes} minutes from midnight")]
    Offset {
        /// What was given.
        minutes: i32,
    },
}

/// How far from midnight a day starts, in minutes.
///
/// A newtype rather than an `i32`, because the one thing that must never happen is passing the
/// offset where the day number goes, and two integers in a row compile perfectly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayStart(i32);

impl DayStart {
    /// Midnight.
    pub const MIDNIGHT: Self = Self(0);

    /// A start-of-day offset from a count of minutes.
    ///
    /// # Errors
    ///
    /// [`ClockError::Offset`] outside -720..=720.
    pub fn from_minutes(minutes: i32) -> Result<Self, ClockError> {
        if !(-MAX_OFFSET_MINUTES..=MAX_OFFSET_MINUTES).contains(&minutes) {
            return Err(ClockError::Offset { minutes });
        }

        Ok(Self(minutes))
    }

    /// The count of minutes from midnight.
    #[must_use]
    pub const fn as_minutes(self) -> i32 {
        self.0
    }
}

/// Which day an instant falls on, somewhere.
///
/// A trait so that every test can say what zone it is in, and so that nothing below this line
/// reads a clock or a zone by itself.
pub trait CivilClock {
    /// The day the given instant falls on, with the day starting where `start` says.
    ///
    /// # Errors
    ///
    /// See [`ClockError`].
    fn day_of(&self, at: Timestamp, start: DayStart) -> Result<CivilDay, ClockError>;
}

/// The zone the operating system says this device is in.
#[derive(Debug, Clone)]
pub struct SystemZone {
    zone: TimeZone,
}

impl SystemZone {
    /// Reads the zone once.
    ///
    /// Once, literally: the value is a snapshot, and somebody who flies east keeps yesterday's
    /// zone until whoever holds this calls again. Deliberate, because a zone that could change
    /// under a calculation would make two days in the same screenful disagree about where the
    /// person was. Deciding when to read it again is the caller's, and belongs where the
    /// application knows it has been woken up.
    ///
    /// # Errors
    ///
    /// [`ClockError::NoZone`] if the system does not report one.
    pub fn detect() -> Result<Self, ClockError> {
        // `try_system` rather than `system`: the latter answers `Etc/Unknown` and logs a warning
        // when it cannot tell, and `Etc/Unknown` behaves as UTC. The unknown zone is refused
        // again below, because it can also arrive from a `TZ` variable set to it, and a zone
        // whose whole meaning is "nobody knows" is not a place a streak can be counted in.
        let zone = TimeZone::try_system().map_err(|_unavailable| ClockError::NoZone)?;
        if zone.is_unknown() {
            return Err(ClockError::NoZone);
        }

        Ok(Self { zone })
    }
}

impl CivilClock for SystemZone {
    fn day_of(&self, at: Timestamp, start: DayStart) -> Result<CivilDay, ClockError> {
        let instant = jiff::Timestamp::from_microsecond(at.as_micros())
            .map_err(|_out_of_range| ClockError::OutOfRange)?;

        // The wall clock in this zone, then the shift, then the date. In this order: the shift is
        // subtracted from the civil time, so an hour that the clocks skipped or repeated cannot
        // move the boundary the day is cut at.
        let wall = instant.to_zoned(self.zone.clone()).datetime();
        let shifted = wall
            .checked_sub(SignedDuration::from_mins(i64::from(start.as_minutes())))
            .map_err(|_out_of_range| ClockError::OutOfRange)?;
        let date = shifted.date();

        // Every one of these refusals is the same refusal: the calendar of the domain names years
        // 1 to 9999 and nothing outside them, and `jiff` reaches further in both directions.
        let year = u16::try_from(date.year()).map_err(|_outside| ClockError::OutOfRange)?;
        let month = u8::try_from(date.month()).map_err(|_outside| ClockError::OutOfRange)?;
        let day = u8::try_from(date.day()).map_err(|_outside| ClockError::OutOfRange)?;

        CivilDay::new(year, month, day).map_err(|_outside| ClockError::OutOfRange)
    }
}

/// A zone named by its IANA identifier. Exists so tests can name one.
///
/// # Errors
///
/// [`ClockError::NoZone`] if that name is not in the database this build has.
pub fn zone_named(name: &str) -> Result<SystemZone, ClockError> {
    let zone = TimeZone::get(name).map_err(|_unknown_name| ClockError::NoZone)?;

    Ok(SystemZone { zone })
}

#[cfg(test)]
mod tests {
    use cairn_domain::{CivilDay, Timestamp};
    use jiff::civil;

    use super::{CivilClock, ClockError, DayStart, SystemZone, zone_named};

    /// The moment a `jiff` instant names, in the units the domain uses.
    fn moment(instant: jiff::Timestamp) -> Timestamp {
        Timestamp::from_micros(instant.as_microsecond())
    }

    /// The moment at which the wall clock in a zone reads the given civil time.
    ///
    /// On the two mornings a year when a wall clock reading is not a single moment, this picks
    /// whatever `jiff` picks. The tests that care about those mornings name the instant in UTC
    /// instead, which is never ambiguous.
    fn wall(zone: &str, at: civil::DateTime) -> Timestamp {
        moment(
            at.in_tz(zone)
                .expect("the tests name zones that exist")
                .timestamp(),
        )
    }

    /// The moment at which the clock in Greenwich reads the given civil time.
    fn utc(at: civil::DateTime) -> Timestamp {
        wall("UTC", at)
    }

    /// A day the tests can compare against, built from its three parts.
    fn day(year: u16, month: u8, of_month: u8) -> CivilDay {
        CivilDay::new(year, month, of_month).expect("the tests name days that exist")
    }

    /// An offset the tests can pass, built from its count of minutes.
    fn starts_at(minutes: i32) -> DayStart {
        DayStart::from_minutes(minutes).expect("the tests name offsets that are allowed")
    }

    /// Madrid, which is the zone every case below is in unless it says otherwise.
    fn madrid() -> SystemZone {
        zone_named("Europe/Madrid").expect("Europe/Madrid is in every copy of the database")
    }

    #[test]
    fn local_midnight_belongs_to_the_day_that_begins() {
        // The first minute of a day is in that day, not in the one that just ended. Off by one
        // here and every streak is a day behind for an hour.
        let at = wall("Europe/Madrid", civil::datetime(2026, 9, 17, 0, 0, 0, 0));

        assert_eq!(
            madrid().day_of(at, DayStart::MIDNIGHT),
            Ok(day(2026, 9, 17))
        );
    }

    #[test]
    fn the_last_second_of_a_day_still_belongs_to_it() {
        let at = wall("Europe/Madrid", civil::datetime(2026, 9, 17, 23, 59, 59, 0));

        assert_eq!(
            madrid().day_of(at, DayStart::MIDNIGHT),
            Ok(day(2026, 9, 17))
        );
    }

    #[test]
    fn with_a_day_that_starts_at_four_the_small_hours_are_still_last_night() {
        let at = wall("Europe/Madrid", civil::datetime(2026, 9, 17, 1, 0, 0, 0));

        assert_eq!(madrid().day_of(at, starts_at(240)), Ok(day(2026, 9, 16)));
    }

    #[test]
    fn with_a_day_that_starts_at_four_the_day_begins_on_the_hour() {
        // Both sides of the boundary, because four o'clock on its own would also be the
        // seventeenth under an offset of the wrong size, or of no offset at all.
        let one_second_early = wall("Europe/Madrid", civil::datetime(2026, 9, 17, 3, 59, 59, 0));
        let on_the_hour = wall("Europe/Madrid", civil::datetime(2026, 9, 17, 4, 0, 0, 0));

        assert_eq!(
            madrid().day_of(one_second_early, starts_at(240)),
            Ok(day(2026, 9, 16))
        );
        assert_eq!(
            madrid().day_of(on_the_hour, starts_at(240)),
            Ok(day(2026, 9, 17))
        );
    }

    #[test]
    fn the_morning_an_hour_does_not_exist_never_makes_the_day_go_backwards() {
        // 2026-03-29: Madrid goes from 01:59:59 to 03:00:00 local, at 01:00:00 UTC. The two
        // instants either side of that are consecutive, and an hour apart on the wall clock.
        let before = utc(civil::datetime(2026, 3, 29, 0, 59, 59, 999_999_000));
        let after = utc(civil::datetime(2026, 3, 29, 1, 0, 0, 0));

        for start in [DayStart::MIDNIGHT, starts_at(240), starts_at(-240)] {
            let earlier = madrid()
                .day_of(before, start)
                .expect("an ordinary instant in Madrid");
            let later = madrid()
                .day_of(after, start)
                .expect("an ordinary instant in Madrid");

            assert!(
                later >= earlier,
                "the day went from {later} back to {earlier} across the spring transition \
                 with a day starting {} minutes from midnight",
                start.as_minutes()
            );
        }
    }

    #[test]
    fn the_morning_an_hour_happens_twice_gives_the_same_day_both_times() {
        // 2026-10-25: Madrid reads 02:30 local twice, once at 00:30 UTC on summer time and once
        // at 01:30 UTC on winter time. Both are the twenty-fifth, and neither is an error.
        let first_pass = utc(civil::datetime(2026, 10, 25, 0, 30, 0, 0));
        let second_pass = utc(civil::datetime(2026, 10, 25, 1, 30, 0, 0));

        assert_eq!(
            madrid().day_of(first_pass, DayStart::MIDNIGHT),
            Ok(day(2026, 10, 25))
        );
        assert_eq!(
            madrid().day_of(second_pass, DayStart::MIDNIGHT),
            Ok(day(2026, 10, 25))
        );
    }

    #[test]
    fn one_instant_is_two_different_days_at_the_two_ends_of_the_world() {
        // Kiritimati is fourteen hours ahead and Niue eleven behind: twenty-five hours apart, so
        // there is always an instant that is two different dates in the two of them at once.
        let at = utc(civil::datetime(2026, 9, 17, 12, 0, 0, 0));

        let ahead = zone_named("Pacific/Kiritimati").expect("Kiritimati is in the database");
        let behind = zone_named("Pacific/Niue").expect("Niue is in the database");

        assert_eq!(ahead.day_of(at, DayStart::MIDNIGHT), Ok(day(2026, 9, 18)));
        assert_eq!(behind.day_of(at, DayStart::MIDNIGHT), Ok(day(2026, 9, 17)));
    }

    #[test]
    fn an_offset_further_than_half_a_day_from_midnight_is_refused() {
        assert_eq!(
            DayStart::from_minutes(721),
            Err(ClockError::Offset { minutes: 721 })
        );
        assert_eq!(
            DayStart::from_minutes(-721),
            Err(ClockError::Offset { minutes: -721 })
        );
        assert_eq!(
            DayStart::from_minutes(720).map(DayStart::as_minutes),
            Ok(720)
        );
        assert_eq!(DayStart::MIDNIGHT.as_minutes(), 0);
    }

    #[test]
    fn the_zone_of_this_machine_is_either_a_real_zone_or_an_admitted_absence() {
        // The prohibition this guards is that there is no third answer. A silent fall back to
        // UTC cannot be provoked from a test without lying to the operating system, so it is
        // ruled out in the code instead: `detect` calls `try_system`, which fails rather than
        // guessing, and then refuses `Etc/Unknown`, which is the guess wearing a name.
        let at = utc(civil::datetime(2026, 9, 17, 12, 0, 0, 0));

        match SystemZone::detect() {
            Ok(zone) => {
                let today = zone
                    .day_of(at, DayStart::MIDNIGHT)
                    .expect("a real zone names a day for an ordinary instant");

                // No zone on earth is more than a day away from Greenwich.
                assert!(
                    today >= day(2026, 9, 16) && today <= day(2026, 9, 18),
                    "the zone of this machine put an ordinary instant on {today}"
                );
            }
            Err(refused) => assert_eq!(refused, ClockError::NoZone),
        }
    }

    #[test]
    fn an_instant_outside_the_years_the_calendar_names_is_refused_without_panicking() {
        // The first instant of the year 10000, and the first of the year zero, in microseconds
        // since the epoch. Written as constants because neither is a date this calendar, or the
        // library underneath it, can be asked to build.
        let year_ten_thousand = Timestamp::from_micros(253_402_300_800_000_000);
        let before_year_one = Timestamp::from_micros(-62_167_219_200_000_000);

        assert_eq!(
            madrid().day_of(year_ten_thousand, DayStart::MIDNIGHT),
            Err(ClockError::OutOfRange)
        );
        assert_eq!(
            madrid().day_of(before_year_one, DayStart::MIDNIGHT),
            Err(ClockError::OutOfRange)
        );
    }

    #[test]
    fn a_name_that_is_not_in_the_database_is_refused_rather_than_approximated() {
        assert_eq!(
            zone_named("Europe/Atlantis").err(),
            Some(ClockError::NoZone)
        );
    }
}
