//! What this process put on the clipboard, and the one timer that will take it back.
//!
//! The platform crate knows how to write to the clipboard and how to empty it if what is on it
//! still digests to what was put there. What it does not know is *when*, or that there must only
//! ever be one wipe waiting. That is here, because both of those are decisions about how this
//! application behaves rather than about how the system works.
//!
//! Two rules shape the whole module. There is exactly one timer alive: copying something else
//! cancels the one before it, because a timer armed for the previous secret would wipe the new
//! one early. And what is waiting is a *digest*, never the value, so that a wipe which happens
//! long after somebody has forgotten they copied anything does not need the secret to be still
//! sitting in this process to do its job.
//!
//! The backend is a trait so the commands can be driven against a double. A clipboard is a single
//! shared resource of the whole desktop: a test that used the real one would replace whatever the
//! person running it had copied, and would fail on a build agent with no desktop at all.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cairn_platform::clipboard::{self, ClipboardError, Fingerprint};

/// Writing to the clipboard, and taking it back, as this application needs it.
///
/// Three methods and no state. Everything that has to be remembered between a copy and the wipe
/// that follows is remembered by [`ClipboardKeeper`], so an implementation of this is free to be
/// nothing but the system calls.
pub trait Clipboard: Send + Sync + 'static {
    /// Puts text on the clipboard, replacing what was there.
    ///
    /// # Errors
    ///
    /// Whatever the platform says.
    fn write(&self, text: &str) -> Result<(), ClipboardError>;

    /// Empties the clipboard, but only if what is on it still digests to `expected`.
    ///
    /// # Errors
    ///
    /// Whatever the platform says. A read that fails must empty nothing.
    fn clear_if_matches(&self, expected: &Fingerprint) -> Result<bool, ClipboardError>;

    /// The digest of what is on the clipboard now, or `None` if it holds no text.
    ///
    /// # Errors
    ///
    /// Whatever the platform says.
    fn fingerprint(&self) -> Result<Option<Fingerprint>, ClipboardError>;
}

/// The clipboard of the machine this is running on.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn write(&self, text: &str) -> Result<(), ClipboardError> {
        clipboard::write(text)
    }

    fn clear_if_matches(&self, expected: &Fingerprint) -> Result<bool, ClipboardError> {
        clipboard::clear_if_matches(expected)
    }

    fn fingerprint(&self) -> Result<Option<Fingerprint>, ClipboardError> {
        clipboard::fingerprint()
    }
}

/// A wipe that has been armed and has not happened yet.
struct Pending {
    /// What this process put there. Never the value itself.
    expected: Fingerprint,
    /// The task that will do the wiping, so that it can be called off.
    task: tauri::async_runtime::JoinHandle<()>,
}

/// Holds what was copied and the single timer that will take it back.
pub struct ClipboardKeeper {
    backend: Arc<dyn Clipboard>,
    /// The one wipe waiting, if there is one.
    pending: Mutex<Option<Pending>>,
}

impl core::fmt::Debug for ClipboardKeeper {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Deliberately says nothing about what is on the clipboard. The fingerprint is not a
        // secret, but a debug line saying one is waiting is a debug line about somebody having
        // just copied a password, and this type ends up inside the state that gets printed.
        formatter.write_str("ClipboardKeeper")
    }
}

impl ClipboardKeeper {
    /// One over the clipboard of this machine.
    #[must_use]
    pub fn system() -> Self {
        Self::with(Arc::new(SystemClipboard))
    }

    /// One over whatever is given, for the tests.
    #[must_use]
    pub fn with(backend: Arc<dyn Clipboard>) -> Self {
        Self {
            backend,
            pending: Mutex::new(None),
        }
    }

    /// Puts a value on the clipboard and arms the wipe that will take it back.
    ///
    /// `after` is how long to wait. `None` is for what is not wiped at all — a user name is not a
    /// secret of the same kind, and taking it back off the clipboard would get in the way more
    /// than it protects.
    ///
    /// Whatever was waiting is called off first, before anything is written. A timer armed for
    /// the previous secret would wipe this one early, and the clipboard holds one thing at a
    /// time, so there is never a reason for two.
    ///
    /// # Errors
    ///
    /// Whatever the platform says. Nothing is armed if the write did not happen.
    pub fn copy(&self, value: &str, after: Option<Duration>) -> Result<(), ClipboardError> {
        self.cancel();
        self.backend.write(value)?;

        let Some(after) = after else {
            return Ok(());
        };

        let expected = clipboard::fingerprint_of(value);
        let backend = Arc::clone(&self.backend);
        let task = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(after).await;
            // Dropped on purpose. By the time this wakes up there is nobody left to tell: the
            // screen that asked has moved on, and a clipboard that could not be read is one that
            // must be left exactly as it is rather than emptied blind.
            let _wiped = backend.clear_if_matches(&expected);
        });

        self.hold(Pending { expected, task });

        Ok(())
    }

    /// Calls off the waiting wipe and does it now instead.
    ///
    /// What the lock calls. Locking the vault has to mean something, and leaving a password on
    /// the clipboard for another fourteen seconds after somebody deliberately shut their vault is
    /// the sort of thing that makes the lock decorative.
    ///
    /// The same check as the timer would have made: if somebody copied something else in the
    /// meantime, theirs is left alone. Answers whether it emptied anything, which is what the
    /// test asserts on; the caller in the lock path has nothing to do with the answer.
    pub fn wipe_now(&self) -> bool {
        let Some(waiting) = self.take() else {
            return false;
        };
        waiting.task.abort();

        self.backend
            .clear_if_matches(&waiting.expected)
            .unwrap_or(false)
    }

    /// Whether a wipe is waiting. For the tests, and for nothing else.
    #[must_use]
    pub fn is_armed(&self) -> bool {
        self.guard().is_some()
    }

    /// Calls off the waiting wipe without doing it.
    fn cancel(&self) {
        if let Some(waiting) = self.take() {
            waiting.task.abort();
        }
    }

    /// Puts a wipe in the slot, calling off anything that somehow got there first.
    fn hold(&self, pending: Pending) {
        if let Some(previous) = self.guard().replace(pending) {
            previous.task.abort();
        }
    }

    /// Takes whatever is waiting out of the slot.
    fn take(&self) -> Option<Pending> {
        self.guard().take()
    }

    /// Takes the slot, recovering from a poisoned lock the way the rest of this application does.
    ///
    /// There is no invariant to protect that a panic could have broken. The worst a half finished
    /// swap leaves behind is a timer that fires and finds a digest that does not match, which is
    /// the ordinary case of somebody having copied something else.
    fn guard(&self) -> std::sync::MutexGuard<'_, Option<Pending>> {
        match self.pending.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                self.pending.clear_poison();
                poisoned.into_inner()
            }
        }
    }
}

impl Default for ClipboardKeeper {
    fn default() -> Self {
        Self::system()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use cairn_platform::clipboard::{ClipboardError, Fingerprint, fingerprint_of};

    use super::{Arc, Clipboard, ClipboardKeeper};

    /// A clipboard in a variable, which is what the real one is from this side.
    #[derive(Debug, Default)]
    struct Fake {
        held: Mutex<Option<String>>,
        refuse: Mutex<Option<ClipboardError>>,
    }

    impl Fake {
        fn holding(&self) -> Option<String> {
            self.held.lock().expect("no test panics here").clone()
        }

        fn put(&self, text: &str) {
            *self.held.lock().expect("no test panics here") = Some(text.to_owned());
        }

        fn refusing(&self, error: ClipboardError) {
            *self.refuse.lock().expect("no test panics here") = Some(error);
        }

        fn refusal(&self) -> Option<ClipboardError> {
            *self.refuse.lock().expect("no test panics here")
        }
    }

    impl Clipboard for Fake {
        fn write(&self, text: &str) -> Result<(), ClipboardError> {
            if let Some(error) = self.refusal() {
                return Err(error);
            }
            self.put(text);
            Ok(())
        }

        fn clear_if_matches(&self, expected: &Fingerprint) -> Result<bool, ClipboardError> {
            if let Some(error) = self.refusal() {
                return Err(error);
            }
            let mut held = self.held.lock().expect("no test panics here");
            match held.as_deref() {
                Some(text) if &fingerprint_of(text) == expected => {
                    *held = None;
                    Ok(true)
                }
                _somebody_elses => Ok(false),
            }
        }

        fn fingerprint(&self) -> Result<Option<Fingerprint>, ClipboardError> {
            if let Some(error) = self.refusal() {
                return Err(error);
            }
            Ok(self.holding().as_deref().map(fingerprint_of))
        }
    }

    /// Waits for the armed wipe to have happened, or gives up.
    ///
    /// Polling rather than sleeping for a fixed stretch: the point is what the clipboard holds,
    /// not how long the runtime took to get there, and a fixed sleep long enough to be reliable
    /// on a loaded build agent is a fixed sleep nobody wants in a test suite.
    fn settles(clipboard: &Fake, wanted: Option<&str>) -> bool {
        for _attempt in 0..200 {
            if clipboard.holding().as_deref() == wanted {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        false
    }

    #[test]
    fn what_is_copied_is_written_and_taken_back_when_the_time_is_up() {
        let clipboard = Arc::new(Fake::default());
        let keeper = ClipboardKeeper::with(Arc::clone(&clipboard) as Arc<dyn Clipboard>);

        keeper
            .copy("el secreto", Some(Duration::from_millis(20)))
            .expect("the clipboard accepts it");
        assert_eq!(clipboard.holding().as_deref(), Some("el secreto"));

        assert!(
            settles(&clipboard, None),
            "the timer never took the secret back"
        );
    }

    #[test]
    fn what_somebody_else_copied_in_the_meantime_is_left_alone() {
        let clipboard = Arc::new(Fake::default());
        let keeper = ClipboardKeeper::with(Arc::clone(&clipboard) as Arc<dyn Clipboard>);

        keeper
            .copy("el secreto", Some(Duration::from_millis(20)))
            .expect("the clipboard accepts it");
        clipboard.put("la lista de la compra");

        assert!(
            !settles(&clipboard, None),
            "the timer wiped what somebody else had copied"
        );
        assert_eq!(
            clipboard.holding().as_deref(),
            Some("la lista de la compra")
        );
    }

    #[test]
    fn copying_something_else_calls_off_the_timer_that_was_waiting() {
        let clipboard = Arc::new(Fake::default());
        let keeper = ClipboardKeeper::with(Arc::clone(&clipboard) as Arc<dyn Clipboard>);

        keeper
            .copy("el primero", Some(Duration::from_millis(20)))
            .expect("the clipboard accepts it");
        keeper
            .copy("el segundo", Some(Duration::from_millis(2_000)))
            .expect("the clipboard accepts it");

        // Long enough for the first timer to have fired, had it survived.
        std::thread::sleep(Duration::from_millis(200));

        assert_eq!(
            clipboard.holding().as_deref(),
            Some("el segundo"),
            "the timer armed for the first secret wiped the second one"
        );
        assert!(keeper.is_armed(), "the second wipe was not armed");
    }

    #[test]
    fn what_is_not_wiped_arms_nothing() {
        let clipboard = Arc::new(Fake::default());
        let keeper = ClipboardKeeper::with(Arc::clone(&clipboard) as Arc<dyn Clipboard>);

        keeper
            .copy("alguien@ejemplo", None)
            .expect("the clipboard accepts it");

        assert!(!keeper.is_armed());
        assert_eq!(clipboard.holding().as_deref(), Some("alguien@ejemplo"));
    }

    #[test]
    fn wiping_now_takes_it_back_at_once_and_leaves_somebody_elses_alone() {
        let clipboard = Arc::new(Fake::default());
        let keeper = ClipboardKeeper::with(Arc::clone(&clipboard) as Arc<dyn Clipboard>);

        keeper
            .copy("el secreto", Some(Duration::from_secs(60)))
            .expect("the clipboard accepts it");

        assert!(keeper.wipe_now());
        assert_eq!(clipboard.holding(), None);
        assert!(!keeper.is_armed());

        keeper
            .copy("otro secreto", Some(Duration::from_secs(60)))
            .expect("the clipboard accepts it");
        clipboard.put("la lista de la compra");

        assert!(!keeper.wipe_now());
        assert_eq!(
            clipboard.holding().as_deref(),
            Some("la lista de la compra")
        );
    }

    #[test]
    fn wiping_when_nothing_was_copied_does_nothing_and_says_so() {
        let clipboard = Arc::new(Fake::default());
        let keeper = ClipboardKeeper::with(Arc::clone(&clipboard) as Arc<dyn Clipboard>);

        assert!(!keeper.wipe_now());
    }

    #[test]
    fn a_clipboard_that_refuses_arms_nothing() {
        let clipboard = Arc::new(Fake::default());
        clipboard.refusing(ClipboardError::Busy);
        let keeper = ClipboardKeeper::with(Arc::clone(&clipboard) as Arc<dyn Clipboard>);

        assert_eq!(
            keeper.copy("el secreto", Some(Duration::from_secs(60))),
            Err(ClipboardError::Busy)
        );
        assert!(
            !keeper.is_armed(),
            "a copy that did not happen left a wipe waiting for it"
        );
        assert_eq!(clipboard.holding(), None);
    }
}
