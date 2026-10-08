//! UB tests for `git2/sys/odb_backend.h`'s raw-lifetime strategies: `OdbDataBox` and
//! `OdbDataVec` over `git_odb_backend_data_alloc` / `git_odb_backend_data_free`.

use std::mem::{MaybeUninit, align_of};
use std::thread;

use libgit2::libgit2::raw::{OdbDataArrayFree, OdbDataBox, OdbDataVec};

use crate::fixture::{Pair, filled, isolated};

/// Up to just past a page; whole-slice comparisons keep BSan's per-access cost low.
const SIZES: [usize; 7] = [1, 2, 7, 16, 63, 4096, 4097];

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + 7) as u8).collect()
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_copies_slices_of_every_size() {
    for len in SIZES {
        let src = pattern(len);
        let mut v = OdbDataVec::<u8>::from_slice(&src).unwrap();
        assert_eq!((v.len(), v.byte_len()), (len, len));
        assert_eq!(v.as_slice(), &src[..]);
        // First and last bytes are in bounds; ASan flags any overrun.
        let s = v.as_mut_slice();
        s[0] ^= 0xff;
        s[len - 1] ^= 0xff;
        assert_eq!(v.as_slice()[len - 1], src[len - 1] ^ if len == 1 { 0 } else { 0xff });
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_clone_is_independent_and_outlives_its_source() {
    let src = pattern(100);
    let v = OdbDataVec::<u8>::from_slice(&src).unwrap();
    let mut w = v.clone();
    assert_ne!(v.as_ptr(), w.as_ptr());
    w.as_mut_slice().fill(0);
    assert_eq!(v.as_slice(), &src[..]);
    let t = w.try_clone().unwrap();
    drop(v);
    drop(w);
    assert_eq!(t.as_slice(), &[0; 100][..]);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_zeroed_is_zero_and_writable_to_the_end() {
    for len in SIZES {
        let mut v = OdbDataArrayFree::new_zeroed::<u64>(len).unwrap();
        assert_eq!(v.as_slice(), &vec![0; len][..]);
        v.as_mut_slice()[len - 1] = u64::MAX;
        assert_eq!(v.as_slice()[len - 1], u64::MAX);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_uninit_may_drop_partially_filled() {
    for len in SIZES {
        let mut v = OdbDataArrayFree::new_uninit::<u32>(len).unwrap();
        assert_eq!(v.len(), len);
        v.as_mut_slice()[len - 1].write(1);
        // Dropped without `assume_init`: the free reads no element.
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_uninit_clones_as_bytes_before_promotion() {
    // `MaybeUninit<u32>` is `Copy`, so a buffer still being filled clones as bytes
    // without reading any element as a `u32`; the copy is promoted on its own.
    let mut v = OdbDataArrayFree::new_uninit::<u32>(16).unwrap();
    v.as_mut_slice()[15].write(0x1234_5678);
    let half = v.clone();
    assert_eq!(half.len(), 16);
    drop(half);
    for (i, slot) in v.as_mut_slice().iter_mut().enumerate() {
        slot.write(i as u32);
    }
    let w = v.clone();
    drop(v);
    let w = filled(w);
    assert_eq!(w.as_slice(), &(0..16).collect::<Vec<u32>>()[..]);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_storage_aligns_wide_elements() {
    let v = OdbDataVec::<u64>::from_slice(&[1, 2, 3]).unwrap();
    assert_eq!(v.as_ptr() as usize % align_of::<u64>(), 0);
    let p = OdbDataArrayFree::new_zeroed::<*const u8>(3).unwrap();
    assert!(p.as_slice().iter().all(|q| q.is_null()));
    let f = OdbDataVec::<f64>::from_slice(&[0.5, -1.0]).unwrap();
    assert_eq!(f.clone().as_slice(), &[0.5, -1.0]);
}

#[test]
fn vec_empty_forms_own_nothing() {
    let a = OdbDataVec::<u8>::from_slice(&[]).unwrap();
    let b = OdbDataArrayFree::new_uninit::<u16>(0).unwrap();
    let c = OdbDataArrayFree::new_zeroed::<u64>(0).unwrap();
    assert!(a.as_ptr().is_null() && b.as_ptr().is_null() && c.as_ptr().is_null());
    assert!(a.clone().is_empty() && a.as_slice().is_empty() && c.as_slice().is_empty());
}

#[test]
fn vec_rejects_oversized_requests_without_panicking() {
    assert!(OdbDataArrayFree::new_uninit::<u8>(isize::MAX as usize + 1).is_none());
    assert!(OdbDataArrayFree::new_uninit::<MaybeUninit<u64>>(usize::MAX / 4).is_none());
    assert!(OdbDataArrayFree::new_zeroed::<u32>(usize::MAX).is_none());
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn box_zeroed_reads_zero_then_takes_writes() {
    let mut pair = OdbDataBox::<Pair>::new_zeroed().unwrap();
    assert_eq!((pair.as_ref().a(), pair.as_ref().b()), (0, 0));
    pair.as_mut().set(u64::MAX, u32::MAX);
    let shared = pair.as_ref();
    assert_eq!((shared.a(), shared.b()), (u64::MAX, u32::MAX));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn box_uninit_may_drop_unfilled() {
    for _ in 0..64 {
        let mut slot = OdbDataBox::<Pair>::new_uninit().unwrap();
        assert!(!slot.as_c_ptr().is_null());
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owners_cross_threads() {
    let src = pattern(4096);
    let v = OdbDataVec::<u8>::from_slice(&src).unwrap();
    thread::scope(|s| {
        for _ in 0..4 {
            // Shared reads and copies of one buffer from several threads at once.
            s.spawn(|| {
                let copy = v.clone();
                assert_eq!(copy.as_slice(), v.as_slice());
            });
        }
    });
    // Allocated here, freed on another thread.
    thread::spawn(move || assert_eq!(v.as_slice(), &src[..])).join().unwrap();

    let handles: Vec<_> = (0..4u32)
        .map(|i| {
            thread::spawn(move || {
                let mut pair = OdbDataBox::<Pair>::new_zeroed().unwrap();
                pair.as_mut().set(u64::from(i), i);
                pair
            })
        })
        .collect();
    for (i, h) in handles.into_iter().enumerate() {
        let pair = h.join().unwrap();
        assert_eq!(pair.as_ref().b(), i as u32);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owners_work_before_any_init() {
    isolated(concat!(module_path!(), "::owners_work_before_any_init"), || {
        // Nothing has initialised libgit2: an unpinned allocation would fail through
        // libgit2's `failalloc` and write the OOM through a nonexistent TLS key.
        let v = OdbDataVec::<u8>::from_slice(b"first").unwrap();
        let w = v.clone();
        drop(v);
        assert_eq!(w.as_slice(), b"first");
        let z = OdbDataArrayFree::new_zeroed::<u32>(8).unwrap();
        assert_eq!(z.as_slice(), &[0; 8]);
        let pair = OdbDataBox::<Pair>::new_zeroed().unwrap();
        assert_eq!(pair.as_ref().a(), 0);
    });
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn first_allocations_race_to_pin_the_runtime() {
    isolated(concat!(module_path!(), "::first_allocations_race_to_pin_the_runtime"), || {
        let barrier = std::sync::Barrier::new(8);
        thread::scope(|s| {
            for i in 0..8u8 {
                let barrier = &barrier;
                s.spawn(move || {
                    barrier.wait();
                    let v = OdbDataVec::<u8>::from_slice(&[i; 32]).unwrap();
                    assert_eq!(v.clone().as_slice(), &[i; 32]);
                });
            }
        });
    });
}
