//! The two periods a deleted entry lives through, and the arithmetic that tells them apart.
//!
//! There are two, they mean different things, and the only way they stay apart is that there is
//! one place where the difference is written down.
//!
//! **Thirty days** is for the person. They threw something away, they changed their mind, they
//! want it back. After that the bin empties itself, because a bin that never empties is a second
//! list nobody maintains.
//!
//! **A hundred and eighty days** is for the merge. The other device has been off for months, and
//! when it comes back it has to learn that the row went rather than offer to send it again.
//! Nobody ever sees this one.
//!
//! Written `30` by hand in the repository and `180` by hand in the sweep, the two would one day
//! be the same number by accident, and the result is either half a year of rubbish on a screen or
//! something deleted coming back after a synchronisation. So they are named here, together, with
//! the reason each is what it is, and a test holds one of them to the number the database uses.

/// How many microseconds are in a day.
///
/// Every moment in this application is microseconds since the epoch, so this is the one
/// conversion the module needs and the only place it is written.
const MICROS_PER_DAY: i64 = 24 * 60 * 60 * 1_000_000;

/// How long something stays where somebody can get it back.
///
/// Thirty days. Long enough to cover a holiday, short enough that the bin is a bin and not a
/// second list. Not the same as [`TOMBSTONE_DAYS`] and deliberately far from it.
pub const TRASH_DAYS: i64 = 30;

/// How long the mark that something was deleted survives, for the benefit of the other device.
///
/// A hundred and eighty, which is what `cairn_db::tombstones::RETENTION_DAYS` says. Named again
/// here so the domain can reason about both without depending on the database, and pinned to it
/// by a test rather than by an import. That test lives in `cairn-db`, which is the one crate
/// that can see both numbers at once.
pub const TOMBSTONE_DAYS: i64 = 180;

/// Where one row stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashState {
    /// In the lists, in the search, and in the way.
    Live,
    /// Thrown away, still whole, still recoverable. Out of every list but the bin.
    InBin {
        /// How many whole days it has been there.
        days: i64,
        /// How many are left before it goes on its own.
        days_left: i64,
    },
    /// Past the bin's window: nothing to show, and the next sweep empties it.
    Expired,
    /// Already emptied. The row is a skeleton and there is nothing inside it.
    Gone,
}

/// Where a row stands, from the three columns that decide it.
///
/// `now_us`, `trashed_at` and the deleted flag arrive from the caller. Nothing here reads a
/// clock.
///
/// Total: every combination of the three has one of the four answers, so there is no `Result`
/// and no case a caller has to remember to handle.
///
/// A `trashed_at` in the future — which will arrive one day from a device whose clock ran ahead —
/// is reported as having been thrown away this instant rather than refused. It is not an error
/// and there is nothing useful to do about it; what must not happen is a negative count of days
/// reaching a screen that then offers "3 days left" with a minus sign in front of it.
#[must_use]
pub fn state(now_us: i64, trashed_at: Option<i64>, deleted: bool) -> TrashState {
    // First, and whatever the other two say. A skeleton is not in the bin: there is nothing left
    // inside it to restore, so offering it as recoverable would be offering an empty row back.
    if deleted {
        return TrashState::Gone;
    }

    let Some(trashed_at) = trashed_at else {
        return TrashState::Live;
    };

    let days = days_in_bin(now_us, trashed_at);
    if days >= TRASH_DAYS {
        return TrashState::Expired;
    }

    TrashState::InBin {
        days,
        days_left: TRASH_DAYS.saturating_sub(days),
    }
}

/// The instant before which everything in the bin has outstayed its welcome.
///
/// What the sweep compares against, so the comparison is written once and no statement anywhere
/// carries a thirty of its own.
#[must_use]
pub fn bin_cutoff_us(now_us: i64) -> i64 {
    now_us.saturating_sub(TRASH_DAYS.saturating_mul(MICROS_PER_DAY))
}

/// How many whole days something has been in the bin, never fewer than none.
///
/// Truncated downwards, so something thrown away twenty-three hours ago has been there zero days.
/// The subtraction saturates because both arguments are sixty-four bit moments, one from a clock
/// and one from a file, and nothing about either guarantees they are close together.
#[expect(
    clippy::integer_division,
    reason = "whole elapsed days is exactly what the bin counts in, and the truncation towards zero is the rule this function exists to state"
)]
fn days_in_bin(now_us: i64, trashed_at: i64) -> i64 {
    now_us.saturating_sub(trashed_at).max(0) / MICROS_PER_DAY
}

#[cfg(test)]
mod tests {
    use super::{MICROS_PER_DAY, TOMBSTONE_DAYS, TRASH_DAYS, TrashState, bin_cutoff_us, state};

    /// A moment in the middle of the range, so the arithmetic either side of it is ordinary.
    const NOW_US: i64 = 1_700_000_000_000_000;

    /// The moment something was thrown away, that many days before now.
    fn days_ago(days: i64) -> i64 {
        NOW_US - days * MICROS_PER_DAY
    }

    #[test]
    fn something_that_was_never_thrown_away_is_live() {
        assert_eq!(state(NOW_US, None, false), TrashState::Live);
    }

    #[test]
    fn something_already_destroyed_is_gone_whatever_the_bin_column_says() {
        assert_eq!(state(NOW_US, None, true), TrashState::Gone);
        assert_eq!(state(NOW_US, Some(days_ago(1)), true), TrashState::Gone);
    }

    #[test]
    fn twenty_three_hours_in_the_bin_is_no_days_at_all() {
        let almost_a_day = NOW_US - 23 * 60 * 60 * 1_000_000;

        assert_eq!(
            state(NOW_US, Some(almost_a_day), false),
            TrashState::InBin {
                days: 0,
                days_left: TRASH_DAYS
            }
        );
    }

    #[test]
    fn the_day_before_the_deadline_has_one_left() {
        assert_eq!(
            state(NOW_US, Some(days_ago(29)), false),
            TrashState::InBin {
                days: 29,
                days_left: 1
            }
        );
    }

    #[test]
    fn exactly_thirty_days_has_already_run_out() {
        // The promise is thirty days to get it back, not thirty-one. On the thirtieth day it is
        // over, which is also the only reading under which `days_left` is never zero.
        assert_eq!(
            state(NOW_US, Some(days_ago(30)), false),
            TrashState::Expired
        );
        assert_eq!(
            state(NOW_US, Some(days_ago(31)), false),
            TrashState::Expired
        );
    }

    #[test]
    fn a_hundred_and_eighty_days_is_still_only_expired() {
        // The tombstone period is not this function's business. Something six months in the bin
        // is exactly as expired as something thirty-one days in it.
        assert_eq!(
            state(NOW_US, Some(days_ago(TOMBSTONE_DAYS)), false),
            TrashState::Expired
        );
    }

    #[test]
    fn a_moment_from_a_clock_that_ran_ahead_is_treated_as_just_now() {
        let tomorrow = NOW_US + MICROS_PER_DAY;

        assert_eq!(
            state(NOW_US, Some(tomorrow), false),
            TrashState::InBin {
                days: 0,
                days_left: TRASH_DAYS
            }
        );
    }

    #[test]
    fn the_widest_possible_pair_of_moments_does_not_overflow() {
        assert_eq!(
            state(i64::MAX, Some(i64::MIN), false),
            TrashState::Expired,
            "the widest gap there is came out as something other than expired"
        );
        assert_eq!(
            state(i64::MIN, Some(i64::MAX), false),
            TrashState::InBin {
                days: 0,
                days_left: TRASH_DAYS
            }
        );
        let _cutoff = bin_cutoff_us(i64::MIN);
    }

    #[test]
    fn the_cutoff_and_the_state_agree_about_where_the_line_is() {
        let cutoff = bin_cutoff_us(NOW_US);

        assert_eq!(
            state(NOW_US, Some(cutoff - 1), false),
            TrashState::Expired,
            "something thrown away before the cutoff was not expired"
        );
        assert!(
            matches!(
                state(NOW_US, Some(cutoff + 1), false),
                TrashState::InBin { .. }
            ),
            "something thrown away after the cutoff was swept"
        );
    }

    #[test]
    fn the_bin_empties_long_before_the_mark_of_the_deletion_does() {
        // The test that looks silly and is the whole invariant. If the bin ever outlasted the
        // tombstone, the screen would be offering to restore rows whose contents the sweep had
        // already emptied, and every one of them would come back blank.
        const {
            assert!(
                TRASH_DAYS < TOMBSTONE_DAYS,
                "the bin outlives the mark that says something was deleted"
            );
        }
    }
}
