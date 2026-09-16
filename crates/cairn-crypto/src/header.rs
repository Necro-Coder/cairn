//! The vault header: a hundred and sixty-eight bytes that say how to get the keys.
//!
//! It is a file of its own, beside the database rather than inside it. Putting it inside
//! would be circular: the database is encrypted with a key that hangs off the data key,
//! and the data key is what this file holds, wrapped. Reading it would require the keys it
//! exists to hand out.
//!
//! The layout is fixed length on purpose. A parser with no length field and no loop is a
//! parser with nowhere for an overflow to hide, and this is the one piece of input in the
//! whole design that somebody who has stolen the file can rewrite at will.
//!
//! ```text
//! offset  bytes  field
//!      0      8  magic, "CAIRNHDR"
//!      8      2  format version, currently 1
//!     10      2  reserved, must be zero
//!     12     16  salt for Argon2id
//!     28      4  Argon2id memory cost, in kibibytes
//!     32      4  Argon2id passes
//!     36      4  Argon2id lanes
//!     40     16  key identifier, a version four UUID
//!     56      8  when the vault was created, microseconds since the epoch, UTC
//!     64      8  when the parameters were last written, same units
//!     72      4  how many times the master password has been changed
//!     76      4  how many times this header has been rewritten
//!  -- 80: end of the authenticated prefix; these bytes are the associated data --
//!     80     24  nonce the data key was wrapped under
//!    104     48  the wrapped data key: thirty-two of key and sixteen of tag
//!  -- 152: end of everything the tag covers --
//!    152      4  failed unlock attempts          UNAUTHENTICATED
//!    156      8  locked until, same units        UNAUTHENTICATED
//!    164      4  CRC32 of the twelve bytes above UNAUTHENTICATED
//! ```
//!
//! The last twelve bytes are not authenticated and cannot be. Writing them happens after a
//! failed unlock, which is precisely the moment there is no key to authenticate with:
//! re-wrapping the data key needs the password, and the password is what was just got
//! wrong. So anybody holding the file can reset the attempt counter, and this is written
//! down rather than glossed over, in the public documentation as well as here. The CRC is
//! there to notice corruption, not tampering, and calling it anything else would be a lie.
//!
//! What that leaves protected is what actually matters against somebody editing the file:
//! the Argon2id parameters, the salt, the key identifier and the two counters all sit
//! inside the authenticated prefix, so lowering the cost of guessing the password breaks
//! the unwrapping instead of cheapening the guess.

use crate::aad::{Aad, ID_LEN};
use crate::error::CryptoError;
use crate::kdf::{Argon2Params, SALT_LEN};
use crate::nonce::NONCE_LEN;

/// Total size of the header, in bytes.
pub const HEADER_LEN: usize = 168;

/// How many bytes at the front are authenticated by the wrapped data key.
pub const AUTHENTICATED_PREFIX_LEN: usize = 80;

/// Size of the wrapped data key: the key itself plus its tag.
pub const WRAPPED_DEK_LEN: usize = 48;

/// What every header of this format starts with.
///
/// Eight bytes that are not valid anything else, so a file that is not one of ours is
/// rejected on the first comparison rather than by failing to decrypt later and looking
/// like a wrong password.
pub const MAGIC: [u8; 8] = *b"CAIRNHDR";

/// The only format version that exists.
///
/// The field is here from the first byte of the first version so that a second one can be
/// introduced without guessing what the first one meant. A reader that finds a version it
/// does not know refuses rather than interpreting the bytes hopefully.
pub const FORMAT_VERSION: u16 = 1;

/// Where each field starts. Written out rather than computed, so the table above and the
/// code can be compared by eye.
mod offset {
    pub(super) const MAGIC: usize = 0;
    pub(super) const FORMAT_VERSION: usize = 8;
    pub(super) const RESERVED: usize = 10;
    pub(super) const KDF_SALT: usize = 12;
    pub(super) const ARGON2_MEMORY: usize = 28;
    pub(super) const ARGON2_PASSES: usize = 32;
    pub(super) const ARGON2_LANES: usize = 36;
    pub(super) const KEY_ID: usize = 40;
    pub(super) const CREATED_AT: usize = 56;
    pub(super) const PARAMS_WRITTEN_AT: usize = 64;
    pub(super) const MASTER_CHANGE_COUNT: usize = 72;
    pub(super) const HEADER_REV: usize = 76;
    pub(super) const WRAP_NONCE: usize = 80;
    pub(super) const WRAPPED_DEK: usize = 104;
    pub(super) const FAILED_ATTEMPTS: usize = 152;
    pub(super) const LOCKED_UNTIL: usize = 156;
    pub(super) const TRAILER_CRC: usize = 164;
}

/// Where the unauthenticated tail begins, and how long the part the CRC covers is.
const TRAILER_START: usize = offset::FAILED_ATTEMPTS;
const TRAILER_COVERED_LEN: usize = offset::TRAILER_CRC - TRAILER_START;

/// A parsed vault header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultHeader {
    kdf_salt: [u8; SALT_LEN],
    params: Argon2Params,
    key_id: [u8; ID_LEN],
    created_at_us: i64,
    params_written_at_us: i64,
    master_change_count: u32,
    header_rev: u32,
    wrap_nonce: [u8; NONCE_LEN],
    wrapped_dek: [u8; WRAPPED_DEK_LEN],
    failed_attempts: u32,
    locked_until_us: i64,
}

impl VaultHeader {
    /// Reads a header from exactly [`HEADER_LEN`] bytes.
    ///
    /// Total: every input of every length either produces a header or an error, and none of
    /// them panics. That is checked by a test that mutates each of the hundred and sixty-
    /// eight bytes in turn, and by a fuzzing target, because this is a parser whose input an
    /// attacker gets to write.
    ///
    /// # Errors
    ///
    /// Refuses a wrong length, a wrong magic, an unknown format version, a non-zero reserved
    /// field, Argon2id parameters outside the allowed range, and a trailer whose checksum
    /// does not match. The parameter check happens before anything is allocated, which is
    /// the whole reason the ceiling exists.
    pub fn parse(bytes: &[u8]) -> Result<Self, CryptoError> {
        let bytes: &[u8; HEADER_LEN] = bytes
            .try_into()
            .map_err(|_| CryptoError::HeaderSize { len: bytes.len() })?;

        if read::<{ offset::MAGIC }, 8>(bytes) != MAGIC {
            return Err(CryptoError::HeaderMagic);
        }

        let format_version = read_u16::<{ offset::FORMAT_VERSION }>(bytes);
        if format_version != FORMAT_VERSION {
            return Err(CryptoError::HeaderVersion {
                found: format_version,
            });
        }

        if read_u16::<{ offset::RESERVED }>(bytes) != 0 {
            return Err(CryptoError::HeaderReserved);
        }

        // Before anything is allocated and before the password is even asked for, because
        // the point of the ceiling is to refuse a header that would exhaust memory rather
        // than to discover afterwards that it did.
        let params = Argon2Params::new(
            read_u32::<{ offset::ARGON2_MEMORY }>(bytes),
            read_u32::<{ offset::ARGON2_PASSES }>(bytes),
            read_u32::<{ offset::ARGON2_LANES }>(bytes),
        )?;

        let covered = read::<{ offset::FAILED_ATTEMPTS }, TRAILER_COVERED_LEN>(bytes);
        if read_u32::<{ offset::TRAILER_CRC }>(bytes) != trailer_checksum(&covered) {
            return Err(CryptoError::HeaderTrailerChecksum);
        }

        Ok(Self {
            kdf_salt: read::<{ offset::KDF_SALT }, SALT_LEN>(bytes),
            params,
            key_id: read::<{ offset::KEY_ID }, ID_LEN>(bytes),
            created_at_us: read_i64::<{ offset::CREATED_AT }>(bytes),
            params_written_at_us: read_i64::<{ offset::PARAMS_WRITTEN_AT }>(bytes),
            master_change_count: read_u32::<{ offset::MASTER_CHANGE_COUNT }>(bytes),
            header_rev: read_u32::<{ offset::HEADER_REV }>(bytes),
            wrap_nonce: read::<{ offset::WRAP_NONCE }, NONCE_LEN>(bytes),
            wrapped_dek: read::<{ offset::WRAPPED_DEK }, WRAPPED_DEK_LEN>(bytes),
            failed_attempts: read_u32::<{ offset::FAILED_ATTEMPTS }>(bytes),
            locked_until_us: read_i64::<{ offset::LOCKED_UNTIL }>(bytes),
        })
    }

    /// Writes the header out, checksum included.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut bytes = [0_u8; HEADER_LEN];

        write::<{ offset::MAGIC }, 8>(&mut bytes, &MAGIC);
        write::<{ offset::FORMAT_VERSION }, 2>(&mut bytes, &FORMAT_VERSION.to_le_bytes());
        // The reserved field stays zero. Written out rather than left to the initialiser, so
        // that this function and the layout above list the same fields in the same order.
        write::<{ offset::RESERVED }, 2>(&mut bytes, &0_u16.to_le_bytes());
        write::<{ offset::KDF_SALT }, SALT_LEN>(&mut bytes, &self.kdf_salt);
        write::<{ offset::ARGON2_MEMORY }, 4>(&mut bytes, &self.params.memory_kib().to_le_bytes());
        write::<{ offset::ARGON2_PASSES }, 4>(&mut bytes, &self.params.passes().to_le_bytes());
        write::<{ offset::ARGON2_LANES }, 4>(&mut bytes, &self.params.lanes().to_le_bytes());
        write::<{ offset::KEY_ID }, ID_LEN>(&mut bytes, &self.key_id);
        write::<{ offset::CREATED_AT }, 8>(&mut bytes, &self.created_at_us.to_le_bytes());
        write::<{ offset::PARAMS_WRITTEN_AT }, 8>(
            &mut bytes,
            &self.params_written_at_us.to_le_bytes(),
        );
        write::<{ offset::MASTER_CHANGE_COUNT }, 4>(
            &mut bytes,
            &self.master_change_count.to_le_bytes(),
        );
        write::<{ offset::HEADER_REV }, 4>(&mut bytes, &self.header_rev.to_le_bytes());
        write::<{ offset::WRAP_NONCE }, NONCE_LEN>(&mut bytes, &self.wrap_nonce);
        write::<{ offset::WRAPPED_DEK }, WRAPPED_DEK_LEN>(&mut bytes, &self.wrapped_dek);
        write::<{ offset::FAILED_ATTEMPTS }, 4>(&mut bytes, &self.failed_attempts.to_le_bytes());
        write::<{ offset::LOCKED_UNTIL }, 8>(&mut bytes, &self.locked_until_us.to_le_bytes());

        let covered = read::<{ offset::FAILED_ATTEMPTS }, TRAILER_COVERED_LEN>(&bytes);
        write::<{ offset::TRAILER_CRC }, 4>(&mut bytes, &trailer_checksum(&covered).to_le_bytes());

        bytes
    }

    /// The associated data the data key is wrapped under: the whole authenticated prefix.
    ///
    /// The prefix rather than a list of the fields that seemed important. A rule covers a
    /// field added next year; a list covers the fields somebody remembered, and the one they
    /// forget is the one an attacker edits.
    #[must_use]
    pub fn wrap_aad(&self) -> Aad {
        let bytes = self.to_bytes();
        Aad::authenticated_prefix(&read::<0, AUTHENTICATED_PREFIX_LEN>(&bytes))
    }
}

impl VaultHeader {
    /// Builds the header for a vault that is being created.
    ///
    /// Everything that is chosen once is chosen here: the salt, the identifier of the data
    /// key, and the moment. The revision starts at one rather than zero, so that "has been
    /// written" and "has never been written" are different values rather than the same one.
    pub(crate) fn new(
        kdf_salt: [u8; SALT_LEN],
        params: Argon2Params,
        key_id: [u8; ID_LEN],
        created_at_us: i64,
    ) -> Self {
        Self {
            kdf_salt,
            params,
            key_id,
            created_at_us,
            params_written_at_us: created_at_us,
            master_change_count: 0,
            header_rev: 1,
            wrap_nonce: [0; NONCE_LEN],
            wrapped_dek: [0; WRAPPED_DEK_LEN],
            failed_attempts: 0,
            locked_until_us: 0,
        }
    }

    /// Puts the wrapped data key in, once there is one.
    ///
    /// Two steps rather than one because of what the associated data is. The key is wrapped
    /// under the authenticated prefix of the header it is going into, so the header has to
    /// exist before the wrapping can happen. The prefix ends at byte eighty and these two
    /// fields start there, so the bytes this writes are not bytes the tag covers, and filling
    /// them in afterwards changes nothing the wrapping depended on.
    pub(crate) fn set_wrapped(
        &mut self,
        wrap_nonce: [u8; NONCE_LEN],
        wrapped_dek: [u8; WRAPPED_DEK_LEN],
    ) {
        self.wrap_nonce = wrap_nonce;
        self.wrapped_dek = wrapped_dek;
    }

    /// The salt Argon2id was given.
    #[must_use]
    pub fn kdf_salt(&self) -> &[u8; SALT_LEN] {
        &self.kdf_salt
    }

    /// The Argon2id parameters this vault was written with.
    #[must_use]
    pub fn params(&self) -> Argon2Params {
        self.params
    }

    /// The identifier of the data key, which goes into the associated data of every record.
    ///
    /// It changes only when the data key changes, which is never after creation. A counter
    /// would say how many times the key had been rotated; a hash of the key would be
    /// material derived from the key written in the clear. A random identifier says nothing
    /// about anybody.
    #[must_use]
    pub fn key_id(&self) -> &[u8; ID_LEN] {
        &self.key_id
    }

    /// When the vault was created, in microseconds since the epoch, UTC.
    #[must_use]
    pub fn created_at_us(&self) -> i64 {
        self.created_at_us
    }

    /// When the Argon2id parameters were last written, same units.
    ///
    /// Kept so that a slow unlock a year from now has an answer other than a shrug.
    #[must_use]
    pub fn params_written_at_us(&self) -> i64 {
        self.params_written_at_us
    }

    /// How many times the master password has been changed.
    #[must_use]
    pub fn master_change_count(&self) -> u32 {
        self.master_change_count
    }

    /// How many times this header has been rewritten.
    #[must_use]
    pub fn header_rev(&self) -> u32 {
        self.header_rev
    }

    /// The wrapped data key, nonce first.
    pub(crate) fn wrapped_dek(&self) -> ([u8; NONCE_LEN], [u8; WRAPPED_DEK_LEN]) {
        (self.wrap_nonce, self.wrapped_dek)
    }

    /// How many unlock attempts have failed since the last successful one.
    ///
    /// Read from the unauthenticated tail. Anybody holding the file can set it to zero, and
    /// nothing here pretends otherwise: the real cost of guessing is Argon2id, and this
    /// number only makes the interface unpleasant to attack.
    #[must_use]
    pub fn failed_attempts(&self) -> u32 {
        self.failed_attempts
    }

    /// Until when further attempts are refused, in microseconds since the epoch, UTC.
    #[must_use]
    pub fn locked_until_us(&self) -> i64 {
        self.locked_until_us
    }

    /// Records the outcome of an unlock attempt in the unauthenticated tail.
    ///
    /// Separate from everything else on purpose, and the only mutation that does not bump
    /// the revision, because it does not touch a single authenticated byte. The alternative
    /// would be re-wrapping the data key after a failed attempt, which needs the password
    /// that was just got wrong.
    pub fn record_attempt(&mut self, failed_attempts: u32, locked_until_us: i64) {
        self.failed_attempts = failed_attempts;
        self.locked_until_us = locked_until_us;
    }

    /// Returns a copy re-wrapped under a new key encryption key, with the same data key.
    ///
    /// This is the operation that makes changing the master password cheap: the data key is
    /// unchanged, so the identifier is unchanged, so every subkey below it is unchanged, so
    /// not one stored byte anywhere else has to be rewritten.
    pub(crate) fn rewrapped(
        &self,
        kdf_salt: [u8; SALT_LEN],
        params: Argon2Params,
        params_written_at_us: i64,
        master_password: MasterPassword,
    ) -> Self {
        Self {
            kdf_salt,
            params,
            key_id: self.key_id,
            created_at_us: self.created_at_us,
            params_written_at_us,
            // Saturating rather than wrapping. A counter that silently returns to zero is
            // worse than one that stops counting, and neither is reachable by a person who
            // would have to change the password four billion times to get there.
            master_change_count: self
                .master_change_count
                .saturating_add(match master_password {
                    MasterPassword::Changed => 1,
                    MasterPassword::Kept => 0,
                }),
            header_rev: self.header_rev.saturating_add(1),
            // Filled in by `set_wrapped` once the key has been sealed under the prefix this
            // very value is part of.
            wrap_nonce: [0; NONCE_LEN],
            wrapped_dek: [0; WRAPPED_DEK_LEN],
            failed_attempts: 0,
            locked_until_us: 0,
        }
    }
}

/// Whether a rewrite of the header was a change of master password.
///
/// An enum rather than a boolean argument, because `rewrapped(salt, params, now, true)`
/// reads as a mystery at the call site and there is nothing to look up: the value is the
/// documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MasterPassword {
    /// The person chose a new master password.
    Changed,
    /// The same password, rewrapped under different derivation parameters.
    Kept,
}

/// The checksum over the unauthenticated tail.
///
/// CRC32 because what it has to catch is a half written file or a flipped bit on a disk,
/// not an edit. Anybody who can write these twelve bytes can recompute this, which is why
/// nothing that matters lives out here.
fn trailer_checksum(covered: &[u8; TRAILER_COVERED_LEN]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(covered);
    hasher.finalize()
}

/// Copies `N` bytes out of the header at a fixed offset.
///
/// The offset and the length are const parameters, so the inline assertion below is checked
/// when this compiles rather than when somebody opens a vault. That is what makes the parser
/// total without a single runtime bounds check: a read that could go past the end is a build
/// that does not exist.
#[expect(
    clippy::indexing_slicing,
    reason = "INVARIANT: the inline const assertion proves the range lies inside an array of exactly HEADER_LEN bytes, at compile time"
)]
fn read<const AT: usize, const N: usize>(bytes: &[u8; HEADER_LEN]) -> [u8; N] {
    const {
        assert!(
            AT + N <= HEADER_LEN,
            "field runs past the end of the header"
        );
    };

    let mut field = [0_u8; N];
    field.copy_from_slice(&bytes[AT..AT + N]);
    field
}

/// Copies `N` bytes into the header at a fixed offset, with the same compile time proof.
#[expect(
    clippy::indexing_slicing,
    reason = "INVARIANT: the inline const assertion proves the range lies inside an array of exactly HEADER_LEN bytes, at compile time"
)]
fn write<const AT: usize, const N: usize>(bytes: &mut [u8; HEADER_LEN], field: &[u8; N]) {
    const {
        assert!(
            AT + N <= HEADER_LEN,
            "field runs past the end of the header"
        );
    };

    bytes[AT..AT + N].copy_from_slice(field);
}

/// Reads a little endian `u16` at a fixed offset.
fn read_u16<const AT: usize>(bytes: &[u8; HEADER_LEN]) -> u16 {
    u16::from_le_bytes(read::<AT, 2>(bytes))
}

/// Reads a little endian `u32` at a fixed offset.
fn read_u32<const AT: usize>(bytes: &[u8; HEADER_LEN]) -> u32 {
    u32::from_le_bytes(read::<AT, 4>(bytes))
}

/// Reads a little endian `i64` at a fixed offset.
fn read_i64<const AT: usize>(bytes: &[u8; HEADER_LEN]) -> i64 {
    i64::from_le_bytes(read::<AT, 8>(bytes))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::{ProptestConfig, any, prop_assert, prop_assert_eq};
    use proptest::proptest;

    use super::{
        AUTHENTICATED_PREFIX_LEN, HEADER_LEN, MasterPassword, TRAILER_COVERED_LEN, TRAILER_START,
        VaultHeader, WRAPPED_DEK_LEN, offset, read, trailer_checksum, write,
    };
    use crate::aad::ID_LEN;
    use crate::error::CryptoError;
    use crate::kdf::{Argon2Params, SALT_LEN};
    use crate::nonce::NONCE_LEN;

    /// A header with every field set to something recognisable and nothing derived.
    fn sample() -> VaultHeader {
        let mut header = VaultHeader::new(
            [0x11; SALT_LEN],
            Argon2Params::DEFAULT,
            [0x22; ID_LEN],
            1_700_000_000_000_000,
        );
        header.set_wrapped([0x33; NONCE_LEN], [0x44; WRAPPED_DEK_LEN]);
        header
    }

    #[test]
    fn a_header_is_exactly_the_documented_size() {
        assert_eq!(HEADER_LEN, 168);
        assert_eq!(AUTHENTICATED_PREFIX_LEN, 80);
        assert_eq!(sample().to_bytes().len(), HEADER_LEN);
    }

    #[test]
    fn the_fields_sit_where_the_layout_says_they_do() {
        // Frozen against the table in the module documentation. This is a file format: a
        // field that moves is every existing vault refusing to open, and the move would
        // otherwise be invisible because writing and reading would agree with each other.
        let bytes = sample().to_bytes();

        assert_eq!(read::<{ offset::MAGIC }, 8>(&bytes), *b"CAIRNHDR");
        assert_eq!(
            read::<{ offset::FORMAT_VERSION }, 2>(&bytes),
            1_u16.to_le_bytes()
        );
        assert_eq!(read::<{ offset::RESERVED }, 2>(&bytes), [0, 0]);
        assert_eq!(
            read::<{ offset::KDF_SALT }, SALT_LEN>(&bytes),
            [0x11; SALT_LEN]
        );
        assert_eq!(
            read::<{ offset::ARGON2_MEMORY }, 4>(&bytes),
            (64_u32 * 1024).to_le_bytes()
        );
        assert_eq!(
            read::<{ offset::ARGON2_PASSES }, 4>(&bytes),
            3_u32.to_le_bytes()
        );
        assert_eq!(
            read::<{ offset::ARGON2_LANES }, 4>(&bytes),
            1_u32.to_le_bytes()
        );
        assert_eq!(read::<{ offset::KEY_ID }, ID_LEN>(&bytes), [0x22; ID_LEN]);
        assert_eq!(
            read::<{ offset::CREATED_AT }, 8>(&bytes),
            1_700_000_000_000_000_i64.to_le_bytes()
        );
        assert_eq!(
            read::<{ offset::WRAP_NONCE }, NONCE_LEN>(&bytes),
            [0x33; NONCE_LEN]
        );
        assert_eq!(
            read::<{ offset::WRAPPED_DEK }, WRAPPED_DEK_LEN>(&bytes),
            [0x44; WRAPPED_DEK_LEN]
        );
    }

    #[test]
    fn a_header_survives_a_trip_through_bytes() {
        let header = sample();
        assert_eq!(VaultHeader::parse(&header.to_bytes()).unwrap(), header);
    }

    #[test]
    fn any_length_but_the_right_one_is_refused() {
        // Zero, one short, one long. A truncated file is the likeliest thing a power cut
        // leaves behind, and it has to be refused rather than read as far as it goes.
        for len in [0, 1, HEADER_LEN - 1, HEADER_LEN + 1, 4096] {
            let outcome = VaultHeader::parse(&vec![0_u8; len]);
            assert!(
                matches!(outcome, Err(CryptoError::HeaderSize { .. })),
                "a file of {len} bytes was not refused on size"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_ours_is_refused_on_the_first_comparison() {
        let mut bytes = sample().to_bytes();
        write::<{ offset::MAGIC }, 8>(&mut bytes, b"NOTOURS!");
        // The checksum still matches, so the only thing refusing this is the magic.
        assert!(matches!(
            VaultHeader::parse(&bytes),
            Err(CryptoError::HeaderMagic)
        ));
    }

    #[test]
    fn an_unknown_format_version_is_refused_rather_than_guessed_at() {
        let mut bytes = sample().to_bytes();
        write::<{ offset::FORMAT_VERSION }, 2>(&mut bytes, &2_u16.to_le_bytes());
        assert!(matches!(
            VaultHeader::parse(&bytes),
            Err(CryptoError::HeaderVersion { found: 2 })
        ));
    }

    #[test]
    fn a_non_zero_reserved_field_is_refused() {
        let mut bytes = sample().to_bytes();
        write::<{ offset::RESERVED }, 2>(&mut bytes, &1_u16.to_le_bytes());
        assert!(matches!(
            VaultHeader::parse(&bytes),
            Err(CryptoError::HeaderReserved)
        ));
    }

    #[test]
    fn parameters_below_the_floor_are_refused() {
        // The cheapest attack on this design: edit the file so guessing the password costs
        // less. It fails here, before the password is asked for, and it would fail again at
        // the unwrapping because these bytes are inside the associated data.
        let mut bytes = sample().to_bytes();
        write::<{ offset::ARGON2_MEMORY }, 4>(&mut bytes, &(8_u32 * 1024).to_le_bytes());
        refresh_checksum(&mut bytes);

        assert!(matches!(
            VaultHeader::parse(&bytes),
            Err(CryptoError::ParamOutOfRange {
                field: "argon2_m_kib",
                ..
            })
        ));
    }

    #[test]
    fn parameters_above_the_ceiling_are_refused_before_anything_is_allocated() {
        // Four gibibytes. Without the ceiling this would be an allocation the machine cannot
        // make, attempted every time the file is opened, before a single byte is validated.
        let mut bytes = sample().to_bytes();
        write::<{ offset::ARGON2_MEMORY }, 4>(&mut bytes, &(4_u32 * 1024 * 1024).to_le_bytes());
        refresh_checksum(&mut bytes);

        assert!(matches!(
            VaultHeader::parse(&bytes),
            Err(CryptoError::ParamOutOfRange {
                field: "argon2_m_kib",
                ..
            })
        ));
    }

    #[test]
    fn a_corrupt_trailer_is_noticed() {
        let mut bytes = sample().to_bytes();
        write::<{ offset::FAILED_ATTEMPTS }, 4>(&mut bytes, &7_u32.to_le_bytes());
        // Deliberately not recomputing the checksum: this is corruption, not an edit.
        assert!(matches!(
            VaultHeader::parse(&bytes),
            Err(CryptoError::HeaderTrailerChecksum)
        ));
    }

    #[test]
    fn the_trailer_is_not_protected_from_anybody_who_can_write_the_file() {
        // The honest counterpart to the test above, written as a test so that nobody has to
        // take the documentation's word for it. Reset the counter, recompute the checksum,
        // and the header parses. That is a property of the design, not a defect in it: the
        // counter is written after a failed unlock, which is exactly when there is no key to
        // authenticate it with.
        let mut bytes = sample().to_bytes();
        write::<{ offset::FAILED_ATTEMPTS }, 4>(&mut bytes, &0_u32.to_le_bytes());
        write::<{ offset::LOCKED_UNTIL }, 8>(&mut bytes, &0_i64.to_le_bytes());
        refresh_checksum(&mut bytes);

        let header = VaultHeader::parse(&bytes).unwrap();
        assert_eq!(header.failed_attempts(), 0);
        assert_eq!(header.locked_until_us(), 0);
    }

    #[test]
    fn recording_an_attempt_touches_nothing_the_tag_covers() {
        // The reason the tail exists. Writing it must not change a byte of the authenticated
        // part, because re-wrapping the data key needs the password that was just got wrong.
        let header = sample();
        let before = header.to_bytes();

        let mut after_header = sample();
        after_header.record_attempt(3, 1_700_000_100_000_000);
        let after = after_header.to_bytes();

        assert_eq!(
            read::<0, TRAILER_START>(&before),
            read::<0, TRAILER_START>(&after),
            "recording a failed attempt changed an authenticated byte"
        );
        assert_ne!(before, after, "recording a failed attempt changed nothing");
        assert_eq!(after_header.wrap_aad(), header.wrap_aad());
    }

    #[test]
    fn the_recorded_attempt_is_what_comes_back_out() {
        // Read through the accessors rather than through the comparison of two headers,
        // because the comparison would still pass if both sides reported the same wrong
        // number. This is what the lock screen asks for, and a counter that always answers
        // zero is a lockout that never happens.
        let mut header = sample();
        header.record_attempt(7, 1_700_000_900_000_000);

        assert_eq!(header.failed_attempts(), 7);
        assert_eq!(header.locked_until_us(), 1_700_000_900_000_000);

        let restored = VaultHeader::parse(&header.to_bytes()).unwrap();
        assert_eq!(restored.failed_attempts(), 7);
        assert_eq!(restored.locked_until_us(), 1_700_000_900_000_000);
    }

    #[test]
    fn rewrapping_keeps_the_key_identifier_and_the_creation_time() {
        // The property the whole hierarchy exists for, at the level of the header: changing
        // the password changes the salt and the wrapping, and nothing that anything else
        // depends on.
        let original = sample();
        let rewrapped = original.rewrapped(
            [0x55; SALT_LEN],
            Argon2Params::new(48 * 1024, 3, 1).unwrap(),
            1_700_000_500_000_000,
            MasterPassword::Changed,
        );

        assert_eq!(rewrapped.key_id(), original.key_id());
        assert_eq!(rewrapped.created_at_us(), original.created_at_us());
        assert_eq!(rewrapped.master_change_count(), 1);
        assert_eq!(rewrapped.header_rev(), original.header_rev() + 1);
        assert_eq!(rewrapped.params_written_at_us(), 1_700_000_500_000_000);
        assert_eq!(rewrapped.failed_attempts(), 0);
    }

    #[test]
    fn changing_the_parameters_does_not_count_as_changing_the_password() {
        let original = sample();
        let rewrapped = original.rewrapped(
            [0x55; SALT_LEN],
            Argon2Params::new(48 * 1024, 3, 1).unwrap(),
            1_700_000_500_000_000,
            MasterPassword::Kept,
        );
        assert_eq!(rewrapped.master_change_count(), 0);
        assert_eq!(rewrapped.header_rev(), original.header_rev() + 1);
    }

    #[test]
    fn the_counters_stop_rather_than_wrap() {
        // Unreachable by a person, who would have to change the password four billion times.
        // Written anyway, because a counter that silently returns to zero is worse than one
        // that stops, and overflow checks are on in release builds.
        let mut header = sample();
        for _ in 0..2 {
            header = header.rewrapped(
                [0x55; SALT_LEN],
                Argon2Params::DEFAULT,
                0,
                MasterPassword::Changed,
            );
        }

        let saturated = VaultHeader::parse(&{
            let mut bytes = header.to_bytes();
            write::<{ offset::MASTER_CHANGE_COUNT }, 4>(&mut bytes, &u32::MAX.to_le_bytes());
            write::<{ offset::HEADER_REV }, 4>(&mut bytes, &u32::MAX.to_le_bytes());
            bytes
        })
        .unwrap()
        .rewrapped(
            [0x66; SALT_LEN],
            Argon2Params::DEFAULT,
            0,
            MasterPassword::Changed,
        );

        assert_eq!(saturated.master_change_count(), u32::MAX);
        assert_eq!(saturated.header_rev(), u32::MAX);
    }

    #[test]
    fn the_associated_data_is_the_authenticated_prefix_and_nothing_else() {
        let header = sample();
        let bytes = header.to_bytes();
        assert_eq!(
            header.wrap_aad().as_bytes(),
            &read::<0, AUTHENTICATED_PREFIX_LEN>(&bytes)
        );
    }

    #[test]
    fn mutating_any_single_byte_never_panics() {
        // The test that matters most in this file. Every one of the hundred and sixty-eight
        // bytes, flipped one bit at a time, has to come back as a header or as an error and
        // never as a crash. This is the one input in the design that somebody who has stolen
        // the file gets to write.
        let original = sample().to_bytes();

        for index in 0..HEADER_LEN {
            for pattern in [0x01_u8, 0x80, 0xff] {
                let mut damaged = original;
                // The index is bounded by the loop and the array is exactly HEADER_LEN long.
                if let Some(byte) = damaged.get_mut(index) {
                    *byte ^= pattern;
                }
                // The result is deliberately ignored. What is under test is that returning at
                // all is possible, not what it returns.
                let _ = VaultHeader::parse(&damaged);
            }
        }
    }

    #[test]
    fn mutating_any_byte_of_the_prefix_changes_the_associated_data() {
        // The other half of the anti-tampering claim. If a byte of the prefix could change
        // without the associated data changing, that byte would not be protected, whatever
        // the layout table says.
        let original = sample();
        let baseline = original.wrap_aad();

        for index in 0..AUTHENTICATED_PREFIX_LEN {
            let mut damaged = original.to_bytes();
            if let Some(byte) = damaged.get_mut(index) {
                *byte ^= 0x01;
            }
            refresh_checksum(&mut damaged);

            // Some mutations make the header unparseable, which is a stronger outcome than a
            // changed tag and counts as protected.
            if let Ok(parsed) = VaultHeader::parse(&damaged) {
                assert_ne!(
                    parsed.wrap_aad(),
                    baseline,
                    "byte {index} of the authenticated prefix is not actually authenticated"
                );
            }
        }
    }

    #[test]
    fn mutating_a_byte_of_the_trailer_never_changes_the_associated_data() {
        // The complement, and the honest one. These bytes are outside the tag on purpose and
        // this proves it rather than asserting it in prose.
        let original = sample();
        let baseline = original.wrap_aad();

        for index in TRAILER_START..HEADER_LEN {
            let mut damaged = original.to_bytes();
            if let Some(byte) = damaged.get_mut(index) {
                *byte ^= 0x01;
            }
            refresh_checksum(&mut damaged);

            if let Ok(parsed) = VaultHeader::parse(&damaged) {
                assert_eq!(
                    parsed.wrap_aad(),
                    baseline,
                    "byte {index} of the trailer moved the associated data"
                );
            }
        }
    }

    /// Recomputes the trailer checksum after an edit, the way somebody with the file would.
    fn refresh_checksum(bytes: &mut [u8; HEADER_LEN]) {
        let covered = read::<{ offset::FAILED_ATTEMPTS }, TRAILER_COVERED_LEN>(bytes);
        write::<{ offset::TRAILER_CRC }, 4>(bytes, &trailer_checksum(&covered).to_le_bytes());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(if cfg!(miri) { 2 } else { 256 }))]

        /// Any header that can be built survives a trip through bytes unchanged.
        #[test]
        fn a_header_always_round_trips(
            salt in any::<[u8; SALT_LEN]>(),
            key_id in any::<[u8; ID_LEN]>(),
            created_at in any::<i64>(),
            nonce in any::<[u8; NONCE_LEN]>(),
            wrapped in any::<[u8; WRAPPED_DEK_LEN]>(),
            failed in any::<u32>(),
            locked_until in any::<i64>(),
            memory_kib in 32_u32..=1024,
            passes in 3_u32..=16,
            lanes in 1_u32..=4,
        ) {
            let params = Argon2Params::new(memory_kib * 1024, passes, lanes).unwrap();

            let mut header = VaultHeader::new(salt, params, key_id, created_at);
            header.set_wrapped(nonce, wrapped);
            header.record_attempt(failed, locked_until);

            let restored = VaultHeader::parse(&header.to_bytes()).unwrap();
            prop_assert_eq!(&restored, &header);
        }

        /// No sequence of bytes of any length makes the parser panic.
        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = VaultHeader::parse(&bytes);
            prop_assert!(true);
        }

        /// Arbitrary bytes at exactly the right length never panic either.
        ///
        /// A separate property because the length check rejects almost everything the one
        /// above generates, so without this the parser past that first line is barely
        /// exercised.
        #[test]
        fn arbitrary_bytes_of_the_right_length_never_panic(
            bytes in proptest::collection::vec(any::<u8>(), HEADER_LEN..=HEADER_LEN)
        ) {
            let _ = VaultHeader::parse(&bytes);
            prop_assert!(true);
        }

        /// A header built from valid bytes always answers the parameters it was built with.
        #[test]
        fn the_parameters_survive_the_round_trip(
            memory_kib in 32_u32..=1024,
            passes in 3_u32..=16,
            lanes in 1_u32..=4,
        ) {
            let params = Argon2Params::new(memory_kib * 1024, passes, lanes).unwrap();
            let header = VaultHeader::new([0; SALT_LEN], params, [0; ID_LEN], 0);
            let restored = VaultHeader::parse(&header.to_bytes()).unwrap();

            prop_assert_eq!(restored.params(), params);
        }
    }
}
