//! Every state the pair of files can be left in, and what happens when the application
//! starts and finds it.
//!
//! The header is rewritten by exactly three operations, and two of them take a copy first.
//! That leaves a small, enumerable set of states on disk, and this walks all of them rather
//! than the two that came to mind. The one that matters is the power cut: the rename is
//! meant to make a half written header impossible, and the copy is meant to cover the case
//! where the disk itself failed.
//!
//! Every test here works on a directory of its own inside the temporary directory of the
//! machine, and removes it afterwards. Nothing writes near the real vault.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "every function in an integration test file is test code, but the lints that forbid panicking constructs only relax themselves inside #[cfg(test)] modules and #[test] functions"
)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use cairn_crypto::{Argon2Params, MIN_MEMORY_KIB, MIN_PASSES, VaultHeader, create, unlock};
use cairn_lib::vault_file::{
    Recovery, VaultFileError, VaultPaths, back_up_verified, discard_backup, read_or_recover,
    write_header,
};

const PASSWORD: &str = "una contrasena de ejemplo para las pruebas";
const CREATED_AT_US: i64 = 1_700_000_000_000_000;

/// A directory of its own for each test, removed when the guard is dropped.
struct Scratch {
    directory: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);

        let directory = std::env::temp_dir().join(format!(
            "cairn-vault-file-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();

        Self { directory }
    }

    fn paths(&self) -> VaultPaths {
        VaultPaths::in_directory(&self.directory)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Best effort. A leftover directory in the temporary folder is untidy; failing the
        // test that already passed because of it would be worse.
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// Parameters cheap enough to run several times in one test.
fn cheap() -> Argon2Params {
    Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, 1).unwrap()
}

fn a_header() -> VaultHeader {
    create(PASSWORD, cheap(), CREATED_AT_US).unwrap().0
}

#[test]
fn an_empty_directory_means_there_is_no_vault_yet() {
    let scratch = Scratch::new("empty");
    let (recovery, header) = read_or_recover(&scratch.paths()).unwrap();

    assert_eq!(recovery, Recovery::NothingThere);
    assert!(header.is_none());
}

#[test]
fn a_good_header_on_its_own_is_read_back_unchanged() {
    let scratch = Scratch::new("good-header");
    let paths = scratch.paths();
    let written = a_header();

    write_header(&paths, &written).unwrap();
    let (recovery, read) = read_or_recover(&paths).unwrap();

    assert_eq!(recovery, Recovery::HeaderWasFine);
    assert_eq!(read.unwrap(), written);
}

#[test]
fn a_good_header_with_a_leftover_backup_clears_the_backup() {
    // The state after a rewrite that finished. The copy has no job left, and leaving it
    // behind would mean the next recovery had a stale file to consider.
    let scratch = Scratch::new("leftover-backup");
    let paths = scratch.paths();

    write_header(&paths, &a_header()).unwrap();
    back_up_verified(&paths).unwrap();
    assert!(paths.backup().exists());

    let (recovery, header) = read_or_recover(&paths).unwrap();

    assert_eq!(recovery, Recovery::HeaderWasFine);
    assert!(header.is_some());
    assert!(!paths.backup().exists(), "the backup was not cleared");
}

#[test]
fn a_header_truncated_by_a_power_cut_is_restored_from_the_backup() {
    // The state the rename is supposed to make impossible, forced by hand, so that the
    // recovery path is exercised rather than assumed.
    let scratch = Scratch::new("truncated");
    let paths = scratch.paths();
    let original = a_header();

    write_header(&paths, &original).unwrap();
    back_up_verified(&paths).unwrap();
    fs::write(paths.header(), [0_u8; 40]).unwrap();

    let (recovery, recovered) = read_or_recover(&paths).unwrap();

    assert_eq!(recovery, Recovery::RestoredFromBackup);
    assert_eq!(recovered.clone().unwrap(), original);
    assert!(!paths.backup().exists());

    // And the restored file is on disk, not only in memory.
    let (again, reread) = read_or_recover(&paths).unwrap();
    assert_eq!(again, Recovery::HeaderWasFine);
    assert_eq!(reread.unwrap(), original);

    // And it still opens with the password it was created with.
    assert!(unlock(&recovered.unwrap(), PASSWORD).is_ok());
}

#[test]
fn a_header_of_the_right_size_but_the_wrong_contents_is_restored_too() {
    // Not a truncation: the right number of bytes, none of them ours. A disk that returns
    // the wrong sector produces this, and it has to be caught by the parser rather than by
    // the length.
    let scratch = Scratch::new("garbage");
    let paths = scratch.paths();
    let original = a_header();

    write_header(&paths, &original).unwrap();
    back_up_verified(&paths).unwrap();
    fs::write(paths.header(), [0x5a_u8; 168]).unwrap();

    let (recovery, recovered) = read_or_recover(&paths).unwrap();
    assert_eq!(recovery, Recovery::RestoredFromBackup);
    assert_eq!(recovered.unwrap(), original);
}

#[test]
fn a_missing_header_with_a_good_backup_is_restored() {
    // The rename never happened, or the file was deleted between the copy and the write.
    let scratch = Scratch::new("missing-header");
    let paths = scratch.paths();
    let original = a_header();

    write_header(&paths, &original).unwrap();
    back_up_verified(&paths).unwrap();
    fs::remove_file(paths.header()).unwrap();

    let (recovery, recovered) = read_or_recover(&paths).unwrap();
    assert_eq!(recovery, Recovery::RestoredFromBackup);
    assert_eq!(recovered.unwrap(), original);
}

#[test]
fn a_broken_header_with_a_broken_backup_is_reported_rather_than_guessed_at() {
    // Nothing left to recover from. It has to say so: quietly answering that there is no
    // vault would invite the next screen to offer to create one over the top of the
    // ciphertext that is still on the disk.
    let scratch = Scratch::new("both-broken");
    let paths = scratch.paths();

    write_header(&paths, &a_header()).unwrap();
    back_up_verified(&paths).unwrap();
    fs::write(paths.header(), [0x00_u8; 168]).unwrap();
    fs::write(paths.backup(), [0x00_u8; 168]).unwrap();

    assert!(matches!(
        read_or_recover(&paths),
        Err(VaultFileError::Malformed(_))
    ));
}

#[test]
fn a_broken_header_with_no_backup_is_reported_rather_than_treated_as_a_new_machine() {
    let scratch = Scratch::new("broken-alone");
    let paths = scratch.paths();

    write_header(&paths, &a_header()).unwrap();
    fs::write(paths.header(), [0x00_u8; 12]).unwrap();

    assert!(matches!(
        read_or_recover(&paths),
        Err(VaultFileError::Malformed(_))
    ));
}

#[test]
fn a_backup_with_no_header_at_all_is_still_a_vault() {
    let scratch = Scratch::new("backup-only");
    let paths = scratch.paths();
    let original = a_header();

    write_header(&paths, &original).unwrap();
    back_up_verified(&paths).unwrap();
    fs::remove_file(paths.header()).unwrap();

    let (recovery, recovered) = read_or_recover(&paths).unwrap();
    assert_eq!(recovery, Recovery::RestoredFromBackup);
    assert!(paths.header().exists());
    assert_eq!(recovered.unwrap(), original);
}

#[test]
fn a_backup_is_verified_by_reading_it_back() {
    let scratch = Scratch::new("verify");
    let paths = scratch.paths();
    let original = a_header();

    write_header(&paths, &original).unwrap();
    back_up_verified(&paths).unwrap();

    let copied = fs::read(paths.backup()).unwrap();
    assert_eq!(copied, original.to_bytes());
    assert_eq!(VaultHeader::parse(&copied).unwrap(), original);
}

#[test]
fn a_backup_of_a_header_that_is_not_there_fails_rather_than_writing_nothing() {
    // An empty backup file would pass a comparison against an empty read and would be
    // useless. It has to fail while there is still time to stop the operation.
    let scratch = Scratch::new("backup-nothing");
    assert!(back_up_verified(&scratch.paths()).is_err());
}

#[test]
fn discarding_a_backup_that_is_not_there_is_not_an_error() {
    let scratch = Scratch::new("discard-absent");
    assert!(discard_backup(&scratch.paths()).is_ok());
}

#[test]
fn writing_the_header_twice_leaves_no_temporary_file_behind() {
    // The temporary file is renamed rather than copied, so it must not exist afterwards. One
    // left behind would be a hundred and sixty-eight bytes of a previous header sitting in
    // the data directory with nothing to say it is stale.
    let scratch = Scratch::new("no-temporary");
    let paths = scratch.paths();

    write_header(&paths, &a_header()).unwrap();
    write_header(&paths, &a_header()).unwrap();

    let leftovers: Vec<String> = fs::read_dir(&scratch.directory)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();

    assert_eq!(leftovers, vec!["vault.header".to_owned()]);
}

#[test]
fn the_whole_sequence_of_a_password_change_survives_being_stopped_anywhere() {
    // Every point at which the process can die during the operation, one at a time. The
    // requirement is not that the change completes; it is that whatever is on disk afterwards
    // opens with one of the two passwords and loses nothing.
    let scratch = Scratch::new("interrupted");
    let paths = scratch.paths();
    let new_password = "una contrasena distinta y mas larga";

    let (original, _) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();
    write_header(&paths, &original).unwrap();

    let (rewritten, _) =
        cairn_crypto::change_password(&original, PASSWORD, new_password, CREATED_AT_US + 1)
            .unwrap();

    // Stopped after the copy, before the write.
    back_up_verified(&paths).unwrap();
    let (recovery, header) = read_or_recover(&paths).unwrap();
    assert_eq!(recovery, Recovery::HeaderWasFine);
    assert!(unlock(&header.unwrap(), PASSWORD).is_ok());

    // Stopped after the write, before the copy was discarded.
    back_up_verified(&paths).unwrap();
    write_header(&paths, &rewritten).unwrap();
    let (recovery, header) = read_or_recover(&paths).unwrap();
    assert_eq!(recovery, Recovery::HeaderWasFine);
    assert!(unlock(&header.unwrap(), new_password).is_ok());

    // Finished.
    discard_backup(&paths).unwrap();
    let (recovery, header) = read_or_recover(&paths).unwrap();
    assert_eq!(recovery, Recovery::HeaderWasFine);
    assert!(unlock(&header.unwrap(), new_password).is_ok());
}

#[test]
fn discarding_a_backup_removes_it_rather_than_leaving_it_where_it_was() {
    // The copy exists for the duration of one rewrite and not afterwards. Leaving it behind
    // means the next startup has two candidates and has to guess, and the one it would guess
    // is the older of the two.
    let scratch = Scratch::new("discard-removes");
    let paths = scratch.paths();
    write_header(&paths, &a_header()).unwrap();
    back_up_verified(&paths).unwrap();
    assert!(paths.backup().exists(), "the copy was not taken");

    discard_backup(&paths).unwrap();

    assert!(
        !paths.backup().exists(),
        "the copy was still there after being discarded"
    );
}

#[test]
fn a_header_that_cannot_be_read_for_another_reason_is_not_treated_as_absent() {
    // The difference between "there is no header" and "the header could not be read" is the
    // difference between offering to create a vault and saying something is wrong. A
    // directory where the file should be produces the second, and a guard that folded them
    // together would have the application offer to create a vault over a live one.
    let scratch = Scratch::new("header-unreadable");
    let paths = scratch.paths();
    fs::create_dir_all(paths.header()).unwrap();

    assert!(
        matches!(read_or_recover(&paths), Err(VaultFileError::Io { .. })),
        "a header that could not be read was reported as a machine with no vault"
    );
}

#[test]
fn a_leftover_backup_that_cannot_be_removed_is_reported_rather_than_ignored() {
    // Removing the copy is the last step of a rewrite and of a clean startup. Reporting
    // success for a copy that is still there would leave the next startup considering a stale
    // file, which is the one case where an older header gets used in place of a good one.
    let scratch = Scratch::new("backup-stuck");
    let paths = scratch.paths();
    write_header(&paths, &a_header()).unwrap();
    // A directory cannot be removed by the call that removes a file, so this stands in for
    // any reason the removal fails.
    fs::create_dir_all(paths.backup()).unwrap();

    assert!(
        matches!(read_or_recover(&paths), Err(VaultFileError::Io { .. })),
        "a copy that could not be removed was reported as removed"
    );
}
