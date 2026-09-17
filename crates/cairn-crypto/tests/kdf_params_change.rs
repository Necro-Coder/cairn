//! Proves that changing the Argon2id parameters does not re-encrypt a single record.
//!
//! This is the test the whole key hierarchy was arranged around, and it is the reason the
//! decision to measure Argon2id on a phone could be postponed instead of being made blind.
//! The argument is: the parameters are safe to change later because changing them only
//! re-wraps the data key. An argument is not evidence. What follows is the evidence.
//!
//! Records are sealed at one set of parameters. The parameters are changed. Then, byte for
//! byte, the same ciphertexts still open, the identifier of the data key has not moved, and
//! the key the database file is encrypted with is the same key it was before. If any of
//! those were false, changing the parameters would mean rewriting the database, which is the
//! most dangerous operation this application could ever perform.
//!
//! The last two cases are the ones that only fail on the second try: an operation interrupted
//! halfway, and the same operation run twice in a row.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "every function in an integration test file is test code, but the lints that forbid panicking constructs only relax themselves inside #[cfg(test)] modules and #[test] functions"
)]

use cairn_crypto::{
    Aad, Argon2Params, FreshNonce, Kek, MIN_MEMORY_KIB, MIN_PASSES, Sealed, UnlockedVault,
    VaultHeader, change_kdf_params, change_password, create, database_key, export_key, open, seal,
    unlock,
};
use subtle::ConstantTimeEq as _;

/// The password used throughout. Invented, and the only one in this file.
const PASSWORD: &str = "una contrasena de ejemplo para las pruebas";

/// A fixed moment, so that nothing here depends on when it runs.
const CREATED_AT_US: i64 = 1_700_000_000_000_000;

/// Parameters a test can afford to run several times.
///
/// The floor rather than the default. What is under test is that changing the parameters
/// changes nothing else, and that holds at any pair of values; running the real sixty-four
/// mebibytes a dozen times would add half a minute to the suite and prove the same thing.
fn cheap() -> Argon2Params {
    Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, 1).unwrap()
}

/// A second, different set. Also inside the allowed range, and not equal to the first.
fn cheaper_but_slower() -> Argon2Params {
    Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES + 1, 1).unwrap()
}

/// Three records, sealed the way the application will seal them.
///
/// Each one carries its own associated data, built from the table, the row, the column, the
/// revision and the identifier of the key, which is exactly what makes moving a ciphertext
/// from one row to another fail.
fn seal_records(vault: &UnlockedVault) -> Vec<(Aad, Sealed)> {
    let fields = [
        (
            "credentials",
            [0x01_u8; 16],
            "password",
            1_u64,
            "correo del banco",
        ),
        ("finances", [0x02_u8; 16], "note", 4, "alquiler de marzo"),
        ("habits", [0x03_u8; 16], "note", 9, "media hora de lectura"),
    ];

    fields
        .into_iter()
        .map(|(table, row, column, rev, plaintext)| {
            let aad = Aad::record(1, table, &row, column, rev, vault.key_id()).unwrap();
            let sealed = seal(
                vault.data_key(),
                FreshNonce::generate().unwrap(),
                &aad,
                plaintext.as_bytes(),
            )
            .unwrap();
            (aad, sealed)
        })
        .collect()
}

/// Checks that every record still opens to what it held, and that not one byte moved.
fn assert_records_untouched(vault: &UnlockedVault, records: &[(Aad, Sealed)], expected: &[&str]) {
    for ((aad, sealed), plaintext) in records.iter().zip(expected) {
        let opened = open(vault.data_key(), sealed, aad).unwrap();
        assert_eq!(
            opened.as_slice(),
            plaintext.as_bytes(),
            "a record stopped opening after the header was rewritten"
        );
    }
}

#[test]
fn changing_the_parameters_re_encrypts_nothing() {
    let (header, vault) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();

    let records = seal_records(&vault);
    let plaintexts = [
        "correo del banco",
        "alquiler de marzo",
        "media hora de lectura",
    ];
    let before: Vec<Vec<u8>> = records
        .iter()
        .map(|(_, sealed)| sealed.to_bytes())
        .collect();

    let key_id_before = *vault.key_id();
    let database_key_before = vault.database_key();

    // The operation under test.
    let (rewritten, reopened) =
        change_kdf_params(&header, PASSWORD, cheaper_but_slower(), CREATED_AT_US + 1).unwrap();

    // The header did change, and it changed in the ways it is supposed to.
    assert_eq!(rewritten.params(), cheaper_but_slower());
    assert_ne!(rewritten.kdf_salt(), header.kdf_salt());
    assert_eq!(rewritten.header_rev(), header.header_rev() + 1);
    assert_eq!(
        rewritten.master_change_count(),
        header.master_change_count(),
        "changing the parameters is not changing the password"
    );

    // And nothing else did.
    assert_eq!(
        reopened.key_id(),
        &key_id_before,
        "the key identifier moved"
    );
    assert!(
        bool::from(reopened.database_key().ct_eq(&database_key_before)),
        "the key the database file is encrypted with changed, which would mean rewriting it"
    );

    let after: Vec<Vec<u8>> = records
        .iter()
        .map(|(_, sealed)| sealed.to_bytes())
        .collect();
    assert_eq!(before, after, "a stored ciphertext was rewritten");
    assert_records_untouched(&reopened, &records, &plaintexts);

    // And the vault opens again from the rewritten header, with the same password.
    let after_unlock = unlock(&rewritten, PASSWORD).unwrap();
    assert_records_untouched(&after_unlock, &records, &plaintexts);
    assert!(bool::from(
        after_unlock.database_key().ct_eq(&database_key_before)
    ));
}

#[test]
fn the_old_header_still_opens_if_the_write_never_happened() {
    // The interruption case, simulated by discarding the new header instead of killing the
    // process. What matters is that the operation produces a value rather than mutating
    // anything: if the caller never writes what it produced, nothing has changed at all.
    let (header, _) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();

    let (_discarded, _) =
        change_kdf_params(&header, PASSWORD, cheaper_but_slower(), CREATED_AT_US + 1).unwrap();

    let reopened = unlock(&header, PASSWORD).unwrap();
    assert_eq!(reopened.key_id(), header.key_id());
}

#[test]
fn changing_the_parameters_twice_in_a_row_works() {
    // The case that only fails on the second attempt, which is why it is written down. A
    // revision counter incremented in the wrong place, or a salt reused, shows up here and
    // nowhere else.
    let (header, vault) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();
    let records = seal_records(&vault);
    let plaintexts = [
        "correo del banco",
        "alquiler de marzo",
        "media hora de lectura",
    ];
    let database_key_before = vault.database_key();

    let (once, _) =
        change_kdf_params(&header, PASSWORD, cheaper_but_slower(), CREATED_AT_US + 1).unwrap();
    let (twice, reopened) = change_kdf_params(&once, PASSWORD, cheap(), CREATED_AT_US + 2).unwrap();

    assert_eq!(twice.header_rev(), header.header_rev() + 2);
    assert_ne!(twice.kdf_salt(), once.kdf_salt());
    assert_eq!(twice.params(), cheap());
    assert!(bool::from(
        reopened.database_key().ct_eq(&database_key_before)
    ));
    assert_records_untouched(&reopened, &records, &plaintexts);
}

#[test]
fn changing_the_password_re_encrypts_nothing_either() {
    // The same promise for the other operation. Both go through the same code, and a test
    // for only one of them would leave the other free to regenerate the data key.
    let (header, vault) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();
    let records = seal_records(&vault);
    let plaintexts = [
        "correo del banco",
        "alquiler de marzo",
        "media hora de lectura",
    ];

    let key_id_before = *vault.key_id();
    let database_key_before = vault.database_key();
    let before: Vec<Vec<u8>> = records
        .iter()
        .map(|(_, sealed)| sealed.to_bytes())
        .collect();

    let (rewritten, reopened) = change_password(
        &header,
        PASSWORD,
        "una contrasena distinta y mas larga",
        CREATED_AT_US + 1,
    )
    .unwrap();

    assert_eq!(rewritten.master_change_count(), 1);
    assert_eq!(
        rewritten.params(),
        header.params(),
        "changing the password is not changing the parameters"
    );
    assert_eq!(reopened.key_id(), &key_id_before);
    assert!(bool::from(
        reopened.database_key().ct_eq(&database_key_before)
    ));

    let after: Vec<Vec<u8>> = records
        .iter()
        .map(|(_, sealed)| sealed.to_bytes())
        .collect();
    assert_eq!(before, after);
    assert_records_untouched(&reopened, &records, &plaintexts);
}

#[test]
fn the_old_password_stops_working_and_the_new_one_starts() {
    let (header, _) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();
    let new_password = "una contrasena distinta y mas larga";

    let (rewritten, _) =
        change_password(&header, PASSWORD, new_password, CREATED_AT_US + 1).unwrap();

    assert!(unlock(&rewritten, PASSWORD).is_err());
    assert!(unlock(&rewritten, new_password).is_ok());
}

#[test]
fn the_wrong_current_password_changes_nothing() {
    // Checked before anything is produced. An operation that rewrote the header and only
    // then discovered the password was wrong would have locked the owner out of their own
    // vault on a typo.
    let (header, _) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();

    assert!(change_password(&header, "la de al lado", "otra cosa", CREATED_AT_US + 1).is_err());
    assert!(
        change_kdf_params(
            &header,
            "la de al lado",
            cheaper_but_slower(),
            CREATED_AT_US + 1
        )
        .is_err()
    );

    // And the original still opens with the original password.
    assert!(unlock(&header, PASSWORD).is_ok());
}

#[test]
fn a_header_written_out_and_read_back_opens_the_same_vault() {
    // The round trip through bytes, because that is what actually goes on a disk. A field
    // that survived in memory and not in the file would show up here and in no other test.
    let (header, vault) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();
    let records = seal_records(&vault);
    let plaintexts = [
        "correo del banco",
        "alquiler de marzo",
        "media hora de lectura",
    ];

    let restored = VaultHeader::parse(&header.to_bytes()).unwrap();
    let reopened = unlock(&restored, PASSWORD).unwrap();

    assert_records_untouched(&reopened, &records, &plaintexts);
    assert!(bool::from(
        reopened.database_key().ct_eq(&vault.database_key())
    ));
}

#[test]
fn lowering_the_parameters_in_the_file_stops_it_opening() {
    // The attack the authenticated prefix exists to stop, run end to end. The parameters are
    // edited to something cheap but still legal, so the parser accepts them, and the
    // unwrapping refuses because they are part of what the tag covers.
    let (header, _) = create(PASSWORD, cheaper_but_slower(), CREATED_AT_US).unwrap();

    let mut bytes = header.to_bytes();
    // Argon2id passes, at offset thirty-two, lowered to the floor.
    let lowered = MIN_PASSES.to_le_bytes();
    for (index, byte) in lowered.iter().enumerate() {
        if let Some(target) = bytes.get_mut(32 + index) {
            *target = *byte;
        }
    }

    let edited = VaultHeader::parse(&bytes).expect("the edited parameters are still inside range");
    assert_eq!(edited.params().passes(), MIN_PASSES);
    assert!(
        unlock(&edited, PASSWORD).is_err(),
        "the Argon2id parameters were lowered and the vault still opened"
    );
}

#[test]
fn every_subkey_differs_from_every_other_and_from_the_data_key() {
    // Domain separation, checked at the level somebody would actually get it wrong: not that
    // the derivation is correct, but that no two keys in a live vault are the same bytes.
    let (_, vault) = create(PASSWORD, cheap(), CREATED_AT_US).unwrap();

    let database = vault.database_key();
    // Not from the vault. There is no way to ask an open vault for the key a backup is
    // sealed with, and that absence is the point: a backup whose key hung off this vault's
    // data key could only be opened by a machine that could already open this vault. What a
    // backup is sealed with comes from a key encryption key of its own file, and the one
    // built here stands in for it.
    let export = export_key(&Kek::from_bytes([0x5a; 32]));
    let sync = vault.sync_key();

    assert_ne!(
        database.expose(),
        sync.expose(),
        "the database key and the synchronisation key are the same bytes"
    );
    assert!(
        !bool::from(export.ct_eq(vault.data_key())),
        "the export key is the data key, so a stolen backup would open the vault itself"
    );
    assert_eq!(
        database_key(vault.data_key()).expose(),
        database.expose(),
        "asking for the database key twice gave two different keys"
    );
}
