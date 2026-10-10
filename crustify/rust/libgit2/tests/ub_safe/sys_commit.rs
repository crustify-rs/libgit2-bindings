//! UB tests for `git2/sys/commit.h`'s [`ParentCallback`], through its public
//! [`call`](ParentCallback::call), which runs the `extern "C"` trampoline libgit2 calls.
//!
//! The callback keeps the last id it handed out, which libgit2 reads through the
//! returned pointer until the next call: the id read back must be the closure's, whole
//! (Miri, ASan), and the closure's own state is written between calls (BSan: nothing
//! keeps a pointer into it). Ids are built and compared without libgit2 (raw bytes and
//! type byte), so Miri runs every test. A panic in the closure resumes from `call` without
//! crossing the trampoline, and leaves the callback usable. Nothing here reaches
//! libgit2's commit routines, whose walk is unit-tested beside the wrapper, so Miri runs
//! it all. A handed-out id cannot outlive the next call, and the callback stays on its
//! thread (doctests).

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
