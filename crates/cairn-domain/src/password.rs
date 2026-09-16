//! What makes a master password acceptable, and how strong it looks.
//!
//! Two separate questions, deliberately. Acceptable is a rule and it blocks: below twelve
//! characters the vault is not created. Strong is an estimate and it never blocks: it is
//! shown so that somebody can decide for themselves, and a program refusing a password
//! because of its own guess at how good it is would be refusing on an opinion.
//!
//! There are no composition rules. Requiring an upper case letter, a digit and a symbol
//! makes real passwords worse rather than better, because people satisfy the rule in the
//! same handful of ways and the result is shorter and more predictable than what they would
//! have written on their own. Length is the thing that helps, so length is the thing
//! required.
//!
//! The estimate below is a heuristic written here rather than a dictionary attack library
//! pulled in from somewhere. The alternative would put several megabytes of word list into
//! the bundle and evaluate the master password in JavaScript, which is two large costs to
//! paint a coloured bar. A coarse band is what the bar can honestly show, and a coarse band
//! is what this computes, next to where the password has already been turned into bytes.

use cairn_crypto::MAX_PASSWORD_BYTES;

/// Fewest characters a master password may have.
///
/// Characters rather than bytes, because that is what somebody typing counts. The limit at
/// the other end is in bytes, because that one is about how much work Argon2id is asked to
/// do and work is measured in bytes.
pub const MIN_PASSWORD_CHARS: usize = 12;

/// Why a password was not accepted.
///
/// Only two reasons exist, and both are about size. Anything else would be this program
/// having an opinion about somebody's password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PasswordProblem {
    /// Shorter than [`MIN_PASSWORD_CHARS`].
    #[error("the password has {chars} characters, and at least {min} are needed")]
    TooShort {
        /// How many characters it has.
        chars: usize,
        /// How many are needed.
        min: usize,
    },

    /// Longer than the key derivation will accept.
    #[error("the password is {bytes} bytes, and at most {max} are accepted")]
    TooLong {
        /// How many bytes of UTF-8 it takes.
        bytes: usize,
        /// How many are accepted.
        max: usize,
    },
}

/// Checks that a master password is one the application will accept.
///
/// # Errors
///
/// Returns [`PasswordProblem::TooShort`] below [`MIN_PASSWORD_CHARS`] characters and
/// [`PasswordProblem::TooLong`] above what the key derivation accepts. Nothing else is
/// refused.
pub fn validate(password: &str) -> Result<(), PasswordProblem> {
    // Counted before the length in bytes, so that somebody who types eleven characters is
    // told they are short rather than being told nothing at all.
    let chars = password.chars().count();
    if chars < MIN_PASSWORD_CHARS {
        return Err(PasswordProblem::TooShort {
            chars,
            min: MIN_PASSWORD_CHARS,
        });
    }

    if password.len() > MAX_PASSWORD_BYTES {
        return Err(PasswordProblem::TooLong {
            bytes: password.len(),
            max: MAX_PASSWORD_BYTES,
        });
    }

    Ok(())
}

/// A coarse band, which is as much as an estimate of this kind can honestly claim.
///
/// Four steps rather than a percentage. A number with a decimal point in it looks like a
/// measurement, and this is not one: it is an arithmetic guess that knows nothing about
/// whether the password is a line from a song.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    /// Would not survive an attacker with the file and a word list.
    Weak,
    /// Better than most, and still not much.
    Fair,
    /// Reasonable against somebody who has the file.
    Good,
    /// More than this design needs.
    Strong,
}

/// How many different kinds of character are in use, as a rough alphabet size.
///
/// Everything outside the four familiar groups counts as one bucket rather than as the
/// whole of Unicode. Counting a single accented letter as tens of thousands of possible
/// characters would produce a wildly flattering number for a password that is one word with
/// an accent in it.
const OTHER_ALPHABET_SIZE: u32 = 40;

/// Estimates how strong a password looks.
///
/// The arithmetic is deliberately plain: an alphabet size from which kinds of character
/// appear, a length that stops rewarding repetition, and the product of the two in bits.
/// What it cannot see is meaning, so a long line of ordinary words scores well here and
/// would fall quickly to somebody with a word list. The interface says as much rather than
/// letting the bar speak for itself.
#[must_use]
pub fn strength(password: &str) -> Strength {
    let bits = estimated_bits(password);

    // Thresholds in the region where the guidance for a password behind a memory hard
    // derivation sits. They are round numbers on purpose: pretending to more precision
    // would be pretending this is a measurement.
    if bits < 40 {
        Strength::Weak
    } else if bits < 60 {
        Strength::Fair
    } else if bits < 80 {
        Strength::Good
    } else {
        Strength::Strong
    }
}

/// The estimate itself, in bits.
///
/// Integer arithmetic throughout. Money and measurements in this project are never floating
/// point, and there is no reason for a number that is shown as one of four words to be the
/// exception.
fn estimated_bits(password: &str) -> u32 {
    let mut has_lower = false;
    let mut has_upper = false;
    let mut has_digit = false;
    let mut has_ascii_symbol = false;
    let mut has_other = false;

    let mut distinct: Vec<char> = Vec::new();
    let mut length: u32 = 0;

    for character in password.chars() {
        length = length.saturating_add(1);
        if !distinct.contains(&character) {
            distinct.push(character);
        }

        if character.is_ascii_lowercase() {
            has_lower = true;
        } else if character.is_ascii_uppercase() {
            has_upper = true;
        } else if character.is_ascii_digit() {
            has_digit = true;
        } else if character.is_ascii_graphic() || character == ' ' {
            has_ascii_symbol = true;
        } else {
            has_other = true;
        }
    }

    let alphabet = u32::from(has_lower) * 26
        + u32::from(has_upper) * 26
        + u32::from(has_digit) * 10
        + u32::from(has_ascii_symbol) * 33
        + u32::from(has_other) * OTHER_ALPHABET_SIZE;

    // The only input with no alphabet at all is the empty one, and the logarithm below is
    // not defined at zero. Every other password sets at least one of the flags above, and
    // the smallest group has ten members.
    if length == 0 {
        return 0;
    }

    // Repetition earns nothing after the first couple of times. Without this, a password of
    // twelve identical letters would score the same as twelve different ones, which is the
    // single most obvious way for an estimate like this to be wrong.
    let distinct_count = u32::try_from(distinct.len()).unwrap_or(u32::MAX);
    let effective_length = length.min(distinct_count.saturating_mul(2));

    // The integer logarithm rounds down, which makes every answer here a slight
    // underestimate. That is the direction to be wrong in.
    effective_length.saturating_mul(alphabet.ilog2())
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PASSWORD_BYTES, MIN_PASSWORD_CHARS, PasswordProblem, Strength, estimated_bits,
        strength, validate,
    };

    /// Every band boundary, from just below to exactly on it.
    ///
    /// The thresholds are the only numbers in this file that decide what somebody is shown,
    /// and an estimate that reads Weak at exactly forty bits is a different program from one
    /// that reads Fair. Lower case letters give an alphabet of twenty-six, whose integer
    /// logarithm is four, so each character is worth exactly four bits and the boundaries
    /// land on whole passwords.
    #[test]
    fn each_band_starts_exactly_where_it_says_it_does() {
        assert_eq!(estimated_bits("abcdefghi"), 36);
        assert_eq!(strength("abcdefghi"), Strength::Weak);

        assert_eq!(estimated_bits("abcdefghij"), 40);
        assert_eq!(strength("abcdefghij"), Strength::Fair);

        assert_eq!(estimated_bits("abcdefghijklmn"), 56);
        assert_eq!(strength("abcdefghijklmn"), Strength::Fair);

        assert_eq!(estimated_bits("abcdefghijklmno"), 60);
        assert_eq!(strength("abcdefghijklmno"), Strength::Good);

        assert_eq!(estimated_bits("abcdefghijklmnopqrs"), 76);
        assert_eq!(strength("abcdefghijklmnopqrs"), Strength::Good);

        assert_eq!(estimated_bits("abcdefghijklmnopqrst"), 80);
        assert_eq!(strength("abcdefghijklmnopqrst"), Strength::Strong);
    }

    /// Each class of character contributes exactly what the alphabet says it does.
    ///
    /// Written as exact numbers rather than as comparisons between two passwords, because a
    /// comparison stays true when both sides move together and these are the arithmetic the
    /// whole estimate rests on.
    #[test]
    fn each_kind_of_character_is_worth_what_the_alphabet_says() {
        // Digits alone: an alphabet of ten, whose integer logarithm is three.
        assert_eq!(estimated_bits("123456789012"), 36);

        // Upper case alone: twenty-six, the same as lower case, so four bits each.
        assert_eq!(estimated_bits("ABCDEFGHIJKL"), 48);

        // Both cases: fifty-two, whose integer logarithm is five.
        assert_eq!(estimated_bits("aBcDeFgHiJkL"), 60);

        // A space counts as an ASCII symbol, not as something exotic: twenty-six and
        // thirty-three make fifty-nine, whose integer logarithm is five, where counting it
        // among the other characters would make sixty-six and six bits each.
        assert_eq!(estimated_bits("hola que tal"), 60);

        // Something outside the four familiar groups: a single bucket of forty.
        assert_eq!(estimated_bits("🗿🗿🗿🗿"), 10);
    }

    #[test]
    fn the_minimum_is_the_number_it_is_documented_to_be() {
        assert_eq!(MIN_PASSWORD_CHARS, 12);
    }

    #[test]
    fn the_maximum_is_the_one_the_key_derivation_enforces() {
        // Not a second opinion about the limit, the same one. Two crates disagreeing about
        // this would mean a password the interface accepted and the core refused, which is
        // the worst possible moment to find out.
        assert_eq!(MAX_PASSWORD_BYTES, cairn_crypto::MAX_PASSWORD_BYTES);
    }

    #[test]
    fn a_password_of_exactly_the_minimum_is_accepted() {
        assert!(validate("abcdefghijkl").is_ok());
    }

    #[test]
    fn a_password_one_character_short_is_refused() {
        assert_eq!(
            validate("abcdefghijk"),
            Err(PasswordProblem::TooShort {
                chars: 11,
                min: MIN_PASSWORD_CHARS
            })
        );
    }

    #[test]
    fn an_empty_password_is_refused_as_short_rather_than_as_something_else() {
        assert_eq!(
            validate(""),
            Err(PasswordProblem::TooShort {
                chars: 0,
                min: MIN_PASSWORD_CHARS
            })
        );
    }

    #[test]
    fn the_minimum_counts_characters_rather_than_bytes() {
        // Twelve characters that take more than twelve bytes. Counting bytes here would
        // accept a password of four accented letters, which is not what anybody typing
        // twelve characters would expect the rule to mean.
        let twelve_accented =
            "\u{e1}\u{e9}\u{ed}\u{f3}\u{fa}\u{e1}\u{e9}\u{ed}\u{f3}\u{fa}\u{e1}\u{e9}";
        assert_eq!(twelve_accented.chars().count(), 12);
        assert!(twelve_accented.len() > 12);
        assert!(validate(twelve_accented).is_ok());
    }

    #[test]
    fn a_password_of_exactly_the_maximum_is_accepted() {
        assert!(validate(&"a".repeat(MAX_PASSWORD_BYTES)).is_ok());
    }

    #[test]
    fn a_password_one_byte_over_the_maximum_is_refused() {
        assert_eq!(
            validate(&"a".repeat(MAX_PASSWORD_BYTES + 1)),
            Err(PasswordProblem::TooLong {
                bytes: MAX_PASSWORD_BYTES + 1,
                max: MAX_PASSWORD_BYTES
            })
        );
    }

    #[test]
    fn the_maximum_counts_bytes_rather_than_characters() {
        // The mirror of the test above, and the reason the two limits count different
        // things. This is about how much work Argon2id is asked to do, and work is bytes.
        let long = "\u{1f5ff}".repeat(MAX_PASSWORD_BYTES.div_ceil(4) + 1);
        assert!(long.chars().count() < MAX_PASSWORD_BYTES);
        assert!(matches!(
            validate(&long),
            Err(PasswordProblem::TooLong { .. })
        ));
    }

    #[test]
    fn nothing_but_length_is_ever_refused() {
        // No composition rules, stated as a test so that adding one has to be deliberate.
        for password in [
            "solo en minusculas y nada mas",
            "SOLO EN MAYUSCULAS Y NADA MAS",
            "111111111111111111111111",
            "                        ",
            "!!!!!!!!!!!!!!!!!!!!!!!!",
            "\u{1f5ff}\u{1f5ff}\u{1f5ff}\u{1f5ff}\u{1f5ff}\u{1f5ff}\u{1f5ff}\u{1f5ff}\u{1f5ff}\u{1f5ff}\u{1f5ff}\u{1f5ff}",
        ] {
            assert!(
                validate(password).is_ok(),
                "a password was refused for something other than its length: {password}"
            );
        }
    }

    #[test]
    fn a_password_that_is_refused_still_has_a_strength_to_show() {
        // The two are independent on purpose. The bar moves while somebody is still typing,
        // long before the password is long enough to be accepted.
        assert!(validate("abc").is_err());
        assert_eq!(strength("abc"), Strength::Weak);
    }

    #[test]
    fn repetition_earns_almost_nothing() {
        // The most obvious way for an estimate like this to be wrong: twelve identical
        // letters scoring the same as twelve different ones.
        let repeated = "aaaaaaaaaaaaaaaaaaaaaaaa";
        let varied = "abcdefghijklmnopqrstuvwx";

        let repeated_bits = estimated_bits(repeated);
        let varied_bits = estimated_bits(varied);
        assert!(
            repeated_bits < varied_bits,
            "{repeated_bits} was not scored below {varied_bits}"
        );
        assert_eq!(strength(repeated), Strength::Weak);
    }

    #[test]
    fn one_letter_over_and_over_scores_almost_nothing() {
        // The alphabet is the kinds of character in use, so this counts as lower case
        // letters. What keeps the score low is the cap on repetition, not the alphabet.
        assert!(estimated_bits("aaaa") <= 8);
        assert_eq!(strength("aaaa"), Strength::Weak);
    }

    #[test]
    fn an_empty_password_scores_nothing_rather_than_dividing_by_something() {
        assert_eq!(estimated_bits(""), 0);
        assert_eq!(strength(""), Strength::Weak);
    }

    #[test]
    fn the_bands_land_where_the_thresholds_say() {
        // One password per band, with the boundary checked rather than assumed.
        let cases = [
            ("abcdef", Strength::Weak),
            ("abcdefgh1", Strength::Fair),
            ("abcdefghijklm1", Strength::Good),
            ("abcdefghijklmnopqrs1", Strength::Strong),
        ];

        for (password, expected) in cases {
            let landed = strength(password);
            let bits = estimated_bits(password);
            assert_eq!(
                landed, expected,
                "{password} scored {bits} bits and landed in {landed:?}"
            );
        }
    }

    #[test]
    fn every_band_is_reachable() {
        // Guards the thresholds. A band no password can reach is a band that should not be
        // in the enumeration.
        let reached: Vec<Strength> = ["aa1", "abcdefgh1", "abcdefghijklm1", "abcdefghijklmnopqrs1"]
            .into_iter()
            .map(strength)
            .collect();

        assert_eq!(
            reached,
            vec![
                Strength::Weak,
                Strength::Fair,
                Strength::Good,
                Strength::Strong
            ]
        );
    }

    #[test]
    fn more_kinds_of_character_score_higher_at_the_same_length() {
        let letters = "abcdefghijklmnop";
        let mixed = "aBcD3fgH!jklMn0p";
        assert_eq!(letters.chars().count(), mixed.chars().count());
        assert!(estimated_bits(mixed) > estimated_bits(letters));
    }

    #[test]
    fn characters_outside_the_familiar_groups_are_counted_modestly() {
        // A password that is one word with an accent in it must not score as though it were
        // drawn from the whole of Unicode.
        let plain = "contrasenaxyz";
        let accented = "contrase\u{f1}axyz";
        assert!(estimated_bits(accented) > estimated_bits(plain));
        assert!(estimated_bits(accented) < estimated_bits(plain) * 2);
    }

    #[test]
    fn the_bands_are_ordered() {
        assert!(Strength::Weak < Strength::Fair);
        assert!(Strength::Fair < Strength::Good);
        assert!(Strength::Good < Strength::Strong);
    }

    #[test]
    fn a_long_password_made_of_one_repeating_block_is_not_called_strong() {
        // A thousand characters, and four of them repeated two hundred and fifty-six times.
        // An estimate that rewarded raw length would call this the strongest password it had
        // ever seen; the cap on repetition is what stops it.
        let repeating_block = "a1B!".repeat(MAX_PASSWORD_BYTES.div_ceil(4));
        assert_eq!(repeating_block.chars().count(), MAX_PASSWORD_BYTES);
        assert!(strength(&repeating_block) < Strength::Strong);
    }

    #[test]
    fn the_longest_password_the_validator_accepts_does_not_overflow_the_estimate() {
        // Saturating arithmetic all the way through. Overflow checks are on in release
        // builds, so a wraparound here would abort the process while somebody was typing.
        let longest: String = (0..MAX_PASSWORD_BYTES)
            .map(|index| {
                let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!?";
                char::from(*alphabet.get(index % alphabet.len()).unwrap_or(&b'a'))
            })
            .collect();

        assert_eq!(longest.len(), MAX_PASSWORD_BYTES);
        assert!(validate(&longest).is_ok());
        assert_eq!(strength(&longest), Strength::Strong);
    }

    #[test]
    fn the_problems_say_what_went_wrong() {
        // The messages reach a person, through the interface, so they are worth a test.
        assert_eq!(
            validate("corto").unwrap_err().to_string(),
            "the password has 5 characters, and at least 12 are needed"
        );
        assert_eq!(
            validate(&"a".repeat(2000)).unwrap_err().to_string(),
            "the password is 2000 bytes, and at most 1024 are accepted"
        );
    }
}
