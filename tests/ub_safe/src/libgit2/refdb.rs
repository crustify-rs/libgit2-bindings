//! UB tests for `git2/refdb.h`: the `git_refdb` owners.
//!
//! `RefdbBox` (the sole reference `git_refdb_open` gives out) and `RefdbArc` (one of
//! several) both release through `git_refdb_free`, the down-ref; every refdb is freed
//! once (`ASan` flags a double free, `LSan` a leak), whatever the order its owners drop
//! in, whether through a box or after conversion into an arc, and always before the
//! repository it borrows (the borrow of the fixture enforces it; outliving it does
//! not compile, per `Refdb`'s doctests). The handles reborrow without aliasing an
//! exclusive one (`compile_fail` doctests on `Refdb`).
//!
//! Refdbs are neither `Send` nor `Sync`, so nothing here crosses threads with one;
//! several threads each opening and dropping refdbs of their own repository exercise
//! libgit2's shared state (its allocator, the repositories' caches) under `TSan`.
//!
//! Then the enum `RefdbType` (`git_refdb_t`), in `refdb_type`.

use std::thread;

use libgit2::libgit2::src::libgit2::refdb::{RefdbArc, RefdbBox, RefdbMut, RefdbRef};

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
    drop(exclusive(m.as_mut()));
    drop(exclusive(m));
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
        drop(shared(a.as_ref()));
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
#[expect(
    clippy::items_after_statements,
    reason = "each helper item sits beside the steps that use it"
)]
fn owners_shorten_their_repository_borrow() {
    let repo = TempRepo::new("refdb-shorten");
    fn shorten<'s, 'l: 's>(b: RefdbBox<'l>) -> RefdbBox<'s> {
        b
    }
    let db = shorten(repo.refdb());
    let arc: RefdbArc<'_> = db.into();
    drop(shared(arc.as_ref()));
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

mod refdb_type {
    //! UB tests for `git2/refdb.h`: the enum `RefdbType` (`git_refdb_t`, unsigned).
    //!
    //! The only safe way in from a raw value is `from_raw` / `try_from`, which must reject
    //! every value without a variant rather than produce an invalid discriminant (which
    //! Miri and `UBSan` would flag on the next read); an `as` cast to it does not compile
    //! (`compile_fail/`). It owns and points at nothing and none of these tests calls
    //! libgit2, so they also run under Miri; the sweep strides under `BSan`.

    use std::hint::black_box;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::refdb::{RefdbType, UnnamedRefdbType};

    /// Every small value, a stride across the rest of `git_refdb_t`, and its ends.
    fn probes() -> impl Iterator<Item = ffi::git_refdb_t> {
        let stride = if cfg!(bsan) || cfg!(miri) { 97 } else { 1 };
        (0..=4096)
            .step_by(stride)
            .chain([1, 2])
            .chain((0..=ffi::git_refdb_t::MAX).step_by(1 << 24))
            .chain([ffi::git_refdb_t::MAX - 1, ffi::git_refdb_t::MAX])
    }

    #[test]
    fn from_raw_yields_only_valid_types() {
        for raw in probes() {
            match RefdbType::from_raw(black_box(raw)) {
                Some(kind) => {
                    // Matching on the value reads its discriminant.
                    let back = match black_box(kind) {
                        RefdbType::Files => ffi::GIT_REFDB_FILES,
                        RefdbType::Reftable => ffi::GIT_REFDB_REFTABLE,
                        _ => unreachable!(),
                    };
                    assert_eq!(back, raw);
                    assert_eq!(RefdbType::try_from(raw), Ok(kind));
                }
                None => assert_eq!(
                    RefdbType::try_from(raw).map_err(UnnamedRefdbType::raw),
                    Err(raw)
                ),
            }
        }
    }

    #[test]
    fn option_niche_holds() {
        for raw in probes() {
            let kind = black_box(RefdbType::from_raw(raw));
            assert_eq!(kind.map_or(raw, RefdbType::raw), raw);
        }
    }

    #[test]
    fn conversions_agree_across_threads() {
        thread::scope(|s| {
            let handles: Vec<_> = [RefdbType::Files, RefdbType::Reftable, RefdbType::default()]
                .into_iter()
                .map(|kind| s.spawn(move || (kind, RefdbType::from_raw(kind.raw()))))
                .collect();
            for h in handles {
                let (kind, back) = h.join().unwrap();
                assert_eq!(back, Some(kind));
            }
        });
    }
}
