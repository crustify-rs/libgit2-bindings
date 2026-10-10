//! UB tests for `git2/refs.h`: the `git_reference` owners, `ReferenceBox<'r>` for an
//! attached reference and `UnattachedReferenceBox` for one a backend builds, and
//! their handles.
//!
//! Every owner releases its reference through `git_reference_free` exactly once, and
//! an attached one copies through `git_reference_dup`; a double free, a leaked copy
//! or a read of a freed reference is flagged by ASan (LSan included). Copies outlive
//! their originals here, chains of copies are dropped in every order, and C reads
//! each survivor back. The constructors copy their arguments (`name` into the
//! reference's flexible array, a symbolic target into its own allocation), so the
//! tests build references from Rust buffers that are overwritten or freed right after,
//! from names long enough that an off-by-one in the flexible array would leave the
//! allocation, and from ids of every type byte; libgit2 then retains no pointer into
//! Rust memory for a later foreign write (BSan).
//!
//! An unattached reference is `Send` and `Sync`: these tests build, share and drop
//! them across threads (TSan), and race the first constructors of a fresh process
//! to pin libgit2's runtime. An attached one is neither (doctests in `refs.rs`). A
//! handle or owner outliving its owner or repository borrow, aliased exclusive
//! handles, and a copy of an unattached reference do not compile (`compile_fail/`
//! and doctests).
//!
//! Every test calls libgit2, so Miri skips them all.

use std::ffi::{CStr, CString};
use std::sync::Barrier;
use std::thread;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::refs::refs::{
    ReferenceBox, ReferenceMut, ReferenceRef, UnattachedReference, UnattachedReferenceBox,
};

use crate::fixture::refs::{Repo, View, adopt_unattached, cmp, owner, view, view_unattached};
use crate::fixture::{init, isolated};

fn sha1(byte: u8) -> Oid {
    Oid::from_raw(&[byte; 20], OidType::Sha1).unwrap()
}

fn padded(byte: u8, len: usize) -> [u8; 32] {
    let mut id = [0; 32];
    id[..len].fill(byte);
    id
}

/// The references a fresh fixture repository has: its symbolic `HEAD`, which names
/// an unborn branch, and a packed, peeled tag.
fn read_all(repo: &Repo) -> Vec<ReferenceBox<'_>> {
    vec![repo.lookup(c"HEAD"), repo.lookup(c"refs/tags/v1")]
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copies_outlive_their_originals() {
    let repo = Repo::new("outlive");
    for original in read_all(&repo) {
        let expected = view(&original);
        let copy = original.clone();
        let copy_of_copy = copy.try_clone().unwrap();
        drop(original);
        assert_eq!(view(&copy), expected);
        drop(copy);
        assert_eq!(view(&copy_of_copy), expected);
        assert_eq!(owner(&copy_of_copy), repo.raw());
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copy_chains_release_every_copy_in_any_order() {
    let repo = Repo::new("chains");
    let tag = repo.lookup(c"refs/tags/v1");
    let expected = view(&tag);
    for order in 0..3 {
        let mut chain = vec![tag.clone()];
        for _ in 0..16 {
            let next = chain.last().unwrap().clone();
            chain.push(next);
        }
        match order {
            0 => chain.reverse(),
            1 => {}
            _ => chain.rotate_left(7),
        }
        while let Some(reference) = chain.pop() {
            assert_eq!(view(&reference), expected);
            for other in &chain {
                assert_eq!(cmp(other.as_c_ptr(), reference.as_c_ptr()), 0);
            }
        }
    }
    assert_eq!(view(&tag), expected);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn clone_from_replaces_a_reference_with_a_copy() {
    let repo = Repo::new("clone-from");
    let head = repo.lookup(c"HEAD");
    let mut target = repo.lookup(c"refs/tags/v1");
    target.clone_from(&head);
    assert_eq!(view(&target), view(&head));
    drop(head);
    assert_eq!(
        view(&target).symbolic.as_deref(),
        Some(c"refs/heads/master")
    );
}

/// Shared handles copy freely and an exclusive one reborrows, each reaching the one
/// reference its owner holds; none outlives it (`compile_fail/`).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn handles_reborrow_the_owner() {
    let repo = Repo::new("handles");
    let mut head = repo.lookup(c"HEAD");
    let expected = view(&head);
    {
        let shared: ReferenceRef<'_, '_> = head.as_ref();
        let again = shared;
        assert_eq!(format!("{shared:?}"), "Reference { .. }");
        assert_eq!(format!("{again:?}"), format!("{:?}", head.as_ref()));
    }
    {
        let mut exclusive: ReferenceMut<'_, '_> = head.as_mut();
        let reborrowed = exclusive.as_mut();
        assert_eq!(format!("{reborrowed:?}"), "Reference { .. }");
        let shared = exclusive.as_ref();
        assert_eq!(format!("{shared:?}"), "Reference { .. }");
    }
    assert_eq!(view(&head), expected);
}

/// Each repository's references are released before it, on whichever thread it lives;
/// no state libgit2 shares between repositories (the allocator, error slots) races.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn repositories_on_separate_threads_copy_independently() {
    init();
    let barrier = Barrier::new(4);
    thread::scope(|s| {
        for t in 0..4 {
            let barrier = &barrier;
            s.spawn(move || {
                let repo = Repo::new(&format!("thread-{t}"));
                let refs = read_all(&repo);
                barrier.wait();
                for _ in 0..50 {
                    let copies: Vec<_> = refs.iter().map(ReferenceBox::clone).collect();
                    for (copy, original) in copies.iter().zip(&refs) {
                        assert_eq!(view(copy), view(original));
                    }
                }
            });
        }
    });
}

/// `name` is copied into the reference: overwriting and freeing the Rust buffer after
/// construction leaves what C reads untouched.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn constructors_copy_their_arguments() {
    let mut name = CString::new("refs/heads/topic")
        .unwrap()
        .into_bytes_with_nul();
    let mut target = CString::new("refs/heads/main")
        .unwrap()
        .into_bytes_with_nul();
    let mut id = sha1(0x44);
    let mut peel = sha1(0x55);
    let direct = UnattachedReference::direct(
        CStr::from_bytes_with_nul(&name).unwrap(),
        id.as_ref(),
        Some(peel.as_ref()),
    )
    .unwrap();
    let symbolic = UnattachedReference::symbolic(
        CStr::from_bytes_with_nul(&name).unwrap(),
        CStr::from_bytes_with_nul(&target).unwrap(),
    )
    .unwrap();
    name[..4].fill(b'X');
    target[..4].fill(b'Y');
    id.as_mut().set_id([0xee; 32]);
    peel.as_mut().set_id([0xee; 32]);
    drop((name, target));

    let d = view_unattached(&direct);
    assert_eq!(d.name.as_c_str(), c"refs/heads/topic");
    assert_eq!(d.kind, ffi::GIT_REFERENCE_DIRECT);
    assert_eq!(d.target, Some((1, padded(0x44, 20))));
    assert_eq!(d.peel, Some((1, padded(0x55, 20))));
    assert_eq!(d.symbolic, None);
    let s = view_unattached(&symbolic);
    assert_eq!(s.name.as_c_str(), c"refs/heads/topic");
    assert_eq!(s.kind, ffi::GIT_REFERENCE_SYMBOLIC);
    assert_eq!(s.target, None);
    assert_eq!(s.symbolic.as_deref(), Some(c"refs/heads/main"));
}

/// Names from empty to far past any path length fill the flexible array exactly;
/// targets likewise fill their own copy.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn names_of_every_length_stay_in_bounds() {
    for len in [0, 1, 7, 8, 15, 16, 63, 64, 255, 1023, 1024, 4097] {
        let name = CString::new(vec![b'n'; len]).unwrap();
        let target = CString::new(vec![b't'; len]).unwrap();
        let direct = UnattachedReference::direct(&name, sha1(0x66).as_ref(), None).unwrap();
        let symbolic = UnattachedReference::symbolic(&name, &target).unwrap();
        assert_eq!(view_unattached(&direct).name, name);
        assert_eq!(view_unattached(&symbolic).name, name);
        assert_eq!(view_unattached(&symbolic).symbolic, Some(target));
    }
    // Bytes that are no UTF-8 are names too.
    let name = CString::new(vec![0xff, 0x80, b'/', 0xc3]).unwrap();
    let symbolic = UnattachedReference::symbolic(&name, &name).unwrap();
    assert_eq!(view_unattached(&symbolic).name, name);
}

/// Ids of every type byte, named or not, become targets, and a missing peeled id is
/// cleared to the target's type.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn ids_of_every_type_byte_are_copied() {
    for kind in [0, 1, 2, 3, u8::MAX] {
        let mut id = Oid::zeroed();
        id.as_mut().set_raw_kind(kind);
        id.as_mut().set_id([0x77; 32]);
        let reference = UnattachedReference::direct(c"refs/x", id.as_ref(), None).unwrap();
        let v = view_unattached(&reference);
        assert_eq!(v.target, Some((kind, [0x77; 32])));
        // The cleared peel is all-zero: no peeled id.
        assert_eq!(v.peel, None);
    }
    let sha256 = Oid::from_raw(&[0x88; 32], OidType::Sha256).unwrap();
    let reference =
        UnattachedReference::direct(c"refs/x", sha256.as_ref(), Some(sha256.as_ref())).unwrap();
    let v = view_unattached(&reference);
    assert_eq!(v.target, Some((2, [0x88; 32])));
    assert_eq!(v.peel, v.target);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn unattached_handles_and_raw_seam() {
    init();
    let mut reference = UnattachedReference::symbolic(c"HEAD", c"refs/heads/main").unwrap();
    let expected = view_unattached(&reference);
    {
        let shared = reference.as_ref();
        let again = shared;
        assert_eq!(format!("{again:?}"), "UnattachedReference { .. }");
        let mut exclusive = reference.as_mut();
        let reborrowed = exclusive.as_mut();
        assert_eq!(format!("{reborrowed:?}"), "UnattachedReference { .. }");
        assert_eq!(
            format!("{:?}", exclusive.as_ref()),
            "UnattachedReference { .. }"
        );
    }
    // Ownership leaves and comes back through the raw seam, as a backend hands a
    // reference to libgit2.
    let raw = reference.into_c();
    let reference = adopt_unattached(raw);
    assert_eq!(view_unattached(&reference), expected);
}

/// Unattached references move between threads and are shared by reference; each is
/// released once, on whichever thread drops it.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn unattached_references_cross_threads() {
    init();
    let shared = UnattachedReference::symbolic(c"HEAD", c"refs/heads/main").unwrap();
    let expected = view_unattached(&shared);
    let built: Vec<UnattachedReferenceBox> = thread::scope(|s| {
        let handles: Vec<_> = (0..8u8)
            .map(|t| {
                let shared = &shared;
                let expected = &expected;
                s.spawn(move || {
                    for _ in 0..50 {
                        assert_eq!(&view_unattached(shared), expected);
                        let _ = shared.as_ref();
                    }
                    let name = CString::new(format!("refs/heads/t{t}")).unwrap();
                    UnattachedReference::direct(&name, sha1(t).as_ref(), None).unwrap()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    // Dropped on threads other than the ones that built them.
    thread::scope(|s| {
        for (t, reference) in built.into_iter().enumerate() {
            s.spawn(move || {
                let v: View = view_unattached(&reference);
                assert_eq!(v.name.to_bytes(), format!("refs/heads/t{t}").as_bytes());
                drop(reference);
            });
        }
    });
}

/// The first constructors of a process race to pin libgit2's runtime; each either
/// pins it or waits for the thread that does, and allocates only once it is up.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn first_constructors_race_to_pin_the_runtime() {
    isolated(
        concat!(
            module_path!(),
            "::first_constructors_race_to_pin_the_runtime"
        ),
        || {
            let barrier = Barrier::new(8);
            thread::scope(|s| {
                let threads: Vec<_> = (0..8u8)
                    .map(|t| {
                        let barrier = &barrier;
                        s.spawn(move || {
                            let id = Oid::zeroed();
                            barrier.wait();
                            let reference = if t % 2 == 0 {
                                UnattachedReference::direct(c"refs/heads/a", id.as_ref(), None)
                            } else {
                                UnattachedReference::symbolic(c"HEAD", c"refs/heads/a")
                            };
                            assert_eq!(
                                view_unattached(&reference.unwrap())
                                    .name
                                    .as_c_str()
                                    .to_bytes()[0],
                                if t % 2 == 0 { b'r' } else { b'H' }
                            );
                        })
                    })
                    .collect();
                // Joined one by one, so each thread's exit, OpenSSL's per-thread
                // cleanup included, is ordered before the process's exit-time teardown.
                for thread in threads {
                    thread.join().unwrap();
                }
            });
        },
    );
}
