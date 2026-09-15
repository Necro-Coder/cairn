//! Build script for the Cairn application.
//!
//! `tauri_build::build()` reads `tauri.conf.json`, resolves the capability files into the
//! access control list the runtime enforces, and embeds the Windows resource data. It has
//! to run before the crate compiles, which is why the configuration is checked by a test
//! as well: a mistake in that file is not a compile error, it is a silently weaker
//! application.
fn main() {
    tauri_build::build();
}
