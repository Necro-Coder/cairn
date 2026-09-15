//! Asserts that the diagnostics screen cannot leak who is running the application.
//!
//! The diagnostics screen exists in release builds, which means a screenshot of it is
//! something a person might reasonably paste into a public issue. Everything it shows has
//! to survive that. An absolute path gives away the Windows account name; a host name
//! gives away the machine; either of them attached to a report about a password manager
//! is worse than the bug being reported.
//!
//! The rule is easy to state and easy to break by accident, which is exactly why it is
//! asserted here rather than left as a note in a review.
// Every function in an integration test file is test code, but the lint that forbids
// panicking constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]`
// functions. The helpers below are neither, and a helper that cannot panic would have to
// return a Result that every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use cairn_lib::commands::diagnostics::{DatabaseStatus, Diagnostics};

/// Serialises a snapshot exactly as the command hands it to the frontend.
fn encoded_snapshot(webview_version: Option<String>) -> String {
    let snapshot = Diagnostics::assemble(12_345, webview_version);
    serde_json::to_string(&snapshot).expect("the snapshot is plain data and always serialises")
}

#[test]
fn no_windows_drive_letter_appears_anywhere() {
    let encoded = encoded_snapshot(Some("152.0.4191.66".to_owned()));
    let bytes: Vec<char> = encoded.chars().collect();

    for window in bytes.windows(3) {
        let looks_like_a_drive = window[0].is_ascii_alphabetic()
            && window[1] == ':'
            && (window[2] == '\\' || window[2] == '/');
        assert!(
            !looks_like_a_drive,
            "the snapshot contains something shaped like an absolute Windows path: {encoded}"
        );
    }
}

#[test]
fn no_unix_home_directory_appears_anywhere() {
    let encoded = encoded_snapshot(None);
    for prefix in ["/home/", "/Users/", "/var/folders/"] {
        assert!(
            !encoded.contains(prefix),
            "the snapshot contains an absolute path beginning with {prefix}: {encoded}"
        );
    }
}

#[test]
fn the_account_name_of_whoever_is_running_this_never_appears() {
    let encoded = encoded_snapshot(Some("152.0.4191.66".to_owned())).to_lowercase();

    // Read from the environment rather than hard coded, so that this test is meaningful
    // on the machine it happens to be running on, including a build agent.
    let candidates = ["USERNAME", "USER", "LOGNAME", "COMPUTERNAME", "HOSTNAME"];
    let mut checked_at_least_one = false;

    for variable in candidates {
        let Ok(value) = std::env::var(variable) else {
            continue;
        };
        // Very short values would produce false positives against ordinary words.
        if value.len() < 3 {
            continue;
        }
        checked_at_least_one = true;
        assert!(
            !encoded.contains(&value.to_lowercase()),
            "the snapshot contains the value of {variable}, which identifies this machine \
             or the person using it"
        );
    }

    assert!(
        checked_at_least_one,
        "none of {candidates:?} was set, so this test verified nothing and must not be \
         reported as passing"
    );
}

#[test]
fn nothing_but_the_agreed_fields_is_reported() {
    // A field added without thinking is how a path ends up on this screen. Adding one
    // means coming here and saying why it is safe.
    let encoded = encoded_snapshot(None);
    let parsed: serde_json::Value =
        serde_json::from_str(&encoded).expect("what we just serialised must parse");
    let object = parsed.as_object().expect("the snapshot is a JSON object");

    let mut fields: Vec<&str> = object.keys().map(String::as_str).collect();
    fields.sort_unstable();

    assert_eq!(
        fields,
        [
            "app",
            "arch",
            "database",
            "os",
            "uptimeMs",
            "webviewVersion"
        ],
        "the set of fields on the diagnostics snapshot has changed"
    );
}

#[test]
fn the_database_is_reported_as_not_initialised_while_no_storage_exists() {
    let snapshot = Diagnostics::assemble(0, None);
    assert_eq!(
        snapshot.database,
        DatabaseStatus::NotInitialized,
        "no database is opened in this phase, so anything else would be a false report"
    );
}

#[test]
fn an_absent_webview_version_serialises_as_null_rather_than_being_dropped() {
    // The screen has to be able to tell "unknown" apart from "nobody asked".
    let encoded = encoded_snapshot(None);
    assert!(
        encoded.contains("\"webviewVersion\":null"),
        "an unknown WebView version must be reported as null: {encoded}"
    );
}
