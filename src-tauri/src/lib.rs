//! The Cairn application shell.
//!
//! Everything lives in the library rather than in `main.rs` so that the desktop binary
//! and, later, the iOS entry point drive exactly the same code.

pub mod commands;
pub mod state;

use state::AppState;

/// Starts the application and blocks until the last window closes.
///
/// # Panics
///
/// Panics if Tauri cannot build the application, which means the bundled configuration
/// or the generated context is broken. That is a build time mistake rather than a
/// runtime condition, and there is no meaningful way to carry on without a window, so
/// failing loudly here is the correct outcome.
#[expect(
    clippy::expect_used,
    reason = "the context is generated at build time, so a failure here means the bundled configuration is broken; there is no window left in which to report it and no safe state to continue from"
)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::app_info::app_info,
            commands::diagnostics::diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("the application context is generated at build time and must be valid");
}
