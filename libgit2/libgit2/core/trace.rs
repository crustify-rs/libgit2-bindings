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

    #[test]
    fn layout_is_the_c_value() {
        assert_eq!(size_of::<TraceLevel>(), size_of::<ffi::git_trace_level_t>());
        assert_eq!(
            size_of::<Option<TraceLevel>>(),
            size_of::<ffi::git_trace_level_t>()
        );
    }
}
