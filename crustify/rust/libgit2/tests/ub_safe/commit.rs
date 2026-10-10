//! UB tests for `git2/commit.h`: the `CommitArray` value held by its `CommitArrayVal`
//! owner, its handles and their accessors, and the `git_commitarray_dispose` policy.
//!
//! Each filled array's buffer is exactly `len` commit pointers long (libgit2 detaches
//! a `git_vector` sized to them), so ASan flags a read past them; every commit
//! reference and buffer must be released exactly once, whether the array is cleared,
//! dropped, or dropped after its repository (ASan: double free, use after free; LSan:
//! leak). Only the empty array is built without libgit2, so little runs under Miri.
//! Illegal uses are compile-fail doctests on the crate: `commit_ptrs` outliving its
//! array, a `clone`, sending an array to another thread.

use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::config::config::Config;
use libgit2::libgit2::object::commit::{CommitArray, CommitArrayRef};

use crate::fixture::commit_and_config::{
    EMPTY, MERGEDREPO, OpenRepo, TESTREPO, commit_id, commit_parents,
};
use crate::fixture::isolated;

fn ids(array: CommitArrayRef<'_>) -> Vec<String> {
    array.commit_ptrs().elems().map(commit_id).collect()
}

const HEAD: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
const MERGED: [&str; 2] = [
    "3a34580a35add43a4cf361e8e9a30060a905c876",
    "e2809157a7766f272e4cfe26e61ef2678a5357ff",
];

/// The empty array, built, read, cleared and dropped without libgit2.
#[test]
fn empty_arrays_never_reach_libgit2() {
    let mut array = CommitArray::new();
    assert!(array.as_ref().is_empty());
    assert_eq!(array.as_ref().len(), 0);
    let run = array.as_ref().commit_ptrs();
    assert!(run.is_empty() && run.elem(0).is_none() && run.slice(1..).is_none());
    array.as_mut().clear();
    array.as_mut().clear();
    assert_eq!(format!("{:?}", array.as_ref()), "[]");
    assert_eq!(format!("{:?}", array.as_mut()), "[]");
    let bare = array.into_inner();
    assert!(bare.as_ref().is_empty());
}

/// Filled arrays read every commit in bounds, and release each reference once, through
/// `clear` and through the drop, reads after a clear included.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn filled_arrays_read_clear_and_release() {
    let repo = OpenRepo::open(MERGEDREPO);
    let mut array = commit_parents(&repo);
    let r = array.as_ref();
    assert_eq!(r.len(), 2);
    assert_eq!(ids(r), MERGED);
    let run = r.commit_ptrs();
    assert!(run.elem(2).is_none() && run.slice(..3).is_none());
    assert_eq!(
        run.slice(1..)
            .unwrap()
            .elems()
            .map(commit_id)
            .collect::<Vec<_>>(),
        [MERGED[1]]
    );
    assert_eq!(format!("{r:?}").matches("0x").count(), 2);

    array.as_mut().clear();
    assert!(array.as_ref().is_empty() && array.as_ref().commit_ptrs().is_empty());
    array.as_mut().clear();

    // The same commits held by several arrays at once are counted references of the
    // repository's cache: releasing them in any order frees each once.
    let arrays: Vec<_> = (0..4).map(|_| commit_parents(&repo)).collect();
    for (i, a) in arrays.into_iter().enumerate().rev() {
        assert_eq!(ids(a.as_ref()), MERGED, "array {i}");
    }
    let unborn = commit_parents(&OpenRepo::open(EMPTY));
    assert!(unborn.as_ref().is_empty());
}

/// Arrays dropped after their repository (the parent first): the commits outlive it.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_outlive_their_repository() {
    let repo = OpenRepo::open(TESTREPO);
    let kept = commit_parents(&repo);
    let mut cleared = commit_parents(&repo);
    let read_before = ids(kept.as_ref());
    drop(repo);
    assert_eq!(ids(kept.as_ref()), read_before);
    assert_eq!(read_before, [HEAD]);
    cleared.as_mut().clear();
    drop(cleared);
    drop(kept);
}

/// Dropping and clearing empty arrays on one thread while another performs the
/// process's first libgit2 initialisation races nothing: the empty array never
/// reaches libgit2's allocator (TSan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_drops_do_not_race_the_first_init() {
    isolated(
        concat!(module_path!(), "::empty_drops_do_not_race_the_first_init"),
        || {
            let barrier = Barrier::new(2);
            thread::scope(|s| {
                s.spawn(|| {
                    barrier.wait();
                    for _ in 0..200 {
                        let mut array = CommitArray::new();
                        array.as_mut().clear();
                        drop(array);
                    }
                });
                s.spawn(|| {
                    barrier.wait();
                    drop(Config::new().unwrap());
                });
            });
        },
    );
}

/// Arrays filled from repositories on several threads at once, each array staying on
/// its own thread, release their commits without racing (TSan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_on_their_own_threads() {
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                let repo = OpenRepo::open(MERGEDREPO);
                for _ in 0..8 {
                    let mut array = commit_parents(&repo);
                    assert_eq!(ids(array.as_ref()), MERGED);
                    array.as_mut().clear();
                }
            });
        }
    });
}
