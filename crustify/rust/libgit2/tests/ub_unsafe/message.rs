//! UB tests for the unsafe public API of `git2/message.h`: [`MessageTrailer::from_raw`].
//!
//! `from_raw` adopts a `git_message_trailer` whose two strings the caller vouches for:
//! non-NULL, NUL-terminated, live and unchanged for `'t`. Each test honours that
//! contract and checks that the adopted trailer then reads exactly those strings, and
//! nothing past them (ASan, Miri), up to the end of `'t`:
//!
//! - trailers the safe API built and gave back (`MessageTrailer::new` → `into_raw`),
//!   including empty and long strings, which run under Miri;
//! - every trailer `git_message_trailers` parses out of a message, each adopted for a
//!   `'t` that ends before `git_message_trailer_array_free` frees the strings, across
//!   messages with no trailers, continuation lines, and one trailer ending the buffer
//!   (the last string of the block, where a read one byte too far leaves the
//!   allocation). The array has no safe wrapper yet, so it is driven raw here.
//!
//! Adopted trailers are then copied, written through their exclusive handle and read
//! from several threads (TSan), as any trailer may be.

use std::ffi::{CStr, CString, c_char};
use std::mem::MaybeUninit;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::object::message::MessageTrailer;

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

/// `git_message_trailers` over `message`; the caller frees the array.
fn parse(message: &CStr) -> ffi::git_message_trailer_array {
    let mut arr = MaybeUninit::<ffi::git_message_trailer_array>::zeroed();
    // SAFETY: `arr` is writable storage, which libgit2 fills on success; `message` is
    // NUL-terminated and only read.
    let rc = unsafe { ffi::git_message_trailers(arr.as_mut_ptr(), message.as_ptr()) };
    assert_eq!(rc, 0);
    // SAFETY: a 0 return means every field was written.
    unsafe { arr.assume_init() }
}

/// The C side's own reading of a string, copied out.
fn c_string(s: *const c_char) -> CString {
    // SAFETY: the callers pass a string `git_message_trailers` set, live until the array
    // is freed, after this copy.
    unsafe { CStr::from_ptr(s) }.to_owned()
}

#[test]
fn trailers_given_back_by_the_safe_api_are_adopted() {
    let long = CString::new("x".repeat(4096)).unwrap();
    let cases: [(&CStr, &CStr); 4] = [
        (c"Signed-off-by", c"A U Thor <a@example.com>"),
        (c"", c""),
        (c"K", &long),
        (&long, c"v\n  continued"),
    ];
    for (key, value) in cases {
        let raw = MessageTrailer::new(key, value).into_raw();
        // SAFETY: `raw` holds `key`'s and `value`'s pointers, non-NULL NUL-terminated
        // strings that outlive this loop iteration, which bounds `'t`, and that nothing
        // writes.
        let mut adopted = unsafe { MessageTrailer::from_raw(raw) };
        assert_eq!(adopted.as_ref().key(), key);
        assert_eq!(adopted.as_ref().value(), value);
        let copy = adopted;
        adopted.as_mut().set_key(c"Replaced");
        assert_eq!(copy.as_ref().key(), key);
        assert_eq!(adopted.as_ref().value(), value);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parsed_trailers_are_adopted_until_the_array_is_freed() {
    init();
    let messages = [
        c"Subject\n\nBody without trailers.\n",
        c"Subject\n\nSigned-off-by: A U Thor <a@example.com>\n",
        c"Subject\n\nBody.\n\nKey: value\nOther-Key : spaced value\nCont: first\n  second\n   third\n",
        c"Subject\n\nBody.\n\nLast: no newline at the end",
        c"Subject\n\nBody.\n\nEmpty:\nAfter: x\n",
        c"",
    ];
    for message in messages {
        let mut arr = parse(message);
        let mut adopted = Vec::new();
        for i in 0..arr.count {
            // SAFETY: `i < count`, so the trailer is in the array's buffer.
            let raw = unsafe { arr.trailers.add(i).read() };
            let expected = (c_string(raw.key), c_string(raw.value));
            // SAFETY: libgit2 set both strings, non-NULL and NUL-terminated, in the
            // array's block, which nothing writes and which lives until
            // `git_message_trailer_array_free` below: `adopted` is cleared before that,
            // which ends `'t`.
            let trailer = unsafe { MessageTrailer::from_raw(raw) };
            assert_eq!(trailer.as_ref().key(), expected.0.as_c_str());
            assert_eq!(trailer.as_ref().value(), expected.1.as_c_str());
            adopted.push((trailer, expected));
        }
        thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| {
                    for (trailer, (key, value)) in &adopted {
                        assert_eq!(trailer.as_ref().key(), key.as_c_str());
                        assert_eq!(trailer.as_ref().value(), value.as_c_str());
                    }
                });
            }
        });
        adopted.clear();
        // SAFETY: `arr` was filled by `git_message_trailers`, nothing borrows its strings
        // any more, and it is freed once.
        unsafe { ffi::git_message_trailer_array_free(&mut arr) };
    }
}
