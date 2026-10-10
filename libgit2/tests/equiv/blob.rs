//! Equivalence of `git2/blob.h`'s `data_is_binary` with `git_blob_data_is_binary`, and
//! of its filter flags with C's in [`filter_flags`].
//!
//! Each probe hands the same bytes, in two independently owned buffers, to C and to
//! Rust, and compares the verdicts and the calling thread's last error after each
//! call: libgit2 must leave an error set beforehand untouched, on both sides. The
//! probes cover every byte-order mark and its truncations, each byte alone and between
//! printable ones, the printable-to-non-printable ratio on both sides of 128, runs of
//! each whitespace byte around that ratio, NULs past the 8000 bytes
//! `git_blob_is_binary` stops at, and a deterministic stream of generated inputs. A
//! multi-call sequence grows one buffer a byte at a time, comparing after each step.
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

/// Each whitespace and control byte the heuristic names, in a run of 128 carrying one
/// control byte: C counts space, form feed, backspace and escape as printable (text),
/// and tab, line feed, vertical tab and carriage return as neither (binary).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn whitespace_classes() {
    init();
    for ws in [b' ', b'\t', b'\n', 0x0b, 0x0c, b'\r', 0x08, 0x1b, 0x7f] {
        for count in [127, 128] {
            let mut run = vec![ws; count];
            run.push(0x01);
            assert_equivalent(&run);
        }
    }
    let mut spaces = vec![b' '; 128];
    spaces.push(0x01);
    assert!(!c_verdict(&spaces).0);
    let mut tabs = vec![b'\t'; 128];
    tabs.push(0x01);
    assert!(c_verdict(&tabs).0);
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

/// Equivalence of `git2/blob.h`'s [`FilterFlag`] and [`FilterFlags`] with the
/// `git_blob_filter_flag_t` values libgit2 declares and `git_blob_filter` reads.
///
/// [`FilterFlag`] must name exactly the `git_blob_filter_flag_t` constants, each with
/// its C value, and [`FilterFlags::OPTIONS_DEFAULT`] must be the flags
/// `git_blob_filter_options_init` sets.
///
/// libgit2 observes a set only as `git_blob_filter_options.flags`. For every
/// combination of the named flags, with and without a bit the header does not name,
/// each side builds its own options (C: by ORing the C constants into what
/// `git_blob_filter_options_init` filled; Rust: from a [`FilterFlags`] collected from
/// the flags), and filters the same blobs of the `crlf.git` fixture under the same
/// paths into its own buffer, one call after another without clearing it. After every
/// call the return codes and the buffers are compared: [`FilterFlag::CheckForBinary`]
/// leaves the buffer as the previous call left it for a binary blob, and the
/// attribute flags change which conversions apply. The safe wrapper of
/// `git_blob_filter` itself belongs to the `object` symbol batches.
mod filter_flags {
    use std::ffi::{CStr, c_int};
    use std::ptr;
    use std::sync::OnceLock;

    use libgit2::ffi;
    use libgit2::libgit2::object::blob::{FilterFlag, FilterFlags, UnnamedFilterFlag};

    use super::init;

    /// Every `git_blob_filter_flag_t` constant and the variant carrying it.
    const FLAGS: [(ffi::git_blob_filter_flag_t, FilterFlag); 4] = [
        (
            ffi::GIT_BLOB_FILTER_CHECK_FOR_BINARY,
            FilterFlag::CheckForBinary,
        ),
        (
            ffi::GIT_BLOB_FILTER_NO_SYSTEM_ATTRIBUTES,
            FilterFlag::NoSystemAttributes,
        ),
        (
            ffi::GIT_BLOB_FILTER_ATTRIBUTES_FROM_HEAD,
            FilterFlag::AttributesFromHead,
        ),
        (
            ffi::GIT_BLOB_FILTER_ATTRIBUTES_FROM_COMMIT,
            FilterFlag::AttributesFromCommit,
        ),
    ];

    /// A bit `git2/blob.h` does not name, which `git_blob_filter` must ignore.
    const UNNAMED: u32 = 1 << 8;

    /// `crlf.git` objects: text with CRLF endings, a binary blob, an `$Id$` blob, and the
    /// commit whose `.gitattributes` gives `*.identlf` the `ident` attribute.
    const ALL_CRLF: &CStr = c"a9a2e8913c1dbe2812fac5e6b4e0a4bd5d0d5966";
    const BINARY: &CStr = c"85340755cfe5e28c2835781978bb1cece91b3d0f";
    const IDENT: &CStr = c"055c8729cdcc372500a08db659c045e16c4409fb";
    const IDENT_COMMIT: &CStr = c"b8986fec0f7bde90f78ac72706e782d82f24f2f0";

    /// The calls of one sequence: a blob and the path it is filtered as.
    const CALLS: [(&CStr, &CStr); 8] = [
        (ALL_CRLF, c"file.crlf"),
        (BINARY, c"file.crlf"),
        (ALL_CRLF, c"file.bin"),
        (BINARY, c"file.txt"),
        (IDENT, c"ident.identlf"),
        (BINARY, c"file.lf"),
        (ALL_CRLF, c"raw/file.crlf"),
        (IDENT, c"ident.bin"),
    ];

    /// Every value up to past the highest flag bit, each single bit and its neighbours,
    /// and the extremes.
    fn probes() -> impl Iterator<Item = u32> {
        (0..=64)
            .chain((0..32).flat_map(|k| {
                let bit = 1u32 << k;
                [bit - 1, bit, bit + 1]
            }))
            .chain([u32::MAX - 1, u32::MAX])
    }

    fn oid(hex: &CStr) -> ffi::git_oid {
        let mut out = ffi::git_oid::default();
        // SAFETY: `out` is writable; `hex` is a NUL-terminated 40-digit SHA-1.
        let rc = unsafe { ffi::git_oid_from_string(&mut out, hex.as_ptr(), ffi::GIT_OID_SHA1) };
        assert_eq!(rc, 0);
        out
    }

    /// One side's own `crlf.git`, opened read-only, and the blobs it looks up.
    struct Side {
        repo: *mut ffi::git_repository,
        blobs: Vec<*mut ffi::git_blob>,
        buf: ffi::git_buf,
    }

    impl Side {
        fn open() -> Self {
            // libgit2 marks a filter initialised on its first load under the registry's
            // read lock only (`filter_initialize`), so concurrent first loads race (TSan):
            // one side runs every call alone first, loading the CRLF and ident filters.
            static PRIMED: OnceLock<()> = OnceLock::new();
            PRIMED.get_or_init(|| {
                let mut side = Self::build();
                let mut opts = c_options();
                opts.flags =
                    (FilterFlag::AttributesFromHead | FilterFlag::AttributesFromCommit).bits();
                for i in 0..CALLS.len() {
                    assert_eq!(side.filter(i, &mut opts).0, 0);
                }
            });
            Self::build()
        }

        fn build() -> Self {
            init();
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../tests/resources/crlf.git\0"
            );
            let mut repo = ptr::null_mut();
            // SAFETY: `repo` receives the repository; `path` is NUL-terminated.
            let rc = unsafe { ffi::git_repository_open_bare(&mut repo, path.as_ptr().cast()) };
            assert_eq!(rc, 0);
            let blobs = CALLS
                .iter()
                .map(|(id, _)| {
                    let mut blob = ptr::null_mut();
                    let id = oid(id);
                    // SAFETY: `repo` is open; `blob` receives a blob this side frees.
                    assert_eq!(unsafe { ffi::git_blob_lookup(&mut blob, repo, &id) }, 0);
                    blob
                })
                .collect();
            Self {
                repo,
                blobs,
                buf: ffi::git_buf::default(),
            }
        }

        /// Filters the `i`th call's blob into this side's buffer, as libgit2 leaves it.
        fn filter(
            &mut self,
            i: usize,
            opts: &mut ffi::git_blob_filter_options,
        ) -> (c_int, Vec<u8>) {
            // SAFETY: the buffer is this side's, zeroed or filled by libgit2; the blob is
            // live; the path is NUL-terminated; `opts` is initialised options.
            let rc = unsafe {
                ffi::git_blob_filter(&mut self.buf, self.blobs[i], CALLS[i].1.as_ptr(), opts)
            };
            let out = if self.buf.ptr.is_null() {
                Vec::new()
            } else {
                // SAFETY: libgit2 left `size` initialised bytes at `ptr`.
                unsafe { std::slice::from_raw_parts(self.buf.ptr.cast::<u8>(), self.buf.size) }
                    .to_vec()
            };
            (rc, out)
        }
    }

    impl Drop for Side {
        fn drop(&mut self) {
            // SAFETY: each object is this side's and released once, the buffer last
            // written by libgit2.
            unsafe {
                ffi::git_buf_dispose(&mut self.buf);
                for &blob in &self.blobs {
                    ffi::git_blob_free(blob);
                }
                ffi::git_repository_free(self.repo);
            }
        }
    }

    /// Options as `git_blob_filter_options_init` fills them, with the commit
    /// `ATTRIBUTES_FROM_COMMIT` reads.
    fn c_options() -> ffi::git_blob_filter_options {
        // SAFETY: `git_blob_filter_options` is plain data, all-zero valid.
        let mut opts: ffi::git_blob_filter_options = unsafe { std::mem::zeroed() };
        // SAFETY: `opts` is writable; the version is this header's.
        let rc = unsafe {
            ffi::git_blob_filter_options_init(&mut opts, ffi::GIT_BLOB_FILTER_OPTIONS_VERSION)
        };
        assert_eq!(rc, 0);
        opts.attr_commit_id = oid(IDENT_COMMIT);
        opts
    }

    #[test]
    fn flag_from_raw_names_exactly_the_c_constants() {
        for (c, flag) in FLAGS {
            assert_eq!(flag.raw(), c, "{flag:?}");
            assert_eq!(FilterFlags::from(flag).bits(), c, "{flag:?}");
        }
        assert_eq!(FilterFlag::ALL.map(FilterFlag::raw), FLAGS.map(|(c, _)| c));
        for raw in probes() {
            let c = FLAGS.iter().find(|(v, _)| *v == raw).map(|&(_, f)| f);
            assert_eq!(FilterFlag::from_raw(raw), c, "{raw}");
            if c.is_none() {
                assert_eq!(
                    FilterFlag::try_from(raw).map_err(UnnamedFilterFlag::raw),
                    Err(raw)
                );
            }
            let set = FilterFlags::from_bits_retain(raw);
            for (bit, flag) in FLAGS {
                assert_eq!(set.contains(flag), raw & bit != 0, "{raw:#x} {flag:?}");
            }
            let named = FLAGS.iter().fold(0, |acc, (bit, _)| acc | bit);
            assert_eq!(set.unnamed_bits(), raw & !named);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn options_default_matches_git_blob_filter_options_init() {
        let opts = c_options();
        assert_eq!(FilterFlags::OPTIONS_DEFAULT.bits(), opts.flags);
        assert_eq!(
            FilterFlags::from_bits_retain(opts.flags)
                .iter()
                .collect::<Vec<_>>(),
            [FilterFlag::CheckForBinary]
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn git_blob_filter_reads_rust_sets_as_c_flags() {
        let (mut c, mut rust) = (Side::open(), Side::open());
        for combo in 0..1u32 << FLAGS.len() {
            for unnamed in [0, UNNAMED] {
                let chosen: Vec<_> = (0..FLAGS.len()).filter(|i| combo & (1 << i) != 0).collect();

                let mut c_opts = c_options();
                c_opts.flags = 0;
                for &i in &chosen {
                    c_opts.flags |= FLAGS[i].0;
                }
                c_opts.flags |= unnamed;

                let mut set: FilterFlags = chosen.iter().map(|&i| FLAGS[i].1).collect();
                set |= FilterFlags::from_bits_retain(unnamed);
                let mut rust_opts = c_options();
                rust_opts.flags = set.bits();

                for (i, call) in CALLS.iter().enumerate() {
                    let expected = c.filter(i, &mut c_opts);
                    let got = rust.filter(i, &mut rust_opts);
                    assert_eq!(got, expected, "{set:?}, call {i} {call:?}");
                    assert_eq!(expected.0, 0, "{set:?}, call {i}");
                }
            }
        }
    }

    /// The flags do reach libgit2: [`FilterFlag::CheckForBinary`] keeps a binary blob
    /// out of the buffer, and the attribute flags turn `ident` expansion on.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn rust_sets_change_what_git_blob_filter_does() {
        let mut side = Side::open();
        let mut opts = c_options();

        opts.flags = FilterFlags::empty().bits();
        let (rc, binary_unchecked) = side.filter(1, &mut opts);
        assert_eq!(rc, 0);
        assert!(!binary_unchecked.is_empty());
        let mut fresh = Side::open();
        opts.flags = FilterFlags::from(FilterFlag::CheckForBinary).bits();
        let (rc, binary_checked) = fresh.filter(1, &mut opts);
        assert_eq!((rc, binary_checked.len()), (0, 0));

        opts.flags = FilterFlags::from(FilterFlag::NoSystemAttributes).bits();
        let (_, plain) = side.filter(4, &mut opts);
        assert_eq!(plain, b"$Id$\n");
        for flag in [
            FilterFlag::AttributesFromHead,
            FilterFlag::AttributesFromCommit,
        ] {
            opts.flags = (FilterFlag::NoSystemAttributes | flag).bits();
            let (rc, expanded) = side.filter(4, &mut opts);
            assert_eq!(rc, 0);
            assert_eq!(
                expanded, b"$Id: 055c8729cdcc372500a08db659c045e16c4409fb $\n",
                "{flag:?}"
            );
        }
    }
}
