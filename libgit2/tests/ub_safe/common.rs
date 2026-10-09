//! UB tests for `git2/common.h`: the enums `Feature` and `BuildInfo`, and the
//! feature set `Features`.
//!
//! `Feature` and `BuildInfo` are fieldless enums over C's `git_feature_t` and
//! `git_buildinfo_t`. The only safe way in from a raw value is `from_raw` /
//! `try_from`, which must reject every value without a variant rather than produce an
//! invalid discriminant (which Miri and UBSan would flag on the next read); an `as`
//! cast to either does not compile (`compile_fail/`). `Features` keeps any `int`
//! C hands it, so its operations must stay in range for every bit pattern, the sign
//! bit included. They are plain values: nothing allocates, points or is freed, so
//! there is no lifetime for ASan or BorrowSanitizer to catch, and sharing them across
//! threads races nothing (TSan). None of them calls libgit2, so they also run under
//! Miri.
//!
//! Then the enum `LibraryOption`.
//!
//! `LibraryOption` is a fieldless enum over C's `git_libgit2_opt_t`. The only safe
//! ways in from a raw value are `from_raw` / `try_from` and, from the `int` that
//! `git_libgit2_opts` takes, `from_key`; each must reject every value without a
//! variant, negative keys included, rather than produce an invalid discriminant
//! (which Miri and UBSan would flag on the next read); an `as` cast to it does not
//! compile (`compile_fail/`). None of these calls libgit2, so they also run under
//! Miri.

use std::collections::HashSet;
use std::ffi::c_int;
use std::hint::black_box;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::core::common::{BuildInfo, Feature, Features, LibraryOption};

/// Every value around the declared ones, each single bit, and a coarse stride across
/// the whole type; BorrowSanitizer instruments each comparison, so the probe set stays
/// small.
fn probes() -> impl Iterator<Item = u32> {
    (0..=300)
        .chain((0..32).map(|k| 1u32 << k))
        .chain((0..=u32::MAX).step_by(1 << 26))
        .chain([u32::MAX - 1, u32::MAX])
}

/// Bit patterns for `Features`: each single bit, its complement, and the extremes.
fn bit_patterns() -> impl Iterator<Item = c_int> {
    (0..32)
        .flat_map(|k| [1 << k, !(1 << k)])
        .chain([0, -1, c_int::MIN, c_int::MAX, (1 << 13) - 1])
}

#[test]
fn feature_from_raw_yields_only_valid_features() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(feature) = Feature::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(feature.raw(), raw);
            assert!(!format!("{feature:?}").is_empty());
            seen.insert(feature);
        } else {
            assert_eq!(Feature::try_from(raw).map_err(|e| e.raw()), Err(raw));
        }
    }
    assert_eq!(seen.len(), Feature::ALL.len());
}

#[test]
fn build_info_from_raw_yields_only_valid_items() {
    let mut seen = HashSet::new();
    for raw in probes() {
        if let Some(info) = BuildInfo::from_raw(black_box(raw)) {
            assert_eq!(info.raw(), raw);
            assert!(!format!("{info:?}").is_empty());
            seen.insert(info);
        } else {
            assert_eq!(BuildInfo::try_from(raw).map_err(|e| e.raw()), Err(raw));
        }
    }
    assert_eq!(seen.len(), 2);
}

#[test]
fn option_niche_holds_for_every_value() {
    // `Option<Feature>` / `Option<BuildInfo>` store `None` in a value no variant uses:
    // a rejected value must never alias a variant, nor a variant read as `None`.
    for raw in probes() {
        let feature = black_box(Feature::from_raw(raw));
        assert_eq!(feature.map(Feature::raw).unwrap_or(raw), raw);
        let info = black_box(BuildInfo::from_raw(raw));
        assert_eq!(info.map(BuildInfo::raw).unwrap_or(raw), raw);
    }
}

#[test]
fn features_stay_in_range_for_every_bit_pattern() {
    for bits in bit_patterns() {
        let mut set = Features::from_bits_retain(black_box(bits));
        assert_eq!(set.bits(), bits);
        let named: Vec<_> = set.iter().collect();
        assert!(named.len() <= Feature::ALL.len());
        assert!(named.iter().all(|&f| set.contains(f)));
        assert_eq!(set.iter().count(), named.len());
        assert!(!format!("{set:?}").is_empty());
        for feature in Feature::ALL {
            set.insert(feature);
            assert!(set.contains(feature));
            set.remove(feature);
            assert!(!set.contains(feature));
        }
        assert_eq!(set.bits(), set.unnamed_bits());
        set |= Features::from_bits_retain(bits);
        assert_eq!(set.bits(), bits);
    }
}

#[test]
fn values_cross_threads() {
    // Plain values: `Send` and `Sync` without an assertion, so sharing them races
    // nothing for TSan to report.
    let set = Features::from_bits_retain(-1);
    let infos = [BuildInfo::Cpu, BuildInfo::Commit];
    thread::scope(|s| {
        let handles: Vec<_> = Feature::ALL
            .iter()
            .map(|feature| {
                let set = &set;
                let infos = &infos;
                s.spawn(move || {
                    (
                        feature.raw(),
                        set.contains(*feature),
                        infos.map(BuildInfo::raw),
                    )
                })
            })
            .collect();
        for (feature, h) in Feature::ALL.iter().zip(handles) {
            assert_eq!(h.join().unwrap(), (feature.raw(), true, [1, 2]));
        }
    });
}

/// Every value around the declared ones, a coarse stride across the whole type, and
/// the extremes; BorrowSanitizer instruments each comparison, so the probe set stays
/// small.
fn option_probes() -> impl Iterator<Item = ffi::git_libgit2_opt_t> {
    (0..=300)
        .chain((0..=ffi::git_libgit2_opt_t::MAX).step_by(1 << 26))
        .chain([ffi::git_libgit2_opt_t::MAX - 1, ffi::git_libgit2_opt_t::MAX])
}

#[test]
fn from_raw_yields_only_valid_options() {
    let mut seen = HashSet::new();
    for raw in option_probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(opt) = LibraryOption::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(opt.raw(), raw);
            assert_eq!(opt.key() as ffi::git_libgit2_opt_t, raw);
            assert!(!format!("{opt:?}").is_empty());
            seen.insert(opt);
        } else {
            assert_eq!(LibraryOption::try_from(raw).map_err(|e| e.raw()), Err(raw));
        }
    }
    assert_eq!(seen.len(), 48);
}

#[test]
fn from_key_yields_only_valid_options() {
    // Negative keys reinterpret as huge unsigned values; neither may alias a variant.
    let keys = (-300..=300).chain([c_int::MIN, c_int::MIN + 1, c_int::MAX - 1, c_int::MAX]);
    for key in keys {
        match LibraryOption::from_key(black_box(key)) {
            Some(opt) => {
                assert!(key >= 0);
                assert_eq!(opt.key(), key);
                assert_eq!(
                    LibraryOption::from_raw(key as ffi::git_libgit2_opt_t),
                    Some(opt)
                );
            }
            None => assert!(key < 0 || LibraryOption::from_raw(key as u32).is_none()),
        }
    }
}

#[test]
fn library_option_niche_holds_for_every_value() {
    // `Option<LibraryOption>` stores `None` in a value no variant uses: a rejected
    // value must never alias a variant, nor a variant read as `None`.
    for raw in option_probes() {
        let opt = black_box(LibraryOption::from_raw(raw));
        assert_eq!(opt.map(LibraryOption::raw).unwrap_or(raw), raw);
    }
}

#[test]
fn options_cross_threads() {
    // A plain value: `Send` and `Sync` without an assertion, so sharing one races
    // nothing for TSan to report.
    let opts: Vec<LibraryOption> = (0..48).filter_map(LibraryOption::from_raw).collect();
    thread::scope(|s| {
        let handles: Vec<_> = opts
            .iter()
            .map(|opt| s.spawn(move || LibraryOption::from_key(opt.key())))
            .collect();
        for (opt, h) in opts.iter().zip(handles) {
            assert_eq!(h.join().unwrap(), Some(*opt));
        }
    });
}
