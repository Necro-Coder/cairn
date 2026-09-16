//! Turning a master password into a key.
//!
//! Argon2id, with parameters that belong to the vault rather than to the code. They live in
//! the header of the vault file, inside the part that the wrapped data key authenticates,
//! so somebody who can write to that file cannot lower them without the unwrapping failing.
//! That is the whole reason they are not a constant here.
//!
//! Because they are read from a file an attacker can write to, the parser has to treat them
//! as hostile. [`Argon2Params`] cannot be constructed out of range, and the range has a
//! ceiling as well as a floor: the floor stops somebody making a brute force attempt cheap,
//! and the ceiling stops a header claiming four gigabytes and killing the process on memory
//! before anything has been validated.
//!
//! Two details about the password itself are frozen here and must never change, because
//! changing either would leave every vault that already exists unopenable. It is normalised
//! to Unicode NFC before derivation, so that two visually identical passwords typed on
//! different keyboards produce the same key. And it is limited to a kilobyte at this
//! boundary, because Argon2id processes whatever it is given and a megabyte pasted into the
//! field is a denial of service with no attacker required.

use core::fmt;

use argon2::{Algorithm, Argon2, Params, Version};
use unicode_normalization::UnicodeNormalization as _;
use zeroize::Zeroizing;

use crate::error::CryptoError;
use crate::keys::{KEY_LEN, Kek};

/// Length of the salt that goes into Argon2id, in bytes.
pub const SALT_LEN: usize = 16;

/// Longest master password this accepts, in bytes of UTF-8.
///
/// Not a security limit. It is a bound on work: Argon2id hashes the whole password, so a
/// field somebody pasted a file into turns one unlock into minutes of processing. A
/// kilobyte is far beyond any password a person types and far below any length that costs
/// anything to hash.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// Least memory a vault may ask Argon2id for, in kibibytes.
///
/// Thirty-two mebibytes, which keeps a margin over the nineteen the current OWASP guidance
/// names as a minimum. Below this the header is refused rather than obeyed: a file an
/// attacker can write is a file an attacker can weaken, and the cheapest attack on this
/// design is to lower the cost of guessing.
pub const MIN_MEMORY_KIB: u32 = 32 * 1024;

/// Most memory a vault may ask Argon2id for, in kibibytes.
///
/// One gibibyte. This one is not about strength, it is about a header that claims four
/// gibibytes: the allocation happens before any password is checked, so without a ceiling a
/// twelve byte edit to the file kills the process every time it is opened.
pub const MAX_MEMORY_KIB: u32 = 1024 * 1024;

/// Fewest passes a vault may ask Argon2id for.
pub const MIN_PASSES: u32 = 3;

/// Most passes a vault may ask Argon2id for.
///
/// Time is bounded for the same reason memory is. A header claiming four billion passes
/// does not allocate anything; it simply never finishes, which from the outside is the same
/// thing as a crash and is harder to explain.
pub const MAX_PASSES: u32 = 16;

/// Most lanes a vault may ask Argon2id for.
///
/// The project uses one. Parallelism buys nothing against an attacker with a graphics card
/// and complicates the memory budget on a phone, so the range exists to bound a hostile
/// header rather than to offer a choice worth making.
pub const MAX_LANES: u32 = 4;

/// The parameters a vault was created with, guaranteed to be inside the allowed range.
///
/// There is no way to build one out of range, so every function downstream can take these
/// and do no checking of its own. That is the point of the type: the validation happens
/// once, where the bytes arrive, rather than at every place that might have forgotten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2Params {
    memory_kib: u32,
    passes: u32,
    lanes: u32,
}

impl Argon2Params {
    /// What a new vault is created with: sixty-four mebibytes, three passes, one lane.
    ///
    /// Deliberately above what is needed rather than below. The measurement on a phone
    /// happens in a later phase, and if it says this is too slow the parameters come down
    /// with an operation this phase proves does not re-encrypt a single record. Starting
    /// low and hoping to raise later would mean getting it right the first time or being
    /// stuck with it.
    pub const DEFAULT: Self = Self {
        memory_kib: 64 * 1024,
        passes: 3,
        lanes: 1,
    };

    /// Checks a set of parameters and refuses anything outside the allowed range.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::ParamOutOfRange`] naming the field, the value and the bound
    /// it broke. The value came from a file rather than from a person, so saying which
    /// number was wrong reveals nothing and saves an afternoon.
    pub const fn new(memory_kib: u32, passes: u32, lanes: u32) -> Result<Self, CryptoError> {
        if memory_kib < MIN_MEMORY_KIB || memory_kib > MAX_MEMORY_KIB {
            return Err(CryptoError::ParamOutOfRange {
                field: "argon2_m_kib",
                value: memory_kib,
                min: MIN_MEMORY_KIB,
                max: MAX_MEMORY_KIB,
            });
        }
        if passes < MIN_PASSES || passes > MAX_PASSES {
            return Err(CryptoError::ParamOutOfRange {
                field: "argon2_t",
                value: passes,
                min: MIN_PASSES,
                max: MAX_PASSES,
            });
        }
        if lanes == 0 || lanes > MAX_LANES {
            return Err(CryptoError::ParamOutOfRange {
                field: "argon2_p",
                value: lanes,
                min: 1,
                max: MAX_LANES,
            });
        }

        Ok(Self {
            memory_kib,
            passes,
            lanes,
        })
    }

    /// Memory cost, in kibibytes.
    #[must_use]
    pub const fn memory_kib(self) -> u32 {
        self.memory_kib
    }

    /// Number of passes over that memory.
    #[must_use]
    pub const fn passes(self) -> u32 {
        self.passes
    }

    /// Number of lanes.
    #[must_use]
    pub const fn lanes(self) -> u32 {
        self.lanes
    }
}

impl fmt::Display for Argon2Params {
    /// Reads the way the diagnostics screen shows it, in mebibytes rather than kibibytes.
    #[expect(
        clippy::integer_division,
        reason = "the remainder is meant to be dropped: this is a label on a diagnostics screen, and a memory size written to the nearest mebibyte is what somebody reading it expects"
    )]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "m={} MiB, t={}, p={}",
            self.memory_kib / 1024,
            self.passes,
            self.lanes
        )
    }
}

/// Derives the key encryption key from the master password.
///
/// The password is normalised to Unicode NFC first. Two strings that look identical on
/// screen can be different sequences of code points depending on the keyboard and the
/// operating system that produced them, and without this they would derive two different
/// keys and one of them would not open the vault.
///
/// # Errors
///
/// Returns [`CryptoError::PasswordTooLong`] if the password is over [`MAX_PASSWORD_BYTES`],
/// and [`CryptoError::Kdf`] if Argon2id itself refuses, which means the parameters could
/// not be turned into an allocation this machine can make.
pub fn derive_kek(
    password: &str,
    salt: &[u8; SALT_LEN],
    params: Argon2Params,
) -> Result<Kek, CryptoError> {
    // Measured before normalising rather than after. The bound is there to stop work, and
    // work that has already been done cannot be refused.
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(CryptoError::PasswordTooLong {
            len: password.len(),
            max: MAX_PASSWORD_BYTES,
        });
    }

    let normalised: Zeroizing<String> = Zeroizing::new(password.nfc().collect());

    let argon_params = Params::new(
        params.memory_kib,
        params.passes,
        params.lanes,
        Some(KEY_LEN),
    )
    .map_err(|_| CryptoError::Kdf)?;

    let mut derived = [0_u8; KEY_LEN];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params)
        .hash_password_into(normalised.as_bytes(), salt, &mut derived)
        .map_err(|_| CryptoError::Kdf)?;

    // `Kek::from_bytes` clears the array it is handed, so the derived copy does not outlive
    // this line.
    Ok(Kek::from_bytes(derived))
}

#[cfg(test)]
mod tests {
    use subtle::ConstantTimeEq as _;

    use super::{
        Argon2Params, MAX_LANES, MAX_MEMORY_KIB, MAX_PASSES, MAX_PASSWORD_BYTES, MIN_MEMORY_KIB,
        MIN_PASSES, SALT_LEN, derive_kek,
    };
    use crate::error::CryptoError;
    use crate::keys::Kek;

    const SALT: [u8; SALT_LEN] = [0x41; SALT_LEN];

    /// Cheap parameters, used wherever the test is about something other than cost.
    ///
    /// At the floor rather than at the default, because sixty-four mebibytes and three
    /// passes take most of a second and a suite that takes a minute is a suite people stop
    /// running. The floor is still a real Argon2id run.
    fn cheap() -> Argon2Params {
        Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, 1).unwrap()
    }

    #[test]
    fn the_defaults_are_the_numbers_they_are_documented_to_be() {
        // Frozen. These are written into every vault header ever created, and a silent
        // change to them is a silent change to how much a stolen file costs to attack.
        assert_eq!(Argon2Params::DEFAULT.memory_kib(), 65_536);
        assert_eq!(Argon2Params::DEFAULT.passes(), 3);
        assert_eq!(Argon2Params::DEFAULT.lanes(), 1);
    }

    #[test]
    fn the_bounds_are_the_numbers_they_are_documented_to_be() {
        assert_eq!(MIN_MEMORY_KIB, 32_768);
        assert_eq!(MAX_MEMORY_KIB, 1_048_576);
        assert_eq!(MIN_PASSES, 3);
        assert_eq!(MAX_PASSES, 16);
        assert_eq!(MAX_LANES, 4);
        assert_eq!(MAX_PASSWORD_BYTES, 1024);
    }

    #[test]
    fn every_bound_is_accepted_and_the_value_just_outside_it_is_not() {
        // Both sides of all five edges. An off by one here is either a vault weaker than it
        // claims or a vault that refuses parameters it wrote itself.
        assert!(Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, 1).is_ok());
        assert!(Argon2Params::new(MAX_MEMORY_KIB, MAX_PASSES, MAX_LANES).is_ok());

        assert!(matches!(
            Argon2Params::new(MIN_MEMORY_KIB - 1, MIN_PASSES, 1),
            Err(CryptoError::ParamOutOfRange {
                field: "argon2_m_kib",
                ..
            })
        ));
        assert!(matches!(
            Argon2Params::new(MAX_MEMORY_KIB + 1, MIN_PASSES, 1),
            Err(CryptoError::ParamOutOfRange {
                field: "argon2_m_kib",
                ..
            })
        ));
        assert!(matches!(
            Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES - 1, 1),
            Err(CryptoError::ParamOutOfRange {
                field: "argon2_t",
                ..
            })
        ));
        assert!(matches!(
            Argon2Params::new(MIN_MEMORY_KIB, MAX_PASSES + 1, 1),
            Err(CryptoError::ParamOutOfRange {
                field: "argon2_t",
                ..
            })
        ));
        assert!(matches!(
            Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, 0),
            Err(CryptoError::ParamOutOfRange {
                field: "argon2_p",
                ..
            })
        ));
        assert!(matches!(
            Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES + 1),
            Err(CryptoError::ParamOutOfRange {
                field: "argon2_p",
                ..
            })
        ));
    }

    #[test]
    fn zero_memory_is_refused_rather_than_treated_as_a_default() {
        assert!(Argon2Params::new(0, 0, 0).is_err());
    }

    #[test]
    fn the_parameters_print_the_way_the_diagnostics_screen_shows_them() {
        assert_eq!(Argon2Params::DEFAULT.to_string(), "m=64 MiB, t=3, p=1");
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Argon2id is memory hard on purpose, and the interpreter is several orders of magnitude slower than the processor; what Miri is here to check is the byte handling, not the hashing"
    )]
    fn the_same_password_and_salt_always_derive_the_same_key() {
        let first = derive_kek("una contrasena de ejemplo", &SALT, cheap()).unwrap();
        let second = derive_kek("una contrasena de ejemplo", &SALT, cheap()).unwrap();
        assert!(bool::from(first.ct_eq(&second)));
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Argon2id is memory hard on purpose, and the interpreter is several orders of magnitude slower than the processor; what Miri is here to check is the byte handling, not the hashing"
    )]
    fn a_different_password_derives_a_different_key() {
        let first = derive_kek("una contrasena de ejemplo", &SALT, cheap()).unwrap();
        let second = derive_kek("otra contrasena de ejemplo", &SALT, cheap()).unwrap();
        assert!(!bool::from(first.ct_eq(&second)));
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Argon2id is memory hard on purpose, and the interpreter is several orders of magnitude slower than the processor; what Miri is here to check is the byte handling, not the hashing"
    )]
    fn a_different_salt_derives_a_different_key() {
        let other_salt = [0x42; SALT_LEN];
        let first = derive_kek("una contrasena de ejemplo", &SALT, cheap()).unwrap();
        let second = derive_kek("una contrasena de ejemplo", &other_salt, cheap()).unwrap();
        assert!(!bool::from(first.ct_eq(&second)));
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Argon2id is memory hard on purpose, and the interpreter is several orders of magnitude slower than the processor; what Miri is here to check is the byte handling, not the hashing"
    )]
    fn different_parameters_derive_a_different_key() {
        let first = derive_kek("una contrasena de ejemplo", &SALT, cheap()).unwrap();
        let heavier = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES + 1, 1).unwrap();
        let second = derive_kek("una contrasena de ejemplo", &SALT, heavier).unwrap();
        assert!(!bool::from(first.ct_eq(&second)));
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Argon2id is memory hard on purpose, and the interpreter is several orders of magnitude slower than the processor; what Miri is here to check is the byte handling, not the hashing"
    )]
    fn two_spellings_of_the_same_password_derive_the_same_key() {
        // The decision that can never be revisited. These two strings look identical on
        // screen: one has a precomposed e with an acute accent, the other has a plain e
        // followed by a combining accent, and which one a keyboard produces depends on the
        // operating system. Without normalising, a vault created on one machine would refuse
        // the same password typed on another, and the person would have no way to tell.
        let precomposed = "caf\u{e9} con az\u{fa}car";
        let decomposed = "cafe\u{301} con azu\u{301}car";
        assert_ne!(
            precomposed, decomposed,
            "the two spellings must differ as text"
        );

        let from_precomposed = derive_kek(precomposed, &SALT, cheap()).unwrap();
        let from_decomposed = derive_kek(decomposed, &SALT, cheap()).unwrap();
        assert!(bool::from(from_precomposed.ct_eq(&from_decomposed)));
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Argon2id is memory hard on purpose, and the interpreter is several orders of magnitude slower than the processor; what Miri is here to check is the byte handling, not the hashing"
    )]
    fn it_is_the_composed_form_that_goes_into_argon2() {
        // The test above proves the two spellings agree. This proves which one they agree
        // on, and both are needed: a change that normalised to the decomposed form instead
        // would pass that test and would still leave every vault created before it
        // unopenable, with nothing to tell anybody why.
        use argon2::{Algorithm, Argon2, Params, Version};

        let composed = "caf\u{e9}";
        let decomposed = "cafe\u{301}";
        assert_eq!(
            composed.len(),
            5,
            "the composed form is five bytes of UTF-8"
        );
        assert_eq!(decomposed.len(), 6, "the decomposed form is six");

        let params = Params::new(MIN_MEMORY_KIB, MIN_PASSES, 1, Some(32)).unwrap();
        let mut direct = [0_u8; 32];
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(composed.as_bytes(), &SALT, &mut direct)
            .unwrap();

        let through_the_kdf = derive_kek(decomposed, &SALT, cheap()).unwrap();
        assert!(bool::from(through_the_kdf.ct_eq(&Kek::from_bytes(direct))));
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Argon2id is memory hard on purpose, and the interpreter is several orders of magnitude slower than the processor; what Miri is here to check is the byte handling, not the hashing"
    )]
    fn a_password_of_exactly_the_limit_is_accepted() {
        let at_the_limit = "a".repeat(MAX_PASSWORD_BYTES);
        assert!(derive_kek(&at_the_limit, &SALT, cheap()).is_ok());
    }

    #[test]
    fn a_password_one_byte_over_the_limit_is_refused() {
        // Refused before any hashing happens. The limit is a bound on work, so work already
        // done cannot be refused.
        let over = "a".repeat(MAX_PASSWORD_BYTES + 1);
        assert!(matches!(
            derive_kek(&over, &SALT, cheap()),
            Err(CryptoError::PasswordTooLong {
                len: 1025,
                max: 1024
            })
        ));
    }

    #[test]
    fn the_limit_counts_bytes_rather_than_characters() {
        // A string of characters that each take four bytes. Counting characters would let a
        // password through that is four times the work the limit intends to allow.
        let four_byte_characters = "\u{1f5ff}".repeat(MAX_PASSWORD_BYTES.div_ceil(4) + 1);
        assert!(matches!(
            derive_kek(&four_byte_characters, &SALT, cheap()),
            Err(CryptoError::PasswordTooLong { .. })
        ));
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Argon2id is memory hard on purpose, and the interpreter is several orders of magnitude slower than the processor; what Miri is here to check is the byte handling, not the hashing"
    )]
    fn an_empty_password_still_derives_a_key() {
        // Not a policy decision. Whether a password is long enough is decided by the layer
        // that talks to a person; this layer refuses nothing that it can process, so that
        // there is exactly one place where the rule lives.
        assert!(derive_kek("", &SALT, cheap()).is_ok());
    }

    /// The published Argon2id answer, from section five of the CFRG draft.
    ///
    /// Driven through the library directly rather than through [`derive_kek`], because the
    /// vector uses a secret key and associated data that this project does not, and thirty-
    /// two kibibytes of memory, which is a thousand times below the floor a vault is allowed
    /// to ask for.
    ///
    /// What it proves is the part that would otherwise be assumed: that the algorithm is
    /// Argon2id and not Argon2i, that the version is the current one and not the decade old
    /// one, and that this crate drives both the way the specification describes. Every one of
    /// those produces something that hashes perfectly well and is not what the documentation
    /// claims.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "Argon2id is memory hard on purpose, and the interpreter is several orders of magnitude slower than the processor; what Miri is here to check is the byte handling, not the hashing"
    )]
    fn the_library_matches_the_published_argon2id_vector() {
        use argon2::{Algorithm, Argon2, AssociatedData, ParamsBuilder, Version};

        /// The tag the specification prints for those inputs.
        const EXPECTED: [u8; 32] = [
            0x0d, 0x64, 0x0d, 0xf5, 0x8d, 0x78, 0x76, 0x6c, 0x08, 0xc0, 0x37, 0xa3, 0x4a, 0x8b,
            0x53, 0xc9, 0xd0, 0x1e, 0xf0, 0x45, 0x2d, 0x75, 0xb6, 0x5e, 0xb5, 0x25, 0x20, 0xe9,
            0x6b, 0x01, 0xe6, 0x59,
        ];

        let params = ParamsBuilder::new()
            .m_cost(32)
            .t_cost(3)
            .p_cost(4)
            .data(AssociatedData::new(&[0x04; 12]).unwrap())
            .build()
            .unwrap();

        let argon =
            Argon2::new_with_secret(&[0x03; 8], Algorithm::Argon2id, Version::V0x13, params)
                .unwrap();

        let mut tag = [0_u8; 32];
        argon
            .hash_password_into(&[0x01; 32], &[0x02; 16], &mut tag)
            .unwrap();

        assert_eq!(tag, EXPECTED);
    }
}
