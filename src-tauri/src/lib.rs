//! The Cairn application shell.
//!
//! Everything lives in the library rather than in `main.rs` so that the desktop binary
//! and, later, the iOS entry point drive exactly the same code.

pub mod clipboard;
pub mod clock;
pub mod commands;
pub mod instance;
pub mod session;
pub mod state;
pub mod storage;
pub mod vault;
pub mod vault_file;
pub mod window;

use tauri::Manager as _;

use instance::Instance;
use state::AppState;
use storage::DataDirectory;
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
    // Before anything allocates a key. Without it, the allowance a process gets on Windows is
    // about a megabyte and a half, which is smaller than the buffers the cipher works in, so
    // every request to keep a page out of the page file is refused and the protection the
    // design describes does not happen. A refusal here is not fatal and is not hidden: the
    // diagnostics screen reports whether the keys are actually resident.
    let _raised =
        cairn_platform::memory::allow_resident(cairn_platform::memory::RECOMMENDED_RESIDENT_BYTES);

    tauri::Builder::default()
        // The native file dialog, and nothing else from this plugin. It is registered here so
        // that Rust can open one; the WebView cannot, because `capabilities/default.json`
        // grants it none of the plugin's permissions. That is the whole reason a path is never
        // a parameter of a backup command.
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Our own directory rather than the one the framework offers. Two reasons, and
            // both of them are about what happens to somebody's data years from now. The
            // framework's answer is the roaming profile on Windows, which a domain copies
            // between machines, and a SQLite file copied while it is open is a corrupt SQLite
            // file. And the name it builds the directory from is the bundle identifier, so
            // changing the identifier would move the vault and leave the old one behind,
            // looking exactly like a machine that never had one.
            //
            // A refusal here is not propagated. Returning the error would close the window
            // before anything could be said in it, and a window that vanishes is what somebody
            // who typed a profile name wrong actually sees: an application that does not
            // start. The cause is dropped rather than carried, because it names the path and
            // the path names the account.
            let directory = cairn_platform::paths::data_directory()
                .ok()
                .map(DataDirectory::new);

            // Before the vault is read, and before anything is managed. Two copies of this
            // application with the same database file open is two connections, two logical
            // clocks issuing readings from the same device identifier, and two sets of keys in
            // memory; the clock is the part that does real damage, because both would hand out
            // readings a row already carries.
            let instance = directory
                .as_ref()
                .map_or_else(Instance::without_a_directory, Instance::take);
            let may_continue = instance.status().may_use_the_directory();
            app.manage(instance);

            // A refused copy stops here, and stopping here is the point: it does not read the
            // header, it does not create a file, and it does not start the watchdog. What it
            // does is draw the screen that says why, which is the one thing a refused copy can
            // usefully do. The interface asks `instance_status` before anything else.
            let Some(directory) = directory.filter(|_at| may_continue) else {
                return Ok(());
            };

            let vault = Vault::open_at(directory.path())?;
            app.manage(AppState::new(vault, directory));

            // Started after the state is managed, because the first thing it does is ask for
            // it. In the core rather than in the interface: a WebView that was made to stop
            // sending heartbeats must not be able to hold the vault open.
            window::spawn_watchdog(app.handle().clone());

            Ok(())
        })
        .on_window_event(window::on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::app_info::app_info,
            commands::backup::backup_export,
            commands::backup::backup_verify,
            commands::backup::backup_status,
            commands::backup::backup_import_begin,
            commands::backup::backup_import_commit,
            commands::backup::backup_import_cancel,
            commands::backup::export_plaintext,
            commands::instance::instance_status,
            commands::diagnostics::diagnostics,
            commands::habits::habits_list,
            commands::habits::habits_get,
            commands::habits::habits_create,
            commands::habits::habits_update,
            commands::habits::habits_update_preview,
            commands::habits::habits_archive,
            commands::habits::habits_delete,
            commands::habits::habits_reorder,
            commands::habits::habits_toggle_day,
            commands::habits::habits_heatmap,
            commands::habits::habits_stats,
            commands::passwords::passwords_list,
            commands::passwords::passwords_search,
            commands::passwords::passwords_get,
            commands::passwords::passwords_history,
            commands::passwords::passwords_folders,
            commands::passwords::passwords_settings,
            commands::passwords::passwords_create,
            commands::passwords::passwords_update,
            commands::passwords::passwords_folder_save,
            commands::passwords::passwords_folder_delete,
            commands::passwords::passwords_folders_reorder,
            commands::passwords::passwords_history_clear,
            commands::passwords::passwords_trash,
            commands::passwords::passwords_delete,
            commands::passwords::passwords_empty_trash,
            commands::passwords::passwords_reveal,
            commands::passwords::passwords_copy,
            commands::passwords::passwords_set_clipboard_seconds,
            commands::sample::diagnostics_insert_sample_habit,
            commands::sample::diagnostics_list_sample_habits,
            commands::sample::diagnostics_delete_sample_habit,
            commands::sample::diagnostics_delete_sample_habits,
            commands::sample::diagnostics_seed_data,
            commands::sample::diagnostics_compact_tombstones,
            commands::vault::vault_status,
            commands::vault::vault_create,
            commands::vault::vault_unlock,
            commands::vault::vault_lock,
            commands::vault::vault_change_password,
            commands::vault::vault_change_kdf_params,
            commands::vault::session_heartbeat,
            commands::vault::session_set_inactivity,
            commands::vault::password_strength,
            window::start_window_drag,
            window::minimize_window,
            window::toggle_maximize_window,
            window::close_window
        ])
        .run(tauri::generate_context!())
        .expect("the application context is generated at build time and must be valid");
}
