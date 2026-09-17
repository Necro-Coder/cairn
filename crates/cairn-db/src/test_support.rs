//! A directory that exists for one test and is removed when it ends.
//!
//! Unique per test and per run. Per run as well as per test because a file left behind by a
//! crashed run would otherwise be opened by the next one, and an encrypted file from a
//! different key turns a real failure into a confusing one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A temporary directory, removed when the value is dropped.
pub(crate) struct Scratch {
    directory: PathBuf,
}

impl Scratch {
    /// A directory of its own for the test that names it.
    pub(crate) fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let ordinal = COUNTER.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "cairn-db-{label}-{}-{nanos}-{ordinal}",
            process::id()
        ));

        fs::create_dir_all(&directory).expect("a temporary directory can be created");

        Self { directory }
    }

    /// The directory itself.
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    /// Where the database file goes, under the name the application uses.
    pub(crate) fn database_path(&self) -> PathBuf {
        self.directory.join(crate::DATABASE_FILE)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
