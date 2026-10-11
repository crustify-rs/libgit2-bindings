//! Constructor/lifecycle equivalence. `git_pathspec_matches_path` (not yet wrapped)
//! observes independently compiled objects; list matching lives in `match_list`.
use core::ffi::CStr;
use core::ptr;
use libgit2::libgit2::core::strarray::StrArrayView;
use libgit2::libgit2::index::pathspec::{Pathspec, PathspecArc, PathspecFlags};
use libgit2_sys as ffi;

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
        let safe = safe_result.unwrap();
        drop(view);
        drop(ptrs);
        drop(safe_patterns);
        drop(raw_patterns);
        for pass in 0..2 {
            // Exercise every combination, including both case flags and an unknown
            // high bit; a second pass checks compiled state was not changed.
            for bits in 0..64 {
                let flags = PathspecFlags::from_bits_retain(bits | (pass << 31));
                for path in paths {
                    // SAFETY: both objects are live independent initialized owners;
                    // path is terminated. Matching reads without retaining pointers.
                    let (a, b) = unsafe {
                        (
                            ffi::git_pathspec_matches_path(
                                safe.as_c_ptr(),
                                flags.bits(),
                                path.as_ptr(),
                            ),
                            ffi::git_pathspec_matches_path(raw, bits | (pass << 31), path.as_ptr()),
                        )
                    };
                    assert_eq!(a, b, "patterns={patterns:?}, path={path:?}, bits={bits}");
                }
            }
        }
        let shared: PathspecArc = safe.into();
        // SAFETY: release exactly the independently allocated reference; no handles
        // to it remain. The Rust owner is a different allocation and stays live.
        unsafe { ffi::git_pathspec_free(raw) };
        // SAFETY: the Rust owner still keeps its own compiled patterns alive.
        let _ =
            unsafe { ffi::git_pathspec_matches_path(shared.as_c_ptr(), 0, c"src/a.rs".as_ptr()) };
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

mod match_list;
