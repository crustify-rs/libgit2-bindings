//! The suite's fixtures. `unsafe` is confined here, standing in for safe constructors
//! the crate does not have yet; replace each once its safe counterpart lands.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_int, c_void};
use std::mem::MaybeUninit;
use std::process::Command;

use libgit2::libgit2::raw::OdbDataVec;

const CHILD: &str = "CRUSTIFY_UB_SAFE_CHILD";

/// Runs `body` alone in a child process, where nothing has initialised libgit2.
pub fn isolated(test: &str, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child `{name}` failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `git_error_last()`'s class and message, copied out; stands in for the safe
/// `git_error_last` wrapper.
pub fn last_error() -> (c_int, Vec<u8>) {
    // SAFETY: `git_error_last` never returns NULL; its message lives until the next
    // libgit2 call on this thread, and is copied out before one.
    unsafe {
        let e = libgit2::ffi::git_error_last();
        ((*e).klass, CStr::from_ptr((*e).message).to_bytes().to_vec())
    }
}

/// Promotes a run every element of which the test wrote; stands in for the typed
/// constructors that fill a run in one safe call.
pub fn filled<T>(v: OdbDataVec<MaybeUninit<T>>) -> OdbDataVec<T> {
    // SAFETY: the caller wrote every element, which is this fixture's contract.
    unsafe { v.assume_init() }
}

/// A plain C struct to place in an `OdbDataBox`; the crate wraps no C type yet.
///
/// Replace with a wrapped libgit2 type once one is resource-free and `CZeroable`.
#[repr(C)]
pub struct pair_st {
    pub a: u64,
    pub b: u32,
}
ffibox::define_ctype!(Pair, PairRef, PairMut, pair_st);

// SAFETY: any `a` and `b` are valid, zero included; nothing tears a pair down.
unsafe impl ffibox::CZeroable for Pair {}
// SAFETY: a pair is plain data with no thread affinity; its handles keep Rust's
// borrow rules, so moving or sharing one across threads races nothing.
unsafe impl Send for Pair {}
// SAFETY: as above.
unsafe impl Sync for Pair {}

impl PairRef<'_> {
    pub fn a(&self) -> u64 {
        // SAFETY: a read through the shared handle's pointer to a live `pair_st`.
        unsafe { core::ptr::addr_of!((*self.as_ptr()).a).read() }
    }
    pub fn b(&self) -> u32 {
        // SAFETY: as above.
        unsafe { core::ptr::addr_of!((*self.as_ptr()).b).read() }
    }
}

impl PairMut<'_> {
    pub fn set(&mut self, a: u64, b: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to a live `pair_st`.
        unsafe {
            core::ptr::addr_of_mut!((*p).a).write(a);
            core::ptr::addr_of_mut!((*p).b).write(b);
        }
    }
}

// --- `git2/sys/alloc.h` ------------------------------------------------------------

use core::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

use libgit2::ffi;
use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator};

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

static MALLOCS: AtomicUsize = AtomicUsize::new(0);
static REALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);

/// libc's allocator behind `Alloc`, counting the calls that reach it. Storage is
/// libc's either way, so it may be installed while libc-allocated blocks are live.
///
/// It refuses [`REFUSED`] bytes: a request libc cannot meet would abort under ASan
/// (`allocator_may_return_null=0`) instead of failing.
pub struct Counted;

/// The one request size [`Counted`] refuses.
pub const REFUSED: usize = 1 << 40;

impl Counted {
    /// `(malloc, realloc, free)` calls so far, process-wide.
    pub fn calls() -> (usize, usize, usize) {
        (
            MALLOCS.load(SeqCst),
            REALLOCS.load(SeqCst),
            FREES.load(SeqCst),
        )
    }
}

// SAFETY: forwards to libc's `malloc` family unchanged, which meets every clause of
// `Alloc`'s contract; the counters are atomics.
unsafe impl Alloc for Counted {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        MALLOCS.fetch_add(1, SeqCst);
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
        REALLOCS.fetch_add(1, SeqCst);
        if len == REFUSED {
            return None;
        }
        let p = p.map_or(core::ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len) }.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        FREES.fetch_add(1, SeqCst);
        // SAFETY: `p` is a live libc block the caller gives up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

/// A `git_allocator.gfree` that releases nothing, to store in an `Allocator`.
pub unsafe extern "C" fn leave_free(_: *mut c_void) {}

/// Installs `allocator` as libgit2's allocator (`GIT_OPT_SET_ALLOCATOR`); stands in
/// for the `git_libgit2_opts` wrapper (`core`), which replaces it.
///
/// Callers install a libc-backed family ([`Counted`]) or install before libgit2 has
/// allocated, so no live block changes family.
pub fn install(allocator: &Allocator) {
    let raw = (&raw const *allocator)
        .cast::<ffi::git_allocator>()
        .cast_mut();
    // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, so `raw`
    // addresses one with every member set; libgit2 copies it (`memcpy`) and keeps no
    // pointer to it. The family is coherent per this function's contract.
    let rc = unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR as c_int, raw) };
    assert_eq!(rc, 0);
}
