//! Equivalence of `git2/trace.h`'s [`TraceLevel`] with the `git_trace_level_t`
//! values libgit2 declares and accepts.
//!
//! [`TraceLevel`] must name exactly the values of `git_trace_level_t`, each with its C
//! value, and order them as libgit2 does when it decides to report a message
//! (`configured >= level`, over the C values). libgit2 observes a level through
//! `git_trace_set`, which accepts any level with a callback and only
//! [`TraceLevel::None`] without one.
//!
//! Documented divergence, outside these comparisons: a debug build of libgit2 aborts
//! on a level above `None` without a callback (`GIT_ASSERT_ARG`), so that probe is
//! skipped under the `coverage` variant, the one debug build.

use std::ffi::{CStr, c_char};
use std::sync::Mutex;

use libgit2::ffi;
use libgit2::libgit2::core::trace::{TraceLevel, UnnamedTraceLevel};

/// Every `git_trace_level_t` constant, beside the variant that must carry it.
const LEVELS: [(ffi::git_trace_level_t, TraceLevel); 7] = [
    (ffi::GIT_TRACE_NONE, TraceLevel::None),
    (ffi::GIT_TRACE_FATAL, TraceLevel::Fatal),
    (ffi::GIT_TRACE_ERROR, TraceLevel::Error),
    (ffi::GIT_TRACE_WARN, TraceLevel::Warn),
    (ffi::GIT_TRACE_INFO, TraceLevel::Info),
    (ffi::GIT_TRACE_DEBUG, TraceLevel::Debug),
    (ffi::GIT_TRACE_TRACE, TraceLevel::Trace),
];

/// `git_trace_set` writes libgit2's global trace state without a lock: one writer at a
/// time in this binary.
static TRACE_STATE: Mutex<()> = Mutex::new(());

/// Every value around the declared ones and the extremes.
fn probes() -> impl Iterator<Item = ffi::git_trace_level_t> {
    (0..=64).chain([
        0x100,
        ffi::git_trace_level_t::MAX - 1,
        ffi::git_trace_level_t::MAX,
    ])
}

/// Initialises libgit2, whose error state a rejected call writes; never released.
fn init() {
    // SAFETY: no arguments; the reference is never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

/// The message of the calling thread's last error.
fn last_message() -> String {
    // SAFETY: `git_error_last` never returns NULL, and its message is a NUL-terminated
    // string, both live until the next libgit2 call on this thread, after the copy.
    unsafe { CStr::from_ptr((*ffi::git_error_last()).message) }
        .to_string_lossy()
        .into_owned()
}

unsafe extern "C" fn ignore(_: ffi::git_trace_level_t, _: *const c_char) {}

/// `git_trace_set(raw, cb)`.
fn trace_set(raw: ffi::git_trace_level_t, cb: ffi::git_trace_cb) -> i32 {
    // SAFETY: `cb` is NULL or a function that ignores its arguments, valid for the
    // process; nothing in this binary emits a trace message.
    unsafe { ffi::git_trace_set(raw, cb) }
}

#[test]
fn levels_carry_their_c_values() {
    for (c, level) in LEVELS {
        assert_eq!(level.raw(), c, "{level:?}");
        assert_eq!(ffi::git_trace_level_t::from(level), c, "{level:?}");
    }
}

#[test]
fn from_raw_names_exactly_the_c_levels() {
    for raw in probes() {
        let c = LEVELS.iter().find(|(c, _)| *c == raw).map(|&(_, l)| l);
        assert_eq!(TraceLevel::from_raw(raw), c, "{raw}");
        assert_eq!(
            TraceLevel::try_from(raw).map_err(UnnamedTraceLevel::raw),
            c.ok_or(raw),
            "{raw}"
        );
    }
}

#[test]
fn order_is_libgit2s_report_rule() {
    for (configured_raw, configured) in LEVELS {
        for (message_raw, message) in LEVELS {
            // `git_trace`: `git_trace__data.level >= level`.
            assert_eq!(
                configured >= message,
                configured_raw >= message_raw,
                "{configured:?} {message:?}"
            );
            assert_eq!(configured.cmp(&message), configured_raw.cmp(&message_raw));
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_trace_set_accepts_every_level_with_a_callback() {
    init();
    let _state = TRACE_STATE.lock().unwrap();
    for raw in probes() {
        assert_eq!(trace_set(raw, Some(ignore)), 0, "{raw}");
    }
    assert_eq!(trace_set(TraceLevel::None.raw(), None), 0);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_trace_set_needs_a_callback_above_none() {
    if std::env::var("CRUSTIFY_BUILD_VARIANT").as_deref() == Ok("coverage") {
        // `GIT_ASSERT_ARG` aborts in the debug build.
        return;
    }
    init();
    let _state = TRACE_STATE.lock().unwrap();
    for raw in probes() {
        let rc = trace_set(raw, None);
        if TraceLevel::from_raw(raw) == Some(TraceLevel::None) {
            assert_eq!(rc, 0, "{raw}");
        } else {
            assert_eq!(rc, -1, "{raw}");
            // The condition as the preprocessor expanded `NULL`.
            let message = last_message();
            assert!(
                message.starts_with("invalid argument: 'level == 0 || callback != "),
                "{raw}: {message}"
            );
        }
    }
    assert_eq!(trace_set(TraceLevel::None.raw(), None), 0);
}
