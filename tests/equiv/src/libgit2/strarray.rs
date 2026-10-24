//! Equivalence of `git2/strarray.h`'s [`StrArray`] and [`StrArrayView`] with the
//! `git_strarray` C code builds, reads, edits and releases with `git_strarray_dispose`.
//!
//! Three sources of arrays, each produced twice, independently, one copy per side:
//!
//! - libgit2's own results: `GIT_OPT_GET_EXTENSIONS`, a detached `git_vector` of `git__strdup`
//!   copies with spare capacity, and, with every builtin disabled, a non-NULL array of no strings.
//!   The Rust side adopts its copy as libgit2 hands it over, as the producers' wrappers will.
//! - copies, built as `git_strarray_copy` builds them (hidden in libgit2.so, so rebuilt here over
//!   the exported allocator, in its order: `git__calloc` the array, then `git__strdup` each
//!   string), against [`StrArray::from_strs`] and [`StrArrayRef::to_str_array`].
//! - argument arrays, a `char *` array C code builds over its own strings, against a
//!   [`StrArrayView`].
//!
//! The C side then runs C code over the raw struct: indexed reads, a lowered `count`
//! with the dropped strings freed, replacing the strings after disposing them, moving
//! them out, handing them to another struct, and `git_strarray_dispose`; the Rust side
//! runs the matching safe operations. After every step both record the array's
//! strings and the allocator events the step caused (pointers numbered by allocation),
//! and the records are compared step by step. Each test reruns alone in a child
//! process that installs the tracing allocator before `git_libgit2_init`.
//!
//! Documented divergences, outside these comparisons:
//!
//! - `free(NULL)` is not recorded: disposing an array without a buffer in Rust never calls libgit2
//!   (`StrArrayDispose` skips a NULL `strings`, which would otherwise read the allocator
//!   unsynchronised against a concurrent first `git_libgit2_init`), where C code passes NULL to
//!   `git__free`, a no-op the allocator still sees.
//! - `git_strarray_copy` silently skips NULL elements of its source; no Rust array can hold one
//!   (views are built from `&CStr`, owned runs refuse them), and libgit2's producers store none.
//! - A handle over an array whose `strings` is NULL with a non-zero `count` (no libgit2 path
//!   produces one) reads as empty in Rust; C code would dereference NULL. The module's `unit_tests`
//!   cover it.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::mem::size_of;
use std::process::Command;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::odb::OdbDataStrVec;
use libgit2::libgit2::src::libgit2::strarray::{StrArray, StrArrayRef, StrArrayVal, StrArrayView};

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
    Alloc { ptr: usize, len: usize },
    Realloc { from: usize, ptr: usize, len: usize },
    Free { ptr: usize },
}

static TRACING: AtomicBool = AtomicBool::new(false);
static TRACE: Mutex<Vec<Raw>> = Mutex::new(Vec::new());

fn trace(event: Raw) {
    if TRACING.load(Ordering::SeqCst) {
        TRACE.lock().unwrap().push(event);
    }
}

unsafe extern "C" fn traced_malloc(len: usize, _: *const c_char, _: c_int) -> *mut c_void {
    // SAFETY: plain libc allocation.
    let ptr = unsafe { malloc(len) };
    trace(Raw::Alloc {
        ptr: ptr as usize,
        len,
    });
    ptr
}

unsafe extern "C" fn traced_realloc(
    from: *mut c_void,
    len: usize,
    _: *const c_char,
    _: c_int,
) -> *mut c_void {
    // SAFETY: `from` is NULL or came from this allocator, i.e. libc.
    let ptr = unsafe { realloc(from, len) };
    trace(Raw::Realloc {
        from: from as usize,
        ptr: ptr as usize,
        len,
    });
    ptr
}

unsafe extern "C" fn traced_free(ptr: *mut c_void) {
    if !ptr.is_null() {
        trace(Raw::Free { ptr: ptr as usize });
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
}

/// One allocator event, with pointers replaced by the number of their allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Alloc { len: usize },
    Realloc { from: Option<usize>, len: usize },
    Free { alloc: Option<usize> },
}

/// What one step left behind: the array's strings, as bytes, and its allocator
/// events.
type Record = (Vec<Vec<u8>>, Vec<Event>);

/// Records one side's sequence of steps.
#[derive(Default)]
struct Recorder {
    steps: Vec<Record>,
    allocs: HashMap<usize, usize>,
    count: usize,
}

impl Recorder {
    fn number(&mut self, ptr: usize) -> usize {
        let n = self.count;
        self.allocs.insert(ptr, n);
        self.count += 1;
        n
    }

    /// Runs one step under tracing; it returns the strings as they read afterwards.
    /// Reading them allocates nothing from libgit2.
    fn step(&mut self, f: impl FnOnce() -> Vec<Vec<u8>>) {
        TRACE.lock().unwrap().clear();
        TRACING.store(true, Ordering::SeqCst);
        let strings = f();
        TRACING.store(false, Ordering::SeqCst);
        let raw = std::mem::take(&mut *TRACE.lock().unwrap());
        let events = raw
            .into_iter()
            .map(|e| match e {
                Raw::Alloc { ptr, len } => {
                    assert_ne!(ptr, 0, "no traced allocation fails");
                    self.number(ptr);
                    Event::Alloc { len }
                }
                Raw::Realloc { from, ptr, len } => {
                    let from = self.allocs.remove(&from);
                    self.number(ptr);
                    Event::Realloc { from, len }
                }
                Raw::Free { ptr } => Event::Free {
                    alloc: self.allocs.remove(&ptr),
                },
            })
            .collect();
        self.steps.push((strings, events));
    }
}

fn compare(c: &Recorder, rust: &Recorder) {
    assert_eq!(c.steps.len(), rust.steps.len());
    for (i, (c, rust)) in c.steps.iter().zip(&rust.steps).enumerate() {
        assert_eq!(c, rust, "step {i}");
    }
}

// --- the C side --------------------------------------------------------------

const EMPTY: ffi::git_strarray = ffi::git_strarray {
    strings: ptr::null_mut(),
    count: 0,
};

/// The strings of a raw array, read as C code indexes them.
fn c_contents(array: &ffi::git_strarray) -> Vec<Vec<u8>> {
    (0..array.count)
        .map(|i| {
            // SAFETY: `strings` holds at least `count` live strings whenever `count` is
            // non-zero, and `i < count`.
            let slot = unsafe { array.strings.add(i) };
            // SAFETY: as above.
            let s = unsafe { *slot };
            // SAFETY: as above, a live NUL-terminated string.
            unsafe { CStr::from_ptr(s) }
        })
        .map(|s| s.to_bytes().to_vec())
        .collect()
}

/// `git__malloc` over the exported allocator; libgit2 is initialised.
fn c_alloc(len: usize) -> *mut c_void {
    // SAFETY: the routine ignores its backend; libgit2 is initialised.
    let p = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), len) };
    assert!(!p.is_null());
    p
}

/// `git__free` over the exported allocator.
fn c_free(p: *mut c_void) {
    // SAFETY: `p` is a live `git__malloc` allocation the caller gives up.
    unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), p) }
}

/// `git_strarray_copy`, as `strarray.c` writes it, over the exported allocator.
fn c_copy(src: &ffi::git_strarray) -> ffi::git_strarray {
    let mut tgt = EMPTY;
    if src.count == 0 {
        return tgt;
    }
    tgt.strings = c_alloc(src.count * size_of::<*mut c_char>()).cast();
    // SAFETY: the fresh array holds `count` pointers (`git__calloc` zeroes them).
    unsafe { ptr::write_bytes(tgt.strings, 0, src.count) };
    for i in 0..src.count {
        // SAFETY: `src` holds `count` elements, and `i < count`.
        let slot = unsafe { src.strings.add(i) };
        // SAFETY: as above.
        let s = unsafe { *slot };
        if s.is_null() {
            continue;
        }
        // SAFETY: a live NUL-terminated string.
        let bytes = unsafe { CStr::from_ptr(s) }.to_bytes_with_nul();
        let dup = c_alloc(bytes.len()).cast::<u8>();
        // SAFETY: `dup` is fresh and `bytes.len()` long; `tgt.count < src.count`.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), dup, bytes.len()) };
        // SAFETY: `tgt` has room for one more string past `count`.
        let slot = unsafe { tgt.strings.add(tgt.count) };
        // SAFETY: as above.
        unsafe { *slot = dup.cast() };
        tgt.count += 1;
    }
    tgt
}

/// C code shortening an owned array: the dropped strings freed, then `count` lowered.
fn c_truncate(array: &mut ffi::git_strarray, len: usize) {
    if len >= array.count {
        return;
    }
    for i in len..array.count {
        // SAFETY: `i < count`: within the array.
        let slot = unsafe { array.strings.add(i) };
        // SAFETY: an owned string, released once.
        c_free(unsafe { *slot }.cast());
    }
    array.count = len;
}

fn c_dispose(array: &mut ffi::git_strarray) {
    // SAFETY: `array` owns its array and strings (NULL, or libgit2's allocations),
    // released once here; it is zeroed afterwards.
    unsafe { ffi::git_strarray_dispose(array) }
}

/// C code moving the strings out, leaving the struct zeroed.
fn c_take(array: &mut ffi::git_strarray) -> ffi::git_strarray {
    std::mem::replace(array, EMPTY)
}

/// `GIT_OPT_GET_EXTENSIONS` into a fresh struct.
fn c_extensions() -> ffi::git_strarray {
    let mut out = EMPTY;
    // SAFETY: `GIT_OPT_GET_EXTENSIONS` takes a `git_strarray *`, which it fills.
    let rc =
        unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_GET_EXTENSIONS.cast_signed(), &raw mut out) };
    assert_eq!(rc, 0);
    out
}

/// `GIT_OPT_SET_EXTENSIONS`; this child owns libgit2's globals.
fn set_extensions(names: &[&CStr]) {
    let ptrs: Vec<*const c_char> = names.iter().map(|s| s.as_ptr()).collect();
    // SAFETY: an array of `len` strings, which libgit2 copies.
    let rc = unsafe {
        ffi::git_libgit2_opts(
            ffi::GIT_OPT_SET_EXTENSIONS.cast_signed(),
            ptrs.as_ptr(),
            ptrs.len(),
        )
    };
    assert_eq!(rc, 0);
}

/// An argument array as C code builds one: a `char *` array over its own strings.
struct CArgs {
    _owned: Vec<CString>,
    ptrs: Vec<*mut c_char>,
}

impl CArgs {
    fn new(strs: &[&[u8]]) -> Self {
        let owned: Vec<CString> = strs.iter().map(|s| CString::new(*s).unwrap()).collect();
        let ptrs = owned.iter().map(|s| s.as_ptr().cast_mut()).collect();
        Self {
            _owned: owned,
            ptrs,
        }
    }

    fn raw(&mut self) -> ffi::git_strarray {
        if self.ptrs.is_empty() {
            return EMPTY;
        }
        ffi::git_strarray {
            strings: self.ptrs.as_mut_ptr(),
            count: self.ptrs.len(),
        }
    }
}

// --- the Rust side -----------------------------------------------------------

fn contents(array: StrArrayRef<'_>) -> Vec<Vec<u8>> {
    array.iter().map(|s| s.to_bytes().to_vec()).collect()
}

/// libgit2's extension list, adopted as libgit2 hands it out.
fn extensions() -> StrArrayVal {
    let raw = c_extensions();
    // SAFETY: libgit2's result owns an array of its allocator holding `count` non-NULL
    // strings from the same allocator, handed over whole.
    let strings = unsafe { OdbDataStrVec::from_raw_parts(raw.strings, raw.count) }.unwrap();
    StrArray::from_strings(strings)
}

/// The same edit sequence over libgit2's result, on both sides: read, copy, shorten
/// the copy, replace the result's strings with the copy's, move them out, dispose
/// the emptied struct, and release the moved strings.
fn result_sequence() {
    let mut c = Recorder::default();
    let mut rust = Recorder::default();

    let mut a = EMPTY;
    let mut a_copy = EMPTY;
    let mut a_moved = EMPTY;
    c.step(|| {
        a = c_extensions();
        c_contents(&a)
    });
    c.step(|| {
        a_copy = c_copy(&a);
        c_contents(&a_copy)
    });
    c.step(|| {
        c_truncate(&mut a_copy, 2);
        c_contents(&a_copy)
    });
    c.step(|| {
        c_truncate(&mut a_copy, 5);
        c_contents(&a_copy)
    });
    c.step(|| c_contents(&a));
    c.step(|| {
        c_dispose(&mut a);
        a = c_take(&mut a_copy);
        c_contents(&a)
    });
    c.step(|| {
        a_moved = c_take(&mut a);
        c_contents(&a)
    });
    c.step(|| {
        c_dispose(&mut a);
        c_dispose(&mut a_copy);
        c_contents(&a)
    });
    c.step(|| {
        c_dispose(&mut a_moved);
        c_contents(&a_moved)
    });

    let mut b = None;
    let mut b_copy = None;
    let mut b_moved = None;
    rust.step(|| contents(b.insert(extensions()).as_ref()));
    rust.step(|| {
        let copy = b.as_ref().unwrap().as_ref().to_str_array().unwrap();
        contents(b_copy.insert(copy).as_ref())
    });
    rust.step(|| {
        let copy = b_copy.as_mut().unwrap();
        copy.as_mut().truncate(2);
        contents(copy.as_ref())
    });
    rust.step(|| {
        let copy = b_copy.as_mut().unwrap();
        copy.as_mut().truncate(5);
        contents(copy.as_ref())
    });
    rust.step(|| contents(b.as_ref().unwrap().as_ref()));
    rust.step(|| {
        let strings = b_copy.as_mut().unwrap().as_mut().take_strings();
        let b = b.as_mut().unwrap();
        b.as_mut().set_strings(strings);
        contents(b.as_ref())
    });
    rust.step(|| {
        let b = b.as_mut().unwrap();
        b_moved = Some(b.as_mut().take_strings());
        contents(b.as_ref())
    });
    rust.step(|| {
        let emptied = b.take().unwrap();
        let read = contents(emptied.as_ref());
        drop(emptied);
        drop(b_copy.take());
        read
    });
    rust.step(|| {
        drop(b_moved.take());
        Vec::new()
    });

    compare(&c, &rust);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn extension_results_read_copy_edit_and_release_alike() {
    isolated(
        concat!(
            module_path!(),
            "::extension_results_read_copy_edit_and_release_alike"
        ),
        || {
            install_tracing_allocator();
            set_extensions(&[c"zeta", c"alpha", c"!noop", c"mid"]);
            result_sequence();
            // The builtins alone, with a vector that never grew.
            set_extensions(&[]);
            result_sequence();
        },
    );
}

/// libgit2's eager vectors hand out a non-NULL array of no strings: both sides read it
/// as empty, copy it without allocating, and release the array once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn allocated_empty_results_release_alike() {
    isolated(
        concat!(module_path!(), "::allocated_empty_results_release_alike"),
        || {
            install_tracing_allocator();
            set_extensions(&[
                c"!noop",
                c"!objectformat",
                c"!worktreeconfig",
                c"!preciousobjects",
                c"!refstorage",
                c"!relativeworktrees",
            ]);
            let mut c = Recorder::default();
            let mut rust = Recorder::default();

            let mut a = EMPTY;
            let mut a_copy = EMPTY;
            c.step(|| {
                a = c_extensions();
                assert!(!a.strings.is_null() && a.count == 0);
                c_contents(&a)
            });
            c.step(|| {
                a_copy = c_copy(&a);
                c_contents(&a_copy)
            });
            c.step(|| {
                c_truncate(&mut a, 0);
                c_dispose(&mut a_copy);
                c_dispose(&mut a);
                c_contents(&a)
            });

            let mut b = None;
            let mut b_copy = None;
            rust.step(|| contents(b.insert(extensions()).as_ref()));
            rust.step(|| {
                let copy = b.as_ref().unwrap().as_ref().to_str_array().unwrap();
                contents(b_copy.insert(copy).as_ref())
            });
            rust.step(|| {
                let mut b = b.take().unwrap();
                b.as_mut().truncate(0);
                drop(b_copy.take());
                drop(b);
                Vec::new()
            });
            compare(&c, &rust);
        },
    );
}

/// The string sets the built arrays hold: none, empty strings, long ones, many.
fn string_sets() -> Vec<Vec<Vec<u8>>> {
    vec![
        vec![],
        vec![b"".to_vec()],
        vec![
            b"refs/heads/main".to_vec(),
            b"".to_vec(),
            b"refs/tags/v1.0".to_vec(),
        ],
        vec![vec![b'x'; 4096], b"y".to_vec()],
        (0..40)
            .map(|i| format!("path/{i}/*.c").into_bytes())
            .collect(),
    ]
}

/// Copies built by both sides from the same argument strings, then edited and released
/// alike; the Rust copy is finally handed to C's `git_strarray_dispose`, as a Rust
/// result handed to libgit2 is.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copies_build_edit_and_release_alike() {
    isolated(
        concat!(module_path!(), "::copies_build_edit_and_release_alike"),
        || {
            install_tracing_allocator();
            for set in string_sets() {
                let strs: Vec<&[u8]> = set.iter().map(Vec::as_slice).collect();
                let mut c = Recorder::default();
                let mut rust = Recorder::default();

                let mut args = CArgs::new(&strs);
                let src = args.raw();
                let mut a = EMPTY;
                let mut a2 = EMPTY;
                c.step(|| {
                    a = c_copy(&src);
                    c_contents(&a)
                });
                c.step(|| {
                    a2 = c_copy(&a);
                    c_truncate(&mut a2, 1);
                    c_contents(&a2)
                });
                c.step(|| {
                    c_dispose(&mut a2);
                    let half = a.count / 2;
                    c_truncate(&mut a, half);
                    c_contents(&a)
                });
                c.step(|| {
                    c_dispose(&mut a);
                    c_contents(&a)
                });

                let owned: Vec<CString> = strs.iter().map(|s| CString::new(*s).unwrap()).collect();
                let mut b = None;
                let mut b2 = None;
                rust.step(|| contents(b.insert(StrArray::from_strs(&owned).unwrap()).as_ref()));
                rust.step(|| {
                    let mut copy = b.as_ref().unwrap().as_ref().to_str_array().unwrap();
                    copy.as_mut().truncate(1);
                    contents(b2.insert(copy).as_ref())
                });
                rust.step(|| {
                    drop(b2.take());
                    let b = b.as_mut().unwrap();
                    let half = b.as_ref().len() / 2;
                    b.as_mut().truncate(half);
                    contents(b.as_ref())
                });
                rust.step(|| {
                    let (strings, count) =
                        b.take().unwrap().as_mut().take_strings().into_raw_parts();
                    let mut raw = ffi::git_strarray { strings, count };
                    c_dispose(&mut raw);
                    c_contents(&raw)
                });
                compare(&c, &rust);
            }
        },
    );
}

/// A view reads as C reads the argument array it stands for, allocates nothing from
/// libgit2 to build or drop, and copies into libgit2's allocator as `git_strarray_copy`
/// copies the argument array.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn views_read_and_copy_as_argument_arrays() {
    isolated(
        concat!(module_path!(), "::views_read_and_copy_as_argument_arrays"),
        || {
            install_tracing_allocator();
            for set in string_sets() {
                let strs: Vec<&[u8]> = set.iter().map(Vec::as_slice).collect();
                let mut c = Recorder::default();
                let mut rust = Recorder::default();

                let mut args = None;
                let mut a_copy = EMPTY;
                c.step(|| {
                    let args = args.insert(CArgs::new(&strs));
                    c_contents(&args.raw())
                });
                c.step(|| {
                    a_copy = c_copy(&args.as_mut().unwrap().raw());
                    c_contents(&a_copy)
                });
                c.step(|| {
                    drop(args.take());
                    c_dispose(&mut a_copy);
                    c_contents(&a_copy)
                });

                let owned: Vec<CString> = strs.iter().map(|s| CString::new(*s).unwrap()).collect();
                let mut view = None;
                let mut b_copy = None;
                rust.step(|| contents(view.insert(StrArrayView::new(&owned)).as_ref()));
                rust.step(|| {
                    let copy = view.as_ref().unwrap().as_ref().to_str_array().unwrap();
                    contents(b_copy.insert(copy).as_ref())
                });
                rust.step(|| {
                    drop(view.take());
                    drop(b_copy.take());
                    Vec::new()
                });
                compare(&c, &rust);
            }
        },
    );
}
