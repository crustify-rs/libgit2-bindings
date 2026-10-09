//! `Signature::parse` refuses a signature whose name or email libgit2 lost to an
//! allocation failure (`git_signature__parse` never checks `extract_trimmed`), and
//! documents the refusal as carrying the out-of-memory error. It read the thread's
//! last error for that, but the parser keeps going after the lost string: a malformed
//! timezone (`+abc`) makes `git__strntol32` set a "failed to convert" error, which the
//! parser ignores before succeeding, so that stale conversion error is what the
//! refusal carried. Exits non-zero on the affected commit, with the error it got;
//! prints the out-of-memory error and exits 0 once `constructed` sets it itself.
//!
//! Run with `LD_LIBRARY_PATH=/target/crustify/builds/plain CRUSTIFY_BUILD_VARIANT=plain`.

use core::ffi::CStr;
use core::ptr::NonNull;
use std::sync::atomic::{AtomicIsize, Ordering::SeqCst};

use libgit2::libgit2::core::common::set_allocator;
use libgit2::libgit2::core::types::Signature;
use libgit2::libgit2::util::errors::ErrorClass;
use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator};
use libgit2::libgit2::util::sys::errors::set;

/// Requests still granted before one is refused; negative grants all.
static GRANTS: AtomicIsize = AtomicIsize::new(-1);

unsafe extern "C" {
    fn malloc(n: usize) -> *mut core::ffi::c_void;
    fn realloc(p: *mut core::ffi::c_void, n: usize) -> *mut core::ffi::c_void;
    fn free(p: *mut core::ffi::c_void);
}

/// libc's allocator, refusing the one request `GRANTS` counts down to.
struct Failing;

impl Failing {
    fn refuse() -> bool {
        GRANTS.fetch_sub(1, SeqCst) == 0
    }
}

// SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
unsafe impl Alloc for Failing {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        if Self::refuse() {
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
        if Self::refuse() {
            return None;
        }
        let p = p.map_or(core::ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len) }.cast())
    }
    unsafe fn free(p: NonNull<u8>) {
        // SAFETY: `p` is a live libc block the caller gives up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

fn main() {
    let allocator = Allocator::new::<Failing>();
    // SAFETY: one coherent family over libc, installed before anything in this process
    // has called into libgit2, on the only thread.
    unsafe { set_allocator(Some(allocator.as_ref())) };

    // Initialises libgit2 and allocates this thread's error state while every
    // request is still granted.
    let whole = Signature::parse(c"A U Thor <a@b> 12 +abc").expect("a well-formed line");
    println!("with memory: {:?}", whole.as_ref());
    set(ErrorClass::Invalid, c"primed");

    // The struct is granted, the name refused: libgit2 parses on, sets a conversion
    // error for `+abc`, and reports success with a NULL name.
    GRANTS.store(1, SeqCst);
    let refused = Signature::parse(c"A U Thor <a@b> 12 +abc");
    GRANTS.store(-1, SeqCst);
    let error = refused.expect_err("a lost name is refused");
    println!(
        "refused with class {:?}, message {:?}",
        error.class(),
        error.message()
    );
    assert_eq!(
        (error.class(), error.message()),
        (Some(ErrorClass::NoMemory), c"Out of memory"),
        "the refusal must carry the allocation failure that lost the name"
    );
}
