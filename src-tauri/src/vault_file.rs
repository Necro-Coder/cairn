//! Reading and writing the vault header file, and recovering from an interrupted write.
//!
//! A hundred and sixty-eight bytes, and the most dangerous file in the application. Lose it
//! and everything else on disk is a large amount of unreadable ciphertext: there is no
//! recovery phrase, no hint and no second door, on purpose. So every write goes to a
//! temporary file and is renamed over the real one, which the operating system does as a
//! single step: after a power cut the file is either entirely the old one or entirely the
//! new one, and never half of each.
//!
//! The rename is what makes the header safe. The copy taken before the two rewriting
//! operations covers what the rename cannot: a disk that stops working in the middle. It is
//! read back and parsed before the operation continues, because a backup that turns out to
//! be corrupt is discovered at the only moment it is no use, which is when it is needed.
//!
//! Nothing here decides anything about keys. It moves bytes that the cryptographic core
//! produced, and hands back bytes for that core to interpret.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use cairn_crypto::{CryptoError, HEADER_LEN, VaultHeader};

/// What the vault occupies on disk.
///
/// Three paths beside each other rather than a directory of their own, because the database
/// file lands here too and a vault is the set of them.
#[derive(Debug, Clone)]
pub struct VaultPaths {
    header: PathBuf,
    backup: PathBuf,
    temporary: PathBuf,
}

impl VaultPaths {
    /// The three paths a vault uses, inside the directory it lives in.
    #[must_use]
    pub fn in_directory(directory: &Path) -> Self {
        Self {
            header: directory.join(cairn_db::HEADER_FILE),
            backup: directory.join(format!("{}.backup", cairn_db::HEADER_FILE)),
            temporary: directory.join(format!("{}.new", cairn_db::HEADER_FILE)),
        }
    }

    /// Where the header itself lives.
    #[must_use]
    pub fn header(&self) -> &Path {
        &self.header
    }

    /// Where the copy taken before a rewrite lives.
    #[must_use]
    pub fn backup(&self) -> &Path {
        &self.backup
    }
}

/// What went wrong while handling the file.
///
/// Separate from the cryptographic errors because these are about the disk rather than about
/// the password. The command boundary still collapses everything an unlock can produce into
/// one message; this exists so that whoever is repairing a machine can still tell which of
/// them it was.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VaultFileError {
    /// The file could not be read, written or replaced.
    #[error("the vault header could not be {operation}")]
    Io {
        /// What was being attempted, in a form that fits the sentence above.
        operation: &'static str,
        /// The underlying failure, kept so the cause is not lost on the way out.
        #[source]
        cause: io::Error,
    },

    /// The bytes on disk are not a header this build can read.
    #[error("the vault header on disk could not be read")]
    Malformed(#[source] CryptoError),

    /// A copy was taken and did not come back the same.
    ///
    /// The whole point of verifying. A backup nobody checked is a backup discovered to be
    /// useless at the one moment it was needed.
    #[error("the backup of the vault header did not read back correctly")]
    BackupNotVerified,
}

/// What reading the vault at startup found, and what it had to do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// No header and no backup. This machine has no vault yet.
    NothingThere,
    /// The header was there and readable. Nothing had to be done.
    HeaderWasFine,
    /// The header was unreadable and the backup was used instead.
    ///
    /// Reported rather than done quietly, because somebody who opens the application after a
    /// power cut deserves to be told that a copy was restored.
    RestoredFromBackup,
}

/// Reads the vault header, recovering from the backup if the header itself is unreadable.
///
/// Called once at startup. The order matters: the header is tried first, so a perfectly good
/// file is never replaced by an older copy of itself.
///
/// # Errors
///
/// Returns [`VaultFileError::Io`] if the directory cannot be read or written, and
/// [`VaultFileError::Malformed`] if neither the header nor the backup is a header this build
/// can read, which means there is nothing left to recover from.
pub fn read_or_recover(
    paths: &VaultPaths,
) -> Result<(Recovery, Option<VaultHeader>), VaultFileError> {
    let header = read_header(&paths.header)?;
    let backup_exists = paths.backup.exists();

    if let Some(header) = header {
        if backup_exists {
            // The header is good, so whatever the copy was taken for either finished or
            // never started. Either way the copy has no job left, and leaving it behind
            // means the next recovery would consider a stale file.
            remove(&paths.backup)?;
        }
        return Ok((Recovery::HeaderWasFine, Some(header)));
    }

    if !backup_exists {
        // Distinguishable from a corrupt header on purpose: this is a machine with no vault,
        // which is the ordinary first run rather than a failure.
        if paths.header.exists() {
            return Err(VaultFileError::Malformed(CryptoError::HeaderSize {
                len: 0,
            }));
        }
        return Ok((Recovery::NothingThere, None));
    }

    let Some(recovered) = read_header(&paths.backup)? else {
        return Err(VaultFileError::Malformed(CryptoError::HeaderSize {
            len: 0,
        }));
    };

    write(&paths.header, &paths.temporary, &recovered.to_bytes())?;
    remove(&paths.backup)?;

    Ok((Recovery::RestoredFromBackup, Some(recovered)))
}

/// Writes a header, replacing whatever was there in a single step.
///
/// # Errors
///
/// Returns [`VaultFileError::Io`] if the temporary file cannot be written, flushed or
/// renamed over the header.
pub fn write_header(paths: &VaultPaths, header: &VaultHeader) -> Result<(), VaultFileError> {
    write(&paths.header, &paths.temporary, &header.to_bytes())
}

/// Takes a copy of the current header and checks that the copy is readable.
///
/// Both operations that rewrite the header call this first. Reading a hundred and sixty-eight
/// bytes back costs microseconds, and it is the difference between having a backup and
/// believing you have one.
///
/// # Errors
///
/// Returns [`VaultFileError::Io`] if the copy cannot be written, and
/// [`VaultFileError::BackupNotVerified`] if what comes back is not byte for byte what went
/// in, or is not a header that parses.
pub fn back_up_verified(paths: &VaultPaths) -> Result<(), VaultFileError> {
    let current = fs::read(&paths.header).map_err(|cause| VaultFileError::Io {
        operation: "read before taking a copy",
        cause,
    })?;

    write(&paths.backup, &paths.temporary, &current)?;

    let written = fs::read(&paths.backup).map_err(|cause| VaultFileError::Io {
        operation: "read back after taking a copy",
        cause,
    })?;

    if written != current {
        return Err(VaultFileError::BackupNotVerified);
    }
    // Parsed as well as compared. Two identical files that are both unreadable would pass a
    // comparison and fail the only job the copy has.
    if VaultHeader::parse(&written).is_err() {
        return Err(VaultFileError::BackupNotVerified);
    }

    Ok(())
}

/// Removes the backup once the operation it protected has finished.
///
/// Asks for the removal rather than checking first. The removal already treats an absent
/// file as success, so a check would be a second answer to the same question taken a moment
/// earlier, and a moment earlier is exactly long enough for it to stop being true.
///
/// # Errors
///
/// Returns [`VaultFileError::Io`] if the file is there and cannot be removed.
pub fn discard_backup(paths: &VaultPaths) -> Result<(), VaultFileError> {
    remove(&paths.backup)
}

/// Reads a header from a path, answering `None` if the file is not there.
fn read_header(path: &Path) -> Result<Option<VaultHeader>, VaultFileError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(cause) => {
            return Err(VaultFileError::Io {
                operation: "read",
                cause,
            });
        }
    };

    // A file of the wrong size is not a header, and neither is one that fails to parse. Both
    // answer `None` here so that the caller can fall back to the copy; the caller is the one
    // that decides whether having nothing readable is a failure.
    if bytes.len() != HEADER_LEN {
        return Ok(None);
    }

    Ok(VaultHeader::parse(&bytes).ok())
}

/// Writes bytes to a temporary file and renames it over the destination.
///
/// The flush before the rename is what makes the promise real on a machine that loses power:
/// without it the rename can be recorded while the contents it points at are still in a
/// buffer, and the result is a file of the right length full of nothing.
fn write(destination: &Path, temporary: &Path, bytes: &[u8]) -> Result<(), VaultFileError> {
    use std::io::Write as _;

    if let Some(directory) = destination.parent() {
        fs::create_dir_all(directory).map_err(|cause| VaultFileError::Io {
            operation: "written, because its directory could not be created",
            cause,
        })?;
    }

    let mut file = fs::File::create(temporary).map_err(|cause| VaultFileError::Io {
        operation: "written to a temporary file",
        cause,
    })?;

    file.write_all(bytes).map_err(|cause| VaultFileError::Io {
        operation: "written",
        cause,
    })?;
    file.sync_all().map_err(|cause| VaultFileError::Io {
        operation: "flushed to the disk",
        cause,
    })?;
    drop(file);

    fs::rename(temporary, destination).map_err(|cause| VaultFileError::Io {
        operation: "renamed over the old one",
        cause,
    })
}

/// Removes a file, treating an already absent file as success.
fn remove(path: &Path) -> Result<(), VaultFileError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(VaultFileError::Io {
            operation: "removed",
            cause,
        }),
    }
}
