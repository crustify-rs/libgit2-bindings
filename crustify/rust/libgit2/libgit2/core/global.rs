//! Safe wrappers over `include/git2/global.h`: initialising and shutting down
//! libgit2's global state.
//!
//! libgit2 counts initialisations (`git_runtime_init`, under its own mutex): only the
//! first, the 0 -> 1 transition, sets the subsystems up (the allocator, the per-thread
//! error state's TLS key, the system directories, OpenSSL, ...), and only the shutdown
//! that brings the count back to 0 tears them down again. A [`Runtime`] is one counted
//! reference: [`init`] takes it, and dropping it (or [`Runtime::shutdown`]) releases
//! exactly that one, so the count can never be released more often than it was taken.
//! A C caller can call `git_libgit2_shutdown` unbalanced, driving the count negative,
//! after which libgit2 never sets itself up again.
//!
//! Every safe wrapper that needs libgit2's global state holds a reference of its own,
//! taken on first use and kept for the rest of the process (`runtime_pinned`, in
//! `util::sys::errors`, a [`Runtime`] that is never dropped). So releasing a
//! [`Runtime`] never tears down state a live object relies on: the count reaches 0
//! only while no wrapper has needed libgit2 yet. A [`Runtime`] is for code that wants
//! the state set up eagerly, to see an initialisation failure where it happens, or to
//! tear libgit2 down again before anything pinned it.
//!
//! When the first initialisation fails part-way, libgit2 leaves its count at 1 with
//! some subsystems set up and others not, the per-thread error state possibly without
//! a TLS key, and answers every later `git_libgit2_init` with success. [`init`] does not
//! trust that: once an initialisation through it has failed, it refuses every later one
//! ([`InitError::Poisoned`]) without calling libgit2.

use core::ffi::c_int;
use core::fmt;
use core::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};

use crate::ffi;

/// The most references [`init`] lets libgit2's count reach: half of `c_int::MAX`,
/// well short of the wrap-around that would make libgit2 set itself up a second time
/// under live users (as `Arc` stops short of `isize::MAX`).
///
/// The unit tests lower it, so they can reach it.
const MAX_REFERENCES: c_int = if cfg!(test) { 64 } else { c_int::MAX / 2 };

/// Serialises [`init`]'s calls and records whether one failed: a failure is recorded
/// before any later caller can reach `git_libgit2_init`, which would answer it with a
/// success.
static INIT: Mutex<bool> = Mutex::new(false);

/// One `git_libgit2_init` reference: libgit2's global state stays initialised while
/// it lives, and dropping it releases the reference with `git_libgit2_shutdown`.
///
/// It holds no pointer and no thread affinity — libgit2 counts references under its
/// own mutex — so it is `Send` and `Sync`, and may be dropped on any thread. It is not
/// `Clone`: another reference is another [`init`], which can fail.
///
/// Handwritten
#[derive(Debug)]
#[must_use = "dropping the reference releases it at once"]
pub struct Runtime {
    count: NonZeroUsize,
}

/// Initialises libgit2's global state, or takes one more reference to it.
///
/// Returns the reference, which knows how many references were live once it was
/// taken, itself included ([`Runtime::count`]): 1 when this call set libgit2 up.
///
/// Fails when libgit2 cannot be initialised ([`InitError::Failed`]); from then on
/// every call fails ([`InitError::Poisoned`]), where C's `git_libgit2_init` would
/// report success over half-initialised state. It also refuses to let the count grow
/// past `c_int::MAX / 2` ([`InitError::TooManyReferences`]), so a program leaking
/// references cannot wrap it around.
///
/// No libgit2 error is set or reported: after a failed initialisation, the per-thread
/// error state may have no TLS key to be looked up under.
///
/// Wraps: git_libgit2_init
pub fn init() -> Result<Runtime, InitError> {
    let mut failed = INIT.lock().unwrap_or_else(PoisonError::into_inner);
    if *failed {
        return Err(InitError::Poisoned);
    }
    // SAFETY: no arguments; libgit2 counts initialisations under its own mutex, so it
    // is callable from any thread at any time.
    let count = unsafe { ffi::git_libgit2_init() };
    if count > MAX_REFERENCES {
        // SAFETY: releases the reference this call just took, which is not the last:
        // the count is far above 1.
        unsafe { ffi::git_libgit2_shutdown() };
        return Err(InitError::TooManyReferences);
    }
    match usize::try_from(count).ok().and_then(NonZeroUsize::new) {
        Some(count) => Ok(Runtime { count }),
        None => {
            *failed = true;
            Err(InitError::Failed)
        }
    }
}

impl Runtime {
    /// How many references were live once this one was taken, itself included, as
    /// `git_libgit2_init` returned it: 1 when taking it set libgit2 up.
    ///
    /// A snapshot: other references come and go independently.
    pub fn count(&self) -> NonZeroUsize {
        self.count
    }

    /// Releases the reference, as dropping it does, and reports how many references
    /// remain, as `git_libgit2_shutdown` returns it: 0 when this one was the last and
    /// libgit2 tore its global state down.
    ///
    /// Fails only when libgit2 cannot take or release its initialisation mutex; the
    /// reference may then stay counted, which keeps the state alive rather than
    /// tearing it down under another holder.
    ///
    /// Wraps: git_libgit2_shutdown
    pub fn shutdown(self) -> Result<usize, ShutdownError> {
        core::mem::forget(self);
        release()
    }
}

/// Releases the reference with `git_libgit2_shutdown`, ignoring a mutex failure as
/// [`Runtime::shutdown`] would report it.
///
/// Wraps: git_libgit2_shutdown
impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = release();
    }
}

/// `git_libgit2_shutdown` for a reference a [`Runtime`] held.
fn release() -> Result<usize, ShutdownError> {
    // SAFETY: no arguments; the caller gives up a reference `init` took, so the count
    // is released no more often than it was taken. libgit2 counts under its own mutex.
    let remaining = unsafe { ffi::git_libgit2_shutdown() };
    usize::try_from(remaining).map_err(|_| ShutdownError)
}

/// [`init`] could not take a reference to libgit2's global state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InitError {
    /// `git_libgit2_init` failed: libgit2 could not take its initialisation mutex, or
    /// a subsystem could not be set up (an allocator refusing requests, say).
    Failed,
    /// An earlier initialisation through [`init`] failed, leaving libgit2
    /// half-initialised for the rest of the process.
    Poisoned,
    /// libgit2's count of references reached [`init`]'s ceiling, `c_int::MAX / 2`.
    TooManyReferences,
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Failed => "libgit2 could not be initialised",
            Self::Poisoned => "an earlier libgit2 initialisation failed",
            Self::TooManyReferences => "too many libgit2 initialisations are live",
        })
    }
}

impl std::error::Error for InitError {}

/// [`Runtime::shutdown`] failed: libgit2 could not take or release its
/// initialisation mutex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShutdownError;

impl fmt::Display for ShutdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("libgit2 could not take its initialisation lock to shut down")
    }
}

impl std::error::Error for ShutdownError {}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::core::common::set_allocator;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use core::ffi::CStr;
    use core::ptr::NonNull;
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

    /// An allocator refusing every request, so the first initialisation fails.
    struct Refusing;

    // SAFETY: it never hands out an allocation, which `Alloc` allows; `free` is never
    // reached, since nothing was allocated.
    unsafe impl Alloc for Refusing {
        fn malloc(_: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            None
        }
        unsafe fn realloc(
            _: Option<NonNull<u8>>,
            _: usize,
            _: Option<&CStr>,
            _: i32,
        ) -> Option<NonNull<u8>> {
            None
        }
        unsafe fn free(_: NonNull<u8>) {}
    }

    static REFUSING: Allocator = Allocator::new::<Refusing>();

    #[test]
    fn errors_report_themselves() {
        assert_eq!(
            InitError::Failed.to_string(),
            "libgit2 could not be initialised"
        );
        assert_eq!(
            InitError::Poisoned.to_string(),
            "an earlier libgit2 initialisation failed"
        );
        assert_eq!(
            InitError::TooManyReferences.to_string(),
            "too many libgit2 initialisations are live"
        );
        assert_eq!(
            ShutdownError.to_string(),
            "libgit2 could not take its initialisation lock to shut down"
        );
        let boxed: Box<dyn std::error::Error> = Box::new(InitError::Poisoned);
        assert!(boxed.source().is_none());
        assert_eq!(format!("{:?}", InitError::Failed), "Failed");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn runtime_debug_shows_its_count() {
        let r = init().unwrap();
        assert!(format!("{r:?}").starts_with("Runtime { count: "));
    }

    /// Corrects C: after a failed first initialisation, `git_libgit2_init` answers
    /// every later call with success over half-initialised state; `init` refuses.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_failed_initialisation_poisons_later_ones() {
        isolated(
            concat!(
                module_path!(),
                "::a_failed_initialisation_poisons_later_ones"
            ),
            || {
                // SAFETY: nothing has initialised libgit2 or allocated through it in
                // this child, and no other thread runs.
                unsafe { set_allocator(Some(REFUSING.as_ref())) };
                assert_eq!(init().unwrap_err(), InitError::Failed);
                // SAFETY: the system allocator, which refused nothing and so owns
                // every block libgit2 holds (none); no other thread runs.
                unsafe { set_allocator(None) };
                assert_eq!(init().unwrap_err(), InitError::Poisoned);
                assert_eq!(init().unwrap_err(), InitError::Poisoned, "for good");
                // C's own answer: success, over the count the failure left at 1.
                // SAFETY: no arguments; the reference dies with the child.
                assert_eq!(unsafe { ffi::git_libgit2_init() }, 2);
                assert_eq!(init().unwrap_err(), InitError::Poisoned);
            },
        );
    }

    /// The count stops at the ceiling (64 in unit tests): the reference that would
    /// pass it is released again and refused.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn the_count_stops_at_the_ceiling() {
        isolated(
            concat!(module_path!(), "::the_count_stops_at_the_ceiling"),
            || {
                let held: Vec<Runtime> = (1..=MAX_REFERENCES)
                    .map(|n| {
                        let r = init().unwrap();
                        assert_eq!(r.count().get(), n as usize);
                        r
                    })
                    .collect();
                assert_eq!(init().unwrap_err(), InitError::TooManyReferences);
                assert_eq!(init().unwrap_err(), InitError::TooManyReferences);
                let mut held = held;
                assert_eq!(held.pop().unwrap().shutdown(), Ok(63));
                assert_eq!(init().unwrap().count().get(), 64, "not poisoned");
                drop(held);
                assert_eq!(init().unwrap().count().get(), 1);
            },
        );
    }

    /// `shutdown` and `drop` release exactly one reference each.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shutdown_and_drop_release_one_each() {
        isolated(
            concat!(module_path!(), "::shutdown_and_drop_release_one_each"),
            || {
                let a = init().unwrap();
                let b = init().unwrap();
                let c = init().unwrap();
                assert_eq!(c.count().get(), 3);
                drop(b);
                assert_eq!(a.shutdown(), Ok(1));
                assert_eq!(c.shutdown(), Ok(0));
                // SAFETY: no arguments; balanced by the shutdown below.
                assert_eq!(unsafe { ffi::git_libgit2_init() }, 1, "back at 0 before");
                // SAFETY: releases the reference just taken.
                assert_eq!(unsafe { ffi::git_libgit2_shutdown() }, 0);
            },
        );
    }
}
