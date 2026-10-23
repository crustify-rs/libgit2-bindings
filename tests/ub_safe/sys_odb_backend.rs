//! UB tests for `git2/sys/odb_backend.h`'s raw-lifetime strategies over
//! `git_odb_backend_data_alloc` / `git_odb_backend_data_free`: `OdbDataBox` and
//! `OdbDataVec` for raw storage, `OdbDataStr` and `OdbDataStrVec` for strings.

use std::ffi::{CStr, CString};
use std::mem::{MaybeUninit, align_of};
use std::thread;

use libgit2::libgit2::raw::{
    OdbDataArrayFree, OdbDataBox, OdbDataStr, OdbDataStrFree, OdbDataStrVec, OdbDataVec,
};

use crate::fixture::{Pair, filled, isolated};

/// Up to just past a page; whole-slice comparisons keep `BSan`'s per-access cost low.
const SIZES: [usize; 7] = [1, 2, 7, 16, 63, 4096, 4097];

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + 7).to_le_bytes()[0]).collect()
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
        s[0] ^= 0xFF;
        s[len - 1] ^= 0xFF;
        assert_eq!(
            v.as_slice()[len - 1],
            src[len - 1] ^ if len == 1 { 0 } else { 0xFF }
        );
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
        slot.write(u32::try_from(i).unwrap());
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
    thread::spawn(move || assert_eq!(v.as_slice(), &src[..]))
        .join()
        .unwrap();

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
        assert_eq!(pair.as_ref().b(), u32::try_from(i).unwrap());
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owners_work_before_any_init() {
    isolated(
        concat!(module_path!(), "::owners_work_before_any_init"),
        || {
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
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn first_allocations_race_to_pin_the_runtime() {
    isolated(
        concat!(
            module_path!(),
            "::first_allocations_race_to_pin_the_runtime"
        ),
        || {
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
        },
    );
}

// --- strings --------------------------------------------------------------------

/// String lengths up to just past a page, the empty string included.
const STR_LENS: [usize; 8] = [0, 1, 2, 7, 16, 63, 4095, 4096];

fn text(len: usize, seed: u8) -> CString {
    CString::new(
        (0..len)
            .map(|i| b'!' + (i.to_le_bytes()[0].wrapping_mul(7) ^ seed) % 90)
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn strs(v: &OdbDataStrVec) -> Vec<&[u8]> {
    v.as_slice().iter().map(OdbDataStr::as_bytes).collect()
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_copies_strings_of_every_length() {
    for len in STR_LENS {
        let src = text(len, 3);
        let s = OdbDataStrFree::dup(&src).unwrap();
        assert_ne!(s.as_ptr(), src.as_ptr());
        assert_eq!((s.len(), s.is_empty()), (len, len == 0));
        assert_eq!(s.as_c_str(), src.as_c_str());
        assert_eq!(s.as_bytes(), src.as_bytes());
        assert_eq!(s.to_str().unwrap().len(), len);
        assert_eq!(format!("{s:?}"), format!("CStrBox({src:?})"));
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_keeps_bytes_that_are_not_utf8() {
    let s = OdbDataStrFree::dup(c"caf\xe9").unwrap();
    assert!(s.to_str().is_err());
    assert_eq!(s.as_bytes(), b"caf\xe9");
    assert_eq!(s.clone().as_bytes(), b"caf\xe9");
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_clone_is_independent_and_outlives_its_source() {
    let s = OdbDataStrFree::dup(c"refs/remotes/origin/HEAD").unwrap();
    let t = s.clone();
    let u = t.try_clone().unwrap();
    assert!(s.as_ptr() != t.as_ptr() && t.as_ptr() != u.as_ptr());
    drop(s);
    drop(t);
    assert_eq!(u.as_c_str(), c"refs/remotes/origin/HEAD");
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_vec_copies_runs_of_every_shape() {
    for n in [1, 2, 3, 17, 512] {
        let src: Vec<CString> = (0..n)
            .map(|i| text(STR_LENS[i % STR_LENS.len()], i.to_le_bytes()[0]))
            .collect();
        let v = OdbDataStrVec::dup_all(&src).unwrap();
        assert_eq!((v.len(), v.is_empty()), (n, false));
        assert_eq!(
            strs(&v),
            src.iter().map(CString::as_bytes).collect::<Vec<_>>()
        );
        assert_eq!(
            v.as_slice().last().unwrap().as_c_str(),
            src.last().unwrap().as_c_str()
        );
        assert!(
            v.as_slice()
                .iter()
                .zip(&src)
                .all(|(a, b)| a.as_ptr() != b.as_ptr())
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_vec_elements_replace_and_move_in_place() {
    let mut v = OdbDataStrVec::dup_all(&[c"c", c"a", c"d", c"b"]).unwrap();
    // The replaced string is freed; ASan flags a leak or a double free.
    v.as_mut_slice()[2] = OdbDataStrFree::dup(c"e").unwrap();
    v.as_mut_slice()
        .sort_by(|x, y| x.as_bytes().cmp(y.as_bytes()));
    assert_eq!(strs(&v), [&b"a"[..], b"b", b"c", b"e"]);
    v.as_mut_slice().swap(0, 3);
    v.as_mut_slice().reverse();

    // Elements swapped between two runs belong to their new run.
    let mut w = OdbDataStrVec::dup_all(&[c"other"]).unwrap();
    std::mem::swap(&mut v.as_mut_slice()[1], &mut w.as_mut_slice()[0]);
    drop(v);
    assert_eq!(strs(&w), [&b"c"[..]]);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_vec_moves_through_a_vec() {
    let v = OdbDataStrVec::dup_all(&[c"origin", c"upstream", c"fork"]).unwrap();
    let mut owned = v.into_vec();
    drop(owned.remove(1));
    owned.push(OdbDataStrFree::dup(c"mirror").unwrap());
    let kept = owned[0].clone();
    let v = OdbDataStrVec::from_vec(owned).unwrap();
    assert_eq!(strs(&v), [&b"origin"[..], b"fork", b"mirror"]);
    // Strings moved out survive the run they left.
    let mut out = v.into_vec();
    let last = out.pop().unwrap();
    drop(out);
    assert_eq!(
        (kept.as_bytes(), last.as_bytes()),
        (&b"origin"[..], &b"mirror"[..])
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_vec_clone_is_independent_and_outlives_its_source() {
    let v = OdbDataStrVec::dup_all(&[c"x", c"", c"zz"]).unwrap();
    let mut w = v.clone();
    assert_ne!(v.as_ptr(), w.as_ptr());
    w.as_mut_slice()[0] = OdbDataStrFree::dup(c"y").unwrap();
    assert_eq!(strs(&v), [&b"x"[..], b"", b"zz"]);
    let t = w.try_clone().unwrap();
    drop(v);
    drop(w);
    assert_eq!(strs(&t), [&b"y"[..], b"", b"zz"]);
    assert_eq!(format!("{t:?}"), r#"["y", "", "zz"]"#);
}

#[test]
fn str_vec_empty_forms_own_nothing() {
    let v = OdbDataStrVec::new();
    let d = OdbDataStrVec::default();
    let e = OdbDataStrVec::dup_all::<&CStr>(&[]).unwrap();
    let f = OdbDataStrVec::from_vec(Vec::new()).unwrap();
    for r in [&v, &d, &e, &f] {
        assert!(r.is_empty() && r.as_ptr().is_null() && r.as_slice().is_empty());
    }
    assert!(v.clone().into_vec().is_empty());
    let (strings, count) = e.into_raw_parts();
    assert!(strings.is_null() && count == 0);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_owners_cross_threads() {
    let s = OdbDataStrFree::dup(c"shared").unwrap();
    let v = OdbDataStrVec::dup_all(&[c"a", c"b", c"c"]).unwrap();
    thread::scope(|scope| {
        for _ in 0..4 {
            // Shared reads and copies of one string and one run at once.
            scope.spawn(|| {
                assert_eq!(s.clone().as_bytes(), b"shared");
                let w = v.clone();
                assert_eq!(strs(&w), strs(&v));
            });
        }
    });
    // Made here, freed on other threads.
    let t = thread::spawn(move || drop(s));
    let u = thread::spawn(move || v.into_vec());
    t.join().unwrap();
    let strings = u.join().unwrap();
    assert_eq!(strings.len(), 3);

    let made: Vec<OdbDataStrVec> = (0..4)
        .map(|i| thread::spawn(move || OdbDataStrVec::dup_all(&[text(64, i), text(1, i)]).unwrap()))
        .map(|h| h.join().unwrap())
        .collect();
    assert!(made.iter().all(|v| v.len() == 2));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_owners_work_before_any_init() {
    isolated(
        concat!(module_path!(), "::str_owners_work_before_any_init"),
        || {
            // Nothing has initialised libgit2: an unpinned allocation would fail through
            // libgit2's `failalloc` and write the OOM through a nonexistent TLS key.
            let s = OdbDataStrFree::dup(c"first").unwrap();
            let v = OdbDataStrVec::dup_all(&[c"a", c"b"]).unwrap();
            assert_eq!((s.clone().as_bytes(), v.clone().len()), (&b"first"[..], 2));
        },
    );
}
