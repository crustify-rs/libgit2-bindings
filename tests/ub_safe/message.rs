//! UB tests for `git2/message.h`'s [`MessageTrailer`], [`MessageTrailerArray`] and
//! their handles.
//!
//! A trailer is a pair of strings borrowed for `'t`: it owns and frees nothing, so the
//! obligations are Rust-side. The getters must hand back exactly the strings stored,
//! for as long as `'t` (ASan flags a read of a dropped string, Miri a dangling or
//! misaligned read, BorrowSanitizer a read through a pointer a reborrow invalidated);
//! the setters replace one field each; copies are independent; and the values cross
//! threads and are read from several at once (TSan). Nothing here calls libgit2, so it
//! all runs under Miri. Illegal paths (a trailer outliving its strings, an exclusive
//! handle shrunk to a shorter-lived string) are in `compile_fail/`.
//!
//! An array owns the block `git_message_trailers` copies the message's trailers into
//! and the trailer buffer pointing into it, both freed by
//! `git_message_trailer_array_free`: every array is freed once, by whichever owner holds
//! its buffers last, whether parsed, taken out, moved in or cleared (ASan: double free,
//! use after free; LSan: leaks), and the empty array never reaches C (Miri runs those
//! paths). Every trailer and string read through an array lives as long as its shared
//! borrow; libgit2 writes the array into Rust's storage and Rust then reads it back
//! through fresh shared reborrows (BorrowSanitizer). Arrays cross threads and are read
//! from several at once (TSan). Outliving the array, or holding its trailers across a
//! write, does not compile (`compile_fail/message_trailer_array_*` and the doctests on
//! `MessageTrailerArray`).

use std::ffi::{CStr, CString};
use std::thread;

use libgit2::libgit2::object::message::{
    MessageTrailer, MessageTrailerArray, MessageTrailerArrayMut, MessageTrailerArrayRef,
    MessageTrailerArrayVal, MessageTrailerMut,
};

/// Stores `value` through an exclusive handle of a trailer whose strings live for `'t`.
fn store<'t>(mut m: MessageTrailerMut<'_, 't>, value: &'t CStr) {
    m.set_value(value);
}

#[test]
fn getters_return_the_stored_strings() {
    let owned: Vec<CString> = (0..32)
        .map(|i| CString::new(format!("Key-{i}")).unwrap())
        .collect();
    let values: Vec<CString> = (0..32)
        .map(|i| CString::new("v".repeat(i)).unwrap())
        .collect();
    let trailers: Vec<MessageTrailer<'_>> = owned
        .iter()
        .zip(&values)
        .map(|(k, v)| MessageTrailer::new(k, v))
        .collect();
    for (i, trailer) in trailers.iter().enumerate() {
        let handle = trailer.as_ref();
        assert_eq!(handle.key(), owned[i].as_c_str());
        assert_eq!(handle.value().to_bytes().len(), i);
        assert!(!format!("{trailer:?}").is_empty());
    }
    // A string read through a handle outlives the handle and the trailer, as `'t` says.
    let key = trailers[3].as_ref().key();
    drop(trailers);
    assert_eq!(key, c"Key-3");
}

#[test]
fn setters_replace_one_field_and_copies_are_independent() {
    let first = CString::new("Signed-off-by").unwrap();
    let second = CString::new("Acked-by").unwrap();
    let mut trailer = MessageTrailer::new(&first, c"");
    let copy = trailer;
    {
        let mut m = trailer.as_mut();
        m.set_key(&second);
        let mut again = m.as_mut();
        again.set_value(&first);
        assert_eq!(again.as_ref().key(), second.as_c_str());
        store(m.as_mut(), &second);
        assert_eq!(m.as_ref().value(), second.as_c_str());
    }
    assert_eq!(trailer.as_ref().key(), c"Acked-by");
    assert_eq!(trailer.as_ref().value(), c"Acked-by");
    assert_eq!(copy.as_ref().key(), c"Signed-off-by");
    assert_eq!(copy.as_ref().value(), c"");
    // A shared handle taken after the writes sees them; one copied out of it stays
    // valid while the trailer is not written.
    let shared = trailer.as_ref();
    let shared_copy = shared;
    assert_eq!(shared.key(), shared_copy.key());
}

/// A `'static` trailer stores only `'static` strings; a shorter-lived one any string
/// that outlives it.
#[test]
fn lifetimes_follow_the_strings() {
    let mut forever: MessageTrailer<'static> = MessageTrailer::new(c"Fixes", c"#1");
    forever.as_mut().set_value(c"#2");
    let local = CString::new("#3").unwrap();
    let mut scoped = forever;
    scoped.as_mut().set_value(&local);
    assert_eq!(scoped.as_ref().value(), c"#3");
    assert_eq!(forever.as_ref().value(), c"#2");
}

#[test]
fn trailers_cross_threads() {
    let key = CString::new("Reviewed-by").unwrap();
    let mut trailers: Vec<MessageTrailer<'_>> = (0..8)
        .map(|_| MessageTrailer::new(&key, c"A U Thor"))
        .collect();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for trailer in &trailers {
                    assert_eq!(trailer.as_ref().key(), c"Reviewed-by");
                    assert_eq!(trailer.as_ref().value(), c"A U Thor");
                }
            });
        }
    });
    thread::scope(|s| {
        for trailer in &mut trailers {
            let m = trailer.as_mut();
            s.spawn(move || store(m, c"C O Mitter"));
        }
    });
    let moved = thread::scope(|s| s.spawn(move || trailers).join().unwrap());
    assert!(moved.iter().all(|t| t.as_ref().value() == c"C O Mitter"));
}

/// Messages whose trailers libgit2 parses, with the (key, value) pairs it finds: none,
/// one, several with continuation lines, one ending the buffer, a patch after them,
/// whitespace around the separator, and an empty message.
const PARSED: [(&CStr, &[(&CStr, &CStr)]); 8] = [
    (c"", &[]),
    (c"Subject only\n", &[]),
    (c"Subject\n\nBody without trailers.\n", &[]),
    (
        c"Subject\n\nSigned-off-by: A U Thor <a@example.com>\n",
        &[(c"Signed-off-by", c"A U Thor <a@example.com>")],
    ),
    (
        c"Subject\n\nBody.\n\nKey: value\nOther-Key : spaced\nCont: first\n  second\n",
        &[
            (c"Key", c"value"),
            (c"Other-Key", c"spaced"),
            (c"Cont", c"first\n  second"),
        ],
    ),
    (
        c"Subject\n\nBody.\n\nLast: no newline at the end",
        &[(c"Last", c"no newline at the end")],
    ),
    (
        c"Subject\n\nBody.\n\nSigned-off-by: A\n---\ndiff --git a/x b/x\n",
        &[(c"Signed-off-by", c"A")],
    ),
    (
        c"Subject\n\nBody.\n\nTabbed:\tvalue\nKey:value\n",
        &[(c"Tabbed", c"value"), (c"Key", c"value")],
    ),
];

fn parse(message: &CStr) -> MessageTrailerArrayVal {
    MessageTrailerArray::parse(message).unwrap()
}

/// The (key, value) pairs of an array, read through its shared handle.
fn pairs<'a>(array: MessageTrailerArrayRef<'a>) -> Vec<(&'a CStr, &'a CStr)> {
    array
        .trailers()
        .iter()
        .map(|t| (t.key(), t.value()))
        .collect()
}

/// Every trailer of every message reads back exactly what libgit2 parsed, through
/// handles and strings that stay valid for the array's shared borrow, including after
/// the owner moves (the buffers do not) and after other arrays are freed.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parsed_arrays_read_back() {
    let arrays: Vec<MessageTrailerArrayVal> = PARSED.iter().map(|(m, _)| parse(m)).collect();
    let mut strings = Vec::new();
    for (array, (message, expected)) in arrays.iter().zip(PARSED) {
        let handle = array.as_ref();
        assert_eq!(handle.len(), expected.len(), "{message:?}");
        assert_eq!(handle.is_empty(), expected.is_empty());
        assert_eq!(pairs(handle), expected.to_vec(), "{message:?}");
        let trailers = handle.trailers();
        assert!(trailers.get(trailers.len()).is_none());
        if let Some(last) = trailers.get(trailers.len().wrapping_sub(1)) {
            // The last string of the block: a read past its NUL leaves the allocation.
            strings.push(last.value());
        }
        assert!(!format!("{handle:?}").is_empty());
    }
    // Strings read earlier are still the array's while it is borrowed.
    assert_eq!(strings[0], c"A U Thor <a@example.com>");
    let moved: Vec<MessageTrailerArrayVal> = arrays.into_iter().rev().collect();
    assert_eq!(pairs(moved[4].as_ref()), PARSED[3].1.to_vec());
}

/// Messages whose trailer block ends inside each state of libgit2's parser (a key, the
/// whitespace after one, after a separator, a skipped line), where the last string is
/// the block's last bytes: every trailer reads within the block (ASan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn blocks_ending_mid_trailer_read_back() {
    let messages = [
        c"Subject\n\nSigned-off-by: A\nfoo",
        c"Subject\n\nSigned-off-by: A\nfoo ",
        c"Subject\n\nSigned-off-by: A\nfoo bar",
        c"Subject\n\nKey\t : v\nSigned-off-by: A\n",
        c"Subject\n\nSigned-off-by: A\nEmpty:",
        c"Subject\n\nSigned-off-by: A\nEmpty:  ",
    ];
    for message in messages {
        let array = parse(message);
        let read = pairs(array.as_ref());
        assert_eq!(read.len(), array.as_ref().len());
        assert!(
            read.iter()
                .any(|&(k, v)| (k, v) == (c"Signed-off-by", c"A"))
        );
        assert!(read.iter().all(|(k, _)| !k.is_empty()));
    }
}

/// The empty array, and every move of it, without C: Miri runs this.
#[test]
fn empty_arrays_move_without_c() {
    let mut array = MessageTrailerArray::new();
    assert!(array.as_ref().is_empty());
    assert_eq!(array.as_ref().trailers().iter().count(), 0);
    let mut taken = array.as_mut().take();
    taken.as_mut().clear();
    array.as_mut().set(taken);
    let mut m = array.as_mut();
    let again = m.as_mut().take();
    m.set(again);
    assert_eq!(format!("{:?}", array.as_ref()), "[]");
    assert_eq!(format!("{:?}", array.as_mut()), "[]");
    let bare = array.into_inner();
    assert!(bare.as_ref().is_empty());
}

/// Taking, replacing and clearing move the block and the trailers together: each is
/// freed once, by whichever owner holds it last, whatever the order the owners go in;
/// a replaced or cleared array then reads its new contents (or none).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn moved_buffers_are_freed_once() {
    let mut first = parse(PARSED[4].0);
    let taken = first.as_mut().take();
    assert!(first.as_ref().is_empty());
    assert_eq!(pairs(taken.as_ref()), PARSED[4].1.to_vec());
    first.as_mut().set(parse(PARSED[3].0));
    assert_eq!(pairs(first.as_ref()), PARSED[3].1.to_vec());
    // Replacing a filled array frees what it held.
    first.as_mut().set(taken);
    assert_eq!(pairs(first.as_ref()), PARSED[4].1.to_vec());
    let mut second = parse(PARSED[5].0);
    {
        let mut m: MessageTrailerArrayMut<'_> = second.as_mut();
        let inner = m.as_mut().take();
        m.set(inner);
        m.as_mut().clear();
        assert!(m.as_ref().is_empty());
        m.set(first.as_mut().take());
    }
    drop(first);
    assert_eq!(pairs(second.as_ref()), PARSED[4].1.to_vec());
    // An array whose message has no trailer still owns its block.
    let mut blank = parse(PARSED[2].0);
    let block_only = blank.as_mut().take();
    drop(blank);
    drop(block_only);
    drop(second);
}

/// Arrays cross threads: one is read from several at once, its handles are sent,
/// an exclusive handle is sent to empty and refill it, and owners parsed on one thread
/// are freed on another (TSan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_cross_threads() {
    let mut array = parse(PARSED[4].0);
    let handle = array.as_ref();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(move || {
                for _ in 0..16 {
                    assert_eq!(pairs(handle), PARSED[4].1.to_vec());
                }
            });
        }
    });
    let shared = &array;
    thread::scope(|s| {
        s.spawn(move || assert_eq!(shared.as_ref().len(), 3));
        s.spawn(move || assert_eq!(shared.as_ref().len(), 3));
    });
    thread::scope(|s| {
        let mut m = array.as_mut();
        s.spawn(move || {
            m.clear();
            m.set(parse(PARSED[3].0));
        });
    });
    assert_eq!(pairs(array.as_ref()), PARSED[3].1.to_vec());
    let owners: Vec<MessageTrailerArrayVal> = thread::scope(|s| {
        let workers: Vec<_> = PARSED
            .iter()
            .map(|(m, _)| s.spawn(move || parse(m)))
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    thread::spawn(move || drop((array, owners))).join().unwrap();
}

/// UB tests for `prettify` (`git_message_prettify`): libgit2 reads the message for the
/// call only and fills a fresh buffer it owns, released once (LSan, ASan). Messages
/// sit in exact-length heap strings, overwritten and freed after the call: blank,
/// whitespace-only, comment-only, unterminated last lines, long lines, and every
/// comment byte, those above `0x7f` (negative as a C `char`) included. Runs on many
/// threads at once (TSan).
mod prettify {
    use std::ffi::{CStr, CString};
    use std::thread;

    use libgit2::libgit2::object::message::prettify;

    fn exact(bytes: &[u8]) -> Box<CStr> {
        CString::new(bytes).unwrap().into_boxed_c_str()
    }

    fn messages() -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = [
            &b""[..],
            b"\n",
            b" \t\r\n\x0b\x0c",
            b"subject",
            b"subject\n\n\n\nbody   \n\n",
            b"# only a comment",
            b"#\n#\n#",
            b"\xff\xfe line\n\x80 other",
            b"a\n \n\t\nb\n",
        ]
        .iter()
        .map(|m| m.to_vec())
        .collect();
        out.push([b'x'; 9000].iter().chain(b"   \n").copied().collect());
        out
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[cfg_attr(
        bsan,
        ignore = "its large inputs outlast the BorrowSanitizer gate, over an hour alone; the other variants run it"
    )]
    fn messages_are_read_during_the_call_only() {
        let comments = if cfg!(bsan) {
            vec![None, Some(b'#'), Some(0xFF)]
        } else {
            std::iter::once(None)
                .chain((0..=u8::MAX).map(Some))
                .collect()
        };
        for message in messages() {
            for &comment in &comments {
                let mut text = exact(&message);
                let out = prettify(&text, comment).unwrap();
                text = exact(b"");
                drop(text);
                let bytes = out.as_ref().as_bytes();
                assert!(bytes.is_empty() || bytes.ends_with(b"\n"));
                assert!(!bytes.windows(3).any(|w| w == b"\n\n\n"));
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn prettify_on_many_threads() {
        thread::scope(|s| {
            for t in 0..4u8 {
                s.spawn(move || {
                    for message in messages() {
                        let out = prettify(&exact(&message), Some(b'#' + t)).unwrap();
                        drop(out);
                    }
                });
            }
        });
    }
}
