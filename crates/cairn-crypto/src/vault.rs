//! Creating a vault, opening it, and the two operations that rewrite its header.
//!
//! Everything here is a pure function of its arguments. Nothing opens a file, reads a clock
//! or spawns anything, because the moment any of that appears the operations below stop
//! being testable without a temporary directory and a real hour of the day. The caller reads
//! the clock and writes the file; this decides what the bytes should be.
//!
//! The shape worth understanding is what changes and what does not. Creating a vault picks a
//! salt, a data key and an identifier for that key, once, forever. Changing the master
//! password, and changing the Argon2id parameters, both derive a new key encryption key and
//! wrap **the same data key** under it. The data key does not change, so its identifier does
//! not change, so every subkey below it does not change, so not one record and not one byte
//! of the database file is rewritten. That is the property the whole design is arranged
//! around, and it is demonstrated rather than asserted by the test beside this file.
//!
//! One key is deliberately absent from [`UnlockedVault`]. There is no way to ask an open
//! vault for the key a backup is sealed with, because a backup that could only be opened by
//! a machine holding this vault's data key would be unreadable on exactly the machines a
//! backup is for. That key is derived in [`crate::export_key`] from a key encryption key of
//! the backup file's own, over the salt and parameters that file carries. See decision
//! record 0011.

use crate::aead::{Sealed, open, seal};
use crate::error::CryptoError;
use crate::header::{MasterPassword, VaultHeader, WRAPPED_DEK_LEN};
use crate::hierarchy;
use crate::kdf::{Argon2Params, SALT_LEN, derive_kek};
use crate::keys::{DataKey, DatabaseKey, KEY_LEN, Kek, SyncKey};
use crate::nonce::{FreshNonce, NONCE_LEN};
use crate::random;

use uuid::Builder;

/// Length of the identifier of the data key, in bytes.
const KEY_ID_LEN: usize = 16;

/// An open vault: the data key, and the identifier that goes into every record it protects.
///
/// Holds no file handle and no clock. It is what the session keeps in memory while the vault
/// is unlocked, and dropping it clears the key.
#[derive(Debug)]
pub struct UnlockedVault {
    data_key: DataKey,
    key_id: [u8; KEY_ID_LEN],
}

impl UnlockedVault {
    /// The key every record is encrypted with.
    #[must_use]
    pub fn data_key(&self) -> &DataKey {
        &self.data_key
    }

    /// The identifier of that key, which goes into the associated data of every record.
    #[must_use]
    pub fn key_id(&self) -> &[u8; KEY_ID_LEN] {
        &self.key_id
    }

    /// The raw key SQLCipher is given for the whole database file.
    ///
    /// Derived on demand rather than held, so that it exists for as long as the caller keeps
    /// it and no longer. It is the same bytes every time, which is exactly the property that
    /// lets the master password change without the database being touched.
    #[must_use]
    pub fn database_key(&self) -> DatabaseKey {
        hierarchy::database_key(&self.data_key)
    }

    /// The pre-shared key for the synchronisation handshake.
    #[must_use]
    pub fn sync_key(&self) -> SyncKey {
        hierarchy::sync_key(&self.data_key)
    }
}

/// Creates a vault: a fresh salt, a fresh data key, a fresh identifier and a header.
///
/// The caller supplies the moment rather than this reading a clock, so that a test can pin
/// it and the header this produces is a function of its arguments alone.
///
/// # Errors
///
/// Returns [`CryptoError::PasswordTooLong`] for a password over the limit,
/// [`CryptoError::Entropy`] if the operating system will not provide random bytes, and
/// [`CryptoError::Kdf`] if Argon2id cannot run at the parameters given.
pub fn create(
    password: &str,
    params: Argon2Params,
    now_us: i64,
) -> Result<(VaultHeader, UnlockedVault), CryptoError> {
    let mut kdf_salt = [0_u8; SALT_LEN];
    random::fill(&mut kdf_salt)?;

    let key_id = fresh_key_id()?;
    let data_key = DataKey::generate()?;
    let kek = derive_kek(password, &kdf_salt, params)?;

    let mut header = VaultHeader::new(kdf_salt, params, key_id, now_us);
    let (wrap_nonce, wrapped_dek) = wrap(&kek, &data_key, &header)?;
    header.set_wrapped(wrap_nonce, wrapped_dek);

    Ok((header, UnlockedVault { data_key, key_id }))
}

/// Opens a vault with its master password.
///
/// # Errors
///
/// Returns [`CryptoError::Open`] for anything that means the vault did not open: the wrong
/// password, a header somebody edited, a flipped bit. They are deliberately the same error,
/// because any difference between them is an oracle. A password over the length limit and a
/// derivation this machine cannot run are reported as themselves, because neither says
/// anything about whether the password was right.
pub fn unlock(header: &VaultHeader, password: &str) -> Result<UnlockedVault, CryptoError> {
    let kek = derive_kek(password, header.kdf_salt(), header.params())?;
    let data_key = unwrap(&kek, header)?;

    Ok(UnlockedVault {
        data_key,
        key_id: *header.key_id(),
    })
}

/// Changes the master password, keeping the same data key.
///
/// A new salt as well as a new key encryption key. Reusing the salt would mean the old and
/// the new password derive from the same starting point, which gives away little on its own
/// and costs nothing to avoid.
///
/// # Errors
///
/// Returns [`CryptoError::Open`] if the current password does not open the vault, and the
/// same errors as [`create`] otherwise.
pub fn change_password(
    header: &VaultHeader,
    current: &str,
    new: &str,
    now_us: i64,
) -> Result<(VaultHeader, UnlockedVault), CryptoError> {
    rewrap(
        header,
        current,
        new,
        header.params(),
        now_us,
        MasterPassword::Changed,
    )
}

/// Changes the Argon2id parameters, keeping the same password and the same data key.
///
/// The operation the whole hierarchy is arranged to make cheap. Nothing outside the header
/// changes: not the data key, not its identifier, not the key the database is encrypted
/// with, and therefore not a single stored byte.
///
/// # Errors
///
/// Returns [`CryptoError::Open`] if the password does not open the vault, and the same errors
/// as [`create`] otherwise. Parameters outside the allowed range cannot be passed in, because
/// [`Argon2Params`] cannot be built out of range.
pub fn change_kdf_params(
    header: &VaultHeader,
    password: &str,
    params: Argon2Params,
    now_us: i64,
) -> Result<(VaultHeader, UnlockedVault), CryptoError> {
    rewrap(
        header,
        password,
        password,
        params,
        now_us,
        MasterPassword::Kept,
    )
}

/// The shared body of the two operations above.
///
/// One function rather than two, because the difference between them is which arguments
/// change and not what happens. Two copies would be two places for the data key to be
/// regenerated by accident in one of them, and that single mistake would make the operation
/// lose every record in the vault.
fn rewrap(
    header: &VaultHeader,
    current: &str,
    new: &str,
    params: Argon2Params,
    now_us: i64,
    master_password: MasterPassword,
) -> Result<(VaultHeader, UnlockedVault), CryptoError> {
    // Opened first. Neither operation may produce anything if the password is wrong, and the
    // only way to know it is right is to unwrap with it.
    let opened = unlock(header, current)?;

    let mut kdf_salt = [0_u8; SALT_LEN];
    random::fill(&mut kdf_salt)?;

    let kek = derive_kek(new, &kdf_salt, params)?;

    let mut rewritten = header.rewrapped(kdf_salt, params, now_us, master_password);
    let (wrap_nonce, wrapped_dek) = wrap(&kek, &opened.data_key, &rewritten)?;
    rewritten.set_wrapped(wrap_nonce, wrapped_dek);

    Ok((rewritten, opened))
}

/// A fresh identifier for a data key.
///
/// A version four UUID built from our own randomness rather than from the one the library
/// would read for itself, so that this crate keeps a single door to the operating system.
fn fresh_key_id() -> Result<[u8; KEY_ID_LEN], CryptoError> {
    let mut bytes = [0_u8; KEY_ID_LEN];
    random::fill(&mut bytes)?;
    Ok(*Builder::from_random_bytes(bytes).into_uuid().as_bytes())
}

/// Seals the data key under the key derived from the key encryption key.
///
/// The associated data is the whole authenticated prefix of the header the wrapping is going
/// into, which is why this takes the header rather than the fields of it that seemed
/// relevant. The prefix ends before the two fields this produces, so there is no circularity:
/// the bytes being authenticated are settled before the wrapping happens.
fn wrap(
    kek: &Kek,
    data_key: &DataKey,
    header: &VaultHeader,
) -> Result<([u8; NONCE_LEN], [u8; WRAPPED_DEK_LEN]), CryptoError> {
    let wrap_key = hierarchy::wrap_key(kek);

    let sealed = seal(
        &wrap_key,
        FreshNonce::generate()?,
        &header.wrap_aad(),
        data_key.expose(),
    )?;

    let wrapped: [u8; WRAPPED_DEK_LEN] = sealed
        .ciphertext()
        .try_into()
        // A thirty-two byte key seals to exactly forty-eight bytes, so this cannot happen.
        // It is mapped to the ordinary failure rather than unwrapped, because a length that
        // surprised us is a reason to refuse rather than a reason to stop the process.
        .map_err(|_| CryptoError::Open)?;

    Ok((*sealed.nonce(), wrapped))
}

/// Opens the wrapped data key.
///
/// Every way this can fail is the same failure to the caller. A wrong password, a header
/// somebody edited, a flipped bit on a disk: telling them apart would tell an attacker which
/// half of the problem to work on.
fn unwrap(kek: &Kek, header: &VaultHeader) -> Result<DataKey, CryptoError> {
    let wrap_key = hierarchy::wrap_key(kek);
    let (nonce, wrapped) = header.wrapped_dek();

    let mut stored = Vec::with_capacity(NONCE_LEN + wrapped.len());
    stored.extend_from_slice(&nonce);
    stored.extend_from_slice(&wrapped);

    let sealed = Sealed::from_bytes(&stored)?;
    let opened = open(&wrap_key, &sealed, &header.wrap_aad())?;

    let bytes: [u8; KEY_LEN] = opened
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::Open)?;

    Ok(DataKey::from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::{create, fresh_key_id};
    use crate::kdf::{Argon2Params, MIN_MEMORY_KIB, MIN_PASSES};

    const PASSWORD: &str = "una contrasena de ejemplo para las pruebas";
    const CREATED_AT_US: i64 = 1_700_000_000_000_000;

    fn cheap() -> Argon2Params {
        Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, 1).unwrap()
    }

    #[test]
    fn every_key_identifier_is_its_own() {
        // The identifier goes into the associated data of every record, so two vaults sharing
        // one would mean a record from either opening against the other's key. Asserted on
        // the generator directly as well as through a created vault, because a generator that
        // returned a constant would still produce a vault that opens.
        let first = fresh_key_id().unwrap();
        let second = fresh_key_id().unwrap();

        assert_ne!(first, second);
        assert_ne!(first, [0; 16]);
        assert_ne!(second, [0; 16]);
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "Argon2id is memory hard on purpose, and the interpreter is several orders of magnitude slower than the processor"
    )]
    fn two_vaults_created_with_the_same_password_are_different_vaults() {
        let (first, _) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();
        let (second, _) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();

        assert_ne!(first.key_id(), second.key_id());
        assert_ne!(first.kdf_salt(), second.kdf_salt());
        assert_ne!(first.to_bytes(), second.to_bytes());
    }
}
