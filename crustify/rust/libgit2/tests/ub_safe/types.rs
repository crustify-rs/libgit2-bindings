//! UB tests for `git2/types.h`: the enums `SubmoduleRecurse` and `SubmoduleUpdate`.
//!
//! Both are fieldless enums over C's `git_submodule_recurse_t` /
//! `git_submodule_update_t`. The only safe way in from a raw value is `from_raw` /
//! `try_from`, which must reject every value without a variant rather than produce
//! an invalid discriminant (which Miri and UBSan would flag on the next read); an `as`
//! cast to either does not compile (`compile_fail/`). None of them calls libgit2, so
//! they also run under Miri.

use std::collections::HashSet;
use std::hint::black_box;
use std::thread;

use libgit2::libgit2::core::types::{SubmoduleRecurse, SubmoduleUpdate};

/// Every value around the declared ones, then a coarse stride across the whole
/// type; BorrowSanitizer instruments each comparison, so the probe set stays small.
fn probes() -> impl Iterator<Item = u32> {
    (0..=300).chain((0..=u32::MAX).step_by(1 << 26)).chain([
        i32::MAX as u32,
        u32::MAX - 1,
        u32::MAX,
    ])
}

#[test]
fn from_raw_yields_only_valid_variants() {
    let mut recurse = HashSet::new();
    let mut update = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        match SubmoduleRecurse::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            Some(v) => {
                assert_eq!(v.raw(), raw);
                assert!(!format!("{v:?}").is_empty());
                recurse.insert(v);
            }
            None => assert_eq!(
                SubmoduleRecurse::try_from(raw).map_err(|e| e.raw()),
                Err(raw)
            ),
        }
        match SubmoduleUpdate::from_raw(black_box(raw)) {
            Some(v) => {
                assert_eq!(v.raw(), raw);
                assert!(!format!("{v:?}").is_empty());
                update.insert(v);
            }
            None => assert_eq!(
                SubmoduleUpdate::try_from(raw).map_err(|e| e.raw()),
                Err(raw)
            ),
        }
    }
    assert_eq!(recurse.len(), 3);
    assert_eq!(update.len(), 4);
}

#[test]
fn option_niche_holds_for_every_value() {
    // `Option<_>` stores `None` in a value no variant uses: a rejected value must
    // never alias a variant, nor a variant read as `None`.
    for raw in probes() {
        let r = black_box(SubmoduleRecurse::from_raw(raw));
        assert_eq!(r.map(SubmoduleRecurse::raw).unwrap_or(raw), raw);
        let u = black_box(SubmoduleUpdate::from_raw(raw));
        assert_eq!(u.map(SubmoduleUpdate::raw).unwrap_or(raw), raw);
    }
}

#[test]
fn defaults_are_valid_variants() {
    let r = black_box(SubmoduleRecurse::default());
    assert_eq!(SubmoduleRecurse::from_raw(r.raw()), Some(r));
    let u = black_box(SubmoduleUpdate::default());
    assert_eq!(SubmoduleUpdate::from_raw(u.raw()), Some(u));
    // The header's "default" update value has no variant; resolving it lands on one.
    assert_eq!(
        SubmoduleUpdate::from_raw(black_box(0)).unwrap_or_default(),
        SubmoduleUpdate::Checkout
    );
}

#[test]
fn values_cross_threads() {
    // Plain values: `Send` and `Sync` without an assertion, so sharing them races
    // nothing for TSan to report.
    let recurse = [
        SubmoduleRecurse::No,
        SubmoduleRecurse::Yes,
        SubmoduleRecurse::OnDemand,
    ];
    let update = [
        SubmoduleUpdate::Checkout,
        SubmoduleUpdate::Rebase,
        SubmoduleUpdate::Merge,
        SubmoduleUpdate::None,
    ];
    thread::scope(|s| {
        let r: Vec<_> = recurse.iter().map(|v| s.spawn(move || v.raw())).collect();
        let u: Vec<_> = update.iter().map(|v| s.spawn(move || v.raw())).collect();
        for (v, h) in recurse.iter().zip(r) {
            assert_eq!(h.join().unwrap(), v.raw());
        }
        for (v, h) in update.iter().zip(u) {
            assert_eq!(h.join().unwrap(), v.raw());
        }
    });
}
