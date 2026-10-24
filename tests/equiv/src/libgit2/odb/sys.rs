//! Equivalence of `git2/sys/odb_backend.h`'s raw-lifetime strategies with the C
//! routines they call, `git_odb_backend_data_alloc` / `git_odb_backend_data_free`.
//!
//! libgit2's allocator and initialisation are process-global, so each test reruns
//! itself alone in a child process, which installs a tracing `git_allocator` before
//! `git_libgit2_init`. The C side passes a real backend; the Rust strategies pass
//! NULL, which both routines ignore.
//!
//! The string strategies are compared with `git__strdup` and `git_strarray_copy`
//! rebuilt over the exported routines (neither is exported: `git_strarray_copy` is
//! deprecated and hidden under `GIT_DEPRECATE_HARD`), and with the C disposers that
//! release libgit2's strings, `git_strarray_dispose` and `git_buf_dispose`, on
//! strings either side made or libgit2 handed out. Strings taken out of a `git_buf`
//! libgit2 filled are compared in `tests/equiv/buffer.rs`, with `core::buffer`.
//!
//! Documented divergences, outside these comparisons:
//!
//! - An empty Rust buffer is NULL and never reaches the allocator, where C would call
//!   `git_odb_backend_data_alloc(backend, 0)`.
//! - Before `git_libgit2_init`, C gets NULL after `git_error_set_oom` writes through a nonexistent
//!   TLS key; the strategies pin the runtime first and succeed. The corrected behaviour is
//!   regression-tested in the module's `unit_tests` and in `ub_safe`.
//! - `git_strarray_copy` silently skips NULL elements of its source (`strarray.c`);
//!   `OdbDataStrVec::from_raw_parts` refuses to adopt an array holding one, which `ub_unsafe`
//!   covers.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::mem::size_of;
use std::process::Command;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::odb::{
    OdbDataArrayFree, OdbDataBox, OdbDataStrFree, OdbDataStrVec, OdbDataVec,
};

#[repr(C)]
#[expect(
    unreachable_pub,
    reason = "`define_ctype!` makes its newtype `pub`, which names this layout"
)]
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
    Alloc {
        ptr: usize,
        len: usize,
        file: String,
        line: c_int,
    },
    Free {
        ptr: usize,
    },
}

static TRACING: AtomicBool = AtomicBool::new(false);
/// Fails the `n`th allocation from now when `n > 0`.
static FAIL_IN: AtomicUsize = AtomicUsize::new(0);
static TRACE: Mutex<Vec<Raw>> = Mutex::new(Vec::new());
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// Counts `FAIL_IN` down; `true` for the allocation it names.
fn refuse_now() -> bool {
    let mut n = FAIL_IN.load(Ordering::SeqCst);
    while n > 0 {
        match FAIL_IN.compare_exchange(n, n - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return n == 1,
            Err(now) => n = now,
        }
    }
    false
}

unsafe extern "C" fn traced_malloc(len: usize, file: *const c_char, line: c_int) -> *mut c_void {
    let ptr = if refuse_now() {
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
        TRACE.lock().unwrap().push(Raw::Alloc {
            ptr: ptr as usize,
            len,
            file,
            line,
        });
    }
    ptr
}

unsafe extern "C" fn traced_realloc(
    ptr: *mut c_void,
    len: usize,
    _file: *const c_char,
    _line: c_int,
) -> *mut c_void {
    assert!(
        !TRACING.load(Ordering::SeqCst),
        "no traced step reallocates"
    );
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
        ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(), &raw mut allocator)
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
    Alloc {
        len: usize,
        file: String,
        line: c_int,
    },
    AllocFailed {
        len: usize,
        file: String,
        line: c_int,
    },
    Free {
        alloc: usize,
    },
}

/// Records one sequence of steps, each step's events normalised separately.
#[derive(Default)]
struct Recorder {
    steps: Vec<Vec<Event>>,
    allocs: HashMap<usize, usize>,
    count: usize,
}

impl Recorder {
    /// Numbers an allocation made before the sequence, as the next one, and counts it
    /// live: the sequence releases it.
    fn adopt(&mut self, ptr: *const c_void) {
        self.allocs.insert(ptr as usize, self.count);
        self.count += 1;
        LIVE.fetch_add(1, Ordering::SeqCst);
    }

    fn step<R>(&mut self, f: impl FnOnce() -> R) -> R {
        TRACE.lock().unwrap().clear();
        TRACING.store(true, Ordering::SeqCst);
        let r = f();
        TRACING.store(false, Ordering::SeqCst);
        let raw = std::mem::take(&mut *TRACE.lock().unwrap());
        let events = raw
            .into_iter()
            .map(|e| match e {
                Raw::Alloc {
                    ptr: 0,
                    len,
                    file,
                    line,
                } => Event::AllocFailed { len, file, line },
                Raw::Alloc {
                    ptr,
                    len,
                    file,
                    line,
                } => {
                    self.allocs.insert(ptr, self.count);
                    self.count += 1;
                    Event::Alloc { len, file, line }
                }
                Raw::Free { ptr } => Event::Free {
                    alloc: self
                        .allocs
                        .remove(&ptr)
                        .expect("freed what the sequence allocated"),
                },
            })
            .collect();
        self.steps.push(events);
        r
    }
}

/// `git_error_last()`'s class and message.
fn last_error() -> (c_int, String) {
    {
        // SAFETY: `git_error_last` never returns NULL; the error lives until the next
        // libgit2 call on this thread, and is copied out before one.
        let e = unsafe { ffi::git_error_last() };
        // SAFETY: as above.
        let klass = unsafe { (*e).klass };
        // SAFETY: as above.
        let message2 = unsafe { (*e).message };
        // SAFETY: as above.
        unsafe {
            (
                klass,
                CStr::from_ptr(message2).to_string_lossy().into_owned(),
            )
        }
    }
}

fn clear_error() {
    // SAFETY: no arguments; resets this thread's error state.
    unsafe { ffi::git_error_clear() }
}

/// A backend the C side passes; both routines ignore it.
fn backend() -> ffi::git_odb_backend {
    ffi::git_odb_backend {
        version: 1,
        ..Default::default()
    }
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
    (0..len).map(|i| (i * 13 + 5).to_le_bytes()[0]).collect()
}

// --- the comparisons ------------------------------------------------------------

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn traced_allocator_sequences() {
    isolated(
        concat!(module_path!(), "::traced_allocator_sequences"),
        || {
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
            assert_eq!(
                LIVE.load(Ordering::SeqCst),
                0,
                "every traced allocation was released"
            );
        },
    );
}

/// `memdup` then free: `from_slice` and drop.
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the sides and values the steps compare"
)]
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
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the sides and values the steps compare"
)]
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
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the sides and values the steps compare"
)]
#[expect(
    clippy::cast_ptr_alignment,
    reason = "libgit2's allocator returns memory aligned for any type, as malloc does"
)]
fn uninit_run_filled_in_place() {
    let mut b = backend();
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let n = 5;

    let p = c.step(|| c_alloc(&mut b, n * size_of::<u32>()).cast::<u32>());
    let mut v = r.step(|| OdbDataArrayFree::new_uninit::<u32>(n).unwrap());
    assert_eq!(c.steps, r.steps);

    for i in 0..n {
        // SAFETY: `p` holds `n` writable, aligned `u32`s.
        let add = unsafe { p.add(i) };
        // SAFETY: as above.
        unsafe { add.write(u32::try_from(i).unwrap() * 3) };
        v.as_mut_slice()[i].write(u32::try_from(i).unwrap() * 3);
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
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the sides and values the steps compare"
)]
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
    let bytes =
        unsafe { std::slice::from_raw_parts(pair.as_c_ptr().cast::<u8>(), size_of::<pair_st>()) };
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
        FAIL_IN.store(1, Ordering::SeqCst);
        assert!(c.step(&mut *c_step), "C reports failure");
        let c_error = last_error();
        clear_error();
        FAIL_IN.store(1, Ordering::SeqCst);
        assert!(r.step(&mut *rust_step), "Rust reports failure");
        assert_eq!(c.steps, r.steps);
        assert!(matches!(r.steps[0][..], [Event::AllocFailed { .. }]));
        assert_eq!(c_error, last_error());
        assert_eq!(c_error.0, ffi::GIT_ERROR_NOMEMORY.cast_signed());
    };

    fail(&mut || c_alloc(&mut backend(), 32).is_null(), &mut || {
        OdbDataVec::<u8>::from_slice(&src).is_none()
    });
    fail(
        &mut || c_alloc(&mut backend(), 7 * 4).is_null(),
        &mut || OdbDataArrayFree::new_uninit::<u32>(7).is_none(),
    );
    fail(
        &mut || c_alloc(&mut backend(), 2 * 8).is_null(),
        &mut || OdbDataArrayFree::new_zeroed::<u64>(2).is_none(),
    );
    fail(
        &mut || c_alloc(&mut backend(), size_of::<pair_st>()).is_null(),
        &mut || OdbDataBox::<Pair>::new_zeroed().is_none(),
    );
    fail(
        &mut || c_alloc(&mut backend(), size_of::<pair_st>()).is_null(),
        &mut || OdbDataBox::<Pair>::new_uninit().is_none(),
    );

    // A failed copy leaves the source intact.
    let p = c_alloc(&mut b, src.len());
    // SAFETY: `p` holds `src.len()` writable bytes.
    unsafe { p.copy_from_nonoverlapping(src.as_ptr(), src.len()) };
    let v = OdbDataVec::<u8>::from_slice(&src).unwrap();
    fail(
        &mut || c_alloc(&mut backend(), src.len()).is_null(),
        &mut || v.try_clone().is_none(),
    );
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

// --- strings --------------------------------------------------------------------

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn traced_string_sequences() {
    isolated(concat!(module_path!(), "::traced_string_sequences"), || {
        install_tracing_allocator();
        let long = CString::new(pattern_text(4096)).unwrap();
        for s in [c"", c"a", c"refs/heads/main", &long] {
            str_copy_then_release(s);
            str_clone_then_release_source_first(s);
            str_handed_to_git_buf_dispose(s);
        }
        let runs: [&[&CStr]; 4] = [
            &[],
            &[c"one"],
            &[c"", c"origin", c"upstream"],
            &[&long, c"x"],
        ];
        for strs in runs {
            run_copy_then_dispose(strs);
            run_handed_to_git_strarray_dispose(strs);
            run_clone_then_release_source_first(strs);
            run_moved_through_a_vec(strs);
        }
        string_allocation_failures();
        run_adopted_from_libgit2();
        assert_eq!(
            LIVE.load(Ordering::SeqCst),
            0,
            "every traced allocation was released"
        );
    });
}

fn pattern_text(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| b'a' + u8::try_from(i % 26).unwrap())
        .collect()
}

/// `git__strdup` over the exported allocator: NULL on failure.
fn c_strdup(s: &CStr) -> *mut c_char {
    let bytes = s.to_bytes_with_nul();
    let p = c_alloc(&mut backend(), bytes.len());
    if !p.is_null() {
        // SAFETY: `p` holds `bytes.len()` writable bytes.
        unsafe { p.copy_from_nonoverlapping(bytes.as_ptr(), bytes.len()) };
    }
    p.cast()
}

fn c_str(s: *const c_char) -> Vec<u8> {
    // SAFETY: callers pass a live NUL-terminated string.
    unsafe { CStr::from_ptr(s) }.to_bytes().to_vec()
}

/// `git_strarray_copy` over the exported allocator: the array (`git__calloc`), then
/// each string; a failure disposes of what was made and leaves a zeroed array.
fn c_strarray_copy(strs: &[&CStr]) -> Result<ffi::git_strarray, ffi::git_strarray> {
    let mut tgt = ffi::git_strarray {
        strings: ptr::null_mut(),
        count: 0,
    };
    if strs.is_empty() {
        return Ok(tgt);
    }
    let size = strs.len() * size_of::<*mut c_char>();
    let arr = c_alloc(&mut backend(), size);
    if arr.is_null() {
        return Err(tgt);
    }
    // SAFETY: `arr` holds `size` writable bytes.
    unsafe { arr.write_bytes(0, size) };
    tgt.strings = arr.cast();
    for s in strs {
        let copy = c_strdup(s);
        if copy.is_null() {
            c_strarray_dispose(&mut tgt);
            return Err(tgt);
        }
        // SAFETY: `tgt.count < strs.len()` slots of the array are in use.
        let add = unsafe { tgt.strings.add(tgt.count) };
        // SAFETY: as above.
        unsafe { add.write(copy) };
        tgt.count += 1;
    }
    Ok(tgt)
}

fn c_strarray_dispose(array: &mut ffi::git_strarray) {
    // SAFETY: `array` holds `count` strings and the array from libgit2's allocator,
    // or is zeroed.
    unsafe { ffi::git_strarray_dispose(array) }
}

fn c_strings(array: &ffi::git_strarray) -> Vec<Vec<u8>> {
    (0..array.count)
        .map(|i| {
            // SAFETY: `array` holds `count` live strings, and `i < count`.
            let slot = unsafe { array.strings.add(i) };
            // SAFETY: as above.
            c_str(unsafe { slot.read() })
        })
        .collect()
}

fn rust_strings(v: &OdbDataStrVec) -> Vec<Vec<u8>> {
    v.as_slice().iter().map(|s| s.as_bytes().to_vec()).collect()
}

fn is_zeroed(array: &ffi::git_strarray) -> bool {
    array.strings.is_null() && array.count == 0
}

/// `git__strdup` then free: `OdbDataStrFree::dup` and drop.
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the sides and values the steps compare"
)]
fn str_copy_then_release(s: &CStr) {
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let p = c.step(|| c_strdup(s));
    let o = r.step(|| OdbDataStrFree::dup(s).unwrap());
    assert_eq!(c.steps, r.steps);
    assert!(matches!(&r.steps[0][..], [Event::Alloc { len, .. }] if *len == s.count_bytes() + 1));
    assert_eq!(c_str(p), o.as_bytes());
    assert_eq!(s.is_empty(), o.is_empty());

    c.step(|| c_free(&mut backend(), p.cast()));
    r.step(|| drop(o));
    assert_eq!(c.steps, r.steps);
}

/// A copy of a copy, the source released first: `clone`.
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the sides and values the steps compare"
)]
fn str_clone_then_release_source_first(s: &CStr) {
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let p = c.step(|| c_strdup(s));
    let o = r.step(|| OdbDataStrFree::dup(s).unwrap());
    // SAFETY: `p` is a live string.
    let q = c.step(|| c_strdup(unsafe { CStr::from_ptr(p) }));
    let w = r.step(|| o.clone());
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_str(q), w.as_bytes());

    c.step(|| c_free(&mut backend(), p.cast()));
    r.step(|| drop(o));
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_str(q), w.as_bytes());

    c.step(|| c_free(&mut backend(), q.cast()));
    r.step(|| drop(w));
    assert_eq!(c.steps, r.steps);
}

/// A string handed to a `git_buf` that `git_buf_dispose` releases, as libgit2's
/// output buffers are.
fn str_handed_to_git_buf_dispose(s: &CStr) {
    let into_buf = |p: *mut c_char| ffi::git_buf {
        ptr: p,
        reserved: s.count_bytes() + 1,
        size: s.count_bytes(),
    };
    let dispose = |buf: &mut ffi::git_buf| {
        // SAFETY: `buf` owns a string from libgit2's allocator.
        unsafe { ffi::git_buf_dispose(buf) }
    };
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let mut cbuf = c.step(|| into_buf(c_strdup(s)));
    let mut rbuf = r.step(|| into_buf(OdbDataStrFree::dup(s).unwrap().into_raw()));
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_str(cbuf.ptr), c_str(rbuf.ptr));

    c.step(|| dispose(&mut cbuf));
    r.step(|| dispose(&mut rbuf));
    assert_eq!(c.steps, r.steps);
    assert!(matches!(c.steps[1][..], [Event::Free { alloc: 0 }]));
    // Both are reset to libgit2's static empty string.
    assert_eq!((cbuf.ptr, cbuf.reserved, cbuf.size), (rbuf.ptr, 0, 0));
    assert_eq!((rbuf.reserved, rbuf.size), (0, 0));
}

/// `git_strarray_copy` then `git_strarray_dispose`: `dup_all` and drop.
fn run_copy_then_dispose(strs: &[&CStr]) {
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let mut a = c.step(|| c_strarray_copy(strs).unwrap());
    let v = r.step(|| OdbDataStrVec::dup_all(strs).unwrap());
    assert_eq!(c.steps, r.steps);
    assert_eq!(
        (a.count, a.strings.is_null()),
        (v.len(), v.as_ptr().is_null())
    );
    assert_eq!(c_strings(&a), rust_strings(&v));

    c.step(|| c_strarray_dispose(&mut a));
    r.step(|| drop(v));
    assert_eq!(c.steps, r.steps);
    assert!(is_zeroed(&a));
}

/// Rust's run released by C: `into_raw_parts` into a `git_strarray`, then
/// `git_strarray_dispose`, as for a C-made copy.
fn run_handed_to_git_strarray_dispose(strs: &[&CStr]) {
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let mut a = c.step(|| c_strarray_copy(strs).unwrap());
    let mut b = r.step(|| {
        let (strings, count) = OdbDataStrVec::dup_all(strs).unwrap().into_raw_parts();
        ffi::git_strarray { strings, count }
    });
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_strings(&a), c_strings(&b));

    c.step(|| c_strarray_dispose(&mut a));
    r.step(|| c_strarray_dispose(&mut b));
    assert_eq!(c.steps, r.steps);
    assert!(is_zeroed(&a) && is_zeroed(&b));
}

/// A copy of a copy, the source released first: `OdbDataStrVec::clone`.
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the sides and values the steps compare"
)]
fn run_clone_then_release_source_first(strs: &[&CStr]) {
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let mut a = c.step(|| c_strarray_copy(strs).unwrap());
    let v = r.step(|| OdbDataStrVec::dup_all(strs).unwrap());
    let mut b = c.step(|| {
        let src: Vec<&CStr> = (0..a.count)
            .map(|i| {
                // SAFETY: `a` holds `count` live strings, and `i < count`.
                let slot = unsafe { a.strings.add(i) };
                // SAFETY: as above.
                let s = unsafe { slot.read() };
                // SAFETY: as above, a live NUL-terminated string.
                unsafe { CStr::from_ptr(s) }
            })
            .collect();
        c_strarray_copy(&src).unwrap()
    });
    let w = r.step(|| v.clone());
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_strings(&b), rust_strings(&w));

    c.step(|| c_strarray_dispose(&mut a));
    r.step(|| drop(v));
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_strings(&b), rust_strings(&w));

    c.step(|| c_strarray_dispose(&mut b));
    r.step(|| drop(w));
    assert_eq!(c.steps, r.steps);
}

/// Strings moved out of a run and back into a fresh one keep their storage; only the
/// array is replaced, as C moving the pointers into a new array would.
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the sides and values the steps compare"
)]
#[expect(
    clippy::cast_ptr_alignment,
    reason = "libgit2's allocator returns memory aligned for any type, as malloc does"
)]
fn run_moved_through_a_vec(strs: &[&CStr]) {
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let mut a = c.step(|| c_strarray_copy(strs).unwrap());
    let v = r.step(|| OdbDataStrVec::dup_all(strs).unwrap());

    let mut b = c.step(|| {
        let mut b = ffi::git_strarray {
            strings: ptr::null_mut(),
            count: 0,
        };
        if a.count > 0 {
            // SAFETY: `a` holds `count` strings, moved out before its array is freed.
            let moved = unsafe { std::slice::from_raw_parts(a.strings, a.count) }.to_vec();
            c_free(&mut backend(), a.strings.cast());
            let arr = c_alloc(&mut backend(), moved.len() * size_of::<*mut c_char>());
            let arr = arr.cast::<*mut c_char>();
            // SAFETY: `arr` holds `moved.len()` writable slots.
            unsafe { arr.copy_from_nonoverlapping(moved.as_ptr(), moved.len()) };
            b = ffi::git_strarray {
                strings: arr,
                count: moved.len(),
            };
        }
        a = ffi::git_strarray {
            strings: ptr::null_mut(),
            count: 0,
        };
        b
    });
    let w = r.step(|| OdbDataStrVec::from_vec(v.into_vec()).unwrap());
    assert_eq!(c.steps, r.steps);
    assert_eq!(c_strings(&b), rust_strings(&w));
    assert!(is_zeroed(&a));

    c.step(|| c_strarray_dispose(&mut b));
    r.step(|| drop(w));
    assert_eq!(c.steps, r.steps);
}

/// Every allocation of a copy refused in turn: C disposes of the partial copy and
/// reports OOM; Rust releases the same storage in the same order, returns `None` and
/// reports the same error.
fn string_allocation_failures() {
    let strs = [c"alpha", c"beta", c"gamma"];
    for at in 1..=strs.len() + 1 {
        let (mut c, mut r) = (Recorder::default(), Recorder::default());
        clear_error();
        FAIL_IN.store(at, Ordering::SeqCst);
        let a = c.step(|| c_strarray_copy(&strs)).unwrap_err();
        let c_error = last_error();
        clear_error();
        FAIL_IN.store(at, Ordering::SeqCst);
        assert!(r.step(|| OdbDataStrVec::dup_all(&strs)).is_none());
        assert_eq!(c.steps, r.steps, "allocation {at} refused");
        assert!(is_zeroed(&a));
        assert_eq!(c_error, last_error());
        assert_eq!(c_error.0, ffi::GIT_ERROR_NOMEMORY.cast_signed());
    }

    // A refused `git__strdup`, and a refused clone, leave their sources intact.
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    let v = OdbDataStrVec::dup_all(&strs).unwrap();
    let s = OdbDataStrFree::dup(c"delta").unwrap();
    clear_error();
    FAIL_IN.store(1, Ordering::SeqCst);
    assert!(c.step(|| c_strdup(c"delta")).is_null());
    let c_error = last_error();
    clear_error();
    FAIL_IN.store(1, Ordering::SeqCst);
    assert!(r.step(|| s.try_clone()).is_none());
    assert_eq!(
        (c.steps.clone(), c_error.clone()),
        (r.steps.clone(), last_error())
    );
    FAIL_IN.store(2, Ordering::SeqCst);
    assert!(v.try_clone().is_none());
    assert_eq!(FAIL_IN.load(Ordering::SeqCst), 0);
    assert_eq!(rust_strings(&v), [&b"alpha"[..], b"beta", b"gamma"]);
    assert_eq!(s.as_bytes(), b"delta");
}

/// `GIT_OPT_GET_EXTENSIONS` hands out a `git_strarray` from a `git_vector`, its array
/// with spare capacity: adopted, it reads and releases as `git_strarray_dispose`
/// would.
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the sides and values the steps compare"
)]
fn run_adopted_from_libgit2() {
    let get = || {
        let mut out = ffi::git_strarray {
            strings: ptr::null_mut(),
            count: 0,
        };
        // SAFETY: `GIT_OPT_GET_EXTENSIONS` takes a `git_strarray *`, which it fills.
        let rc = unsafe {
            ffi::git_libgit2_opts(ffi::GIT_OPT_GET_EXTENSIONS.cast_signed(), &raw mut out)
        };
        assert_eq!(rc, 0);
        out
    };
    let exts = [c"!noop", c"crustify"];
    let ptrs: Vec<*const c_char> = exts.iter().map(|s| s.as_ptr()).collect();
    // SAFETY: `GIT_OPT_SET_EXTENSIONS` takes an array of `len` strings, which it copies.
    let rc = unsafe {
        ffi::git_libgit2_opts(
            ffi::GIT_OPT_SET_EXTENSIONS.cast_signed(),
            ptrs.as_ptr(),
            ptrs.len(),
        )
    };
    assert_eq!(rc, 0);

    let mut a = get();
    let b = get();
    // SAFETY: `b` is libgit2's output, handed over whole and not used again.
    let v = unsafe { OdbDataStrVec::from_raw_parts(b.strings, b.count) }.unwrap();
    assert_eq!(c_strings(&a), rust_strings(&v));
    assert!(rust_strings(&v).contains(&b"crustify".to_vec()));
    assert!(!rust_strings(&v).contains(&b"noop".to_vec()));

    let n = a.count;
    let (mut c, mut r) = (Recorder::default(), Recorder::default());
    for (rec, array) in [(&mut c, &a), (&mut r, &b)] {
        for i in 0..array.count {
            // SAFETY: `array` holds `count` live strings, and `i < count`.
            let slot = unsafe { array.strings.add(i) };
            // SAFETY: as above.
            rec.adopt(unsafe { slot.read() }.cast());
        }
        rec.adopt(array.strings.cast());
    }
    c.step(|| c_strarray_dispose(&mut a));
    r.step(|| drop(v));
    assert_eq!(c.steps, r.steps);
    // Every string in order, then the array.
    assert_eq!(
        c.steps[0],
        (0..=n)
            .map(|alloc| Event::Free { alloc })
            .collect::<Vec<_>>()
    );

    // SAFETY: an empty list of user extensions.
    let rc = unsafe {
        ffi::git_libgit2_opts(
            ffi::GIT_OPT_SET_EXTENSIONS.cast_signed(),
            ptr::null::<*const c_char>(),
            0usize,
        )
    };
    assert_eq!(rc, 0);
}
