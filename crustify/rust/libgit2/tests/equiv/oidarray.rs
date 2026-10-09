//! Equivalence of `git2/oidarray.h`'s [`OidArray`] with the `git_oidarray` C code
//! builds, reads, edits and releases with `git_oidarray_dispose`.
//!
//! Two sources of arrays, each produced twice, independently, one copy per side:
//!
//! - libgit2's own results: `git_merge_bases` on the `testrepo.git` fixture, whose
//!   `ids` is a `git_array` grown with `git__reallocarray`. The Rust side adopts its
//!   copy's buffer as libgit2 hands it over, as the `git_merge_bases` wrapper will.
//! - arrays built the way `git_smart__shallow_roots` builds a transport's result, a
//!   `git__malloc`'d copy (NULL when empty), against [`OidArray::from_slice`].
//!
//! The C side then runs C code over the raw struct: indexed reads, the in-place
//! reversal of `git_oidarray__reverse` (hidden in libgit2.so), a lowered `count`,
//! replacing or moving `ids` out, a copy, and `git_oidarray_dispose`; the Rust side
//! runs the matching safe operations. After every step both record the array's
//! contents and the allocator events the step caused (pointers numbered by
//! allocation), and the records are compared step by step. Each test reruns alone in
//! a child process that installs the tracing allocator before `git_libgit2_init`.
//!
//! Documented divergences, outside these comparisons:
//!
//! - `free(NULL)` is not recorded: replacing or disposing an empty array in Rust never
//!   calls libgit2 (`OidArrayDispose` skips a NULL `ids`, which would otherwise read
//!   the allocator unsynchronised against a concurrent first `git_libgit2_init`),
//!   where C code passes NULL to `git__free`, a no-op the allocator still sees.
//! - A handle over an array whose `ids` is NULL with a non-zero `count` (no libgit2
//!   path produces one) reads as empty in Rust; C code would dereference NULL. The
//!   module's `unit_tests` cover it.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::size_of;
use std::process::Command;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::oid::oid::{Oid, OidRef, OidType};
use libgit2::libgit2::oid::oidarray::{OidArray, OidArrayRef};
use libgit2::libgit2::raw::OdbDataVec;

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
    let rc =
        unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR as c_int, &raw mut allocator) };
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

/// What one step left behind: the array's ids, as bytes, and its allocator events.
type Record = (Vec<(u8, [u8; 32])>, Vec<Event>);

/// Records one side's sequence of steps.
#[derive(Default)]
struct Recorder {
    steps: Vec<Record>,
    allocs: HashMap<usize, usize>,
    count: usize,
}

impl Recorder {
    /// Numbers an allocation made before the sequence, which the sequence takes over.
    fn adopt(&mut self, ptr: *const c_void) {
        if !ptr.is_null() {
            self.allocs.insert(ptr as usize, self.count);
            self.count += 1;
        }
    }

    fn number(&mut self, ptr: usize) -> usize {
        let n = self.count;
        self.allocs.insert(ptr, n);
        self.count += 1;
        n
    }

    /// Runs one step under tracing; it returns the array's ids as they read
    /// afterwards. Reading them allocates nothing from libgit2.
    fn step(&mut self, f: impl FnOnce() -> Vec<(u8, [u8; 32])>) {
        TRACE.lock().unwrap().clear();
        TRACING.store(true, Ordering::SeqCst);
        let ids = f();
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
        self.steps.push((ids, events));
    }
}

fn compare(c: &Recorder, rust: &Recorder) {
    assert_eq!(c.steps.len(), rust.steps.len());
    for (i, (c, rust)) in c.steps.iter().zip(&rust.steps).enumerate() {
        assert_eq!(c, rust, "step {i}");
    }
}

// --- the C side --------------------------------------------------------------

/// The ids of a raw array, read as C code indexes them.
fn c_contents(array: &ffi::git_oidarray) -> Vec<(u8, [u8; 32])> {
    (0..array.count)
        // SAFETY: `ids` holds at least `count` ids whenever `count` is non-zero.
        .map(|i| unsafe { *array.ids.add(i) })
        .map(|oid| (oid.type_, oid.id))
        .collect()
}

/// `git_oidarray__reverse`, which libgit2.so does not export, as `oidarray.c` writes it.
fn c_reverse(array: &mut ffi::git_oidarray) {
    for i in 0..array.count / 2 {
        let j = array.count - 1 - i;
        // SAFETY: `i < j < count`, both within the buffer; `git_oid_cpy` is a `memcpy`
        // of distinct ids.
        unsafe {
            let tmp = *array.ids.add(i);
            *array.ids.add(i) = *array.ids.add(j);
            *array.ids.add(j) = tmp;
        }
    }
}

/// A `git__malloc`'d copy of `count` ids, NULL for none: `git_smart__shallow_roots`.
fn c_copy(ids: *const ffi::git_oid, count: usize) -> ffi::git_oidarray {
    let len = count * size_of::<ffi::git_oid>();
    let mut out = ffi::git_oidarray {
        ids: ptr::null_mut(),
        count,
    };
    if len != 0 {
        // SAFETY: the routine ignores its backend; libgit2 is initialised.
        out.ids = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), len) }.cast();
        assert!(!out.ids.is_null());
        // SAFETY: `ids` holds `count` ids, the fresh buffer as many.
        unsafe { ptr::copy_nonoverlapping(ids, out.ids, count) };
    }
    out
}

fn c_dispose(array: &mut ffi::git_oidarray) {
    // SAFETY: `array` owns `ids` (NULL, or libgit2's allocation), released once here;
    // the caller does not read it again before reinitialising it.
    unsafe { ffi::git_oidarray_dispose(array) }
}

// --- the Rust side -----------------------------------------------------------

fn bytes(oid: OidRef<'_>) -> (u8, [u8; 32]) {
    (oid.raw_kind(), oid.id())
}

fn contents(array: OidArrayRef<'_>) -> Vec<(u8, [u8; 32])> {
    array.ids().iter().map(bytes).collect()
}

/// The safe API's reversal: swaps through the exclusive run's handles.
fn reverse(array: &mut libgit2::libgit2::oid::oidarray::OidArrayVal) {
    let mut m = array.as_mut();
    let mut ids = m.ids_mut();
    let n = ids.len();
    for i in 0..n / 2 {
        let a = ids.get(i).unwrap().to_oid();
        let b = ids.get(n - 1 - i).unwrap().to_oid();
        ids.get_mut(i).unwrap().copy_from(b.as_ref());
        ids.get_mut(n - 1 - i).unwrap().copy_from(a.as_ref());
    }
}

fn oid(seed: u8) -> Oid {
    let mut oid = Oid::zeroed();
    oid.as_mut().set_kind(OidType::Sha1);
    oid.as_mut().set_id(core::array::from_fn(|i| {
        seed.wrapping_mul(7).wrapping_add(i as u8)
    }));
    oid
}

// --- libgit2's results ---------------------------------------------------------

/// `git_merge_bases(a4a7dce, be3563a)` in the `testrepo.git` fixture, which has two.
fn merge_bases(repo: *mut ffi::git_repository) -> ffi::git_oidarray {
    let parse = |hex: &CStr| {
        let mut oid = ffi::git_oid::default();
        // SAFETY: `oid` is writable and `hex` NUL-terminated.
        let rc = unsafe { ffi::git_oid_from_string(&mut oid, hex.as_ptr(), ffi::GIT_OID_SHA1) };
        assert_eq!(rc, 0);
        oid
    };
    let one = parse(c"a4a7dce85cf63874e984719f4fdd239f5145052f");
    let two = parse(c"be3563ae3f795b2b4353bcce3a527ad0a4f7f644");
    let mut out = ffi::git_oidarray {
        ids: ptr::null_mut(),
        count: 0,
    };
    // SAFETY: `out` is writable, `repo` an open repository, `one` and `two` live ids.
    let rc = unsafe { ffi::git_merge_bases(&mut out, repo, &one, &two) };
    assert_eq!(rc, 0);
    out
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn merge_base_results_read_edit_and_release_alike() {
    isolated(
        concat!(
            module_path!(),
            "::merge_base_results_read_edit_and_release_alike"
        ),
        || {
            install_tracing_allocator();
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../tests/resources/testrepo.git\0"
            );
            let mut repo = ptr::null_mut();
            // SAFETY: `repo` receives the repository; `path` is NUL-terminated; no
            // ceiling directories.
            let rc = unsafe {
                ffi::git_repository_open_ext(
                    &mut repo,
                    path.as_ptr().cast(),
                    ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                    ptr::null(),
                )
            };
            assert_eq!(rc, 0);

            let mut c = Recorder::default();
            let mut raw = merge_bases(repo);
            assert_eq!(raw.count, 2);
            c.adopt(raw.ids.cast());
            c.step(|| c_contents(&raw));
            c.step(|| {
                c_reverse(&mut raw);
                c_contents(&raw)
            });
            c.step(|| {
                raw.count = 1;
                c_contents(&raw)
            });
            c.step(|| {
                c_dispose(&mut raw);
                Vec::new()
            });

            let mut rust = Recorder::default();
            let given = merge_bases(repo);
            rust.adopt(given.ids.cast());
            // SAFETY: `git_merge_bases` hands over `count` ids in libgit2's allocator,
            // which the caller owns and would release with `git_oidarray_dispose`
            // (`git__free`), as `OdbDataVec` does; `given` is not used again.
            let ids = unsafe { OdbDataVec::from_raw_parts_or_empty(given.ids.cast(), given.count) };
            let mut array = OidArray::from_ids(ids.unwrap());
            rust.step(|| contents(array.as_ref()));
            rust.step(|| {
                reverse(&mut array);
                contents(array.as_ref())
            });
            rust.step(|| {
                array.as_mut().truncate(1);
                contents(array.as_ref())
            });
            rust.step(|| {
                drop(array);
                Vec::new()
            });

            compare(&c, &rust);
            // Reversed, the fixture's bases are 9fd738e then c47800c.
            let first = &c.steps[1].0[0].1;
            assert_eq!(first[..4], [0x9f, 0xd7, 0x38, 0xe8]);
            assert_eq!(c.steps[3].1, [Event::Free { alloc: Some(0) }]);
            // SAFETY: `repo` is open and released once.
            unsafe { ffi::git_repository_free(repo) };
        },
    );
}

// --- arrays built by hand ----------------------------------------------------------

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn built_arrays_match_c_built_ones() {
    isolated(
        concat!(module_path!(), "::built_arrays_match_c_built_ones"),
        || {
            install_tracing_allocator();
            for n in [0usize, 1, 3, 8] {
                let src: Vec<Oid> = (0..n).map(|i| oid(i as u8)).collect();
                let next: Vec<Oid> = (0..n + 1).map(|i| oid(100 + i as u8)).collect();
                let raw_src = src.as_ptr().cast::<ffi::git_oid>();
                let raw_next = next.as_ptr().cast::<ffi::git_oid>();

                let mut c = Recorder::default();
                let empty = ffi::git_oidarray {
                    ids: ptr::null_mut(),
                    count: 0,
                };
                let (mut arr, mut copy, mut moved) = (empty, empty, empty);
                c.step(|| {
                    arr = c_copy(raw_src, n);
                    c_contents(&arr)
                });
                c.step(|| {
                    copy = c_copy(arr.ids, arr.count);
                    c_contents(&copy)
                });
                c.step(|| {
                    c_dispose(&mut copy);
                    c_contents(&arr)
                });
                // The replacement is built first, as `set_ids` receives it built.
                c.step(|| {
                    let fresh = c_copy(raw_next, n + 1);
                    c_dispose(&mut arr);
                    arr = fresh;
                    c_contents(&arr)
                });
                c.step(|| {
                    arr.count = n / 2;
                    c_contents(&arr)
                });
                c.step(|| {
                    moved = arr;
                    arr = empty;
                    c_contents(&moved)
                });
                c.step(|| {
                    c_dispose(&mut arr);
                    Vec::new()
                });
                c.step(|| {
                    c_dispose(&mut moved);
                    Vec::new()
                });

                let mut rust = Recorder::default();
                let mut array = OidArray::new();
                let (mut copy, mut moved) = (None, None);
                rust.step(|| {
                    array = OidArray::from_slice(&src).unwrap();
                    contents(array.as_ref())
                });
                rust.step(|| {
                    copy = array.as_ref().to_oid_array();
                    contents(copy.as_ref().unwrap().as_ref())
                });
                rust.step(|| {
                    drop(copy.take());
                    contents(array.as_ref())
                });
                rust.step(|| {
                    array
                        .as_mut()
                        .set_ids(OdbDataVec::from_slice(&next).unwrap());
                    contents(array.as_ref())
                });
                rust.step(|| {
                    array.as_mut().truncate(n / 2);
                    contents(array.as_ref())
                });
                rust.step(|| {
                    let ids: &OdbDataVec<Oid> = moved.insert(array.as_mut().take_ids());
                    ids.as_handles().iter().map(bytes).collect()
                });
                rust.step(|| {
                    drop(array);
                    Vec::new()
                });
                rust.step(|| {
                    drop(moved);
                    Vec::new()
                });

                compare(&c, &rust);
            }
        },
    );
}
