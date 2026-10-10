//! UB tests for `git2/tree.h`: the enum `WalkMode` (`git_treewalk_mode`, unsigned).
//!
//! `WalkMode` is a fieldless enum over C's `git_treewalk_mode`. The only safe way in
//! from a raw value is `from_raw` / `try_from`, which must reject every value without
//! a variant rather than produce an invalid discriminant (which Miri and UBSan would
//! flag on the next read); an `as` cast to it does not compile (`compile_fail/`). It
//! owns and points at nothing and none of these tests calls libgit2, so they also run
//! under Miri; the sweep strides under BSan and Miri.

use std::collections::HashSet;
use std::hint::black_box;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::object::tree::{UnnamedWalkMode, WalkMode};

/// Every small value, a stride across the rest of `git_treewalk_mode`, and its ends.
fn probes() -> impl Iterator<Item = ffi::git_treewalk_mode> {
    let stride = if cfg!(bsan) || cfg!(miri) { 97 } else { 1 };
    (0..=4096)
        .step_by(stride)
        .chain([0, 1, 2])
        .chain((0..=ffi::git_treewalk_mode::MAX).step_by(1 << 24))
        .chain([ffi::git_treewalk_mode::MAX - 1, ffi::git_treewalk_mode::MAX])
}

#[test]
fn from_raw_yields_only_valid_modes() {
    let mut seen = HashSet::new();
    for raw in probes() {
        match WalkMode::from_raw(black_box(raw)) {
            Some(mode) => {
                // Matching on the value reads its discriminant.
                let back = match black_box(mode) {
                    WalkMode::PreOrder => ffi::GIT_TREEWALK_PRE,
                    WalkMode::PostOrder => ffi::GIT_TREEWALK_POST,
                    _ => unreachable!(),
                };
                assert_eq!(back, raw);
                assert_eq!(WalkMode::try_from(raw), Ok(mode));
                assert!(!format!("{mode:?}").is_empty());
                seen.insert(mode);
            }
            None => {
                let err = WalkMode::try_from(raw).unwrap_err();
                assert_eq!(err.raw(), raw);
                assert!(!err.to_string().is_empty());
            }
        }
    }
    assert_eq!(seen.len(), 2);
}

#[test]
fn option_niche_holds() {
    for raw in probes() {
        let mode = black_box(WalkMode::from_raw(raw));
        assert_eq!(mode.map(WalkMode::raw).unwrap_or(raw), raw);
        let result = black_box(WalkMode::try_from(raw));
        assert_eq!(
            result
                .map(WalkMode::raw)
                .unwrap_or_else(UnnamedWalkMode::raw),
            raw
        );
    }
}

#[test]
fn conversions_agree_across_threads() {
    thread::scope(|s| {
        let handles: Vec<_> = [WalkMode::PreOrder, WalkMode::PostOrder]
            .into_iter()
            .chain((0..4).filter_map(WalkMode::from_raw))
            .map(|mode| s.spawn(move || (mode, WalkMode::from_raw(mode.raw()))))
            .collect();
        for h in handles {
            let (mode, back) = h.join().unwrap();
            assert_eq!(back, Some(mode));
        }
        let unnamed = s.spawn(|| WalkMode::try_from(2).unwrap_err());
        assert_eq!(unnamed.join().unwrap().raw(), 2);
    });
}
