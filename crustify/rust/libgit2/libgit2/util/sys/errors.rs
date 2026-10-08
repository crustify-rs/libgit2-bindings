//! Safe wrappers over `include/git2/sys/errors.h`: setting and clearing the calling
//! thread's last libgit2 error.
//!
//! libgit2 keeps one last error per thread (`errors.c`'s `error_threadstate`), in
//! thread-local storage under a key that `git_libgit2_init` creates and the last
//! `git_libgit2_shutdown` deletes. Every routine here looks that state up first;
//! without a live key the lookup reads whatever block TLS key 0 holds, and writes into
//! it. Each wrapper therefore pins the runtime first (`runtime_pinned`), and does
//! nothing when libgit2 cannot be initialised. C callers reaching these routines
//! before initialisation corrupt foreign thread-local state instead; the wrappers
//! deliberately diverge from it.
//!
//! The thread's message lives in one buffer that every later error overwrites, whether
//! set here or by any other libgit2 call on the thread, so the message `git_error_last`
//! reports must be copied out before the next one.
//!
//! Once an allocation for a message fails, that buffer stays out of memory for the
//! rest of the thread (`str.c` marks it `git_str__oom`, which `git_str_clear` keeps):
//! the failure leaves `GIT_ERROR_NOMEMORY` as the last error, and every later message
//! on the thread is dropped — [`set`] and [`set_without_message`] keep the previous
//! error, [`set_str`] fails. The wrappers keep libgit2's behaviour.
//!
//! The error class is libgit2's `git_error_t`, taken raw and stored as given, any value
//! included. Raw seam: the safe `git_error_t` wrapper (`util/errors.rs`) replaces it.

use core::ffi::{CStr, c_int};
use core::fmt;
use core::ptr;
use std::io;
use std::sync::OnceLock;

use crate::ffi;

/// Holds one `git_libgit2_init` reference for the rest of the process; `false` when
/// libgit2 cannot be initialised.
///
/// Until the first initialisation, and after the last shutdown, libgit2's error state
/// is looked up under a TLS key that does not exist: `errors.c` then writes into
/// whatever block key 0 holds. Its allocator also fails every request, and
/// `git__malloc` reports that through the same state. A reference that is never
/// released keeps the key live across balanced `git_libgit2_init` /
/// `git_libgit2_shutdown` pairs.
///
/// The calling thread's `errno` survives the first call, which initialises libgit2,
/// so a `GIT_ERROR_OS` error set right after still describes the caller's failure.
///
/// Raw seam: replace with the safe `git_libgit2_init` wrapper (`core/global.rs`).
pub(crate) fn runtime_pinned() -> bool {
    static PINNED: OnceLock<bool> = OnceLock::new();
    if let Some(&pinned) = PINNED.get() {
        return pinned;
    }
    let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
    let pinned = *PINNED.get_or_init(|| {
        // SAFETY: no arguments; libgit2 counts initialisations under its own lock, so
        // concurrent callers are serialised. A negative result is a failed
        // initialisation, after which nothing reaches libgit2's thread state.
        let count = unsafe { ffi::git_libgit2_init() };
        count >= 0
    });
    // SAFETY: `__errno_location` returns the calling thread's `errno`, valid for writes
    // for the thread's lifetime.
    unsafe { libc::__errno_location().write(errno) };
    pinned
}

/// Clears the calling thread's last libgit2 error: `git_error_last` then reports
/// `GIT_ERROR_NONE`, "no error".
///
/// Also resets the thread's `errno` to 0, as libgit2 does, unless the thread's error
/// state cannot be allocated.
///
/// Wraps: git_error_clear
pub fn clear() {
    if runtime_pinned() {
        // SAFETY: no arguments; the runtime is pinned, so the thread state is looked up
        // under libgit2's live TLS key.
        unsafe { ffi::git_error_clear() }
    }
}

/// Sets the calling thread's last error to `message` in `class`.
///
/// `message` is handed to `git_error_set` as the argument of a `"%s"` format, so a `%`
/// in it is literal. For `GIT_ERROR_OS`, libgit2 appends `": "` and, when the thread's
/// `errno` is non-zero, its description, then resets `errno` to 0. libgit2 copies the
/// message; nothing borrows it after the call.
///
/// When the message cannot be stored, the last error is `GIT_ERROR_NOMEMORY`, or the
/// previous one once the thread's buffer is out of memory (see the module docs).
///
/// ```compile_fail,E0308
/// // The message must be NUL-terminated: a `&str` is not.
/// libgit2::libgit2::util::sys::errors::set(libgit2::ffi::GIT_ERROR_INVALID, "bad");
/// ```
///
/// Wraps: git_error_set
pub fn set(class: ffi::git_error_t, message: &CStr) {
    if runtime_pinned() {
        // SAFETY: a `"%s"` format with the one NUL-terminated string argument it
        // names, both read during the call only; `class` is any `int` (the cast keeps
        // C's conversion of a `git_error_t`). The runtime is pinned, so the thread
        // state is looked up under libgit2's live TLS key.
        unsafe { ffi::git_error_set(class as c_int, c"%s".as_ptr(), message.as_ptr()) }
    }
}

/// Sets the calling thread's last error to `class` without a message of its own:
/// `git_error_set` with a NULL format.
///
/// For `GIT_ERROR_OS` the message is the description of the thread's `errno` when it
/// is non-zero, and empty otherwise, and `errno` is reset to 0; for any other class
/// the message is empty.
///
/// Only the `errno` description needs storage: when it cannot be stored, the last
/// error is `GIT_ERROR_NOMEMORY`. Once the thread's buffer is out of memory, the
/// previous error stays, whatever the class (see the module docs).
///
/// Wraps: git_error_set
pub fn set_without_message(class: ffi::git_error_t) {
    if runtime_pinned() {
        // SAFETY: a NULL format, which libgit2 checks before formatting, so no variadic
        // argument is read; `class` is any `int`, as in `set`. The runtime is pinned.
        unsafe { ffi::git_error_set(class as c_int, ptr::null()) }
    }
}

/// Sets the calling thread's last error to a verbatim copy of `message` in `class`.
///
/// Unlike [`set`], it never consults `errno`, and it reports whether the message was
/// stored. On failure the last error is `GIT_ERROR_NOMEMORY`, or the previous one when
/// the thread's buffer is already out of memory (see the module docs).
///
/// Wraps: git_error_set_str
pub fn set_str(class: ffi::git_error_t, message: &CStr) -> Result<(), MessageNotStored> {
    if !runtime_pinned() {
        return Err(MessageNotStored);
    }
    // SAFETY: a NUL-terminated string, copied during the call and never retained;
    // `class` is any `int`, as in `set`. The runtime is pinned.
    match unsafe { ffi::git_error_set_str(class as c_int, message.as_ptr()) } {
        0 => Ok(()),
        _ => Err(MessageNotStored),
    }
}

/// Sets the calling thread's last error to libgit2's static out-of-memory error,
/// `GIT_ERROR_NOMEMORY`, "Out of memory", copying no message.
///
/// Use it to report an allocation failure: unlike [`set_str`] it needs no storage for
/// the message, beyond the thread's error state on its first use.
///
/// Wraps: git_error_set_oom
pub fn set_oom() {
    if runtime_pinned() {
        // SAFETY: no arguments; the runtime is pinned, so the thread state is looked up
        // under libgit2's live TLS key.
        unsafe { ffi::git_error_set_oom() }
    }
}

/// [`set_str`] could not store the message: libgit2 ran out of memory, or cannot be
/// initialised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageNotStored;

impl fmt::Display for MessageNotStored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("libgit2 could not store the error message")
    }
}

impl std::error::Error for MessageNotStored {}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use std::process::Command;

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

    fn set_errno(value: c_int) {
        // SAFETY: the calling thread's `errno`, valid for writes.
        unsafe { libc::__errno_location().write(value) }
    }

    fn errno() -> c_int {
        io::Error::last_os_error().raw_os_error().unwrap()
    }

    /// `git_error_last()`'s class and message, copied out.
    fn last_error() -> (c_int, Vec<u8>) {
        // SAFETY: `git_error_last` never returns NULL, and its message lives until the
        // next libgit2 call on this thread; both are copied out before one.
        unsafe {
            let e = ffi::git_error_last();
            ((*e).klass, CStr::from_ptr((*e).message).to_bytes().to_vec())
        }
    }

    #[test]
    fn message_not_stored_reports_itself() {
        let e = MessageNotStored;
        assert_eq!(e.to_string(), "libgit2 could not store the error message");
        assert_eq!(format!("{e:?}"), "MessageNotStored");
        let boxed: Box<dyn std::error::Error> = Box::new(e);
        assert!(boxed.source().is_none());
    }

    /// The pin's own initialisation leaves the caller's `errno` alone, so an OS error
    /// set right after describes the caller's failure.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn first_pin_keeps_errno() {
        isolated(concat!(module_path!(), "::first_pin_keeps_errno"), || {
            set_errno(libc::ENOENT);
            assert!(runtime_pinned());
            assert_eq!(errno(), libc::ENOENT);
            set(ffi::GIT_ERROR_OS, c"open");
            let expected = b"open: No such file or directory".to_vec();
            assert_eq!(last_error(), (ffi::GIT_ERROR_OS as c_int, expected));
            assert_eq!(errno(), 0, "libgit2 consumed it");
        });
    }

    /// Later pins are a lookup, and keep `errno` too.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn later_pins_keep_errno() {
        assert!(runtime_pinned());
        set_errno(libc::EACCES);
        assert!(runtime_pinned());
        assert_eq!(errno(), libc::EACCES);
        set_errno(0);
    }

    /// Threads racing to the first pin each keep their own `errno`, and all see the
    /// one reference.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn racing_first_pins_keep_each_errno() {
        isolated(
            concat!(module_path!(), "::racing_first_pins_keep_each_errno"),
            || {
                let barrier = std::sync::Barrier::new(8);
                std::thread::scope(|s| {
                    for code in 1..=8 {
                        let barrier = &barrier;
                        s.spawn(move || {
                            barrier.wait();
                            set_errno(code);
                            assert!(runtime_pinned());
                            assert_eq!(errno(), code);
                        });
                    }
                });
                // SAFETY: no arguments; the extra reference dies with the process.
                let count = unsafe { ffi::git_libgit2_init() };
                assert_eq!(count, 2, "one pinned reference");
            },
        );
    }
}
