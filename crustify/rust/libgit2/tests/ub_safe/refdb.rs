//! UB tests for `git2/refdb.h`: the `git_refdb` owners.
//!
//! `RefdbBox` (the sole reference `git_refdb_open` gives out) and `RefdbArc` (one of
//! several) both release through `git_refdb_free`, the down-ref; every refdb is freed
//! once (ASan flags a double free, LSan a leak), whatever the order its owners drop
//! in, whether through a box or after conversion into an arc, and always before the
//! repository it borrows (the borrow of the fixture enforces it; outliving it does
//! not compile, per `Refdb`'s doctests). The handles reborrow without aliasing an
//! exclusive one (`compile_fail` doctests on `Refdb`).
//!
//! Refdbs are neither `Send` nor `Sync`, so nothing here crosses threads with one;
//! several threads each opening and dropping refdbs of their own repository exercise
//! libgit2's shared state (its allocator, the repositories' caches) under TSan.

use std::thread;

use libgit2::libgit2::refs::refdb::{RefdbArc, RefdbBox, RefdbMut, RefdbRef};

use crate::fixture::TempRepo;

fn shared(r: RefdbRef<'_, '_>) -> String {
    format!("{r:?}")
}

fn exclusive(mut m: RefdbMut<'_, '_>) -> String {
    let inner = format!("{:?}", m.as_mut());
    assert_eq!(shared(m.as_ref()), inner);
    format!("{m:?}")
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn handles_reborrow_and_drop_once() {
    let repo = TempRepo::new("refdb-handles");
    let mut db = repo.refdb();
    let r = db.as_ref();
    let copy = r;
    assert_eq!(shared(r), shared(copy));
    assert_eq!(exclusive(db.as_mut()), "Refdb { .. }");
    // The exclusive handle again, after the shared ones are gone.
    let mut m = db.as_mut();
    let _ = exclusive(m.as_mut());
    let _ = exclusive(m);
    drop(db);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owners_drop_in_any_order() {
    let repo = TempRepo::new("refdb-orders");
    let boxes: Vec<RefdbBox<'_>> = (0..6).map(|_| repo.refdb()).collect();
    let mut arcs: Vec<RefdbArc<'_>> = Vec::new();
    let mut kept: Vec<RefdbBox<'_>> = Vec::new();
    for (i, b) in boxes.into_iter().enumerate() {
        if i % 2 == 0 {
            arcs.push(RefdbArc::from(b));
        } else {
            kept.push(b);
        }
    }
    for a in &arcs {
        let _ = shared(a.as_ref());
    }
    // Drop from the back, the front, then the rest.
    drop(arcs.pop());
    drop(kept.remove(0));
    drop(arcs);
    drop(kept);
}

/// A shorter-lived repository borrow is fine for a refdb built for a longer one: the
/// owner is covariant in `'r`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owners_shorten_their_repository_borrow() {
    let repo = TempRepo::new("refdb-shorten");
    fn shorten<'s, 'l: 's>(b: RefdbBox<'l>) -> RefdbBox<'s> {
        b
    }
    let db = shorten(repo.refdb());
    let arc: RefdbArc<'_> = db.into();
    let _ = shared(arc.as_ref());
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn threads_open_and_drop_their_own_refdbs() {
    thread::scope(|s| {
        for t in 0..4 {
            s.spawn(move || {
                let repo = TempRepo::new(&format!("refdb-thread-{t}"));
                for _ in 0..16 {
                    let db = repo.refdb();
                    let arc = RefdbArc::from(repo.refdb());
                    drop(db);
                    drop(arc);
                }
            });
        }
    });
}
