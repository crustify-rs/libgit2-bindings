//! Equivalence of `git2/tag.h`'s `name_is_valid` with `git_tag_name_is_valid`.
//!
//! Each probe hands the same name, in two independently owned `CString`s, to C and to
//! Rust, and compares the outcome (C's return code and `valid` against the `Result`)
//! and the calling thread's last error after the call: both leave
//! `GIT_ERROR_REFERENCE`, naming the reference, when `refs/tags/<name>` is what fails,
//! and nothing when the tag rules (a leading `-`, `HEAD`) refuse the name first. The
//! probes cover named edge cases and every name of up to three characters over the
//! characters the rules tell apart. A multi-call sequence alternates valid and invalid
//! names without clearing the error, comparing after each step: a valid name leaves
//! the previous error in place on both sides.
//!
//! Outside these comparisons: C also takes a NULL name, reporting it invalid; the
//! wrapper takes a `&CStr`, which is never NULL. Out-of-memory failures are compared in
//! the wrapper's unit tests, which install a refusing allocator.

use std::ffi::{CStr, CString, c_int};

use libgit2::ffi;
use libgit2::libgit2::object::tag::name_is_valid;

/// Initialises libgit2; never released.
fn init() {
    // SAFETY: no arguments; the reference is never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

/// The calling thread's last error class and message.
fn c_last() -> (c_int, String) {
    // SAFETY: `git_error_last` never returns NULL after init, and its message is a
    // NUL-terminated string, both live until the next libgit2 call on this thread,
    // after the copy.
    unsafe {
        let e = ffi::git_error_last();
        (
            (*e).klass,
            CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
        )
    }
}

fn clear() {
    // SAFETY: no arguments; libgit2 is initialised.
    unsafe { ffi::git_error_clear() }
}

/// C's outcome on its own copy of `name`, as `Ok(valid)` or `Err(rc)`, and the error it
/// leaves.
fn c_check(name: &CStr) -> (Result<bool, c_int>, (c_int, String)) {
    let copy = name.to_owned();
    let mut valid: c_int = -1;
    // SAFETY: `valid` is a writable `int`; `copy` is a NUL-terminated string, read for
    // the call only.
    let rc = unsafe { ffi::git_tag_name_is_valid(&mut valid, copy.as_ptr()) };
    let outcome = if rc < 0 {
        Err(rc)
    } else {
        assert!(valid == 0 || valid == 1, "C wrote valid = {valid}");
        Ok(valid != 0)
    };
    (outcome, c_last())
}

/// Rust's outcome on its own copy of `name`, and the error it leaves.
fn rust_check(name: &CStr) -> (Result<bool, c_int>, (c_int, String)) {
    let copy = name.to_owned();
    let outcome = name_is_valid(&copy).map_err(|e| e.raw_class());
    (outcome, c_last())
}

fn assert_equivalent(name: &CStr) {
    clear();
    let c = c_check(name);
    assert!(c.0.is_ok(), "C failed on {name:?}");
    clear();
    assert_eq!(rust_check(name), c, "{name:?}");
}

/// Names around every rule: the tag rules, each refname rule, and long ones.
fn named() -> Vec<CString> {
    let names: [&[u8]; 39] = [
        b"",
        b"v1.0",
        b"-v1.0",
        b"v-1",
        b"HEAD",
        b"HEADS",
        b"head",
        b"HEAD/x",
        b"x/HEAD",
        b"@",
        b"v@",
        b"a@{b",
        b"a{b",
        b"a..b",
        b".a",
        b"a.",
        b"a/.b",
        b"a/b.",
        b"a.lock",
        b"a.lock/b",
        b"a/b.lock",
        b"a//b",
        b"/a",
        b"a/",
        b"a b",
        b"a\tb",
        b"a~b",
        b"a^b",
        b"^a",
        b"a:b",
        b"a?b",
        b"a*b",
        b"a[b",
        b"a\\b",
        b"a\x7fb",
        b"\xc3\xa9t\xc3\xa9",
        b"\xff\x80",
        b"refs/heads/main",
        b"release/2024/q1",
    ];
    let mut names: Vec<CString> = names.iter().map(|n| CString::new(*n).unwrap()).collect();
    names.push(CString::new(vec![b'a'; 5000]).unwrap());
    names.push(CString::new("a/".repeat(500) + "b").unwrap());
    names
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn named_edge_cases() {
    init();
    for name in named() {
        assert_equivalent(&name);
    }
}

/// Every name of up to three characters over the characters the rules tell apart.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn short_names_exhaustively() {
    init();
    const CHARS: &[u8] = b"aH.-/@{*^ ~k";
    let mut names = vec![Vec::new()];
    for len in 1..=3 {
        let longer: Vec<Vec<u8>> = names
            .iter()
            .filter(|n: &&Vec<u8>| n.len() == len - 1)
            .flat_map(|n| CHARS.iter().map(move |&c| [&n[..], &[c]].concat()))
            .collect();
        names.extend(longer);
    }
    for name in names {
        assert_equivalent(&CString::new(name).unwrap());
    }
}

/// Without clearing the error between calls, C and Rust leave the same error after
/// each step.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn alternating_sequence_without_clearing() {
    init();
    let sequence: [&CStr; 10] = [
        c"v1", c"a..b", c"v2", c"-x", c"HEAD", c"b..c", c"v3", c"a.lock", c"", c"v4",
    ];
    clear();
    let c: Vec<_> = sequence.iter().map(|n| c_check(n)).collect();
    clear();
    let rust: Vec<_> = sequence.iter().map(|n| rust_check(n)).collect();
    for ((name, c), rust) in sequence.iter().zip(&c).zip(&rust) {
        assert_eq!(rust, c, "{name:?}");
    }
    // The reference error from "a..b" survives the valid "v2".
    assert_eq!(c[2].1.0, ffi::GIT_ERROR_REFERENCE as c_int);
    assert!(c[2].1.1.contains("refs/tags/a..b"));
}
