//! Equivalence of `git2/sys/odb_backend.h`'s raw-lifetime strategies with the C
//! routines they call, `git_odb_backend_data_alloc` / `git_odb_backend_data_free`.
//!
//! libgit2's allocator and initialisation are process-global, so the test reruns
//! itself alone in a child process, which installs a tracing `git_allocator` before
//! `git_libgit2_init`. The C side passes a real backend; the Rust strategies pass
//! NULL, which both routines ignore.
//!
//! Documented divergences, outside these comparisons:
//!
//! - An empty Rust buffer is NULL and never reaches the allocator, where C would call
//!   `git_odb_backend_data_alloc(backend, 0)`.
//! - Before `git_libgit2_init`, C gets NULL after `git_error_set_oom` writes through
//!   a nonexistent TLS key; the strategies pin the runtime first and succeed. The
//!   corrected behaviour is regression-tested in the module's `unit_tests` and in
//!   `ub_safe`.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::size_of;
use std::process::Command;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use libgit2::ffi;
use libgit2::libgit2::raw::{OdbDataArrayFree, OdbDataBox, OdbDataVec};

#[repr(C)]
pub struct pair_st {
    pub a: u64,
    pub b: u32,
}
ffibox::define_ctype!(Pair, PairRef, PairMut, pair_st);
// SAFETY: any `a` and `b` are valid, zero included; nothing tears a pair down.
unsafe impl ffibox::CZeroable for Pair {}

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process, so no other test shares libgit2's globals.
fn isolated(test: &str, body: fn()) {
    let name = test.strip_prefix("equiv::").unwrap_or(test);
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

// --- the tracing allocator ------------------------------------------------------

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

#[derive(Debug)]
enum Raw {
    Alloc { ptr: usize, len: usize, file: String, line: c_int },
    Free { ptr: usize },
}

static TRACING: AtomicBool = AtomicBool::new(false);
static FAIL_NEXT: AtomicBool = AtomicBool::new(false);
static TRACE: Mutex<Vec<Raw>> = Mutex::new(Vec::new());
static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn traced_malloc(len: usize, file: *const c_char, line: c_int) -> *mut c_void {
    let ptr = if FAIL_NEXT.swap(false, Ordering::SeqCst) {
        ptr::null_mut()
    } else {
        // SAFETY: plain libc allocation.
        unsafe { malloc(len) }
    };
    if TRACING.load(Ordering::SeqCst) {
        if !ptr.is_null() {
            LIVE.fetch_add(1, Ordering::SeqCst);
        }
        // SAFETY: libgit2 passes `__FILE__`, a NUL-terminated literal.
        let file = unsafe { CStr::from_ptr(file) }.to_string_lossy();
        let file = file.rsplit('/').next().unwrap().to_owned();
        TRACE.lock().unwrap().push(Raw::Alloc { ptr: ptr as usize, len, file, line });
    }
    ptr
}

unsafe extern "C" fn traced_realloc(
    ptr: *mut c_void,
    len: usize,
    _file: *const c_char,
    _line: c_int,
) -> *mut c_void {
    assert!(!TRACING.load(Ordering::SeqCst), "no traced step reallocates");
    // SAFETY: `ptr` came from `traced_malloc` / `traced_realloc`, i.e. libc.
    unsafe { realloc(ptr, len) }
}

unsafe extern "C" fn traced_free(ptr: *mut c_void) {
    if TRACING.load(Ordering::SeqCst) && !ptr.is_null() {
        LIVE.fetch_sub(1, Ordering::SeqCst);
        TRACE.lock().unwrap().push(Raw::Free { ptr: ptr as usize });
    }
    // SAFETY: `ptr` came from libc through this allocator, or is NULL.
    unsafe { free(ptr) }
}

/// Installs the tracing allocator, then initialises libgit2 on top of it.
fn install_tracing_allocator() {
    let mut allocator = ffi::git_allocator {
        gmalloc: Some(traced_malloc),
        grealloc: Some(traced_realloc),
        gfree: Some(traced_free),
    };
    // SAFETY: `GIT_OPT_SET_ALLOCATOR` takes a `git_allocator *` with every member set,
    // which it copies; nothing is allocated yet, so no storage changes allocator.
    let rc = unsafe {
        ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR as c_int, &raw mut allocator)
    };
    assert_eq!(rc, 0);
    // SAFETY: no arguments; refcounted, never released in this process.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
    // Warm this thread's error state, which libgit2 allocates on first use.
    last_error();
}

/// One allocator event, with pointers replaced by the index of their allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Alloc { len: usize, file: String, line: c_int },
    AllocFailed { len: usize, file: String, line: c_int },
    Free { alloc: usize },
}

/// Records one sequence of steps, each step's events normalised separately.
#[derive(Default)]
struct Recorder {
    steps: Vec<Vec<Event>>,
    allocs: HashMap<usize, usize>,
    count: usize,
}

impl Recorder {
    fn step<R>(&mut self, f: impl FnOnce() -> R) -> R {
        TRACE.lock().unwrap().clear();
        TRACING.store(true, Ordering::SeqCst);
        let r = f();
        TRACING.store(false, Ordering::SeqCst);
        let raw = std::mem::take(&mut *TRACE.lock().unwrap());
        let events = raw
            .into_iter()
            .map(|e| match e {
                Raw::Alloc { ptr: 0, len, file, line } => Event::AllocFailed { len, file, line },
                Raw::Alloc { ptr, len, file, line } => {
                    self.allocs.insert(ptr, self.count);
                    self.count += 1;
                    Event::Alloc { len, file, line }
                }
                Raw::Free { ptr } => Event::Free {
                    alloc: self.allocs.remove(&ptr).expect("freed what the sequence allocated"),
                },
            })
            .collect();
        self.steps.push(events);
        r
    }
}

/// `git_error_last()`'s class and message.
fn last_error() -> (c_int, String) {
    // SAFETY: `git_error_last` never returns NULL; the error lives until the next
    // libgit2 call on this thread, and is copied out before one.
    unsafe {
        let e = ffi::git_error_last();
        ((*e).klass, CStr::from_ptr((*e).message).to_string_lossy().into_owned())
    }
}

fn clear_error() {
    // SAFETY: no arguments; resets this thread's error state.
    unsafe { ffi::git_error_clear() }
}

/// A backend the C side passes; both routines ignore it.
fn backend() -> ffi::git_odb_backend {
    ffi::git_odb_backend { version: 1, ..Default::default() }
}

fn c_alloc(backend: &mut ffi::git_odb_backend, len: usize) -> *mut u8 {
    // SAFETY: a valid backend and any length; returns fresh storage or NULL.
    unsafe { ffi::git_odb_backend_data_alloc(backend, len) }.cast()
}

fn c_free(backend: &mut ffi::git_odb_backend, data: *mut u8) {
    // SAFETY: callers pass a live `git_odb_backend_data_alloc` allocation once.
    unsafe { ffi::git_odb_backend_data_free(backend, data.cast()) }
}

fn c_read(data: *const u8, len: usize) -> Vec<u8> {
    // SAFETY: callers pass `len` initialised bytes.
    unsafe { std::slice::from_raw_parts(data, len) }.to_vec()
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 13 + 5) as u8).collect()
}

// --- the comparisons ------------------------------------------------------------

#[test]
fn traced_allocator_sequences() {
    isolated(concat!(module_path!(), "::traced_allocator_sequences"), || {
        install_tracing_allocator();
        for len in [1, 3, 64, 4096] {
            copy_then_release(len);
            clone_then_release_source_first(len);
        }
        uninit_run_filled_in_place();
        zeroed_run();
        zeroed_object();
        uninit_object_released_unfilled();
        allocation_failures();
        empty_runs();
        assert_eq!(LIVE.load(Ordering::SeqCst), 0, "every traced allocation was released");
    });
}

/// `memdup` then free: `from_slice` and drop.
fn copy_then_release(len: usize) {
    let src = pattern(len);
    let mut b = backend();
    let (mut c, mut r) = (Recorder::default(), Recorder::default());

    let p = c.step(|| {
        let p = c_alloc(&mut b, len);
        // SAFETY: `p` holds `len` writable bytes.
        unsafe { p.copy_from_nonoverlapping(src.as_ptr(), len) };
        p
    });
    let v = r.step(|| OdbDataVec::<u8>::from_slice(&src).unwrap());
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_read(p, len), v.as_slice());

    c.step(|| c_free(&mut b, p));
    r.step(|| drop(v));
    assert_eq!(c.steps, r.steps);
    assert!(matches!(c.steps[1][..], [Event::Free { alloc: 0 }]));
}

/// A copy of a copy, the source released first: `clone`.
fn clone_then_release_source_first(len: usize) {
    let src = pattern(len);
    let mut b = backend();
    let (mut c, mut r) = (Recorder::default(), Recorder::default());

    let p = c.step(|| {
        let p = c_alloc(&mut b, len);
        // SAFETY: `p` holds `len` writable bytes.
        unsafe { p.copy_from_nonoverlapping(src.as_ptr(), len) };
        p
    });
    let v = r.step(|| OdbDataVec::<u8>::from_slice(&src).unwrap());
    let q = c.step(|| {
        let q = c_alloc(&mut b, len);
        // SAFETY: both hold `len` bytes; distinct allocations.
        unsafe { q.copy_from_nonoverlapping(p, len) };
        q
    });
    let w = r.step(|| v.clone());
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_read(q, len), w.as_slice());

    c.step(|| c_free(&mut b, p));
    r.step(|| drop(v));
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_read(q, len), w.as_slice());

    c.step(|| c_free(&mut b, q));
    r.step(|| drop(w));
    assert_eq!(c.steps, r.steps);
}

/// Allocate, then fill in place: `new_uninit` and `assume_init`.
fn uninit_run_filled_in_place() {
    let mut b = backend();
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let n = 5;

    let p = c.step(|| c_alloc(&mut b, n * size_of::<u32>()).cast::<u32>());
    let mut v = r.step(|| OdbDataArrayFree::new_uninit::<u32>(n).unwrap());
    assert_eq!(c.steps, r.steps);

    for i in 0..n {
        // SAFETY: `p` holds `n` writable, aligned `u32`s.
        unsafe { p.add(i).write(i as u32 * 3) };
        v.as_mut_slice()[i].write(i as u32 * 3);
    }
    // SAFETY: every element was written above.
    let v = unsafe { v.assume_init() };
    // SAFETY: `p` holds `n` initialised `u32`s.
    assert_eq!(unsafe { std::slice::from_raw_parts(p, n) }, v.as_slice());

    c.step(|| c_free(&mut b, p.cast()));
    r.step(|| drop(v));
    assert_eq!(c.steps, r.steps);
}

/// Allocate and clear, as `git__calloc`: `new_zeroed`.
fn zeroed_run() {
    let mut b = backend();
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let n = 3;

    let p = c.step(|| {
        let p = c_alloc(&mut b, n * size_of::<u64>());
        // SAFETY: `p` holds `n * 8` writable bytes.
        unsafe { p.write_bytes(0, n * size_of::<u64>()) };
        p
    });
    let v = r.step(|| OdbDataArrayFree::new_zeroed::<u64>(n).unwrap());
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_read(p, n * 8), vec![0; n * 8]);
    assert_eq!(v.as_slice(), &[0; 3]);

    c.step(|| c_free(&mut b, p));
    r.step(|| drop(v));
    assert_eq!(c.steps, r.steps);
}

/// One cleared object: `OdbDataBox::new_zeroed`.
fn zeroed_object() {
    let mut b = backend();
    let (mut c, mut r) = (Recorder::default(), Recorder::default());

    let p = c.step(|| {
        let p = c_alloc(&mut b, size_of::<pair_st>());
        // SAFETY: `p` holds one `pair_st`'s bytes.
        unsafe { p.write_bytes(0, size_of::<pair_st>()) };
        p
    });
    let pair = r.step(|| OdbDataBox::<Pair>::new_zeroed().unwrap());
    assert_eq!(c.steps, r.steps);
    // SAFETY: `pair` holds an initialised `pair_st`, read without forming a reference.
    let bytes = unsafe {
        std::slice::from_raw_parts(pair.as_c_ptr().cast::<u8>(), size_of::<pair_st>())
    };
    assert_eq!(c_read(p, size_of::<pair_st>()), bytes);

    c.step(|| c_free(&mut b, p));
    r.step(|| drop(pair));
    assert_eq!(c.steps, r.steps);
}

/// Storage released before anything is written: `OdbDataBox::new_uninit`.
fn uninit_object_released_unfilled() {
    let mut b = backend();
    let (mut c, mut r) = (Recorder::default(), Recorder::default());

    let p = c.step(|| c_alloc(&mut b, size_of::<pair_st>()));
    let slot = r.step(|| OdbDataBox::<Pair>::new_uninit().unwrap());
    assert_eq!(c.steps, r.steps);

    c.step(|| c_free(&mut b, p));
    r.step(|| drop(slot));
    assert_eq!(c.steps, r.steps);
}

/// The allocator refuses: NULL and an OOM error in C, `None` and the same error here.
fn allocation_failures() {
    let mut b = backend();
    let src = pattern(32);

    let fail = |c_step: &mut dyn FnMut() -> bool, rust_step: &mut dyn FnMut() -> bool| {
        let (mut c, mut r) = (Recorder::default(), Recorder::default());
        clear_error();
        FAIL_NEXT.store(true, Ordering::SeqCst);
        assert!(c.step(&mut *c_step), "C reports failure");
        let c_error = last_error();
        clear_error();
        FAIL_NEXT.store(true, Ordering::SeqCst);
        assert!(r.step(&mut *rust_step), "Rust reports failure");
        assert_eq!(c.steps, r.steps);
        assert!(matches!(r.steps[0][..], [Event::AllocFailed { .. }]));
        assert_eq!(c_error, last_error());
        assert_eq!(c_error.0, ffi::GIT_ERROR_NOMEMORY as c_int);
    };

    fail(&mut || c_alloc(&mut backend(), 32).is_null(), &mut || {
        OdbDataVec::<u8>::from_slice(&src).is_none()
    });
    fail(&mut || c_alloc(&mut backend(), 7 * 4).is_null(), &mut || {
        OdbDataArrayFree::new_uninit::<u32>(7).is_none()
    });
    fail(&mut || c_alloc(&mut backend(), 2 * 8).is_null(), &mut || {
        OdbDataArrayFree::new_zeroed::<u64>(2).is_none()
    });
    fail(&mut || c_alloc(&mut backend(), size_of::<pair_st>()).is_null(), &mut || {
        OdbDataBox::<Pair>::new_zeroed().is_none()
    });
    fail(&mut || c_alloc(&mut backend(), size_of::<pair_st>()).is_null(), &mut || {
        OdbDataBox::<Pair>::new_uninit().is_none()
    });

    // A failed copy leaves the source intact.
    let p = c_alloc(&mut b, src.len());
    // SAFETY: `p` holds `src.len()` writable bytes.
    unsafe { p.copy_from_nonoverlapping(src.as_ptr(), src.len()) };
    let v = OdbDataVec::<u8>::from_slice(&src).unwrap();
    fail(&mut || c_alloc(&mut backend(), src.len()).is_null(), &mut || {
        v.try_clone().is_none()
    });
    assert_eq!(c_read(p, src.len()), v.as_slice());
    c_free(&mut b, p);
    drop(v);
}

/// The documented divergence: empty Rust buffers never reach the allocator.
fn empty_runs() {
    let mut r = Recorder::default();
    r.step(|| {
        let a = OdbDataVec::<u8>::from_slice(&[]).unwrap();
        let b = OdbDataArrayFree::new_uninit::<u32>(0).unwrap();
        let c = OdbDataArrayFree::new_zeroed::<u64>(0).unwrap();
        drop((a.clone(), a, b, c));
    });
    assert_eq!(r.steps, [vec![]]);
}
