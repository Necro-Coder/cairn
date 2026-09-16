//! The one door to the random number generator of the operating system.
//!
//! Two things in this crate need randomness and both of them are things that must never be
//! guessable: a key and a nonce. Neither has a fallback. If the operating system will not
//! answer, the operation fails, because bytes from a lesser source produce a key that looks
//! exactly like a good one and is not.
//!
//! The indirection exists so that the failure has a test. An error path nobody has ever run
//! is an error path nobody knows works, and this particular one is the difference between
//! refusing to create a vault and creating one with a predictable key. Under test, and only
//! under test, a call can be told to fail, and the failure is confined to the thread that
//! asked for it so that the rest of the suite carries on unaffected.

use zeroize::Zeroize as _;

use crate::error::CryptoError;

/// Fills the buffer with bytes from the operating system.
///
/// On failure the buffer is cleared before returning, so that a caller which ignores the
/// error, and there should never be one, is left with zeroes rather than with whatever the
/// half finished read put there.
///
/// # Errors
///
/// Returns [`CryptoError::Entropy`] if the operating system refuses.
pub(crate) fn fill(destination: &mut [u8]) -> Result<(), CryptoError> {
    if was_refused(destination) {
        destination.zeroize();
        return Err(CryptoError::Entropy);
    }

    Ok(())
}

/// Reads the bytes and answers whether the operating system refused.
///
/// Split out so that there is exactly one place where a refusal turns into an error. An
/// injected fault that returned early from [`fill`] would leave the real refusal on a
/// branch of its own that no test ever runs, which is the opposite of what the injection
/// is for.
fn was_refused(destination: &mut [u8]) -> bool {
    #[cfg(test)]
    if fault::should_fail() {
        return true;
    }

    getrandom::fill(destination).is_err()
}

/// Lets a test make the next read fail.
///
/// Thread local rather than global, so that a test arming the fault cannot affect the
/// other tests running beside it. Armed for exactly one call and disarmed by the call it
/// broke, so a test cannot leave the fault switched on for whatever runs next on the same
/// thread.
#[cfg(test)]
pub(crate) mod fault {
    use core::cell::Cell;

    thread_local! {
        static ARMED: Cell<bool> = const { Cell::new(false) };
    }

    /// Makes the next read of randomness on this thread fail.
    pub(crate) fn arm() {
        ARMED.with(|armed| armed.set(true));
    }

    /// Whether the next read should fail, clearing the arming as it answers.
    pub(crate) fn should_fail() -> bool {
        ARMED.with(Cell::take)
    }
}

#[cfg(test)]
mod tests {
    use super::{fault, fill};
    use crate::error::CryptoError;

    #[test]
    fn a_normal_read_fills_the_buffer() {
        let mut buffer = [0_u8; 32];
        fill(&mut buffer).unwrap();
        assert_ne!(buffer, [0_u8; 32]);
    }

    #[test]
    fn an_armed_fault_makes_the_next_read_fail() {
        let mut buffer = [0xff_u8; 32];
        fault::arm();
        assert!(matches!(fill(&mut buffer), Err(CryptoError::Entropy)));
    }

    #[test]
    fn a_failed_read_leaves_no_partial_bytes_behind() {
        // The caller should be propagating the error rather than looking at the buffer.
        // This is what happens if one of them does not.
        let mut buffer = [0xff_u8; 32];
        fault::arm();
        let _ = fill(&mut buffer);
        assert_eq!(buffer, [0_u8; 32]);
    }

    #[test]
    fn the_fault_only_affects_one_call() {
        let mut buffer = [0_u8; 32];
        fault::arm();
        assert!(fill(&mut buffer).is_err());
        fill(&mut buffer).unwrap();
        assert_ne!(buffer, [0_u8; 32]);
    }
}
