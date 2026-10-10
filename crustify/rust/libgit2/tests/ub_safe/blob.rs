//! UB tests for `git2/blob.h`: `data_is_binary` (`git_blob_data_is_binary`).
//!
//! libgit2 reads the slice it is given, up to four bytes at once when it looks for a
//! byte-order mark, and nothing else. Each input sits in an exact-length heap buffer,
//! so ASan flags any read past it, with every byte-order mark cut short at the end of
//! its buffer; the empty slice hands libgit2 a dangling pointer it must not read. The
//! slice is written between calls (BSan: libgit2 keeps no pointer into it), and read
//! from many threads at once (TSan: libgit2 touches no shared state).

use std::thread;

use libgit2::libgit2::object::blob::data_is_binary;

/// `bytes` in a heap buffer of exactly its length.
fn exact(bytes: &[u8]) -> Box<[u8]> {
    bytes.to_vec().into_boxed_slice()
}

/// Inputs that end inside each byte-order mark and right after it, NULs, control and
/// high bytes, and the 128:1 printable ratio on either side.
fn inputs() -> Vec<Vec<u8>> {
    let mut inputs = Vec::new();
    for bom in [
        &b"\0\0\xfe\xff"[..],
        b"\xef\xbb\xbf",
        b"\xfe\xff",
        b"\xff\xfe\0\0",
        b"\xff\xfe",
    ] {
        for len in 0..=bom.len() {
            inputs.push(bom[..len].to_vec());
            let mut tail = bom[..len].to_vec();
            tail.push(b'x');
            inputs.push(tail);
        }
    }
    inputs.extend((0..=u8::MAX).map(|c| vec![c]));
    inputs.extend((0..=u8::MAX).map(|c| vec![c, c]));
    for nonprintable in [1, 2] {
        let mut ratio = vec![b'x'; 256];
        ratio.extend(std::iter::repeat_n(0x01, nonprintable));
        inputs.push(ratio);
    }
    inputs
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_slices_are_not_read() {
    assert!(!data_is_binary(&[]));
    assert!(!data_is_binary(&exact(b"")));
    assert!(!data_is_binary(&Vec::<u8>::with_capacity(0)));
    let some = exact(b"abc");
    assert!(!data_is_binary(&some[3..]));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn exact_buffers_are_read_within_bounds() {
    for input in inputs() {
        let buf = exact(&input);
        let whole = data_is_binary(&buf);
        // Every suffix ends at the buffer's end; libgit2 must not read past it.
        for start in 0..buf.len() {
            let _ = data_is_binary(&buf[start..]);
        }
        assert_eq!(data_is_binary(&buf), whole, "{input:x?}");
    }
}

/// Slices from the stack, a static, and odd offsets into the heap: libgit2 reads bytes,
/// so no alignment is assumed.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn slices_from_any_storage_and_alignment() {
    static TEXT: [u8; 6] = *b"static";
    let stack = *b"\xef\xbb\xbfstack";
    assert!(!data_is_binary(&TEXT));
    assert!(!data_is_binary(&stack));
    let heap = exact(b"\0unaligned text\x01\x02");
    for offset in 1..8 {
        assert!(!data_is_binary(&heap[offset..offset + 6]));
    }
    assert!(data_is_binary(&heap[..1]));
}

/// The caller writes the slice between calls: libgit2 keeps no pointer into it, so each
/// call sees the current bytes and nothing writes them behind Rust's back.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn slice_written_between_calls() {
    let mut buf = exact(b"plain text");
    assert!(!data_is_binary(&buf));
    buf[5] = 0;
    assert!(data_is_binary(&buf));
    let view = &mut buf[..];
    view[5] = b' ';
    assert!(!data_is_binary(view));
    view[..2].copy_from_slice(b"\xff\xfe");
    assert!(data_is_binary(view));
    assert_eq!(&buf[2..], b"ain text");
}

/// Many threads read one shared slice and their own at once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn concurrent_calls_share_nothing() {
    let shared = exact(b"shared text\n");
    thread::scope(|s| {
        for i in 0..8u8 {
            let shared = &shared;
            s.spawn(move || {
                let own = exact(&[b'a' + i, if i % 2 == 0 { b'b' } else { 0 }]);
                for _ in 0..200 {
                    assert!(!data_is_binary(shared));
                    assert_eq!(data_is_binary(&own), i % 2 == 1);
                }
            });
        }
    });
}
