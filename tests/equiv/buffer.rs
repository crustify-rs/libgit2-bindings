//! Equivalence of `git2/buffer.h`'s [`Buf`] with the `git_buf` C code fills, reads,
//! reuses and releases with `git_buf_dispose`.
//!
//! Two sources of buffers, each produced twice, independently, one copy per side:
//!
//! - libgit2's own results: `GIT_OPT_GET_USER_AGENT` (an owned string) and an empty
//!   `GIT_OPT_GET_TEMPLATE_PATH` (the unallocated static empty string), filled into a fresh buffer
//!   and into one that already owns an allocation, which libgit2 reuses or reallocates. The Rust
//!   side adopts its copy as libgit2 leaves it, as the wrappers of those routines will.
//! - buffers built the way libgit2 hands one out, a `git__malloc`'d copy NUL-terminated at `size`
//!   with `reserved` its allocation size, against [`Buf::from_bytes`].
//!
//! The C side then runs C code over the raw struct: reads of `ptr` / `size` /
//! `reserved`, an in-place `git_buf_set` (hidden in libgit2.so under
//! `GIT_DEPRECATE_HARD`, rebuilt here), moving `ptr` out, and `git_buf_dispose`; the
//! Rust side runs the matching safe operations. After every step both record the
//! buffer's contents and the allocator events the step caused (pointers numbered by
//! allocation), and the records are compared step by step. Each test reruns alone in a
//! child process that installs the tracing allocator before `git_libgit2_init`.
//!
//! Documented divergences, outside these comparisons:
//!
//! - `free(NULL)` is not recorded: disposing a buffer that owns nothing in Rust never calls libgit2
//!   (`BufDispose` skips `reserved == 0`, where `git_buf_dispose` would pass a NULL `ptr` to
//!   `git__free`, reading the allocator unsynchronised against a concurrent first
//!   `git_libgit2_init`).
//! - A disposed or taken Rust buffer is `GIT_BUF_INIT` (`ptr` NULL); `git_buf_dispose` points `ptr`
//!   at libgit2's static empty string instead. Both own nothing and read as empty, and libgit2
//!   treats both alike when they are passed in again.
//! - Growing a buffer: `git_buf_set` reallocates with `git__realloc`, which libgit2 does not
//!   export; `BufMut::set_bytes` allocates afresh and releases the old allocation. Contents and
//!   capacity are compared, not the events.
//! - A handle over a buffer with `reserved == 0` and a non-zero `size` (no public libgit2 path
//!   produces one) reads as empty in Rust; C code would read `size` bytes. The module's
//!   `unit_tests` cover it.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::process::Command;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use libgit2::ffi;
use libgit2::libgit2::core::buffer::{Buf, BufRef, BufVal};
use libgit2::libgit2::raw::OdbDataStrFree;

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

unsafe extern "C" fn traced_malloc(len: usize, _: *const c_char, _: c_int) -> *mut c_void {
    // SAFETY: plain libc allocation.
    let ptr = unsafe { malloc(len) };
    if TRACING.load(Ordering::SeqCst) {
        TRACE.lock().unwrap().push(Raw::Alloc {
            ptr: ptr as usize,
            len,
        });
    }
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
    if TRACING.load(Ordering::SeqCst) {
        TRACE.lock().unwrap().push(Raw::Realloc {
            from: from as usize,
            ptr: ptr as usize,
            len,
        });
    }
    ptr
}

unsafe extern "C" fn traced_free(ptr: *mut c_void) {
    if TRACING.load(Ordering::SeqCst) && !ptr.is_null() {
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
    // SAFETY: no arguments; the result is not read.
    unsafe { ffi::git_error_last() };
}

/// One allocator event, with pointers replaced by the index of their allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Alloc { len: usize },
    Realloc { from: usize, len: usize },
    Free { alloc: usize },
}

/// A buffer's observable state: its contents and its allocation size.
#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    bytes: Vec<u8>,
    c_str: Vec<u8>,
    capacity: usize,
}

/// Records one sequence of steps: each step's events, normalised separately, and the
/// buffer's state after it.
#[derive(Default)]
struct Recorder {
    steps: Vec<(Vec<Event>, State)>,
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

    #[expect(clippy::match_same_arms, reason = "each arm names its own case")]
    fn step(&mut self, f: impl FnOnce() -> State) {
        TRACE.lock().unwrap().clear();
        TRACING.store(true, Ordering::SeqCst);
        let state = f();
        TRACING.store(false, Ordering::SeqCst);
        let raw = std::mem::take(&mut *TRACE.lock().unwrap());
        let events = raw
            .into_iter()
            .map(|e| match e {
                Raw::Alloc { ptr, len } => {
                    self.number(ptr);
                    Event::Alloc { len }
                }
                // `realloc(NULL)` allocates: `git_str` grows an unallocated string so.
                Raw::Realloc { from: 0, ptr, len } => {
                    self.number(ptr);
                    Event::Alloc { len }
                }
                Raw::Realloc { from, ptr, len } => {
                    let from = self
                        .allocs
                        .remove(&from)
                        .expect("reallocated what the sequence allocated");
                    self.allocs.insert(ptr, from);
                    Event::Realloc { from, len }
                }
                Raw::Free { ptr } => Event::Free {
                    alloc: self
                        .allocs
                        .remove(&ptr)
                        .expect("freed what the sequence allocated"),
                },
            })
            .collect();
        self.steps.push((events, state));
    }
}

// --- the C side -----------------------------------------------------------------

fn c_state(buf: &ffi::git_buf) -> State {
    if buf.ptr.is_null() {
        return State {
            bytes: Vec::new(),
            c_str: Vec::new(),
            capacity: buf.reserved,
        };
    }
    // SAFETY: a non-NULL `ptr` holds `size` bytes and a NUL: libgit2's results and the
    // buffers below all do.
    let bytes = unsafe { std::slice::from_raw_parts(buf.ptr.cast::<u8>(), buf.size) }.to_vec();
    // SAFETY: as above.
    let c_str = unsafe { CStr::from_ptr(buf.ptr) }.to_bytes().to_vec();
    State {
        bytes,
        c_str,
        capacity: buf.reserved,
    }
}

fn c_get(opt: u32, buf: &mut ffi::git_buf) {
    // SAFETY: the `GET` options take a `git_buf *` that is zeroed, disposed or owns a
    // libgit2 allocation, which they reuse or replace.
    let rc = unsafe { ffi::git_libgit2_opts(opt.cast_signed(), &raw mut *buf) };
    assert_eq!(rc, 0);
}

fn c_dispose(buf: &mut ffi::git_buf) {
    // SAFETY: `buf` is zeroed, disposed or as libgit2 / `c_from_bytes` left it.
    unsafe { ffi::git_buf_dispose(buf) }
}

/// A buffer as libgit2 hands one out, holding a copy of `bytes`.
fn c_from_bytes(bytes: &[u8]) -> ffi::git_buf {
    // SAFETY: the routine ignores `backend`; any length.
    let p =
        unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), bytes.len() + 1) }.cast::<u8>();
    assert!(!p.is_null());
    // SAFETY: `p` holds `bytes.len() + 1` writable bytes, distinct from `bytes`.
    unsafe { p.copy_from_nonoverlapping(bytes.as_ptr(), bytes.len()) };
    // SAFETY: as above.
    let value = unsafe { p.add(bytes.len()) };
    // SAFETY: as above.
    unsafe { value.write(0) };
    ffi::git_buf {
        ptr: p.cast(),
        reserved: bytes.len() + 1,
        size: bytes.len(),
    }
}

/// `git_buf_set` for contents that fit the allocation (`buf.c`): moved in place,
/// NUL-terminated.
fn c_set_in_place(buf: &mut ffi::git_buf, bytes: &[u8]) {
    assert!(bytes.len() < buf.reserved);
    // SAFETY: `buf` owns `reserved` writable bytes, of which `bytes.len() + 1` fit.
    unsafe { ptr::copy(bytes.as_ptr(), buf.ptr.cast::<u8>(), bytes.len()) };
    // SAFETY: as above.
    let value = unsafe { buf.ptr.add(bytes.len()) };
    // SAFETY: as above.
    unsafe { value.write(0) };
    buf.size = bytes.len();
}

fn set_opt(opt: u32, s: *const c_char) {
    // SAFETY: the `SET` options taking a string copy it, or restore the default for NULL.
    assert_eq!(unsafe { ffi::git_libgit2_opts(opt.cast_signed(), s) }, 0);
}

// --- the Rust side ----------------------------------------------------------------

fn r_state(buf: BufRef<'_>) -> State {
    State {
        bytes: buf.as_bytes().to_vec(),
        c_str: buf.as_c_str().to_bytes().to_vec(),
        capacity: buf.capacity(),
    }
}

/// libgit2 fills the Rust buffer: handed over raw, adopted back as libgit2 left it.
fn r_get(opt: u32, buf: &mut BufVal) {
    let mut raw = buf.as_mut().take().into_inner().into_raw();
    c_get(opt, &mut raw);
    // SAFETY: `raw` is as libgit2 left it.
    buf.as_mut().set(unsafe { Buf::from_raw(raw) });
}

// --- the comparisons ------------------------------------------------------------

/// libgit2's results, filled fresh, refilled (reused in place, then reallocated),
/// emptied, and released.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn libgit2_results_read_reuse_and_release_alike() {
    isolated(
        concat!(
            module_path!(),
            "::libgit2_results_read_reuse_and_release_alike"
        ),
        || {
            install_tracing_allocator();
            let ua = ffi::GIT_OPT_GET_USER_AGENT;
            let tp = ffi::GIT_OPT_GET_TEMPLATE_PATH;
            set_opt(ffi::GIT_OPT_SET_TEMPLATE_PATH, c"".as_ptr());

            let script: [(&CStr, u32); 6] = [
                (c"crustify/1.0", ua),
                (c"crustify/2.0", ua),
                (c"a user agent much longer than any allocation so far", ua),
                (c"", tp),
                (c"x", ua),
                (c"", tp),
            ];
            let (mut c, mut r) = (Recorder::default(), Recorder::default());
            let mut cbuf = ffi::git_buf {
                ptr: ptr::null_mut(),
                reserved: 0,
                size: 0,
            };
            let mut rbuf = Buf::new();
            for (agent, opt) in script {
                set_opt(ffi::GIT_OPT_SET_USER_AGENT, agent.as_ptr());
                c.step(|| {
                    c_get(opt, &mut cbuf);
                    c_state(&cbuf)
                });
                r.step(|| {
                    r_get(opt, &mut rbuf);
                    r_state(rbuf.as_ref())
                });
            }
            // An empty result into a fresh buffer: the static empty string, unallocated.
            let mut cempty = ffi::git_buf {
                ptr: ptr::null_mut(),
                reserved: 0,
                size: 0,
            };
            let mut rempty = Buf::new();
            c.step(|| {
                c_get(tp, &mut cempty);
                c_state(&cempty)
            });
            r.step(|| {
                r_get(tp, &mut rempty);
                r_state(rempty.as_ref())
            });
            assert!(!cempty.ptr.is_null() && cempty.reserved == 0);

            c.step(|| {
                c_dispose(&mut cempty);
                c_dispose(&mut cbuf);
                c_state(&cbuf)
            });
            r.step(|| {
                drop(rempty);
                drop(rbuf);
                State {
                    bytes: Vec::new(),
                    c_str: Vec::new(),
                    capacity: 0,
                }
            });
            assert_eq!(c.steps, r.steps);

            let events: Vec<_> = c.steps.iter().map(|(e, _)| e.clone()).collect();
            assert!(
                matches!(events[0][..], [Event::Alloc { .. }, ..]),
                "{events:?}"
            );
            assert!(
                events[1]
                    .iter()
                    .all(|e| !matches!(e, Event::Free { alloc: 0 }))
            );
            assert!(
                events[2]
                    .iter()
                    .any(|e| matches!(e, Event::Realloc { from: 0, .. })),
                "a longer result reallocates the buffer's own allocation: {events:?}"
            );
            assert_eq!(c.steps[3].1.bytes, b"");
            assert!(
                c.steps[3].1.capacity > 0,
                "an empty result keeps the allocation"
            );
            assert_eq!(c.steps[6].1.capacity, 0);
            assert_eq!(events[6], [], "filled without allocating");
            assert_eq!(
                events[7],
                [Event::Free { alloc: 0 }],
                "one allocation, freed once"
            );
            assert_eq!((cbuf.reserved, cbuf.size), (0, 0));

            set_opt(ffi::GIT_OPT_SET_TEMPLATE_PATH, ptr::null());
            set_opt(ffi::GIT_OPT_SET_USER_AGENT, ptr::null());
        },
    );
}

/// Buffers built as libgit2 hands one out, then read, copied, edited in place, grown,
/// reused by libgit2, moved out and released.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
#[expect(
    clippy::too_many_lines,
    reason = "one scenario, its steps run and compared in order"
)]
#[expect(
    clippy::similar_names,
    reason = "the names pair each value with its counterpart on the other side"
)]
fn rust_built_buffers_match_libgit2_shaped_ones() {
    isolated(
        concat!(
            module_path!(),
            "::rust_built_buffers_match_libgit2_shaped_ones"
        ),
        || {
            install_tracing_allocator();
            set_opt(ffi::GIT_OPT_SET_USER_AGENT, c"crustify/1.0".as_ptr());
            let long: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 1).to_le_bytes()[0]).collect();
            let contents: [&[u8]; 5] = [b"a", b"refs/heads/main", b"pa\0ck\0", b"\0", &long];
            for bytes in contents {
                let (mut c, mut r) = (Recorder::default(), Recorder::default());
                let mut cbuf = ffi::git_buf {
                    ptr: ptr::null_mut(),
                    reserved: 0,
                    size: 0,
                };
                let mut rbuf = Buf::new();
                c.step(|| {
                    cbuf = c_from_bytes(bytes);
                    c_state(&cbuf)
                });
                r.step(|| {
                    rbuf = Buf::from_bytes(bytes).unwrap();
                    r_state(rbuf.as_ref())
                });

                // A copy, then the original edited in place: the copy is unaffected.
                let mut ccopy = ffi::git_buf {
                    ptr: ptr::null_mut(),
                    reserved: 0,
                    size: 0,
                };
                let mut rcopy = Buf::new();
                c.step(|| {
                    ccopy = c_from_bytes(&c_state(&cbuf).bytes);
                    c_state(&ccopy)
                });
                r.step(|| {
                    rcopy = rbuf.as_ref().to_buf().unwrap();
                    r_state(rcopy.as_ref())
                });
                let shorter = &bytes[..bytes.len() / 2];
                c.step(|| {
                    c_set_in_place(&mut cbuf, shorter);
                    c_state(&cbuf)
                });
                r.step(|| {
                    rbuf.as_mut().set_bytes(shorter).unwrap();
                    r_state(rbuf.as_ref())
                });
                c.step(|| c_state(&ccopy));
                r.step(|| r_state(rcopy.as_ref()));

                // libgit2 reuses either side's allocation alike.
                c.step(|| {
                    c_get(ffi::GIT_OPT_GET_USER_AGENT, &mut cbuf);
                    c_state(&cbuf)
                });
                r.step(|| {
                    r_get(ffi::GIT_OPT_GET_USER_AGENT, &mut rbuf);
                    r_state(rbuf.as_ref())
                });

                // Moved out as a string, leaving the buffer owning nothing, and released.
                let mut cstr = ptr::null_mut();
                let mut rstr = None;
                c.step(|| {
                    cstr = cbuf.ptr;
                    cbuf = ffi::git_buf {
                        ptr: ptr::null_mut(),
                        reserved: 0,
                        size: 0,
                    };
                    c_state(&cbuf)
                });
                r.step(|| {
                    rstr = rbuf.as_mut().take_str();
                    r_state(rbuf.as_ref())
                });
                // SAFETY: `cstr` is the NUL-terminated string the buffer owned.
                let c_str = unsafe { CStr::from_ptr(cstr) };
                assert_eq!(c_str, rstr.as_ref().unwrap().as_c_str());
                c.step(|| {
                    // SAFETY: the routine ignores `backend`; `cstr` is a live libgit2
                    // allocation, released once.
                    unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), cstr.cast()) };
                    c_dispose(&mut ccopy);
                    c_dispose(&mut cbuf);
                    c_state(&cbuf)
                });
                r.step(|| {
                    drop(rstr.take());
                    drop(rcopy);
                    drop(rbuf);
                    State {
                        bytes: Vec::new(),
                        c_str: Vec::new(),
                        capacity: 0,
                    }
                });
                assert_eq!(c.steps, r.steps, "contents {bytes:?}");
                assert_eq!(
                    c.steps.last().unwrap().0,
                    [Event::Free { alloc: 0 }, Event::Free { alloc: 1 }],
                    "both allocations freed once"
                );
            }

            // Growing: contents and capacity agree; C reallocates in place where Rust
            // allocates afresh (see the module docs).
            let mut cbuf = c_from_bytes(b"ab");
            let mut rbuf = Buf::from_bytes(b"ab").unwrap();
            let grown = b"abcdefgh";
            // `git_buf_set` grows to exactly `datalen + 1`.
            // SAFETY: `cbuf.ptr` is a live allocation of this allocator.
            let p = unsafe { traced_realloc(cbuf.ptr.cast(), grown.len() + 1, ptr::null(), 0) };
            cbuf = ffi::git_buf {
                ptr: p.cast(),
                reserved: grown.len() + 1,
                size: cbuf.size,
            };
            c_set_in_place(&mut cbuf, grown);
            rbuf.as_mut().set_bytes(grown).unwrap();
            assert_eq!(c_state(&cbuf), r_state(rbuf.as_ref()));
            c_dispose(&mut cbuf);
            drop(rbuf);

            set_opt(ffi::GIT_OPT_SET_USER_AGENT, ptr::null());
        },
    );
}

/// Strings moved out of libgit2's results, as an owned string or owned bytes, release
/// as `git_buf_dispose` would; from the static empty string nothing is taken or freed.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
#[expect(
    clippy::too_many_lines,
    reason = "one scenario, its steps run and compared in order"
)]
fn contents_taken_from_libgit2_results() {
    isolated(
        concat!(module_path!(), "::contents_taken_from_libgit2_results"),
        || {
            install_tracing_allocator();
            set_opt(ffi::GIT_OPT_SET_USER_AGENT, c"crustify/1.0".as_ptr());
            set_opt(ffi::GIT_OPT_SET_TEMPLATE_PATH, c"".as_ptr());
            for opt in [ffi::GIT_OPT_GET_USER_AGENT, ffi::GIT_OPT_GET_TEMPLATE_PATH] {
                for as_bytes in [false, true] {
                    let (mut c, mut r) = (Recorder::default(), Recorder::default());
                    let mut cbuf = ffi::git_buf {
                        ptr: ptr::null_mut(),
                        reserved: 0,
                        size: 0,
                    };
                    let mut rbuf = Buf::new();
                    c.step(|| {
                        c_get(opt, &mut cbuf);
                        c_state(&cbuf)
                    });
                    r.step(|| {
                        r_get(opt, &mut rbuf);
                        r_state(rbuf.as_ref())
                    });

                    // C reads the contents, then disposes the buffer; Rust takes them,
                    // reads them, and releases them.
                    let mut taken = State {
                        bytes: Vec::new(),
                        c_str: Vec::new(),
                        capacity: 0,
                    };
                    c.step(|| {
                        let read = c_state(&cbuf);
                        c_dispose(&mut cbuf);
                        State {
                            capacity: cbuf.reserved,
                            ..read
                        }
                    });
                    r.step(|| {
                        let read = if as_bytes {
                            let v = rbuf.as_mut().take_bytes();
                            let c_str = v.as_slice().split(|&b| b == 0).next().unwrap().to_vec();
                            State {
                                bytes: v.as_slice().to_vec(),
                                c_str,
                                capacity: 0,
                            }
                        } else {
                            match rbuf.as_mut().take_str() {
                                Some(s) => State {
                                    bytes: s.as_bytes().to_vec(),
                                    c_str: s.as_bytes().to_vec(),
                                    capacity: 0,
                                },
                                None => State {
                                    bytes: Vec::new(),
                                    c_str: Vec::new(),
                                    capacity: 0,
                                },
                            }
                        };
                        taken = r_state(rbuf.as_ref());
                        read
                    });
                    assert_eq!(taken.capacity, 0, "the buffer is left owning nothing");
                    // Both buffers own nothing now: disposing frees nothing more.
                    c.step(|| {
                        c_dispose(&mut cbuf);
                        c_state(&cbuf)
                    });
                    r.step(|| {
                        drop(std::mem::replace(&mut rbuf, Buf::new()));
                        r_state(rbuf.as_ref())
                    });
                    assert_eq!(c.steps, r.steps, "option {opt}, bytes {as_bytes}");
                    if opt == ffi::GIT_OPT_GET_USER_AGENT {
                        assert_eq!(c.steps[1].0, [Event::Free { alloc: 0 }]);
                        assert_eq!(c.steps[1].1.bytes, b"crustify/1.0");
                    } else {
                        assert!(
                            c.steps.iter().all(|(e, _)| e.is_empty()),
                            "nothing allocated"
                        );
                    }
                }
            }

            // A string handed to a buffer: `git_buf_dispose` releases it as `set_str` does.
            let (mut c, mut r) = (Recorder::default(), Recorder::default());
            let mut cbuf = ffi::git_buf {
                ptr: ptr::null_mut(),
                reserved: 0,
                size: 0,
            };
            let mut rbuf = Buf::new();
            c.step(|| {
                cbuf = c_from_bytes(b"origin");
                c_state(&cbuf)
            });
            r.step(|| {
                rbuf.as_mut()
                    .set_str(OdbDataStrFree::dup(c"origin").unwrap());
                r_state(rbuf.as_ref())
            });
            c.step(|| {
                c_dispose(&mut cbuf);
                State {
                    capacity: 0,
                    ..c_state(&cbuf)
                }
            });
            r.step(|| {
                drop(std::mem::replace(&mut rbuf, Buf::new()));
                r_state(rbuf.as_ref())
            });
            assert_eq!(c.steps, r.steps);
            assert_eq!(c.steps[1].0, [Event::Free { alloc: 0 }]);

            set_opt(ffi::GIT_OPT_SET_TEMPLATE_PATH, ptr::null());
            set_opt(ffi::GIT_OPT_SET_USER_AGENT, ptr::null());
        },
    );
}
