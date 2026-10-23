//! UB tests for the unsafe API of `git2/sys/odb_backend.h`'s string strategies:
//! `OdbDataStrVec::from_raw_parts`, called within its contract on every kind of
//! array libgit2 or the safe API hands out, edge cases included.
//!
//! Taking a string out of a `git_buf` moved to `core::buffer`'s `BufMut::take_str`,
//! whose adoption, `Buf::from_raw`, `tests/ub_unsafe/buffer.rs` covers.

use std::ffi::{CStr, c_char, c_int};
use std::mem::size_of;
use std::{ptr, thread};

use libgit2::ffi;
use libgit2::libgit2::raw::{OdbDataStr, OdbDataStrFree, OdbDataStrVec};

/// The raw C calls below allocate, which needs an initialised libgit2; the extra
/// reference dies with the process.
fn init() {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

fn strs(v: &OdbDataStrVec) -> Vec<&[u8]> {
    v.as_slice().iter().map(OdbDataStr::as_bytes).collect()
}

/// A `git__malloc` array of `capacity` slots holding copies of `items`, NULL entries
/// kept, as C code filling a `git_strarray` makes one.
fn c_array(items: &[Option<&CStr>], capacity: usize) -> *mut *mut c_char {
    assert!(capacity >= items.len() && capacity > 0);
    // SAFETY: `backend` is unused by the routine; any length.
    let arr = unsafe {
        ffi::git_odb_backend_data_alloc(ptr::null_mut(), capacity * size_of::<*mut c_char>())
    }
    .cast::<*mut c_char>();
    assert!(!arr.is_null());
    for (i, item) in items.iter().enumerate() {
        let s = item.map_or(ptr::null_mut(), |s| {
            OdbDataStrFree::dup(s).unwrap().into_raw()
        });
        // SAFETY: `i < capacity` slots of `arr` are writable.
        unsafe { arr.add(i).write(s) };
    }
    arr
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopts_its_own_raw_parts() {
    for n in [1, 2, 9, 300] {
        let names: Vec<_> = (0..n)
            .map(|i| std::ffi::CString::new(format!("ref{i}")).unwrap())
            .collect();
        let (strings, count) = OdbDataStrVec::dup_all(&names).unwrap().into_raw_parts();
        // SAFETY: the pair `into_raw_parts` gave up: a `git__malloc` array of `count`
        // owned strings, handed back whole.
        let v = unsafe { OdbDataStrVec::from_raw_parts(strings, count) }.unwrap();
        assert_eq!((v.as_ptr(), v.len()), (strings, n));
        assert_eq!(
            v.as_slice()[n - 1].as_bytes(),
            format!("ref{}", n - 1).as_bytes()
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopts_a_c_array_with_spare_capacity() {
    init();
    let arr = c_array(&[Some(c"a"), Some(c""), Some(c"ccc")], 8);
    // SAFETY: a `git__malloc` array whose first 3 slots hold owned strings; the spare
    // slots are never read.
    let mut v = unsafe { OdbDataStrVec::from_raw_parts(arr, 3) }.unwrap();
    assert_eq!(strs(&v), [&b"a"[..], b"", b"ccc"]);
    v.as_mut_slice()[1] = OdbDataStrFree::dup(c"bb").unwrap();
    let w = v.clone();
    drop(v);
    assert_eq!(strs(&w), [&b"a"[..], b"bb", b"ccc"]);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopts_an_allocated_array_of_no_strings() {
    init();
    let arr = c_array(&[], 1);
    // SAFETY: a `git__malloc` array holding no strings, handed over whole.
    let v = unsafe { OdbDataStrVec::from_raw_parts(arr, 0) }.unwrap();
    assert!(v.is_empty() && v.as_slice().is_empty());
    assert_eq!(v.as_ptr(), arr, "the array is owned, and freed on drop");
    assert!(v.clone().as_ptr().is_null());
    assert!(v.into_vec().is_empty());
}

#[test]
fn null_arrays() {
    // SAFETY: NULL with no count is a zeroed `git_strarray`'s pair.
    let v = unsafe { OdbDataStrVec::from_raw_parts(ptr::null_mut(), 0) }.unwrap();
    assert!(v.is_empty() && v.as_ptr().is_null());
    for count in [1, 7, usize::MAX] {
        // SAFETY: a NULL array is refused before any element is read.
        assert!(unsafe { OdbDataStrVec::from_raw_parts(ptr::null_mut(), count) }.is_none());
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn refuses_a_null_element_and_leaves_ownership() {
    init();
    for at in 0..3 {
        let mut items = vec![Some(c"x"), Some(c"y")];
        items.insert(at, None);
        let arr = c_array(&items, 3);
        // SAFETY: a `git__malloc` array of 3 slots, owned strings or NULL.
        assert!(unsafe { OdbDataStrVec::from_raw_parts(arr, 3) }.is_none());
        // Still the caller's: libgit2's disposer releases it; ASan flags a leak or a
        // double free.
        let mut array = ffi::git_strarray {
            strings: arr,
            count: 3,
        };
        // SAFETY: `array` owns its array and strings; `git__free(NULL)` is a no-op.
        unsafe { ffi::git_strarray_dispose(&mut array) };
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopts_what_libgit2_hands_out() {
    init();
    let mut out = ffi::git_strarray {
        strings: ptr::null_mut(),
        count: 0,
    };
    // SAFETY: `GIT_OPT_GET_EXTENSIONS` takes a `git_strarray *`, which it fills from a
    // `git_vector`, so the array may have spare capacity.
    let rc = unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_GET_EXTENSIONS as c_int, &raw mut out) };
    assert_eq!(rc, 0);
    // SAFETY: libgit2's output, owned by the caller and handed over whole.
    let v = unsafe { OdbDataStrVec::from_raw_parts(out.strings, out.count) }.unwrap();
    assert!(!v.is_empty());
    assert!(strs(&v).contains(&&b"noop"[..]));
    let mut owned = v.into_vec();
    owned.retain(|s| s.as_bytes() != b"noop");
    let w = OdbDataStrVec::from_vec(owned).unwrap();
    // An adopted run crosses threads like any other.
    thread::spawn(move || assert!(!strs(&w).contains(&&b"noop"[..])))
        .join()
        .unwrap();
}
