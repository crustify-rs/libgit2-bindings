//! UB tests for `src/libgit2/diff_parse.c`'s parse options, `DiffParseOptions`, and
//! [`Diff::from_buffer_with_options`].
//!
//! The options are plain values: built on any thread with no libgit2 initialised, handles
//! taken again and again and used between writes (`BSan`), copies moved to and read on
//! other threads at once (`TSan`), and every byte C reads written (`MSan`). Parses with
//! every id type and with versions libgit2 does not read hand back diffs that outlive
//! their input and options, and drop on other threads (`ASan`, `LSan`).
#![forbid(unsafe_code)]
use libgit2::libgit2::src::libgit2::diff::{Diff, DiffArc};
use libgit2::libgit2::src::libgit2::diff_parse::{DiffParseOptions, DiffParseOptionsRef};
use libgit2::libgit2::src::libgit2::oid::OidType;

/// A git patch with abbreviated ids.
const PATCH: &[u8] = b"diff --git a/file b/file\nindex 1111111..2222222 100644\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n";
/// A pure rename.
const RENAME: &[u8] =
    b"diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n";

const KINDS: [Option<OidType>; 3] = [None, Some(OidType::Sha1), Some(OidType::Sha256)];

/// Handles taken from one value over and over, a shared handle read between writes made
/// through fresh exclusive handles, and copies that outlive the handle they came from.
/// Needs no libgit2, so Miri runs it.
#[test]
fn handles_reborrow_and_copies_outlive_them() {
    let mut opts = DiffParseOptions::new();
    let mut copies = Vec::new();
    for (n, kind) in KINDS.into_iter().cycle().take(12).enumerate() {
        {
            let mut m = opts.as_mut();
            m.set_oid_type(kind);
            m.set_version(u32::try_from(n).unwrap());
            let shared: DiffParseOptionsRef<'_> = m.as_ref();
            assert_eq!(shared.oid_type(), kind);
        }
        let r = opts.as_ref();
        let r2 = r;
        assert_eq!(r.version(), r2.version());
        copies.push(r.to_options());
        copies.push(opts);
    }
    for pair in copies.chunks(2) {
        assert_eq!(pair[0], pair[1]);
    }
    assert_eq!(copies.last().unwrap().as_ref().version(), 11);
}

/// Options built on several threads at once before libgit2 is initialised, moved to
/// other threads and read there through shared handles at once.
#[test]
fn options_build_anywhere_and_cross_threads() {
    let built: Vec<DiffParseOptions> = (0..8)
        .map(|_| std::thread::spawn(DiffParseOptions::new))
        .map(|t| t.join().unwrap())
        .collect();
    for opts in &built {
        assert_eq!(*opts, DiffParseOptions::default());
    }
    let mut shared = built[0];
    shared.as_mut().set_oid_type(Some(OidType::Sha256));
    std::thread::scope(|s| {
        for _ in 0..4 {
            let r = shared.as_ref();
            s.spawn(move || {
                assert_eq!(r.oid_type(), Some(OidType::Sha256));
                assert!(format!("{r:?}").starts_with("DiffParseOptions { version: 1"));
            });
        }
    });
}

/// Every id type and versions libgit2 never reads parse; the diff outlives its input and
/// the options it was parsed with. Options cross to other threads, which parse with them at once
/// and drop their diffs there (`TSan`, `ASan`).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parses_outlive_options_and_input_across_threads() {
    let mut threads = Vec::new();
    for kind in KINDS {
        for version in [0, DiffParseOptions::VERSION, u32::MAX] {
            let mut opts = DiffParseOptions::new();
            opts.as_mut().set_oid_type(kind);
            opts.as_mut().set_version(version);
            threads.push(std::thread::spawn(move || {
                let input = [PATCH, RENAME].concat();
                let diff = Diff::from_buffer_with_options(&input, opts).unwrap();
                drop(input);
                assert_eq!(diff.as_ref().len(), 2);
                assert_eq!(
                    diff.as_ref().get(0).unwrap().new_file().id().kind(),
                    Some(kind.unwrap_or_default())
                );
                let shared: DiffArc<'static> = diff.into();
                assert_eq!(shared.as_ref().get(1).unwrap().similarity(), 100);
            }));
        }
    }
    for t in threads {
        t.join().unwrap();
    }
}

/// Failures leave nothing behind (`LSan`), whatever the options; the empty input is an
/// empty diff under every type.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn failures_and_empty_input_under_every_type() {
    for kind in KINDS {
        let mut opts = DiffParseOptions::new();
        opts.as_mut().set_oid_type(kind);
        assert!(
            Diff::from_buffer_with_options(b"", opts)
                .unwrap()
                .as_ref()
                .is_empty()
        );
        for input in [
            b"garbage".as_slice(),
            b"diff --git a/a b/a\n@@ broken\n",
            b"\0",
            &PATCH[..PATCH.len() - 5],
        ] {
            assert!(Diff::from_buffer_with_options(input, opts).is_err());
        }
    }
}
