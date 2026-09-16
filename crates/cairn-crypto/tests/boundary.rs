//! Proves that encryption happens in one place.
//!
//! The design rests on [`cairn_crypto::seal`] being the only way anything in this project
//! gets encrypted, and on the associated data being impossible to forget because the
//! function demands it. All of that is undone by one `use chacha20poly1305::...` somewhere
//! else, written by somebody in a hurry who needed to encrypt one value and did not want
//! to read this crate first. The second way of doing it is always the one without the
//! associated data.
//!
//! So the rule is checked rather than written down. This walks the workspace and fails if
//! any Rust source or any crate manifest outside this crate so much as names the cipher
//! library.
//!
//! A search over text rather than a lint, deliberately. It is the only thing that sees an
//! import inside a `#[cfg(test)]` module, inside a macro body, or inside a branch that is
//! compiled on one platform and not the one the pipeline happens to be running on. A rule
//! in the dependency policy can be added on top of this; it cannot replace it.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "every function in an integration test file is test code, but the lints that forbid panicking constructs only relax themselves inside #[cfg(test)] modules and #[test] functions"
)]

use std::fs;
use std::path::{Path, PathBuf};

/// The name that may not appear anywhere else.
///
/// Split so that this file, which lives outside the crate whose sources are being
/// searched, does not have to make an exception for itself. It is also the honest way to
/// write it: the string being searched for is built, not present.
const FORBIDDEN: [&str; 2] = ["chacha20", "poly1305"];

/// Directories that are not source and would make the walk take minutes.
///
/// `target` and `node_modules` are build output. `.git` is history. `dist` is a bundle.
/// `fases` is working material that never reaches a commit.
const SKIPPED_DIRECTORIES: [&str; 5] = ["target", "node_modules", ".git", "dist", "fases"];

/// The crate that is allowed to name the cipher, relative to the workspace root.
const ALLOWED: &str = "crates/cairn-crypto";

#[test]
fn no_other_crate_names_the_cipher_library() {
    let root = workspace_root();
    let allowed = root.join(ALLOWED);

    let mut offenders: Vec<String> = Vec::new();
    for file in interesting_files(&root) {
        if file.starts_with(&allowed) {
            continue;
        }

        let Ok(contents) = fs::read_to_string(&file) else {
            // Not valid UTF-8, so not a Rust source or a manifest whatever its extension
            // says. Nothing to read and nothing to hide.
            continue;
        };

        if contents.contains(&FORBIDDEN.concat()) {
            offenders.push(relative(&root, &file));
        }
    }

    assert!(
        offenders.is_empty(),
        "the cipher library is named outside {ALLOWED}, which means there is now a second \
         way to encrypt in this project: {offenders:?}"
    );
}

#[test]
fn the_search_would_actually_find_something() {
    // Guards the guard. A walk that silently visited nothing, because a path changed or a
    // skip rule grew too wide, would pass the test above for ever while checking nothing.
    let root = workspace_root();
    let files = interesting_files(&root);

    assert!(
        files.len() > 5,
        "the walk found only {} files, so it is not looking where the sources are",
        files.len()
    );

    let inside_this_crate = files
        .iter()
        .filter(|file| file.starts_with(root.join(ALLOWED)))
        .any(|file| {
            fs::read_to_string(file).is_ok_and(|contents| contents.contains(&FORBIDDEN.concat()))
        });

    assert!(
        inside_this_crate,
        "the search found no mention of the cipher inside {ALLOWED} either, so it is not \
         matching what it is supposed to match"
    );
}

/// The workspace root, derived from where this crate sits rather than from a command.
fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("this crate lives two directories below the workspace root")
        .to_path_buf()
}

/// Every Rust source and every crate manifest under the root, skipping build output.
///
/// Only those two kinds of file. The lockfile names every dependency by design, and the
/// public documentation is going to describe the algorithm in prose, so searching those
/// would mean either a failing test or a growing list of exceptions.
fn interesting_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();

            if path.is_dir() {
                if !SKIPPED_DIRECTORIES.contains(&name.as_ref()) {
                    pending.push(path);
                }
                continue;
            }

            if name.ends_with(".rs") || name == "Cargo.toml" {
                found.push(path);
            }
        }
    }

    found
}

/// A path relative to the workspace root, with forward slashes, so that a failure message
/// reads the same on every machine and contains nothing personal.
fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}
