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

/// The extra headers: the `Header` value (`git_commit_header`) and its handles.
///
/// A header is two borrowed string pointers. Its handles project each field out of a
/// pointer and rebuild a `&CStr` from it, so these tests exercise every getter and
/// setter on every holder Rust has for one (a local, a heap slot, an element of a run
/// that moves when it grows, a copy, a `static`), through reborrows, with strings that
/// live on the heap and are dropped right after their last read (ASan: a getter
/// reading past the string or after it), and across threads under the `Send` / `Sync`
/// the crate asserts (TSan). A misaligned projection or a write through the shared
/// handle would be flagged by Miri and ASan/UBSan. Headers that outlive their strings,
/// handles that outlive their header, aliased handles and setters on the shared handle
/// do not compile (`compile_fail/` and the type's doctests). Nothing here reaches
/// libgit2, which keeps no pointer to a header past a call, so every test runs under
/// Miri and there is no foreign write or retained pointer to race (BSan).
mod header {
    use std::ffi::{CStr, CString};
    use std::sync::Mutex;
    use std::thread;

    use libgit2::libgit2::object::commit::{Header, HeaderMut, HeaderRef};

    static STATIC: Header<'static> = Header::new(c"static", c"header");

    fn read<'h>(r: HeaderRef<'_, 'h>) -> (&'h CStr, &'h CStr) {
        (r.field(), r.value())
    }

    fn write<'h>(mut m: HeaderMut<'_, 'h>, field: &'h CStr, value: &'h CStr) {
        m.set_field(field);
        m.set_value(value);
    }

    #[test]
    fn every_field_round_trips_on_every_holder() {
        let strings: Vec<CString> = ["a", "", "multi\nline value", "x".repeat(4096).as_str()]
            .into_iter()
            .map(|s| CString::new(s).unwrap())
            .collect();
        let mut local = Header::new(c"f", c"v");
        let mut boxed = Box::new(Header::new(c"f", c"v"));
        let mut run = vec![Header::new(c"f", c"v")];
        for (i, pair) in strings.windows(2).enumerate() {
            let (f, v) = (pair[0].as_c_str(), pair[1].as_c_str());
            write(local.as_mut(), f, v);
            write(Header::as_mut(&mut boxed), f, v);
            // Growing the run moves its elements; each handle is taken afresh.
            run.push(Header::new(f, v));
            write(run[i].as_mut(), f, v);
            for r in [
                local.as_ref(),
                Header::as_ref(&boxed),
                run[i].as_ref(),
                run[i + 1].as_ref(),
            ] {
                assert_eq!(read(r), (f, v));
            }
            let copy = local;
            assert_eq!(read(copy.as_ref()), (f, v));
        }
        assert_eq!(read(STATIC.as_ref()), (c"static", c"header"));
    }

    /// Strings on the heap, freed right after the header's last use: every read lands
    /// inside a live allocation (ASan).
    #[test]
    fn reads_stay_inside_heap_strings() {
        for len in [0, 1, 7, 8, 15, 16, 255] {
            let field = CString::new("f".repeat(len)).unwrap();
            let value = CString::new("v".repeat(len + 1)).unwrap();
            let header = Header::new(&field, &value);
            assert_eq!(header.as_ref().field().to_bytes().len(), len);
            assert_eq!(header.as_ref().value().to_bytes().len(), len + 1);
            assert!(format!("{header:?}").len() > 2 * len);
            // The header is `Copy`: its last use is above; the strings go now.
            drop(field);
            drop(value);
        }
    }

    #[test]
    fn reborrows_keep_one_writer() {
        let mut header = Header::new(c"a", c"b");
        let mut m = header.as_mut();
        {
            let mut inner = m.as_mut();
            inner.set_field(c"inner");
            assert_eq!(inner.as_ref().field(), c"inner");
        }
        let shared = m.as_ref();
        assert_eq!(read(shared), (c"inner", c"b"));
        m.set_value(c"after");
        assert_eq!(read(header.as_ref()), (c"inner", c"after"));
    }

    #[test]
    fn copies_are_independent() {
        let a = Header::new(c"a", c"1");
        let mut b = a;
        let mut c = a.as_ref().to_header();
        b.as_mut().set_value(c"2");
        c.as_mut().set_field(c"c");
        assert_eq!(read(a.as_ref()), (c"a", c"1"));
        assert_eq!(read(b.as_ref()), (c"a", c"2"));
        assert_eq!(read(c.as_ref()), (c"c", c"1"));
    }

    /// A run of headers read from several threads at once, each element written by
    /// one thread through its own exclusive handle, and headers moved between threads
    /// under the asserted `Send` / `Sync` (TSan).
    #[test]
    fn headers_cross_threads() {
        let owned: Vec<CString> = (0..8)
            .map(|i| CString::new(format!("value {i}")).unwrap())
            .collect();
        let mut run: Vec<Header<'_>> = owned.iter().map(|v| Header::new(c"field", v)).collect();
        thread::scope(|s| {
            for (i, h) in run.iter_mut().enumerate() {
                let owned = &owned;
                s.spawn(move || {
                    let mut m = h.as_mut();
                    m.set_field(c"written");
                    assert_eq!(m.as_ref().value(), owned[i].as_c_str());
                });
            }
        });
        let run = &run;
        let seen = Mutex::new(0);
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for h in run {
                        let r: HeaderRef<'_, '_> = h.as_ref();
                        assert_eq!(r.field(), c"written");
                        assert!(r.value().to_bytes().starts_with(b"value "));
                    }
                    *seen.lock().unwrap() += 1;
                });
            }
            let one = run[3];
            s.spawn(move || assert_eq!(one.as_ref().value(), c"value 3"));
            let handle = run[5].as_ref();
            s.spawn(move || assert_eq!(handle.value(), c"value 5"));
        });
        assert_eq!(*seen.lock().unwrap(), 4);
    }
}
