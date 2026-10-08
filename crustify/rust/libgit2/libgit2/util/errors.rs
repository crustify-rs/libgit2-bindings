//! Safe wrappers over `include/git2/errors.h`.
//!
//! libgit2 publishes a `git_error` only by `const` pointer, from `git_error_last`,
//! and never hands one over: it is either one of four static errors (out of memory,
//! not initialised, thread-state failure, no error) or the calling thread's error
//! state, whose `message` aliases a buffer that the next `git_error_set*` or
//! `git_error_clear` on that thread rewrites and may reallocate. Its lifecycle routines
//! (`git_error_save`, `git_error_restore`, `git_error_free`) are internal to
//! libgit2.so.
//!
//! So an [`ErrorRef`] is valid only until the next libgit2 call on its thread, which no
//! Rust lifetime can express across safe calls: whoever builds one must not hand it out
//! past such a call. [`ErrorBuf`] is the owned copy that outlives it, and crosses
//! threads; it is what a safe wrapper returns.

use core::ffi::{CStr, c_int};
use core::fmt;
use core::ptr::addr_of;
use std::ffi::CString;

use ffibox::define_ctype;

use crate::ffi;

define_ctype!(
    /// Extra details of a libgit2 error: its class and message (`git_error`).
    ///
    /// Only ever borrowed: an [`ErrorRef`] over an error libgit2 published, whose
    /// getters read it in place. Besides `from_ptr`'s requirements, building one
    /// requires `message` to be NULL or a NUL-terminated string, both staying live and
    /// unchanged for the handle's lifetime: for a thread's error, until the next
    /// libgit2 call on that thread. The handles are neither `Send` nor `Sync`, since
    /// that error belongs to its thread:
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::util::errors::ErrorRef;
    ///
    /// fn send<T: Send>(_: T) {}
    /// fn forward(error: ErrorRef<'static>) {
    ///     send(error);
    /// }
    /// ```
    ///
    /// There are no setters: libgit2 publishes the struct read-only and no public
    /// routine takes one. [`ErrorRef::to_error_buf`] copies it.
    ///
    /// Wraps: git_error
    Error, ErrorRef, ErrorMut, ffi::git_error
);

impl<'a> ErrorRef<'a> {
    /// The error message, borrowed from the error. libgit2 never publishes a NULL
    /// message; one reads as empty.
    ///
    /// Field: git_error.message
    pub fn message(&self) -> &'a CStr {
        // SAFETY: a read through the shared handle's pointer to a live `git_error`.
        let message = unsafe { addr_of!((*self.as_ptr()).message).read() };
        if message.is_null() {
            return c"";
        }
        // SAFETY: per this handle's invariant, a non-NULL `message` is a NUL-terminated
        // string that stays live and unchanged for `'a`.
        unsafe { CStr::from_ptr(message) }
    }

    /// The error's class, a `git_error_t` value (`GIT_ERROR_*`).
    ///
    /// Raw seam: the `git_error_t` wrapper replaces the integer.
    ///
    /// Field: git_error.klass
    pub fn class(&self) -> i32 {
        // SAFETY: a read through the shared handle's pointer to a live `git_error`.
        unsafe { addr_of!((*self.as_ptr()).klass).read() }
    }

    /// An owned copy of the error, which outlives it.
    pub fn to_error_buf(&self) -> ErrorBuf {
        ErrorBuf {
            class: self.class(),
            message: self.message().to_owned(),
        }
    }
}

impl fmt::Debug for ErrorRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("class", &self.class())
            .field("message", &self.message())
            .finish()
    }
}

/// An owned copy of a libgit2 error, held in Rust storage.
///
/// libgit2's own copy, `git_error_save`, is internal to libgit2.so, and nothing public
/// takes a `git_error` back, so the copy needs no C layout. It displays as its message;
/// libgit2 messages are bytes (paths among them), shown lossily.
///
/// Handwritten
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ErrorBuf {
    class: c_int,
    message: CString,
}

impl ErrorBuf {
    /// The error's class, a `git_error_t` value (`GIT_ERROR_*`).
    ///
    /// Raw seam: the `git_error_t` wrapper replaces the integer.
    pub fn class(&self) -> i32 {
        self.class
    }

    /// The error message.
    pub fn message(&self) -> &CStr {
        &self.message
    }
}

impl From<ErrorRef<'_>> for ErrorBuf {
    fn from(error: ErrorRef<'_>) -> Self {
        error.to_error_buf()
    }
}

impl fmt::Display for ErrorBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message.to_string_lossy())
    }
}

impl std::error::Error for ErrorBuf {}

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

    fn init() {
        // SAFETY: no arguments; the reference is never released, so the runtime and
        // this process's thread states stay live.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// `git_error_last()` adopted as a handle, and C's own reading of it.
    ///
    /// The handle is used only before the next libgit2 call on this thread.
    fn last() -> (ErrorRef<'static>, Vec<u8>, c_int) {
        // SAFETY: `git_error_last` never returns NULL; the error and its message stay
        // live and unchanged until the next libgit2 call on this thread, which every
        // caller makes only after it is done with the handle.
        unsafe {
            let e = ffi::git_error_last();
            let c_message = CStr::from_ptr((*e).message).to_bytes().to_vec();
            (
                ErrorRef::from_ptr(e.cast_mut()).unwrap(),
                c_message,
                (*e).klass,
            )
        }
    }

    fn set_str(class: ffi::git_error_t, message: &CStr) {
        // SAFETY: `message` is a live C string, which libgit2 copies.
        let rc = unsafe { ffi::git_error_set_str(class as c_int, message.as_ptr()) };
        assert_eq!(rc, 0);
    }

    #[test]
    fn null_message_reads_as_empty() {
        let mut raw = ffi::git_error {
            message: core::ptr::null_mut(),
            klass: 3,
        };
        // SAFETY: `raw` is a live `git_error` with a NULL message for the handle's life.
        let e = unsafe { ErrorRef::from_ptr(&raw mut raw) }.unwrap();
        assert_eq!(e.message(), c"");
        assert_eq!(e.class(), 3);
        assert_eq!(e.to_error_buf().message(), c"");
    }

    #[test]
    fn copy_matches_and_outlives_the_borrowed_error() {
        let mut text = *b"bad object\0";
        let mut raw = ffi::git_error {
            message: text.as_mut_ptr().cast(),
            klass: 9,
        };
        let buf = {
            // SAFETY: `raw` and `text` stay live and unchanged for the handle's life.
            let e = unsafe { ErrorRef::from_ptr(&raw mut raw) }.unwrap();
            assert_eq!(e.message(), c"bad object");
            assert_eq!(e.class(), 9);
            assert_eq!(
                format!("{e:?}"),
                r#"Error { class: 9, message: "bad object" }"#
            );
            ErrorBuf::from(e)
        };
        text[0] = b'X';
        assert_eq!(&text[..3], b"Xad");
        assert_eq!(buf.message(), c"bad object");
        assert_eq!(buf.class(), 9);
        assert_eq!(buf.to_string(), "bad object");
        assert_eq!(buf.clone(), buf);
        let dyn_err: &dyn std::error::Error = &buf;
        assert!(dyn_err.source().is_none());
    }

    #[test]
    fn non_utf8_message_displays_lossily() {
        let mut text = *b"path \xff\0";
        let mut raw = ffi::git_error {
            message: text.as_mut_ptr().cast(),
            klass: 2,
        };
        // SAFETY: as above.
        let e = unsafe { ErrorRef::from_ptr(&raw mut raw) }.unwrap();
        let buf = e.to_error_buf();
        assert_eq!(buf.message().to_bytes(), b"path \xff");
        assert_eq!(buf.to_string(), "path \u{fffd}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn thread_error_reads_as_c_does() {
        init();
        set_str(ffi::GIT_ERROR_CONFIG, c"first failure");
        let (e, message, klass) = last();
        assert_eq!(e.message().to_bytes(), message);
        assert_eq!(e.class(), klass);
        assert_eq!(e.class(), ffi::GIT_ERROR_CONFIG as c_int);
        assert_eq!(e.message(), c"first failure");
    }

    /// The copy survives the thread's message buffer being rewritten and reallocated,
    /// which leaves a borrowed message dangling; ASan flags a copy that aliases it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copy_survives_the_next_error() {
        init();
        set_str(ffi::GIT_ERROR_ODB, c"short");
        let first = last().0.to_error_buf();
        let long = CString::new("x".repeat(8192)).unwrap();
        set_str(ffi::GIT_ERROR_INDEX, &long);
        let (e, message, klass) = last();
        assert_eq!(e.message(), long.as_c_str());
        assert_eq!((e.message().to_bytes(), e.class()), (&message[..], klass));
        assert_eq!(first.message(), c"short");
        assert_eq!(first.class(), ffi::GIT_ERROR_ODB as c_int);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn static_errors_read_as_c_does() {
        init();
        // SAFETY: no arguments.
        unsafe { ffi::git_error_clear() };
        let (e, message, klass) = last();
        assert_eq!(
            (e.message(), e.class()),
            (c"no error", ffi::GIT_ERROR_NONE as c_int)
        );
        assert_eq!((e.message().to_bytes(), e.class()), (&message[..], klass));

        // SAFETY: no arguments.
        unsafe { ffi::git_error_set_oom() };
        let (e, message, klass) = last();
        assert_eq!(
            (e.message(), e.class()),
            (c"Out of memory", ffi::GIT_ERROR_NOMEMORY as c_int)
        );
        assert_eq!((e.message().to_bytes(), e.class()), (&message[..], klass));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn error_before_init_is_static() {
        isolated(
            concat!(module_path!(), "::error_before_init_is_static"),
            || {
                let (e, message, klass) = last();
                assert_eq!(e.message(), c"library has not been initialized");
                assert_eq!(e.class(), ffi::GIT_ERROR_INVALID as c_int);
                assert_eq!((e.message().to_bytes(), e.class()), (&message[..], klass));
            },
        );
    }

    /// A thread whose error state cannot be allocated reads a static error.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn thread_state_failure_is_static() {
        use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
        use core::ptr::NonNull;
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

        static REFUSING: AtomicBool = AtomicBool::new(false);

        unsafe extern "C" {
            fn malloc(n: usize) -> *mut core::ffi::c_void;
            fn realloc(p: *mut core::ffi::c_void, n: usize) -> *mut core::ffi::c_void;
            fn free(p: *mut core::ffi::c_void);
        }

        /// libc's allocator, refusing everything while `REFUSING` is set.
        struct Refusing;

        // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
        unsafe impl Alloc for Refusing {
            fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                if REFUSING.load(SeqCst) {
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
                if REFUSING.load(SeqCst) {
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

        isolated(
            concat!(module_path!(), "::thread_state_failure_is_static"),
            || {
                let allocator = Allocator::new::<Refusing>();
                // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with
                // every member set; libgit2 copies it before anything is allocated.
                let rc = unsafe {
                    ffi::git_libgit2_opts(
                        ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                        allocator.as_ref().as_ptr(),
                    )
                };
                assert_eq!(rc, 0);
                init();
                REFUSING.store(true, SeqCst);
                let buf = std::thread::spawn(|| {
                    let (e, message, klass) = last();
                    assert_eq!((e.message().to_bytes(), e.class()), (&message[..], klass));
                    e.to_error_buf()
                })
                .join()
                .unwrap();
                REFUSING.store(false, SeqCst);
                assert_eq!(buf.message(), c"thread-local data initialization failure");
                assert_eq!(buf.class(), ffi::GIT_ERROR_THREAD as c_int);
            },
        );
    }

    /// Each thread has its own error; the copy crosses threads, the handle cannot.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copy_crosses_threads() {
        init();
        set_str(ffi::GIT_ERROR_NET, c"here");
        let here = last().0.to_error_buf();
        let there = std::thread::spawn(move || {
            assert_eq!(here.message(), c"here");
            last().0.to_error_buf()
        })
        .join()
        .unwrap();
        assert_eq!(there.message(), c"no error");
        assert_eq!(last().0.message(), c"here");
    }
}
