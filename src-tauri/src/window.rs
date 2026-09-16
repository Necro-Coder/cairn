//! What the window does to the vault, and the one thing the vault says back.
//!
//! Three things close the vault without anybody asking. Nobody has touched the window for as
//! long as the setting allows. The window has been away from the front for longer than the
//! grace period. Or the window was minimised, which closes it at once.
//!
//! The distinction between the last two is deliberate. Switching away for a second to copy
//! something out of another program happens constantly, and a vault that closed the moment it
//! was not in front would become a vault whose automatic locking somebody turns off. Nobody
//! minimises a window they are about to use, so that one needs no grace at all.
//!
//! Activity means keyboard or mouse inside this window, reported by the interface. System
//! activity is never asked about and would be the wrong question: somebody typing in another
//! application is not somebody using this one, and a vault held open by a different program
//! being busy is a vault that stays open all day.
//!
//! One event goes the other way, and only one. [`LOCKED_EVENT`] carries why it happened so
//! that the screen which appears can say so. Everything else the interface wants it asks for,
//! because a command that answers a question is easier to reason about than a stream of
//! announcements that may have been missed.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter as _, Manager as _, Runtime, WindowEvent};

use crate::clock::now_us;
use crate::session::LockReason;
use crate::state::AppState;

/// The only event this application sends to the interface.
pub const LOCKED_EVENT: &str = "session://locked";

/// How often the watchdog looks at the clock.
///
/// A second. The two things it decides are measured in tens of seconds and in minutes, so
/// this is precise enough to be honest about the countdown the interface is drawing, and
/// coarse enough to cost nothing.
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);

/// What goes with the event.
///
/// A reason and nothing else. There is no state here that a command does not already answer,
/// and an event carrying state is an event that can be missed and leave the interface wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedEvent {
    /// Why the vault closed.
    pub reason: LockReason,
}

/// Closes the vault and tells the interface, if it was open.
///
/// Silent when it was already closed, because an event for something that did not happen
/// would have the interface discard state it has already discarded and, worse, show the
/// reason for a lock that was not this one.
pub fn lock_and_announce<R: Runtime>(app: &AppHandle<R>, reason: LockReason) {
    if !app.state::<AppState>().session().lock() {
        return;
    }

    announce(app, reason);
}

/// Tells the interface the vault has closed.
///
/// A failure to send is deliberately not propagated. The keys are already gone, which is the
/// part that matters; the window that did not hear about it is a window that is closing or
/// already closed, and there is nowhere left to report the failure to.
fn announce<R: Runtime>(app: &AppHandle<R>, reason: LockReason) {
    let _delivered = app.emit(LOCKED_EVENT, LockedEvent { reason });
}

/// Applies a window event to the session.
///
/// Called for every window, and the vault is one per process rather than one per window, so
/// what matters is what happened rather than which window it happened to.
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
    let app = window.app_handle();

    match *event {
        WindowEvent::Focused(true) => app.state::<AppState>().session().note_focus_gained(),
        WindowEvent::Focused(false) => app.state::<AppState>().session().note_focus_lost(now_us()),
        // There is no event for being minimised, so the size change is where it is noticed.
        // A window that cannot answer whether it is minimised is treated as not being so:
        // the inactivity timer still covers it, and locking on a question nobody could
        // answer would close the vault of somebody who only resized their window.
        WindowEvent::Resized(_) if window.is_minimized().unwrap_or(false) => {
            lock_and_announce(app, LockReason::Minimised);
        }
        _ => {}
    }
}

/// Starts the task that closes the vault when its own clock says to.
///
/// In the core rather than in the interface, because the interface is the part an attacker
/// reaches first. A WebView that stopped sending heartbeats, or was made to stop, must not be
/// able to hold the vault open, so the decision is taken here and the interface is told.
pub fn spawn_watchdog<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(WATCHDOG_INTERVAL).await;

            if let Some(reason) = app.state::<AppState>().session().lock_if_due(now_us()) {
                announce(&app, reason);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::session::LockReason;

    use super::{LOCKED_EVENT, LockedEvent, WATCHDOG_INTERVAL};

    #[test]
    fn the_event_has_the_name_the_interface_listens_for() {
        // Renaming this silently would leave an interface that never hears about a lock and
        // keeps drawing what it had.
        assert_eq!(LOCKED_EVENT, "session://locked");
    }

    #[test]
    fn the_payload_is_a_reason_and_nothing_else() {
        assert_eq!(
            serde_json::to_string(&LockedEvent {
                reason: LockReason::Inactivity
            })
            .expect("the payload serialises"),
            r#"{"reason":"inactivity"}"#
        );
    }

    #[test]
    fn every_reason_has_a_name_the_interface_can_match_on() {
        for (reason, name) in [
            (LockReason::Inactivity, "inactivity"),
            (LockReason::FocusLost, "focusLost"),
            (LockReason::Minimised, "minimised"),
            (LockReason::Requested, "requested"),
        ] {
            assert_eq!(
                serde_json::to_string(&reason).expect("the reason serialises"),
                format!("\"{name}\"")
            );
        }
    }

    #[test]
    fn the_watchdog_looks_often_enough_to_be_honest_about_the_countdown() {
        // The interface draws a countdown in seconds. A watchdog slower than that would let
        // it show zero for a while with the vault still open.
        assert!(WATCHDOG_INTERVAL.as_secs() <= 1);
    }
}
