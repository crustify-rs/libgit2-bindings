//! Safe wrappers over `include/git2/tag.h`.
//!
//! [`name_is_valid`] checks a tag name the way libgit2 checks the names it creates tags
//! under: the name must not start with `-` nor be `HEAD`, and `refs/tags/<name>` must be
//! a valid reference name. libgit2 builds that reference name in its allocator, so the
//! check pins libgit2's runtime first (initialising libgit2 on the first call), as every
//! wrapper that reaches libgit2's allocator or error state does.

use core::ffi::{CStr, c_int};

use crate::ffi;
use crate::libgit2::util::errors::{ErrorBuf, last};
use crate::libgit2::util::sys::errors::runtime_pinned;

/// Whether `name` is a valid tag name: it does not start with `-`, is not `HEAD`, and,
/// prefixed with `refs/tags/`, is a valid reference name (no empty, `.`-led or
/// `.lock`-ended component, no `..`, `@{`, control character, space or any of
/// `~^:?*[\`, and no trailing `.` or `/`).
///
/// The name is read for the call only. libgit2 reports an invalid name as `Ok(false)`,
/// but leaves a `GIT_ERROR_REFERENCE` error on the calling thread when the reference
/// name is what fails ("the given reference name '…' is not valid"); the wrapper keeps
/// that.
///
/// Fails, with libgit2's error, only when libgit2 runs out of memory building the
/// reference name, or cannot be initialised.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::object::tag::name_is_valid;
///
/// assert_eq!(name_is_valid(c"v1.0"), Ok(true));
/// assert_eq!(name_is_valid(c"release/2024"), Ok(true));
/// assert_eq!(name_is_valid(c"-v1.0"), Ok(false));
/// assert_eq!(name_is_valid(c"HEAD"), Ok(false));
/// assert_eq!(name_is_valid(c"v1..0"), Ok(false));
/// ```
///
/// Wraps: git_tag_name_is_valid
pub fn name_is_valid(name: &CStr) -> Result<bool, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut valid: c_int = 0;
    // SAFETY: `valid` is a writable `int`, which libgit2 writes before returning; `name`
    // is a non-NULL NUL-terminated string, read and copied for the call, never retained.
    // The runtime is pinned, so the allocation reads a settled allocator and a failure is
    // reported on live thread-local state.
    let rc = unsafe { ffi::git_tag_name_is_valid(&mut valid, name.as_ptr()) };
    if rc < 0 {
        return Err(last());
    }
    Ok(valid != 0)
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::util::errors::ErrorClass;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use crate::libgit2::util::sys::errors::set;
    use core::ffi::c_void;
    use core::ptr::NonNull;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::SeqCst};

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process, where nothing has initialised libgit2.
    fn isolated(test: &str, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    static REFUSING: AtomicBool = AtomicBool::new(false);
    static ALLOWED: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// Whether the next request is refused: while `REFUSING` is set, after `ALLOWED`
    /// more requests.
    fn refuse() -> bool {
        REFUSING.load(SeqCst) && ALLOWED.fetch_sub(1, SeqCst) <= 0
    }

    /// libc's allocator, refusing requests per [`refuse`].
    struct Countdown;

    // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
    unsafe impl Alloc for Countdown {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            if refuse() {
                return None;
            }
            // SAFETY: plain libc allocation.
            NonNull::new(unsafe { malloc(len) }.cast())
        }
        unsafe fn realloc(
            p: Option<NonNull<u8>>,
            len: usize,
            _: Option<&CStr>,
            _: i32,
        ) -> Option<NonNull<u8>> {
            if refuse() {
                return None;
            }
            let p = p.map_or(core::ptr::null_mut(), |p| p.as_ptr().cast());
            // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
            NonNull::new(unsafe { realloc(p, len) }.cast())
        }
        unsafe fn free(p: NonNull<u8>) {
            // SAFETY: `p` is a live libc block the caller gives up.
            unsafe { free(p.as_ptr().cast()) }
        }
    }

    /// Installs [`Countdown`] as libgit2's allocator, before anything is allocated.
    fn install_countdown() {
        let allocator = Allocator::new::<Countdown>();
        // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with every
        // member set; libgit2 copies it before anything is allocated.
        let rc = unsafe {
            ffi::git_libgit2_opts(
                ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
    }

    /// A refused allocation of the reference name is libgit2's out-of-memory error, not
    /// an invalid name; with the allocation granted the same name is valid.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn out_of_memory_is_an_error() {
        isolated(
            concat!(module_path!(), "::out_of_memory_is_an_error"),
            || {
                install_countdown();
                // The thread's error state, allocated now, so a refusal reports out of
                // memory rather than a thread state it could not allocate.
                set(ErrorClass::Invalid, c"error state allocated");
                ALLOWED.store(0, SeqCst);
                REFUSING.store(true, SeqCst);
                let refused = name_is_valid(c"v1.0");
                REFUSING.store(false, SeqCst);
                let error = refused.unwrap_err();
                assert_eq!(
                    (error.class(), error.message()),
                    (Some(ErrorClass::NoMemory), c"Out of memory")
                );
                assert_eq!(name_is_valid(c"v1.0"), Ok(true));
            },
        );
    }

    /// When libgit2 cannot be initialised, the check reports it without calling
    /// `git_tag_name_is_valid`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn uninitialisable_library_is_reported() {
        isolated(
            concat!(module_path!(), "::uninitialisable_library_is_reported"),
            || {
                install_countdown();
                ALLOWED.store(0, SeqCst);
                REFUSING.store(true, SeqCst);
                let error = name_is_valid(c"v1.0").unwrap_err();
                assert_eq!(
                    (error.class(), error.message()),
                    (
                        Some(ErrorClass::Invalid),
                        c"library has not been initialized"
                    )
                );
            },
        );
    }
}
