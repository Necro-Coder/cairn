//! The identifier of this installation, and the small sealed file it lives in.
//!
//! Every row records which device wrote it, and the hybrid logical clock that orders writes
//! takes its tie-break from the same value. So it has to exist before the first row is written,
//! be the same on every later start, and be impossible to change without the change being
//! noticed.
//!
//! A file of its own rather than a column in the database or a field in the vault header. A
//! column would be unavailable exactly when it is most needed, which is while the database is
//! being created; the header is full and has two frozen vaults that every future build has to
//! keep opening, so growing it would mean a second format version for sixteen bytes. The file
//! is sealed with the data key under associated data of its own, so it is readable the instant
//! the vault opens, it survives the database being deleted, and editing a byte of it fails the
//! tag instead of silently changing which device this is.

use std::fs;
use std::path::Path;

use cairn_crypto::{Aad, DataKey, FreshNonce, ID_LEN, Sealed, open, seal};
use uuid::Uuid;

use crate::error::DbError;

/// The table name that goes into the associated data of the device file.
///
/// Not a table. It is the name this value is filed under, and it is in the same position as a
/// table name so that the encoding is the one the rest of the project uses and there is not a
/// second layout to reason about.
const DEVICE_AAD_TABLE: &str = "cairn.device";

/// The column name that goes into the associated data of the device file.
const DEVICE_AAD_COLUMN: &str = "device_id";

/// The identifier of this installation.
///
/// A version four UUID. The first six bytes are what the logical clock uses to break ties, which
/// is why it is read from here rather than generated separately: two identifiers that can drift
/// apart are two identifiers one of which is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(Uuid);

impl DeviceId {
    /// Reads a new identifier from the operating system.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Sealed`] if the operating system will not provide random bytes.
    pub fn generate() -> Result<Self, DbError> {
        let mut bytes = [0_u8; ID_LEN];
        cairn_crypto::fill_random(&mut bytes)?;

        Ok(Self(uuid::Builder::from_random_bytes(bytes).into_uuid()))
    }

    /// The identifier a stored row carries, read back from its sixteen bytes.
    ///
    /// Any sixteen bytes are accepted. What the schema guarantees is the length; whether they
    /// name a device this vault has ever seen is a question for the merge, not for a decoder.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; ID_LEN]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    /// The identifier as a UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// The sixteen bytes, as they are stored in every row.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; ID_LEN] {
        self.0.as_bytes()
    }

    /// The forty-eight bits the logical clock breaks ties with.
    ///
    /// The first six bytes of the identifier rather than a separate value, so there is nothing
    /// to keep in step. What the clock needs is a number that differs between devices and never
    /// changes on one, and six bytes of a version four UUID is that.
    #[must_use]
    pub fn tie_break(&self) -> [u8; 6] {
        let bytes = self.0.as_bytes();
        let mut short = [0_u8; 6];
        short.copy_from_slice(&bytes[..6]);

        short
    }
}

/// Reads the identifier of this installation, creating it the first time.
///
/// Called on the first unlock. Creating it needs the data key, which is another reason it is not
/// read at startup: before the vault is open there is nothing to seal it with.
///
/// # Errors
///
/// Returns [`DbError::Io`] if the file cannot be read or written, and [`DbError::Sealed`] if it
/// is there and does not decrypt, which means somebody edited it or it is from another vault.
/// Both are refusals rather than reasons to make a new identifier: quietly minting a second one
/// would make every row this machine has already written look as if another device wrote it.
pub fn load_or_create(
    path: &Path,
    key: &DataKey,
    key_id: &[u8; ID_LEN],
) -> Result<DeviceId, DbError> {
    if let Some(existing) = load(path, key, key_id)? {
        return Ok(existing);
    }

    let device = DeviceId::generate()?;
    store(path, key, key_id, device)?;

    Ok(device)
}

/// Reads the identifier, answering `None` when the file is not there.
///
/// # Errors
///
/// Returns [`DbError::Io`] if the file is there and cannot be read, and [`DbError::Sealed`] if it
/// does not decrypt or is not sixteen bytes long.
pub fn load(
    path: &Path,
    key: &DataKey,
    key_id: &[u8; ID_LEN],
) -> Result<Option<DeviceId>, DbError> {
    let stored = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(cause) => {
            return Err(DbError::Io {
                what: "the device identifier",
                operation: "read",
                cause,
            });
        }
    };

    let sealed = Sealed::from_bytes(&stored)?;
    let plaintext = open(key, &sealed, &associated_data(key_id)?)?;

    let bytes: [u8; ID_LEN] = plaintext
        .as_slice()
        .try_into()
        // Decrypted and still the wrong length means the file was written by something that is
        // not this program. Reported as a value that did not open, because it did not.
        .map_err(|_wrong_length| DbError::Sealed(cairn_crypto::CryptoError::Open))?;

    Ok(Some(DeviceId(Uuid::from_bytes(bytes))))
}

/// Writes the identifier, replacing whatever was there in a single step.
///
/// Through a temporary file and a rename, like the vault header, for the same reason: after a
/// power cut the file is either entirely the old one or entirely the new one. A half written
/// device identifier would fail its tag on the next start and lock somebody out of a database
/// they can otherwise read perfectly.
fn store(
    path: &Path,
    key: &DataKey,
    key_id: &[u8; ID_LEN],
    device: DeviceId,
) -> Result<(), DbError> {
    use std::io::Write as _;

    let sealed = seal(
        key,
        FreshNonce::generate()?,
        &associated_data(key_id)?,
        device.as_bytes(),
    )?;

    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|cause| DbError::Io {
            what: "the device identifier",
            operation: "written, because its directory could not be created",
            cause,
        })?;
    }

    let temporary = path.with_extension("device.new");
    let mut file = fs::File::create(&temporary).map_err(|cause| DbError::Io {
        what: "the device identifier",
        operation: "written to a temporary file",
        cause,
    })?;
    file.write_all(&sealed.to_bytes())
        .map_err(|cause| DbError::Io {
            what: "the device identifier",
            operation: "written",
            cause,
        })?;
    file.sync_all().map_err(|cause| DbError::Io {
        what: "the device identifier",
        operation: "flushed to the disk",
        cause,
    })?;
    drop(file);

    fs::rename(&temporary, path).map_err(|cause| DbError::Io {
        what: "the device identifier",
        operation: "renamed over the old one",
        cause,
    })
}

/// The associated data the device file is sealed under.
fn associated_data(key_id: &[u8; ID_LEN]) -> Result<Aad, DbError> {
    Ok(Aad::record(
        crate::codec::RECORD_FORMAT_VERSION,
        DEVICE_AAD_TABLE,
        &[0_u8; ID_LEN],
        DEVICE_AAD_COLUMN,
        0,
        key_id,
    )?)
}

#[cfg(test)]
mod tests {
    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, UnlockedVault};

    use super::{DeviceId, load, load_or_create};
    use crate::error::DbError;
    use crate::test_support::Scratch;

    fn an_open_vault(password: &str) -> UnlockedVault {
        let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create(password, params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");
        vault
    }

    #[test]
    fn the_identifier_is_created_once_and_read_back_every_time_after() {
        let scratch = Scratch::new("device");
        let path = scratch.directory().join(crate::DEVICE_FILE);
        let vault = an_open_vault("una frase larga para la prueba");

        let first = load_or_create(&path, vault.data_key(), vault.key_id())
            .expect("the identifier can be created");
        let second = load_or_create(&path, vault.data_key(), vault.key_id())
            .expect("the identifier can be read back");

        assert_eq!(first, second);
    }

    #[test]
    fn deleting_the_database_does_not_change_which_device_this_is() {
        // The reason it is a file of its own. A device identifier that changes when the
        // database is rebuilt would make every row written before the rebuild look as if
        // another machine wrote it, which is a merge conflict against yourself.
        let scratch = Scratch::new("device-survives");
        let path = scratch.directory().join(crate::DEVICE_FILE);
        let vault = an_open_vault("una frase larga para la prueba");

        let before = load_or_create(&path, vault.data_key(), vault.key_id()).unwrap();
        std::fs::write(scratch.database_path(), b"not a database").unwrap();
        std::fs::remove_file(scratch.database_path()).unwrap();
        let after = load_or_create(&path, vault.data_key(), vault.key_id()).unwrap();

        assert_eq!(before, after);
    }

    #[test]
    fn a_single_changed_byte_is_refused_rather_than_read() {
        let scratch = Scratch::new("device-tampered");
        let path = scratch.directory().join(crate::DEVICE_FILE);
        let vault = an_open_vault("una frase larga para la prueba");

        load_or_create(&path, vault.data_key(), vault.key_id()).unwrap();

        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&path, &bytes).unwrap();

        assert!(matches!(
            load(&path, vault.data_key(), vault.key_id()),
            Err(DbError::Sealed(_))
        ));
    }

    #[test]
    fn every_byte_of_the_file_is_authenticated() {
        // Flipping a bit anywhere has to be refused, not only at the end. A file where only
        // part of the bytes are covered is a file with a part somebody can edit.
        let scratch = Scratch::new("device-every-byte");
        let path = scratch.directory().join(crate::DEVICE_FILE);
        let vault = an_open_vault("una frase larga para la prueba");

        load_or_create(&path, vault.data_key(), vault.key_id()).unwrap();
        let original = std::fs::read(&path).unwrap();

        for index in 0..original.len() {
            let mut damaged = original.clone();
            damaged[index] ^= 0x80;
            std::fs::write(&path, &damaged).unwrap();

            assert!(
                matches!(
                    load(&path, vault.data_key(), vault.key_id()),
                    Err(DbError::Sealed(_))
                ),
                "a change at byte {index} was accepted"
            );
        }
    }

    #[test]
    fn another_vault_cannot_read_it() {
        let scratch = Scratch::new("device-other-vault");
        let path = scratch.directory().join(crate::DEVICE_FILE);
        let mine = an_open_vault("una frase larga para la prueba");
        let theirs = an_open_vault("otra frase completamente distinta");

        load_or_create(&path, mine.data_key(), mine.key_id()).unwrap();

        assert!(matches!(
            load(&path, theirs.data_key(), theirs.key_id()),
            Err(DbError::Sealed(_))
        ));
    }

    #[test]
    fn a_file_that_is_not_there_is_absence_rather_than_failure() {
        let scratch = Scratch::new("device-absent");
        let vault = an_open_vault("una frase larga para la prueba");

        let answer = load(
            &scratch.directory().join(crate::DEVICE_FILE),
            vault.data_key(),
            vault.key_id(),
        )
        .expect("a missing file is not a failure");

        assert_eq!(answer, None);
    }

    #[test]
    fn two_identifiers_generated_in_a_row_are_different() {
        let first = DeviceId::generate().unwrap();
        let second = DeviceId::generate().unwrap();

        assert_ne!(first, second);
        assert_ne!(first.tie_break(), second.tie_break());
    }

    #[test]
    fn the_tie_break_is_the_first_six_bytes_of_the_identifier() {
        let device = DeviceId::generate().unwrap();
        assert_eq!(device.tie_break(), device.as_bytes()[..6]);
    }

    #[test]
    fn a_generated_identifier_is_a_version_four_uuid() {
        // Version four and the right variant, which is what marks it as random rather than as
        // something derived from a clock or a network address.
        let device = DeviceId::generate().unwrap();

        assert_eq!(device.as_uuid().get_version_num(), 4);
        assert_eq!(device.as_uuid().get_variant(), uuid::Variant::RFC4122);
    }

    #[test]
    fn writing_it_leaves_no_temporary_file_behind() {
        let scratch = Scratch::new("device-temporary");
        let path = scratch.directory().join(crate::DEVICE_FILE);
        let vault = an_open_vault("una frase larga para la prueba");

        load_or_create(&path, vault.data_key(), vault.key_id()).unwrap();

        let leftovers: Vec<String> = std::fs::read_dir(scratch.directory())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec![crate::DEVICE_FILE.to_owned()]);
    }
}
