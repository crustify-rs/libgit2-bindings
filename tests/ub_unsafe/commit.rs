//! UB tests for the unsafe API of `git2/commit.h`.
//!
//! `CommitArray::from_raw`, called within its contract on every shape of
//! `git_commitarray` libgit2 leaves behind or C code builds, edge cases included, and
//! then driven through the safe API: read, emptied, cleared, handed back with
//! `into_raw` and adopted again, and released before its repository, as the contract
//! requires (ASan: double free, invalid free, use after free, overflow; LSan: leak).
//!
//! The signing callback of `CreateExtOptions` is set safely
//! (`CreateExtOptionsMut::set_signer`); its tests are in `ub_safe`.

use std::ffi::CStr;
use std::mem::size_of;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::object::commit::{CommitArray, CommitArrayRef, CommitArrayVal, CommitRef};

const HEAD: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
const MERGED: [&str; 2] = [
    "3a34580a35add43a4cf361e8e9a30060a905c876",
    "e2809157a7766f272e4cfe26e61ef2678a5357ff",
];

/// Opens the fixture repository `name` under `tests/resources`.
fn open(name: &str) -> *mut ffi::git_repository {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock. The
    // reference dies with the process.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
    let path = std::ffi::CString::new(format!(
        "{}/c/tests/resources/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut repo = ptr::null_mut();
    // SAFETY: a live slot and a NUL-terminated path; no ceiling directories.
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

fn free_repo(repo: *mut ffi::git_repository) {
    // SAFETY: the tests free each repository they open once.
    unsafe { ffi::git_repository_free(repo) };
}

fn hex(commit: CommitRef<'_, '_>) -> String {
    let commit = commit.to_commit();
    let mut out = [0u8; 41];
    // SAFETY: a commit its owner keeps alive; 41 writable bytes.
    unsafe {
        ffi::git_oid_tostr(
            out.as_mut_ptr().cast(),
            out.len(),
            ffi::git_commit_id(commit.as_c_ptr()),
        )
    };
    CStr::from_bytes_until_nul(&out)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

fn ids(array: CommitArrayRef<'_, '_>) -> Vec<String> {
    array.iter().map(hex).collect()
}

/// `git_repository_commit_parents` into a zeroed struct.
fn parents(repo: *mut ffi::git_repository) -> ffi::git_commitarray {
    let mut raw = ffi::git_commitarray::default();
    // SAFETY: a zeroed array and an open repository.
    let rc = unsafe { ffi::git_repository_commit_parents(&mut raw, repo) };
    assert_eq!(rc, 0);
    raw
}

/// A new reference to the commit `hex` in `repo`.
fn lookup(repo: *mut ffi::git_repository, hex: &CStr) -> *mut ffi::git_commit {
    let mut id = ffi::git_oid::default();
    // SAFETY: a writable oid and a NUL-terminated hex string.
    let rc = unsafe { ffi::git_oid_from_string(&mut id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
    assert_eq!(rc, 0);
    let mut commit = ptr::null_mut();
    // SAFETY: a live slot, an open repository and a live oid.
    assert_eq!(unsafe { ffi::git_commit_lookup(&mut commit, repo, &id) }, 0);
    commit
}

/// A buffer of libgit2's allocator for `slots` commit pointers, the first `commits`
/// filled from `commits`, the rest NULL: as C code builds an array (with spare
/// capacity when `slots` is larger).
fn c_built(commits: &[*mut ffi::git_commit], slots: usize) -> ffi::git_commitarray {
    assert!(slots >= commits.len() && slots > 0);
    // SAFETY: the routine ignores `backend`; the runtime is initialised.
    let buf = unsafe {
        ffi::git_odb_backend_data_alloc(ptr::null_mut(), slots * size_of::<*mut ffi::git_commit>())
    }
    .cast::<*mut ffi::git_commit>();
    assert!(!buf.is_null());
    for i in 0..slots {
        let commit = commits.get(i).copied().unwrap_or(ptr::null_mut());
        // SAFETY: `buf` holds `slots` pointer slots, suitably aligned (libgit2's
        // allocator is malloc).
        unsafe { buf.add(i).write(commit) };
    }
    ffi::git_commitarray {
        commits: buf,
        count: commits.len(),
    }
}

/// Every libgit2 result, adopted: read in bounds, cleared, filled again through
/// `into_raw`, adopted again, emptied with `take` and dropped before the repository.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopts_what_libgit2_fills() {
    for (name, expected) in [
        ("testrepo.git", &[HEAD][..]),
        ("mergedrepo/.gitted", &MERGED[..]),
        ("empty_bare.git", &[][..]),
    ] {
        let repo = open(name);
        // SAFETY: what libgit2 left, given up here: `{NULL, 0}`, or a buffer of
        // exactly `count` references to commits (these repositories' `HEAD`s and
        // `MERGE_HEAD`s are commits) from its allocator, released before `repo`.
        let mut array = unsafe { CommitArray::from_raw(parents(repo)) };
        assert_eq!(ids(array.as_ref()), expected, "{name}");
        assert!(array.as_ref().get(expected.len()).is_none());
        array.as_mut().clear();
        assert!(array.as_ref().is_empty());

        let mut raw = array.into_inner().into_raw();
        assert!(raw.commits.is_null() && raw.count == 0);
        // SAFETY: `raw` is empty, so nothing is overwritten unreleased.
        let rc = unsafe { ffi::git_repository_commit_parents(&mut raw, repo) };
        assert_eq!(rc, 0);
        // SAFETY: as above: libgit2 refilled it.
        let mut again: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(raw) };
        assert_eq!(ids(again.as_ref()), expected, "{name}: refilled");
        let taken = again.as_mut().take();
        assert_eq!(taken.len(), expected.len());
        assert!(again.as_ref().is_empty());
        drop(again);
        drop(taken);
        free_repo(repo);
    }
}

/// Arrays C code builds over libgit2's allocator, with spare capacity, with a buffer
/// but no commits, and handed back to C's dispose through `into_raw`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopts_what_c_code_builds() {
    let repo = open("mergedrepo/.gitted");
    let commits = [
        lookup(repo, c"3a34580a35add43a4cf361e8e9a30060a905c876"),
        lookup(repo, c"e2809157a7766f272e4cfe26e61ef2678a5357ff"),
        lookup(repo, c"3a34580a35add43a4cf361e8e9a30060a905c876"),
    ];
    // SAFETY: a libgit2 buffer of 5 slots owning the 3 commit references looked up
    // above, given up here, released before `repo`; the 2 spare slots are never read
    // or released.
    let array = unsafe { CommitArray::from_raw(c_built(&commits, 5)) };
    assert_eq!(ids(array.as_ref()), [MERGED[0], MERGED[1], MERGED[0]]);
    assert!(array.as_ref().get(3).is_none());
    drop(array);

    // The same shape, emptied with `take`: the spare slots go with the buffer.
    let commits = [lookup(repo, c"e2809157a7766f272e4cfe26e61ef2678a5357ff")];
    // SAFETY: as above, one reference in 4 slots.
    let mut array = unsafe { CommitArray::from_raw(c_built(&commits, 4)) };
    let taken = array.as_mut().take();
    assert_eq!(hex(taken[0].as_ref()), MERGED[1]);
    drop((array, taken));

    // SAFETY: a libgit2 buffer owning no commits, given up here.
    let mut empty_buffer: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(c_built(&[], 2)) };
    assert!(empty_buffer.as_ref().is_empty());
    empty_buffer.as_mut().clear();
    // SAFETY: a second such buffer, released by the drop.
    drop(unsafe { CommitArray::<'_>::from_raw(c_built(&[], 1)) });
    // SAFETY: a third, emptied with `take`, which frees the buffer.
    let mut taken: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(c_built(&[], 3)) };
    assert!(taken.as_mut().take().is_empty());

    let one = [lookup(repo, c"e2809157a7766f272e4cfe26e61ef2678a5357ff")];
    // SAFETY: a libgit2 buffer owning one commit reference, given up here; it leaves
    // through `into_raw` before `repo` is freed.
    let adopted: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(c_built(&one, 1)) };
    let mut back = adopted.into_inner().into_raw();
    assert_eq!(back.count, 1);
    // SAFETY: `back` owns its commit and buffer again, released once by C.
    unsafe { ffi::git_commitarray_dispose(&mut back) };
    assert!(back.commits.is_null() && back.count == 0);
    free_repo(repo);
}

/// A NULL buffer carries no requirement: whatever the count, the array is empty and
/// released without libgit2, so this runs under Miri.
#[test]
fn adopts_a_null_buffer_with_any_count() {
    for count in [0, 1, 7, usize::MAX] {
        // SAFETY: `commits` is NULL.
        let mut array: CommitArrayVal<'static> = unsafe {
            CommitArray::from_raw(ffi::git_commitarray {
                commits: ptr::null(),
                count,
            })
        };
        assert_eq!(array.as_ref().len(), 0);
        assert!(array.as_ref().get(0).is_none());
        assert_eq!(format!("{:?}", array.as_ref()), "[]");
        if count == 7 {
            assert!(array.as_mut().take().is_empty());
        } else if count % 2 == 1 {
            array.as_mut().clear();
            let raw = array.into_inner().into_raw();
            assert_eq!((raw.commits, raw.count), (ptr::null(), 0));
        }
    }
}
