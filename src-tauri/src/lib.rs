//! The Cairn application shell.
//!
//! Everything lives in the library rather than in `main.rs` so that the desktop binary
//! and, later, the iOS entry point drive exactly the same code.

pub mod clock;
pub mod commands;
pub mod session;
pub mod state;
pub mod vault;
pub mod vault_file;
pub mod window;

use tauri::Manager as _;

use state::AppState;
use vault::Vault;

/// Starts the application and blocks until the last window closes.
///
/// The vault is read once here, before the first window is drawn, because every command
/// afterwards works from what that read found. A header that cannot be parsed is not a
/// reason to refuse to start: the application still has to be able to say so, and it has to
/// refuse to create a second vault over the top of the first.
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
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            app.manage(AppState::new(Vault::open_at(&directory)?));

            // Started after the state is managed, because the first thing it does is ask for
            // it. In the core rather than in the interface: a WebView that was made to stop
            // sending heartbeats must not be able to hold the vault open.
            window::spawn_watchdog(app.handle().clone());

            Ok(())
        })
        .on_window_event(window::on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::app_info::app_info,
            commands::diagnostics::diagnostics,
            commands::vault::vault_status,
            commands::vault::vault_create,
            commands::vault::vault_unlock,
            commands::vault::vault_lock,
            commands::vault::vault_change_password,
            commands::vault::vault_change_kdf_params,
            commands::vault::session_heartbeat,
            commands::vault::password_strength
        ])
        .run(tauri::generate_context!())
        .expect("the application context is generated at build time and must be valid");
}
