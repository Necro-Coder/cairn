//! The migration tool, run the way somebody repairing a machine would run it.
//!
//! Driven as a process rather than as a function, because what is being checked is the part a
//! unit test cannot reach: that the password is read from standard input, that the exit code
//! says what happened, and that a refusal says nothing about which part of the vault refused.
// Every function in an integration test file is test code, but the lint that forbids panicking
// constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]` functions. The
// helpers below are neither, and a helper that cannot panic would have to return a Result that
// every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES};

/// The password every vault in this file is created with.
const PASSWORD: &str = "una frase larga para la prueba";

/// A directory that exists for one test and is removed when it ends.
struct Scratch {
    directory: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "cairn-migrate-{label}-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).expect("a temporary directory can be created");

        Self { directory }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// Creates a vault header in a directory, with no database beside it yet.
fn a_vault(directory: &Path) {
    let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
        .expect("the lowest accepted parameters are accepted");
    let (header, _vault) = cairn_crypto::create(PASSWORD, params, 0)
        .expect("creating a vault at the lowest parameters cannot fail here");

    fs::write(
        directory.join(cairn_db::HEADER_FILE),
        header.to_bytes().as_slice(),
    )
    .expect("the header can be written");
}

/// Runs the tool against a directory, typing a password at it.
fn migrate(directory: &Path, password: &str, arguments: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_migrate"))
        .arg("--directory")
        .arg(directory)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the tool can be started");

    // A failure to write is not a failure of the test. Several of these cases are about the
    // tool refusing before it ever asks for a password, and a tool that has already exited is a
    // pipe with nobody at the other end: the write comes back as a broken pipe on Linux and
    // succeeds into a buffer on Windows. What the test is about is what the tool printed and
    // what exit code it gave, both of which are read below.
    let _typed = child
        .stdin
        .as_mut()
        .expect("the tool takes standard input")
        .write_all(format!("{password}\n").as_bytes());

    child.wait_with_output().expect("the tool finishes")
}

/// What the tool printed on standard output.
fn printed(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_vault_with_no_database_reports_version_zero_and_then_raises_it() {
    let scratch = Scratch::new("up");
    a_vault(&scratch.directory);

    let before = migrate(&scratch.directory, PASSWORD, &["status"]);
    assert!(before.status.success(), "status failed on a new vault");
    assert!(
        printed(&before).contains("schema on disk: 0"),
        "a database that does not exist yet did not report version zero"
    );

    let applied = migrate(&scratch.directory, PASSWORD, &["up"]);
    assert!(applied.status.success(), "up failed on a new vault");

    let after = migrate(&scratch.directory, PASSWORD, &["status"]);
    assert!(
        printed(&after).contains(&format!("schema on disk: {}", cairn_db::LATEST_VERSION)),
        "the schema was not raised to the newest version this build carries"
    );
}

#[test]
fn applying_twice_says_there_was_nothing_to_do() {
    let scratch = Scratch::new("twice");
    a_vault(&scratch.directory);

    assert!(
        migrate(&scratch.directory, PASSWORD, &["up"])
            .status
            .success()
    );
    let again = migrate(&scratch.directory, PASSWORD, &["up"]);

    assert!(again.status.success());
    assert!(
        printed(&again).contains("nothing to apply"),
        "a second run claimed to have applied something"
    );
}

#[test]
fn reverting_drops_what_the_migrations_made_and_can_be_applied_again() {
    let scratch = Scratch::new("down");
    a_vault(&scratch.directory);

    assert!(
        migrate(&scratch.directory, PASSWORD, &["up"])
            .status
            .success()
    );

    let reverted = migrate(&scratch.directory, PASSWORD, &["down", "0"]);
    assert!(reverted.status.success(), "down failed");
    assert!(printed(&reverted).contains("the file is now at 0"));

    let status = migrate(&scratch.directory, PASSWORD, &["status"]);
    assert!(printed(&status).contains("schema on disk: 0"));

    // The half that makes a migration reversible rather than merely undoable once.
    let again = migrate(&scratch.directory, PASSWORD, &["up"]);
    assert!(
        again.status.success(),
        "the schema could not be raised again"
    );
}

#[test]
fn a_wrong_password_is_refused_without_saying_which_part_refused() {
    let scratch = Scratch::new("wrong-password");
    a_vault(&scratch.directory);

    let refused = migrate(
        &scratch.directory,
        "otra frase completamente distinta",
        &["status"],
    );

    assert!(!refused.status.success(), "a wrong password was accepted");

    let complaint = String::from_utf8_lossy(&refused.stderr);
    assert!(
        complaint.contains("did not open with that password"),
        "the refusal did not say what to do about it: {complaint}"
    );
    // The message is the same whatever failed inside. Anything more specific is an oracle that
    // tells somebody with the file which half of their guess was right.
    for leak in ["Argon2", "tag", "wrap", "key"] {
        assert!(
            !complaint.contains(leak),
            "the refusal mentions {leak}, which narrows the guess"
        );
    }
}

#[test]
fn a_directory_with_no_vault_in_it_is_refused_before_a_password_is_asked_for() {
    let scratch = Scratch::new("no-vault");

    let refused = migrate(&scratch.directory, PASSWORD, &["status"]);

    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("header could not be read"),
        "an empty directory did not say that there is no vault in it"
    );
    assert!(
        !scratch.directory.join(cairn_db::DATABASE_FILE).exists(),
        "a database was created in a directory with no vault in it"
    );
}

#[test]
fn an_argument_that_is_not_a_command_is_refused_with_the_usage() {
    let scratch = Scratch::new("bad-argument");

    let refused = migrate(&scratch.directory, PASSWORD, &["sideways"]);

    assert!(!refused.status.success());
    let complaint = String::from_utf8_lossy(&refused.stderr);
    assert!(complaint.contains("sideways is not one of the arguments"));
    assert!(complaint.contains("Usage: migrate"));
}

#[test]
fn the_password_is_not_among_the_arguments() {
    // The rule this tool exists to keep. Arguments are readable by every other process on the
    // machine, so a future flag that takes a password has to fail here rather than in a review.
    let scratch = Scratch::new("usage");

    let usage = migrate(&scratch.directory, PASSWORD, &["--help"]);

    assert!(usage.status.success());
    let text = printed(&usage);
    assert!(text.contains("read from standard input"));
    assert!(
        !text.contains("--password"),
        "the tool offers a way to put a password on the command line"
    );
}
