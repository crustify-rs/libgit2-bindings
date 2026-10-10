//! Equivalence of `git2/commit.h`'s [`CommitArray`] with the `git_commitarray` C code
//! holds, has libgit2 fill and releases with `git_commitarray_dispose`.
//!
//! Each array is produced twice, independently, one copy per side, by
//! `git_repository_commit_parents` on its own open repository: `testrepo.git` (one
//! parent), `mergedrepo` (a merge in progress: `HEAD` and `MERGE_HEAD`) and
//! `empty_bare.git` (an unborn `HEAD`: `{NULL, 0}`). The Rust side adopts its copy as
//! libgit2 hands it over, as the `git_repository_commit_parents` wrapper will. Both
//! sides then read the array (count, and each commit's id), release it (C:
//! `git_commitarray_dispose`; Rust: [`CommitArrayMut::clear`]), have libgit2 fill it
//! again, read it after freeing the repository, and release it for good (C: dispose;
//! Rust: drop). After every step both record the struct's fields as the public API
//! exposes them and the results are compared.
//!
//! Documented divergences, outside these comparisons:
//!
//! - An empty array (`commits` NULL) never reaches libgit2 in Rust:
//!   `CommitArrayDispose` skips the C call, which would be `git__free(NULL)` reading
//!   the allocator unsynchronised against a concurrent first `git_libgit2_init`. Both
//!   sides end in `{NULL, 0}` either way.
//! - A handle over an array whose `commits` is NULL with a non-zero `count` (no
//!   libgit2 path produces one) reads as empty and clears to `{NULL, 0}` in Rust; C's
//!   dispose would dereference NULL. The module's `unit_tests` cover it.

use std::ffi::CStr;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::object::commit::{CommitArray, CommitArrayRef, CommitArrayVal};

const TESTREPO: &CStr = c"tests/resources/testrepo.git";
const MERGEDREPO: &CStr = c"tests/resources/mergedrepo/.gitted";
const EMPTY: &CStr = c"tests/resources/empty_bare.git";

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

/// Opens the fixture repository `name`, relative to the libgit2 checkout.
fn open(name: &CStr) -> *mut ffi::git_repository {
    init();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../");
    let path = std::ffi::CString::new(format!("{root}{}", name.to_str().unwrap())).unwrap();
    let mut repo = ptr::null_mut();
    // SAFETY: `repo` receives the repository; `path` is NUL-terminated; no ceiling
    // directories.
    let rc = unsafe {
        ffi::git_repository_open_ext(
            &mut repo,
            path.as_ptr(),
            ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
            ptr::null(),
        )
    };
    assert_eq!(rc, 0);
    repo
}

/// The hex id of the live commit `commit`.
fn hex(commit: *mut ffi::git_commit) -> String {
    let mut out = [0u8; 41];
    // SAFETY: `commit` is a live commit its array keeps alive; its id is formatted
    // into `out`, 41 writable bytes, NUL included.
    unsafe {
        ffi::git_oid_tostr(
            out.as_mut_ptr().cast(),
            out.len(),
            ffi::git_commit_id(commit),
        )
    };
    CStr::from_bytes_until_nul(&out)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

/// What one side observes of an array: `(count, commit ids)`.
type Record = (usize, Vec<String>);

/// The C side's view of `raw`: `count` and the id of each of its commits.
fn c_record(raw: &ffi::git_commitarray) -> Record {
    let ids = (0..raw.count)
        // SAFETY: libgit2 left `commits` holding `count` live commit pointers.
        .map(|i| hex(unsafe { *raw.commits.add(i) }))
        .collect();
    (raw.count, ids)
}

/// The Rust side's view, through the public API only.
fn rust_record(array: CommitArrayRef<'_>) -> Record {
    (array.len(), array.commit_ptrs().elems().map(hex).collect())
}

/// `git_repository_commit_parents` into `raw`, which must hold nothing.
fn c_fill(raw: &mut ffi::git_commitarray, repo: *mut ffi::git_repository) -> i32 {
    // SAFETY: `raw` is writable and empty, so nothing is overwritten unreleased;
    // `repo` is open.
    unsafe { ffi::git_repository_commit_parents(raw, repo) }
}

/// The same fill, adopted by the Rust side.
fn rust_fill(repo: *mut ffi::git_repository) -> (i32, CommitArrayVal) {
    let mut raw = ffi::git_commitarray::default();
    let rc = c_fill(&mut raw, repo);
    // SAFETY: `raw` is what libgit2 left: `{NULL, 0}`, or a buffer of commit references
    // from its allocator, given up here.
    (rc, unsafe { CommitArray::from_raw(raw) })
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_arrays_match() {
    // C's dispose of the empty array still reaches `git__free(NULL)`, which reads the
    // allocator a concurrent first `git_libgit2_init` writes: initialise first.
    init();
    let mut raw = ffi::git_commitarray::default();
    let mut array = CommitArray::new();
    assert_eq!(c_record(&raw), rust_record(array.as_ref()));
    // SAFETY: `raw` holds nothing; the dispose accepts the empty array.
    unsafe { ffi::git_commitarray_dispose(&mut raw) };
    array.as_mut().clear();
    assert_eq!(c_record(&raw), rust_record(array.as_ref()));
    let back = array.into_inner().into_raw();
    assert_eq!((raw.commits, raw.count), (back.commits, back.count));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn filled_arrays_match_through_their_lifecycle() {
    for (name, expected) in [(TESTREPO, 1), (MERGEDREPO, 2), (EMPTY, 0)] {
        let (c_repo, rust_repo) = (open(name), open(name));

        let mut raw = ffi::git_commitarray::default();
        let c_rc = c_fill(&mut raw, c_repo);
        let (rust_rc, mut array) = rust_fill(rust_repo);
        assert_eq!(c_rc, rust_rc, "{name:?}: fill");
        let filled = c_record(&raw);
        assert_eq!(filled.0, expected, "{name:?}");
        assert_eq!(filled, rust_record(array.as_ref()), "{name:?}: fill");
        assert_eq!(
            raw.commits.is_null(),
            array.as_ref().commit_ptrs().is_empty()
        );

        // SAFETY: `raw` owns what libgit2 filled it with, released once.
        unsafe { ffi::git_commitarray_dispose(&mut raw) };
        array.as_mut().clear();
        assert_eq!(
            c_record(&raw),
            rust_record(array.as_ref()),
            "{name:?}: release"
        );
        assert!(raw.commits.is_null());

        // Filled again: the C side reuses its struct; the Rust side hands its cleared
        // value back to C, as the wrapper's out-parameter does.
        let c_rc = c_fill(&mut raw, c_repo);
        let mut cleared = array.into_inner().into_raw();
        let rust_rc = c_fill(&mut cleared, rust_repo);
        // SAFETY: as in `rust_fill`.
        let array = unsafe { CommitArray::from_raw(cleared) };
        assert_eq!(c_rc, rust_rc, "{name:?}: refill");
        assert_eq!(c_record(&raw), filled);
        assert_eq!(rust_record(array.as_ref()), filled, "{name:?}: refill");

        // The commits outlive their repositories.
        for repo in [c_repo, rust_repo] {
            // SAFETY: each repository is open and freed once.
            unsafe { ffi::git_repository_free(repo) };
        }
        assert_eq!(
            c_record(&raw),
            rust_record(array.as_ref()),
            "{name:?}: orphaned"
        );

        // SAFETY: `raw` owns its refill, released once.
        unsafe { ffi::git_commitarray_dispose(&mut raw) };
        drop(array);
        assert_eq!(c_record(&raw), (0, Vec::new()));
    }
}
