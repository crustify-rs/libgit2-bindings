//! Equivalence of `git2/message.h`'s [`MessageTrailer`] with the `git_message_trailer`
//! C code reads.
//!
//! A trailer owns nothing and no routine takes one back, so what C observes of it is
//! its two fields. Two directions are compared:
//!
//! - C parses: for each message of a corpus, `git_message_trailers` fills an array; C reads each
//!   trailer's `key` and `value` (its own `strlen` and bytes), and Rust adopts the same trailer
//!   with [`MessageTrailer::from_raw`] and reads it through its handle. The counts, keys and values
//!   are compared, then the array is freed by C.
//! - Rust builds: [`MessageTrailer::new`] and the setters, step by step, and C reads the struct
//!   [`into_raw`](MessageTrailer::into_raw) gives back after each step: the very pointers stored,
//!   and the strings through them.
//!
//! [`MessageTrailerArray`] against the `git_message_trailer_array` C code fills with
//! `git_message_trailers` and releases with `git_message_trailer_array_free`, each side
//! parsing its own copy of every message:
//!
//! - [`MessageTrailerArray::parse`] against `git_message_trailers` over a corpus: the code, the
//!   count, the keys and values, and the state the struct is left in (a block always, a trailer
//!   buffer only with trailers).
//! - A sequence of parses, replacements, takes and clears against the C code that does the same to
//!   raw structs (free the old one, copy the struct, zero it), compared after every step: each live
//!   array's trailers, and the allocator events the step caused, pointers numbered by allocation,
//!   so that every buffer is freed in the same step on both sides. It runs alone in a child process
//!   that installs a tracing allocator before `git_libgit2_init`.
//! - A failed allocation of the trailer block, in a child process whose allocator fails on demand:
//!   the code (or Rust's error), the error class and message, and an array left as it was (empty).
//!
//! Documented divergence, outside these comparisons: an empty array is released by
//! Rust without calling libgit2, where C code hands its NULL buffers to `git__free`, a
//! no-op the tracing allocator does not record either.

use std::ffi::{CStr, CString, c_char};
use std::mem::MaybeUninit;

use glibc_bindings::libc::ffi as libc;
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::trailer::{MessageTrailer, MessageTrailerArray};

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

/// C's reading of a NUL-terminated string: `strlen`, then the bytes.
fn c_read(s: *const c_char) -> Vec<u8> {
    // SAFETY: the callers pass a live NUL-terminated string.
    let len = unsafe { libc::strlen(s) };
    // SAFETY: `len` bytes precede the NUL.
    unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len) }.to_vec()
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parsed_trailers_match() {
    init();
    let messages = [
        c"Subject\n\nBody.\n",
        c"Subject\n\nSigned-off-by: A U Thor <a@example.com>\n",
        c"Subject\n\nBody.\n\nKey: value\nOther-Key : spaced value\nCont: first\n  second\n",
        c"Subject\n\nBody.\n\nLast: no newline",
        c"Subject\n\nBody.\n\nEmpty:\nAfter: x\nnot a trailer\n",
        c"Subject\n\nBody.\n\nFixes #12\nKey=value\n",
        c"Subject\n\nBody.\n\nSigned-off-by: A\n\n---\ndiff --git a/x b/x\n",
        c"Subject\n\nBody.\n\nTabbed:\tvalue\n# comment\nKey: v\n",
        c"",
    ];
    for message in messages {
        let mut arr = MaybeUninit::<ffi::git_message_trailer_array>::zeroed();
        // SAFETY: writable storage; `message` is NUL-terminated and only read.
        let rc = unsafe { ffi::git_message_trailers(arr.as_mut_ptr(), message.as_ptr()) };
        assert_eq!(rc, 0);
        // SAFETY: a 0 return means every field was written.
        let mut arr = unsafe { arr.assume_init() };
        let (mut c_side, mut rust_side) = (Vec::new(), Vec::new());
        for i in 0..arr.count {
            // SAFETY: `i < count`, so the trailer is in the array's buffer.
            let add = unsafe { arr.trailers.add(i) };
            // SAFETY: as above.
            let raw = unsafe { add.read() };
            c_side.push((c_read(raw.key), c_read(raw.value)));
            // SAFETY: libgit2 set both strings, non-NULL and NUL-terminated, in the
            // array's block, which lives, unwritten, until the array is freed below,
            // after the last read of `trailer`.
            let trailer = unsafe { MessageTrailer::from_raw(raw) };
            let handle = trailer.as_ref();
            rust_side.push((
                handle.key().to_bytes().to_vec(),
                handle.value().to_bytes().to_vec(),
            ));
        }
        assert_eq!(rust_side, c_side, "{message:?}");
        // SAFETY: filled by `git_message_trailers`, freed once.
        unsafe { ffi::git_message_trailer_array_free(&raw mut arr) };
    }
}

/// C's view of a Rust-built trailer: the pointers and the strings behind them.
fn c_view(trailer: MessageTrailer<'_>) -> (*const c_char, *const c_char, Vec<u8>, Vec<u8>) {
    let raw = trailer.into_raw();
    (raw.key, raw.value, c_read(raw.key), c_read(raw.value))
}

#[test]
fn built_trailers_match() {
    let key = CString::new("Signed-off-by").unwrap();
    let value = CString::new("A U Thor <a@example.com>").unwrap();
    let other: &CStr = c"Acked-by";
    let mut trailer = MessageTrailer::new(&key, &value);
    let expect = |k: &CStr, v: &CStr| {
        (
            k.as_ptr(),
            v.as_ptr(),
            k.to_bytes().to_vec(),
            v.to_bytes().to_vec(),
        )
    };
    assert_eq!(c_view(trailer), expect(&key, &value));
    trailer.as_mut().set_key(other);
    assert_eq!(c_view(trailer), expect(other, &value));
    trailer.as_mut().set_value(c"");
    assert_eq!(c_view(trailer), expect(other, c""));
    trailer.as_mut().set_key(&value);
    assert_eq!(c_view(trailer), expect(&value, c""));
}

/// The messages both sides parse; the last ones end the trailer block inside each
/// state of libgit2's parser: in a key, in the whitespace after one, after a
/// separator, and in a line it skips.
const MESSAGES: [&CStr; 15] = [
    c"Subject\n\nBody.\n",
    c"Subject\n\nSigned-off-by: A U Thor <a@example.com>\n",
    c"Subject\n\nBody.\n\nKey: value\nOther-Key : spaced value\nCont: first\n  second\n",
    c"Subject\n\nBody.\n\nLast: no newline",
    c"Subject\n\nBody.\n\nEmpty:\nAfter: x\nnot a trailer\n",
    c"Subject\n\nBody.\n\nFixes #12\nKey=value\n",
    c"Subject\n\nBody.\n\nSigned-off-by: A\n\n---\ndiff --git a/x b/x\n",
    c"Subject\n\nBody.\n\nTabbed:\tvalue\n# comment\nKey: v\n",
    c"",
    c"Subject\n\nSigned-off-by: A\nfoo",
    c"Subject\n\nSigned-off-by: A\nfoo ",
    c"Subject\n\nSigned-off-by: A\nfoo bar",
    c"Subject\n\nKey\t : v\nSigned-off-by: A\n",
    c"Subject\n\nSigned-off-by: A\nEmpty:",
    c"Subject\n\nSigned-off-by: A\nEmpty:  ",
];

/// What one side's array holds: its trailers, and whether it holds a block and a
/// trailer buffer.
type ArrayView = (Vec<(Vec<u8>, Vec<u8>)>, bool, bool);

/// C's view of a raw array.
fn c_array(arr: &ffi::git_message_trailer_array) -> ArrayView {
    let pairs = (0..arr.count)
        .map(|i| {
            // SAFETY: `i < count`, within the trailer buffer.
            let add2 = unsafe { arr.trailers.add(i) };
            // SAFETY: as above.
            let t = unsafe { add2.read() };
            (c_read(t.key), c_read(t.value))
        })
        .collect();
    (
        pairs,
        !arr._trailer_block.is_null(),
        !arr.trailers.is_null(),
    )
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parsed_arrays_match() {
    init();
    for message in MESSAGES {
        let mut c = ffi::git_message_trailer_array::default();
        // SAFETY: zeroed storage; `message` is NUL-terminated and only read.
        let rc = unsafe { ffi::git_message_trailers(&raw mut c, message.as_ptr()) };
        let c_view = (rc, c_array(&c));
        // SAFETY: filled by `git_message_trailers`, freed once.
        unsafe { ffi::git_message_trailer_array_free(&raw mut c) };

        let rust = MessageTrailerArray::parse(message).unwrap();
        let trailers: Vec<_> = rust
            .as_ref()
            .trailers()
            .iter()
            .map(|t| (t.key().to_bytes().to_vec(), t.value().to_bytes().to_vec()))
            .collect();
        assert_eq!(trailers.len(), rust.as_ref().len());
        // The state Rust's storage is left in, read as C reads it, then adopted back.
        let raw = rust.into_inner().into_raw();
        let (_, block, buffer) = c_array(&raw);
        // SAFETY: the buffers `into_raw` gave up, adopted once.
        drop(unsafe { MessageTrailerArray::from_raw(raw) });
        assert_eq!((0, (trailers, block, buffer)), c_view, "{message:?}");
    }
}

/// Both sides' arrays under a tracing allocator that can fail on demand, each test in
/// a child process.
mod allocator {
    use std::collections::HashMap;
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::process::Command;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::trailer::{MessageTrailerArray, MessageTrailerArrayVal};
    use libgit2::libgit2::src::util::errors::last;

    use super::{ArrayView, MESSAGES, c_array};

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

    /// Runs `body` alone in a child process, which installs the allocator first.
    fn isolated(test: &str, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            install();
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

    #[derive(Debug)]
    enum Raw {
        Alloc { ptr: usize, len: usize },
        Realloc { from: usize, ptr: usize, len: usize },
        Free { ptr: usize },
    }

    static TRACING: AtomicBool = AtomicBool::new(false);
    static TRACE: Mutex<Vec<Raw>> = Mutex::new(Vec::new());
    /// How many of the next allocations fail.
    static FAIL: AtomicUsize = AtomicUsize::new(0);

    fn trace(event: Raw) {
        if TRACING.load(Ordering::SeqCst) {
            TRACE.lock().unwrap().push(event);
        }
    }

    fn failing() -> bool {
        let mut n = FAIL.load(Ordering::SeqCst);
        while n > 0 {
            match FAIL.compare_exchange(n, n - 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return true,
                Err(now) => n = now,
            }
        }
        false
    }

    unsafe extern "C" fn traced_malloc(len: usize, _: *const c_char, _: c_int) -> *mut c_void {
        if failing() {
            return std::ptr::null_mut();
        }
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
        if failing() {
            return std::ptr::null_mut();
        }
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

    /// Installs the allocator, then initialises libgit2 on top of it and pins the
    /// crate's runtime, so neither allocates during a step.
    fn install() {
        let mut allocator = ffi::git_allocator {
            gmalloc: Some(traced_malloc),
            grealloc: Some(traced_realloc),
            gfree: Some(traced_free),
        };
        // SAFETY: `GIT_OPT_SET_ALLOCATOR` takes a `git_allocator *` with every member
        // set, which it copies; nothing is allocated yet.
        let rc = unsafe {
            ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(), &raw mut allocator)
        };
        assert_eq!(rc, 0);
        // SAFETY: no arguments; refcounted, never released in this process.
        assert!(unsafe { ffi::git_libgit2_init() } > 0);
        drop(MessageTrailerArray::parse(c"").unwrap());
        // The thread's error state, which a failure writes to.
        drop(last());
    }

    /// One allocator event, with pointers replaced by the number of their allocation.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Event {
        Alloc { len: usize },
        Realloc { from: Option<usize>, len: usize },
        Free { alloc: Option<usize> },
    }

    /// Records one side's steps: the live arrays after each, and its events.
    #[derive(Default)]
    struct Recorder {
        steps: Vec<(Vec<ArrayView>, Vec<Event>)>,
        allocs: HashMap<usize, usize>,
        count: usize,
    }

    impl Recorder {
        fn number(&mut self, ptr: usize) {
            self.allocs.insert(ptr, self.count);
            self.count += 1;
        }

        fn step(&mut self, f: impl FnOnce() -> Vec<ArrayView>) {
            TRACE.lock().unwrap().clear();
            TRACING.store(true, Ordering::SeqCst);
            let views = f();
            TRACING.store(false, Ordering::SeqCst);
            let raw = std::mem::take(&mut *TRACE.lock().unwrap());
            let events = raw
                .into_iter()
                .map(|e| match e {
                    Raw::Alloc { ptr, len } => {
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
            self.steps.push((views, events));
        }
    }

    fn c_parse(message: &CStr) -> ffi::git_message_trailer_array {
        let mut arr = ffi::git_message_trailer_array::default();
        // SAFETY: zeroed storage; `message` is NUL-terminated and only read.
        let rc = unsafe { ffi::git_message_trailers(&raw mut arr, message.as_ptr()) };
        assert_eq!(rc, 0);
        arr
    }

    /// C code releasing an array and zeroing it, as a holder reusing the storage does.
    fn c_free(arr: &mut ffi::git_message_trailer_array) {
        // SAFETY: an array `git_message_trailers` filled, or the zeroed one; freed once,
        // then zeroed before anything reads it.
        unsafe { ffi::git_message_trailer_array_free(arr) };
        *arr = ffi::git_message_trailer_array::default();
    }

    fn rust_view(array: &MessageTrailerArrayVal) -> ArrayView {
        // Rust's storage, read as C reads it: `MessageTrailerArray` is transparent over
        // the C struct, which `CVal` holds first.
        let raw = std::ptr::from_ref(array).cast::<ffi::git_message_trailer_array>();
        // SAFETY: a live array, copied out (plain data) without writing.
        c_array(&unsafe { raw.read() })
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn sequences_match() {
        isolated(concat!(module_path!(), "::sequences_match"), || {
            let (m1, m2, m3) = (MESSAGES[2], MESSAGES[1], MESSAGES[0]);
            let mut c = Recorder::default();
            let (mut a, mut b, mut t) = Default::default();
            c.step(|| {
                a = c_parse(m1);
                vec![c_array(&a)]
            });
            c.step(|| {
                b = c_parse(m2);
                vec![c_array(&a), c_array(&b)]
            });
            c.step(|| {
                c_free(&mut a);
                a = std::mem::take(&mut b);
                vec![c_array(&a)]
            });
            c.step(|| {
                t = std::mem::take(&mut a);
                vec![c_array(&a), c_array(&t)]
            });
            c.step(|| {
                c_free(&mut a);
                a = c_parse(m3);
                vec![c_array(&a), c_array(&t)]
            });
            c.step(|| {
                c_free(&mut t);
                vec![c_array(&a), c_array(&t)]
            });
            c.step(|| {
                c_free(&mut a);
                c_free(&mut t);
                vec![]
            });

            let mut rust = Recorder::default();
            let mut a = MessageTrailerArray::new();
            let mut t = MessageTrailerArray::new();
            let mut b = None;
            rust.step(|| {
                a = MessageTrailerArray::parse(m1).unwrap();
                vec![rust_view(&a)]
            });
            rust.step(|| {
                let parsed = b.insert(MessageTrailerArray::parse(m2).unwrap());
                vec![rust_view(&a), rust_view(parsed)]
            });
            rust.step(|| {
                a.as_mut().set(b.take().unwrap());
                vec![rust_view(&a)]
            });
            rust.step(|| {
                t = a.as_mut().take();
                vec![rust_view(&a), rust_view(&t)]
            });
            rust.step(|| {
                a.as_mut().set(MessageTrailerArray::parse(m3).unwrap());
                vec![rust_view(&a), rust_view(&t)]
            });
            rust.step(|| {
                t.as_mut().clear();
                vec![rust_view(&a), rust_view(&t)]
            });
            rust.step(|| {
                drop((a, t));
                vec![]
            });

            assert_eq!(c.steps.len(), rust.steps.len());
            for (i, (c, rust)) in c.steps.iter().zip(&rust.steps).enumerate() {
                assert_eq!(c, rust, "step {i}");
            }
            // Every allocation was freed, on both sides.
            assert!(c.allocs.is_empty() && rust.allocs.is_empty());
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failed_allocations_match() {
        isolated(
            concat!(module_path!(), "::failed_allocations_match"),
            || {
                let message = MESSAGES[1];
                let mut c = ffi::git_message_trailer_array::default();
                FAIL.store(1, Ordering::SeqCst);
                // SAFETY: zeroed storage; `message` is NUL-terminated and only read.
                let rc = unsafe { ffi::git_message_trailers(&raw mut c, message.as_ptr()) };
                let c_error = last();
                assert_eq!(FAIL.load(Ordering::SeqCst), 0);
                let c_seen = (
                    rc,
                    c_error.raw_class(),
                    c_error.message().to_owned(),
                    c_array(&c),
                );

                FAIL.store(1, Ordering::SeqCst);
                let e = MessageTrailerArray::parse(message).unwrap_err();
                assert_eq!(FAIL.load(Ordering::SeqCst), 0);
                let empty = MessageTrailerArray::new();
                let rust_seen = (-1, e.raw_class(), e.message().to_owned(), rust_view(&empty));
                assert_eq!(rust_seen, c_seen);
                assert_eq!(c_seen.3, (vec![], false, false));

                // Both parse again once the allocator recovers.
                let parsed = MessageTrailerArray::parse(message).unwrap();
                assert_eq!(parsed.as_ref().len(), 1);
            },
        );
    }
}

/// Equivalence of `prettify` with `git_message_prettify`.
///
/// Each side cleans up its own copy of every message of a corpus (blank, whitespace-only
/// and comment-only messages, runs of blank lines, trailing whitespace of every kind,
/// unterminated last lines, comment characters mid-line and after whitespace, bytes past
/// `0x7f`), with comments kept and with every comment byte, those past `0x7f` included
/// (a negative C `char`); the results are compared byte for byte. A buffer is reused on
/// the C side across messages, as `git_buf` callers do.
mod prettify {
    use std::ffi::CString;

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::message::prettify;

    use super::init;

    fn corpus() -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = [
            &b""[..],
            b"\n",
            b"\n\n\n",
            b" \t\x0b\x0c\r\n",
            b"subject",
            b"subject\n",
            b"  subject  \n\n\n\n  body\t\r\n\n",
            b"# comment\nsubject\n# another\n\nbody\n#",
            b" # indented comment\n;semi\n;\n",
            b"a\n\n# c\n\nb",
            b"\xff\xfe\x80\n\n\x80 not ascii\n",
            b"line one\r\nline two\r\n\r\n\r\n",
        ]
        .iter()
        .map(|m| m.to_vec())
        .collect();
        let mut long = vec![b'x'; 5000];
        long.extend_from_slice(b"  \n\n#c\n");
        long.extend(vec![b'y'; 5000]);
        out.push(long);
        out
    }

    fn c_prettify(buf: &mut ffi::git_buf, message: &[u8], comment: Option<u8>) -> Vec<u8> {
        let message = CString::new(message).unwrap();
        let (strip, c) = comment.map_or((0, 0), |c| (1, c.cast_signed()));
        // SAFETY: a buffer libgit2 filled before or zeroed; a NUL-terminated message.
        let rc = unsafe { ffi::git_message_prettify(buf, message.as_ptr(), strip, c) };
        assert_eq!(rc, 0);
        if buf.ptr.is_null() {
            return Vec::new();
        }
        // SAFETY: libgit2 filled `size` bytes at `ptr`.
        unsafe { std::slice::from_raw_parts(buf.ptr.cast::<u8>(), buf.size) }.to_vec()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn prettified_messages_match_c() {
        init();
        let mut buf = ffi::git_buf::default();
        for message in corpus() {
            for comment in std::iter::once(None).chain((0..=u8::MAX).map(Some)) {
                let c = c_prettify(&mut buf, &message, comment);
                let rust = prettify(&CString::new(message.clone()).unwrap(), comment).unwrap();
                assert_eq!(rust.as_ref().as_bytes(), c, "{message:x?} {comment:?}");
            }
        }
        // SAFETY: the buffer libgit2 filled, disposed once.
        unsafe { ffi::git_buf_dispose(&raw mut buf) };
    }
}
