//! Whether this copy of the application owns the data directory.
//!
//! The first thing the interface asks, before it draws anything else. Everything else in the
//! application assumes the vault is reachable, and in a second copy it is not.

use crate::instance::{Instance, InstanceStatus};

/// What this process is allowed to do with the data directory.
///
/// Cannot fail. The lock was taken, or not taken, before the window existed; this reports what
/// happened. A command that could fail here would leave the interface with nothing to draw at
/// the one moment it most needs something to say.
#[tauri::command]
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and requires the state guard by value"
)]
pub fn instance_status(instance: tauri::State<'_, Instance>) -> InstanceStatus {
    instance.status()
}
