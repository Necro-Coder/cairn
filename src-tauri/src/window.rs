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
//!
//! The decisions are separated from the plumbing on purpose. [`outcome_of`] turns an event
//! and one flag into what should happen, and [`apply`] turns that into what did happen. Both
//! are ordinary functions over ordinary values, so the rules above are tested rather than
//! only described; what is left needs a real window and is three lines long.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter as _, Manager as _, Runtime, WindowEvent};

use crate::clock::now_us;
use crate::session::{LockReason, Session};
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

/// What something that happened to the window means for the vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowOutcome {
    /// The window is in front again, so the countdown is cancelled.
    BackInFront,
    /// The window is no longer in front, so the countdown starts.
    Gone,
    /// Close the vault now, for this reason.
    CloseNow(LockReason),
    /// Nothing about this concerns the vault.
    Nothing,
}

/// Decides what a window event means, given whether the window is minimised.
///
/// The flag is a parameter rather than something read in here, because asking a window
/// whether it is minimised needs a window and deciding what to do about it does not.
///
/// A window that could not answer the question is passed `false` by the caller. That is the
/// safe direction: the inactivity timer still covers a minimised window, and closing the
/// vault of somebody who merely resized theirs would not be recoverable by waiting.
#[must_use]
pub fn outcome_of(event: &WindowEvent, minimised: bool) -> WindowOutcome {
    match *event {
        WindowEvent::Focused(true) => WindowOutcome::BackInFront,
        WindowEvent::Focused(false) => WindowOutcome::Gone,
        // There is no event for being minimised, so the size change is where it is noticed.
        WindowEvent::Resized(_) if minimised => WindowOutcome::CloseNow(LockReason::Minimised),
        _ => WindowOutcome::Nothing,
    }
}

/// Applies an outcome to the session, answering why the vault closed if it did.
///
/// Answering rather than announcing, because sending the event needs a handle to the
/// application and deciding whether there is anything to send does not.
///
/// A close that found the vault already shut answers `None`. An event for something that did
/// not happen would have the interface discard state it has already discarded and, worse,
/// show the reason for a lock that was not this one.
pub fn apply(session: &Session, outcome: WindowOutcome, now_us: i64) -> Option<LockReason> {
    match outcome {
        WindowOutcome::BackInFront => {
            session.note_focus_gained();
            None
        }
        WindowOutcome::Gone => {
            session.note_focus_lost(now_us);
            None
        }
        WindowOutcome::CloseNow(reason) => session.lock().then_some(reason),
        WindowOutcome::Nothing => None,
    }
}

/// Closes the vault and tells the interface, if it was open.
pub fn lock_and_announce<R: Runtime>(app: &AppHandle<R>, reason: LockReason) {
    let closed = apply(
        app.state::<AppState>().session(),
        WindowOutcome::CloseNow(reason),
        now_us(),
    );

    if let Some(reason) = closed {
        announce(app, reason);
    }
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
    let outcome = outcome_of(event, window.is_minimized().unwrap_or(false));
    let app = window.app_handle();

    // Asked for rather than assumed. A second copy of the application refused the instance
    // lock never opens the vault, so there is no session for its window to affect, and a
    // handler that took the state unconditionally would panic while drawing the screen whose
    // whole job is to explain the refusal calmly.
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };

    if let Some(reason) = apply(state.session(), outcome, now_us()) {
        announce(app, reason);
    }
}

/// The label of the one window this application has.
///
/// Written once here rather than at each call site, and asserted against `tauri.conf.json`
/// by `tests/config_hardening.rs`. A typo would make every window control silently refuse.
const MAIN_WINDOW: &str = "main";

/// Why a window control could not do what it was asked.
///
/// One variant, because there is one thing that can go wrong: the window is not there, or
/// the window manager refused. None of these commands takes a parameter, so there is no
/// input to be wrong about.
///
/// Serialised as a tagged object, the same shape as [`crate::commands::VaultError`], so the
/// interface matches on `kind` rather than parsing a sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[non_exhaustive]
pub enum WindowError {
    /// The window is gone, or the window manager would not do it.
    #[error("the window is not available")]
    Unavailable,
}

/// The window the controls act on.
fn main_window<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::WebviewWindow<R>, WindowError> {
    app.get_webview_window(MAIN_WINDOW)
        .ok_or(WindowError::Unavailable)
}

/// Runs the two halves of closing the application, in the order they have to happen.
///
/// The vault is closed first and the window second. The other order looks equivalent and is
/// not: closing the window starts tearing down the WebView and the event loop with the key
/// still live in memory, and however short that window is, it is a window in which the
/// process holds a decrypted key with nothing left running that would clear it.
///
/// Separated from the commands and written over two closures so that the order is a
/// property something can test, rather than two lines in a function that needs a real
/// window to run at all. The same division as [`outcome_of`] and [`apply`] above.
///
/// # Errors
///
/// Whatever closing the window failed with. The vault has been closed either way, which is
/// the half that matters and the reason it goes first.
pub fn shut_down<E>(
    close_vault: impl FnOnce(),
    close_window: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    close_vault();
    close_window()
}

/// Takes the window into a drag, because the title bar is ours to draw.
///
/// A command of our own rather than `core:window:allow-start-dragging` in the capability
/// list. The standard route would add four core APIs to what script injected into the
/// WebView could call, and the list is currently two permissions that can only listen to
/// events this application emits.
///
/// Saying that a custom command leaves the surface untouched would be flattering it, and
/// this is a file an audit reads. These four are reachable from injected script exactly as a
/// core API would be — that is what an IPC command is. What differs is their shape: none of
/// them takes an argument, and each acts on the one window named in the configuration, so
/// what a compromised WebView gains is the ability to move, minimise, maximise or close that
/// window rather than a general window API to aim. Minimising and closing both shut the
/// vault, which is the direction a surface should fail in.
///
/// # Errors
///
/// [`WindowError::Unavailable`] if the window is gone or the window manager refused.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and hands the handle over by value"
)]
pub fn start_window_drag(app: AppHandle) -> Result<(), WindowError> {
    main_window(&app)?
        .start_dragging()
        .map_err(|_error| WindowError::Unavailable)
}

/// Minimises the window.
///
/// Closing the vault is not done here. Being minimised already closes it, through
/// [`on_window_event`], which is where it belongs: the vault has to close whether the
/// window was minimised from this button or from the keyboard.
///
/// # Errors
///
/// [`WindowError::Unavailable`] if the window is gone or the window manager refused.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and hands the handle over by value"
)]
pub fn minimize_window(app: AppHandle) -> Result<(), WindowError> {
    main_window(&app)?
        .minimize()
        .map_err(|_error| WindowError::Unavailable)
}

/// Maximises the window, or restores it, and answers which it now is.
///
/// Answering rather than leaving the interface to ask means the button can redraw itself
/// from the result of the press instead of from a second round trip that could disagree
/// with it.
///
/// # Errors
///
/// [`WindowError::Unavailable`] if the window is gone or the window manager refused.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and hands the handle over by value"
)]
pub fn toggle_maximize_window(app: AppHandle) -> Result<bool, WindowError> {
    let window = main_window(&app)?;
    let maximised = window
        .is_maximized()
        .map_err(|_error| WindowError::Unavailable)?;

    if maximised {
        window
            .unmaximize()
            .map_err(|_error| WindowError::Unavailable)?;
    } else {
        window
            .maximize()
            .map_err(|_error| WindowError::Unavailable)?;
    }

    Ok(!maximised)
}

/// Closes the vault, and then the window.
///
/// That order, and it is the one sequence in this module where the order is the point. See
/// [`shut_down`].
///
/// # Errors
///
/// [`WindowError::Unavailable`] if the window is gone or the window manager refused. The
/// vault is closed before that can happen.
#[tauri::command]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the command macro generates the call and hands the handle over by value"
)]
pub fn close_window(app: AppHandle) -> Result<(), WindowError> {
    let window = main_window(&app)?;

    shut_down(
        || lock_and_announce(&app, LockReason::Requested),
        || window.close().map_err(|_error| WindowError::Unavailable),
    )
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
    use std::cell::RefCell;

    use cairn_crypto::{Argon2Params, MAX_LANES, MIN_MEMORY_KIB, MIN_PASSES};
    use cairn_domain::session::InactivityTimeout;
    use tauri::{PhysicalSize, WindowEvent};

    use super::{
        LOCKED_EVENT, LockedEvent, MAIN_WINDOW, WATCHDOG_INTERVAL, WindowError, WindowOutcome,
        apply, outcome_of, shut_down,
    };
    use crate::session::{LockReason, Session};

    /// A moment in the middle of the range.
    const NOW_US: i64 = 1_700_000_000_000_000;

    /// Thirty seconds, the grace period a window that lost focus gets.
    const GRACE_US: i64 = 30 * 1_000_000;

    /// A size change, which is the only way being minimised reaches this module.
    fn resized() -> WindowEvent {
        WindowEvent::Resized(PhysicalSize::new(800, 600))
    }

    /// A session with the vault open, to apply outcomes to.
    fn an_open_session() -> Session {
        let params = Argon2Params::new(MIN_MEMORY_KIB, MIN_PASSES, MAX_LANES)
            .expect("the lowest accepted parameters are accepted");
        let (_header, vault) = cairn_crypto::create("una frase larga para la prueba", params, 0)
            .expect("creating a vault at the lowest parameters cannot fail here");

        let session = Session::new(InactivityTimeout::default(), NOW_US);
        tauri::async_runtime::block_on(session.unlock_with(move || Ok(vault), NOW_US))
            .expect("the vault opens");

        session
    }

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

    #[test]
    fn gaining_and_losing_focus_are_opposite_answers() {
        assert_eq!(
            outcome_of(&WindowEvent::Focused(true), false),
            WindowOutcome::BackInFront
        );
        assert_eq!(
            outcome_of(&WindowEvent::Focused(false), false),
            WindowOutcome::Gone
        );
    }

    #[test]
    fn being_minimised_closes_the_vault_at_once() {
        // Nobody minimises a window they are about to use, so this one needs no grace.
        assert_eq!(
            outcome_of(&resized(), true),
            WindowOutcome::CloseNow(LockReason::Minimised)
        );
    }

    #[test]
    fn merely_resizing_a_window_does_nothing_to_the_vault() {
        // The same event arrives for an ordinary resize, and a window that could not answer
        // whether it is minimised is passed false, so this is also what happens then. Closing
        // the vault of somebody who dragged a corner would not be recoverable by waiting.
        assert_eq!(outcome_of(&resized(), false), WindowOutcome::Nothing);
    }

    #[test]
    fn events_that_are_nothing_to_do_with_the_vault_are_left_alone() {
        assert_eq!(
            outcome_of(&WindowEvent::Destroyed, false),
            WindowOutcome::Nothing
        );
        assert_eq!(
            outcome_of(
                &WindowEvent::Moved(tauri::PhysicalPosition::new(10, 10)),
                true
            ),
            WindowOutcome::Nothing,
            "a window being moved while minimised closed the vault"
        );
    }

    #[test]
    fn losing_focus_starts_a_countdown_that_coming_back_cancels() {
        let session = an_open_session();

        assert_eq!(apply(&session, WindowOutcome::Gone, NOW_US), None);
        assert_eq!(
            session.due_to_lock(NOW_US + GRACE_US),
            Some(LockReason::FocusLost)
        );

        assert_eq!(apply(&session, WindowOutcome::BackInFront, NOW_US), None);
        assert_eq!(session.due_to_lock(NOW_US + GRACE_US), None);
        assert!(session.is_unlocked());
    }

    #[test]
    fn closing_now_reports_the_reason_and_leaves_the_vault_shut() {
        let session = an_open_session();

        assert_eq!(
            apply(
                &session,
                WindowOutcome::CloseNow(LockReason::Minimised),
                NOW_US
            ),
            Some(LockReason::Minimised)
        );
        assert!(!session.is_unlocked());
    }

    #[test]
    fn closing_a_vault_that_was_already_shut_announces_nothing() {
        // An event for something that did not happen would have the interface show the reason
        // for a lock that was not this one.
        let session = Session::new(InactivityTimeout::default(), NOW_US);

        assert_eq!(
            apply(
                &session,
                WindowOutcome::CloseNow(LockReason::Requested),
                NOW_US
            ),
            None
        );
    }

    #[test]
    fn closing_the_application_closes_the_vault_before_the_window() {
        // The one sequence in this module where the order is the point. The other way round
        // starts tearing down the WebView and the event loop with the key still live, and
        // however short that is, it is a moment with a decrypted key in a process that has
        // nothing left running to clear it.
        // A cell rather than a plain vector, because both closures have to write to the
        // same list and the borrow checker will not hand out two mutable borrows of one.
        let order = RefCell::new(Vec::new());

        let closed: Result<(), ()> = shut_down(
            || order.borrow_mut().push("vault"),
            || {
                order.borrow_mut().push("window");
                Ok(())
            },
        );

        assert_eq!(closed, Ok(()));
        assert_eq!(order.into_inner(), ["vault", "window"]);
    }

    #[test]
    fn a_window_that_refuses_to_close_has_still_had_its_vault_closed() {
        // Which is why the vault goes first. A window manager that says no leaves the
        // application running, and it has to be running with the keys already gone.
        let mut closed_the_vault = false;

        let closed = shut_down(|| closed_the_vault = true, || Err(WindowError::Unavailable));

        assert_eq!(closed, Err(WindowError::Unavailable));
        assert!(closed_the_vault);
    }

    #[test]
    fn the_window_error_is_a_tagged_object_the_interface_can_match_on() {
        assert_eq!(
            serde_json::to_string(&WindowError::Unavailable).expect("the error serialises"),
            r#"{"kind":"unavailable"}"#
        );
    }

    #[test]
    fn the_controls_act_on_the_window_the_configuration_declares() {
        // `tests/config_hardening.rs` asserts the other half: that the configuration still
        // declares a window with this label. A typo would make every control silently
        // refuse, with nothing on screen to say why.
        assert_eq!(MAIN_WINDOW, "main");
    }

    #[test]
    fn an_outcome_of_nothing_changes_nothing() {
        let session = an_open_session();

        assert_eq!(apply(&session, WindowOutcome::Nothing, NOW_US), None);
        assert!(session.is_unlocked());
        assert_eq!(session.due_to_lock(NOW_US + GRACE_US), None);
    }
}
