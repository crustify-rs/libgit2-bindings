//! Equivalence of `git2/sys/alloc.h`'s `Allocator` with a raw `git_allocator`.
//!
//! - Values: the same member writes on an `Allocator` and on a raw `git_allocator`
//!   leave the same members, step by step.
//! - Behaviour: libgit2 runs one workload on a raw allocator of C-shaped functions and
//!   on `Allocator::new` over an `Alloc` implementation that does the same; every call
//!   either receives is traced, with pointers replaced by allocation numbers, and the
//!   traces, return values and error states are compared after each step.
//!
//! libgit2's allocator is process-global, so the behavioural test reruns alone in a
//! child process. Both allocators forward to libc, so switching between them while
//! blocks are live is sound; each side runs on a fresh thread, so each starts without
//! thread-local error state and releases it on exit, through the allocator under test.
//!
//! Documented divergence, outside the comparison: `free(NULL)` stops at the trampoline
//! and never reaches `Alloc::free`, so the raw side does not trace it either.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::process::Command;
use std::ptr::{self, NonNull};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

use libgit2::ffi;
use libgit2::libgit2::raw::OdbDataStrFree;
use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator, FreeFn, MallocFn, ReallocFn};

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

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

// --- values -------------------------------------------------------------------------

unsafe extern "C" fn malloc_a(_: usize, _: *const c_char, _: c_int) -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "C" fn malloc_b(_: usize, _: *const c_char, _: c_int) -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "C" fn realloc_a(
    _: *mut c_void,
    _: usize,
    _: *const c_char,
    _: c_int,
) -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "C" fn realloc_b(
    _: *mut c_void,
    _: usize,
    _: *const c_char,
    _: c_int,
) -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "C" fn free_a(_: *mut c_void) {}
unsafe extern "C" fn free_b(_: *mut c_void) {}

fn same<F: Copy>(a: Option<F>, b: Option<F>, addr: fn(F) -> usize) -> bool {
    a.map(addr) == b.map(addr)
}

fn assert_same_members(rust: &Allocator, c: &ffi::git_allocator) {
    let r = rust.as_ref();
    assert!(same(r.malloc(), c.gmalloc, |f: MallocFn| f as usize));
    assert!(same(r.realloc(), c.grealloc, |f: ReallocFn| f as usize));
    assert!(same(r.free(), c.gfree, |f: FreeFn| f as usize));
}

#[test]
fn member_writes_match_a_raw_allocator() {
    // One stored pointer value per function: each coercion may yield a new address.
    let (ma, mb): (MallocFn, MallocFn) = (malloc_a, malloc_b);
    let (ra, rb): (ReallocFn, ReallocFn) = (realloc_a, realloc_b);
    let (fa, fb): (FreeFn, FreeFn) = (free_a, free_b);

    let mut rust = Allocator::from_fns(ma, ra, fa);
    let mut c = ffi::git_allocator {
        gmalloc: Some(ma),
        grealloc: Some(ra),
        gfree: Some(fa),
    };
    assert_same_members(&rust, &c);

    rust.as_mut().set_malloc(mb);
    c.gmalloc = Some(mb);
    assert_same_members(&rust, &c);

    rust.as_mut().set_free(fb);
    c.gfree = Some(fb);
    assert_same_members(&rust, &c);

    rust.as_mut().set_realloc(rb);
    c.grealloc = Some(rb);
    assert_same_members(&rust, &c);

    rust.as_mut().set_malloc(ma);
    c.gmalloc = Some(ma);
    assert_same_members(&rust, &c);
}

// --- behaviour ------------------------------------------------------------------------

/// The one request size both allocators refuse.
const REFUSED: usize = 1 << 40;

/// One call an allocator received, pointers as they were.
#[derive(Debug)]
enum Raw {
    Malloc {
        len: usize,
        file: Option<CString>,
        line: c_int,
        got: usize,
    },
    Realloc {
        from: usize,
        len: usize,
        file: Option<CString>,
        line: c_int,
        got: usize,
    },
    Free {
        ptr: usize,
    },
}

static TRACING: AtomicBool = AtomicBool::new(false);
static TRACE: Mutex<Vec<Raw>> = Mutex::new(Vec::new());

fn trace(call: Raw) {
    if TRACING.load(SeqCst) {
        TRACE.lock().unwrap().push(call);
    }
}

fn libc_malloc(len: usize) -> *mut c_void {
    if len == REFUSED {
        return ptr::null_mut();
    }
    // SAFETY: plain libc allocation.
    unsafe { malloc(len) }
}

/// # Safety
///
/// `p` is NULL or a live libc block the caller gives up on success.
unsafe fn libc_realloc(p: *mut c_void, len: usize) -> *mut c_void {
    if len == REFUSED {
        return ptr::null_mut();
    }
    // SAFETY: per this function's contract.
    unsafe { realloc(p, len) }
}

/// # Safety
///
/// `file` is NULL or a live C string.
unsafe fn file_of(file: *const c_char) -> Option<CString> {
    // SAFETY: per this function's contract.
    (!file.is_null()).then(|| unsafe { CStr::from_ptr(file) }.to_owned())
}

// The C side: functions in `git_allocator`'s own shape.

unsafe extern "C" fn c_traced_malloc(len: usize, file: *const c_char, line: c_int) -> *mut c_void {
    let got = libc_malloc(len);
    // SAFETY: libgit2 passes `__FILE__`.
    let file = unsafe { file_of(file) };
    trace(Raw::Malloc {
        len,
        file,
        line,
        got: got as usize,
    });
    got
}

unsafe extern "C" fn c_traced_realloc(
    p: *mut c_void,
    len: usize,
    file: *const c_char,
    line: c_int,
) -> *mut c_void {
    // SAFETY: `p` is NULL or a live block of this libc-backed family.
    let got = unsafe { libc_realloc(p, len) };
    // SAFETY: libgit2 passes `__FILE__`.
    let file = unsafe { file_of(file) };
    trace(Raw::Realloc {
        from: p as usize,
        len,
        file,
        line,
        got: got as usize,
    });
    got
}

unsafe extern "C" fn c_traced_free(p: *mut c_void) {
    if !p.is_null() {
        trace(Raw::Free { ptr: p as usize });
    }
    // SAFETY: `p` is NULL or a live libc block, given up.
    unsafe { free(p) }
}

// The Rust side: the same logic behind `Alloc`.

struct Traced;

// SAFETY: forwards to libc's `malloc` family (refusing one size), which meets `Alloc`'s
// contract; the trace sits behind a lock.
unsafe impl Alloc for Traced {
    fn malloc(len: usize, file: Option<&CStr>, line: i32) -> Option<NonNull<u8>> {
        let got = libc_malloc(len);
        trace(Raw::Malloc {
            len,
            file: file.map(CStr::to_owned),
            line,
            got: got as usize,
        });
        NonNull::new(got.cast())
    }

    unsafe fn realloc(
        p: Option<NonNull<u8>>,
        len: usize,
        file: Option<&CStr>,
        line: i32,
    ) -> Option<NonNull<u8>> {
        let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live block of this libc-backed family.
        let got = unsafe { libc_realloc(p, len) };
        trace(Raw::Realloc {
            from: p as usize,
            len,
            file: file.map(CStr::to_owned),
            line,
            got: got as usize,
        });
        NonNull::new(got.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        trace(Raw::Free {
            ptr: p.as_ptr() as usize,
        });
        // SAFETY: `p` is a live libc block, given up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

/// One call with its pointers numbered by the allocation that made them (`None` for
/// NULL, or for a block from before the side began).
#[derive(Debug, PartialEq, Eq)]
enum Event {
    Malloc {
        len: usize,
        file: Option<CString>,
        line: c_int,
        got: Option<usize>,
    },
    Realloc {
        from: Option<usize>,
        len: usize,
        file: Option<CString>,
        line: c_int,
        got: Option<usize>,
    },
    Free {
        block: Option<usize>,
    },
}

/// What one step left behind: its calls, its result and the thread's error.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    events: Vec<Event>,
    result: c_int,
    error: (c_int, CString),
}

#[derive(Default)]
struct Numbering {
    blocks: std::collections::HashMap<usize, usize>,
    next: usize,
}

impl Numbering {
    fn made(&mut self, p: usize) -> Option<usize> {
        (p != 0).then(|| {
            self.blocks.insert(p, self.next);
            self.next += 1;
            self.next - 1
        })
    }

    fn gone(&mut self, p: usize) -> Option<usize> {
        self.blocks.remove(&p)
    }

    fn normalise(&mut self, raw: Vec<Raw>) -> Vec<Event> {
        raw.into_iter()
            .map(|r| match r {
                Raw::Malloc {
                    len,
                    file,
                    line,
                    got,
                } => Event::Malloc {
                    len,
                    file,
                    line,
                    got: self.made(got),
                },
                Raw::Realloc {
                    from,
                    len,
                    file,
                    line,
                    got,
                } => {
                    // A failed resize leaves `from` live under its number.
                    let from_no = if got == 0 {
                        self.blocks.get(&from).copied()
                    } else {
                        self.gone(from)
                    };
                    Event::Realloc {
                        from: from_no,
                        len,
                        file,
                        line,
                        got: self.made(got),
                    }
                }
                Raw::Free { ptr } => Event::Free {
                    block: self.gone(ptr),
                },
            })
            .collect()
    }
}

fn last_error() -> (c_int, CString) {
    // SAFETY: `git_error_last` never returns NULL; the error is copied out before the
    // next libgit2 call on this thread.
    unsafe {
        let e = ffi::git_error_last();
        ((*e).klass, CStr::from_ptr((*e).message).to_owned())
    }
}

fn set_str(class: ffi::git_error_t, message: &CStr) -> c_int {
    // SAFETY: `message` is a live C string, which libgit2 copies.
    unsafe { ffi::git_error_set_str(class as c_int, message.as_ptr()) }
}

fn install(allocator: *const ffi::git_allocator) {
    // SAFETY: `allocator` is NULL (the default allocator) or addresses a
    // `git_allocator` with every member set, which libgit2 copies; every family
    // installed here is libc-backed, so live blocks stay valid across the switch.
    let rc = unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR as c_int, allocator) };
    assert_eq!(rc, 0);
}

/// Runs the workload on a fresh thread under the installed allocator; one `Step` per
/// stage, the last being the thread's exit, which releases its error state.
fn workload() -> Vec<Step> {
    let (mut steps, mut numbering) = std::thread::spawn(|| {
        let mut numbering = Numbering::default();
        let mut steps = Vec::new();
        let mut step = |f: &mut dyn FnMut() -> c_int| {
            TRACE.lock().unwrap().clear();
            TRACING.store(true, SeqCst);
            let result = f();
            TRACING.store(false, SeqCst);
            let events = numbering.normalise(std::mem::take(&mut *TRACE.lock().unwrap()));
            steps.push(Step {
                events,
                result,
                error: last_error(),
            });
        };
        let long = CString::new("libgit2 grows its message buffer ".repeat(64)).unwrap();
        // The thread's error state is allocated, then its message buffer grows.
        step(&mut || set_str(ffi::GIT_ERROR_CONFIG, c"short"));
        step(&mut || set_str(ffi::GIT_ERROR_ODB, &long));
        step(&mut || set_str(ffi::GIT_ERROR_INDEX, c"shrinks back"));
        // An allocation libgit2 makes and frees for the caller.
        step(&mut || OdbDataStrFree::dup(c"a string").map_or(-1, |s| s.len() as c_int));
        // A zero-byte request, which libgit2 makes and releases like any other.
        step(&mut || {
            // SAFETY: the routines ignore `backend`; a NULL result owns nothing, and a
            // block is released once.
            unsafe {
                let p = ffi::git_odb_backend_data_alloc(ptr::null_mut(), 0);
                let got = !p.is_null();
                ffi::git_odb_backend_data_free(ptr::null_mut(), p);
                got as c_int
            }
        });
        // A refused allocation, reported as out of memory.
        step(&mut || {
            // SAFETY: the routine ignores `backend`; a NULL result owns nothing.
            let p = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), REFUSED) };
            p.is_null() as c_int
        });
        step(&mut || {
            // SAFETY: no arguments.
            unsafe { ffi::git_error_clear() };
            0
        });
        // Trace the thread's exit, which frees its error state after this returns.
        TRACE.lock().unwrap().clear();
        TRACING.store(true, SeqCst);
        (steps, numbering)
    })
    .join()
    .unwrap();
    TRACING.store(false, SeqCst);
    let events = numbering.normalise(std::mem::take(&mut *TRACE.lock().unwrap()));
    steps.push(Step {
        events,
        result: 0,
        error: (0, CString::default()),
    });
    steps
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn libgit2_drives_both_allocators_alike() {
    isolated(
        concat!(module_path!(), "::libgit2_drives_both_allocators_alike"),
        || {
            let c_side = ffi::git_allocator {
                gmalloc: Some(c_traced_malloc),
                grealloc: Some(c_traced_realloc),
                gfree: Some(c_traced_free),
            };
            let rust_side = Allocator::new::<Traced>();
            install(&c_side);
            // SAFETY: no arguments; refcounted, never released in this process.
            assert!(unsafe { ffi::git_libgit2_init() } > 0);

            let run = |allocator: *const ffi::git_allocator| {
                install(allocator);
                workload()
            };
            let c_steps = run(&c_side);
            let rust_steps = run((&raw const rust_side).cast());
            // NULL restores the default allocator, libc's as well.
            install(ptr::null());

            assert_eq!(c_steps.len(), rust_steps.len());
            for (i, (c, rust)) in c_steps.iter().zip(&rust_steps).enumerate() {
                assert_eq!(c, rust, "step {i}");
            }
            // The comparison saw real traffic of every kind.
            let all: Vec<_> = c_steps.iter().flat_map(|s| &s.events).collect();
            assert!(
                all.iter()
                    .any(|e| matches!(e, Event::Malloc { got: Some(_), .. }))
            );
            assert!(all.iter().any(|e| matches!(
                e,
                Event::Malloc {
                    got: None,
                    len: REFUSED,
                    ..
                }
            )));
            assert!(all.iter().any(|e| matches!(
                e,
                Event::Realloc {
                    from: Some(_),
                    got: Some(_),
                    ..
                }
            )));
            assert!(
                all.iter()
                    .any(|e| matches!(e, Event::Free { block: Some(_) }))
            );
            // The zero-byte request was an allocation of its own, released once.
            assert_eq!(c_steps[4].result, 1);
            assert!(
                matches!(
                    c_steps[4].events[..],
                    [
                        Event::Malloc { len: 0, got: Some(n), .. },
                        Event::Free { block: Some(m) }
                    ] if n == m
                ),
                "{:?}",
                c_steps[4].events
            );
            assert_eq!(c_steps[5].error.0, ffi::GIT_ERROR_NOMEMORY as c_int);
        },
    );
}
