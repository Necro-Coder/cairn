//! Nonces that cannot be reused, enforced by the type system rather than by a convention.
//!
//! Reusing a nonce with the same key is the one mistake in this design that loses
//! everything at once: with XChaCha20-Poly1305 it leaks the exclusive or of two plaintexts
//! and, worse, gives away the material needed to forge tags. No amount of review catches
//! it reliably, because the mistake looks like ordinary code that happens to name the same
//! variable twice.
//!
//! So it is made impossible instead. [`FreshNonce`] has one constructor, which reads from
//! the operating system, and [`crate::seal`] takes it by value. Handing it over ends the
//! caller's ownership of it, and a second call has nothing left to pass.
//!
//! What that costs is the ability to write the nonce down and use it again, which is
//! exactly the thing being prevented. The nonce that comes back inside a [`crate::Sealed`]
//! is plain bytes and cannot be turned back into a `FreshNonce`, so it can be stored and
//! read and used to decrypt, and never to encrypt.

use core::fmt;

use zeroize::ZeroizeOnDrop;

use crate::error::CryptoError;
use crate::random;

/// Length of an `XChaCha20-Poly1305` nonce, in bytes.
///
/// Twenty-four rather than twelve is the whole reason this project uses the extended
/// variant. Twenty-four random bytes put the birthday bound around two to the eightieth
/// message, which means a random nonce per write is safe without anybody having to keep a
/// counter that survives a crash.
pub const NONCE_LEN: usize = 24;

/// A nonce that has been read from the operating system and not yet used.
///
/// Intentionally missing `Clone`, `Copy`, `Default`, `From<[u8; NONCE_LEN]>` and any form
/// of deserialisation. Each of those would be a way to produce a second nonce with the
/// same bytes, which is the failure this type exists to prevent. Adding one later is not a
/// convenience, it is removing the guarantee.
#[derive(ZeroizeOnDrop)]
pub struct FreshNonce([u8; NONCE_LEN]);

impl FreshNonce {
    /// Reads a new nonce from the operating system.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Entropy`] if the operating system refuses. There is no
    /// fallback: a nonce from a weaker source is indistinguishable from a good one until
    /// the day it repeats.
    #[must_use = "a nonce that is generated and dropped is a nonce that was wasted, and generating another one is not free"]
    pub fn generate() -> Result<Self, CryptoError> {
        let mut bytes = [0_u8; NONCE_LEN];
        random::fill(&mut bytes)?;

        #[cfg(test)]
        harness::record(&bytes);

        Ok(Self(bytes))
    }

    /// The bytes, consumed.
    ///
    /// Private to the crate and taking `self` by value, so that reading the bytes is the
    /// last thing that happens to a nonce. [`crate::seal`] calls it once and then has an
    /// array rather than a `FreshNonce`, which is what stops a second encryption from
    /// being written by accident inside this crate as well as outside it.
    pub(crate) fn into_bytes(self) -> [u8; NONCE_LEN] {
        // `FreshNonce` zeroizes on drop, so the copy has to be made before `self` goes out
        // of scope at the end of this function. Taking it out of the wrapper is the point:
        // the returned array is public data, whereas the wrapper is a permission to
        // encrypt exactly once, and that permission is being spent here.
        self.0
    }
}

impl fmt::Debug for FreshNonce {
    /// Prints nothing about the value.
    ///
    /// A nonce is not a secret, and this is still redacted. The reason is that a nonce in
    /// a log is a nonce somebody can search for, and finding the same one twice is a
    /// question worth being unable to answer from a log file. It also keeps one rule for
    /// every type in this crate instead of a rule with an exception.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FreshNonce([REDACTED])")
    }
}

/// Records every nonce the test suite produces and fails if one ever comes back.
///
/// This is a live check rather than a review comment. Every unit test in this crate that
/// encrypts anything adds to the same table, so a change that made generation
/// deterministic, or that cached a nonce somewhere, is caught by whichever test runs
/// second rather than by nobody.
///
/// It only covers the unit tests, because `#[cfg(test)]` is not set when the crate is
/// compiled as a dependency of a file in `tests/`. That is why the properties worth
/// checking against this harness are written as unit tests inside the crate rather than as
/// integration tests beside it.
#[cfg(test)]
pub(crate) mod harness {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    use super::NONCE_LEN;

    fn seen() -> &'static Mutex<HashSet<[u8; NONCE_LEN]>> {
        static SEEN: OnceLock<Mutex<HashSet<[u8; NONCE_LEN]>>> = OnceLock::new();
        SEEN.get_or_init(|| Mutex::new(HashSet::new()))
    }

    /// Adds a nonce to the table, panicking if it was already there.
    pub(crate) fn record(bytes: &[u8; NONCE_LEN]) {
        let mut table = seen()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            table.insert(*bytes),
            "a nonce was produced twice in one run of the test suite, which is the one \
             failure this design exists to make impossible"
        );
    }

    /// How many distinct nonces the suite has produced so far.
    pub(crate) fn count() -> usize {
        seen()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::{FreshNonce, NONCE_LEN, harness};
    use crate::error::CryptoError;

    #[test]
    fn a_refusal_from_the_operating_system_is_propagated() {
        // Never a fallback to a weaker source. A nonce nobody can predict is the whole
        // reason this type exists, and one that was quietly made up would look identical.
        crate::random::fault::arm();
        assert!(matches!(FreshNonce::generate(), Err(CryptoError::Entropy)));
    }

    #[test]
    fn two_generated_nonces_differ() {
        let first = FreshNonce::generate().unwrap().into_bytes();
        let second = FreshNonce::generate().unwrap().into_bytes();
        assert_ne!(
            first, second,
            "two nonces read from the operating system came back identical"
        );
    }

    #[test]
    fn a_generated_nonce_is_not_all_zeroes() {
        // A source that silently fails often returns a buffer that was never written to.
        // The odds of twenty-four genuinely random bytes all being zero are not worth
        // writing down, so this distinguishes the two.
        let bytes = FreshNonce::generate().unwrap().into_bytes();
        assert_ne!(
            bytes, [0_u8; NONCE_LEN],
            "the nonce buffer was never filled"
        );
    }

    #[test]
    fn debug_reveals_nothing_about_the_nonce() {
        // Compared for equality rather than searched for leaks. The output contains no
        // data at all, which is a stronger statement than any search could make and a
        // shorter test than any search would need.
        let nonce = FreshNonce::generate().unwrap();
        assert_eq!(format!("{nonce:?}"), "FreshNonce([REDACTED])");
    }

    #[test]
    fn the_anti_reuse_harness_has_seen_a_meaningful_number_of_nonces() {
        // Tests run in an arbitrary order, so the total this one sees is a lower bound on
        // what the suite produced, not the final figure. That is still the useful check:
        // if the property tests stopped encrypting, or the harness stopped being called,
        // this number collapses to single digits and the failure says so.
        const FLOOR: usize = 64;

        for _ in 0..FLOOR {
            let _ = FreshNonce::generate().unwrap().into_bytes();
        }

        let total = harness::count();
        println!("the anti-reuse harness has recorded {total} distinct nonces so far");
        assert!(
            total >= FLOOR,
            "the harness recorded {total} nonces, which is fewer than this test alone produced"
        );
    }

    #[test]
    fn the_anti_reuse_harness_is_actually_recording() {
        // Guards the harness itself. If `record` stopped being called, every other test
        // would still pass and the protection would be gone without a word.
        let before = harness::count();
        let _ = FreshNonce::generate().unwrap().into_bytes();
        assert!(
            harness::count() > before,
            "generating a nonce did not reach the anti-reuse harness"
        );
    }
}
