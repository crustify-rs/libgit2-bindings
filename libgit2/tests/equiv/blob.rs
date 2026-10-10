//! Equivalence of `git2/blob.h`'s `data_is_binary` with `git_blob_data_is_binary`.
//!
//! Each probe hands the same bytes, in two independently owned buffers, to C and to
//! Rust, and compares the verdicts and the calling thread's last error after each
//! call: libgit2 must leave an error set beforehand untouched, on both sides. The
//! probes cover every byte-order mark and its truncations, each byte alone and between
//! printable ones, the printable-to-non-printable ratio on both sides of 128, NULs
//! past the 8000 bytes `git_blob_is_binary` stops at, and a deterministic stream of
//! generated inputs. A multi-call sequence grows one buffer a byte at a time,
//! comparing after each step.
//!
//! Documented divergence, outside these comparisons: C counts bytes in `int`s, which
//! overflow (UB, flagged by UBSan at `src/util/str.c:1271`) on a slice holding more
//! than `INT_MAX` printable or non-printable bytes. Rust never hands such a slice to C
//! and counts it in `usize`s; the corrected behaviour is unit-tested beside the
//! wrapper.

use std::ffi::{CStr, c_int};

use libgit2::ffi;
use libgit2::libgit2::object::blob::data_is_binary;

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

/// Sets a marker error, which neither side may disturb.
fn mark() {
    // SAFETY: a valid class and a NUL-terminated message, copied by libgit2.
    let rc = unsafe { ffi::git_error_set_str(ffi::GIT_ERROR_INVALID as c_int, c"marker".as_ptr()) };
    assert_eq!(rc, 0);
}

/// C's verdict on its own copy of `data`, and the error it leaves.
fn c_verdict(data: &[u8]) -> (bool, (c_int, String)) {
    let copy = data.to_vec();
    mark();
    // SAFETY: `copy` is live for `copy.len()` bytes, far below `INT_MAX`.
    let rc = unsafe { ffi::git_blob_data_is_binary(copy.as_ptr().cast(), copy.len()) };
    assert!(rc == 0 || rc == 1, "C returned {rc}");
    (rc != 0, c_last())
}

/// Rust's verdict on its own copy of `data`, and the error it leaves.
fn rust_verdict(data: &[u8]) -> (bool, (c_int, String)) {
    let copy = data.to_vec();
    mark();
    (data_is_binary(&copy), c_last())
}

fn assert_equivalent(data: &[u8]) {
    let c = c_verdict(data);
    assert_eq!(c.1, (ffi::GIT_ERROR_INVALID as c_int, "marker".into()));
    assert_eq!(rust_verdict(data), c, "{data:x?}");
}

/// A deterministic xorshift stream of inputs, biased towards the bytes the heuristic
/// tells apart.
fn generated() -> impl Iterator<Item = Vec<u8>> {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    const PICKS: [u8; 12] = [
        0, 0x01, 0x08, b'\t', b'\n', 0x0c, 0x1b, b'a', 0x7f, 0xef, 0xfe, 0xff,
    ];
    (0..2000).map(move |_| {
        let len = (next() % 400) as usize;
        (0..len)
            .map(|_| {
                let r = next();
                if r % 4 == 0 {
                    PICKS[(r >> 8) as usize % PICKS.len()]
                } else if r % 4 == 1 {
                    (r >> 8) as u8
                } else {
                    b' ' + (r >> 8) as u8 % 95
                }
            })
            .collect()
    })
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn byte_order_marks_and_their_truncations() {
    init();
    for bom in [
        &b"\0\0\xfe\xff"[..],
        b"\xef\xbb\xbf",
        b"\xfe\xff",
        b"\xff\xfe\0\0",
        b"\xff\xfe",
    ] {
        for len in 0..=bom.len() {
            for tail in [&b""[..], b"x", b"\0", b"text\n", b"\x01"] {
                assert_equivalent(&[&bom[..len], tail].concat());
            }
        }
    }
    for odd in [
        &b"\xff\xff"[..],
        b"\xfe\xfe",
        b"\0\0\xfe",
        b"\0\0\xfe\xfe",
        b"\xff\xfe\0a",
    ] {
        assert_equivalent(odd);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn every_byte_alone_and_among_text() {
    init();
    for c in 0..=u8::MAX {
        assert_equivalent(&[c]);
        assert_equivalent(&[b'a', c, b'b']);
        let mut many = vec![b'x'; 200];
        many.push(c);
        assert_equivalent(&many);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn printable_ratio_around_128() {
    init();
    for printable in [0, 1, 127, 128, 255, 256, 383, 384] {
        for nonprintable in 0..4 {
            let mut data = vec![b'p'; printable];
            data.extend(std::iter::repeat_n(0x02, nonprintable));
            data.extend(b"\r\n\t \x0b");
            assert_equivalent(&data);
        }
    }
}

/// Unlike `git_blob_is_binary`, both read past 8000 bytes.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn whole_slice_is_read() {
    init();
    let mut data = vec![b't'; 10_000];
    assert_equivalent(&data);
    data[9_000] = 0;
    assert_equivalent(&data);
    data[9_000] = 0x03;
    assert_equivalent(&data);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn generated_inputs() {
    init();
    for data in generated() {
        assert_equivalent(&data);
    }
}

/// One buffer grows a byte at a time, crossing a byte-order mark, the ratio and a NUL;
/// both sides agree after every step.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn growing_buffer_sequence() {
    init();
    let mut data = Vec::new();
    let steps = b"\xef\xbb\xbf"
        .iter()
        .copied()
        .chain(std::iter::repeat_n(b'w', 130))
        .chain([0x01, 0x02, b'\n', 0x7f])
        .chain(std::iter::repeat_n(b'w', 300))
        .chain([0]);
    for byte in steps {
        data.push(byte);
        assert_equivalent(&data);
    }
}
