//! Follows the contract `ReallocFn` and `Alloc::realloc` state, to the letter, on the
//! allocator the `Allocator::new` docs recommend (libc behind `Alloc`, "which already
//! meets `Alloc`'s contract"): a NULL result from `realloc` "is a failure, and `ptr`
//! stays live", so the caller releases `ptr` afterwards. For a zero-byte resize glibc
//! has already released it: ASan reports a double free.
//!
//! Run from this directory with the ASan build:
//!
//! ```sh
//! CRUSTIFY_BUILD_VARIANT=asan-ubsan RUSTFLAGS=-Zsanitizer=address \
//! LD_LIBRARY_PATH=/target/crustify/builds/asan-ubsan \
//! ASAN_SYMBOLIZER_PATH=/usr/bin/llvm-symbolizer-18 \
//!   cargo +nightly run --target x86_64-unknown-linux-gnu
//! ```

use core::ffi::{CStr, c_void};
use core::ptr::{self, NonNull};

use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator};

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// libc's allocator, as the `Allocator::new` doc example writes it.
struct Libc;

// SAFETY: forwards to libc's `malloc` family, unchanged.
unsafe impl Alloc for Libc {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        NonNull::new(unsafe { malloc(len) }.cast())
    }
    unsafe fn realloc(
        p: Option<NonNull<u8>>,
        len: usize,
        _: Option<&CStr>,
        _: i32,
    ) -> Option<NonNull<u8>> {
        let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
        NonNull::new(unsafe { realloc(p, len) }.cast())
    }
    unsafe fn free(p: NonNull<u8>) {
        unsafe { free(p.as_ptr().cast()) }
    }
}

static LIBC: Allocator = Allocator::new::<Libc>();

fn main() {
    let r = LIBC.as_ref();
    let (malloc, realloc, free) = (r.malloc().unwrap(), r.realloc().unwrap(), r.free().unwrap());

    // SAFETY: NULL `file`; a live block of this family.
    let p = unsafe { malloc(16, ptr::null(), 0) };
    assert!(!p.is_null());

    // SAFETY: `p` is a live allocation of this family, which the caller owns and, on
    // success, gives up — `ReallocFn`'s whole stated contract.
    let q = unsafe { realloc(p, 0, ptr::null(), 0) };

    if q.is_null() {
        // Per `ReallocFn`: "on failure NULL is returned and `ptr` stays live", and per
        // `Alloc::realloc`: "returns `None` and leaves `ptr` live and unchanged".
        // SAFETY: as the contract states, `p` is still a live block of this family.
        unsafe { free(p) };
        println!("realloc(p, 0) returned NULL: freed p as the contract allows");
    } else {
        // SAFETY: `q` is a live block of this family.
        unsafe { free(q) };
        println!("realloc(p, 0) returned a block: no ambiguity on this libc");
    }
}
