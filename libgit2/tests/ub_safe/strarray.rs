//! UB tests for `git2/strarray.h`: the owned `StrArray` held by its `StrArrayVal`
//! owner, the borrowed `StrArrayView`, their handles, the iterator, and the
//! `git_strarray_dispose` policy.
//!
//! Every owned string sits in an allocation of exactly its length and terminator
//! (`from_strs`), and every array in one of exactly `len` pointers, so ASan flags any
//! read past them; every replaced, taken, truncated or copied string must be released
//! exactly once (ASan: double free, use after free; LSan: leak). libgit2's own results
//! (the extension list) carry spare capacity and, when empty, a non-NULL array. Illegal
//! borrows are in `compile_fail/`. Views and empty arrays are built without libgit2,
//! so they run under Miri.

use std::ffi::{CStr, CString};
use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::core::common;
use libgit2::libgit2::core::strarray::{
    Iter, StrArray, StrArrayMut, StrArrayRef, StrArrayVal, StrArrayView,
};
use libgit2::libgit2::raw::{OdbDataStrFree, OdbDataStrVec};

use crate::fixture::{isolated, set_extensions};

/// `n` distinct strings of lengths 0, 1, 2, ..., so each allocation's size differs.
fn strings(n: usize) -> Vec<CString> {
    (0..n)
        .map(|i| CString::new(vec![b'a' + (i % 26) as u8; i]).unwrap())
        .collect()
}

/// Every string of `array`, read through its handle.
fn contents(array: StrArrayRef<'_>) -> Vec<Vec<u8>> {
    array.iter().map(|s| s.to_bytes().to_vec()).collect()
}

fn expected<S: AsRef<CStr>>(strs: &[S]) -> Vec<Vec<u8>> {
    strs.iter()
        .map(|s| s.as_ref().to_bytes().to_vec())
        .collect()
}

/// Reads every string in full, byte by byte through `CStr`, and past the end.
fn read_all(array: StrArrayRef<'_>, n: usize) {
    assert_eq!(array.len(), n);
    assert_eq!(array.is_empty(), n == 0);
    for i in 0..n {
        let s = array.get(i).unwrap().to_bytes_with_nul();
        assert_eq!(s.iter().position(|&b| b == 0), Some(s.len() - 1));
    }
    assert!(array.get(n).is_none() && array.get(usize::MAX).is_none());
    assert_eq!(array.iter().len(), n);
    assert_eq!(array.iter().rev().count(), n);
    assert!(array.iter().nth(n).is_none());
}

#[test]
fn arrays_cross_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<StrArray>();
    send_sync::<StrArrayVal>();
    send_sync::<StrArrayRef<'_>>();
    send_sync::<StrArrayMut<'_>>();
    send_sync::<StrArrayView<'_>>();
    send_sync::<Iter<'_>>();
}

/// The empty array over a bare value: no allocation, no disposal, so Miri runs it.
#[test]
fn empty_bare_array_in_rust_storage() {
    let mut bare = StrArray::new().into_inner();
    let mut m = bare.as_mut();
    m.truncate(usize::MAX);
    m.truncate(0);
    assert!(m.take_strings().is_empty());
    m.set_strings(OdbDataStrVec::new());
    let r = m.as_ref();
    assert!(r.is_empty() && r.get(0).is_none() && r.iter().next().is_none());
    drop(StrArray::new());
    drop(StrArray::from_strings(OdbDataStrVec::new()));
}

/// Views over Rust strings of many sizes, read in full and past the end, dropped
/// before the strings; built without libgit2, so Miri checks every read.
#[test]
fn views_stay_in_bounds() {
    for n in [0, 1, 2, 33, 128] {
        let owned = strings(n);
        let view = StrArrayView::new(&owned);
        read_all(view.as_ref(), n);
        assert_eq!(contents(view.as_ref()), expected(&owned));
        let refs: Vec<&CStr> = owned.iter().map(CString::as_c_str).collect();
        let collected: StrArrayView<'_> = refs.iter().copied().collect();
        read_all(collected.as_ref(), n);
        let moved = std::hint::black_box(collected);
        assert_eq!(contents(moved.as_ref()), expected(&owned));
        drop(moved);
        drop(view);
        drop(owned);
    }
    read_all(StrArrayView::default().as_ref(), 0);
}

/// Before `git_libgit2_init` the empty array needs nothing from libgit2 (its dispose
/// never calls it), and a copy pins the runtime first.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_before_init() {
    isolated(concat!(module_path!(), "::arrays_before_init"), || {
        let empty = StrArray::new();
        assert!(empty.as_ref().is_empty());
        drop(empty);
        drop(StrArray::from_strs::<&CStr>(&[]).unwrap());
        let names = [c"x", c"yz"];
        let view = StrArrayView::new(&names);
        let array = view.as_ref().to_str_array().unwrap();
        assert_eq!(contents(array.as_ref()), expected(&names));
    });
}

/// Empty arrays, which never initialise libgit2, dropped and emptied while another
/// thread's first allocation initialises it: their dispose must not reach
/// `git__free`, which reads the allocator that initialisation writes (TSan). Alone in a
/// child, so the initialisation is really the process's first.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_array_drops_do_not_race_the_first_init() {
    isolated(
        concat!(
            module_path!(),
            "::empty_array_drops_do_not_race_the_first_init"
        ),
        || {
            let start = Barrier::new(2);
            thread::scope(|s| {
                s.spawn(|| {
                    start.wait();
                    for _ in 0..64 {
                        drop(StrArray::new());
                        drop(StrArray::from_strs::<&CStr>(&[]).unwrap());
                        let mut emptied = StrArray::new();
                        drop(emptied.as_mut().take_strings());
                        emptied.as_mut().set_strings(OdbDataStrVec::new());
                        emptied.as_mut().truncate(0);
                        drop(emptied);
                        drop(StrArrayView::default());
                    }
                });
                s.spawn(|| {
                    start.wait();
                    let array = StrArray::from_strs(&[c"first"]).unwrap();
                    assert_eq!(array.as_ref().len(), 1);
                });
            });
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owned_arrays_stay_in_bounds() {
    for n in [0, 1, 2, 33, 128] {
        let src = strings(n);
        let array = StrArray::from_strs(&src).unwrap();
        read_all(array.as_ref(), n);
        assert_eq!(contents(array.as_ref()), expected(&src));
        let copy = array.as_ref().to_str_array().unwrap();
        drop(array);
        read_all(copy.as_ref(), n);
    }
    let ext = common::extensions().unwrap();
    let n = ext.as_ref().len();
    read_all(ext.as_ref(), n);
}

/// Truncation, replacement and moves release each string and array once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn replacements_release_each_string_once() {
    let mut array = StrArray::from_strs(&strings(10)).unwrap();
    array.as_mut().truncate(7);
    array.as_mut().truncate(7);
    array.as_mut().truncate(8);
    assert_eq!(contents(array.as_ref()), expected(&strings(7)));
    array
        .as_mut()
        .set_strings(OdbDataStrVec::dup_all(&strings(3)).unwrap());
    assert_eq!(contents(array.as_ref()), expected(&strings(3)));
    array.as_mut().set_strings(OdbDataStrVec::new());
    assert!(array.as_ref().is_empty());
    array
        .as_mut()
        .set_strings(OdbDataStrVec::dup_all(&strings(5)).unwrap());
    array.as_mut().truncate(0);
    assert!(array.as_ref().is_empty());
    // The truncated array still holds its buffer, released with the next replacement.
    array
        .as_mut()
        .set_strings(common::extensions().unwrap().as_mut().take_strings());
    let n = array.as_ref().len();
    array.as_mut().truncate(n / 2);
    assert_eq!(array.as_ref().len(), n / 2);
    drop(array);

    // A libgit2 result replaced in place frees libgit2's strings and array.
    let mut ext = common::extensions().unwrap();
    ext.as_mut()
        .set_strings(OdbDataStrVec::dup_all(&[c"one"]).unwrap());
    assert_eq!(contents(ext.as_ref()), expected(&[c"one"]));
}

/// Strings move between owners, runs and `Vec`s without copies or double frees.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn strings_move_between_arrays_and_owners() {
    let mut a = StrArray::from_strs(&strings(4)).unwrap();
    let mut b = StrArray::new();
    let before: Vec<*const core::ffi::c_char> = a.as_ref().iter().map(CStr::as_ptr).collect();
    b.as_mut().set_strings(a.as_mut().take_strings());
    assert!(a.as_ref().is_empty());
    let after: Vec<*const core::ffi::c_char> = b.as_ref().iter().map(CStr::as_ptr).collect();
    assert_eq!(before, after, "moved, not copied");
    let mut v = b.as_mut().take_strings().into_vec();
    v.reverse();
    v.push(OdbDataStrFree::dup(c"tail").unwrap());
    let c = StrArray::from_strings(OdbDataStrVec::from_vec(v).unwrap());
    assert_eq!(c.as_ref().len(), 5);
    assert_eq!(c.as_ref().get(4), Some(c"tail"));
    assert_eq!(c.as_ref().get(3), Some(c""));
    // An allocated empty libgit2 result moves the same way.
    drop(StrArray::from_strings(
        StrArray::new().as_mut().take_strings(),
    ));
}

/// A copy owns its strings: editing or dropping one side leaves the other intact.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copies_are_independent() {
    let src = strings(6);
    let mut a = StrArray::from_strs(&src).unwrap();
    let b = a.as_ref().to_str_array().unwrap();
    a.as_mut().truncate(2);
    a.as_mut()
        .set_strings(OdbDataStrVec::dup_all(&[c"z"]).unwrap());
    assert_eq!(contents(b.as_ref()), expected(&src));
    drop(a);
    assert_eq!(contents(b.as_ref()), expected(&src));
    let view_src = strings(3);
    let view = StrArrayView::new(&view_src);
    let owned = view.as_ref().to_str_array().unwrap();
    drop(view);
    drop(view_src);
    assert_eq!(contents(owned.as_ref()), expected(&strings(3)));
}

/// The exclusive handle reborrows, and hands out shared handles, in sequence.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn handles_reborrow() {
    let mut array = StrArray::from_strs(&strings(4)).unwrap();
    let mut m = array.as_mut();
    {
        let mut inner = m.as_mut();
        inner.truncate(3);
        let r = inner.as_ref();
        assert_eq!(r.len(), 3);
    }
    let first = m.as_ref().get(1).unwrap().to_owned();
    m.truncate(1);
    assert_eq!(first.as_bytes(), b"b");
    assert_eq!(format!("{m:?}"), r#"[""]"#);
    let r = array.as_ref();
    let (x, y) = (r, r);
    assert_eq!(x.get(0), y.get(0));
}

/// Owners and views shared by reference, and owners moved, across threads (TSan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_shared_and_moved_across_threads() {
    let array = StrArray::from_strs(&strings(16)).unwrap();
    let view_src = strings(16);
    let view = StrArrayView::new(&view_src);
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| assert_eq!(contents(array.as_ref()), expected(&strings(16))));
            s.spawn(|| assert_eq!(contents(view.as_ref()), expected(&view_src)));
            let r = array.as_ref();
            s.spawn(move || assert_eq!(r.iter().len(), 16));
        }
    });
    // Arrays built on one thread, edited and disposed on others, and strings moved
    // between them.
    let mut moved = thread::spawn(|| StrArray::from_strs(&strings(8)).unwrap())
        .join()
        .unwrap();
    let taken = thread::spawn(move || {
        moved.as_mut().truncate(4);
        moved.as_mut().take_strings()
    })
    .join()
    .unwrap();
    let rebuilt = StrArray::from_strings(taken);
    assert_eq!(contents(rebuilt.as_ref()), expected(&strings(4)));
    thread::spawn(move || drop(rebuilt)).join().unwrap();
    thread::spawn(move || drop(array)).join().unwrap();
    let copy = thread::scope(|s| {
        s.spawn(|| view.as_ref().to_str_array().unwrap())
            .join()
            .unwrap()
    });
    assert_eq!(contents(copy.as_ref()), expected(&view_src));
}

/// libgit2's own results under every edit: a vector's array with spare capacity, and
/// an allocated empty one when every builtin is disabled. Alone in a child, since the
/// extension list is process-global.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn libgit2_results_edit_and_release() {
    isolated(
        concat!(module_path!(), "::libgit2_results_edit_and_release"),
        || {
            set_extensions(&[c"zeta", c"alpha", c"!noop"]);
            let mut ext = common::extensions().unwrap();
            let names = contents(ext.as_ref());
            assert!(names.contains(&b"zeta".to_vec()) && !names.contains(&b"noop".to_vec()));
            let copy = ext.as_ref().to_str_array().unwrap();
            ext.as_mut().truncate(1);
            assert_eq!(contents(ext.as_ref()), names[..1]);
            assert_eq!(contents(copy.as_ref()), names);

            set_extensions(&[
                c"!noop",
                c"!objectformat",
                c"!worktreeconfig",
                c"!preciousobjects",
                c"!refstorage",
                c"!relativeworktrees",
            ]);
            let mut none = common::extensions().unwrap();
            read_all(none.as_ref(), 0);
            none.as_mut().truncate(0);
            assert!(none.as_ref().to_str_array().unwrap().as_ref().is_empty());
            let taken = none.as_mut().take_strings();
            assert!(taken.is_empty() && !taken.as_ptr().is_null());
            drop(taken);
            drop(common::extensions().unwrap());
        },
    );
}
