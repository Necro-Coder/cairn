//! The one place that asks what time it is.
//!
//! Everything below this layer takes the moment as an argument. That is what lets the
//! session arithmetic, the header and the merge rules be tested without waiting for a real
//! second to pass, and it is why there is exactly one function here that reads anything.
//!
//! Microseconds since the epoch in UTC, signed, because that is what the header stores and
//! converting at each boundary would be one more place to get it wrong.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The current moment, in microseconds since the epoch in UTC.
///
/// A machine whose clock is set before 1970 answers with a negative number rather than
/// failing, and a clock set beyond the range of the type saturates. Neither is worth a
/// fallible signature: nothing downstream trusts this value for security, and the two things
/// that could be affected, the inactivity timer and the lockout, both treat a moment that
/// makes no sense as a reason to wait rather than as a reason to act.
#[must_use]
pub fn now_us() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(after) => micros_after_epoch(after),
        Err(before) => micros_before_epoch(before.duration()),
    }
}

/// The current moment, in milliseconds since the epoch in UTC.
///
/// What the logical clock reads. Milliseconds rather than microseconds because that is the unit
/// the reading carries, and a clock set before the epoch answers with zero: a negative wall
/// reading is not something the sixteen byte layout can hold, and the clock refuses to go
/// backwards anyway, so the first write on such a machine simply starts from the beginning.
#[must_use]
pub fn now_ms() -> u64 {
    u64::try_from(cairn_domain::Timestamp::from_micros(now_us()).as_millis()).unwrap_or(0)
}

/// Converts a distance after the epoch, saturating rather than wrapping.
fn micros_after_epoch(distance: Duration) -> i64 {
    i64::try_from(distance.as_micros()).unwrap_or(i64::MAX)
}

/// Converts a distance before the epoch, saturating rather than wrapping.
///
/// Negated after the conversion rather than before, because the largest distance a duration
/// can express is larger than the largest number this type can hold in either direction.
fn micros_before_epoch(distance: Duration) -> i64 {
    micros_after_epoch(distance).saturating_neg()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{micros_after_epoch, micros_before_epoch, now_us};

    /// The first of January 2020, in microseconds since the epoch. Any machine running this
    /// has a clock set after it unless somebody moved it on purpose.
    const TWENTY_TWENTY_US: i64 = 1_577_836_800_000_000;

    /// The first of January 2200, by which point this code is somebody else's problem.
    const TWENTY_TWO_HUNDRED_US: i64 = 7_258_118_400_000_000;

    #[test]
    fn the_clock_reports_a_moment_in_roughly_this_century() {
        let now = now_us();

        assert!(
            (TWENTY_TWENTY_US..TWENTY_TWO_HUNDRED_US).contains(&now),
            "the clock on this machine reads {now}, which is not a plausible moment"
        );
    }

    #[test]
    fn an_ordinary_moment_converts_exactly() {
        assert_eq!(micros_after_epoch(Duration::from_micros(1)), 1);
        assert_eq!(
            micros_after_epoch(Duration::from_secs(1_700_000_000)),
            1_700_000_000_000_000
        );
    }

    #[test]
    fn a_clock_set_before_the_epoch_reads_as_a_negative_moment() {
        assert_eq!(micros_before_epoch(Duration::from_secs(1)), -1_000_000);
        assert_eq!(micros_before_epoch(Duration::ZERO), 0);
    }

    #[test]
    fn a_clock_set_beyond_the_range_saturates_rather_than_wrapping() {
        // A duration can express distances this type cannot hold, so both directions have to
        // stop at the end rather than reappearing at the other one. A wrap here would put
        // the moment in the distant past, and the lockout arithmetic would then believe the
        // wait had already elapsed.
        assert_eq!(micros_after_epoch(Duration::MAX), i64::MAX);
        assert_eq!(micros_before_epoch(Duration::MAX), i64::MIN + 1);
    }
}
