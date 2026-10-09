//! Equivalence of `git2/common.h`'s [`Feature`], [`Features`] and [`BuildInfo`] with
//! the `git_feature_t` and `git_buildinfo_t` values libgit2 declares and accepts.
//!
//! [`Feature`] and [`BuildInfo`] must name exactly the values of their C enums, each
//! with its C value. libgit2 observes them through three routines: the bits
//! `git_libgit2_features` returns must read back unchanged through
//! [`Features::from_bits_retain`], with [`Features::contains`] and [`Features::iter`]
//! agreeing with C's bit tests and no bit left unnamed; `git_libgit2_feature_backend`
//! must answer NULL for every value [`Feature::from_raw`] refuses, a backend only for
//! a feature the set contains, and the same backend on every call; and
//! `git_libgit2_buildinfo` must answer NULL exactly for the values
//! [`BuildInfo::from_raw`] refuses. Every call is repeated and compared with the
//! first answer, since the routines keep no state.
//!
//! None of the three touches libgit2's runtime state (no allocation, no error) for
//! the values probed here, so they run without `git_libgit2_init`. The safe wrappers
//! of the routines themselves belong to the `core` symbol batches.

use std::collections::BTreeSet;
use std::ffi::{CStr, c_char, c_int};

use libgit2::ffi;
use libgit2::libgit2::core::common::{
    BuildInfo, Feature, Features, UnnamedBuildInfo, UnnamedFeature,
};

/// Every `git_feature_t` constant and the variant carrying it.
const FEATURES: [(ffi::git_feature_t, Feature); 13] = [
    (ffi::GIT_FEATURE_THREADS, Feature::Threads),
    (ffi::GIT_FEATURE_HTTPS, Feature::Https),
    (ffi::GIT_FEATURE_SSH, Feature::Ssh),
    (ffi::GIT_FEATURE_NSEC, Feature::Nsec),
    (ffi::GIT_FEATURE_HTTP_PARSER, Feature::HttpParser),
    (ffi::GIT_FEATURE_REGEX, Feature::Regex),
    (ffi::GIT_FEATURE_I18N, Feature::I18n),
    (ffi::GIT_FEATURE_AUTH_NTLM, Feature::AuthNtlm),
    (ffi::GIT_FEATURE_AUTH_NEGOTIATE, Feature::AuthNegotiate),
    (ffi::GIT_FEATURE_COMPRESSION, Feature::Compression),
    (ffi::GIT_FEATURE_SHA1, Feature::Sha1),
    (ffi::GIT_FEATURE_SHA256, Feature::Sha256),
    (ffi::GIT_FEATURE_HTTP, Feature::Http),
];

/// Every `git_buildinfo_t` constant and the variant carrying it.
const BUILD_INFOS: [(ffi::git_buildinfo_t, BuildInfo); 2] = [
    (ffi::GIT_BUILDINFO_CPU, BuildInfo::Cpu),
    (ffi::GIT_BUILDINFO_COMMIT, BuildInfo::Commit),
];

/// Every value up to past the highest feature bit, each single bit and its
/// neighbours, and a coarse stride across the whole type; BorrowSanitizer
/// instruments each FFI call and Miri interprets each step, so the dense range
/// shrinks under them.
fn probes() -> impl Iterator<Item = u32> {
    let dense = if cfg!(any(bsan, miri)) { 64 } else { 1 << 14 };
    (0..=dense)
        .chain((0..32).flat_map(|k| {
            let bit = 1u32 << k;
            [bit - 1, bit, bit + 1]
        }))
        .chain((0..=u32::MAX).step_by(1 << 26))
        .chain([u32::MAX - 1, u32::MAX])
}

fn c_features() -> c_int {
    // SAFETY: no arguments; returns a constant computed from the build's defines.
    unsafe { ffi::git_libgit2_features() }
}

/// `git_libgit2_feature_backend`'s answer, copied out; `None` for NULL.
fn c_backend(raw: ffi::git_feature_t) -> Option<String> {
    // SAFETY: takes any value by value; returns NULL or a static string. No probed
    // value reaches its assertions, which only an unknown backend of a compiled-in
    // feature does.
    let ptr = unsafe { ffi::git_libgit2_feature_backend(raw) };
    c_string(ptr)
}

/// `git_libgit2_buildinfo`'s answer, copied out; `None` for NULL.
fn c_buildinfo(raw: ffi::git_buildinfo_t) -> Option<String> {
    // SAFETY: takes any value by value; returns NULL or a static string.
    let ptr = unsafe { ffi::git_libgit2_buildinfo(raw) };
    c_string(ptr)
}

fn c_string(ptr: *const c_char) -> Option<String> {
    // SAFETY: both routines return NULL or a NUL-terminated string literal, which
    // lives for the whole program.
    (!ptr.is_null()).then(|| unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned())
}

#[test]
fn feature_from_raw_names_exactly_the_c_constants() {
    for (c, feature) in FEATURES {
        assert_eq!(feature.raw(), c, "{feature:?}");
        assert_eq!(ffi::git_feature_t::from(feature), c, "{feature:?}");
    }
    assert_eq!(Feature::ALL.map(Feature::raw), FEATURES.map(|(c, _)| c));
    for raw in probes() {
        let c = FEATURES.iter().find(|(c, _)| *c == raw).map(|&(_, f)| f);
        assert_eq!(Feature::from_raw(raw), c, "{raw}");
        assert_eq!(
            Feature::try_from(raw).map_err(UnnamedFeature::raw),
            c.ok_or(raw),
            "{raw}"
        );
    }
}

#[test]
fn build_info_from_raw_names_exactly_the_c_constants() {
    for (c, info) in BUILD_INFOS {
        assert_eq!(info.raw(), c, "{info:?}");
        assert_eq!(ffi::git_buildinfo_t::from(info), c, "{info:?}");
    }
    for raw in probes() {
        let c = BUILD_INFOS.iter().find(|(c, _)| *c == raw).map(|&(_, i)| i);
        assert_eq!(BuildInfo::from_raw(raw), c, "{raw}");
        assert_eq!(
            BuildInfo::try_from(raw).map_err(UnnamedBuildInfo::raw),
            c.ok_or(raw),
            "{raw}"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_libgit2_features_reads_back_through_features() {
    let c = c_features();
    let set = Features::from_bits_retain(c);
    assert_eq!(set.bits(), c);
    // The header names every bit this libgit2 sets.
    assert_eq!(set.unnamed_bits(), 0, "{set:?}");
    for (raw, feature) in FEATURES {
        assert_eq!(set.contains(feature), c & raw as c_int != 0, "{feature:?}");
    }
    let named: Vec<_> = FEATURES
        .iter()
        .filter(|&&(raw, _)| c & raw as c_int != 0)
        .map(|&(_, f)| f)
        .collect();
    assert_eq!(set.iter().collect::<Vec<_>>(), named);
    // Rebuilding the set from its features gives C's bits back.
    assert_eq!(set.iter().collect::<Features>().bits(), c);
    // The header's "always available" features.
    for feature in [
        Feature::HttpParser,
        Feature::Regex,
        Feature::Compression,
        Feature::Sha1,
        Feature::Sha256,
    ] {
        assert!(set.contains(feature), "{feature:?} missing from {set:?}");
    }
    // Repeated calls answer the same set.
    for _ in 0..4 {
        assert_eq!(Features::from_bits_retain(c_features()), set);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_libgit2_feature_backend_answers_only_named_features() {
    let set = Features::from_bits_retain(c_features());
    let mut backed = BTreeSet::new();
    for raw in probes() {
        let first = c_backend(raw);
        match Feature::from_raw(raw) {
            None => assert_eq!(first, None, "{raw}"),
            Some(feature) => {
                if let Some(backend) = &first {
                    assert!(!backend.is_empty(), "{feature:?}");
                    // A backend implies the feature is compiled in.
                    assert!(
                        set.contains(feature),
                        "{feature:?}: {backend} not in {set:?}"
                    );
                    backed.insert(feature);
                }
            }
        }
        assert_eq!(c_backend(raw), first, "{raw}");
    }
    // HTTP has no backend selection; every always-available feature has one.
    assert!(!backed.contains(&Feature::Http));
    for feature in [
        Feature::HttpParser,
        Feature::Regex,
        Feature::Compression,
        Feature::Sha1,
        Feature::Sha256,
    ] {
        assert!(backed.contains(&feature), "{feature:?}");
    }
    // Each single feature of the set, through the Rust value, answers as its raw bit.
    for feature in set {
        assert_eq!(c_backend(feature.raw()), c_backend(feature.raw()));
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_libgit2_buildinfo_answers_exactly_the_named_items() {
    for raw in probes() {
        let first = c_buildinfo(raw);
        match BuildInfo::from_raw(raw) {
            None => assert_eq!(first, None, "{raw}"),
            Some(BuildInfo::Cpu) => {
                // cmake always records the processor.
                assert!(first.as_deref().is_some_and(|s| !s.is_empty()), "{first:?}");
            }
            Some(BuildInfo::Commit) => {
                // Recorded when `git rev-parse HEAD` worked at configure time.
                if let Some(commit) = &first {
                    assert!(
                        matches!(commit.len(), 40 | 64)
                            && commit.bytes().all(|b| b.is_ascii_hexdigit()),
                        "{commit}"
                    );
                }
            }
            Some(other) => panic!("{other:?} has no C counterpart"),
        }
        assert_eq!(c_buildinfo(raw), first, "{raw}");
    }
}
