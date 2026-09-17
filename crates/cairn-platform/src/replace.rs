//! Replacing one file with another, atomically, on a platform that does not want to.
//!
//! This exists for one moment: the instant a restored database takes the place of the live
//! one. Before it, the person has the vault they had; after it, they have the one from the
//! backup. There must be no third state, because a machine that loses power in the middle of
//! a restore should come back with one of those two and never with a database that is half of
//! each.
//!
//! On every Unix, `rename(2)` already promises that, and the standard library exposes it.
//!
//! On Windows it does not. `std::fs::rename` maps to `MoveFileExW` without
//! `MOVEFILE_REPLACE_EXISTING`, so it fails outright when the destination exists — which is
//! always, here. The obvious repair, removing the destination first and then renaming, opens
//! exactly the window this module is about: a crash in between leaves no database at all.
//!
//! So this module makes one system call with the flag that says "replace it", and that call
//! is the only `unsafe` it contains.

use std::io;
use std::path::Path;

/// Why a replacement did not happen.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReplaceError {
    /// The path holds something that cannot be handed to the system call.
    ///
    /// On Windows a path with an interior null byte, which no real path has and which a path
    /// built from a string somebody supplied could. Refused rather than truncated: a
    /// truncated path names a different file, and this call replaces whatever it names.
    #[error("that path cannot be used to name a file")]
    ImpossiblePath,

    /// The system refused.
    #[error("the file could not be replaced")]
    Refused(#[source] io::Error),
}

/// Replaces `destination` with `source`, leaving no moment where neither exists.
///
/// After this returns, `source` is gone and `destination` is what `source` was. Both paths
/// must be on the same volume; a move between volumes is a copy and a delete, and a copy and
/// a delete is the thing this exists to avoid.
///
/// # Errors
///
/// Returns [`ReplaceError::ImpossiblePath`] if a path cannot be passed to the platform, and
/// [`ReplaceError::Refused`] with the system's own error if the replacement failed. In either
/// case nothing has changed: the destination is whatever it was.
#[cfg_attr(
    windows,
    expect(
        unsafe_code,
        reason = "one call to MoveFileExW, which the standard library does not expose with the replace flag; the call carries its own SAFETY comment"
    )
)]
pub fn replace(source: &Path, destination: &Path) -> Result<(), ReplaceError> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };

        let from = wide(source)?;
        let to = wide(destination)?;

        // SAFETY: both pointers are to null-terminated UTF-16 buffers that live for the whole
        // call, built above and not moved. `MoveFileExW` reads them and writes through
        // neither. The flags are two constants from the platform crate. Nothing else of this
        // program's memory is reachable from here.
        let moved = unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };

        if moved == 0 {
            Err(ReplaceError::Refused(io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    #[cfg(not(windows))]
    {
        // `rename(2)` replaces the destination atomically and has done since it was written.
        // There is nothing to add and nothing unsafe to reach for.
        std::fs::rename(source, destination).map_err(ReplaceError::Refused)
    }
}

/// A path as a null-terminated UTF-16 buffer, which is what the Windows call takes.
///
/// Refuses an interior null rather than stopping at it. A path truncated at a null names a
/// different file, and the caller is about to replace whatever it names.
#[cfg(windows)]
fn wide(path: &Path) -> Result<Vec<u16>, ReplaceError> {
    use std::os::windows::ffi::OsStrExt as _;

    let mut encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    if encoded.contains(&0) {
        return Err(ReplaceError::ImpossiblePath);
    }
    encoded.push(0);

    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{ReplaceError, replace};

    /// A directory of its own for one test, removed when it ends.
    struct Scratch {
        directory: PathBuf,
    }

    impl Scratch {
        fn new(label: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "cairn-replace-{label}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&directory).expect("a temporary directory can be created");

            Self { directory }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.directory.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn write(path: &Path, contents: &str) {
        fs::write(path, contents).expect("the file can be written");
    }

    #[test]
    fn a_file_replaces_one_that_is_already_there() {
        // The whole reason this module exists. `std::fs::rename` fails here on Windows.
        let scratch = Scratch::new("over");
        let source = scratch.path("new");
        let destination = scratch.path("live");

        write(&source, "the restored one");
        write(&destination, "the one that was there");

        replace(&source, &destination).expect("the replacement happens");

        assert_eq!(
            fs::read_to_string(&destination).expect("the destination is readable"),
            "the restored one"
        );
        assert!(!source.exists(), "the source is still there afterwards");
    }

    #[test]
    fn a_file_moves_into_a_name_that_is_free() {
        let scratch = Scratch::new("fresh");
        let source = scratch.path("new");
        let destination = scratch.path("live");

        write(&source, "contents");

        replace(&source, &destination).expect("the move happens");

        assert_eq!(
            fs::read_to_string(&destination).expect("the destination is readable"),
            "contents"
        );
    }

    #[test]
    fn replacing_from_a_source_that_is_not_there_changes_nothing() {
        // The failure that matters most: if the new database is missing, the old one has to
        // still be the old one afterwards.
        let scratch = Scratch::new("missing");
        let source = scratch.path("not-here");
        let destination = scratch.path("live");

        write(&destination, "the one that was there");

        let refused = replace(&source, &destination).expect_err("a missing source is refused");

        assert!(matches!(refused, ReplaceError::Refused(_)), "{refused:?}");
        assert_eq!(
            fs::read_to_string(&destination).expect("the destination is readable"),
            "the one that was there",
            "the destination was disturbed by a failed replacement"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_path_with_an_interior_null_is_refused_rather_than_truncated() {
        // A path truncated at a null names a different file, and this call replaces whatever
        // it names. No real path has one; a path built from a string somebody supplied could.
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt as _;

        let scratch = Scratch::new("null");
        let destination = scratch.path("live");
        write(&destination, "the one that was there");

        let nasty = PathBuf::from(OsString::from_wide(&[u16::from(b'a'), 0, u16::from(b'b')]));

        let refused = replace(&nasty, &destination).expect_err("an impossible path is refused");

        assert!(
            matches!(refused, ReplaceError::ImpossiblePath),
            "{refused:?}"
        );
        assert_eq!(
            fs::read_to_string(&destination).expect("the destination is readable"),
            "the one that was there"
        );
    }
}
