//! An `extensions()` whose copy fails part-way leaks what it had copied
//! (LeakSanitizer): a C defect the wrapper cannot repair.
//!
//! `GIT_OPT_GET_EXTENSIONS` builds the answer in a `git_vector` of `git__strdup`
//! copies and, when one copy fails, returns -1 leaving the vector and the copies made
//! so far allocated (`git_repository__extensions`, `src/libgit2/repository.c`). The
//! `unsafe` calls below honour their contracts: the allocator is installed before
//! libgit2 initialises and is a libc family, and no other thread runs.
//!
//! Build and run against the `asan-ubsan` variant, from this directory:
//!
//! ```sh
//! CRUSTIFY_BUILD_VARIANT=asan-ubsan \
//! CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Zsanitizer=address \
//! ASAN_OPTIONS=detect_leaks=1:abort_on_error=1 \
//! LD_LIBRARY_PATH=/target/crustify/builds/asan-ubsan \
//! cargo +nightly run --target x86_64-unknown-linux-gnu
//! ```
//!
//! Exits non-zero with `LeakSanitizer: detected memory leaks` (blocks allocated by
//! `git_vector_init` and `git__strdup` under `git_repository__extensions`).

use std::ffi::{CStr, c_void};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

use libgit2::libgit2::core::common;
use libgit2::libgit2::util::errors::ErrorClass;
use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator};

/// Requests still granted before the allocator refuses; `usize::MAX` grants all.
static GRANTS: AtomicUsize = AtomicUsize::new(usize::MAX);

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

fn granted() -> bool {
    GRANTS
        .try_update(SeqCst, SeqCst, |n| {
            if n == usize::MAX {
                Some(n)
            } else {
                n.checked_sub(1)
            }
        })
        .is_ok()
}

/// libc's allocator, refusing every request once `GRANTS` runs out.
struct Counting;

// SAFETY: forwards to libc's `malloc` family unchanged, or fails, which `Alloc` allows.
unsafe impl Alloc for Counting {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        if !granted() {
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
        if !granted() {
            return None;
        }
        let p = p.map_or(std::ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len) }.cast())
    }
    unsafe fn free(p: NonNull<u8>) {
        // SAFETY: `p` is a live libc block the caller gives up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

static COUNTING: Allocator = Allocator::new::<Counting>();

fn main() {
    // SAFETY: nothing has initialised libgit2 in this process; no other thread runs.
    unsafe { common::set_allocator(Some(COUNTING.as_ref())) };

    // Initialises libgit2 and allocates this thread's error state, so the failure
    // below is reported as out of memory rather than as missing thread state.
    assert_eq!(common::extensions().unwrap().as_ref().len(), 6);
    // SAFETY: no other thread runs.
    let e = unsafe { common::set_server_timeout(-1) }.unwrap_err();
    assert_eq!(e.class(), Some(ErrorClass::Invalid));

    // The vector and the first two copies succeed; the third copy is refused.
    GRANTS.store(3, SeqCst);
    let e = common::extensions().unwrap_err();
    GRANTS.store(usize::MAX, SeqCst);
    assert_eq!(e.class(), Some(ErrorClass::NoMemory), "{e:?}");
    println!("the partial copy failed as expected: {e}; LeakSanitizer reports what it left behind at exit");
}
