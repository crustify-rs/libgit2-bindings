//! Equivalence of `git2/message.h`'s [`MessageTrailer`] with the `git_message_trailer`
//! C code reads.
//!
//! A trailer owns nothing and no routine takes one back, so what C observes of it is
//! its two fields. Two directions are compared:
//!
//! - C parses: for each message of a corpus, `git_message_trailers` fills an array; C
//!   reads each trailer's `key` and `value` (its own `strlen` and bytes), and Rust
//!   adopts the same trailer with [`MessageTrailer::from_raw`] and reads it through
//!   its handle. The counts, keys and values are compared, then the array is freed by
//!   C.
//! - Rust builds: [`MessageTrailer::new`] and the setters, step by step, and C reads
//!   the struct [`into_raw`](MessageTrailer::into_raw) gives back after each step:
//!   the very pointers stored, and the strings through them.

use std::ffi::{CStr, CString, c_char};
use std::mem::MaybeUninit;

use libgit2::ffi;
use libgit2::libgit2::object::message::MessageTrailer;

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
            let raw = unsafe { arr.trailers.add(i).read() };
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
        unsafe { ffi::git_message_trailer_array_free(&mut arr) };
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
