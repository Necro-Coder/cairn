//! When a vault locks itself, and how long somebody has to wait after getting the password
//! wrong.
//!
//! Arithmetic only. Nothing here reads a clock, holds a key or touches a file: every
//! function takes the moment as a parameter and returns what should happen. That is what
//! makes a five minute lockout testable in a microsecond, and it is the reason this lives in
//! the crate that knows nothing about the operating system.
//!
//! The thing worth being clear about is what the backoff is for. It is not the defence
//! against somebody guessing the master password; Argon2id is, and it charges roughly a
//! tenth of a second for every guess whether this code exists or not. Anybody who has the
//! file will attack the file, where none of this applies. What the backoff does is make the
//! interface unpleasant to sit in front of and try passwords by hand, and past five minutes
//! the only person still being punished is the owner.

/// Microseconds in a second, since every moment in this project is microseconds since the
/// epoch in UTC.
const MICROS_PER_SECOND: i64 = 1_000_000;

/// The same number, unsigned, for the two roundings below.
///
/// Rounding up is `div_ceil`, which exists for unsigned integers and not for signed ones, and
/// both callers have already established that what they are rounding is positive.
const MICROS_PER_SECOND_UNSIGNED: u64 = MICROS_PER_SECOND.unsigned_abs();

/// How long the first failed attempt costs, in seconds.
pub const FIRST_BACKOFF_S: u32 = 1;

/// The longest the backoff ever grows to, in seconds.
///
/// Five minutes. Beyond this the wait stops being a deterrent and starts being a punishment
/// for the person who mistyped, because an attacker with the file left the interface behind
/// a long time ago.
pub const MAX_BACKOFF_S: u32 = 5 * 60;

/// How many failures it takes to reach the ceiling.
///
/// One, two, four, eight, sixteen, thirty-two, sixty-four, a hundred and twenty-eight, two
/// hundred and fifty-six, and then the ceiling. Written down as a constant so that the
/// schedule is a fact in one place rather than something to be re-derived from the
/// arithmetic below.
pub const FAILURES_TO_REACH_CEILING: u32 = 10;

/// How long to refuse further attempts after `failed_attempts` consecutive failures.
///
/// Doubling, from one second, capped. Zero failures cost nothing, which is what makes the
/// first attempt after an unlock immediate.
#[must_use]
pub fn backoff_seconds(failed_attempts: u32) -> u32 {
    if failed_attempts == 0 {
        return 0;
    }

    // Shifting rather than a loop, and saturating rather than wrapping, because the shift
    // would be undefined past the width of the type and a wrap would turn a five minute
    // lockout into no lockout at all on the thirty-third failure.
    let doubling = FIRST_BACKOFF_S.checked_shl(failed_attempts - 1);

    doubling.unwrap_or(MAX_BACKOFF_S).min(MAX_BACKOFF_S)
}

/// The moment further attempts stop being refused, given when the failure happened.
///
/// Saturating, so that a clock near the end of its range produces the largest moment it can
/// rather than wrapping round to the distant past and letting the lockout disappear.
#[must_use]
pub fn locked_until_us(failed_attempts: u32, failed_at_us: i64) -> i64 {
    let seconds = i64::from(backoff_seconds(failed_attempts));

    failed_at_us.saturating_add(seconds.saturating_mul(MICROS_PER_SECOND))
}

/// How many seconds of the lockout are left at `now_us`, or zero if it has passed.
///
/// A clock that has moved backwards since the lockout was written would otherwise produce a
/// wait longer than the one that was imposed. It is capped at the longest backoff for that
/// reason: the moment in the file is not authenticated, so it is a number from a file rather
/// than a fact, and a file that says the vault is locked until the year three thousand must
/// not be believed.
#[must_use]
pub fn backoff_remaining_s(locked_until_us: i64, now_us: i64) -> u32 {
    let remaining_us = locked_until_us.saturating_sub(now_us);
    if remaining_us <= 0 {
        return 0;
    }

    // Rounded up, so that a wait of a hundred milliseconds reads as one second rather than
    // as none. Somebody told to wait zero seconds and then refused would think it was broken.
    // The magnitude rather than the value, because the guard above has already ruled out
    // everything that is not positive and the unsigned form is what rounds up.
    let seconds = remaining_us
        .unsigned_abs()
        .div_ceil(MICROS_PER_SECOND_UNSIGNED);

    u32::try_from(seconds)
        .unwrap_or(MAX_BACKOFF_S)
        .min(MAX_BACKOFF_S)
}

/// How long the vault may sit idle before it locks itself.
///
/// Never is one of the choices, and it is a variant rather than a sentinel value, because a
/// timeout of zero meaning forever is the kind of thing that reads correctly right up until
/// somebody writes the comparison the other way round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InactivityTimeout {
    /// Locks itself after this many minutes without keyboard or mouse in the window.
    After(InactivityMinutes),
    /// Never locks itself. Chosen deliberately, and the interface says what it means.
    Never,
}

/// The inactivity periods that can be chosen.
///
/// A closed set rather than a number, because the interface offers four periods and never,
/// and a value that cannot be spelled cannot arrive from the bridge. Thirty seconds of
/// warning is shown before the shortest of them, which is why there is no shorter one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InactivityMinutes {
    /// One minute.
    One,
    /// Five minutes. What a new installation uses.
    Five,
    /// Fifteen minutes.
    Fifteen,
    /// Thirty minutes.
    Thirty,
}

impl InactivityMinutes {
    /// The period in minutes.
    #[must_use]
    pub const fn minutes(self) -> u32 {
        match self {
            Self::One => 1,
            Self::Five => 5,
            Self::Fifteen => 15,
            Self::Thirty => 30,
        }
    }

    /// The period in microseconds.
    #[must_use]
    pub fn micros(self) -> i64 {
        i64::from(self.minutes()) * 60 * MICROS_PER_SECOND
    }
}

impl Default for InactivityTimeout {
    /// Five minutes, which is what a new installation uses.
    fn default() -> Self {
        Self::After(InactivityMinutes::Five)
    }
}

/// How long before the lock the interface warns, in seconds.
///
/// The warning is calculated by the interface from the countdown it is given, rather than
/// being announced by an event. One fewer thing crossing the bridge, and the interface
/// already has to count down to show the number.
pub const WARNING_BEFORE_LOCK_S: u32 = 30;

/// How long after the window loses focus the vault locks, in seconds.
///
/// Not instant, deliberately. Switching away for a second to copy something is constant, and
/// a vault that locks the moment it is not in front becomes a vault whose automatic locking
/// gets turned off. Minimising is different and locks at once: nobody minimises a window
/// they are about to use.
pub const LOCK_AFTER_FOCUS_LOST_S: u32 = 30;

/// What should happen to a session that has been idle since `last_activity_us`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleDecision {
    /// Nothing yet. The number is how many seconds are left before it locks.
    Wait {
        /// Seconds remaining before the lock.
        remaining_s: u32,
    },
    /// Lock now.
    Lock,
    /// Never locks itself, because that is what was chosen.
    NeverLocks,
}

/// Decides what to do with a session that was last used at `last_activity_us`.
///
/// A clock that has moved backwards produces a wait rather than a lock. The alternative,
/// treating a negative elapsed time as an enormous one, would lock the vault every time the
/// machine adjusted its clock, which is a thing machines do without asking.
#[must_use]
pub fn idle_decision(
    timeout: InactivityTimeout,
    last_activity_us: i64,
    now_us: i64,
) -> IdleDecision {
    let InactivityTimeout::After(minutes) = timeout else {
        return IdleDecision::NeverLocks;
    };

    let locks_at = last_activity_us.saturating_add(minutes.micros());
    let remaining_us = locks_at.saturating_sub(now_us);

    if remaining_us <= 0 {
        return IdleDecision::Lock;
    }

    // Rounded up for the same reason as the backoff: a countdown that shows zero and has not
    // locked yet looks like a program that has stopped working.
    let seconds = remaining_us
        .unsigned_abs()
        .div_ceil(MICROS_PER_SECOND_UNSIGNED);

    IdleDecision::Wait {
        remaining_s: u32::try_from(seconds).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FAILURES_TO_REACH_CEILING, FIRST_BACKOFF_S, IdleDecision, InactivityMinutes,
        InactivityTimeout, LOCK_AFTER_FOCUS_LOST_S, MAX_BACKOFF_S, WARNING_BEFORE_LOCK_S,
        backoff_remaining_s, backoff_seconds, idle_decision, locked_until_us,
    };

    /// A moment in the middle of the range, so that arithmetic either side of it is ordinary.
    const NOW_US: i64 = 1_700_000_000_000_000;

    #[test]
    fn the_schedule_is_the_one_that_was_decided() {
        // Written out rather than computed, because this table is the decision and the
        // arithmetic above is only one way of producing it.
        let expected = [
            (0, 0),
            (1, 1),
            (2, 2),
            (3, 4),
            (4, 8),
            (5, 16),
            (6, 32),
            (7, 64),
            (8, 128),
            (9, 256),
            (10, 300),
            (11, 300),
            (50, 300),
        ];

        for (failures, seconds) in expected {
            assert_eq!(
                backoff_seconds(failures),
                seconds,
                "the wait after {failures} failures is not what was decided"
            );
        }
    }

    #[test]
    fn the_constants_are_the_numbers_they_are_documented_to_be() {
        assert_eq!(FIRST_BACKOFF_S, 1);
        assert_eq!(MAX_BACKOFF_S, 300);
        assert_eq!(WARNING_BEFORE_LOCK_S, 30);
        assert_eq!(LOCK_AFTER_FOCUS_LOST_S, 30);
    }

    #[test]
    fn the_ceiling_is_reached_where_it_is_documented_to_be() {
        assert!(backoff_seconds(FAILURES_TO_REACH_CEILING - 1) < MAX_BACKOFF_S);
        assert_eq!(backoff_seconds(FAILURES_TO_REACH_CEILING), MAX_BACKOFF_S);
    }

    #[test]
    fn a_successful_unlock_costs_nothing() {
        // The counter is reset to zero after an unlock, and zero has to mean no wait at all.
        // A first attempt that had to wait a second would be a program that felt broken.
        assert_eq!(backoff_seconds(0), 0);
        assert_eq!(locked_until_us(0, NOW_US), NOW_US);
        assert_eq!(backoff_remaining_s(NOW_US, NOW_US), 0);
    }

    #[test]
    fn an_absurd_number_of_failures_still_lands_on_the_ceiling() {
        // The shift would be undefined past the width of the type, and a wrap would turn the
        // lockout into nothing at exactly the moment it is most wanted.
        assert_eq!(backoff_seconds(u32::MAX), MAX_BACKOFF_S);
        assert_eq!(backoff_seconds(32), MAX_BACKOFF_S);
        assert_eq!(backoff_seconds(33), MAX_BACKOFF_S);
    }

    #[test]
    fn the_lockout_ends_the_number_of_seconds_later_that_it_said() {
        let until = locked_until_us(4, NOW_US);

        assert_eq!(until, NOW_US + 8 * 1_000_000);
        assert_eq!(backoff_remaining_s(until, NOW_US), 8);
        assert_eq!(backoff_remaining_s(until, NOW_US + 7_500_000), 1);
        assert_eq!(backoff_remaining_s(until, until), 0);
        assert_eq!(backoff_remaining_s(until, until + 1), 0);
    }

    #[test]
    fn part_of_a_second_still_reads_as_a_second() {
        // Somebody told to wait zero seconds and then refused would think it was broken.
        assert_eq!(backoff_remaining_s(NOW_US + 1, NOW_US), 1);
        assert_eq!(backoff_remaining_s(NOW_US + 999_999, NOW_US), 1);
        assert_eq!(backoff_remaining_s(NOW_US + 1_000_001, NOW_US), 2);
    }

    #[test]
    fn a_clock_moved_backwards_cannot_extend_the_wait_past_the_ceiling() {
        // The moment lives in the part of the header nobody authenticates, so it is a number
        // from a file rather than a fact. A file claiming the vault is locked for a thousand
        // years must not be believed.
        let absurd = i64::MAX;

        assert_eq!(backoff_remaining_s(absurd, NOW_US), MAX_BACKOFF_S);
        assert_eq!(
            backoff_remaining_s(NOW_US + 1_000_000_000_000, NOW_US),
            MAX_BACKOFF_S
        );
    }

    #[test]
    fn a_clock_at_the_end_of_its_range_does_not_wrap_into_no_lockout_at_all() {
        assert_eq!(locked_until_us(10, i64::MAX), i64::MAX);
        assert!(locked_until_us(10, i64::MAX - 1) >= i64::MAX - 1);
    }

    #[test]
    fn the_default_is_five_minutes() {
        assert_eq!(
            InactivityTimeout::default(),
            InactivityTimeout::After(InactivityMinutes::Five)
        );
    }

    #[test]
    fn every_period_is_the_number_of_minutes_it_is_called() {
        for (period, minutes) in [
            (InactivityMinutes::One, 1),
            (InactivityMinutes::Five, 5),
            (InactivityMinutes::Fifteen, 15),
            (InactivityMinutes::Thirty, 30),
        ] {
            assert_eq!(period.minutes(), minutes);
            assert_eq!(period.micros(), i64::from(minutes) * 60 * 1_000_000);
        }
    }

    #[test]
    fn a_session_that_has_just_been_used_waits_the_whole_period() {
        let decision = idle_decision(
            InactivityTimeout::After(InactivityMinutes::Five),
            NOW_US,
            NOW_US,
        );

        assert_eq!(decision, IdleDecision::Wait { remaining_s: 300 });
    }

    #[test]
    fn a_session_idle_for_the_whole_period_locks() {
        let five_minutes = 300 * 1_000_000;

        assert_eq!(
            idle_decision(
                InactivityTimeout::After(InactivityMinutes::Five),
                NOW_US,
                NOW_US + five_minutes
            ),
            IdleDecision::Lock
        );
        assert_eq!(
            idle_decision(
                InactivityTimeout::After(InactivityMinutes::Five),
                NOW_US,
                NOW_US + five_minutes + 1
            ),
            IdleDecision::Lock
        );
    }

    #[test]
    fn the_second_before_the_lock_is_still_a_wait() {
        let decision = idle_decision(
            InactivityTimeout::After(InactivityMinutes::One),
            NOW_US,
            NOW_US + 59_000_000,
        );

        assert_eq!(decision, IdleDecision::Wait { remaining_s: 1 });
    }

    #[test]
    fn a_clock_moved_backwards_waits_rather_than_locking() {
        // Machines adjust their clocks without asking. Treating a negative elapsed time as an
        // enormous one would lock the vault every time one did.
        let decision = idle_decision(
            InactivityTimeout::After(InactivityMinutes::Five),
            NOW_US,
            NOW_US - 60 * 60 * 1_000_000,
        );

        assert!(
            matches!(decision, IdleDecision::Wait { .. }),
            "a clock that moved backwards locked the vault"
        );
    }

    #[test]
    fn never_means_never_whatever_the_clock_says() {
        for now in [NOW_US, NOW_US + 1, i64::MAX, i64::MIN] {
            assert_eq!(
                idle_decision(InactivityTimeout::Never, NOW_US, now),
                IdleDecision::NeverLocks
            );
        }
    }

    #[test]
    fn the_warning_fits_inside_the_shortest_period() {
        // The interface shows a warning before the lock, counted down from the number it is
        // given. A warning longer than the shortest period would mean the warning appeared
        // before the session had started.
        assert!(
            i64::from(WARNING_BEFORE_LOCK_S) * 1_000_000 < InactivityMinutes::One.micros(),
            "the warning does not fit inside the shortest period that can be chosen"
        );
    }
}
