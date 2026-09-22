//! The system clipboard, written from here so that a copied secret never goes back to the WebView.
//!
//! Copying a password is the one thing somebody does with a password manager more often than
//! anything else, and the obvious way to build it — hand the value to the interface and let it
//! call the browser's clipboard API — puts the secret back in JavaScript for no reason at all. So
//! the value is written here, in the core, and the command that asks for it answers with nothing.
//!
//! **What this module does not do yet.** It does not mark the contents with
//! `ExcludeClipboardContentFromMonitorProcessing` or `CanIncludeInClipboardHistoryAndRoaming`,
//! which is what would keep a copied password out of the Windows clipboard history. That is a
//! later phase, and until then every password copied lands in that history in the clear and
//! survives the vault being locked. The interface says so beside the button rather than letting
//! anybody assume otherwise, and the flag it draws that warning from lives with the commands.
//!
//! **Why the raw calls.** There are safe wrappers for the clipboard on crates.io and none of them
//! exposes the two raw formats above, so a wrapper would have to be torn out again in the phase
//! that needs them. Three calls, in the one crate where `unsafe` is allowed, is the smaller cost.
//!
//! **Why a digest rather than the text.** Taking a secret back off the clipboard means checking
//! that what is on it is still the thing that was put there, because somebody may have copied
//! something else in the meantime and wiping that would be deleting their work. Checking with a
//! digest means a caller that wants to know never has to hold either value: not the secret it
//! wrote, and not whatever the clipboard holds now, which may be somebody else's.

use sha2::{Digest as _, Sha256};

/// Why the clipboard did not do what was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ClipboardError {
    /// Another process has it open. Ordinary, frequent, and not a failure of this program.
    #[error("the clipboard is busy")]
    Busy,

    /// There is no clipboard here: a service, a session with no desktop, an unsupported target.
    #[error("there is no clipboard available")]
    Unavailable,

    /// The system refused for a reason worth keeping.
    #[error("the clipboard refused: {code}")]
    Refused {
        /// The system error number, for a log on this side of the bridge.
        code: u32,
    },
}

/// The digest this module compares by, and the one a caller stores while it waits.
pub type Fingerprint = [u8; 32];

/// The digest of a piece of text, as this module would fingerprint it once it is on the clipboard.
///
/// Public so that a caller can work out what it is about to write without writing it first, and
/// so that the thing it compares against later is produced by the same code either way.
#[must_use]
pub fn fingerprint_of(text: &str) -> Fingerprint {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher.finalize().into()
}

#[cfg(windows)]
pub use windows_clipboard::{clear_if_matches, fingerprint, write};

#[cfg(not(windows))]
pub use elsewhere::{clear_if_matches, fingerprint, write};

/// The three functions on a machine that has no clipboard this program can reach.
///
/// Not a silent success and not a panic. A build for a platform this module has not been written
/// for has to compile, has to be callable, and has to say plainly that nothing was copied, so
/// that the interface can report it rather than showing somebody a tick over nothing.
#[cfg(not(windows))]
mod elsewhere {
    use super::{ClipboardError, Fingerprint};

    /// Always [`ClipboardError::Unavailable`] here.
    ///
    /// # Errors
    ///
    /// Always.
    pub fn write(_text: &str) -> Result<(), ClipboardError> {
        Err(ClipboardError::Unavailable)
    }

    /// Always [`ClipboardError::Unavailable`] here.
    ///
    /// # Errors
    ///
    /// Always.
    pub fn fingerprint() -> Result<Option<Fingerprint>, ClipboardError> {
        Err(ClipboardError::Unavailable)
    }

    /// Always [`ClipboardError::Unavailable`] here.
    ///
    /// # Errors
    ///
    /// Always.
    pub fn clear_if_matches(_expected: &Fingerprint) -> Result<bool, ClipboardError> {
        Err(ClipboardError::Unavailable)
    }
}

#[cfg(windows)]
mod windows_clipboard {
    use windows_sys::Win32::Foundation::{GetLastError, HANDLE, HWND};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
        OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    use super::{ClipboardError, Fingerprint, fingerprint_of};

    /// The error Windows reports when another process holds the clipboard open.
    const ERROR_ACCESS_DENIED: u32 = 5;

    /// Holds the clipboard open, and closes it however the caller leaves.
    ///
    /// The whole reason this is a value with a destructor rather than a call at the end of each
    /// function: a `?` between opening and closing would leave the clipboard open for every other
    /// process on the machine until this one exits.
    struct Open;

    impl Open {
        /// Opens the clipboard, once, with one retry.
        ///
        /// One retry and no loop. Another process holding it is ordinary — every window that
        /// watches the clipboard opens it for an instant — and spinning on it would turn somebody
        /// else's paste into this program's busy wait.
        #[allow(
            unsafe_code,
            reason = "opens and closes the system clipboard; each call carries its own SAFETY comment and none of them touches memory this program owns"
        )]
        fn take() -> Result<Self, ClipboardError> {
            for attempt in 0..2 {
                // SAFETY: `OpenClipboard` takes a window handle and null means "this task". It
                // reads none of our memory and allocates nothing on our behalf.
                let opened =
                    unsafe { OpenClipboard(core::ptr::null_mut::<core::ffi::c_void>() as HWND) };
                if opened != 0 {
                    return Ok(Self);
                }

                // SAFETY: reads the calling thread's last error code. Takes no arguments and
                // touches none of our memory.
                let code = unsafe { GetLastError() };
                if code != ERROR_ACCESS_DENIED {
                    return Err(ClipboardError::Refused { code });
                }
                if attempt == 1 {
                    return Err(ClipboardError::Busy);
                }

                // Long enough for the window that is watching the clipboard to let go, short
                // enough that nobody notices. Not a loop: one more try, and then the truth.
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            Err(ClipboardError::Busy)
        }
    }

    impl Drop for Open {
        #[allow(
            unsafe_code,
            reason = "closes the clipboard this value opened; the call takes no arguments and touches none of our memory"
        )]
        fn drop(&mut self) {
            // SAFETY: closes the clipboard this value opened. It takes no arguments, reads none
            // of our memory, and the only way to hold this value is to have opened it.
            let _closed = unsafe { CloseClipboard() };
        }
    }

    /// Puts text on the clipboard, replacing what was there.
    ///
    /// # Errors
    ///
    /// [`ClipboardError`] in each of its three shapes.
    #[allow(
        unsafe_code,
        reason = "hands a block of system memory to the clipboard; every call carries its own SAFETY comment stating who owns what afterwards"
    )]
    pub fn write(text: &str) -> Result<(), ClipboardError> {
        // Built before the clipboard is opened, so that nothing is held open while this program
        // allocates and converts.
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(0);
        let bytes = wide.len().saturating_mul(core::mem::size_of::<u16>());

        let _open = Open::take()?;

        // SAFETY: empties the clipboard this thread has open. It takes no arguments and reads
        // none of our memory. It must be called before `SetClipboardData` or the new handle is
        // refused.
        let emptied = unsafe { EmptyClipboard() };
        if emptied == 0 {
            // SAFETY: as above, reads the calling thread's last error code.
            return Err(ClipboardError::Refused {
                code: unsafe { GetLastError() },
            });
        }

        // SAFETY: allocates `bytes` of movable system memory and reads nothing of ours. A null
        // answer means the allocation failed, which is checked on the next line.
        let block = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
        if block.is_null() {
            // SAFETY: as above.
            return Err(ClipboardError::Refused {
                code: unsafe { GetLastError() },
            });
        }

        // SAFETY: `block` is the handle just returned by `GlobalAlloc` and has not been locked,
        // freed or handed anywhere. Locking it yields a pointer to `bytes` writable bytes.
        let destination = unsafe { GlobalLock(block) };
        if destination.is_null() {
            // SAFETY: `block` is still ours: nothing has been handed to the clipboard yet, so
            // freeing it here is freeing our own failed allocation.
            let _freed = unsafe { windows_sys::Win32::Foundation::GlobalFree(block) };
            // SAFETY: as above.
            return Err(ClipboardError::Refused {
                code: unsafe { GetLastError() },
            });
        }

        // SAFETY: `destination` points at `bytes` writable bytes just locked, `wide` holds
        // exactly `bytes` bytes of initialised `u16`, and the two regions cannot overlap because
        // one is a fresh system allocation and the other a Rust `Vec`.
        unsafe {
            core::ptr::copy_nonoverlapping(
                wide.as_ptr().cast::<u8>(),
                destination.cast::<u8>(),
                bytes,
            );
        }

        // SAFETY: unlocks the block locked just above, exactly once. After this the pointer must
        // not be used again, and it is not.
        let _unlocked = unsafe { GlobalUnlock(block) };

        // SAFETY: hands the block to the clipboard in the format its contents are in. **The
        // system owns the block from here on**: it must not be freed by this program, and the
        // handle must not be used again. On failure it is still ours, which is what the branch
        // below relies on.
        let given = unsafe { SetClipboardData(u32::from(CF_UNICODETEXT), block as HANDLE) };
        if given.is_null() {
            // SAFETY: the clipboard refused the handle, so ownership never passed and this is
            // freeing our own allocation.
            let _freed = unsafe { windows_sys::Win32::Foundation::GlobalFree(block) };
            // SAFETY: as above.
            return Err(ClipboardError::Refused {
                code: unsafe { GetLastError() },
            });
        }

        Ok(())
    }

    /// The digest of what is on the clipboard now, or `None` if it holds no text.
    ///
    /// # Errors
    ///
    /// [`ClipboardError`] in each of its three shapes.
    #[allow(
        unsafe_code,
        reason = "reads a block of system memory the clipboard owns; every call carries its own SAFETY comment stating that it is never freed here"
    )]
    pub fn fingerprint() -> Result<Option<Fingerprint>, ClipboardError> {
        let _open = Open::take()?;

        // SAFETY: asks whether a format is present. It takes an integer and touches none of our
        // memory.
        let has_text = unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT)) };
        if has_text == 0 {
            // An image, a file list, or nothing at all. Not an error: the question was whether
            // the clipboard holds text, and the answer is no.
            return Ok(None);
        }

        // SAFETY: asks the clipboard for its handle in that format. **The block belongs to the
        // system**: it is never freed here, and it stays valid only while the clipboard is open,
        // which it is for the whole of this function.
        let block = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT)) };
        if block.is_null() {
            return Ok(None);
        }

        // SAFETY: `block` is the handle the clipboard just gave us and the clipboard is open, so
        // locking it is valid and yields a pointer to a null terminated UTF-16 string.
        let source = unsafe { GlobalLock(block.cast()) }.cast::<u16>();
        if source.is_null() {
            // SAFETY: as elsewhere, reads the calling thread's last error code.
            return Err(ClipboardError::Refused {
                code: unsafe { GetLastError() },
            });
        }

        let text = read_wide(source);

        // SAFETY: unlocks the block locked just above, exactly once, and does not free it: the
        // system owns it.
        let _unlocked = unsafe { GlobalUnlock(block.cast()) };

        Ok(Some(fingerprint_of(&text)))
    }

    /// Empties the clipboard, but only if what is on it still digests to `expected`.
    ///
    /// Answers whether it emptied it. `false` means somebody copied something else in the
    /// meantime and it was left alone.
    ///
    /// # Errors
    ///
    /// [`ClipboardError`] in each of its three shapes. **A read that fails empties nothing**:
    /// wiping what could not be checked would be deleting what the person copied.
    #[allow(
        unsafe_code,
        reason = "empties the clipboard after checking its contents; each call carries its own SAFETY comment"
    )]
    pub fn clear_if_matches(expected: &Fingerprint) -> Result<bool, ClipboardError> {
        // Two openings rather than one. The alternative is a single function holding the
        // clipboard open across the read, the comparison and the wipe, which is a few
        // microseconds longer and would need the reading half written twice. What can happen in
        // between is the same thing that can happen at any other moment: somebody copies
        // something else, and then the digest no longer matches and nothing is wiped.
        let Some(found) = fingerprint()? else {
            return Ok(false);
        };
        if &found != expected {
            return Ok(false);
        }

        let _open = Open::take()?;

        // SAFETY: empties the clipboard this thread has open. It takes no arguments and reads
        // none of our memory.
        let emptied = unsafe { EmptyClipboard() };
        if emptied == 0 {
            // SAFETY: as above, reads the calling thread's last error code.
            return Err(ClipboardError::Refused {
                code: unsafe { GetLastError() },
            });
        }

        Ok(true)
    }

    /// Reads a null terminated UTF-16 string into an owned one, with a ceiling.
    ///
    /// The ceiling is what makes this defensible at all. A pointer with no length is only as
    /// bounded as the terminator somebody else wrote, and the clipboard is written by every other
    /// program on the machine; sixteen mebibytes of text is far more than anything worth copying
    /// and far less than a walk off the end of memory.
    #[allow(
        unsafe_code,
        reason = "walks a null terminated string the system handed over; the SAFETY comment states the bound"
    )]
    fn read_wide(source: *const u16) -> String {
        /// The most text this reads before stopping, in UTF-16 code units.
        const CEILING: usize = 8 * 1024 * 1024;

        let mut units = Vec::new();
        for at in 0..CEILING {
            // SAFETY: `source` points at a null terminated UTF-16 string inside a block the
            // clipboard has locked for us, and the loop stops at the terminator or at the
            // ceiling, so the offset stays inside the block in either case.
            let unit = unsafe { *source.add(at) };
            if unit == 0 {
                break;
            }
            units.push(unit);
        }

        String::from_utf16_lossy(&units)
    }
}

#[cfg(test)]
mod tests {
    use super::fingerprint_of;

    #[test]
    fn the_same_text_always_fingerprints_the_same_and_different_text_does_not() {
        assert_eq!(
            fingerprint_of("una contraseña"),
            fingerprint_of("una contraseña")
        );
        assert_ne!(fingerprint_of("una contraseña"), fingerprint_of("otra"));
    }

    #[test]
    fn an_empty_string_has_a_fingerprint_like_anything_else() {
        // It is a value, not the absence of one, and the caller has to be able to tell the two
        // apart: an empty clipboard answers `None` and an empty string answers a digest.
        assert_ne!(fingerprint_of(""), fingerprint_of(" "));
    }

    #[cfg(not(windows))]
    #[test]
    fn all_three_say_there_is_no_clipboard_here() {
        use super::{ClipboardError, clear_if_matches, fingerprint, write};

        assert_eq!(write("lo que sea"), Err(ClipboardError::Unavailable));
        assert_eq!(fingerprint(), Err(ClipboardError::Unavailable));
        assert_eq!(
            clear_if_matches(&fingerprint_of("lo que sea")),
            Err(ClipboardError::Unavailable)
        );
    }

    /// The tests that need a real clipboard, which is a shared resource of the whole desktop.
    ///
    /// Ignored by default and run by name. They replace whatever the person running them had
    /// copied, which is rude enough on a development machine and impossible on a build agent
    /// with no desktop at all.
    #[cfg(windows)]
    mod with_a_real_clipboard {
        use super::super::{clear_if_matches, fingerprint, fingerprint_of, write};

        #[test]
        #[ignore = "uses the real clipboard of the desktop running the tests"]
        fn what_is_written_is_what_is_fingerprinted() {
            write("una frase con acentos y un emoji 🪨").expect("the clipboard accepts text");

            assert_eq!(
                fingerprint().expect("the clipboard can be read"),
                Some(fingerprint_of("una frase con acentos y un emoji 🪨")),
                "the conversion to UTF-16 and back does not round trip"
            );
        }

        #[test]
        #[ignore = "uses the real clipboard of the desktop running the tests"]
        fn empty_text_is_written_and_read_without_panicking() {
            write("").expect("the clipboard accepts empty text");

            assert_eq!(
                fingerprint().expect("the clipboard can be read"),
                Some(fingerprint_of(""))
            );
        }

        #[test]
        #[ignore = "uses the real clipboard of the desktop running the tests"]
        fn what_was_written_is_taken_back_and_what_replaced_it_is_left_alone() {
            let ours = fingerprint_of("el secreto");
            write("el secreto").expect("the clipboard accepts text");

            assert!(clear_if_matches(&ours).expect("the clipboard can be read"));
            assert_eq!(
                fingerprint().expect("the clipboard can be read"),
                None,
                "the clipboard was not emptied"
            );

            // And now the other half: somebody copies something else before the timer wakes up.
            write("el secreto").expect("the clipboard accepts text");
            write("la lista de la compra").expect("somebody else copies something");

            assert!(!clear_if_matches(&ours).expect("the clipboard can be read"));
            assert_eq!(
                fingerprint().expect("the clipboard can be read"),
                Some(fingerprint_of("la lista de la compra")),
                "what somebody else copied was wiped"
            );
        }
    }
}
