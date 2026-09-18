//! Comparing two secrets without saying how far along they stopped matching.
//!
//! `==` on a slice, on a string or on an array is allowed to return the moment it finds a byte
//! that differs, and in practice it does. That is exactly right for a file name and exactly
//! wrong for anything somebody is trying to guess: the time the comparison took is a report on
//! how many leading bytes were correct, and a caller who can ask again with a different guess
//! can walk a secret out one byte at a time.
//!
//! This is the one door for that comparison in this project, for the same reason
//! [`crate::fill_random`] is the one door to the generator: the rule is worth a place to point
//! at rather than a habit. Anything outside this crate that has two secrets to compare calls
//! this.
//!
//! What it does not hide is the length. `subtle` refuses two slices of different lengths
//! before it looks at a byte of either, and hiding a length would mean reading past the end of
//! the shorter one. That is the right trade here, because nothing this compares has a secret
//! length: a token is always the same number of characters and so is a tag.

use subtle::ConstantTimeEq as _;

/// Whether two byte strings are the same, in time that does not depend on where they differ.
///
/// The answer is the same one `==` would give. What is different is that the comparison looks
/// at every byte of both whatever it finds, so the time it takes says nothing beyond the
/// lengths, which were never the secret.
///
/// ```
/// # use cairn_crypto::constant_time_eq;
/// assert!(constant_time_eq(b"the same", b"the same"));
/// assert!(!constant_time_eq(b"the same", b"the other"));
/// ```
#[must_use]
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    bool::from(left.ct_eq(right))
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn two_equal_byte_strings_match() {
        assert!(constant_time_eq(b"5f3a9c", b"5f3a9c"));
    }

    #[test]
    fn a_difference_at_the_end_is_found_as_surely_as_one_at_the_start() {
        // The property the timing makes invisible, asserted for the answer rather than for
        // the time: both of these are a mismatch and neither is more of one.
        assert!(!constant_time_eq(b"5f3a9c", b"af3a9c"));
        assert!(!constant_time_eq(b"5f3a9c", b"5f3a9d"));
    }

    #[test]
    fn a_prefix_is_not_a_match() {
        assert!(!constant_time_eq(b"5f3a9c", b"5f3a"));
        assert!(!constant_time_eq(b"5f3a", b"5f3a9c"));
    }

    #[test]
    fn two_empty_byte_strings_match() {
        assert!(constant_time_eq(b"", b""));
    }
}
