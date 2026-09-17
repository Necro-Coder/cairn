//! Two vaults, frozen as files, that every future version of this code has to keep opening.
//!
//! The header is a file format, and a file format is a promise. Every other test in this
//! crate writes something and reads it back, which proves that this build agrees with
//! itself; none of them would notice a change that made this build disagree with the one
//! somebody has a vault from. These two would.
//!
//! One is at the parameters a new vault is created with. The other is at a different set,
//! because the operation that changes them is the one this phase exists to make safe, and a
//! frozen example of its output is worth more than another test that generates one.
//!
//! The passwords are in the README beside the files and are printed here as well. There is
//! nothing to protect: the contents are three sentences somebody made up.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "every function in an integration test file is test code, but the lints that forbid panicking constructs only relax themselves inside #[cfg(test)] modules and #[test] functions"
)]

use std::fs;
use std::path::{Path, PathBuf};

use cairn_crypto::{
    Aad, Argon2Params, FreshNonce, Sealed, VaultHeader, create, open, seal, unlock,
};

/// The password both fixtures were created with. Invented, and written down next to them.
const FIXTURE_PASSWORD: &str = "una contrasena de ejemplo para el fixture";

/// What the frozen record says. Invented.
const FIXTURE_PLAINTEXT: &str = "media hora de lectura antes de dormir";

/// The associated data of the frozen record, field by field.
///
/// Written out rather than derived, because that is the point: a future version has to
/// produce the same associated data from the same fields, and the only way to check that is
/// to state the fields somewhere other than in the code that built them.
const FIXTURE_FORMAT_VERSION: u16 = 1;
const FIXTURE_TABLE: &str = "habits";
const FIXTURE_ROW_ID: [u8; 16] = [
    0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x60, 0x71, 0x82, 0x93, 0xa4, 0xb5, 0xc6, 0xd7, 0xe8, 0xf9,
];
const FIXTURE_COLUMN: &str = "note";
const FIXTURE_REV: u64 = 3;

/// The moment both fixtures claim to have been created at. Fixed, so the files are stable.
const FIXTURE_CREATED_AT_US: i64 = 1_700_000_000_000_000;

/// Where the fixtures live, relative to this crate.
fn fixture_directory(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Opens one fixture and reads the record inside it.
fn open_fixture(name: &str) -> (VaultHeader, Argon2Params, String) {
    let directory = fixture_directory(name);

    let header_bytes = fs::read(directory.join("cairn.header"))
        .unwrap_or_else(|cause| panic!("the fixture {name} has no header file: {cause}"));
    let header = VaultHeader::parse(&header_bytes)
        .unwrap_or_else(|cause| panic!("the header of fixture {name} no longer parses: {cause}"));

    let vault = unlock(&header, FIXTURE_PASSWORD)
        .unwrap_or_else(|cause| panic!("the fixture {name} no longer opens: {cause}"));

    let sealed_bytes = fs::read(directory.join("record.sealed"))
        .unwrap_or_else(|cause| panic!("the fixture {name} has no record file: {cause}"));
    let sealed = Sealed::from_bytes(&sealed_bytes).unwrap();

    let aad = Aad::record(
        FIXTURE_FORMAT_VERSION,
        FIXTURE_TABLE,
        &FIXTURE_ROW_ID,
        FIXTURE_COLUMN,
        FIXTURE_REV,
        vault.key_id(),
    )
    .unwrap();

    let opened = open(vault.data_key(), &sealed, &aad).unwrap_or_else(|cause| {
        panic!("the record inside fixture {name} no longer decrypts: {cause}")
    });

    let text = String::from_utf8(opened.to_vec()).unwrap();
    let params = header.params();

    (header, params, text)
}

#[test]
fn the_fixture_at_the_default_parameters_still_opens() {
    let (header, params, text) = open_fixture("default-parameters");

    assert_eq!(params, Argon2Params::DEFAULT);
    assert_eq!(text, FIXTURE_PLAINTEXT);
    assert_eq!(header.created_at_us(), FIXTURE_CREATED_AT_US);
    assert_eq!(header.master_change_count(), 0);
}

#[test]
fn the_fixture_at_lowered_parameters_still_opens() {
    // The output of the operation this phase exists to make safe, frozen as a file. If
    // changing the parameters ever started producing something a later build could not read,
    // this is where it would show.
    let (header, params, text) = open_fixture("lowered-parameters");

    assert_eq!(params.memory_kib(), 32 * 1024);
    assert_eq!(params.passes(), 4);
    assert_eq!(params.lanes(), 1);
    assert_eq!(text, FIXTURE_PLAINTEXT);
    assert_eq!(header.created_at_us(), FIXTURE_CREATED_AT_US);
}

#[test]
fn the_two_fixtures_are_different_vaults() {
    // Not the same file with the parameters edited: different salts, different keys,
    // different ciphertext. Otherwise the second one would prove nothing the first did not.
    let default = fs::read(fixture_directory("default-parameters").join("cairn.header")).unwrap();
    let lowered = fs::read(fixture_directory("lowered-parameters").join("cairn.header")).unwrap();

    assert_ne!(default, lowered);
    assert_ne!(
        VaultHeader::parse(&default).unwrap().key_id(),
        VaultHeader::parse(&lowered).unwrap().key_id()
    );
}

#[test]
fn the_wrong_password_does_not_open_a_fixture() {
    let bytes = fs::read(fixture_directory("default-parameters").join("cairn.header")).unwrap();
    let header = VaultHeader::parse(&bytes).unwrap();
    assert!(unlock(&header, "la de al lado").is_err());
}

/// Writes the fixtures again from scratch.
///
/// Ignored by default, because running it makes the files it is supposed to be protecting.
/// It exists so that the files can be reproduced deliberately, with `cargo test -p
/// cairn-crypto --test fixtures -- --ignored`, and so that anybody reading the directory can
/// see exactly what produced it.
///
/// Regenerating them is a decision, not a fix. If a fixture stops opening, the question is
/// what changed about the format, and replacing the file answers it by deleting the evidence.
#[test]
#[ignore = "writes the fixture files; run deliberately, never to make a failing test pass"]
fn regenerate_the_fixtures() {
    let cases = [
        ("default-parameters", Argon2Params::DEFAULT),
        (
            "lowered-parameters",
            Argon2Params::new(32 * 1024, 4, 1).unwrap(),
        ),
    ];

    for (name, params) in cases {
        let directory = fixture_directory(name);
        fs::create_dir_all(&directory).unwrap();

        let (header, vault) = create(FIXTURE_PASSWORD, params, FIXTURE_CREATED_AT_US).unwrap();

        let aad = Aad::record(
            FIXTURE_FORMAT_VERSION,
            FIXTURE_TABLE,
            &FIXTURE_ROW_ID,
            FIXTURE_COLUMN,
            FIXTURE_REV,
            vault.key_id(),
        )
        .unwrap();

        let sealed = seal(
            vault.data_key(),
            FreshNonce::generate().unwrap(),
            &aad,
            FIXTURE_PLAINTEXT.as_bytes(),
        )
        .unwrap();

        fs::write(directory.join("cairn.header"), header.to_bytes()).unwrap();
        fs::write(directory.join("record.sealed"), sealed.to_bytes()).unwrap();
    }
}
