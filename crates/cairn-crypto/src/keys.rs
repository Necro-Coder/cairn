//! The thirty-two byte keys this crate deals in, and what keeps them apart.
//!
//! [`Kek`] is what Argon2id produces from the master password. It encrypts exactly one
//! thing, the wrapped data key, and it is thrown away the moment the vault locks.
//!
//! [`DataKey`] is what [`crate::seal`] encrypts with: the data encryption key, and the
//! subkeys derived for wrapping and for exports, because to the cipher they are the same
//! thing.
//!
//! [`DatabaseKey`] and [`SyncKey`] are derived too, and are deliberately not data keys.
//! One is handed to SQLCipher and never goes near this cipher; the other is the only key in
//! the hierarchy that is used over a network. Domain separation that exists only in the
//! derivation is a convention; separating them by type is the compiler enforcing it.
//!
//! All of them are the same thing underneath, and all of them keep that thing private. Bytes go in,
//! bytes come out only inside this crate, `Debug` says nothing, and equality is a constant
//! time comparison because the ordinary one returns early on the first differing byte and
//! how long that took is a measurement of how much of the key was guessed right.

use core::fmt;

use secrecy::{ExposeSecret as _, SecretBox};
use subtle::{Choice, ConstantTimeEq};
use zeroize::Zeroize as _;

use crate::error::CryptoError;
use crate::random;

/// Length of every key in this crate, in bytes.
pub const KEY_LEN: usize = 32;

/// A thirty-two byte secret on the heap, zeroized when it is dropped.
///
/// Private, and wrapped by the public types below rather than being one of them. Every key
/// in this crate is interchangeable as bytes and none of them may be interchangeable as
/// values: passing the key that unwraps the vault where the key that encrypts records
/// belongs is a mistake the compiler can catch, and it only can if they are different
/// types.
struct Key32(SecretBox<[u8; KEY_LEN]>);

impl Key32 {
    /// Takes ownership of the bytes and clears the caller copy.
    ///
    /// The argument is taken by value and zeroized before returning, so that a key handed
    /// in from a derivation buffer does not survive in that buffer as well. It is the
    /// array belonging to the caller that is being cleared, which is why it has to arrive
    /// by value and be mutable.
    fn from_bytes(mut bytes: [u8; KEY_LEN]) -> Self {
        let key = Self(SecretBox::new(Box::new(bytes)));
        bytes.zeroize();
        key
    }

    /// Reads a new key from the operating system.
    fn generate() -> Result<Self, CryptoError> {
        let mut bytes = [0_u8; KEY_LEN];
        random::fill(&mut bytes)?;
        Ok(Self::from_bytes(bytes))
    }

    fn expose(&self) -> &[u8; KEY_LEN] {
        self.0.expose_secret()
    }
}

impl ConstantTimeEq for Key32 {
    fn ct_eq(&self, other: &Self) -> Choice {
        self.expose().ct_eq(other.expose())
    }
}

/// The key encryption key: what Argon2id produces from the master password.
///
/// It encrypts one thing, the wrapped data key, and it changes whenever the password or the
/// derivation parameters change. Nothing else in the system hangs off it, which is why
/// changing the password does not re-encrypt a single record.
///
/// Has no `Clone`, no `Copy` and no `Default`. A key that can be duplicated is a key whose
/// copies have to be tracked, and the whole value of zeroizing on drop is that there is one
/// place to drop.
pub struct Kek(Key32);

/// A key that goes into the cipher.
///
/// The data encryption key is one. So is the subkey that wraps it, and so is the subkey
/// that encrypts an export, because to the cipher they are the same thing and separating
/// them by name rather than by derivation would be decoration.
///
/// Has no `Clone`, no `Copy` and no `Default`, for the same reason as [`Kek`].
pub struct DataKey(Key32);

/// The raw key SQLCipher is given for the whole database file.
///
/// A type of its own rather than another [`DataKey`], because it never goes near the
/// cipher in this crate and handing it to [`crate::seal`] would be a mistake worth making
/// impossible. It hangs off the data key, not off the key encryption key, which is what
/// makes changing the master password a hundred and sixty-eight byte rewrite instead of
/// re-encrypting the database.
pub struct DatabaseKey(Key32);

/// The pre-shared key for the synchronisation handshake.
///
/// Also its own type, and for a stronger reason: it is the only key in the hierarchy that
/// is used over a network. Confusing it with the key that encrypts records at rest would
/// put the contents of the vault on the wire, so the compiler is made to refuse.
pub struct SyncKey(Key32);

/// Gives a key type the two things every key type needs and nothing else.
///
/// The types themselves are written out above rather than produced here, so that searching
/// for `struct DataKey` finds it. What the macro generates is only the part that would
/// otherwise be four identical copies, which is four places for one of them to quietly grow
/// a `Clone`, or a `Debug` that prints something.
///
/// Construction is deliberately not in here. Two of these keys are roots and are built from
/// bytes or read from the system; the other two are derived and must only ever come out of
/// the derivation. Generating a database key at random would produce something that works
/// perfectly until the next time the vault is opened.
macro_rules! impl_key {
    ($name:ident) => {
        impl $name {
            /// Wraps bytes that came out of a derivation, clearing the caller copy.
            ///
            /// Private to the crate: every key type is constructible here because the
            /// derivation lives here, and nowhere else, because nowhere else should be
            /// inventing key material.
            pub(crate) fn from_derived(bytes: [u8; KEY_LEN]) -> Self {
                Self(Key32::from_bytes(bytes))
            }
        }

        impl fmt::Debug for $name {
            /// Prints the type name and nothing else.
            ///
            /// Any derived `Debug` on a struct that happens to hold one of these would
            /// otherwise print the key, and that struct is usually the one somebody logs
            /// while chasing an unrelated bug.
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([REDACTED])"))
            }
        }

        impl ConstantTimeEq for $name {
            /// Compares in time that does not depend on where the first difference is.
            ///
            /// There is no `PartialEq` on any key type, so `==` does not compile on one.
            /// That is deliberate: the ordinary comparison returns as soon as two bytes
            /// differ, and how long it took is a measurement of how much was guessed right.
            fn ct_eq(&self, other: &Self) -> Choice {
                self.0.ct_eq(&other.0)
            }
        }
    };
}

impl_key!(Kek);
impl_key!(DataKey);
impl_key!(DatabaseKey);
impl_key!(SyncKey);

impl Kek {
    /// The key bytes, readable only inside this crate.
    ///
    /// Crate private, and staying that way. This key and the data key are the two that must
    /// never leave, because between them they open everything, and the hardest rule in the
    /// project is that no key and no decrypted text crosses into JavaScript. Keeping the
    /// accessor here means the compiler enforces that rather than a reviewer remembering it.
    pub(crate) fn expose(&self) -> &[u8; KEY_LEN] {
        self.0.expose()
    }

    /// Takes ownership of bytes that came out of Argon2id, clearing the caller copy.
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self::from_derived(bytes)
    }
}

impl DataKey {
    /// The key bytes, readable only inside this crate. See [`Kek::expose`].
    pub(crate) fn expose(&self) -> &[u8; KEY_LEN] {
        self.0.expose()
    }

    /// Takes ownership of the bytes and clears the caller copy.
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self::from_derived(bytes)
    }

    /// Reads a new data key from the operating system.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Entropy`] if the operating system refuses. There is no
    /// fallback, because a key from a weaker source looks exactly like a good one right up
    /// until somebody predicts it.
    pub fn generate() -> Result<Self, CryptoError> {
        Ok(Self(Key32::generate()?))
    }
}

impl DatabaseKey {
    /// The key bytes, which SQLCipher has to be given.
    ///
    /// The one derived key with a public accessor, and it is public because the database
    /// layer is a different crate and cannot open the file without these thirty-two bytes.
    /// That does not weaken the rule it looks like an exception to: the rule is that no key
    /// crosses into JavaScript, and this one is handed from one Rust crate to another and
    /// never reaches a command.
    #[must_use]
    pub fn expose(&self) -> &[u8; KEY_LEN] {
        self.0.expose()
    }
}

impl SyncKey {
    /// The key bytes, which the handshake has to be given.
    ///
    /// Public for the same reason as [`DatabaseKey::expose`]: the code that uses it is a
    /// different crate. It is the only key in the hierarchy that is used over a network,
    /// which is why it has a type of its own rather than being another data key.
    #[must_use]
    pub fn expose(&self) -> &[u8; KEY_LEN] {
        self.0.expose()
    }
}

#[cfg(test)]
mod tests {
    use subtle::ConstantTimeEq as _;

    use super::{DataKey, KEY_LEN, Kek};
    use crate::error::CryptoError;

    /// Thirty-two bytes written out by hand. Not derived from anything and not a secret.
    const PATTERN: [u8; KEY_LEN] = [
        0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1,
        0xf0, 0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2,
        0xe1, 0xf0,
    ];

    #[test]
    fn debug_of_a_data_key_reveals_nothing() {
        let key = DataKey::from_bytes(PATTERN);
        assert_eq!(format!("{key:?}"), "DataKey([REDACTED])");
    }

    #[test]
    fn debug_of_a_kek_reveals_nothing() {
        let key = Kek::from_bytes(PATTERN);
        assert_eq!(format!("{key:?}"), "Kek([REDACTED])");
    }

    #[test]
    fn debug_of_a_struct_holding_a_key_reveals_nothing() {
        // The case that matters in practice. Nobody prints the key on purpose; they print
        // the state that happens to contain it.
        #[derive(Debug)]
        struct Session {
            key: DataKey,
        }

        let session = Session {
            key: DataKey::from_bytes(PATTERN),
        };
        let printed = format!("{session:?}");

        assert!(
            printed.contains("[REDACTED]"),
            "the key inside the struct was not redacted: {printed}"
        );
        for byte in session.key.expose() {
            assert!(
                !printed.contains(&format!("{byte:#04x}")),
                "the printed struct contains a byte of the key: {printed}"
            );
        }
    }

    #[test]
    fn the_key_holds_the_bytes_it_was_given() {
        // `from_bytes` clears the array it was handed, so this also proves that the wipe
        // happens after the copy rather than before it. If the order were wrong, the key
        // would come back as thirty-two zeroes.
        let key = DataKey::from_bytes(PATTERN);
        assert_eq!(key.expose(), &PATTERN);
    }

    #[test]
    fn equal_keys_compare_equal_and_different_keys_do_not() {
        let first = DataKey::from_bytes(PATTERN);
        let same = DataKey::from_bytes(PATTERN);

        let mut other_bytes = PATTERN;
        other_bytes[KEY_LEN - 1] ^= 0x01;
        let other = DataKey::from_bytes(other_bytes);

        assert!(bool::from(first.ct_eq(&same)));
        assert!(!bool::from(first.ct_eq(&other)));
    }

    #[test]
    fn a_refusal_from_the_operating_system_is_propagated() {
        crate::random::fault::arm();
        assert!(matches!(DataKey::generate(), Err(CryptoError::Entropy)));
    }

    #[test]
    fn two_generated_keys_differ_and_are_not_zero() {
        let first = DataKey::generate().unwrap();
        let second = DataKey::generate().unwrap();
        assert!(!bool::from(first.ct_eq(&second)));
        assert_ne!(first.expose(), &[0_u8; KEY_LEN]);
    }
}
