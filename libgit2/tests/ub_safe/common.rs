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

use std::collections::HashSet;
use std::ffi::c_int;
use std::hint::black_box;
use std::thread;

use libgit2::libgit2::core::common::{BuildInfo, Feature, Features};

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
