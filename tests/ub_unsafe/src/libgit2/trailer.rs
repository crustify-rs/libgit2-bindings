//! UB tests for the unsafe public API of `git2/message.h`: [`MessageTrailer::from_raw`]
//! and [`MessageTrailerArray::from_raw`].
//!
//! `MessageTrailer::from_raw` adopts a `git_message_trailer` whose two strings the
//! caller vouches for: non-NULL, NUL-terminated, live and unchanged for `'t`. Each test
//! honours that contract and checks that the adopted trailer then reads exactly those
//! strings, and nothing past them (`ASan`, Miri), up to the end of `'t`:
//!
//! - trailers the safe API built and gave back (`MessageTrailer::new` → `into_raw`), including
//!   empty and long strings, which run under Miri;
//! - every trailer of an array [`MessageTrailerArray::parse`] fills, each adopted for a `'t` that
//!   ends before the array is dropped, which frees the strings, across messages with no trailers,
//!   continuation lines, and one trailer ending the buffer (the last string of the block, where a
//!   read one byte too far leaves the allocation).
//!
//! Adopted trailers are then copied, written through their exclusive handle and read
//! from several threads (`TSan`), as any trailer may be.
//!
//! `MessageTrailerArray::from_raw` adopts a `git_message_trailer_array` whose buffers
//! the caller gives up: NULL or libgit2's allocations, with at least `count` trailers
//! whose strings live while the array owns them. The tests adopt arrays libgit2 filled
//! (`git_message_trailers`, the C code the routine is for), arrays the safe API gave
//! back (`into_inner` → `into_raw`), the empty array, and the edge states C leaves: a
//! block without a trailer buffer (a message without trailers), and a NULL trailer
//! buffer with a stale `count`. Each adopted array reads its trailers back, is taken
//! from, refilled and cleared, crosses threads, and is freed exactly once by its last
//! owner (`ASan`: double free, use after free; `LSan`: leaks). The NULL states run under
//! Miri.

use std::ffi::{CStr, CString, c_char};
use std::mem::MaybeUninit;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::trailer::{
    MessageTrailer, MessageTrailerArray, MessageTrailerArrayRef, MessageTrailerArrayVal,
};

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

/// `git_message_trailers` over `message`, as C code fills an array; the caller frees
/// it, or adopts it.
fn parse_raw(message: &CStr) -> ffi::git_message_trailer_array {
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

const MESSAGES: [&CStr; 6] = [
    c"Subject\n\nBody without trailers.\n",
    c"Subject\n\nSigned-off-by: A U Thor <a@example.com>\n",
    c"Subject\n\nBody.\n\nKey: value\nOther-Key : spaced value\nCont: first\n  second\n   third\n",
    c"Subject\n\nBody.\n\nLast: no newline at the end",
    c"Subject\n\nBody.\n\nEmpty:\nAfter: x\n",
    c"",
];

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parsed_trailers_are_adopted_until_the_array_is_freed() {
    init();
    for message in MESSAGES {
        let array = MessageTrailerArray::parse(message).unwrap();
        let trailers = array.as_ref().trailers();
        let mut adopted = Vec::new();
        for i in 0..trailers.len() {
            // SAFETY: `i < len`, so the trailer is in the array's buffer, read as a copy.
            let add = unsafe { trailers.as_ptr().add(i) };
            // SAFETY: as above.
            let raw = unsafe { add.read() };
            let expected = (c_string(raw.key), c_string(raw.value));
            // SAFETY: libgit2 set both strings, non-NULL and NUL-terminated, in the
            // array's block, which nothing writes and which lives until `array` drops
            // below: `adopted` is cleared before that, which ends `'t`.
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
        drop(array);
    }
}

/// The (key, value) pairs an array's handle reads, copied out.
fn pairs(array: MessageTrailerArrayRef<'_>) -> Vec<(CString, CString)> {
    array
        .trailers()
        .iter()
        .map(|t| (t.key().to_owned(), t.value().to_owned()))
        .collect()
}

/// What C code reads of a raw array, copied out.
fn c_pairs(raw: &ffi::git_message_trailer_array) -> Vec<(CString, CString)> {
    (0..raw.count)
        .map(|i| {
            // SAFETY: `i < count`, within the trailer buffer libgit2 filled.
            let add2 = unsafe { raw.trailers.add(i) };
            // SAFETY: as above.
            let t = unsafe { add2.read() };
            (c_string(t.key), c_string(t.value))
        })
        .collect()
}

/// Arrays C code filled are adopted whole, read exactly what C reads, and are freed
/// once by whichever owner holds their buffers last, after being taken from, moved
/// into another array, refilled and cleared, and sent across threads.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_c_filled_are_adopted_and_freed_once() {
    init();
    let mut owners: Vec<MessageTrailerArrayVal> = Vec::new();
    for message in MESSAGES {
        let raw = parse_raw(message);
        let expected = c_pairs(&raw);
        // SAFETY: `raw` was filled by `git_message_trailers`: its block and trailer
        // buffer (NULL without a trailer) are libgit2's allocations, given up here, and
        // its `count` trailers point into the block, which the result now owns.
        let mut array = unsafe { MessageTrailerArray::from_raw(raw) };
        assert_eq!(pairs(array.as_ref()), expected, "{message:?}");
        let taken = array.as_mut().take();
        assert!(array.as_ref().is_empty());
        assert_eq!(pairs(taken.as_ref()), expected);
        array.as_mut().set(taken);
        assert_eq!(pairs(array.as_ref()), expected);
        owners.push(array);
    }
    // Refill one adopted array with another, then clear one: each frees what it held.
    let moved = owners[2].as_mut().take();
    owners[1].as_mut().set(moved);
    owners[3].as_mut().clear();
    let expected: Vec<_> = owners.iter().map(|a| pairs(a.as_ref())).collect();
    let back = thread::spawn(move || {
        let read: Vec<_> = owners.iter().map(|a| pairs(a.as_ref())).collect();
        (owners, read)
    })
    .join()
    .unwrap();
    assert_eq!(back.1, expected);
    thread::spawn(move || drop(back.0)).join().unwrap();
}

/// An array the safe API gave up is adopted back, as C code hands one over: the same
/// buffers, freed once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_given_back_by_the_safe_api_are_adopted() {
    init();
    for message in MESSAGES {
        let parsed = MessageTrailerArray::parse(message).unwrap();
        let expected = pairs(parsed.as_ref());
        let raw = parsed.into_inner().into_raw();
        // SAFETY: the buffers `into_raw` gave up, libgit2's allocations with `count`
        // trailers pointing into the block, adopted once.
        let adopted = unsafe { MessageTrailerArray::from_raw(raw) };
        assert_eq!(pairs(adopted.as_ref()), expected);
        let mut again = adopted.into_inner().into_raw();
        assert_eq!(c_pairs(&again), expected);
        // SAFETY: given up once more, and freed once by C.
        unsafe { ffi::git_message_trailer_array_free(&raw mut again) };
    }
}

/// The NULL states: the empty array, and a NULL trailer buffer with a stale `count`,
/// adopted (no requirement on them), read as empty and released without C: Miri runs
/// this.
#[test]
fn null_arrays_are_adopted_without_c() {
    for count in [0, 1, usize::MAX] {
        let raw = ffi::git_message_trailer_array {
            trailers: std::ptr::null_mut(),
            count,
            _trailer_block: std::ptr::null_mut(),
        };
        // SAFETY: both buffers NULL, which owns nothing.
        let mut array = unsafe { MessageTrailerArray::from_raw(raw) };
        assert_eq!(array.as_ref().len(), 0);
        assert!(array.as_ref().trailers().get(0).is_none());
        let taken = array.as_mut().take();
        array.as_mut().set(taken);
        array.as_mut().clear();
        assert_eq!(format!("{:?}", array.as_ref()), "[]");
    }
}

/// A block with no trailer buffer, as libgit2 leaves a message without trailers, is
/// freed alone.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn block_only_arrays_are_adopted() {
    init();
    let raw = parse_raw(c"Subject\n\nNo trailer here.\n");
    assert!(raw.trailers.is_null() && !raw._trailer_block.is_null());
    // SAFETY: filled by `git_message_trailers`; the block is libgit2's, given up here.
    let array = unsafe { MessageTrailerArray::from_raw(raw) };
    assert!(array.as_ref().is_empty());
    drop(array);
}
