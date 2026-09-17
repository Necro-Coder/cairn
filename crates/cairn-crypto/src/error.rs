//! The one error type this crate returns.
//!
//! There is a single enum rather than one per module because of what the caller is
//! allowed to learn. The command boundary maps every failure of an unlock to one message,
//! and the surest way to keep that promise is for the layer underneath to have already
//! collapsed the interesting cases: [`CryptoError::Open`] covers a wrong key, associated
//! data that does not match, a flipped bit and a truncated blob, and it carries nothing
//! that tells them apart.
//!
//! The variants that do carry numbers are the ones about sizes the caller chose. A length
//! it passed in is not a secret, and refusing to say which limit was exceeded would turn a
//! programming mistake into an afternoon of guessing.

/// Everything that can go wrong inside this crate.
///
/// Marked non-exhaustive on purpose: adding a variant later must not break a caller that
/// already handles the ones it cares about and falls through on the rest.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CryptoError {
    /// The operating system refused to hand over random bytes.
    ///
    /// There is no fallback and there will never be one. Carrying on with a weaker source
    /// would produce a key or a nonce that looks exactly like a good one and is not.
    #[error("the operating system did not provide random bytes")]
    Entropy,

    /// Authenticated decryption failed.
    ///
    /// Deliberately says nothing else. A wrong key, associated data that does not match, a
    /// single flipped bit and a blob that was cut short all arrive here, and anything that
    /// distinguished them would be an oracle.
    #[error("authenticated decryption failed")]
    Open,

    /// A field of the associated data was longer than the format allows.
    #[error("associated data field {field} is {len} bytes, and at most {max} are allowed")]
    FieldTooLong {
        /// Which field. A fixed name from this crate, never text supplied by a caller.
        field: &'static str,
        /// How long it was, in bytes.
        len: usize,
        /// How long it is allowed to be, in bytes.
        max: usize,
    },

    /// The plaintext handed to [`crate::seal`] was longer than one record may be.
    #[error("plaintext is {len} bytes, and at most {max} are allowed in one record")]
    PlaintextTooLong {
        /// How long it was, in bytes.
        len: usize,
        /// How long it is allowed to be, in bytes.
        max: usize,
    },

    /// A master password was longer than this accepts.
    ///
    /// Not a rule about what makes a good password, which is a decision for the layer that
    /// talks to a person. It is a bound on work: Argon2id hashes whatever it is handed, so
    /// a field somebody pasted a file into is a denial of service with no attacker in it.
    #[error("the password is {len} bytes, and at most {max} are accepted")]
    PasswordTooLong {
        /// How long it was, in bytes of UTF-8.
        len: usize,
        /// How long it is allowed to be.
        max: usize,
    },

    /// An Argon2id parameter was outside the range a vault may ask for.
    ///
    /// Says which field and which bound, because the number came out of a file rather than
    /// out of a person. The floor stops somebody making a brute force attempt cheap; the
    /// ceiling stops a header claiming more memory than the machine has.
    #[error("{field} is {value}, and the allowed range is {min} to {max}")]
    ParamOutOfRange {
        /// Which field, named as it is in the header layout.
        field: &'static str,
        /// What the header asked for.
        value: u32,
        /// The lowest value allowed.
        min: u32,
        /// The highest value allowed.
        max: u32,
    },

    /// Argon2id itself refused to run.
    ///
    /// In practice this means the parameters, though inside the allowed range, could not be
    /// turned into an allocation on this machine. It is deliberately separate from a failed
    /// unwrapping, because it is a fact about the machine rather than about the password.
    #[error("the key derivation could not be run")]
    Kdf,

    /// The header file was not exactly the size a header is.
    ///
    /// The first thing checked and the likeliest thing a power cut leaves behind, which is
    /// why the format has one fixed size rather than a length field to be trusted.
    #[error("a vault header cannot be {len} bytes long")]
    HeaderSize {
        /// How many bytes were offered.
        len: usize,
    },

    /// The file did not begin with the eight bytes every header of ours begins with.
    #[error("this is not a vault header")]
    HeaderMagic,

    /// The header announced a format version this build does not know.
    ///
    /// Refused rather than read hopefully. A reader that guesses at a layout it has never
    /// seen is a reader that hands out the wrong bytes as a key.
    #[error("this vault header is version {found}, which this build cannot read")]
    HeaderVersion {
        /// The version the file claimed.
        found: u16,
    },

    /// The reserved field was not zero.
    #[error("the reserved field of the vault header is not zero")]
    HeaderReserved,

    /// The checksum over the unauthenticated tail did not match.
    ///
    /// Detects corruption, not tampering. Anybody who can write the file can recompute the
    /// checksum, and that is written down here and in the public documentation rather than
    /// left for somebody to discover.
    #[error("the vault header trailer is corrupt")]
    HeaderTrailerChecksum,

    /// The file does not begin the way a backup of ours begins.
    ///
    /// Covers a wrong length and a wrong magic together, because they answer one question
    /// and a caller told which of the two it was learns nothing it can act on. It is the
    /// first of the four things an import is ever allowed to say.
    #[error("this is not a Cairn backup")]
    NotABackup,

    /// The backup announced a format version this build cannot read.
    ///
    /// Older than the conversion path covers, or newer than this build knows about.
    /// Refused rather than read hopefully: guessing at a layout somebody else defined is
    /// how a restore corrupts data with the best of intentions.
    #[error("this backup is version {found}, which this build cannot read")]
    BackupVersion {
        /// The version the file claimed.
        found: u16,
    },

    /// The backup was compressed with something this build does not carry.
    #[error("this backup uses compression {found}, which this build does not carry")]
    BackupCompression {
        /// The code the file claimed.
        found: u16,
    },

    /// The reserved field of the backup header was not zero.
    ///
    /// Refused so that a later version of the format can put something there and know for
    /// certain that no file of this version ever meant anything by those bytes.
    #[error("the reserved field of the backup header is not zero")]
    BackupReserved,

    /// The file ended somewhere a file cannot end.
    ///
    /// Structural, decided before any key is involved: a body with no room for a tag is not
    /// a body under any key at all, so saying so is not an oracle. Anything that depends on
    /// a key arrives as [`CryptoError::Open`] instead.
    #[error("the backup is damaged or incomplete")]
    Damaged,

    /// One nonce base cannot cover another chunk.
    ///
    /// Refusing rather than wrapping. A wrapped counter reuses the nonce of the first
    /// chunk, and losing an export is a far better outcome than writing one that leaks the
    /// exclusive or of two chunks and can have its tags forged.
    #[error("this backup has more chunks than one nonce base may cover")]
    ChunkCountExhausted,

    /// A stored blob was too short to be a nonce followed by a tag.
    ///
    /// This is a structural check that runs before any key is involved, which is why it is
    /// allowed to be its own variant: it says nothing about whether the key is the right
    /// one, because at that point no key has been looked at.
    #[error("a sealed value cannot be {len} bytes long")]
    MalformedSealed {
        /// How many bytes were offered.
        len: usize,
    },
}
