//! Constructor, lifecycle and path-matching equivalence on independently compiled
//! objects: the safe `matches_path` against `git_pathspec_matches_path` on a raw
//! pathspec. List matching lives in `match_list`.
use core::ffi::CStr;
use core::ptr;
use libgit2::libgit2::core::strarray::StrArrayView;
use libgit2::libgit2::index::index::Index;
use libgit2::libgit2::index::pathspec::{MatchList, Pathspec, PathspecArc, PathspecFlags};
use libgit2_sys as ffi;

/// The C result for one path, which is only ever 0 or 1 for non-null arguments.
fn raw_matches(raw: *const ffi::git_pathspec, flags: PathspecFlags, path: &CStr) -> bool {
    // SAFETY: callers pass a live, initialized pathspec they own; `path` is
    // NUL-terminated. C reads both and retains neither.
    let rc = unsafe { ffi::git_pathspec_matches_path(raw, flags.bits(), path.as_ptr()) };
    assert!(rc == 0 || rc == 1, "unexpected matches_path result {rc}");
    rc == 1
}

/// Flags covering every bit `git_pathspec_matches_path` consults and none it ignores.
const CONSULTED: PathspecFlags = PathspecFlags::from_bits_retain(
    PathspecFlags::NO_GLOB.bits() | PathspecFlags::IGNORE_CASE.bits(),
);

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn construction_copy_and_release_sequences() {
    let cases: &[&[&CStr]] = &[
        &[],
        &[c""],
        &[c"", c""],
        &[c"#comment"],
        &[c"!"],
        &[c"\\"],
        &[c"src/*.rs", c"docs", c"a\\*"],
        &[c"!src/skip.rs", c"src/*", c"dir/", c" leading space"],
        &[c"foo/bar", c"foo/baz"],
        &[c"\\*escaped", c"\xff*"],
        &[c"*"],
        &[c"**/*.rs", c"[a-z]?.txt"],
        // The parser drops both (nothing is left once the slash is stripped).
        &[c"/"],
        &[c"!/", c"*"],
        &[c"Docs", c"!DOCS/skip"],
    ];
    let paths = [
        c"",
        c"src/a.rs",
        c"SRC/A.RS",
        c"src/skip.rs",
        c"!src/skip.rs",
        c"src/*.rs",
        c"docs",
        c"docs/sub/file",
        c"a*",
        c"nomatch",
        c"foo/bar",
        c"foo/baz",
        c"foo/bars",
        c"*escaped",
        c"\xffname",
        c" leading space",
        c"dir/file",
        c"ab.txt",
        c"/",
        c"/abs",
        c"!",
        c"!/x",
        c"DOCS/x",
        c"Docs/skip",
        c"dir",
    ];
    for patterns in cases {
        let safe_patterns: Vec<_> = patterns.iter().map(|s| (*s).to_owned()).collect();
        let raw_patterns: Vec<_> = patterns.iter().map(|s| (*s).to_owned()).collect();
        let view = StrArrayView::new(&safe_patterns);
        let safe_result = Pathspec::new(view.as_ref()); // also pins runtime
        let mut ptrs: Vec<_> = raw_patterns.iter().map(|s| s.as_ptr().cast_mut()).collect();
        let input = ffi::git_strarray {
            strings: ptrs.as_mut_ptr(),
            count: ptrs.len(),
        };
        let mut raw = ptr::null_mut();
        // SAFETY: initialized runtime; valid input pointers only read during the
        // call, and writable output slot. This result owns a separate C allocation.
        let rc = unsafe { ffi::git_pathspec_new(&mut raw, &input) };
        assert_eq!(safe_result.is_ok(), rc == 0);
        let mut safe = safe_result.unwrap();
        drop(view);
        drop(ptrs);
        drop(safe_patterns);
        drop(raw_patterns);
        let mut expected = Vec::new();
        for pass in 0..2 {
            // Exercise every combination, including both case flags and an unknown
            // high bit; a second pass, through an exclusive reborrow, checks compiled
            // state was not changed by the first.
            for bits in 0..64 {
                let flags = PathspecFlags::from_bits_retain(bits | (pass << 31));
                for path in paths {
                    let safe_result = if pass == 0 {
                        safe.as_ref().matches_path(flags, path)
                    } else {
                        safe.as_mut().as_ref().matches_path(flags, path)
                    };
                    let raw_result = raw_matches(raw, flags, path);
                    assert_eq!(
                        safe_result, raw_result,
                        "patterns={patterns:?}, path={path:?}, bits={bits}"
                    );
                    // The documented contract: no other bit changes the result.
                    assert_eq!(
                        safe_result,
                        safe.as_ref().matches_path(flags & CONSULTED, path)
                    );
                    if pass == 0 && bits < 4 {
                        expected.push((flags, path, raw_result));
                    }
                }
            }
        }
        let shared: PathspecArc = safe.into();
        // SAFETY: release exactly the independently allocated reference; no handles
        // to it remain. The Rust owner is a different allocation and stays live.
        unsafe { ffi::git_pathspec_free(raw) };
        // Releasing the C object leaves the Rust owner's compiled patterns intact.
        for (flags, path, raw_result) in expected {
            assert_eq!(shared.as_ref().matches_path(flags, path), raw_result);
        }
    }
}

#[test]
fn flag_values_match_public_c_constants() {
    for (safe, raw) in [
        (PathspecFlags::DEFAULT, ffi::GIT_PATHSPEC_DEFAULT),
        (PathspecFlags::IGNORE_CASE, ffi::GIT_PATHSPEC_IGNORE_CASE),
        (PathspecFlags::USE_CASE, ffi::GIT_PATHSPEC_USE_CASE),
        (PathspecFlags::NO_GLOB, ffi::GIT_PATHSPEC_NO_GLOB),
        (
            PathspecFlags::NO_MATCH_ERROR,
            ffi::GIT_PATHSPEC_NO_MATCH_ERROR,
        ),
        (
            PathspecFlags::FIND_FAILURES,
            ffi::GIT_PATHSPEC_FIND_FAILURES,
        ),
        (
            PathspecFlags::FAILURES_ONLY,
            ffi::GIT_PATHSPEC_FAILURES_ONLY,
        ),
    ] {
        assert_eq!(safe.bits(), raw);
    }
}

/// The parser strips a leading slash (and a `!`), leaving `"/"` and `"!/"` nothing to
/// compile: like an empty pathspec, they match every path in either case mode, on both
/// sides. Beside them, `"/x"` is anchored and `"x/"` matches only below `x`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn slash_only_patterns_compile_to_nothing() {
    for (pattern, path, expected) in [
        (c"/", c"/abs", true),
        (c"/", c"abs", true),
        (c"/", c"", true),
        (c"!/", c"!", true),
        (c"!/", c"abs", true),
        (c"/x", c"x", true),
        (c"/x", c"y/x", false),
        (c"x/", c"x/y", true),
        (c"x/", c"xy", false),
    ] {
        let patterns = [pattern];
        let view = StrArrayView::new(&patterns);
        let safe = Pathspec::new(view.as_ref()).unwrap();
        let mut raw = ptr::null_mut();
        let mut strings = [pattern.as_ptr().cast_mut()];
        let input = ffi::git_strarray {
            strings: strings.as_mut_ptr(),
            count: 1,
        };
        // SAFETY: initialized runtime; the input is read during the call only; the
        // output slot is writable and receives an independent pathspec.
        assert_eq!(unsafe { ffi::git_pathspec_new(&mut raw, &input) }, 0);
        for flags in [PathspecFlags::DEFAULT, PathspecFlags::IGNORE_CASE] {
            let safe_result = safe.as_ref().matches_path(flags, path);
            assert_eq!(safe_result, raw_matches(raw, flags, path));
            assert_eq!(safe_result, expected, "{pattern:?} {path:?} {flags:?}");
        }
        // SAFETY: the independent pathspec's only reference, released once.
        unsafe { ffi::git_pathspec_free(raw) };
    }
}

/// Matching through each step of a lifecycle in which a match list retains, then
/// releases, a second reference: the results never change, on either side.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn matching_across_retaining_list_sequence() {
    let patterns = [c"src/*.rs", c"!src/skip.rs", c"docs", c"[A-Z]*.md"];
    let paths = [
        c"src/a.rs",
        c"src/skip.rs",
        c"SRC/b.RS",
        c"docs/x/y",
        c"docsx",
        c"README.md",
        c"readme.md",
        c"",
    ];
    let flag_sets = [
        PathspecFlags::DEFAULT,
        PathspecFlags::IGNORE_CASE,
        PathspecFlags::NO_GLOB,
        PathspecFlags::USE_CASE | PathspecFlags::IGNORE_CASE | PathspecFlags::NO_GLOB,
        PathspecFlags::FIND_FAILURES | PathspecFlags::FAILURES_ONLY,
    ];
    let view = StrArrayView::new(&patterns);
    let safe: PathspecArc = Pathspec::new(view.as_ref()).unwrap().into();
    let mut ptrs: Vec<_> = patterns.iter().map(|s| s.as_ptr().cast_mut()).collect();
    let input = ffi::git_strarray {
        strings: ptrs.as_mut_ptr(),
        count: ptrs.len(),
    };
    let mut raw = ptr::null_mut();
    // SAFETY: initialized runtime (the safe constructor ran); valid input pointers
    // only read during the call; writable output slot for an independent pathspec.
    assert_eq!(unsafe { ffi::git_pathspec_new(&mut raw, &input) }, 0);
    let compare = |step: &str| {
        for flags in flag_sets {
            for path in paths {
                assert_eq!(
                    safe.as_ref().matches_path(flags, path),
                    raw_matches(raw, flags, path),
                    "{step}: path={path:?}, flags={flags:?}"
                );
            }
        }
    };
    compare("fresh");
    let mut safe_index = Index::new().unwrap();
    let raw_index = Index::new().unwrap();
    let safe_list =
        MatchList::from_index(safe_index.as_mut(), &safe, PathspecFlags::FIND_FAILURES).unwrap();
    let mut raw_list = ptr::null_mut();
    // SAFETY: an independent live index and the raw pathspec; writable out slot. The
    // list takes its own counted reference to `raw`.
    let rc = unsafe {
        ffi::git_pathspec_match_index(
            &mut raw_list,
            raw_index.as_c_ptr(),
            PathspecFlags::FIND_FAILURES.bits(),
            raw,
        )
    };
    assert_eq!(rc, 0);
    compare("retained by a list");
    drop(safe_list);
    // SAFETY: the fresh raw list is released once; `raw` keeps its own reference.
    unsafe { ffi::git_pathspec_match_list_free(raw_list) };
    compare("list released");
    drop(view);
    drop(ptrs);
    compare("inputs released");
    // SAFETY: the last reference to the raw pathspec; no list retains it any more.
    unsafe { ffi::git_pathspec_free(raw) };
}

mod match_list;
