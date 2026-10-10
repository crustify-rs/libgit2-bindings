//! UB tests for `git2/tag.h`: `name_is_valid` (`git_tag_name_is_valid`).
//!
//! libgit2 reads the name up to its NUL, copies it into a reference name in its
//! allocator, and writes the verdict into a Rust local. Each name sits in an
//! exact-length heap buffer (a `CString`), so ASan flags any read past its NUL, and the
//! copy must be released (LSan). The name is rewritten between calls (BSan: libgit2
//! keeps no pointer into it), and many threads check names at once, racing the first
//! call's initialisation of libgit2 (TSan).

use std::ffi::{CStr, CString};
use std::thread;

use libgit2::libgit2::object::tag::name_is_valid;

use crate::fixture::isolated;

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn names_are_read_up_to_their_nul() {
    for name in [
        &b""[..],
        b"v",
        b"-",
        b"HEAD",
        b"HEADS",
        b"a/b",
        b"a/",
        b"/a",
        b"a.lock",
        b".lock",
        b"a.",
        b"@",
        b"a@{b",
        b"\xff\xfe\x80",
        b"a\x7f",
        b"refs/tags/v1",
    ] {
        let name = CString::new(name).unwrap();
        let _ = name_is_valid(&name).unwrap();
    }
}

/// A long name makes libgit2 grow its reference name, and every component of a long
/// path is validated.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn long_names_are_copied_and_released() {
    let long = CString::new(vec![b'a'; 1 << 20]).unwrap();
    assert_eq!(name_is_valid(&long), Ok(true));
    let path = CString::new("a/".repeat(4096) + "z").unwrap();
    assert_eq!(name_is_valid(&path), Ok(true));
    let trailing = CString::new("a/".repeat(4096)).unwrap();
    assert_eq!(name_is_valid(&trailing), Ok(false));
}

/// The caller rewrites the name's buffer between calls: libgit2 keeps no pointer into
/// it, and each call sees the current bytes.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn name_rewritten_between_calls() {
    let mut buf = *b"v1.0\0";
    let name = CStr::from_bytes_with_nul(&buf).unwrap();
    assert_eq!(name_is_valid(name), Ok(true));
    buf[0] = b'-';
    let name = CStr::from_bytes_with_nul(&buf).unwrap();
    assert_eq!(name_is_valid(name), Ok(false));
    buf[0] = b'v';
    buf[1] = b'.';
    let name = CStr::from_bytes_with_nul(&buf).unwrap();
    assert_eq!(name_is_valid(name), Ok(false));
}

/// Many threads make the first call at once, in a process where nothing has
/// initialised libgit2, then keep checking names: the runtime pin, libgit2's allocator
/// and each thread's error state must not race.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn concurrent_first_calls() {
    isolated(concat!(module_path!(), "::concurrent_first_calls"), || {
        thread::scope(|s| {
            for i in 0..8 {
                s.spawn(move || {
                    let good = CString::new(format!("v{i}")).unwrap();
                    let bad = CString::new(format!("v{i}..")).unwrap();
                    for _ in 0..100 {
                        assert_eq!(name_is_valid(&good), Ok(true));
                        assert_eq!(name_is_valid(&bad), Ok(false));
                    }
                });
            }
        });
    });
}
