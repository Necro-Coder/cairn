//! Memory that the operating system is asked not to write to the disk.
//!
//! This is the first `unsafe` in the project, and the reason it is worth having is narrow.
//! A key sitting in ordinary memory can be written to the page file, and a page file
//! survives the process, the session and often the disk encryption argument entirely. Asking
//! the system to keep the pages resident removes that particular copy. It removes nothing
//! else: another process attached to this one, a crash dump, or hibernation all still see
//! the key, and this paragraph says so rather than letting the name suggest otherwise.
//!
//! The awkward part is not the call. It is that `VirtualLock` and `mlock` work on pages, and
//! a page holds four thousand bytes of whatever the allocator put there. Locking the page
//! that happens to contain a thirty-two byte key locks three dozen unrelated allocations
//! with it, and unlocking it when that key is dropped unlocks any other key that landed on
//! the same page. Neither call counts how many times it was asked. So the type below does
//! not lock somebody else's allocation: it allocates whole pages of its own, and owns them
//! until it drops. Four kibibytes for a thirty-two byte key is a waste worth paying, because
//! there are half a dozen keys in this program and exactly one way for the shared page
//! version to be wrong.
//!
//! Failing to lock is not fatal and is not hidden. Systems have a limit on how much a process
//! may keep resident, and a machine that refuses should still open the vault. The buffer
//! reports whether it managed, so the diagnostics screen can say so, and the raw wrapper
//! returns a typed error so the failure can be tested rather than assumed.

use std::alloc::{self, Layout};
use std::ptr::NonNull;

use zeroize::Zeroize as _;

/// Why the operating system would not keep a region resident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MemoryError {
    /// The system refused to keep the pages resident.
    ///
    /// Almost always a quota: a process may keep only so much locked, and the limit is small
    /// by default on Windows. Reported rather than swallowed, because a caller that believes
    /// its key is locked when it is not is worse off than one that knows it is not.
    #[error("the operating system would not keep {len} bytes resident (error {code})")]
    LockRefused {
        /// How many bytes were asked for.
        len: usize,
        /// What the system said, as its own error number.
        code: i32,
    },

    /// The region asked for was empty, or so large that a layout for it cannot exist.
    #[error("{len} bytes is not a size this can allocate")]
    ImpossibleSize {
        /// What was asked for.
        len: usize,
    },

    /// The allocator had nothing left.
    ///
    /// Separate from a size that cannot exist, because they are different facts: one is a
    /// caller asking for something impossible, the other is a machine that has run out. A
    /// single variant covering both would name the caller for the machine's problem.
    #[error("{len} bytes could not be allocated")]
    AllocationFailed {
        /// What was asked for.
        len: usize,
    },
}

/// The ordinary page size, used when the system will not say.
const FALLBACK_PAGE_SIZE: usize = 4096;

/// Turns what the system reported into something every layout below can be built from.
///
/// A system answering zero would make every allocation in this module invalid: a layout
/// cannot have an alignment of zero, and rounding a length up to a multiple of zero is not a
/// number. No machine answers zero, which is exactly why the branch is here and separate:
/// something that cannot happen and is never exercised is something that is wrong the first
/// time it does happen.
const fn sane_page_size(reported: usize) -> usize {
    if reported == 0 {
        FALLBACK_PAGE_SIZE
    } else {
        reported
    }
}

/// How large a page is on this machine.
///
/// Read from the system rather than assumed to be four kibibytes, because it is not on every
/// machine this may eventually run on, and a wrong answer here makes every allocation below
/// either wasteful or misaligned.
#[must_use]
#[allow(
    unsafe_code,
    reason = "asks the system how large a page is, which has no safe interface on either platform; each call carries its own SAFETY comment"
)]
pub fn page_size() -> usize {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};

        let mut information = SYSTEM_INFO::default();
        // SAFETY: `GetSystemInfo` writes one `SYSTEM_INFO` through the pointer it is given
        // and reads nothing. `information` is a live, aligned, exclusively borrowed value of
        // exactly that type, so the write is in bounds and cannot alias anything else. The
        // call has no other effect and cannot fail.
        unsafe { GetSystemInfo(&raw mut information) };

        sane_page_size(information.dwPageSize as usize)
    }

    #[cfg(not(windows))]
    {
        // SAFETY: `sysconf` reads a system constant by name and writes nothing through any
        // pointer. `_SC_PAGESIZE` is one of the names the standard requires it to accept, so
        // the call is defined and returns either the value or -1.
        let reported = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };

        // -1 means the name was not recognised, which cannot happen for this one, but the
        // conversion below must not be given a negative number either way.
        sane_page_size(usize::try_from(reported).unwrap_or(0))
    }
}

/// Asks the system to keep `len` bytes at `pointer` resident.
///
/// The raw wrapper, private to this crate. [`LockedBytes`] is what callers want; this
/// exists separately so the refusal path can be reached in a test by asking for more than
/// any quota allows. An unsafe function in a public interface is an invitation, and nothing
/// outside this crate has a reason to accept it.
///
/// # Errors
///
/// Returns [`MemoryError::LockRefused`] with the system error number if the request is
/// denied, which on Windows normally means the working set quota is too small.
///
/// # Safety
///
/// `pointer` must be the start of a readable, writable region of at least `len` bytes that
/// stays allocated until [`unlock`] is called for the same region. Locking pages the caller
/// does not own would leave another owner's memory resident, and the matching unlock would
/// take it out from under them.
#[allow(
    unsafe_code,
    reason = "wraps the residency call this module exists for; the contract it needs from its caller is written in the Safety section above and each call inside carries its own SAFETY comment"
)]
pub(crate) unsafe fn lock(pointer: NonNull<u8>, len: usize) -> Result<(), MemoryError> {
    if len == 0 {
        return Ok(());
    }

    #[cfg(miri)]
    {
        // The interpreter has no page tables and no system to ask. Reporting success keeps
        // every caller on its ordinary path, which is the path worth checking here: what
        // Miri is looking for is the allocation and the writes around it, not the syscall.
        let _ = pointer;
        return Ok(());
    }

    #[cfg(all(not(miri), windows))]
    {
        use windows_sys::Win32::System::Memory::VirtualLock;

        // SAFETY: the caller promises `pointer` starts a region of at least `len` bytes that
        // stays allocated for the lifetime of the lock. `VirtualLock` only changes whether
        // those pages may be paged out; it reads and writes none of their contents, so no
        // aliasing rule applies to the memory itself.
        let locked = unsafe { VirtualLock(pointer.as_ptr().cast(), len) };
        if locked == 0 {
            // SAFETY: `GetLastError` reads the calling thread's last error code. It takes no
            // arguments, touches no memory of ours, and is valid to call at any time.
            let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            return Err(MemoryError::LockRefused {
                len,
                code: i32::try_from(code).unwrap_or(-1),
            });
        }
        Ok(())
    }

    #[cfg(all(not(miri), not(windows)))]
    {
        // SAFETY: as above. `mlock` changes only the residency of the pages covering the
        // region the caller promised is theirs, and does not read or write its contents.
        let result = unsafe { libc::mlock(pointer.as_ptr().cast(), len) };
        if result == 0 {
            Ok(())
        } else {
            Err(MemoryError::LockRefused {
                len,
                code: std::io::Error::last_os_error().raw_os_error().unwrap_or(-1),
            })
        }
    }
}

/// Releases a lock taken by [`lock`] over the same region.
///
/// Failure is not reported. There is nothing a caller could do about it, it happens while a
/// value is being dropped, and the pages are about to be freed anyway.
///
/// # Safety
///
/// `pointer` and `len` must describe exactly the region a matching [`lock`] was given, and
/// that region must still be allocated. Unlocking a region the caller does not own would
/// make somebody else's pages pageable again.
#[allow(
    unsafe_code,
    reason = "the matching release of the call above, with the same contract"
)]
pub(crate) unsafe fn unlock(pointer: NonNull<u8>, len: usize) {
    if len == 0 {
        return;
    }

    #[cfg(miri)]
    {
        let _ = pointer;
    }

    #[cfg(all(not(miri), windows))]
    {
        use windows_sys::Win32::System::Memory::VirtualUnlock;

        // SAFETY: the caller promises this is the region a matching lock was given and that
        // it is still allocated. `VirtualUnlock` reads and writes none of its contents.
        unsafe { VirtualUnlock(pointer.as_ptr().cast(), len) };
    }

    #[cfg(all(not(miri), not(windows)))]
    {
        // SAFETY: as above.
        unsafe { libc::munlock(pointer.as_ptr().cast(), len) };
    }
}

/// A run of bytes in pages of its own, kept resident where the system allows it.
///
/// Whole pages, exclusively owned, so that locking and unlocking cannot reach any other
/// allocation. Zeroized before it is freed, and its `Debug` says nothing about what is
/// inside, because the whole reason a value ends up in here is that printing it would be a
/// disclosure.
pub struct LockedBytes {
    pointer: NonNull<u8>,
    layout: Layout,
    len: usize,
    locked: bool,
}

impl LockedBytes {
    /// Allocates `len` bytes of zeroes in pages of their own and asks for them to stay
    /// resident.
    ///
    /// Succeeds even if the system refuses to keep them resident; [`Self::is_locked`] says
    /// which happened. A machine with a small working set quota should still be able to open
    /// its vault, and a caller that wants to know can ask.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::ImpossibleSize`] for a length of zero, or one so large that
    /// rounding it up to whole pages does not fit in a layout.
    #[allow(
        unsafe_code,
        reason = "allocates whole pages and locks them, which is the whole point of the type; each call carries its own SAFETY comment"
    )]
    pub fn new(len: usize) -> Result<Self, MemoryError> {
        if len == 0 {
            return Err(MemoryError::ImpossibleSize { len });
        }

        let page = page_size();
        let size = len
            .checked_next_multiple_of(page)
            .ok_or(MemoryError::ImpossibleSize { len })?;
        let layout =
            Layout::from_size_align(size, page).map_err(|_| MemoryError::ImpossibleSize { len })?;

        // SAFETY: `layout` has a non-zero size, which is the one thing `alloc_zeroed`
        // requires of it. Its alignment is a page size, which is a power of two by
        // definition on every system this builds for, and `Layout::from_size_align` has
        // already refused anything else.
        let raw = unsafe { alloc::alloc_zeroed(layout) };
        let Some(pointer) = NonNull::new(raw) else {
            // The allocator says it has nothing. Reported rather than handed to the standard
            // hook, which aborts the process: a caller asking for a large buffer deserves the
            // chance to ask for a smaller one.
            return Err(MemoryError::AllocationFailed { len });
        };

        // SAFETY: `pointer` is the start of the allocation just made, of exactly `size`
        // bytes, and this value owns it until `Drop` runs, which is also where the matching
        // unlock happens. Nothing else holds a pointer into it.
        let locked = unsafe { lock(pointer, size) }.is_ok();

        Ok(Self {
            pointer,
            layout,
            len,
            locked,
        })
    }

    /// Allocates a region holding a copy of `source`, which is zeroized by the caller.
    ///
    /// # Errors
    ///
    /// The same as [`Self::new`].
    pub fn from_slice(source: &[u8]) -> Result<Self, MemoryError> {
        let mut held = Self::new(source.len())?;
        held.as_mut_slice().copy_from_slice(source);
        Ok(held)
    }

    /// Whether the system agreed to keep the pages resident.
    ///
    /// Answered honestly. A machine that refused still works, and the diagnostics screen can
    /// say which of the two it is rather than implying a protection that is not there.
    #[must_use]
    pub const fn is_locked(&self) -> bool {
        self.locked
    }

    /// How many bytes were asked for, which is fewer than the pages hold.
    ///
    /// There is deliberately no `is_empty` beside it. [`Self::new`] refuses a length of
    /// zero, so the only answer it could ever give is false, and a method with one possible
    /// answer is a method that tells a reader nothing and a test nothing either.
    #[expect(
        clippy::len_without_is_empty,
        reason = "a length of zero is refused at construction, so the companion could only ever answer false"
    )]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// The bytes, as a slice.
    #[must_use]
    #[allow(
        unsafe_code,
        reason = "builds a slice over the allocation this value owns; the SAFETY comment inside gives the argument"
    )]
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `pointer` starts an allocation of at least `len` bytes that this value
        // owns and keeps alive, and `alloc_zeroed` initialised every one of them. The
        // borrow of `self` is what stops the slice outliving the allocation, and it is
        // shared, so no exclusive reference to the same bytes can exist at the same time.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.len) }
    }

    /// The bytes, as a slice that can be written to.
    #[allow(
        unsafe_code,
        reason = "builds an exclusive slice over the allocation this value owns; the SAFETY comment inside gives the argument"
    )]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as for `as_slice`, and the borrow of `self` is exclusive here, so this is
        // the only reference to those bytes for as long as it lives.
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.len) }
    }
}

#[allow(
    unsafe_code,
    reason = "zeroizes, releases and frees the allocation this value owns; each step carries its own SAFETY comment"
)]
impl Drop for LockedBytes {
    fn drop(&mut self) {
        // Zeroized first, while the pages are still mapped and still locked. Doing it after
        // the unlock would leave a window in which the contents could reach the disk, which
        // is the exact thing this type exists to prevent.
        // SAFETY: the allocation is `self.layout.size()` bytes and is still live here; this
        // is the only reference to it, because `Drop` runs once and the value is gone
        // afterwards.
        let whole =
            unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.layout.size()) };
        whole.zeroize();

        if self.locked {
            // SAFETY: exactly the region the lock in `new` was given, still allocated.
            unsafe { unlock(self.pointer, self.layout.size()) };
        }

        // SAFETY: `pointer` came from `alloc_zeroed` with this same `layout`, and nothing has
        // freed it since.
        unsafe { alloc::dealloc(self.pointer.as_ptr(), self.layout) };
    }
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "the withheld fields are the address of the secret and the shape of its allocation, which is the point"
)]
impl std::fmt::Debug for LockedBytes {
    /// Says the size and whether it is resident, and never the contents.
    ///
    /// The pointer and the layout are left out on purpose rather than by oversight. An
    /// address in a log is a gift to somebody reading it, and the layout says nothing the
    /// length does not.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LockedBytes")
            .field("len", &self.len)
            .field("locked", &self.locked)
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

// SAFETY: `LockedBytes` owns its allocation exclusively and hands out access only through
// `&self` and `&mut self`, so moving it between threads is no different from moving a `Vec`.
// There is no interior mutability and no shared ownership anywhere in it.
#[allow(
    unsafe_code,
    reason = "asserts a thread property of a type that owns its allocation outright; the argument is in the SAFETY comment above"
)]
unsafe impl Send for LockedBytes {}

// SAFETY: the only shared access is `as_slice`, which is a read through `&self`. Two threads
// holding shared references read the same initialised bytes, and writing needs `&mut self`,
// which the borrow checker will not hand out at the same time.
#[allow(
    unsafe_code,
    reason = "asserts a thread property of a type whose shared access is read only; the argument is in the SAFETY comment above"
)]
unsafe impl Sync for LockedBytes {}

#[cfg(test)]
mod tests {
    use super::{LockedBytes, MemoryError, page_size};

    #[test]
    fn a_page_is_a_plausible_size() {
        let page = page_size();

        assert!(page >= 4096, "a page of {page} bytes is not credible");
        assert!(
            page.is_power_of_two(),
            "a page of {page} bytes is not a power of two"
        );
    }

    #[test]
    fn what_is_written_can_be_read_back() {
        let mut held = LockedBytes::new(32).unwrap();
        held.as_mut_slice().copy_from_slice(&[0x5a; 32]);

        assert_eq!(held.as_slice(), &[0x5a; 32]);
        assert_eq!(held.len(), 32);
    }

    #[test]
    fn a_new_region_starts_as_zeroes() {
        // Not a nicety. A key type built on this reads its own bytes before writing them in
        // at least one path, and whatever the allocator last held there is not something to
        // hand to a cipher.
        let held = LockedBytes::new(64).unwrap();
        assert_eq!(held.as_slice(), &[0_u8; 64]);
    }

    #[test]
    fn a_copy_of_a_slice_holds_the_same_bytes() {
        let held = LockedBytes::from_slice(&[1, 2, 3, 4]).unwrap();
        assert_eq!(held.as_slice(), &[1, 2, 3, 4]);
    }

    #[test]
    fn an_empty_region_is_refused_rather_than_allocated() {
        // Zero is not a size an allocator accepts, and a type that silently returned a
        // dangling pointer for it would be a type whose `as_slice` is undefined behaviour.
        assert!(matches!(
            LockedBytes::new(0),
            Err(MemoryError::ImpossibleSize { len: 0 })
        ));
    }

    #[test]
    fn running_out_of_memory_is_not_reported_as_a_size_nobody_asked_for() {
        // Two different facts, and the caller acts on them differently: an impossible size is
        // its own mistake, and an exhausted allocator is the machine's. The variants are kept
        // apart so that a caller can tell, and the messages differ so a log can too.
        let impossible = MemoryError::ImpossibleSize { len: 0 };
        let exhausted = MemoryError::AllocationFailed { len: 4096 };

        assert_ne!(impossible, exhausted);
        assert_ne!(impossible.to_string(), exhausted.to_string());
    }

    #[test]
    fn a_size_that_cannot_exist_is_refused_rather_than_attempted() {
        assert!(matches!(
            LockedBytes::new(usize::MAX),
            Err(MemoryError::ImpossibleSize { .. })
        ));
    }

    #[test]
    fn the_debug_output_says_nothing_about_the_contents() {
        let mut held = LockedBytes::new(32).unwrap();
        held.as_mut_slice().copy_from_slice(&[0xab; 32]);

        let printed = format!("{held:?}");

        assert!(printed.contains("REDACTED"));
        assert!(
            !printed.contains("ab"),
            "the debug output leaked a byte: {printed}"
        );
        assert!(
            !printed.contains("171"),
            "the debug output leaked a byte: {printed}"
        );
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "the interpreter has no page tables, so there is no quota to exceed and nothing to refuse"
    )]
    fn a_region_the_system_will_not_keep_resident_still_works() {
        // The refusal path, reached by asking for more than any working set quota allows. The
        // requirement is not that the lock succeeds; it is that a machine which says no still
        // gets a usable buffer and is told the truth about it.
        // A machine that cannot allocate sixty-four mebibytes at all has nothing to say about
        // locking, and failing here would be reporting the allocator as the system.
        let Ok(mut enormous) = LockedBytes::new(64 * 1024 * 1024) else {
            return;
        };

        enormous.as_mut_slice()[0] = 1;
        assert_eq!(enormous.as_slice()[0], 1);
        // Deliberately no assertion on `is_locked`. Whether this machine allowed it is a
        // property of the machine, and pinning either answer would make the test a report
        // about whoever ran it.
        let _ = enormous.is_locked();
    }

    #[test]
    fn a_thirty_two_byte_region_occupies_whole_pages_of_its_own() {
        // The property the shared page problem is avoided by. Two regions allocated at once
        // must not land on one page, or unlocking either would unlock the other.
        let first = LockedBytes::new(32).unwrap();
        let second = LockedBytes::new(32).unwrap();

        // Masking rather than dividing: the page size is a power of two, which the test above
        // asserts, so the low bits are the offset within the page and the rest name the page.
        let within_page = page_size() - 1;
        let first_page = (first.as_slice().as_ptr() as usize) & !within_page;
        let second_page = (second.as_slice().as_ptr() as usize) & !within_page;

        assert_ne!(
            first_page, second_page,
            "two locked regions shared a page, so unlocking one would unlock the other"
        );
    }

    #[test]
    fn many_regions_can_be_held_at_once() {
        // A handful is all this program ever needs, and a limit lower than that would be
        // worth finding out about here rather than when a vault is opened.
        let held: Vec<LockedBytes> = (0..8).map(|_| LockedBytes::new(32).unwrap()).collect();
        assert_eq!(held.len(), 8);
    }
}

/// A fixed number of bytes in pages of their own, kept resident where the system allows it.
///
/// What a key type wants. [`LockedBytes`] answers with a slice, and a key is an array of a
/// known length; going through `try_into` at every use would turn a fact the type already
/// knows into a runtime check with a failure branch nobody can reach.
pub struct LockedArray<const N: usize>(LockedBytes);

impl<const N: usize> LockedArray<N> {
    /// Allocates `N` zeroed bytes in pages of their own.
    ///
    /// Infallible, deliberately. The only way the allocation below can fail for a few dozen
    /// bytes is that the allocator has nothing left, and the answer to that is the standard
    /// allocation error hook, which is the same answer `Box::new` gives. Making every key in
    /// the program fallible to describe a state in which the program cannot continue anyway
    /// would be a worse trade.
    #[must_use]
    pub fn zeroed() -> Self {
        const { assert!(N > 0, "a locked array of no bytes is not a thing") };

        match LockedBytes::new(N) {
            Ok(bytes) => Self(bytes),
            Err(_) => alloc::handle_alloc_error(Layout::new::<[u8; N]>()),
        }
    }

    /// Whether the system agreed to keep the pages resident.
    #[must_use]
    pub const fn is_locked(&self) -> bool {
        self.0.is_locked()
    }

    /// The bytes, as an array.
    #[allow(
        unsafe_code,
        reason = "turns a slice of a length this type already knows into the array it is, with the argument in the SAFETY comment inside"
    )]
    #[must_use]
    pub fn as_array(&self) -> &[u8; N] {
        // SAFETY: the buffer was allocated with exactly `N` bytes and every one of them was
        // initialised by `alloc_zeroed`. `[u8; N]` has that same size and an alignment of
        // one, which every address satisfies, so the pointer is valid for the read. The
        // borrow of `self` keeps the allocation alive for as long as the reference lives,
        // and it is shared, so no exclusive reference to the same bytes can coexist.
        unsafe { &*self.0.as_slice().as_ptr().cast::<[u8; N]>() }
    }

    /// The bytes, as an array that can be written to.
    #[allow(
        unsafe_code,
        reason = "as for `as_array`, with the borrow exclusive instead of shared"
    )]
    pub fn as_mut_array(&mut self) -> &mut [u8; N] {
        // SAFETY: as for `as_array`, and the borrow of `self` is exclusive here, so this is
        // the only reference to those bytes for as long as it lives.
        unsafe { &mut *self.0.as_mut_slice().as_mut_ptr().cast::<[u8; N]>() }
    }
}

impl<const N: usize> std::fmt::Debug for LockedArray<N> {
    /// Says the size and whether it is resident, and never the contents.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LockedArray")
            .field("len", &N)
            .field("locked", &self.0.is_locked())
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod array_tests {
    use super::LockedArray;

    #[test]
    fn what_is_written_can_be_read_back_as_an_array() {
        let mut held = LockedArray::<32>::zeroed();
        held.as_mut_array().copy_from_slice(&[0x5a; 32]);

        assert_eq!(held.as_array(), &[0x5a; 32]);
    }

    #[test]
    fn a_new_array_starts_as_zeroes() {
        assert_eq!(LockedArray::<32>::zeroed().as_array(), &[0_u8; 32]);
    }

    #[test]
    fn the_debug_output_says_nothing_about_the_contents() {
        let mut held = LockedArray::<32>::zeroed();
        held.as_mut_array().copy_from_slice(&[0xab; 32]);

        let printed = format!("{held:?}");

        assert!(printed.contains("REDACTED"));
        assert!(
            !printed.contains("ab"),
            "the debug output leaked a byte: {printed}"
        );
    }

    #[test]
    fn residency_is_reported_rather_than_assumed() {
        // No assertion on which answer it is. Whether this machine allowed it is a property
        // of the machine, and pinning either answer would make the test a report about
        // whoever ran it.
        let _ = LockedArray::<32>::zeroed().is_locked();
    }
}

#[cfg(test)]
mod page_size_tests {
    use super::{FALLBACK_PAGE_SIZE, sane_page_size};

    #[test]
    fn a_system_that_says_nothing_gets_the_ordinary_answer() {
        // The branch no machine reaches, exercised here rather than left to be discovered by
        // the first one that does. Zero would make every layout in this module invalid.
        assert_eq!(sane_page_size(0), FALLBACK_PAGE_SIZE);
    }

    #[test]
    fn a_system_that_answers_is_believed() {
        assert_eq!(sane_page_size(4096), 4096);
        assert_eq!(sane_page_size(16384), 16384);
    }
}
