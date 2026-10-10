//! Safe wrappers over `include/git2/blob.h`.
//!
//! [`data_is_binary`] applies the binary-content heuristic `git_blob_is_binary` uses
//! (core git's) to a byte slice. libgit2 reads the bytes for the call only and touches
//! neither its allocator nor the thread's error state, so the check needs no
//! initialised runtime and is safe from any thread.

use core::ffi::c_int;

use crate::ffi;

/// Whether `data` is most likely binary, by core git's heuristic: it is binary when it
/// opens with a UTF-16 or UTF-32 byte-order mark, when it holds a NUL byte, or when
/// fewer than 128 printable bytes come for each non-printable one. Printable bytes are
/// those above `0x1f` but DEL, and backspace, escape and form feed; whitespace counts as
/// neither. A UTF-8 byte-order mark is skipped, and empty data is text.
///
/// Unlike `git_blob_is_binary`, it reads the whole slice, not its first 8000 bytes.
///
/// libgit2 counts printable and non-printable bytes in C `int`s, which overflow (UB)
/// once a slice holds more than `INT_MAX` of either. A slice that long never reaches it:
/// the same heuristic runs in Rust, with counters as wide as the slice's length.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::object::blob::data_is_binary;
///
/// assert!(!data_is_binary(b"hello, world\n"));
/// assert!(data_is_binary(b"hello\0world"));
/// assert!(data_is_binary(b"\xff\xfeh\0i\0"));
/// assert!(!data_is_binary(b""));
/// ```
///
/// Wraps: git_blob_data_is_binary
pub fn data_is_binary(data: &[u8]) -> bool {
    if !fits_c_counters(data.len()) {
        return wide::is_binary(data);
    }
    // SAFETY: `data` is live for `data.len()` bytes, read for the call only and never
    // retained or written; with a length of 0 libgit2 does not read the pointer at all.
    // The length is at most `INT_MAX`, so neither of libgit2's `int` counters, each
    // bounded by it, overflows. No allocation or error state is touched, so no runtime
    // pin is needed.
    let rc = unsafe { ffi::git_blob_data_is_binary(data.as_ptr().cast(), data.len()) };
    rc != 0
}

/// Whether libgit2's `int` counters can count a slice of `len` bytes: each counts a
/// subset of its bytes, so at most `INT_MAX` of them never overflows either.
fn fits_c_counters(len: usize) -> bool {
    len <= c_int::MAX as usize
}

/// `git_str_is_binary` (`src/util/str.c`), over slices longer than its `int` counters
/// can count.
mod wide {
    /// The bytes a byte-order mark at the start of `data` takes, and whether it marks
    /// UTF-16 or UTF-32 (which make the data binary), as `git_str_detect_bom` reads it.
    pub(super) fn bom(data: &[u8]) -> (usize, bool) {
        match data {
            [0, 0, 0xfe, 0xff, ..] => (4, true),
            [0xef, 0xbb, 0xbf, ..] => (3, false),
            [0xfe, 0xff, ..] => (2, true),
            [0xff, 0xfe, 0, 0, ..] => (4, true),
            [0xff, 0xfe, ..] => (2, true),
            _ => (0, false),
        }
    }

    /// [`super::data_is_binary`]'s heuristic, with `usize` counters.
    pub(super) fn is_binary(data: &[u8]) -> bool {
        let (skip, wide_bom) = bom(data);
        if wide_bom {
            return true;
        }
        let (mut printable, mut nonprintable) = (0usize, 0usize);
        for &c in &data[skip..] {
            if (c > 0x1f && c != 0x7f) || matches!(c, 0x08 | 0x1b | 0x0c) {
                printable += 1;
            } else if c == 0 {
                return true;
            } else if !matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
                nonprintable += 1;
            }
        }
        (printable >> 7) < nonprintable
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    /// Inputs around every branch of the heuristic: each byte-order mark and its
    /// truncations, NULs before and after one, each class of byte alone, and the
    /// printable-to-non-printable ratio on both sides of 128.
    fn corpus() -> Vec<Vec<u8>> {
        let mut inputs: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"a".to_vec(),
            b"\0".to_vec(),
            b"\xef\xbb\xbf".to_vec(),
            b"\xef\xbb\xbfplain".to_vec(),
            b"\xef\xbb\xbf\0".to_vec(),
            b"\xef\xbb".to_vec(),
            b"\xfe\xff".to_vec(),
            b"\xfe\xfe".to_vec(),
            b"\xff\xfe".to_vec(),
            b"\xff\xfe\0".to_vec(),
            b"\xff\xfe\0\0".to_vec(),
            b"\xff\xfe\0a".to_vec(),
            b"\0\0\xfe\xff".to_vec(),
            b"\0\0\xfe".to_vec(),
            b"\0\0\xfe\xfe".to_vec(),
            b"\xff\xff".to_vec(),
            b"line\r\n\tindented\x0b\x0c".to_vec(),
            b"\x08\x1b\x0c".to_vec(),
            b"\x7f".to_vec(),
            b"\x01".to_vec(),
            b"\x80\xff\xa0".to_vec(),
        ];
        for (printable, nonprintable) in [(127, 1), (128, 1), (255, 1), (256, 2), (256, 3)] {
            let mut ratio = vec![b'x'; printable];
            ratio.extend(std::iter::repeat_n(0x01, nonprintable));
            inputs.push(ratio);
        }
        inputs.extend((0..=u8::MAX).map(|c| vec![c]));
        inputs.extend((0..=u8::MAX).map(|c| vec![b'a', c, b'b']));
        inputs
    }

    /// `git_blob_data_is_binary` itself, as the C reference.
    fn c_is_binary(data: &[u8]) -> bool {
        // SAFETY: `data` is live for `data.len()` bytes, at most `INT_MAX`.
        unsafe { ffi::git_blob_data_is_binary(data.as_ptr().cast(), data.len()) != 0 }
    }

    /// The Rust heuristic that stands in for libgit2 on oversized slices agrees with it
    /// on every input libgit2 can take.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn wide_heuristic_matches_libgit2() {
        for input in corpus() {
            assert_eq!(wide::is_binary(&input), c_is_binary(&input), "{input:x?}");
            assert_eq!(data_is_binary(&input), c_is_binary(&input), "{input:x?}");
        }
    }

    /// The byte-order marks `git_str_detect_bom` recognises, and the bytes each takes.
    #[test]
    fn byte_order_marks() {
        assert_eq!(wide::bom(b"\0\0\xfe\xffx"), (4, true));
        assert_eq!(wide::bom(b"\xef\xbb\xbfx"), (3, false));
        assert_eq!(wide::bom(b"\xfe\xffx"), (2, true));
        assert_eq!(wide::bom(b"\xff\xfe\0\0x"), (4, true));
        assert_eq!(wide::bom(b"\xff\xfe\0x"), (2, true));
        assert_eq!(wide::bom(b"\xef\xbb"), (0, false));
        assert_eq!(wide::bom(b"\xff"), (0, false));
        assert_eq!(wide::bom(b""), (0, false));
    }

    /// The Rust heuristic's verdicts on each branch, without libgit2, so Miri checks
    /// its slicing past a byte-order mark too.
    #[test]
    fn wide_heuristic_verdicts() {
        for (data, binary) in [
            (&b""[..], false),
            (b"\xef\xbb\xbf", false),
            (b"\xef\xbb\xbf\0", true),
            (b"\xff\xfe", true),
            (b"\0\0\xfe\xff", true),
            (b"text\r\n\t\x0b\x0c\x08\x1b", false),
            (b"\x01", true),
            (b"\x7f", true),
            (b"\x80\xff", false),
        ] {
            assert_eq!(wide::is_binary(data), binary, "{data:x?}");
        }
        let mut ratio = vec![b'x'; 256];
        ratio.push(0x01);
        assert!(!wide::is_binary(&ratio));
        ratio.push(0x01);
        assert!(!wide::is_binary(&ratio));
        ratio.push(0x01);
        assert!(wide::is_binary(&ratio));
    }

    /// Slices up to `INT_MAX` bytes reach libgit2; one byte more takes the Rust
    /// heuristic.
    #[test]
    fn longest_slice_for_libgit2_is_int_max() {
        assert!(fits_c_counters(0));
        assert!(fits_c_counters(c_int::MAX as usize));
        assert!(!fits_c_counters(c_int::MAX as usize + 1));
        assert!(!fits_c_counters(usize::MAX));
    }

    /// End to end, a slice whose printable bytes would overflow libgit2's `int` counter
    /// (`INT_MAX + 1` of them) is counted in Rust. It scans 2 GiB, which takes minutes
    /// under the sanitizers: run it with `--ignored`.
    #[test]
    #[ignore = "scans 2 GiB; run with --ignored"]
    fn oversized_slices_are_counted_in_rust() {
        let mut data = vec![b'a'; c_int::MAX as usize + 2];
        // One non-printable byte for 2^24 printable ones is still text.
        data[0] = 0x01;
        assert!(!data_is_binary(&data));
    }
}
