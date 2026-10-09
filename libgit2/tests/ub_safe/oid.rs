//! UB tests for `git2/oid.h`: the enum `OidType`.
//!
//! `OidType` is a fieldless enum over C's `git_oid_t`. The only safe way in from a
//! raw value is `from_raw` / `try_from`, which must reject every value without a
//! variant rather than produce an invalid discriminant (which Miri and UBSan would
//! flag on the next read); an `as` cast to it does not compile (`compile_fail/`). Its
//! sizes index a `git_oid`'s id, so they must stay within `GIT_OID_MAX_SIZE` and its
//! hex form. None of it calls libgit2, so these tests also run under Miri.

use std::collections::HashSet;
use std::hint::black_box;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::oid::oid::OidType;

/// Every value around the declared ones and `git_oid.type`'s byte boundary, then a
/// coarse stride across the whole type; BorrowSanitizer instruments each comparison,
/// so the probe set stays small.
fn probes() -> impl Iterator<Item = ffi::git_oid_t> {
    (0..=300)
        .chain((0..=ffi::git_oid_t::MAX).step_by(1 << 26))
        .chain([ffi::git_oid_t::MAX - 1, ffi::git_oid_t::MAX])
}

#[test]
fn from_raw_yields_only_valid_types() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(ty) = OidType::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(ty.raw(), raw);
            assert!(!format!("{ty:?}").is_empty());
            seen.insert(ty);
        } else {
            assert_eq!(OidType::try_from(raw).map_err(|e| e.raw()), Err(raw));
        }
    }
    assert_eq!(seen.len(), 2);
}

#[test]
fn option_niche_holds_for_every_value() {
    // `Option<OidType>` stores `None` in a value no variant uses: a rejected value
    // must never alias a variant, nor a variant read as `None`.
    for raw in probes() {
        let ty = black_box(OidType::from_raw(raw));
        assert_eq!(ty.map(OidType::raw).unwrap_or(raw), raw);
    }
}

#[test]
fn sizes_stay_within_a_git_oid() {
    let oid = ffi::git_oid::default();
    let mut hex = [0u8; 2 * 32];
    for raw in probes() {
        let Some(ty) = OidType::from_raw(black_box(raw)) else {
            continue;
        };
        // Slicing panics, rather than reading out of bounds, if a size overshoots.
        let id = &oid.id[..black_box(ty).size()];
        let digits = &mut hex[..black_box(ty).hex_size()];
        assert_eq!(digits.len(), 2 * id.len());
        digits.fill(b'0');
    }
}

#[test]
fn types_cross_threads() {
    // A plain value: `Send` and `Sync` without an assertion, so sharing one races
    // nothing for TSan to report.
    let types = [OidType::Sha1, OidType::Sha256, OidType::default()];
    thread::scope(|s| {
        let handles: Vec<_> = types
            .iter()
            .map(|ty| s.spawn(move || (ty.raw(), ty.size(), ty.hex_size())))
            .collect();
        for (ty, h) in types.iter().zip(handles) {
            assert_eq!(h.join().unwrap(), (ty.raw(), ty.size(), ty.hex_size()));
        }
    });
}
