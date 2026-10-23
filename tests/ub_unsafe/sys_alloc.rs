//! `git2/sys/alloc.h`: the `unsafe extern "C"` members an `Allocator` built by
//! `Allocator::new` holds, called within the contracts `MallocFn`, `ReallocFn` and
//! `FreeFn` state, edge cases included. One suite per member.
//!
//! The implementation behind them is libc's allocator, which Miri models, so these run
//! there too; ASan/LSan flag a trampoline that loses, leaks or double-frees a block.

use core::ffi::{CStr, c_void};
use core::ptr::{self, NonNull};
use core::slice;

use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator, FreeFn, MallocFn, ReallocFn};

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// libc's allocator behind `Alloc`, refusing [`REFUSED`] bytes: a request libc cannot
/// meet would abort under ASan (`allocator_may_return_null=0`) instead of failing.
struct Libc;

const REFUSED: usize = 1 << 40;

// SAFETY: forwards to libc's `malloc` family unchanged, which meets `Alloc`'s contract.
unsafe impl Alloc for Libc {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        if len == REFUSED {
            return None;
        }
        // SAFETY: plain libc allocation.
        NonNull::new(unsafe { malloc(len) }.cast())
    }

    unsafe fn realloc(
        p: Option<NonNull<u8>>,
        len: usize,
        _: Option<&CStr>,
        _: i32,
    ) -> Option<NonNull<u8>> {
        if len == REFUSED {
            return None;
        }
        let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len) }.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        // SAFETY: `p` is a live libc block the caller gives up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

static LIBC: Allocator = Allocator::new::<Libc>();

fn members() -> (MallocFn, ReallocFn, FreeFn) {
    let r = LIBC.as_ref();
    (r.malloc().unwrap(), r.realloc().unwrap(), r.free().unwrap())
}

/// Fills `len` bytes at `p` with a pattern and checks them back.
///
/// # Safety
///
/// `p` is valid for writes of `len` bytes.
unsafe fn scribble(p: *mut c_void, len: usize, seed: u8) {
    let p = p.cast::<u8>();
    for i in 0..len {
        // SAFETY: `i < len`.
        unsafe { p.add(i).write(seed.wrapping_add(i as u8)) };
    }
    // SAFETY: the `len` bytes were just written.
    let bytes = unsafe { slice::from_raw_parts(p, len) };
    assert!(
        bytes
            .iter()
            .enumerate()
            .all(|(i, &b)| b == seed.wrapping_add(i as u8))
    );
}

#[test]
fn malloc_member_returns_writable_aligned_blocks() {
    let (malloc, _, free) = members();
    for (len, file) in [
        (1, ptr::null()),
        (24, c"alloc.c".as_ptr()),
        (4096, c"".as_ptr()),
    ] {
        // SAFETY: `file` is NULL or a live C string; the block is released below.
        let p = unsafe { malloc(len, file, 42) };
        assert!(!p.is_null());
        let align = (1usize << len.ilog2()).min(16);
        assert_eq!(p as usize % align, 0, "aligned as malloc aligns");
        // SAFETY: `p` holds `len` writable bytes.
        unsafe { scribble(p, len, len as u8) };
        // SAFETY: `p` is a live block of this family, given up.
        unsafe { free(p) };
    }
}

#[test]
fn malloc_member_accepts_zero_bytes() {
    let (malloc, _, free) = members();
    // SAFETY: NULL `file`; a non-NULL result is a block released below.
    let p = unsafe { malloc(0, ptr::null(), 0) };
    // SAFETY: NULL or a live block of this family, given up.
    unsafe { free(p) };
}

/// Zero-byte requests, through either allocating member, are allocations of their own:
/// resized and released like any other. A live block is never resized to zero bytes,
/// which the contract excludes (glibc would release it and return NULL).
#[test]
fn zero_byte_requests_are_allocations() {
    let (malloc, realloc, free) = members();
    // SAFETY: NULL `file`; each non-NULL result is a block released below.
    let (a, b) = unsafe {
        (
            malloc(0, ptr::null(), 0),
            realloc(ptr::null_mut(), 0, c"".as_ptr(), 0),
        )
    };
    // SAFETY: `a` is NULL or a live block of this family, given up on success.
    let grown = unsafe { realloc(a, 8, ptr::null(), 0) };
    assert!(!grown.is_null());
    // SAFETY: `grown` holds 8 writable bytes.
    unsafe { scribble(grown, 8, 3) };
    // SAFETY: `grown` is a live block; `b` is NULL or one; each given up once.
    unsafe {
        free(grown);
        free(b);
    }
}

#[test]
fn malloc_member_reports_failure_as_null() {
    let (malloc, _, _) = members();
    // SAFETY: NULL `file`; the implementation refuses the request.
    assert!(unsafe { malloc(REFUSED, ptr::null(), 0) }.is_null());
}

#[test]
fn realloc_member_resizes_and_keeps_the_prefix() {
    let (_, realloc, free) = members();
    // SAFETY: NULL allocates afresh, as `malloc`; `file` is a live C string.
    let p = unsafe { realloc(ptr::null_mut(), 8, c"str.c".as_ptr(), 1) };
    assert!(!p.is_null());
    // SAFETY: `p` holds 8 writable bytes.
    unsafe { scribble(p, 8, 7) };
    // SAFETY: `p` is a live block of this family, given up on success.
    let grown = unsafe { realloc(p, 1 << 16, ptr::null(), 2) };
    assert!(!grown.is_null());
    // SAFETY: the first 8 bytes were carried over.
    let kept = unsafe { slice::from_raw_parts(grown.cast::<u8>(), 8) };
    assert_eq!(kept, [7, 8, 9, 10, 11, 12, 13, 14]);
    // SAFETY: as above, shrinking.
    let shrunk = unsafe { realloc(grown, 3, ptr::null(), 3) };
    assert!(!shrunk.is_null());
    // SAFETY: the first 3 bytes were carried over.
    let kept = unsafe { slice::from_raw_parts(shrunk.cast::<u8>(), 3) };
    assert_eq!(kept, [7, 8, 9]);
    // SAFETY: `shrunk` is a live block of this family, given up.
    unsafe { free(shrunk) };
}

#[test]
fn realloc_member_failure_leaves_the_block_live() {
    let (_, realloc, free) = members();
    // SAFETY: NULL allocates afresh.
    let p = unsafe { realloc(ptr::null_mut(), 16, ptr::null(), 0) };
    assert!(!p.is_null());
    // SAFETY: `p` holds 16 writable bytes.
    unsafe { scribble(p, 16, 1) };
    // SAFETY: `p` is a live block; the implementation refuses the request.
    assert!(unsafe { realloc(p, REFUSED, ptr::null(), 0) }.is_null());
    // SAFETY: `p` is still live and unchanged.
    unsafe { scribble(p, 16, 1) };
    // SAFETY: `p` is a live block of this family, given up.
    unsafe { free(p) };
}

#[test]
fn free_member_ignores_null() {
    let (_, _, free) = members();
    // SAFETY: NULL is a no-op.
    unsafe { free(ptr::null_mut()) };
}

#[test]
fn free_member_releases_blocks_from_either_allocating_member() {
    let (malloc, realloc, free) = members();
    // SAFETY: NULL `file`; both blocks are released below.
    let (a, b) = unsafe {
        (
            malloc(32, ptr::null(), 0),
            realloc(ptr::null_mut(), 32, ptr::null(), 0),
        )
    };
    assert!(!a.is_null() && !b.is_null());
    // SAFETY: two distinct live blocks of this family, each given up once.
    unsafe {
        free(a);
        free(b);
    }
}
