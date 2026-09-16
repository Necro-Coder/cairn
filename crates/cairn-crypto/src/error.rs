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
