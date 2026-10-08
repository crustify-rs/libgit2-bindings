//! UB tests for the unsafe API of `git2/sys/odb_backend.h`'s string strategies:
//! `OdbDataStrVec::from_raw_parts`, called within its contract on every kind of
//! array libgit2 or the safe API hands out, and `OdbDataStrFree::take_buf` on every
//! shape of `git_buf` libgit2 leaves behind, edge cases included.

use std::ffi::{CStr, c_char, c_int};
use std::mem::size_of;
use std::ptr;
use std::sync::Mutex;
use std::thread;

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

// --- OdbDataStrFree::take_buf -----------------------------------------------------

fn zeroed_buf() -> ffi::git_buf {
    ffi::git_buf {
        ptr: ptr::null_mut(),
        reserved: 0,
        size: 0,
    }
}

fn is_disposed(buf: &ffi::git_buf) -> bool {
    buf.ptr.is_null() && buf.reserved == 0 && buf.size == 0
}

/// `GIT_OPT_GET_USER_AGENT` into `buf`: an owned string.
fn get_user_agent(buf: &mut ffi::git_buf) {
    // SAFETY: `GIT_OPT_GET_USER_AGENT` takes a `git_buf *` that is zeroed or as a
    // libgit2 routine left it, which it fills.
    let rc = unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_GET_USER_AGENT as c_int, &raw mut *buf) };
    assert_eq!(rc, 0);
}

/// Serialises the tests that set the template path, a libgit2 global written
/// without a lock.
static TEMPLATE_PATH: Mutex<()> = Mutex::new(());

/// `GIT_OPT_GET_TEMPLATE_PATH` into `buf` with the path set to `path`; the empty
/// path is filled without allocating. Hold [`TEMPLATE_PATH`] around it and the
/// matching [`reset_template_path`].
fn get_template_path(buf: &mut ffi::git_buf, path: &CStr) {
    // SAFETY: `GIT_OPT_SET_TEMPLATE_PATH` copies its `const char *`.
    let rc =
        unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_SET_TEMPLATE_PATH as c_int, path.as_ptr()) };
    assert_eq!(rc, 0);
    // SAFETY: as `get_user_agent`.
    let rc =
        unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_GET_TEMPLATE_PATH as c_int, &raw mut *buf) };
    assert_eq!(rc, 0);
}

/// Restores the guessed default template path.
fn reset_template_path() {
    // SAFETY: NULL resets the template path.
    let rc = unsafe {
        ffi::git_libgit2_opts(
            ffi::GIT_OPT_SET_TEMPLATE_PATH as c_int,
            ptr::null::<c_char>(),
        )
    };
    assert_eq!(rc, 0);
}

fn dispose(buf: &mut ffi::git_buf) {
    // SAFETY: `buf` is zeroed or as a libgit2 routine left it.
    unsafe { ffi::git_buf_dispose(buf) }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn take_buf_adopts_what_libgit2_owns() {
    init();
    let mut buf = zeroed_buf();
    get_user_agent(&mut buf);
    assert!(buf.reserved > 0, "the buffer owns its string");
    let (ptr, size) = (buf.ptr, buf.size);
    // SAFETY: `buf` is as `GIT_OPT_GET_USER_AGENT` left it, and is not read again
    // through its old pointer.
    let s = unsafe { OdbDataStrFree::take_buf(&mut buf) }.unwrap();
    assert!(is_disposed(&buf), "the buffer is left disposed");
    assert_eq!(
        s.as_ptr(),
        ptr.cast_const(),
        "the string is moved, not copied"
    );
    assert_eq!(s.len(), size);
    assert!(s.to_str().unwrap().contains("libgit2"));
    // A disposed buffer owns nothing: libgit2's disposer frees nothing; ASan flags a
    // double free, and a leak if `s` is not released.
    dispose(&mut buf);
    let t = s.clone();
    drop(s);
    assert_eq!(t.len(), size);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn take_buf_leaves_the_static_empty_string_alone() {
    init();
    let _path = TEMPLATE_PATH.lock().unwrap();
    let mut buf = zeroed_buf();
    get_template_path(&mut buf, c"");
    assert!(
        !buf.ptr.is_null(),
        "an empty result points at libgit2's static empty string"
    );
    assert_eq!(
        (buf.reserved, buf.size),
        (0, 0),
        "which the buffer does not own"
    );
    // SAFETY: `buf` is as `GIT_OPT_GET_TEMPLATE_PATH` left it.
    assert!(unsafe { OdbDataStrFree::take_buf(&mut buf) }.is_none());
    assert!(is_disposed(&buf));
    dispose(&mut buf);
    reset_template_path();
}

#[test]
fn take_buf_of_a_zeroed_buffer_is_none() {
    let mut buf = zeroed_buf();
    // SAFETY: a zeroed `git_buf` is `GIT_BUF_INIT`.
    assert!(unsafe { OdbDataStrFree::take_buf(&mut buf) }.is_none());
    assert!(is_disposed(&buf));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn take_buf_then_reuse_the_buffer() {
    init();
    let _path = TEMPLATE_PATH.lock().unwrap();
    let mut buf = zeroed_buf();
    let mut taken = Vec::new();
    for round in 0..3 {
        get_user_agent(&mut buf);
        // SAFETY: `buf` is as libgit2 left it: filled on top of a disposed buffer.
        let s = unsafe { OdbDataStrFree::take_buf(&mut buf) }.unwrap();
        assert!(
            taken.iter().all(|t: &OdbDataStr| t.as_ptr() != s.as_ptr()),
            "round {round}"
        );
        taken.push(s);
    }
    // Filled, then taken, then filled by a routine that allocates nothing: the
    // buffer never holds a stale pointer in between.
    get_template_path(&mut buf, c"");
    // SAFETY: as above.
    assert!(unsafe { OdbDataStrFree::take_buf(&mut buf) }.is_none());
    get_user_agent(&mut buf);
    dispose(&mut buf);
    // `git_buf_dispose` points a disposed buffer at the static empty string.
    assert!(!buf.ptr.is_null() && buf.reserved == 0 && buf.size == 0);
    // SAFETY: `buf` is as `git_buf_dispose` left it.
    assert!(unsafe { OdbDataStrFree::take_buf(&mut buf) }.is_none());
    reset_template_path();
    // Taken strings outlive the buffer and cross threads.
    let n = thread::spawn(move || taken.into_iter().map(|s| s.len()).sum::<usize>())
        .join()
        .unwrap();
    assert!(n > 0);
}
