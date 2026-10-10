//! UB tests for `git2/sys/path.h`: the enum `GitFile`.
//!
//! `GitFile` is a fieldless enum over C's `git_path_gitfile`, whose raw value libgit2
//! uses as an index into a table of three names. The only safe way in from a raw value
//! is `from_raw` / `try_from`, which must reject every value without a variant rather
//! than produce an invalid discriminant (which Miri and UBSan would flag on the next
//! read); an `as` cast to it does not compile (`compile_fail/`). None of them calls
//! libgit2, so they also run under Miri.

use std::collections::HashSet;
use std::hint::black_box;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::repository::sys::path::GitFile;

/// Every value around the declared ones, then a coarse stride across the whole type.
fn probes() -> impl Iterator<Item = ffi::git_path_gitfile> {
    (0..=300)
        .chain((0..=ffi::git_path_gitfile::MAX).step_by(1 << 26))
        .chain([ffi::git_path_gitfile::MAX - 1, ffi::git_path_gitfile::MAX])
}

#[test]
fn from_raw_yields_only_valid_files() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(file) = GitFile::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(file.raw(), raw);
            assert_eq!(ffi::git_path_gitfile::from(file), raw);
            assert!(!format!("{file:?}").is_empty());
            seen.insert(file);
        } else {
            let err = GitFile::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert!(!err.to_string().is_empty());
        }
    }
    assert_eq!(seen.len(), 3);
}

#[test]
fn option_niche_holds_for_every_value() {
    for raw in probes() {
        let file = black_box(GitFile::from_raw(raw));
        assert_eq!(file.is_some(), raw < 3);
        assert_eq!(file.map(GitFile::raw), file.is_some().then_some(raw));
    }
}

#[test]
fn files_cross_threads() {
    let files = [
        GitFile::Gitignore,
        GitFile::Gitmodules,
        GitFile::Gitattributes,
    ];
    let handles: Vec<_> = files
        .into_iter()
        .map(|f| thread::spawn(move || GitFile::from_raw(f.raw())))
        .collect();
    for (h, f) in handles.into_iter().zip(files) {
        assert_eq!(h.join().unwrap(), Some(f));
    }
}
