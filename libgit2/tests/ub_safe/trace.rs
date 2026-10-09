//! UB tests for `git2/trace.h`: the enum `TraceLevel`.
//!
//! `TraceLevel` is a fieldless enum over C's `git_trace_level_t`. The only safe way in
//! from a raw value is `from_raw` / `try_from`, which must reject every value without
//! a variant rather than produce an invalid discriminant (which Miri and UBSan would
//! flag on the next read); an `as` cast to it does not compile (`compile_fail/`).
//! None of them calls libgit2, so they also run under Miri.

use std::collections::HashSet;
use std::hint::black_box;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::core::trace::TraceLevel;

/// Every value around the declared ones, then a coarse stride across the whole type;
/// BorrowSanitizer instruments each comparison, so the probe set stays small.
fn probes() -> impl Iterator<Item = ffi::git_trace_level_t> {
    (0..=64)
        .chain((0..=ffi::git_trace_level_t::MAX).step_by(1 << 26))
        .chain([ffi::git_trace_level_t::MAX - 1, ffi::git_trace_level_t::MAX])
}

#[test]
fn from_raw_yields_only_valid_levels() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(level) = TraceLevel::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(level.raw(), raw);
            assert!(!format!("{level:?}").is_empty());
            seen.insert(level);
        } else {
            assert_eq!(TraceLevel::try_from(raw).map_err(|e| e.raw()), Err(raw));
        }
    }
    assert_eq!(seen.len(), 7);
}

#[test]
fn levels_cross_threads() {
    let levels: Vec<TraceLevel> = (0..=6).filter_map(TraceLevel::from_raw).collect();
    let back = thread::spawn(move || {
        let mut sorted = levels;
        sorted.sort();
        sorted
    })
    .join()
    .unwrap();
    assert_eq!(back.first(), Some(&TraceLevel::None));
    assert_eq!(back.last(), Some(&TraceLevel::Trace));
}
