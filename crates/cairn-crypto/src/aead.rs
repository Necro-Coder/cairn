//! The only two functions in this project that encrypt and decrypt anything.
//!
//! Everything sensitive goes through [`seal`] and comes back through [`open`]. There is no
//! second path, no helper that takes a key and a slice, and no direct use of the cipher
//! crate anywhere else in the workspace. A test walks the source tree to keep it that way,
//! because one import in a hurry is all it takes to grow a second way of doing this, and
//! the second way is always the one without the associated data.
//!
//! [`seal`] takes the nonce by value. That single detail is what makes nonce reuse a
//! compile error rather than a code review: once the nonce has been handed over, the
//! caller has nothing left to hand over again.

use zeroize::Zeroizing;

use crate::aad::Aad;
use crate::error::CryptoError;
use crate::keys::DataKey;
use crate::nonce::{FreshNonce, NONCE_LEN};

use chacha20poly1305::aead::{Aead as _, Payload};
use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce};

/// Length of the authentication tag, in bytes.
pub const TAG_LEN: usize = 16;

/// Longest plaintext one call may encrypt, in bytes.
///
/// This is a bound on one record, not on one file. Records are field values: a password, a
/// note, an amount. Sixteen megabytes is far more than any of those and still small enough
/// that a caller asking for a buffer cannot exhaust memory by accident. Anything genuinely
/// large is an export, and exports are framed into chunks that each go through here on
/// their own.
pub const MAX_PLAINTEXT_LEN: usize = 16 * 1024 * 1024;

/// An encrypted value together with the nonce it was encrypted under.
///
/// The nonce travels with the ciphertext because it has to: decryption needs it and it is
/// not a secret. It travels as plain bytes rather than as a [`FreshNonce`] because a
/// `FreshNonce` is a permission to encrypt once, and reading one back off a disk would
/// hand out that permission a second time for a nonce that has already been spent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
}

impl Sealed {
    /// The nonce this value was encrypted under.
    #[must_use]
    pub fn nonce(&self) -> &[u8; NONCE_LEN] {
        &self.nonce
    }

    /// The ciphertext, with its authentication tag at the end.
    #[must_use]
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }

    /// The nonce followed by the ciphertext, which is how this is stored.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(NONCE_LEN + self.ciphertext.len());
        bytes.extend_from_slice(&self.nonce);
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// Rebuilds a sealed value whose nonce was never written down.
    ///
    /// For the one format that recomputes its nonces instead of storing them: an exported
    /// backup derives each chunk's nonce from a base and the chunk's index, so the file
    /// holds ciphertext and tag and nothing else. A nonce read out of a file is a nonce the
    /// attacker chose; a nonce recomputed from a position is one that reordering a chunk
    /// gets wrong, which makes the tag fail twice over.
    ///
    /// This takes plain bytes rather than a [`FreshNonce`] for the same reason
    /// [`Sealed::from_bytes`] does. A `FreshNonce` is permission to encrypt once, and this
    /// builds a value that is about to be decrypted.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::MalformedSealed`] if the ciphertext is too short to carry a
    /// tag. A shape check, made before any key is involved, so it is not an oracle.
    pub fn from_parts(nonce: [u8; NONCE_LEN], ciphertext: Vec<u8>) -> Result<Self, CryptoError> {
        if ciphertext.len() < TAG_LEN {
            return Err(CryptoError::MalformedSealed {
                len: ciphertext.len(),
            });
        }

        Ok(Self { nonce, ciphertext })
    }

    /// Reads back what [`Sealed::to_bytes`] wrote.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::MalformedSealed`] if there are not enough bytes to be a
    /// nonce followed by a tag. This is a shape check and not a key check: it happens
    /// before any key is involved, so it reveals nothing about whether a key is right.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let Some((nonce, ciphertext)) = bytes.split_at_checked(NONCE_LEN) else {
            return Err(CryptoError::MalformedSealed { len: bytes.len() });
        };
        if ciphertext.len() < TAG_LEN {
            return Err(CryptoError::MalformedSealed { len: bytes.len() });
        }

        let mut nonce_bytes = [0_u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(nonce);

        Ok(Self {
            nonce: nonce_bytes,
            ciphertext: ciphertext.to_vec(),
        })
    }
}

/// Encrypts one value.
///
/// The nonce is consumed. That is the design: a caller holding a [`FreshNonce`] can
/// encrypt exactly once with it, and after the call there is nothing left to reuse.
///
/// # Errors
///
/// Returns [`CryptoError::PlaintextTooLong`] if the value is larger than one record may
/// be, and [`CryptoError::Open`] if the cipher itself refuses, which in practice means a
/// length the implementation cannot represent.
pub fn seal(
    key: &DataKey,
    nonce: FreshNonce,
    aad: &Aad,
    plaintext: &[u8],
) -> Result<Sealed, CryptoError> {
    if plaintext.len() > MAX_PLAINTEXT_LEN {
        return Err(CryptoError::PlaintextTooLong {
            len: plaintext.len(),
            max: MAX_PLAINTEXT_LEN,
        });
    }

    let nonce_bytes = nonce.into_bytes();
    let cipher = XChaCha20Poly1305::new(key.expose().into());

    let ciphertext = cipher
        .encrypt(
            &XNonce::from(nonce_bytes),
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        // The cipher only fails here on a length it cannot represent, which the check
        // above has already ruled out, so this is the one branch in this crate that the
        // suite cannot reach and that is left uncovered on purpose. Reaching it would mean
        // removing the length check, which is worth more than the coverage figure. It is
        // still written rather than unwrapped: the day the limit above changes, this is
        // what stops the process dying instead of returning an error.
        .map_err(|_| CryptoError::Open)?;

    Ok(Sealed {
        nonce: nonce_bytes,
        ciphertext,
    })
}

/// Decrypts one value, checking the tag and the associated data.
///
/// The plaintext comes back in a wrapper that clears it when it is dropped. Callers that
/// need it for longer have to say so by keeping the wrapper, which is the point: a
/// decrypted password left in a plain `Vec` is a decrypted password left in memory.
///
/// # Errors
///
/// Returns [`CryptoError::Open`] and nothing else. A wrong key, associated data that does
/// not match, a flipped bit and a truncated value all arrive here identically, because any
/// difference between them is an oracle.
pub fn open(key: &DataKey, sealed: &Sealed, aad: &Aad) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.expose().into());

    let plaintext = cipher
        .decrypt(
            &XNonce::from(sealed.nonce),
            Payload {
                msg: &sealed.ciphertext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| CryptoError::Open)?;

    Ok(Zeroizing::new(plaintext))
}

#[cfg(test)]
mod tests {
    use super::{MAX_PLAINTEXT_LEN, NONCE_LEN, Sealed, TAG_LEN, open, seal};
    use crate::aad::{Aad, ID_LEN};
    use crate::error::CryptoError;
    use crate::keys::{DataKey, KEY_LEN};
    use crate::nonce::FreshNonce;

    const ROW: [u8; ID_LEN] = [0xaa; ID_LEN];
    const KEY_ID: [u8; ID_LEN] = [0xbb; ID_LEN];

    fn key() -> DataKey {
        DataKey::from_bytes([0x5a; KEY_LEN])
    }

    fn aad() -> Aad {
        Aad::record(1, "credentials", &ROW, "password", 1, &KEY_ID).unwrap()
    }

    #[test]
    fn what_is_sealed_can_be_opened() {
        let plaintext = b"una nota que nadie mas deberia leer";
        let sealed = seal(&key(), FreshNonce::generate().unwrap(), &aad(), plaintext).unwrap();
        let opened = open(&key(), &sealed, &aad()).unwrap();
        assert_eq!(opened.as_slice(), plaintext);
    }

    #[test]
    fn an_empty_value_round_trips() {
        let sealed = seal(&key(), FreshNonce::generate().unwrap(), &aad(), b"").unwrap();
        assert_eq!(sealed.ciphertext().len(), TAG_LEN);
        assert!(open(&key(), &sealed, &aad()).unwrap().is_empty());
    }

    #[test]
    fn the_ciphertext_is_never_the_plaintext() {
        let plaintext = b"contrasena de ejemplo";
        let sealed = seal(&key(), FreshNonce::generate().unwrap(), &aad(), plaintext).unwrap();
        assert_ne!(sealed.ciphertext(), plaintext.as_slice());
        assert_eq!(sealed.ciphertext().len(), plaintext.len() + TAG_LEN);
    }

    #[test]
    fn two_seals_of_the_same_value_produce_different_ciphertexts() {
        // Not a nicety. Equal ciphertexts would mean the nonce was reused, and would let
        // anybody holding the file see which records hold the same value.
        let plaintext = b"el mismo valor dos veces";
        let first = seal(&key(), FreshNonce::generate().unwrap(), &aad(), plaintext).unwrap();
        let second = seal(&key(), FreshNonce::generate().unwrap(), &aad(), plaintext).unwrap();

        assert_ne!(first.nonce(), second.nonce());
        assert_ne!(first.ciphertext(), second.ciphertext());
    }

    #[test]
    fn a_different_key_does_not_open_it() {
        let sealed = seal(&key(), FreshNonce::generate().unwrap(), &aad(), b"secreto").unwrap();
        let other = DataKey::from_bytes([0x5b; KEY_LEN]);
        assert!(matches!(
            open(&other, &sealed, &aad()),
            Err(CryptoError::Open)
        ));
    }

    #[test]
    fn different_associated_data_does_not_open_it() {
        // The test that proves the defence against a record being moved. The ciphertext is
        // untouched and the key is right; only the row it claims to belong to changed.
        let sealed = seal(&key(), FreshNonce::generate().unwrap(), &aad(), b"secreto").unwrap();
        let elsewhere =
            Aad::record(1, "credentials", &[0xcc; ID_LEN], "password", 1, &KEY_ID).unwrap();
        assert!(matches!(
            open(&key(), &sealed, &elsewhere),
            Err(CryptoError::Open)
        ));
    }

    #[test]
    fn flipping_any_bit_of_the_ciphertext_stops_it_opening() {
        let plaintext = b"cada byte cuenta";
        let sealed = seal(&key(), FreshNonce::generate().unwrap(), &aad(), plaintext).unwrap();

        for index in 0..sealed.ciphertext().len() {
            let mut damaged = sealed.clone();
            damaged.ciphertext[index] ^= 0x01;
            assert!(
                matches!(open(&key(), &damaged, &aad()), Err(CryptoError::Open)),
                "a change to ciphertext byte {index} went unnoticed"
            );
        }
    }

    #[test]
    fn changing_the_nonce_stops_it_opening() {
        let sealed = seal(&key(), FreshNonce::generate().unwrap(), &aad(), b"secreto").unwrap();
        for index in 0..NONCE_LEN {
            let mut damaged = sealed.clone();
            damaged.nonce[index] ^= 0x01;
            assert!(
                matches!(open(&key(), &damaged, &aad()), Err(CryptoError::Open)),
                "a change to nonce byte {index} went unnoticed"
            );
        }
    }

    #[test]
    fn truncating_the_ciphertext_stops_it_opening() {
        let sealed = seal(&key(), FreshNonce::generate().unwrap(), &aad(), b"secreto").unwrap();
        let mut damaged = sealed.clone();
        damaged.ciphertext.pop();
        assert!(matches!(
            open(&key(), &damaged, &aad()),
            Err(CryptoError::Open)
        ));
    }

    #[test]
    fn a_sealed_value_survives_a_trip_through_bytes() {
        let sealed = seal(
            &key(),
            FreshNonce::generate().unwrap(),
            &aad(),
            b"ida y vuelta",
        )
        .unwrap();
        let restored = Sealed::from_bytes(&sealed.to_bytes()).unwrap();
        assert_eq!(restored, sealed);
        assert_eq!(
            open(&key(), &restored, &aad()).unwrap().as_slice(),
            b"ida y vuelta"
        );
    }

    #[test]
    fn a_value_rebuilt_from_its_parts_opens() {
        // The read path of a format whose nonces are recomputed rather than stored. What it
        // proves is that a value taken apart and put back together with the same nonce is
        // the same value, which is what lets a backup hold no nonces at all.
        let sealed = seal(
            &key(),
            FreshNonce::generate().unwrap(),
            &aad(),
            b"por partes",
        )
        .unwrap();
        let rebuilt = Sealed::from_parts(*sealed.nonce(), sealed.ciphertext().to_vec()).unwrap();

        assert_eq!(rebuilt, sealed);
        assert_eq!(
            open(&key(), &rebuilt, &aad()).unwrap().as_slice(),
            b"por partes"
        );
    }

    #[test]
    fn a_value_rebuilt_under_another_nonce_does_not_open() {
        let sealed = seal(
            &key(),
            FreshNonce::generate().unwrap(),
            &aad(),
            b"por partes",
        )
        .unwrap();
        let mut elsewhere = *sealed.nonce();
        elsewhere[0] ^= 0x01;

        let rebuilt = Sealed::from_parts(elsewhere, sealed.ciphertext().to_vec()).unwrap();
        assert!(matches!(
            open(&key(), &rebuilt, &aad()),
            Err(CryptoError::Open)
        ));
    }

    #[test]
    fn parts_with_no_room_for_a_tag_are_refused() {
        // The boundary, both sides of it. A ciphertext of exactly a tag is an encryption of
        // nothing and is legal; one byte less cannot be anything at all.
        for len in 0..TAG_LEN {
            assert!(
                matches!(
                    Sealed::from_parts([0_u8; NONCE_LEN], vec![0_u8; len]),
                    Err(CryptoError::MalformedSealed { .. })
                ),
                "a ciphertext of {len} bytes was accepted"
            );
        }
        assert!(Sealed::from_parts([0_u8; NONCE_LEN], vec![0_u8; TAG_LEN]).is_ok());
    }

    #[test]
    fn a_blob_too_short_to_be_a_nonce_and_a_tag_is_rejected() {
        // The shortest legal value is a nonce plus an empty ciphertext plus a tag. One
        // byte less than that has to be refused before any key is looked at, because a
        // truncated file is the likeliest thing a power cut leaves behind.
        let shortest = NONCE_LEN + TAG_LEN;

        let accepted: Vec<usize> = (0..shortest)
            .filter(|len| Sealed::from_bytes(&vec![0_u8; *len]).is_ok())
            .collect();
        assert!(
            accepted.is_empty(),
            "blobs of these lengths were accepted as sealed values: {accepted:?}"
        );

        assert!(Sealed::from_bytes(&vec![0_u8; shortest]).is_ok());
    }

    #[test]
    fn the_record_limit_is_the_number_it_is_documented_to_be() {
        // Frozen rather than derived. The limit is part of what this project promises: it
        // appears in the public documentation and in the error a caller gets back. A test
        // that only checks "one byte past the limit is refused" passes for any limit at
        // all, which means it is not testing the limit.
        assert_eq!(MAX_PLAINTEXT_LEN, 16_777_216);
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "sixteen megabytes through an interpreter is minutes of work for a boundary the ordinary run already checks"
    )]
    fn a_plaintext_of_exactly_the_record_limit_is_accepted() {
        // The other half of the boundary. Without it, a check written as "at least" rather
        // than "more than" passes every test while refusing a value that is legal.
        let at_the_limit = vec![0_u8; MAX_PLAINTEXT_LEN];
        let sealed = seal(
            &key(),
            FreshNonce::generate().unwrap(),
            &aad(),
            &at_the_limit,
        )
        .unwrap();
        assert_eq!(sealed.ciphertext().len(), MAX_PLAINTEXT_LEN + TAG_LEN);
    }

    #[test]
    fn a_plaintext_over_the_record_limit_is_refused() {
        // Allocated once and only in this test. The point is the boundary, so the value
        // checked is exactly one byte past it.
        let too_long = vec![0_u8; MAX_PLAINTEXT_LEN + 1];
        let outcome = seal(&key(), FreshNonce::generate().unwrap(), &aad(), &too_long);
        assert!(matches!(
            outcome,
            Err(CryptoError::PlaintextTooLong {
                max: MAX_PLAINTEXT_LEN,
                ..
            })
        ));
    }

    /// The published XChaCha20-Poly1305 test vector, appendix A.3.1 of the CFRG draft.
    ///
    /// Written out here rather than trusted to the suite of the dependency. What this
    /// checks is not that the algorithm is correct, it is that this code drives it the way
    /// the specification describes: the right key size, a twenty-four byte nonce, and the
    /// associated data in the associated data slot rather than prepended to the message.
    /// Getting any of those wrong produces something that encrypts and decrypts perfectly
    /// well and is not the algorithm the documentation claims.
    #[test]
    fn the_cipher_matches_the_published_test_vector() {
        use chacha20poly1305::aead::{Aead as _, Payload};
        use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce};

        const VECTOR_KEY: [u8; 32] = [
            0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d,
            0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b,
            0x9c, 0x9d, 0x9e, 0x9f,
        ];
        const VECTOR_NONCE: [u8; 24] = [
            0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d,
            0x4e, 0x4f, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57,
        ];
        const VECTOR_AAD: [u8; 12] = [
            0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        ];
        // Split across two lines with a continuation that starts in column zero, because
        // the escape swallows the newline and every space after it: any indentation on the
        // second line would silently become part of the message being encrypted.
        const VECTOR_PLAINTEXT: &[u8] = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        const VECTOR_CIPHERTEXT: [u8; 114] = [
            0xbd, 0x6d, 0x17, 0x9d, 0x3e, 0x83, 0xd4, 0x3b, 0x95, 0x76, 0x57, 0x94, 0x93, 0xc0,
            0xe9, 0x39, 0x57, 0x2a, 0x17, 0x00, 0x25, 0x2b, 0xfa, 0xcc, 0xbe, 0xd2, 0x90, 0x2c,
            0x21, 0x39, 0x6c, 0xbb, 0x73, 0x1c, 0x7f, 0x1b, 0x0b, 0x4a, 0xa6, 0x44, 0x0b, 0xf3,
            0xa8, 0x2f, 0x4e, 0xda, 0x7e, 0x39, 0xae, 0x64, 0xc6, 0x70, 0x8c, 0x54, 0xc2, 0x16,
            0xcb, 0x96, 0xb7, 0x2e, 0x12, 0x13, 0xb4, 0x52, 0x2f, 0x8c, 0x9b, 0xa4, 0x0d, 0xb5,
            0xd9, 0x45, 0xb1, 0x1b, 0x69, 0xb9, 0x82, 0xc1, 0xbb, 0x9e, 0x3f, 0x3f, 0xac, 0x2b,
            0xc3, 0x69, 0x48, 0x8f, 0x76, 0xb2, 0x38, 0x35, 0x65, 0xd3, 0xff, 0xf9, 0x21, 0xf9,
            0x66, 0x4c, 0x97, 0x63, 0x7d, 0xa9, 0x76, 0x88, 0x12, 0xf6, 0x15, 0xc6, 0x8b, 0x13,
            0xb5, 0x2e,
        ];
        const VECTOR_TAG: [u8; 16] = [
            0xc0, 0x87, 0x59, 0x24, 0xc1, 0xc7, 0x98, 0x79, 0x47, 0xde, 0xaf, 0xd8, 0x78, 0x0a,
            0xcf, 0x49,
        ];

        let cipher = XChaCha20Poly1305::new(&VECTOR_KEY.into());
        let produced = cipher
            .encrypt(
                &XNonce::from(VECTOR_NONCE),
                Payload {
                    msg: VECTOR_PLAINTEXT,
                    aad: &VECTOR_AAD,
                },
            )
            .unwrap();

        let (body, tag) = produced.split_at(VECTOR_CIPHERTEXT.len());
        assert_eq!(body, VECTOR_CIPHERTEXT);
        assert_eq!(tag, VECTOR_TAG);
    }
}

/// Properties, checked over generated inputs rather than over the cases anybody thought of.
///
/// These live inside the crate rather than beside it in an integration test for one
/// reason: the anti-reuse harness in [`crate::nonce`] is compiled under `#[cfg(test)]`,
/// which is not set when this crate is built as a dependency of a file in `tests/`.
/// Running the property tests here means every nonce they produce, which is thousands of
/// them, goes through that check.
#[cfg(test)]
mod properties {
    use proptest::prelude::{ProptestConfig, any, prop_assert, prop_assert_eq, prop_assert_ne};
    use proptest::{prop_assume, proptest};

    use super::{Sealed, open, seal};
    use crate::aad::{Aad, ID_LEN, MAX_NAME_LEN};
    use crate::error::CryptoError;
    use crate::keys::{DataKey, KEY_LEN};
    use crate::nonce::FreshNonce;

    fn aad_of(table: &str, row: [u8; ID_LEN], column: &str, rev: u64) -> Aad {
        Aad::record(1, table, &row, column, rev, &[0x01; ID_LEN]).unwrap()
    }

    /// How many inputs each property is checked against.
    ///
    /// Fewer than the proptest default, because every case runs a real key schedule. It is
    /// still two orders of magnitude more inputs than a table of examples, and it keeps the
    /// suite fast enough that nobody is tempted to skip it.
    ///
    /// Far fewer under Miri, which interprets every instruction and would turn a few
    /// seconds into an afternoon. Miri is here to find undefined behaviour in the way this
    /// code is written, and one trip through each function finds as much of that as a
    /// thousand do.
    const CASES: u32 = if cfg!(miri) { 2 } else { 256 };

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(CASES))]

        /// Whatever goes in comes back out, for any key, any plaintext and any context.
        #[test]
        fn anything_sealed_opens_again(
            key_bytes in any::<[u8; KEY_LEN]>(),
            plaintext in proptest::collection::vec(any::<u8>(), 0..4096),
            table in "[a-z_]{0,20}",
            column in "[a-z_]{0,20}",
            row in any::<[u8; ID_LEN]>(),
            rev in any::<u64>(),
        ) {
            let key = DataKey::from_bytes(key_bytes);
            let aad = aad_of(&table, row, &column, rev);

            let sealed = seal(&key, FreshNonce::generate().unwrap(), &aad, &plaintext).unwrap();
            let opened = open(&key, &sealed, &aad).unwrap();

            prop_assert_eq!(opened.as_slice(), plaintext.as_slice());
        }

        /// The stored form survives a trip through bytes unchanged.
        #[test]
        fn the_stored_form_round_trips(
            key_bytes in any::<[u8; KEY_LEN]>(),
            plaintext in proptest::collection::vec(any::<u8>(), 0..2048),
        ) {
            let key = DataKey::from_bytes(key_bytes);
            let aad = aad_of("t", [0; ID_LEN], "c", 0);

            let sealed = seal(&key, FreshNonce::generate().unwrap(), &aad, &plaintext).unwrap();
            let restored = Sealed::from_bytes(&sealed.to_bytes()).unwrap();

            prop_assert_eq!(&restored, &sealed);

            let opened = open(&key, &restored, &aad).unwrap();
            prop_assert_eq!(opened.as_slice(), plaintext.as_slice());
        }

        /// Associated data that differs in any field never opens the value.
        ///
        /// This is the property the whole defence against a moved record rests on. A
        /// single example proves that one rearrangement fails; this proves that none of
        /// them work.
        #[test]
        fn associated_data_that_differs_never_opens(
            key_bytes in any::<[u8; KEY_LEN]>(),
            plaintext in proptest::collection::vec(any::<u8>(), 0..512),
            table in "[a-z_]{0,20}",
            other_table in "[a-z_]{0,20}",
            column in "[a-z_]{0,20}",
            other_column in "[a-z_]{0,20}",
            row in any::<[u8; ID_LEN]>(),
            other_row in any::<[u8; ID_LEN]>(),
            rev in any::<u64>(),
            other_rev in any::<u64>(),
        ) {
            prop_assume!(
                (&table, &column, row, rev) != (&other_table, &other_column, other_row, other_rev)
            );

            let key = DataKey::from_bytes(key_bytes);
            let sealed = seal(
                &key,
                FreshNonce::generate().unwrap(),
                &aad_of(&table, row, &column, rev),
                &plaintext,
            )
            .unwrap();

            let elsewhere = aad_of(&other_table, other_row, &other_column, other_rev);
            prop_assert!(matches!(open(&key, &sealed, &elsewhere), Err(CryptoError::Open)));
        }

        /// A different key never opens the value.
        #[test]
        fn a_different_key_never_opens(
            key_bytes in any::<[u8; KEY_LEN]>(),
            other_bytes in any::<[u8; KEY_LEN]>(),
            plaintext in proptest::collection::vec(any::<u8>(), 0..512),
        ) {
            prop_assume!(key_bytes != other_bytes);

            let aad = aad_of("t", [0; ID_LEN], "c", 0);
            let key = DataKey::from_bytes(key_bytes);
            let sealed = seal(&key, FreshNonce::generate().unwrap(), &aad, &plaintext).unwrap();

            let other = DataKey::from_bytes(other_bytes);
            prop_assert!(matches!(open(&other, &sealed, &aad), Err(CryptoError::Open)));
        }

        /// No two seals in one run ever share a nonce.
        ///
        /// The harness inside the nonce module already panics on a repeat, so this test
        /// mostly exists to feed it volume. Asserting the pair here as well means a
        /// failure names the property rather than only the mechanism that caught it.
        #[test]
        fn two_seals_never_share_a_nonce(
            key_bytes in any::<[u8; KEY_LEN]>(),
            plaintext in proptest::collection::vec(any::<u8>(), 0..64),
        ) {
            let key = DataKey::from_bytes(key_bytes);
            let aad = aad_of("t", [0; ID_LEN], "c", 0);

            let first = seal(&key, FreshNonce::generate().unwrap(), &aad, &plaintext).unwrap();
            let second = seal(&key, FreshNonce::generate().unwrap(), &aad, &plaintext).unwrap();

            prop_assert_ne!(first.nonce(), second.nonce());
            prop_assert_ne!(first.ciphertext(), second.ciphertext());
        }

        /// A name at or under the limit encodes; one over it is refused rather than cut.
        #[test]
        fn a_name_is_never_silently_truncated(len in 0_usize..=(MAX_NAME_LEN + 8)) {
            let name = "n".repeat(len);
            let outcome = Aad::record(1, &name, &[0; ID_LEN], "c", 0, &[0; ID_LEN]);

            // The match is bound first because the assertion macro turns the expression
            // it is given into a format string, and a struct pattern has braces in it.
            let refused = matches!(outcome, Err(CryptoError::FieldTooLong { .. }));

            if len <= MAX_NAME_LEN {
                prop_assert!(!refused, "a name of {} bytes was refused", len);
            } else {
                prop_assert!(refused, "a name of {} bytes was not refused", len);
            }
        }
    }
}
