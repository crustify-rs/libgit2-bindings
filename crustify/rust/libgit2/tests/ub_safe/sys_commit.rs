//! UB tests for `git2/sys/commit.h`'s [`ParentCallback`], through its public
//! [`call`](ParentCallback::call), which runs the `extern "C"` trampoline libgit2 calls.
//!
//! The callback keeps the last id it handed out, which libgit2 reads through the
//! returned pointer until the next call: the id read back must be the closure's, whole
//! (Miri, ASan), and the closure's own state is written between calls (BSan: nothing
//! keeps a pointer into it). Ids are built and compared without libgit2 (raw bytes and
//! type byte), so Miri runs these tests. A panic in the closure resumes from `call`
//! without crossing the trampoline, and leaves the callback usable. A handed-out id
//! cannot outlive the next call, and the callback stays on its thread (doctests).
//!
//! Then the commit routines taking ids, in [`creating`], which reach libgit2.

use std::panic::{AssertUnwindSafe, catch_unwind};

use libgit2::libgit2::object::sys::commit::ParentCallback;
use libgit2::libgit2::oid::oid::{Oid, OidType};

fn id(byte: u8, kind: OidType) -> Oid {
    let mut id = Oid::zero(kind);
    id.as_mut().set_id([byte; 32]);
    id
}

#[test]
fn ids_are_read_back_whole() {
    let mut store: Vec<Oid> = Vec::new();
    let mut next = |i: usize| {
        let kind = if i.is_multiple_of(2) {
            OidType::Sha1
        } else {
            OidType::Sha256
        };
        let fresh = id(i as u8, kind);
        store.push(fresh);
        (i < 6).then_some(fresh)
    };
    let mut callback = ParentCallback::new(&mut next);
    for i in 0..6 {
        let got = callback.call(i).unwrap();
        assert_eq!(got.id(), [i as u8; 32]);
        let kind = if i.is_multiple_of(2) {
            OidType::Sha1
        } else {
            OidType::Sha256
        };
        assert_eq!(u32::from(got.raw_kind()), kind.raw());
    }
    assert!(callback.call(6).is_none());
    assert!(callback.call(usize::MAX).is_none());
    drop(callback);
    assert_eq!(store.len(), 8);
    store.clear();
}

#[test]
fn panics_resume_and_leave_the_callback_usable() {
    let mut calls = 0;
    let mut next = |i: usize| {
        calls += 1;
        if i == 1 {
            panic!("parent {i}");
        }
        Some(id(7, OidType::Sha1))
    };
    let mut callback = ParentCallback::new(&mut next);
    assert!(callback.call(0).is_some());
    let caught = catch_unwind(AssertUnwindSafe(|| callback.call(1).is_some()));
    assert_eq!(
        caught
            .unwrap_err()
            .downcast_ref::<String>()
            .map(String::as_str),
        Some("parent 1")
    );
    let got = callback.call(2).unwrap();
    assert_eq!(got.id(), [7; 32]);
    assert_eq!(u32::from(got.raw_kind()), OidType::Sha1.raw());
    assert!(format!("{callback:?}").starts_with("ParentCallback"));
    drop(callback);
    assert_eq!(calls, 3);
}

/// UB tests for `create_from_ids` and `create_from_callback`: the tree and the parents
/// by id, as heap values freed right after the call (ASan: a read past one; BSan: a
/// pointer kept), many parents included, and ids of another type than the repository's,
/// which libgit2 would format out of bounds (ASan), refused before it runs, as is a
/// signature it cannot write (UBSan). Each thread writes into its own repository at
/// once, moving no reference (TSan). They call libgit2, so Miri skips them.
mod creating {
    use std::ffi::{CStr, CString};
    use std::thread;

    use libgit2::libgit2::core::types::{SignatureView, Time};
    use libgit2::libgit2::object::sys::commit::{
        ParentCallback, create_from_callback, create_from_ids,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidRef, OidType};
    use libgit2::libgit2::util::errors::ErrorClass;

    use crate::fixture::TempRepo;
    use crate::fixture::object_and_tree_entry::repository;

    const EMPTY_TREE: &CStr = c"4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    fn empty_tree() -> Box<Oid> {
        Box::new(Oid::from_string(EMPTY_TREE, OidType::Sha1).unwrap())
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn ids_are_read_for_the_call_only() {
        let temp = TempRepo::new("sys-commit-ids");
        let repo = repository(&temp);
        let sig = SignatureView::new(c"A", c"a@x", Time::new(0, 0));
        let mut tips: Vec<Oid> = Vec::new();
        for n in 0..6 {
            let tree = empty_tree();
            let heap: Vec<Box<Oid>> = tips.iter().map(|t| Box::new(*t)).collect();
            let refs: Vec<OidRef<'_>> = heap.iter().map(|b| b.as_ref().as_ref()).collect();
            let message = CString::new(format!("ids {n}\n"))
                .unwrap()
                .into_boxed_c_str();
            let by_ids = create_from_ids(
                repo,
                None,
                sig.as_ref(),
                sig.as_ref(),
                None,
                &message,
                tree.as_ref().as_ref(),
                &refs,
            )
            .unwrap();
            drop(refs);
            drop(heap);
            let mut next = |i: usize| tips.get(i).copied();
            let mut callback = ParentCallback::new(&mut next);
            let by_callback = create_from_callback(
                repo,
                None,
                sig.as_ref(),
                sig.as_ref(),
                None,
                &message,
                tree.as_ref().as_ref(),
                &mut callback,
            )
            .unwrap();
            drop(callback);
            drop((tree, message));
            assert_eq!(by_ids, by_callback, "{n}");
            tips.push(by_ids);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn foreign_ids_and_unwritable_signatures_are_refused() {
        let temp = TempRepo::new("sys-commit-refused");
        let repo = repository(&temp);
        let (good, min) = (
            SignatureView::new(c"A", c"a@x", Time::new(0, 0)),
            SignatureView::new(c"A", c"a@x", Time::new(0, i32::MIN)),
        );
        let tree = empty_tree();
        let mut wide = Oid::zero(OidType::Sha256);
        wide.as_mut().set_id([0xab; 32]);
        for parents in [
            &[wide.as_ref()][..],
            &[tree.as_ref().as_ref(), wide.as_ref()],
        ] {
            let e = create_from_ids(
                repo,
                None,
                good.as_ref(),
                good.as_ref(),
                None,
                c"m\n",
                tree.as_ref().as_ref(),
                parents,
            )
            .unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
        }
        let e = create_from_ids(
            repo,
            None,
            good.as_ref(),
            good.as_ref(),
            None,
            c"m\n",
            wide.as_ref(),
            &[],
        )
        .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        let e = create_from_ids(
            repo,
            None,
            min.as_ref(),
            good.as_ref(),
            None,
            c"m\n",
            tree.as_ref().as_ref(),
            &[],
        )
        .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        let mut next = |i: usize| (i < 3).then_some(wide);
        let mut callback = ParentCallback::new(&mut next);
        let e = create_from_callback(
            repo,
            None,
            good.as_ref(),
            good.as_ref(),
            None,
            c"m\n",
            tree.as_ref().as_ref(),
            &mut callback,
        )
        .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        let e = create_from_callback(
            repo,
            None,
            good.as_ref(),
            min.as_ref(),
            None,
            c"m\n",
            tree.as_ref().as_ref(),
            &mut callback,
        )
        .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn threads_write_into_their_own_repositories() {
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let temp = TempRepo::new(&format!("sys-commit-thread-{t}"));
                    let repo = repository(&temp);
                    let sig = SignatureView::new(c"T", c"t@x", Time::new(t as i64, 0));
                    let tree = empty_tree();
                    let mut tip: Option<Oid> = None;
                    for i in 0..4 {
                        let message = CString::new(format!("{t} {i}\n")).unwrap();
                        let parents: Vec<_> = tip.iter().map(Oid::as_ref).collect();
                        let id = create_from_ids(
                            repo,
                            None,
                            sig.as_ref(),
                            sig.as_ref(),
                            None,
                            &message,
                            tree.as_ref().as_ref(),
                            &parents,
                        )
                        .unwrap();
                        let mut next = |k: usize| (k == 0).then_some(id);
                        let mut callback = ParentCallback::new(&mut next);
                        let child = create_from_callback(
                            repo,
                            None,
                            sig.as_ref(),
                            sig.as_ref(),
                            None,
                            &message,
                            tree.as_ref().as_ref(),
                            &mut callback,
                        )
                        .unwrap();
                        tip = Some(child);
                    }
                });
            }
        });
    }
}
