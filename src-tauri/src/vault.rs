//! The vault as it exists on this machine: where its header lives, and what that header
//! currently says.
//!
//! Between [`crate::vault_file`], which moves bytes, and [`crate::commands::vault`], which
//! answers the WebView. This is the part that knows there is exactly one vault, that it was
//! read once at startup, and that a header held in memory and a header on disk must never
//! disagree.
//!
//! The rule that shapes everything here is that the file is written before the value in
//! memory is replaced. If the write fails the application carries on with the header it had,
//! which is the one still on the disk. The other order would leave a running program
//! believing in a header nobody could read back.
//!
//! No keys. Those are in [`crate::session`], and the only thing the two have in common is
//! that a command holds one after the other and never the other way round.

use std::path::Path;

use cairn_crypto::{Argon2Params, VaultHeader};

use crate::vault_file::{
    Recovery, VaultFileError, VaultPaths, back_up_verified, discard_backup, read_or_recover,
    write_header,
};

/// What reading the vault at startup found.
///
/// The last variant is the one worth naming. A header that is there and cannot be read still
/// counts as a vault, because the alternative is an application that offers to create a new
/// one over the top of it, and that offer is the single most destructive thing this program
/// could do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultCondition {
    /// No header and no copy. This machine has no vault yet.
    NoVaultYet,
    /// The header was readable and nothing had to be done.
    Readable,
    /// The header was unreadable and the copy beside it was used instead.
    RestoredFromBackup,
    /// There is a header and neither it nor the copy beside it can be read.
    Unreadable,
}

impl From<Recovery> for VaultCondition {
    fn from(recovery: Recovery) -> Self {
        match recovery {
            Recovery::NothingThere => Self::NoVaultYet,
            Recovery::HeaderWasFine => Self::Readable,
            Recovery::RestoredFromBackup => Self::RestoredFromBackup,
        }
    }
}

/// The vault on this machine.
#[derive(Debug)]
pub struct Vault {
    /// The three files a vault occupies.
    paths: VaultPaths,
    /// The header as it is on disk right now, or `None` when there is none to read.
    header: Option<VaultHeader>,
    /// What reading it at startup found.
    condition: VaultCondition,
}

impl Vault {
    /// Reads the vault in a directory, recovering from the copy if the header is unreadable.
    ///
    /// Called once, at startup. Everything afterwards works from what this found.
    ///
    /// A header that cannot be read is not an error here, because the application still has
    /// something useful to do: say so, refuse to create a second vault over the top, and let
    /// somebody put the file back from their own copy.
    ///
    /// # Errors
    ///
    /// Returns [`VaultFileError::Io`] if the directory itself cannot be read or written.
    /// That one is fatal at startup, because nothing below can work without it.
    pub fn open_at(directory: &Path) -> Result<Self, VaultFileError> {
        let paths = VaultPaths::in_directory(directory);

        match read_or_recover(&paths) {
            Ok((recovery, header)) => Ok(Self {
                paths,
                header,
                condition: recovery.into(),
            }),
            Err(VaultFileError::Malformed(_)) => Ok(Self {
                paths,
                header: None,
                condition: VaultCondition::Unreadable,
            }),
            Err(other) => Err(other),
        }
    }

    /// Whether this machine has a vault at all.
    ///
    /// True for a header that cannot be read. The question this answers is whether creating
    /// one would destroy something, and over an unreadable header it would.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.condition != VaultCondition::NoVaultYet
    }

    /// What reading the vault at startup found.
    #[must_use]
    pub fn condition(&self) -> VaultCondition {
        self.condition
    }

    /// The header, or `None` on a machine with no vault yet.
    #[must_use]
    pub fn header(&self) -> Option<&VaultHeader> {
        self.header.as_ref()
    }

    /// The Argon2id parameters currently in force.
    #[must_use]
    pub fn params(&self) -> Option<Argon2Params> {
        self.header.as_ref().map(VaultHeader::params)
    }

    /// How many unlock attempts have failed since the last successful one.
    ///
    /// Zero when there is no vault, which is the truthful answer: nothing has been attempted
    /// against a vault that does not exist.
    #[must_use]
    pub fn failed_attempts(&self) -> u32 {
        self.header.as_ref().map_or(0, VaultHeader::failed_attempts)
    }

    /// Until when further attempts are refused, in microseconds since the epoch, UTC.
    #[must_use]
    pub fn locked_until_us(&self) -> i64 {
        self.header.as_ref().map_or(0, VaultHeader::locked_until_us)
    }

    /// Writes a header for a vault that did not exist, and remembers it.
    ///
    /// Separate from [`Vault::rewrite`] because there is nothing to back up and nothing to
    /// lose: a failure here leaves a machine with no vault, which is what it had.
    ///
    /// # Errors
    ///
    /// Returns [`VaultFileError::Io`] if the file cannot be written.
    pub fn install(&mut self, header: VaultHeader) -> Result<(), VaultFileError> {
        write_header(&self.paths, &header)?;
        self.header = Some(header);
        self.condition = VaultCondition::Readable;

        Ok(())
    }

    /// Replaces the header of an existing vault, taking a verified copy first.
    ///
    /// The copy is read back and parsed before the new header is written, because a copy
    /// discovered to be unreadable is discovered at the one moment it was needed. It is
    /// removed once the new header is in place, so the next startup does not consider a
    /// stale file.
    ///
    /// # Errors
    ///
    /// Returns [`VaultFileError::BackupNotVerified`] if the copy does not read back as what
    /// went into it, and [`VaultFileError::Io`] if any of the three file operations fails. In
    /// every failing case the header in memory is left as it was, which is the one still on
    /// the disk.
    pub fn rewrite(&mut self, header: VaultHeader) -> Result<(), VaultFileError> {
        back_up_verified(&self.paths)?;
        write_header(&self.paths, &header)?;
        discard_backup(&self.paths)?;

        self.header = Some(header);

        Ok(())
    }

    /// Records the outcome of an unlock attempt, in memory and on disk.
    ///
    /// The only write that skips the copy, because it touches none of the authenticated
    /// bytes: losing it to a power cut costs an attacker one attempt of leniency and costs
    /// the owner nothing.
    ///
    /// Does nothing on a machine with no vault. There is no header to record it in, and
    /// there was nothing to attempt.
    ///
    /// # Errors
    ///
    /// Returns [`VaultFileError::Io`] if the header cannot be written.
    pub fn record_attempt(
        &mut self,
        failed_attempts: u32,
        locked_until_us: i64,
    ) -> Result<(), VaultFileError> {
        let Some(header) = self.header.as_ref() else {
            return Ok(());
        };

        let mut updated = header.clone();
        updated.record_attempt(failed_attempts, locked_until_us);

        write_header(&self.paths, &updated)?;
        self.header = Some(updated);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES, VaultHeader};

    use super::{Vault, VaultCondition};
    use crate::vault_file::VaultFileError;

    /// Not a real password. A phrase invented for the test, in a directory removed after it.
    const NOT_A_REAL_PASSWORD: &str = "una frase larga inventada para la prueba";

    /// A moment in the middle of the range.
    const NOW_US: i64 = 1_700_000_000_000_000;

    struct Scratch {
        directory: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);

            let directory = std::env::temp_dir().join(format!(
                "cairn-vault-{name}-{}-{unique}",
                std::process::id()
            ));
            Self { directory }
        }

        fn open(&self) -> Vault {
            Vault::open_at(&self.directory).expect("the scratch directory can be read")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    /// The cheapest parameters the cryptographic crate will accept, so that the tests below
    /// spend their time on the file handling rather than on the hashing.
    fn cheapest_params() -> Argon2Params {
        Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted")
    }

    fn a_header() -> VaultHeader {
        let (header, _vault) = cairn_crypto::create(NOT_A_REAL_PASSWORD, cheapest_params(), NOW_US)
            .expect("creating a vault at the lowest parameters cannot fail here");
        header
    }

    #[test]
    fn a_directory_with_nothing_in_it_is_a_machine_with_no_vault() {
        let scratch = Scratch::new("empty");
        let vault = scratch.open();

        assert!(!vault.exists());
        assert_eq!(vault.condition(), VaultCondition::NoVaultYet);
        assert_eq!(vault.params(), None);
        assert_eq!(vault.header(), None);
        assert_eq!(vault.failed_attempts(), 0);
        assert_eq!(vault.locked_until_us(), 0);
    }

    #[test]
    fn a_header_nobody_can_read_still_counts_as_a_vault() {
        // The single most destructive thing this program could do is offer to create a new
        // vault over a header that is merely damaged. So an unreadable header is a vault that
        // exists: the creation is refused, the condition is reported, and somebody can put
        // their own copy of the file back.
        let scratch = Scratch::new("unreadable");
        fs::create_dir_all(&scratch.directory).expect("the directory can be created");
        fs::write(scratch.directory.join("cairn.header"), [0_u8; 168])
            .expect("the damaged file can be written");

        let vault = scratch.open();

        assert_eq!(vault.condition(), VaultCondition::Unreadable);
        assert!(
            vault.exists(),
            "an unreadable header was reported as no vault, which invites creating one over it"
        );
        assert_eq!(vault.header(), None);
        assert_eq!(vault.params(), None);
    }

    #[test]
    fn a_header_that_was_installed_is_there_on_the_next_start() {
        let scratch = Scratch::new("install");
        let header = a_header();

        scratch
            .open()
            .install(header.clone())
            .expect("the header can be written");

        let reopened = scratch.open();
        assert!(reopened.exists());
        assert_eq!(reopened.condition(), VaultCondition::Readable);
        assert_eq!(reopened.header(), Some(&header));
        assert_eq!(reopened.params(), Some(cheapest_params()));
    }

    #[test]
    fn rewriting_replaces_the_header_and_leaves_no_copy_behind() {
        // The copy exists for the duration of the rewrite and not afterwards. Leaving it
        // there would mean the next startup had two candidates and had to guess.
        let scratch = Scratch::new("rewrite");
        let mut vault = scratch.open();
        vault
            .install(a_header())
            .expect("the header can be written");

        let replacement = a_header();
        vault
            .rewrite(replacement.clone())
            .expect("the header can be replaced");

        assert_eq!(vault.header(), Some(&replacement));
        assert_eq!(scratch.open().header(), Some(&replacement));

        let leftovers: Vec<String> = fs::read_dir(&scratch.directory)
            .expect("the directory exists")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["cairn.header".to_owned()]);
    }

    #[test]
    fn a_recorded_attempt_survives_a_restart_and_changes_nothing_else() {
        // The count is the only thing an unlock writes, and it must not disturb a single
        // authenticated byte: if it did, recording a wrong password would make the vault
        // impossible to open with the right one.
        let scratch = Scratch::new("attempts");
        let header = a_header();
        let mut vault = scratch.open();
        vault
            .install(header.clone())
            .expect("the header is written");

        vault
            .record_attempt(3, NOW_US + 4_000_000)
            .expect("the attempt is recorded");

        let reopened = scratch.open();
        assert_eq!(reopened.failed_attempts(), 3);
        assert_eq!(reopened.locked_until_us(), NOW_US + 4_000_000);
        assert_eq!(
            reopened.header().map(VaultHeader::key_id),
            Some(header.key_id()),
            "recording an attempt changed the identity of the data key"
        );
        assert_eq!(reopened.params(), Some(cheapest_params()));
    }

    #[test]
    fn recording_an_attempt_against_no_vault_is_not_a_failure() {
        // Reached by an unlock sent to a machine that has no vault. There is nothing to
        // record it in, and treating that as an error would mean the command reported a
        // storage problem where the honest answer is that there is no vault.
        let scratch = Scratch::new("attempt-no-vault");
        let mut vault = scratch.open();

        assert!(vault.record_attempt(1, NOW_US).is_ok());
        assert!(!vault.exists());
    }

    #[test]
    fn a_failed_rewrite_leaves_the_header_that_is_still_on_the_disk() {
        // The whole reason the file is written before the value in memory is replaced. A
        // directory where the copy should go makes the copy fail, and what must survive is
        // the header the application had, because that is the one still readable.
        let scratch = Scratch::new("rewrite-fails");
        let header = a_header();
        let mut vault = scratch.open();
        vault
            .install(header.clone())
            .expect("the header is written");

        fs::create_dir_all(scratch.directory.join("cairn.header.backup"))
            .expect("the obstruction can be created");

        let failure = vault
            .rewrite(a_header())
            .expect_err("the rewrite could not take a copy");

        assert!(matches!(failure, VaultFileError::Io { .. }));
        assert_eq!(
            vault.header(),
            Some(&header),
            "a rewrite that failed replaced the header in memory anyway"
        );
    }
}
