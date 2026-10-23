//! UB tests for `git2/oidarray.h`: the `OidArray` value held by its `OidArrayVal`
//! owner, its handles and their accessors, and the `git_oidarray_dispose` policy.
//!
//! Each array's ids sit in a buffer of exactly `len` ids (`from_slice`), so ASan flags
//! any access past them; every replaced, taken, truncated or copied buffer must be
//! released exactly once (ASan: double free, use after free; LSan: leak). Illegal
//! borrows are in `compile_fail/`. Only the empty array is built without libgit2, so
//! little runs under Miri.

use std::sync::Barrier;
use std::thread;

use ffibox::CZeroable;
use libgit2::libgit2::oid::oid::{Oid, OidRef, OidType};
use libgit2::libgit2::oid::oidarray::{OidArray, OidArrayMut, OidArrayRef, OidArrayVal};
use libgit2::libgit2::raw::OdbDataVec;

use crate::fixture::isolated;

/// A SHA1 oid whose id bytes start at `seed`, built without libgit2.
fn oid(seed: u8) -> Oid {
    let mut oid = Oid::zeroed();
    oid.as_mut().set_kind(OidType::Sha1);
    oid.as_mut()
        .set_id(core::array::from_fn(|i| seed.wrapping_add(i as u8)));
    oid
}

fn oids(n: usize) -> Vec<Oid> {
    (0..n).map(|i| oid(i as u8)).collect()
}

fn bytes(oid: OidRef<'_>) -> (u8, [u8; 32]) {
    (oid.raw_kind(), oid.id())
}

/// Every id of `array`, read through its run.
fn contents(array: OidArrayRef<'_>) -> Vec<(u8, [u8; 32])> {
    array.ids().iter().map(bytes).collect()
}

fn expected(oids: &[Oid]) -> Vec<(u8, [u8; 32])> {
    oids.iter().map(|o| bytes(o.as_ref())).collect()
}

#[test]
fn arrays_cross_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<OidArray>();
    send_sync::<OidArrayVal>();
    send_sync::<OidArrayRef<'_>>();
    send_sync::<OidArrayMut<'_>>();
}

/// The empty array over a bare value: no allocation, no disposal, so Miri runs it.
#[test]
fn empty_bare_array_in_rust_storage() {
    let mut bare = OidArray::new().into_inner();
    let mut m = bare.as_mut();
    m.truncate(usize::MAX);
    assert!(m.take_ids().is_empty());
    m.set_ids(OdbDataVec::empty());
    let r = m.as_ref();
    assert!(r.is_empty() && r.ids().is_empty() && r.ids().get(0).is_none());
    assert!(m.ids_mut().get_mut(0).is_none());
}

/// Before `git_libgit2_init` the empty array needs nothing from libgit2 (its dispose
/// never calls it), and a copy pins the runtime first.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_before_init() {
    isolated(concat!(module_path!(), "::arrays_before_init"), || {
        let empty = OidArray::new();
        assert!(empty.as_ref().is_empty());
        drop(empty);
        drop(OidArray::from_slice(&[]).unwrap());
        let src = oids(3);
        let array = OidArray::from_slice(&src).unwrap();
        assert_eq!(contents(array.as_ref()), expected(&src));
    });
}

/// Empty arrays, which never initialise libgit2, dropped while another thread's first
/// allocation initialises it: their dispose must not reach `git__free`, which reads
/// the allocator that initialisation writes (TSan). Alone in a child, so the
/// initialisation is really the process's first.
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
                        drop(OidArray::new());
                        drop(OidArray::from_slice(&[]).unwrap());
                        let mut emptied = OidArray::new();
                        drop(emptied.as_mut().take_ids());
                        emptied.as_mut().set_ids(OdbDataVec::empty());
                        drop(emptied);
                    }
                });
                s.spawn(|| {
                    start.wait();
                    let array = OidArray::from_slice(&oids(1)).unwrap();
                    assert_eq!(array.as_ref().len(), 1);
                });
            });
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn views_stay_in_bounds() {
    for n in [0, 1, 2, 33, 128] {
        let src = oids(n);
        let mut array = OidArray::from_slice(&src).unwrap();
        let r = array.as_ref();
        assert_eq!(r.len(), n);
        let ids = r.ids();
        assert_eq!(ids.len(), n);
        assert_eq!(ids.iter().count(), n);
        assert!(ids.get(n).is_none() && ids.get(usize::MAX).is_none());
        assert!(ids.slice(..=n).is_none() && ids.split_at(n + 1).is_none());
        assert_eq!(contents(r), expected(&src));
        if let Some(last) = n.checked_sub(1) {
            assert_eq!(bytes(ids.get(last).unwrap()), bytes(src[last].as_ref()));
        }

        let mut m = array.as_mut();
        let mut run = m.ids_mut();
        assert_eq!(run.len(), n);
        assert!(run.get_mut(n).is_none());
        // Overwrite every id in place, the last one included.
        for mut id in run.iter_mut() {
            id.set_id([0xEE; 32]);
        }
        assert!(contents(m.as_ref()).iter().all(|&(_, id)| id == [0xEE; 32]));
        m.truncate(n / 2);
        assert_eq!(m.as_ref().ids().len(), n / 2);
        assert!(m.as_ref().ids().get(n / 2).is_none());
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn replacements_release_each_buffer_once() {
    let mut array = OidArray::new();
    for n in [3, 0, 1, 64, 0, 2] {
        let src = oids(n);
        array
            .as_mut()
            .set_ids(OdbDataVec::from_slice(&src).unwrap());
        assert_eq!(contents(array.as_ref()), expected(&src));
    }
    // Taken ids outlive the array they came from.
    let taken = array.as_mut().take_ids();
    assert!(array.as_ref().is_empty());
    assert!(array.as_mut().take_ids().is_empty());
    drop(array);
    assert_eq!(taken.len(), 2);
    let again = OidArray::from_ids(taken);
    assert_eq!(contents(again.as_ref()), expected(&oids(2)));

    // A truncated array releases its whole buffer, taken or disposed.
    let mut short = OidArray::from_slice(&oids(8)).unwrap();
    short.as_mut().truncate(1);
    let taken = short.as_mut().take_ids();
    assert_eq!(taken.len(), 1);
    drop(taken);
    let mut short = OidArray::from_slice(&oids(8)).unwrap();
    short.as_mut().truncate(0);
    assert!(short.as_ref().ids().is_empty());
    drop(short);
}

/// Buffers moved between live arrays, through bare values and cloned runs are each
/// released exactly once, and read within bounds wherever they end up.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn buffers_move_between_arrays_and_owners() {
    let src = oids(6);
    let mut first = OidArray::from_slice(&src).unwrap();
    let mut second = OidArray::from_slice(&oids(2)).unwrap();

    // The buffer moves from one live array to another, which releases its own.
    let moved = first.as_mut().take_ids();
    second.as_mut().set_ids(moved);
    assert!(first.as_ref().is_empty());
    assert_eq!(contents(second.as_ref()), expected(&src));
    assert_eq!(
        format!("{:?}", second.as_ref()),
        format!("{:?}", src.iter().map(|o| o.as_ref()).collect::<Vec<_>>())
    );

    // A cloned run is a buffer of its own.
    let run = second.as_mut().take_ids();
    let twin = run.clone();
    assert_ne!(run.as_ptr(), twin.as_ptr());
    first.as_mut().set_ids(run);
    second.as_mut().set_ids(twin);
    second
        .as_mut()
        .ids_mut()
        .get_mut(5)
        .unwrap()
        .set_id([7; 32]);
    assert_eq!(contents(first.as_ref()), expected(&src));
    assert_eq!(contents(second.as_ref())[5].1, [7; 32]);

    assert_eq!(contents(first.as_ref()), expected(&src));
    drop(first);
    drop(second);
}

/// A copy owns a buffer of its own: dropping the source first leaves it readable, and
/// edits do not cross.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copies_are_independent() {
    let src = oids(5);
    let mut array = OidArray::from_slice(&src).unwrap();
    let copy = array.as_ref().to_oid_array().unwrap();
    array
        .as_mut()
        .ids_mut()
        .get_mut(4)
        .unwrap()
        .set_raw_kind(0xFF);
    let copy_of_copy = copy.as_ref().to_oid_array().unwrap();
    drop(array);
    assert_eq!(contents(copy.as_ref()), expected(&src));
    drop(copy);
    assert_eq!(contents(copy_of_copy.as_ref()), expected(&src));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn handles_reborrow() {
    let src = oids(4);
    let mut array = OidArray::from_slice(&src).unwrap();
    let mut m = array.as_mut();
    {
        let mut inner = m.as_mut();
        inner.truncate(3);
        inner
            .ids_mut()
            .get_mut(0)
            .unwrap()
            .set_kind(OidType::Sha256);
    }
    let r = m.as_ref();
    let (a, b) = (r, r);
    assert_eq!(a.len(), 3);
    assert_eq!(b.ids().get(0).unwrap().kind(), Some(OidType::Sha256));
    // A run borrowed from a copied shared handle lives as long as the owner's borrow.
    let ids = {
        let shared = array.as_ref();
        shared.ids()
    };
    assert_eq!(ids.len(), 3);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_shared_and_moved_across_threads() {
    let src = oids(16);
    let mut array = OidArray::from_slice(&src).unwrap();

    // Concurrent readers of one array.
    thread::scope(|s| {
        let (shared, src) = (&array, &src);
        for _ in 0..4 {
            s.spawn(move || assert_eq!(contents(shared.as_ref()), expected(src)));
        }
    });

    // An exclusive handle sent to one writer, read back after it joins.
    thread::scope(|s| {
        let mut m = array.as_mut();
        s.spawn(move || {
            m.ids_mut().get_mut(15).unwrap().set_id([1; 32]);
            m.set_ids(OdbDataVec::from_slice(&oids(2)).unwrap());
        });
    });
    assert_eq!(contents(array.as_ref()), expected(&oids(2)));

    // Arrays built on one thread, disposed on another, and ids moved between them.
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let built = OidArray::from_slice(&oids(i + 1)).unwrap();
            thread::spawn(move || {
                let n = built.as_ref().len();
                drop(built);
                OdbDataVec::from_slice(&oids(n)).unwrap()
            })
        })
        .collect();
    for (i, h) in handles.into_iter().enumerate() {
        let ids = h.join().unwrap();
        array.as_mut().set_ids(ids);
        assert_eq!(array.as_ref().len(), i + 1);
    }
    thread::spawn(move || drop(array)).join().unwrap();
}
