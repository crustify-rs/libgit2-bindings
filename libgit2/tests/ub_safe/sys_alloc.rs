//! `git2/sys/alloc.h`: `Allocator` values, their handles, and the trampolines
//! `Allocator::new` builds, driven by libgit2 itself.
//!
//! Each libgit2 test installs a Rust allocator in a child process of its own (libgit2's
//! allocator is process-global), then runs safe workloads that allocate, resize and
//! free through it; ASan/LSan flag a trampoline that mishandles a block, TSan one that
//! races, and the counters show the calls reached Rust.

use std::thread;

use libgit2::libgit2::raw::{OdbDataArrayFree, OdbDataStrFree, OdbDataVec};
use libgit2::libgit2::util::sys::alloc::{Allocator, FreeFn};

use crate::fixture::{self, Counted, REFUSED, isolated, leave_free};

static COUNTED: Allocator = Allocator::new::<Counted>();

/// libgit2 initialises, allocates, resizes and frees through the trampolines.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn libgit2_runs_on_a_rust_allocator() {
    isolated(
        concat!(module_path!(), "::libgit2_runs_on_a_rust_allocator"),
        || {
            fixture::install(&COUNTED);
            let v = OdbDataVec::<u8>::from_slice(b"through Rust").unwrap();
            let (mallocs, reallocs, _) = Counted::calls();
            assert!(
                mallocs > 1,
                "initialisation and the run allocated through Rust"
            );
            assert!(reallocs > 0, "initialisation resized through Rust");

            let s = OdbDataStrFree::dup(c"string").unwrap();
            let w = v.clone();
            let (_, _, frees) = Counted::calls();
            drop((v, s));
            assert_eq!(Counted::calls().2, frees + 2);
            assert_eq!(w.as_slice(), b"through Rust");

            // A failed allocation leaves the allocator usable.
            assert!(OdbDataArrayFree::new_uninit::<u8>(REFUSED).is_none());
            assert!(OdbDataStrFree::dup(c"after").is_some());
        },
    );
}

/// Threads allocate and free through the same installed Rust allocator, and read the
/// shared value's members, concurrently.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn threads_share_a_rust_allocator() {
    isolated(
        concat!(module_path!(), "::threads_share_a_rust_allocator"),
        || {
            fixture::install(&COUNTED);
            let workers: Vec<_> = (0..4u8)
                .map(|t| {
                    thread::spawn(move || {
                        for i in 0..64u8 {
                            let v = OdbDataVec::<u8>::from_slice(&[t, i, t ^ i]).unwrap();
                            let s = OdbDataStrFree::dup(c"shared").unwrap();
                            assert_eq!(v.as_slice(), [t, i, t ^ i]);
                            assert_eq!(s.as_bytes(), b"shared");
                            assert!(COUNTED.as_ref().malloc().is_some());
                        }
                    })
                })
                .collect();
            for w in workers {
                w.join().unwrap();
            }
            let (mallocs, _, frees) = Counted::calls();
            assert!(mallocs >= 4 * 64 * 2 && frees >= 4 * 64 * 2);
        },
    );
}

/// A value is copied, not shared: replacing a member of the copy leaves the original.
#[test]
fn copies_are_independent() {
    // One stored pointer value per function: each coercion may yield a new address.
    let leave: FreeFn = leave_free;
    let original = Allocator::new::<Counted>();
    let mut copy = original;
    let counted_free = original.as_ref().free().unwrap();
    copy.as_mut().set_free(leave);
    assert!(std::ptr::fn_addr_eq(copy.as_ref().free().unwrap(), leave));
    assert!(std::ptr::fn_addr_eq(
        original.as_ref().free().unwrap(),
        counted_free
    ));
    assert_ne!(format!("{original:?}"), format!("{copy:?}"));
}

/// Handles over values on other threads' stacks, through scoped borrows.
#[test]
fn handles_cross_threads() {
    let mut a = Allocator::new::<Counted>();
    let shared = a.as_ref();
    let seen: Vec<_> = thread::scope(|s| {
        let readers: Vec<_> = (0..4)
            .map(|_| s.spawn(move || format!("{shared:?}")))
            .collect();
        readers.into_iter().map(|r| r.join().unwrap()).collect()
    });
    assert!(seen.windows(2).all(|w| w[0] == w[1]));
    let leave: FreeFn = leave_free;
    let mut m = a.as_mut();
    thread::scope(|s| {
        s.spawn(move || m.set_free(leave));
    });
    assert!(std::ptr::fn_addr_eq(a.as_ref().free().unwrap(), leave));
}
