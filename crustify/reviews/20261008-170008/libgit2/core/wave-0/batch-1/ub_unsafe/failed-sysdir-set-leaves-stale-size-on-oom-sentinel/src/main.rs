//! A `set_search_path` (or `set_template_path`, `set_homedir`) whose copy fails for
//! want of memory leaves the setting unreadable: the next safe `search_path()` reads
//! past the end of a one-byte static (AddressSanitizer, global-buffer-overflow).
//!
//! `git_sysdir_set` copies the path with `git_str_sets`; when the copy cannot grow
//! the level's buffer, `git_str_try_grow` frees the old string, points the buffer at
//! the one-byte sentinel `git_str__oom` and leaves `size` as it was
//! (`src/util/str.c`). `GIT_OPT_GET_SEARCH_PATH` then copies `size` bytes from that
//! sentinel (`git_str_put(&str, tmp->ptr, tmp->size)`, `src/libgit2/settings.c`).
//! Both `unsafe` calls below honour their contracts: the allocator is installed before
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
//! Aborts with `AddressSanitizer: global-buffer-overflow` (read in `git_str_put`
//! under `git_libgit2_opts`, of `git_str__oom`) on the affected commit; exits 0 once
//! the setters clear the level after a failed copy.

use std::ffi::{CStr, CString, c_void};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

use libgit2::libgit2::core::common::{self, SearchPathLevel};
use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator};

static REFUSE: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// libc's allocator, refusing every request while `REFUSE` is set.
struct Refusable;

// SAFETY: forwards to libc's `malloc` family unchanged, or fails, which `Alloc` allows.
unsafe impl Alloc for Refusable {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        if REFUSE.load(SeqCst) {
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
        if REFUSE.load(SeqCst) {
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

static REFUSABLE: Allocator = Allocator::new::<Refusable>();

fn main() {
    // SAFETY: nothing has initialised libgit2 in this process; no other thread runs.
    unsafe { common::set_allocator(Some(REFUSABLE.as_ref())) };

    // Initialises libgit2 (pins the runtime) and gives the level a value of its own,
    // long enough that the failing copy below must grow the buffer.
    let level = SearchPathLevel::Global;
    let before = CString::new("/".repeat(64)).unwrap();
    // SAFETY: no other thread runs.
    unsafe { common::set_search_path(level, Some(&before)) }.unwrap();
    assert_eq!(common::search_path(level).unwrap(), before);

    REFUSE.store(true, SeqCst);
    let longer = CString::new("/".repeat(4096)).unwrap();
    // SAFETY: no other thread runs.
    let err = unsafe { common::set_search_path(level, Some(&longer)) }.unwrap_err();
    REFUSE.store(false, SeqCst);
    println!("the failed copy reported: {err}");

    // Memory is back; the previous value, or at worst an empty one, should read.
    let after = common::search_path(level).unwrap();
    println!("the level now reads as {after:?}");
}
