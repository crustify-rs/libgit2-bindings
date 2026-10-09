//! Safe wrappers over `include/git2/trace.h`.
//!
//! libgit2 passes a trace level (`git_trace_level_t`) by value: `git_trace_set` takes
//! the most verbose level to report, and the trace callback receives the level of
//! each message. libgit2 reports a message when the configured level is greater than
//! or equal to the message's, so the levels are ordered by their C values, from
//! [`TraceLevel::None`] (0, nothing reported) to [`TraceLevel::Trace`] (6,
//! everything); [`TraceLevel`]'s `Ord` is that order. `git_trace_set` stores any
//! value it is given, so a raw value is converted fallibly
//! ([`TraceLevel::from_raw`]) and never transmuted.
//!
//! The trace callback (`git_trace_cb`) is a plain C function, with no payload:
//! libgit2 calls it with a level and the formatted message, from whichever thread runs
//! the traced operation (today only the HTTP client traces). A [`TraceCallback`] is
//! such a function: Rust builds one over a [`Tracer`], a type whose associated
//! function receives the messages, and calling one is safe.
//!
//! [`set`] installs a callback at a level and [`clear`] removes it (`git_trace_set`).
//! libgit2 keeps both in one global that its traced routines read without a lock, so
//! both are `unsafe`: no other thread may be inside libgit2 meanwhile.

use core::ffi::{CStr, c_char};
use core::fmt;

use crate::ffi;

/// How much libgit2 traces: a level enables its own messages and those of every
/// lower level, so `configured >= level` decides whether a message is reported, as
/// libgit2 decides it.
///
/// Wraps: git_trace_level_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum TraceLevel {
    /// No tracing will be performed.
    None = ffi::GIT_TRACE_NONE,
    /// Severe errors that may impact the program's execution.
    Fatal = ffi::GIT_TRACE_FATAL,
    /// Errors that do not impact the program's execution.
    Error = ffi::GIT_TRACE_ERROR,
    /// Warnings that suggest abnormal data.
    Warn = ffi::GIT_TRACE_WARN,
    /// Informational messages about program execution.
    Info = ffi::GIT_TRACE_INFO,
    /// Detailed data that allows for debugging.
    Debug = ffi::GIT_TRACE_DEBUG,
    /// Exceptionally detailed debugging data.
    Trace = ffi::GIT_TRACE_TRACE,
}

impl TraceLevel {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_trace_level_t) -> Option<Self> {
        match raw {
            ffi::GIT_TRACE_NONE => Some(Self::None),
            ffi::GIT_TRACE_FATAL => Some(Self::Fatal),
            ffi::GIT_TRACE_ERROR => Some(Self::Error),
            ffi::GIT_TRACE_WARN => Some(Self::Warn),
            ffi::GIT_TRACE_INFO => Some(Self::Info),
            ffi::GIT_TRACE_DEBUG => Some(Self::Debug),
            ffi::GIT_TRACE_TRACE => Some(Self::Trace),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    pub const fn raw(self) -> ffi::git_trace_level_t {
        self as ffi::git_trace_level_t
    }
}

impl TryFrom<ffi::git_trace_level_t> for TraceLevel {
    type Error = UnnamedTraceLevel;

    fn try_from(raw: ffi::git_trace_level_t) -> Result<Self, UnnamedTraceLevel> {
        Self::from_raw(raw).ok_or(UnnamedTraceLevel(raw))
    }
}

impl From<TraceLevel> for ffi::git_trace_level_t {
    fn from(value: TraceLevel) -> ffi::git_trace_level_t {
        value.raw()
    }
}

/// A trace level that this version of libgit2's header does not name; [`raw`](Self::raw)
/// has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedTraceLevel(ffi::git_trace_level_t);

impl UnnamedTraceLevel {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_trace_level_t {
        self.0
    }
}

impl fmt::Display for UnnamedTraceLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 trace level {} is not named by git2/trace.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedTraceLevel {}

// --- Callback ------------------------------------------------------------------------

/// `git_trace_cb`: receive one trace message, `msg`, reported at `level`.
///
/// libgit2's one caller, `git_trace__write_fmt` (behind every `git_trace`), passes a
/// named level above [`TraceLevel::None`] and a message it formatted into a buffer of
/// its own, which it frees when the call returns. It calls from whichever thread runs
/// the traced operation, so from several threads at once, and the message is whatever
/// was traced, a server's bytes included: NUL-terminated, but not necessarily UTF-8.
///
/// # Safety
///
/// `msg` must be a NUL-terminated string live for the call. A function may add
/// obligations of its own; the trampolines [`TraceCallback::new`] stores add none.
pub type TraceFn = unsafe extern "C" fn(level: ffi::git_trace_level_t, msg: *const c_char);

/// A receiver of libgit2's trace messages, which [`TraceCallback::new`] adapts to a C
/// callback.
///
/// libgit2 hands the callback no payload, so a tracer is a type rather than a value:
/// [`trace`](Self::trace) is an associated function, and what it keeps lives in
/// `static`s or thread-locals, which Rust already makes safe to reach from every
/// thread libgit2 calls it on, concurrently.
///
/// Handwritten
pub trait Tracer {
    /// Receives one message, reported at `level`.
    ///
    /// `message` is borrowed for the call only: libgit2 frees it when this returns, so
    /// keep a copy (`to_owned`) of what must outlive it. A panic here aborts the
    /// process, since it cannot unwind into C.
    ///
    /// Keeping the borrow does not compile:
    ///
    /// ```compile_fail,E0521
    /// use std::ffi::CStr;
    /// use std::sync::Mutex;
    /// use libgit2::libgit2::core::trace::{TraceLevel, Tracer};
    ///
    /// static KEPT: Mutex<Option<&'static CStr>> = Mutex::new(None);
    ///
    /// struct Keeping;
    ///
    /// impl Tracer for Keeping {
    ///     fn trace(_: TraceLevel, message: &CStr) {
    ///         *KEPT.lock().unwrap() = Some(message);
    ///     }
    /// }
    /// ```
    fn trace(level: TraceLevel, message: &CStr);
}

/// A trace callback (`git_trace_cb`): the C function libgit2 calls with each trace
/// message, the value [`set`] installs.
///
/// Built over a [`Tracer`] with [`new`](Self::new), whose `extern "C"` trampoline
/// adapts libgit2's call to it, or over a C function with
/// [`from_fn`](Self::from_fn), which vouches that the function is safe to call for
/// the rest of the process. Either way [`call`](Self::call) is safe: it passes a
/// named level and a borrowed message, as libgit2 does. A function pointer and
/// nothing else: `Copy`, owning nothing, never released, and callable from any thread.
///
/// ```
/// use std::ffi::CStr;
/// use libgit2::libgit2::core::trace::{TraceCallback, TraceLevel, Tracer};
///
/// struct Stderr;
///
/// impl Tracer for Stderr {
///     fn trace(level: TraceLevel, message: &CStr) {
///         eprintln!("{level:?}: {}", message.to_string_lossy());
///     }
/// }
///
/// const CALLBACK: TraceCallback = TraceCallback::new::<Stderr>();
/// CALLBACK.call(TraceLevel::Info, c"traced");
/// assert!(CALLBACK.raw().is_some());
/// ```
///
/// Its function is private: only `new` and the `unsafe` `from_fn` build one.
///
/// ```compile_fail,E0423
/// use std::ffi::c_char;
/// use libgit2::{ffi, libgit2::core::trace::TraceCallback};
///
/// unsafe extern "C" fn c_tracer(_: ffi::git_trace_level_t, _: *const c_char) {}
/// let _ = TraceCallback(c_tracer);
/// ```
///
/// Wraps: git_trace_cb
/// Handwritten
#[derive(Clone, Copy)]
pub struct TraceCallback(TraceFn);

impl TraceCallback {
    /// The callback that forwards libgit2's calls to `T`.
    ///
    /// Its trampoline turns the message into a `&CStr` borrowed for the call and the
    /// level into a [`TraceLevel`]. It drops a call that libgit2 never makes, with a
    /// level the header does not name or a NULL message, rather than reach `T` with it.
    pub const fn new<T: Tracer>() -> Self {
        Self(trace_with::<T>)
    }

    /// The callback over a C function.
    ///
    /// # Safety
    ///
    /// For the rest of the process, `f` must be callable with any [`TraceLevel`] and
    /// any NUL-terminated string live for the call, from any thread, concurrently and
    /// reentrantly, with no obligation beyond [`TraceFn`]'s, and must not unwind. Those
    /// are the calls [`call`](Self::call) and libgit2 make, and the obligation cannot
    /// end with this value: it is `Copy`, [`as_fn`](Self::as_fn) hands the function
    /// out again, and [`set`] installs it, after which libgit2 calls `f` from any
    /// thread for as long as it stays installed, with no `TraceCallback` left to tie
    /// that to.
    pub const unsafe fn from_fn(f: TraceFn) -> Self {
        Self(f)
    }

    /// Calls the callback with `message` reported at `level`, as libgit2 does.
    pub fn call(self, level: TraceLevel, message: &CStr) {
        // SAFETY: `message` is a NUL-terminated string borrowed for the call and
        // `level` a named one: `TraceFn`'s contract, which `new`'s trampolines add
        // nothing to and `from_fn`'s caller vouched is all the function needs.
        unsafe { (self.0)(level.raw(), message.as_ptr()) }
    }

    /// The C function, as libgit2 calls it.
    pub const fn as_fn(self) -> TraceFn {
        self.0
    }

    /// The C value, as `git_trace_set` takes it: never NULL.
    pub const fn raw(self) -> ffi::git_trace_cb {
        Some(self.0)
    }
}

impl fmt::Debug for TraceCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TraceCallback")
            .field(&(self.0 as *const ()))
            .finish()
    }
}

/// `git_trace_cb` over [`Tracer::trace`].
///
/// # Safety
///
/// As [`TraceFn`]: `msg` is NULL or a NUL-terminated string live for the call.
unsafe extern "C" fn trace_with<T: Tracer>(level: ffi::git_trace_level_t, msg: *const c_char) {
    let Some(level) = TraceLevel::from_raw(level) else {
        return;
    };
    if msg.is_null() {
        return;
    }
    // SAFETY: per this function's contract, a non-NULL `msg` is a C string live for the
    // call; the borrow handed to `T` ends with it.
    let message = unsafe { CStr::from_ptr(msg) };
    T::trace(level, message);
}

// --- Configuration -------------------------------------------------------------------

/// Installs `callback` as libgit2's tracer, reporting each message at `level` or a
/// less verbose one; [`TraceLevel::None`] reports nothing but leaves `callback`
/// installed. Replaces the level and callback set before.
///
/// libgit2 calls `callback` from whichever thread runs a traced operation until the
/// next `set` or [`clear`], which a [`TraceCallback`] allows for the rest of the
/// process. A level above `None` always comes with a callback here: C refuses the
/// pair without one (`-1`, or an abort in a debug build), which is [`clear`]'s state
/// at `None` only.
///
/// ```compile_fail,E0308
/// use libgit2::libgit2::core::trace::{self, TraceLevel};
///
/// // SAFETY: (not reached) no other thread calls into libgit2.
/// unsafe { trace::set(TraceLevel::Debug, None) };
/// ```
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the level and the callback in a global that its traced
/// routines read without synchronisation, the level first and the callback twice.
/// A traced call on the same thread, `callback` included, may.
///
/// Wraps: git_trace_set
pub unsafe fn set(level: TraceLevel, callback: TraceCallback) {
    // SAFETY: a named level with a callback, which `git_trace_set` accepts without
    // touching the error state, so before `git_libgit2_init` too; `callback` is safe
    // to call from any thread for the rest of the process, as libgit2 will call it.
    // The caller excludes every concurrent reader and writer of the global.
    let rc = unsafe { ffi::git_trace_set(level.raw(), callback.raw()) };
    debug_assert_eq!(rc, 0, "git_trace_set refused a callback");
}

/// Removes libgit2's tracer: nothing is reported until the next [`set`].
///
/// # Safety
///
/// As [`set`]'s: no other thread may call into libgit2 while this runs. A thread
/// tracing meanwhile could read the callback as set before its check and as NULL
/// at the call (libgit2 reads it twice), and call NULL.
///
/// Wraps: git_trace_set
pub unsafe fn clear() {
    // SAFETY: `GIT_TRACE_NONE` without a callback, which `git_trace_set` accepts
    // without touching the error state. The caller excludes every concurrent reader
    // and writer of the global, so no traced call sees the NULL after its check.
    let rc = unsafe { ffi::git_trace_set(TraceLevel::None.raw(), None) };
    debug_assert_eq!(rc, 0, "git_trace_set refused to clear");
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    const LEVELS: [TraceLevel; 7] = [
        TraceLevel::None,
        TraceLevel::Fatal,
        TraceLevel::Error,
        TraceLevel::Warn,
        TraceLevel::Info,
        TraceLevel::Debug,
        TraceLevel::Trace,
    ];

    #[test]
    fn raw_round_trips() {
        for level in LEVELS {
            assert_eq!(TraceLevel::from_raw(level.raw()), Some(level));
            assert_eq!(TraceLevel::try_from(level.raw()), Ok(level));
            assert_eq!(ffi::git_trace_level_t::from(level), level.raw());
        }
    }

    #[test]
    fn unnamed_values_are_refused_without_panicking() {
        for raw in [7, 8, 0x100, ffi::git_trace_level_t::MAX] {
            assert_eq!(TraceLevel::from_raw(raw), None);
            let err = TraceLevel::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(
                err.to_string(),
                format!("libgit2 trace level {raw} is not named by git2/trace.h")
            );
        }
    }

    #[test]
    fn order_is_verbosity() {
        assert!(LEVELS.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(LEVELS.iter().max(), Some(&TraceLevel::Trace));
        assert_eq!(LEVELS.iter().min(), Some(&TraceLevel::None));
    }

    mod callback {
        use super::super::*;
        use super::LEVELS;
        use std::cell::RefCell;
        use std::ffi::CString;

        thread_local! {
            static SEEN: RefCell<Vec<(TraceLevel, CString)>> = const { RefCell::new(Vec::new()) };
        }

        fn take_seen() -> Vec<(TraceLevel, CString)> {
            SEEN.with(|s| s.take())
        }

        /// Records each message on the calling thread.
        struct Recorder;

        impl Tracer for Recorder {
            fn trace(level: TraceLevel, message: &CStr) {
                SEEN.with(|s| s.borrow_mut().push((level, message.to_owned())));
            }
        }

        /// A C-ABI function honouring `from_fn`'s contract: it records on the calling
        /// thread, and only reads `msg` during the call.
        unsafe extern "C" fn c_recorder(level: ffi::git_trace_level_t, msg: *const c_char) {
            // SAFETY: `TraceFn`'s contract: `msg` is a C string live for the call.
            let message = unsafe { CStr::from_ptr(msg) }.to_owned();
            let level = TraceLevel::from_raw(level).expect("a named level");
            SEEN.with(|s| s.borrow_mut().push((level, message)));
        }

        #[test]
        fn new_forwards_every_level_and_message() {
            let cb = TraceCallback::new::<Recorder>();
            for level in LEVELS {
                cb.call(level, c"plain");
            }
            cb.call(TraceLevel::Info, c"");
            cb.call(TraceLevel::Debug, c"\xff\xfe not UTF-8");
            let mut want: Vec<_> = LEVELS.iter().map(|&l| (l, c"plain".to_owned())).collect();
            want.push((TraceLevel::Info, c"".to_owned()));
            want.push((TraceLevel::Debug, c"\xff\xfe not UTF-8".to_owned()));
            assert_eq!(take_seen(), want);
        }

        #[test]
        fn trampoline_drops_calls_libgit2_never_makes() {
            let f = TraceCallback::new::<Recorder>().as_fn();
            for raw in [7, 8, 0x100, ffi::git_trace_level_t::MAX] {
                // SAFETY: a C string live for the call; any level is accepted.
                unsafe { f(raw, c"unnamed".as_ptr()) };
            }
            // SAFETY: the trampoline accepts NULL, as its contract states.
            unsafe { f(ffi::GIT_TRACE_INFO, core::ptr::null()) };
            assert_eq!(take_seen(), []);
            // SAFETY: as above.
            unsafe { f(ffi::GIT_TRACE_WARN, c"named".as_ptr()) };
            assert_eq!(take_seen(), [(TraceLevel::Warn, c"named".to_owned())]);
        }

        #[test]
        fn message_is_borrowed_for_the_call_only() {
            let cb = TraceCallback::new::<Recorder>();
            for n in [0, 1, 255, 4096] {
                let message = CString::new(vec![b'x'; n]).unwrap();
                cb.call(TraceLevel::Trace, &message);
                // The tracer copied it: freeing the original leaves the copy intact.
                drop(message);
                let seen = take_seen();
                assert_eq!(seen.len(), 1);
                assert_eq!(seen[0].1.as_bytes().len(), n);
            }
        }

        #[test]
        fn from_fn_adopts_a_c_function() {
            // SAFETY: `c_recorder` reads `msg` only during the call, takes every named
            // level, keeps per-thread state only and never unwinds for a named level.
            let cb = unsafe { TraceCallback::from_fn(c_recorder) };
            cb.call(TraceLevel::Error, c"from C");
            assert_eq!(take_seen(), [(TraceLevel::Error, c"from C".to_owned())]);
            // Adopting the trampoline of `new` gives the same callback.
            // SAFETY: `new`'s trampolines meet `from_fn`'s contract.
            let again = unsafe { TraceCallback::from_fn(TraceCallback::new::<Recorder>().as_fn()) };
            again.call(TraceLevel::Fatal, c"again");
            assert_eq!(take_seen(), [(TraceLevel::Fatal, c"again".to_owned())]);
        }

        #[test]
        fn raw_is_the_function_never_null() {
            let cb = TraceCallback::new::<Recorder>();
            // Compare through one stored pointer: Miri gives each coercion its own
            // address.
            let f = cb.as_fn();
            let raw = cb.raw().expect("never NULL");
            assert!(core::ptr::fn_addr_eq(raw, f));
            let copy = cb;
            assert!(core::ptr::fn_addr_eq(copy.as_fn(), f));
            let debug = format!("{cb:?}");
            assert!(debug.starts_with("TraceCallback(0x"), "{debug}");
        }

        #[test]
        fn calls_cross_threads() {
            let cb = TraceCallback::new::<Recorder>();
            std::thread::scope(|s| {
                for i in 0..4u8 {
                    s.spawn(move || {
                        let message = CString::new(vec![b'a' + i; 8]).unwrap();
                        cb.call(TraceLevel::Info, &message);
                        assert_eq!(take_seen(), [(TraceLevel::Info, message)]);
                    });
                }
            });
            assert_eq!(take_seen(), []);
        }
    }

    #[test]
    fn layout_is_the_c_value() {
        assert_eq!(size_of::<TraceLevel>(), size_of::<ffi::git_trace_level_t>());
        assert_eq!(
            size_of::<Option<TraceLevel>>(),
            size_of::<ffi::git_trace_level_t>()
        );
    }
}
