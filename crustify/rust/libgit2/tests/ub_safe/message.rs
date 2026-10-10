//! UB tests for `git2/message.h`'s [`MessageTrailer`] and its handles.
//!
//! A trailer is a pair of strings borrowed for `'t`: it owns and frees nothing, so the
//! obligations are Rust-side. The getters must hand back exactly the strings stored,
//! for as long as `'t` (ASan flags a read of a dropped string, Miri a dangling or
//! misaligned read, BorrowSanitizer a read through a pointer a reborrow invalidated);
//! the setters replace one field each; copies are independent; and the values cross
//! threads and are read from several at once (TSan). Nothing here calls libgit2, so it
//! all runs under Miri. Illegal paths (a trailer outliving its strings, an exclusive
//! handle shrunk to a shorter-lived string) are in `compile_fail/`.

use std::ffi::{CStr, CString};
use std::thread;

use libgit2::libgit2::object::message::{MessageTrailer, MessageTrailerMut};

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
