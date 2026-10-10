//! UB tests for `git2/blob.h`: `data_is_binary` (`git_blob_data_is_binary`), and the
//! filter flags in [`filter_flags`].
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

/// The filter flags: the enum `FilterFlag` and the flag set `FilterFlags`.
///
/// `FilterFlag` is a fieldless enum over C's `git_blob_filter_flag_t`. The only safe
/// way in from a raw value is `from_raw` / `try_from`, which must reject every value
/// without a variant rather than produce an invalid discriminant (which Miri and UBSan
/// would flag on the next read); an `as` cast to it does not compile (`compile_fail/`).
/// `FilterFlags` keeps any `u32`, so its operations must stay in range for every bit
/// pattern, its shifts and masks included (UBSan). Nothing here reaches libgit2, so
/// every test runs under Miri; both types are plain values, `Send` and `Sync` by
/// construction, with nothing shared to race (TSan) and no pointer to retain (BSan).
mod filter_flags {
    use std::collections::HashSet;
    use std::hint::black_box;
    use std::thread;

    use libgit2::libgit2::object::blob::{FilterFlag, FilterFlags};

    /// Every value around the declared ones, each single bit, and a coarse stride
    /// across the whole type.
    fn probes() -> impl Iterator<Item = u32> {
        (0..=300)
            .chain((0..32).map(|k| 1u32 << k))
            .chain((0..=u32::MAX).step_by(1 << 26))
            .chain([u32::MAX - 1, u32::MAX])
    }

    /// Bit patterns for `FilterFlags`: each single bit, its complement, and the
    /// extremes.
    fn bit_patterns() -> impl Iterator<Item = u32> {
        (0..32)
            .flat_map(|k| [1 << k, !(1 << k)])
            .chain([0, u32::MAX, 0b1111, 0x8000_0000])
    }

    #[test]
    fn flag_from_raw_yields_only_valid_flags() {
        let mut seen = HashSet::new();
        for raw in probes() {
            // `black_box` keeps the conversion from being folded at compile time.
            if let Some(flag) = FilterFlag::from_raw(black_box(raw)) {
                // Reading the discriminant back validates it.
                assert_eq!(flag.raw(), raw);
                assert!(!format!("{flag:?}").is_empty());
                seen.insert(flag);
            } else {
                assert_eq!(FilterFlag::try_from(raw).map_err(|e| e.raw()), Err(raw));
            }
        }
        assert_eq!(seen.len(), FilterFlag::ALL.len());
    }

    #[test]
    fn option_niche_holds_for_every_value() {
        // `Option<FilterFlag>` stores `None` in a value no variant uses: a rejected
        // value must never alias a variant, nor a variant read as `None`.
        for raw in probes() {
            let flag = black_box(FilterFlag::from_raw(raw));
            assert_eq!(flag.map(FilterFlag::raw).unwrap_or(raw), raw);
        }
    }

    #[test]
    fn flags_stay_in_range_for_every_bit_pattern() {
        for bits in bit_patterns() {
            let mut set = FilterFlags::from_bits_retain(black_box(bits));
            assert_eq!(set.bits(), bits);
            let named: Vec<_> = set.iter().collect();
            assert!(named.len() <= FilterFlag::ALL.len());
            assert!(named.iter().all(|&f| set.contains(f)));
            assert_eq!(set.into_iter().count(), named.len());
            assert!(!format!("{set:?}").is_empty());
            let truncated = FilterFlags::from_bits_truncate(bits);
            assert_eq!(truncated.unnamed_bits(), 0);
            assert_eq!(truncated.bits() | set.unnamed_bits(), bits);
            for flag in FilterFlag::ALL {
                set.insert(flag);
                assert!(set.contains(flag));
                set.remove(flag);
                assert!(!set.contains(flag));
            }
            assert_eq!(set.bits(), set.unnamed_bits());
            set |= FilterFlags::from_bits_retain(bits);
            assert_eq!(set.bits(), bits);
        }
    }

    /// Sets built and read on several threads at once, and moved between them.
    #[test]
    fn sets_cross_threads() {
        let shared = FilterFlag::CheckForBinary | FilterFlag::AttributesFromCommit;
        thread::scope(|s| {
            let handles: Vec<_> = FilterFlag::ALL
                .into_iter()
                .map(|flag| {
                    let shared = &shared;
                    s.spawn(move || {
                        let mut mine = *shared;
                        mine |= flag;
                        assert!(mine.contains(flag));
                        mine
                    })
                })
                .collect();
            for h in handles {
                assert!(h.join().unwrap().bits() & shared.bits() == shared.bits());
            }
        });
    }
}
