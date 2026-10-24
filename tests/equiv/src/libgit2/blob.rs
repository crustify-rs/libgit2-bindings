//! Equivalence of `git2/blob.h`'s `data_is_binary` with `git_blob_data_is_binary`, of
//! its filter flags with C's, and `BlobRef::filter` with `git_blob_filter`, in
//! [`filter_flags`], of the blob owner, its constructors and its getters with
//! `git_blob_lookup` and friends in [`owner`], and of the filter options with
//! `git_blob_filter_options_init` in [`filter_options`].
//!
//! `BlobRef::filter` refuses two inputs libgit2 mishandles, outside these comparisons:
//! an absolute path, which libgit2 asserts on (a release build fails with the error the
//! wrapper reports, a debug one aborts), and an attribute commit id of another type
//! than the repository's, which libgit2 would format out of bounds. The unit tests of
//! `blob.rs` hold both refusals.
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
//! overflow (UB, flagged by `UBSan` at `src/util/str.c:1271`) on a slice holding more
//! than `INT_MAX` printable or non-printable bytes. Rust never hands such a slice to C
//! and counts it in `usize`s; the corrected behaviour is unit-tested beside the
//! wrapper.

use std::ffi::{CStr, c_int};

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::blob::data_is_binary;

/// Initialises libgit2; never released.
fn init() {
    // SAFETY: no arguments; the reference is never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

/// The calling thread's last error class and message.
fn c_last() -> (c_int, String) {
    {
        // SAFETY: `git_error_last` never returns NULL after init, and its message is a
        // NUL-terminated string, both live until the next libgit2 call on this thread,
        // after the copy.
        let e = unsafe { ffi::git_error_last() };
        // SAFETY: as above.
        let klass = unsafe { (*e).klass };
        // SAFETY: as above.
        let message2 = unsafe { (*e).message };
        // SAFETY: as above.
        unsafe {
            (
                klass,
                CStr::from_ptr(message2).to_string_lossy().into_owned(),
            )
        }
    }
}

/// Sets a marker error, which neither side may disturb.
fn mark() {
    // SAFETY: a valid class and a NUL-terminated message, copied by libgit2.
    let rc =
        unsafe { ffi::git_error_set_str(ffi::GIT_ERROR_INVALID.cast_signed(), c"marker".as_ptr()) };
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
    assert_eq!(c.1, (ffi::GIT_ERROR_INVALID.cast_signed(), "marker".into()));
    assert_eq!(rust_verdict(data), c, "{data:x?}");
}

/// A deterministic xorshift stream of inputs, biased towards the bytes the heuristic
/// tells apart.
#[expect(
    clippy::items_after_statements,
    reason = "each helper item sits beside the steps that use it"
)]
fn generated() -> impl Iterator<Item = Vec<u8>> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    const PICKS: [u8; 12] = [
        0, 0x01, 0x08, b'\t', b'\n', 0x0C, 0x1B, b'a', 0x7F, 0xEF, 0xFE, 0xFF,
    ];
    (0..2000).map(move |_| {
        let len = (next() % 400) as usize;
        (0..len)
            .map(|_| {
                let r = next();
                if r % 4 == 0 {
                    PICKS[(r >> 8) as usize % PICKS.len()]
                } else if r % 4 == 1 {
                    (r >> 8).to_le_bytes()[0]
                } else {
                    b' ' + (r >> 8).to_le_bytes()[0] % 95
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
    for ws in [b' ', b'\t', b'\n', 0x0B, 0x0C, b'\r', 0x08, 0x1B, 0x7F] {
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
        .chain([0x01, 0x02, b'\n', 0x7F])
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
/// each side builds its own options (C: by `ORing` the C constants into what
/// `git_blob_filter_options_init` filled; Rust: [`FilterOptions::new`] given a
/// [`FilterFlags`] collected from the flags and the commit id through its setters),
/// and filters the same blobs of the `crlf.git` fixture under the same
/// paths into its own buffer, one call after another without clearing it. After every
/// call the return codes and the buffers are compared: [`FilterFlag::CheckForBinary`]
/// leaves the buffer as the previous call left it for a binary blob, and the
/// attribute flags change which conversions apply. The safe wrapper of
/// `git_blob_filter` itself belongs to the `object` symbol batches.
mod filter_flags {
    use std::ffi::{CStr, CString, c_int};
    use std::ptr;
    use std::sync::OnceLock;

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::blob::{
        FilterFlag, FilterFlags, FilterOptions, UnnamedFilterFlag,
    };
    use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};

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
        let rc = unsafe { ffi::git_oid_from_string(&raw mut out, hex.as_ptr(), ffi::GIT_OID_SHA1) };
        assert_eq!(rc, 0);
        out
    }

    /// One side's own `crlf.git`, opened read-only, and the blobs it looks up.
    struct Side {
        repo: *mut ffi::git_repository,
        blobs: Vec<*mut ffi::git_blob>,
        buf: ffi::git_buf,
    }

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

    /// Runs `body` alone in a child process. libgit2 marks a filter initialised on its
    /// first load under the registry's read lock only (`filter_initialize`), so a
    /// load racing with another module's first load (the checkout and index fixtures
    /// add files through raw calls) is a data race (`TSan`); only a process of its own
    /// orders every load after the first.
    fn isolated(test: &str, body: fn()) {
        let name = test.strip_prefix("equiv::").unwrap_or(test);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    impl Side {
        fn open() -> Self {
            // Within the isolated child, one side still runs every call alone first,
            // loading the CRLF and ident filters before the sides run concurrently.
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
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/c/tests/resources/crlf.git\0");
            let mut repo = ptr::null_mut();
            // SAFETY: `repo` receives the repository; `path` is NUL-terminated.
            let rc = unsafe { ffi::git_repository_open_bare(&raw mut repo, path.as_ptr().cast()) };
            assert_eq!(rc, 0);
            let blobs = CALLS
                .iter()
                .map(|(id, _)| {
                    let mut blob = ptr::null_mut();
                    let id = oid(id);
                    // SAFETY: `repo` is open; `blob` receives a blob this side frees.
                    let ret = unsafe { ffi::git_blob_lookup(&raw mut blob, repo, &raw const id) };
                    assert_eq!(ret, 0);
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
                ffi::git_blob_filter(&raw mut self.buf, self.blobs[i], CALLS[i].1.as_ptr(), opts)
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
            unsafe { ffi::git_buf_dispose(&raw mut self.buf) };
            // SAFETY: as above.
            unsafe {
                for &blob in &self.blobs {
                    ffi::git_blob_free(blob);
                }
            }
            // SAFETY: as above.
            unsafe { ffi::git_repository_free(self.repo) };
        }
    }

    /// Options as `git_blob_filter_options_init` fills them, with the commit
    /// `ATTRIBUTES_FROM_COMMIT` reads.
    fn c_options() -> ffi::git_blob_filter_options {
        // SAFETY: `git_blob_filter_options` is plain data, all-zero valid.
        let mut opts: ffi::git_blob_filter_options = unsafe { std::mem::zeroed() };
        // SAFETY: `opts` is writable; the version is this header's.
        let rc = unsafe {
            ffi::git_blob_filter_options_init(&raw mut opts, ffi::GIT_BLOB_FILTER_OPTIONS_VERSION)
        };
        assert_eq!(rc, 0);
        opts.attr_commit_id = oid(IDENT_COMMIT);
        opts
    }

    /// Options built through [`FilterOptions`]: the defaults, `flags`, and the commit
    /// `ATTRIBUTES_FROM_COMMIT` reads, as the value C reads.
    fn rust_options(flags: FilterFlags) -> ffi::git_blob_filter_options {
        let mut opts = FilterOptions::new();
        let mut m = opts.as_mut();
        m.set_flags(flags);
        let commit = Oid::from_string(IDENT_COMMIT, OidType::Sha1).unwrap();
        m.set_attr_commit_id(commit.as_ref());
        opts.into()
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
        isolated(
            concat!(
                module_path!(),
                "::git_blob_filter_reads_rust_sets_as_c_flags"
            ),
            git_blob_filter_reads_rust_sets_as_c_flags_body,
        );
    }

    fn git_blob_filter_reads_rust_sets_as_c_flags_body() {
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
                let mut rust_opts = rust_options(set);

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
        isolated(
            concat!(
                module_path!(),
                "::rust_sets_change_what_git_blob_filter_does"
            ),
            rust_sets_change_what_git_blob_filter_does_body,
        );
    }

    fn rust_sets_change_what_git_blob_filter_does_body() {
        let mut side = Side::open();
        let mut opts = c_options();

        opts.flags = FilterFlags::empty().bits();
        let (rc, binary_unchecked) = side.filter(1, &mut opts);
        assert_eq!(rc, 0);
        assert!(!binary_unchecked.is_empty(), "{binary_unchecked:?}");
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

    /// [`BlobRef::filter`] against `git_blob_filter`: each side filters its own blobs of
    /// its own `crlf.git` through every flag combination, with options and without, and
    /// with versions libgit2 refuses, comparing the code, the bytes and the thread's
    /// error after every call.
    ///
    /// [`BlobRef::filter`]: libgit2::libgit2::src::libgit2::blob::BlobRef::filter
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn blob_filter_matches_git_blob_filter() {
        isolated(
            concat!(module_path!(), "::blob_filter_matches_git_blob_filter"),
            blob_filter_matches_git_blob_filter_body,
        );
    }

    /// A call's outcome: the code, the bytes, and the error left (for a failure).
    type Filtered = (c_int, Vec<u8>, Option<(c_int, String)>);

    fn c_filtered(
        side: &mut Side,
        i: usize,
        opts: Option<&mut ffi::git_blob_filter_options>,
    ) -> Filtered {
        // SAFETY: as in `Side::filter`; NULL options are the defaults.
        let rc = unsafe {
            ffi::git_blob_filter(
                &raw mut side.buf,
                side.blobs[i],
                CALLS[i].1.as_ptr(),
                opts.map_or(ptr::null_mut(), |o| o),
            )
        };
        let out = if rc < 0 || side.buf.ptr.is_null() {
            Vec::new()
        } else {
            // SAFETY: libgit2 left `size` initialised bytes at `ptr`.
            unsafe { std::slice::from_raw_parts(side.buf.ptr.cast::<u8>(), side.buf.size) }.to_vec()
        };
        // SAFETY: the side's buffer, emptied for the next call.
        unsafe { ffi::git_buf_dispose(&raw mut side.buf) };
        (rc, out, (rc < 0).then(super::c_last))
    }

    fn blob_filter_matches_git_blob_filter_body() {
        use libgit2::libgit2::src::libgit2::blob::Blob;
        use libgit2::libgit2::src::libgit2::repository::Repository;

        let mut c = Side::open();
        let path = CString::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/c/tests/resources/crlf.git"
        ))
        .unwrap();
        let repo = Repository::open(&path).unwrap();
        let blobs: Vec<_> = CALLS
            .iter()
            .map(|(id, _)| {
                let id = Oid::from_string(id, OidType::Sha1).unwrap();
                Blob::lookup(repo.as_ref(), id.as_ref()).unwrap()
            })
            .collect();
        let rust_filtered = |i: usize, opts: Option<&FilterOptions>| -> Filtered {
            match blobs[i]
                .as_ref()
                .filter(CALLS[i].1, opts.map(FilterOptions::as_ref))
            {
                Ok(buf) => (0, buf.as_ref().as_bytes().to_vec(), None),
                Err(e) => (
                    e.raw_code(),
                    Vec::new(),
                    Some((e.raw_class(), e.message().to_string_lossy().into_owned())),
                ),
            }
        };
        let mut compared = 0;
        for i in 0..CALLS.len() {
            assert_eq!(
                rust_filtered(i, None),
                c_filtered(&mut c, i, None),
                "call {i}"
            );
        }
        for combo in 0..1u32 << FLAGS.len() {
            let set: FilterFlags = (0..FLAGS.len())
                .filter(|k| combo & (1 << k) != 0)
                .map(|k| FLAGS[k].1)
                .collect();
            let mut c_opts = c_options();
            c_opts.flags = set.bits() | UNNAMED;
            let mut rust_opts = FilterOptions::new();
            rust_opts
                .as_mut()
                .set_flags(set | FilterFlags::from_bits_retain(UNNAMED));
            let commit = Oid::from_string(IDENT_COMMIT, OidType::Sha1).unwrap();
            rust_opts.as_mut().set_attr_commit_id(commit.as_ref());
            for i in 0..CALLS.len() {
                let expected = c_filtered(&mut c, i, Some(&mut c_opts));
                assert_eq!(
                    rust_filtered(i, Some(&rust_opts)),
                    expected,
                    "{set:?} call {i}"
                );
                compared += 1;
            }
        }
        for version in [
            0,
            ffi::GIT_BLOB_FILTER_OPTIONS_VERSION.cast_signed() + 1,
            -1,
        ] {
            let mut c_opts = c_options();
            c_opts.version = version;
            let mut rust_opts = FilterOptions::new();
            rust_opts.as_mut().set_version(version);
            let expected = c_filtered(&mut c, 0, Some(&mut c_opts));
            assert!(expected.0 < 0);
            assert_eq!(
                rust_filtered(0, Some(&rust_opts)),
                expected,
                "version {version}"
            );
        }
        assert_eq!(compared, CALLS.len() << FLAGS.len());
    }
}

/// The blob owner and its constructors against the C routines they wrap.
///
/// [`Blob::lookup`] must find what `git_blob_lookup` finds and [`Blob::lookup_prefix`]
/// what `git_blob_lookup_prefix` does: the same blob or the same failure (code, error
/// class and message). A blob is compared as each side reads it: C through
/// `git_blob_id`, `git_blob_rawcontent`, `git_blob_rawsize`, `git_blob_is_binary` and
/// `git_blob_owner`, Rust through [`BlobRef::id`], [`BlobRef::content`],
/// [`BlobRef::size`], [`BlobRef::is_binary`] and [`BlobRef::owner`]. Each side opens
/// `testrepo.git` on its own and runs one script of lookups, in order: every blob by
/// full id and by prefix, prefixes of 0 to past 40 digits, ids of a commit, a tree and
/// a tag, ambiguous, missing and zero ids, and repeats. After every step the sides are
/// compared, and so is which earlier result each new blob is the same blob as. Then
/// each side copies every result (C: `git_blob_dup`; Rust: `clone` and
/// [`BlobRef::to_blob`] in turn), releases the originals, and compares the copies.
/// The same script runs with blobs cached (an isolated child sets the process-wide
/// limit), where lookups of one id share a blob, and against a repository without a git
/// directory.
///
/// Objects of every type convert to a blob, [`Blob::from_object`], exactly when C's
/// `git_object_type` says the object is one, and the blob reads as C reads the object
/// cast to `git_blob *`.
///
/// The accessors also run on blobs of every size around the 8000 bytes
/// `git_blob_is_binary` reads, binary or not, written into a repository of each side's
/// own, in [`contents_match`].
///
/// Documented divergence, outside these comparisons: an id of another type than the
/// repository's is refused by the wrappers (code `-1`, `GIT_ERROR_INVALID`), where the
/// C routines format it out of bounds; the unit tests of `blob.rs` hold the refusal.
mod owner {
    use std::ffi::{CStr, CString, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::blob::{Blob, BlobArc, BlobRef};
    use libgit2::libgit2::src::libgit2::object::Object;
    use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox, RepositoryError};

    use super::{c_last, init};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/testrepo.git"
    );
    const README: &str = "a8233120f6ad708f843d861ce2b7228ec4e3dec6";
    const BRANCH_FILE: &str = "3697d64be941a53d4ae8f6a271e4e3fa56b022cc";
    const NEW_TXT: &str = "a71586c1dfe8a71c6cbf6c129f404c5642ff31bd";
    const COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
    const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
    const TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";

    /// What a lookup gave: the blob's id, type, content, size, binary verdict and
    /// repository path, as each side reads them, or the failure.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Outcome {
        Found(String, ffi::git_object_t, Vec<u8>, Facts),
        Failed(c_int, c_int, String),
    }

    /// The size, binary verdict and repository path of a blob.
    type Facts = (u64, bool, Option<String>);

    /// The Rust side's view of a live blob, through its handle's getters.
    fn rust_found(blob: BlobRef<'_, '_>) -> Outcome {
        let path = blob
            .owner()
            .path()
            .map(|p| p.to_string_lossy().into_owned());
        Outcome::Found(
            blob.id().to_hex_string().unwrap(),
            blob.as_object().kind().raw(),
            blob.content().to_vec(),
            (blob.size(), blob.is_binary(), path),
        )
    }

    /// C's view of a live blob.
    fn found(p: *const ffi::git_blob) -> Outcome {
        let mut out = [0u8; 65];
        {
            let object = p.cast::<ffi::git_object>();
            // SAFETY: `p` is a live blob; its id and content live as long as it and are
            // copied at once; `out` has 65 writable bytes, NUL included.
            let object_id = unsafe { ffi::git_object_id(object) };
            // SAFETY: as above.
            unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), object_id) };
            // SAFETY: as above.
            let size = unsafe { usize::try_from(ffi::git_blob_rawsize(p)).unwrap() };
            let data = if size == 0 {
                Vec::new()
            } else {
                // SAFETY: as above.
                let content = unsafe { ffi::git_blob_rawcontent(p) };
                // SAFETY: as above, the content holds `size` bytes; they are copied.
                unsafe { std::slice::from_raw_parts(content.cast::<u8>(), size) }.to_vec()
            };
            // SAFETY: as above.
            let object_type = unsafe { ffi::git_object_type(object) };
            // SAFETY: as above.
            let blob_rawsize = unsafe { ffi::git_blob_rawsize(p) };
            // SAFETY: as above.
            let blob_is_binary = unsafe { ffi::git_blob_is_binary(p) };
            // SAFETY: as above.
            let blob_owner = unsafe { ffi::git_blob_owner(p) };
            // SAFETY: as above.
            let repository_path = unsafe { ffi::git_repository_path(blob_owner) };
            // SAFETY: as above.
            unsafe {
                Outcome::Found(
                    CStr::from_bytes_until_nul(&out)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                    object_type,
                    data,
                    (blob_rawsize, blob_is_binary != 0, {
                        let path = repository_path;
                        (!path.is_null())
                            .then(|| CStr::from_ptr(path).to_string_lossy().into_owned())
                    }),
                )
            }
        }
    }

    /// One lookup of the script: an id (or prefix) in hex, and a length (`None` for
    /// the full lookup).
    type Step = (&'static str, Option<usize>);

    fn script() -> Vec<Step> {
        let mut steps = vec![
            (README, None),
            (BRANCH_FILE, None),
            (NEW_TXT, Some(7)),
            (README, Some(40)),
            (README, None),
            (COMMIT, None),
            (TREE, Some(10)),
            (TAG, None),
            ("3d35", Some(4)),
            ("dededededededededededededededededededede", None),
            ("0000000000000000000000000000000000000000", None),
            ("de", Some(2)),
        ];
        for len in [0, 1, 3, 4, 5, 7, 20, 39, 40, 41, 64, usize::MAX] {
            steps.push((README, Some(len)));
        }
        steps.push((BRANCH_FILE, Some(12)));
        steps.push((README, None));
        steps
    }

    /// The C side: the raw repository and the blobs it looked up.
    struct CSide {
        repo: *mut ffi::git_repository,
        held: Vec<*mut ffi::git_blob>,
    }

    impl CSide {
        fn open() -> Self {
            let path = CString::new(TESTREPO).unwrap();
            let mut repo = ptr::null_mut();
            // SAFETY: a writable slot and a NUL-terminated path.
            let rc = unsafe { ffi::git_repository_open(&raw mut repo, path.as_ptr()) };
            assert_eq!(rc, 0);
            Self {
                repo,
                held: Vec::new(),
            }
        }

        fn pathless() -> Self {
            let mut repo = ptr::null_mut();
            // SAFETY: a writable slot.
            assert_eq!(unsafe { ffi::git_repository_new(&raw mut repo) }, 0);
            Self {
                repo,
                held: Vec::new(),
            }
        }

        fn run(&mut self, (hex, len): Step) -> (Outcome, Option<usize>) {
            let mut id = ffi::git_oid::default();
            // SAFETY: a writable id, and `hex.len()` readable bytes of hex digits.
            let rc = unsafe {
                ffi::git_oid_from_prefix(
                    &raw mut id,
                    hex.as_ptr().cast(),
                    hex.len(),
                    ffi::GIT_OID_SHA1,
                )
            };
            assert_eq!(rc, 0);
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot, the live repository, and a live SHA-1 id, the
            // repository's type.
            let rc = match len {
                // SAFETY: as above.
                None => unsafe { ffi::git_blob_lookup(&raw mut out, self.repo, &raw const id) },
                Some(len) => {
                    // SAFETY: as above.
                    unsafe {
                        ffi::git_blob_lookup_prefix(&raw mut out, self.repo, &raw const id, len)
                    }
                }
            };
            if rc < 0 {
                assert!(out.is_null());
                let (class, message) = c_last();
                return (Outcome::Failed(rc, class, message), None);
            }
            let same = self.held.iter().position(|&h| h == out);
            self.held.push(out);
            (found(out), same)
        }

        /// Copies every blob held, releases the originals, and reads the copies.
        fn copy_and_release(&mut self) -> Vec<Outcome> {
            let copies: Vec<_> = self
                .held
                .iter()
                .map(|&b| {
                    let mut out = ptr::null_mut();
                    // SAFETY: a writable slot and a live blob this side holds.
                    assert_eq!(unsafe { ffi::git_blob_dup(&raw mut out, b) }, 0);
                    assert_eq!(out, b);
                    out
                })
                .collect();
            for b in std::mem::replace(&mut self.held, copies) {
                // SAFETY: a reference this side holds, released once.
                unsafe { ffi::git_blob_free(b) };
            }
            self.held.iter().map(|&b| found(b)).collect()
        }
    }

    impl Drop for CSide {
        fn drop(&mut self) {
            // SAFETY: each blob reference is this side's and released once, before the
            // repository.
            unsafe {
                for &b in &self.held {
                    ffi::git_blob_free(b);
                }
            }
            // SAFETY: as above.
            unsafe { ffi::git_repository_free(self.repo) };
        }
    }

    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "the closure lets each handle reborrow for its own lifetime, which the method path cannot"
    )]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the helper consumes what it reports, as its callers hand it over"
    )]
    fn rust_failed(e: RepositoryError) -> Outcome {
        Outcome::Failed(
            e.raw_code(),
            e.class().map_or(0, |c| c.raw()),
            e.message().to_string_lossy().into_owned(),
        )
    }

    /// The Rust side: one step against `repo`, holding what it finds.
    fn rust_run<'r>(
        repo: &'r RepositoryBox,
        held: &mut Vec<BlobArc<'r>>,
        (hex, len): Step,
    ) -> (Outcome, Option<usize>) {
        let id = Oid::from_prefix(hex.as_bytes(), OidType::Sha1).unwrap();
        let result = match len {
            None => Blob::lookup(repo.as_ref(), id.as_ref()),
            Some(len) => Blob::lookup_prefix(repo.as_ref(), id.as_ref(), len),
        };
        match result {
            Err(e) => (rust_failed(e), None),
            Ok(blob) => {
                let same = held.iter().position(|h| BlobArc::ptr_eq(h, &blob));
                let outcome = rust_found(blob.as_ref());
                held.push(blob);
                (outcome, same)
            }
        }
    }

    /// Runs the script on both sides, comparing after every step and after the copies;
    /// how many found blobs were one found before.
    fn compare(mut c: CSide, rust: &RepositoryBox) -> usize {
        let mut held = Vec::new();
        let mut shared = 0;
        for step in script() {
            let expected = c.run(step);
            let got = rust_run(rust, &mut held, step);
            assert_eq!(got, expected, "{step:?}");
            shared += usize::from(got.1.is_some());
        }
        let expected = c.copy_and_release();
        let copies: Vec<_> = held
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if i % 2 == 0 {
                    b.clone()
                } else {
                    b.as_ref().to_blob()
                }
            })
            .collect();
        for (copy, original) in copies.iter().zip(&held) {
            assert!(BlobArc::ptr_eq(copy, original));
        }
        drop(held);
        let got: Vec<_> = copies.iter().map(|b| rust_found(b.as_ref())).collect();
        assert_eq!(got, expected);
        shared
    }

    fn open_rust() -> RepositoryBox {
        Repository::open(&CString::new(TESTREPO).unwrap()).unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_match() {
        init();
        // The cache keeps no blob by default: every lookup parses a fresh one.
        assert_eq!(compare(CSide::open(), &open_rust()), 0);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_without_a_git_directory_match() {
        init();
        assert_eq!(compare(CSide::pathless(), &Repository::new().unwrap()), 0);
    }

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

    /// Runs `body` alone in a child process: it sets a process-wide cache option.
    fn isolated(test: &str, body: fn()) {
        let name = test.strip_prefix("equiv::").unwrap_or(test);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// With blobs cached, lookups of one id hand back the cache's blob, on both sides.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn cached_lookups_match() {
        isolated(concat!(module_path!(), "::cached_lookups_match"), || {
            init();
            // SAFETY: an `int` object type and a `size_t`; this child runs nothing else
            // meanwhile.
            let rc = unsafe {
                ffi::git_libgit2_opts(
                    ffi::GIT_OPT_SET_CACHE_OBJECT_LIMIT.cast_signed(),
                    ffi::GIT_OBJECT_BLOB,
                    1usize << 20,
                )
            };
            assert_eq!(rc, 0);
            let (c, rust) = (CSide::open(), open_rust());
            assert!(compare(c, &rust) > 0);
        });
    }

    /// Objects of every type convert to a blob exactly when C says they are one.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn object_conversions_match() {
        init();
        let c = CSide::open();
        let rust = open_rust();
        for hex in [README, NEW_TXT, COMMIT, TREE, TAG] {
            let mut id = ffi::git_oid::default();
            let mut object = ptr::null_mut();
            let expected = {
                // SAFETY: a writable id and slot, `hex` 40 hex digits, the live repository.
                let rc = unsafe {
                    ffi::git_oid_from_string(
                        &raw mut id,
                        CString::new(hex).unwrap().as_ptr(),
                        ffi::GIT_OID_SHA1,
                    )
                };
                assert_eq!(rc, 0);
                // SAFETY: as above.
                let rc = unsafe {
                    ffi::git_object_lookup(
                        &raw mut object,
                        c.repo,
                        &raw const id,
                        ffi::GIT_OBJECT_ANY,
                    )
                };
                assert_eq!(rc, 0);
                // SAFETY: as above.
                let view = unsafe {
                    (ffi::git_object_type(object) == ffi::GIT_OBJECT_BLOB)
                        .then(|| found(object.cast()))
                };
                // SAFETY: as above.
                unsafe { ffi::git_object_free(object) };
                view
            };
            let oid = Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap();
            let object = Object::lookup(rust.as_ref(), oid.as_ref(), None).unwrap();
            let got = match Blob::from_object(object) {
                Ok(blob) => {
                    let view = rust_found(blob.as_ref());
                    let p = blob.as_c_ptr();
                    let back = Blob::into_object(blob);
                    assert_eq!(back.as_c_ptr().cast::<ffi::git_blob>(), p);
                    Some(view)
                }
                Err(_) => None,
            };
            assert_eq!(got, expected, "{hex}");
        }
    }
    /// Blobs written with a NUL or a long non-printable run before, at and after the
    /// 8000 bytes `git_blob_is_binary` reads, and empty ones, read alike by both sides,
    /// each from its own repository, twice in a row.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn contents_match() {
        init();
        let dirs: Vec<_> = ["c", "rust"]
            .iter()
            .map(|side| {
                let dir = std::env::temp_dir().join(format!(
                    "crustify-equiv-blob-contents-{}-{side}",
                    std::process::id()
                ));
                drop(std::fs::remove_dir_all(&dir));
                dir
            })
            .collect();
        let mut c_repo = ptr::null_mut();
        let c_path = CString::new(dirs[0].to_str().unwrap()).unwrap();
        // SAFETY: a writable slot and a NUL-terminated path.
        let rc = unsafe { ffi::git_repository_init(&raw mut c_repo, c_path.as_ptr(), 1) };
        assert_eq!(rc, 0);
        let rust =
            Repository::init(&CString::new(dirs[1].to_str().unwrap()).unwrap(), true).unwrap();
        let mut inputs: Vec<Vec<u8>> = vec![Vec::new(), b"text\n".to_vec()];
        for at in [0, 7999, 8000, 8001, 20_000] {
            let mut nul = vec![b'a'; 20_001];
            nul[at] = 0;
            inputs.push(nul);
            let mut control = vec![b'a'; at];
            control.extend(std::iter::repeat_n(1u8, 200));
            inputs.push(control);
        }
        for data in &inputs {
            let mut id = ffi::git_oid::default();
            // SAFETY: a writable id, the live repository and `data.len()` bytes.
            let rc = unsafe {
                ffi::git_blob_create_from_buffer(
                    &raw mut id,
                    c_repo,
                    data.as_ptr().cast(),
                    data.len(),
                )
            };
            assert_eq!(rc, 0);
            let mut blob = ptr::null_mut();
            // SAFETY: a writable slot, the live repository and an id of its type.
            let ret = unsafe { ffi::git_blob_lookup(&raw mut blob, c_repo, &raw const id) };
            assert_eq!(ret, 0);
            let mut expected = found(blob);
            // SAFETY: the blob, released once.
            unsafe { ffi::git_blob_free(blob) };
            // The repository paths differ by design: compare their file names only.
            if let Outcome::Found(_, _, _, (_, _, path)) = &mut expected {
                *path = path.as_ref().map(|p| p.replace("-c/", "-rust/"));
            }
            let rid = libgit2::libgit2::src::libgit2::blob::create_from_buffer(rust.as_ref(), data)
                .unwrap();
            let rblob = Blob::lookup(rust.as_ref(), rid.as_ref()).unwrap();
            assert_eq!(rust_found(rblob.as_ref()), expected, "{}", data.len());
            assert_eq!(rust_found(rblob.as_ref()), expected);
        }
        drop(rust);
        // SAFETY: the C side's repository, released once.
        unsafe { ffi::git_repository_free(c_repo) };
        for dir in dirs {
            drop(std::fs::remove_dir_all(dir));
        }
    }
}

/// The filter options against `git_blob_filter_options_init` and the struct C fills.
///
/// [`FilterOptions::new`] must be what `git_blob_filter_options_init` writes for the
/// header's version, field by field, and every setter must leave the struct as C's
/// assignment to the same field does: each side holds its own options and applies one
/// script of writes (versions, flag sets with unnamed bits, commit ids of every type,
/// a `reserved` pointer set and cleared), comparing the raw structs after every step.
/// `git_blob_filter_options_init` refuses every other version, with libgit2's error,
/// and leaves the struct alone; the Rust constructor takes no version, so there is
/// nothing to compare for those.
mod filter_options {
    use std::ffi::c_int;
    use std::ptr::NonNull;

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::blob::{FilterFlags, FilterOptions};
    use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};

    use super::{c_last, init};

    /// The fields of a raw struct, `reserved` by address.
    fn fields(o: &ffi::git_blob_filter_options) -> (c_int, u32, usize, u8, [u8; 32]) {
        (
            o.version,
            o.flags,
            o.reserved as usize,
            o.attr_commit_id.type_,
            o.attr_commit_id.id,
        )
    }

    fn c_init(version: u32) -> (c_int, ffi::git_blob_filter_options) {
        let mut opts = ffi::git_blob_filter_options {
            version: 0x5A5A,
            flags: 0xA5A5,
            reserved: NonNull::<u8>::dangling().as_ptr().cast(),
            attr_commit_id: ffi::git_oid::default(),
        };
        // SAFETY: `opts` is a live, writable struct.
        let rc = unsafe { ffi::git_blob_filter_options_init(&raw mut opts, version) };
        (rc, opts)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_git_blob_filter_options_init() {
        init();
        let (rc, c) = c_init(ffi::GIT_BLOB_FILTER_OPTIONS_VERSION);
        assert_eq!(rc, 0);
        let rust: ffi::git_blob_filter_options = FilterOptions::new().into();
        assert_eq!(fields(&rust), fields(&c));
        assert_eq!(FilterOptions::VERSION, c.version);
        assert_eq!(FilterOptions::default(), FilterOptions::from(c));
        // Other versions are refused, and leave the struct untouched; libgit2 prints
        // the version as an `int`.
        for (version, shown) in [(0, "0"), (2, "2"), (u32::MAX, "-1")] {
            let (rc, untouched) = c_init(version);
            assert_eq!(rc, -1);
            assert_eq!((untouched.version, untouched.flags), (0x5A5A, 0xA5A5));
            let message = format!("invalid version {shown} on git_blob_filter_options");
            assert_eq!(c_last(), (ffi::GIT_ERROR_INVALID.cast_signed(), message));
        }
    }

    /// One write of the script.
    #[derive(Clone, Copy, Debug)]
    enum Write {
        Version(c_int),
        Flags(u32),
        Id(u8, u8),
        Reserved,
        ClearReserved,
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn setters_match_c_field_writes() {
        init();
        let script = [
            Write::Flags(0),
            Write::Version(2),
            Write::Flags(u32::MAX),
            Write::Id(1, 0xAB),
            Write::Reserved,
            Write::Version(-1),
            Write::Flags(0x8000_0009),
            Write::Id(2, 0x5A),
            Write::ClearReserved,
            Write::Id(0, 0),
            Write::Version(1),
            Write::Flags(1),
        ];
        let (_, mut c) = c_init(ffi::GIT_BLOB_FILTER_OPTIONS_VERSION);
        let mut rust = FilterOptions::new();
        let mut slot = 0u8;
        let reserved = std::ptr::from_mut(&mut slot).cast();
        for step in script {
            match step {
                Write::Version(v) => {
                    c.version = v;
                    rust.as_mut().set_version(v);
                }
                Write::Flags(f) => {
                    c.flags = f;
                    rust.as_mut().set_flags(FilterFlags::from_bits_retain(f));
                }
                Write::Id(kind, byte) => {
                    c.attr_commit_id.type_ = kind;
                    c.attr_commit_id.id = [byte; 32];
                    let mut id = Oid::zero(OidType::Sha1);
                    id.as_mut().set_raw_kind(kind);
                    id.as_mut().set_id([byte; 32]);
                    rust.as_mut().set_attr_commit_id(id.as_ref());
                }
                Write::Reserved => {
                    // Only a raw value can carry a pointer: both sides take C's.
                    c.reserved = reserved;
                    let mut raw: ffi::git_blob_filter_options = rust.into();
                    raw.reserved = reserved;
                    rust = raw.into();
                }
                Write::ClearReserved => {
                    c.reserved = std::ptr::null_mut();
                    rust.as_mut().clear_reserved();
                }
            }
            let got: ffi::git_blob_filter_options = rust.into();
            assert_eq!(fields(&got), fields(&c), "{step:?}");
            assert_eq!(rust.as_ref().has_reserved(), !c.reserved.is_null());
            assert_eq!(rust.as_ref().flags().bits(), c.flags);
            assert_eq!(rust.as_ref().version(), c.version);
            assert_eq!(rust.as_ref().attr_commit_id().id(), c.attr_commit_id.id);
        }
    }
}

/// Equivalence of the blob writers with the C routines they wrap:
/// `git_blob_create_from_buffer`, `git_blob_create_from_workdir`,
/// `git_blob_create_from_disk`, and `git_blob_create_from_stream` with
/// `git_blob_create_from_stream_commit`.
///
/// Each side builds its own repository, a working directory with the same
/// `.gitattributes` (CRLF and `ident` filters) and files, and runs one script of writes:
/// buffers of every shape, working-directory files (filtered, unfiltered, nested, a
/// symbolic link, a directory, missing ones), files on disk inside and outside the
/// working directory, and streams written in pieces past their 2 MiB buffer, with and
/// without a hint path, committed or dropped. C writes through the raw calls, Rust
/// through the wrappers; after every step the ids, or the codes, error classes and
/// messages (each side's directory spelled `<dir>`), are compared. A bare repository
/// and one without paths run the script again on both sides.
///
/// Documented divergence, outside these comparisons: an absolute path that would select
/// filters (`create_from_workdir`'s path, `create_from_stream`'s hint) is refused by
/// the wrappers with the error a release build of libgit2 reports from its assertion,
/// where a debug build aborts (the coverage build is one); the unit tests of `blob.rs`
/// hold the refusal.
mod writing {
    use std::ffi::{CStr, CString, c_int};
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::blob::{
        create_from_buffer, create_from_disk, create_from_stream, create_from_workdir,
    };
    use libgit2::libgit2::src::libgit2::oid::Oid;
    use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox, RepositoryError};

    use super::init;

    /// A write: its id in hex, or its code, error class and message.
    type Outcome = Result<String, (c_int, c_int, String)>;

    /// One write of the script.
    #[derive(Clone, Copy, Debug)]
    enum Step {
        Buffer(&'static [u8]),
        Big(usize),
        Workdir(&'static str),
        /// A path relative to the working directory, written as an absolute one.
        DiskInside(&'static str),
        /// A file outside the working directory, with the bytes given.
        DiskOutside(&'static [u8]),
        Stream(Option<&'static str>, &'static [&'static [u8]]),
        StreamBig(usize),
        StreamDropped,
    }

    fn script() -> Vec<Step> {
        use Step::*;
        vec![
            Buffer(b""),
            Buffer(b"hello\n"),
            Buffer(b"a\0b\r\n"),
            Big(65_537),
            Big(3 << 20),
            Workdir("a.txt"),
            Workdir("sub/b.id"),
            Workdir("c.bin"),
            Workdir("link"),
            Workdir("sub"),
            Workdir("missing.txt"),
            Workdir("sub/../a.txt"),
            DiskInside("a.txt"),
            DiskInside("sub/b.id"),
            DiskInside("c.bin"),
            DiskInside("sub"),
            DiskInside("missing.txt"),
            DiskOutside(b"one\r\ntwo\r\n"),
            Stream(None, &[]),
            Stream(None, &[b"hello", b"\n"]),
            Stream(Some("a.txt"), &[b"one\r\n", b"two\r\n"]),
            Stream(Some("x.id"), &[b"$Id: old $\n"]),
            Stream(Some("missing/dir.txt"), &[b"x\r\n"]),
            StreamBig(3 << 20),
            StreamDropped,
            Buffer(b"hello\n"),
        ]
    }

    /// One side's repositories: a working directory, and a file outside it.
    struct Side {
        root: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl Side {
        fn new(tag: &str, bare: bool) -> Self {
            init();
            let root = std::env::temp_dir()
                .join(format!("crustify-blob-equiv-{}-{tag}", std::process::id()));
            drop(std::fs::remove_dir_all(&root));
            let work = root.join("work");
            std::fs::create_dir_all(work.join("sub")).unwrap();
            let repo = Repository::init(&c_path(&work), bare).unwrap();
            std::fs::write(work.join(".gitattributes"), b"*.txt text\n*.id ident\n").unwrap();
            std::fs::write(work.join("a.txt"), b"one\r\ntwo\r\n").unwrap();
            std::fs::write(work.join("sub/b.id"), b"$Id: stale $\nbody\n").unwrap();
            std::fs::write(work.join("c.bin"), b"one\r\ntwo\r\n").unwrap();
            std::os::unix::fs::symlink("a.txt", work.join("link")).unwrap();
            Self {
                root,
                repo: Some(repo),
            }
        }

        fn pathless() -> Self {
            init();
            Self {
                root: PathBuf::from("/nonexistent/crustify-pathless"),
                repo: Some(Repository::new().unwrap()),
            }
        }

        fn raw(&self) -> *mut ffi::git_repository {
            self.repo.as_ref().unwrap().as_c_ptr()
        }

        fn inside(&self, rel: &str) -> CString {
            c_path(&self.root.join("work").join(rel))
        }

        fn outside(&self, data: &[u8]) -> CString {
            let path = self.root.join("outside.txt");
            if self.root.exists() {
                std::fs::write(&path, data).unwrap();
            }
            c_path(&path)
        }

        /// `outcome`'s message with this side's directory spelled `<dir>`, and the random
        /// suffix of a temporary file's name `<random>`.
        fn normalised(&self, outcome: Outcome) -> Outcome {
            let root = self.root.to_str().unwrap();
            outcome.map_err(|(code, class, message)| {
                let mut message = message.replace(root, "<dir>");
                if let Some(at) = message.find("streamed_git2_") {
                    let start = at + "streamed_git2_".len();
                    let len = message[start..]
                        .bytes()
                        .take_while(u8::is_ascii_alphanumeric)
                        .count();
                    message.replace_range(start..start + len, "<random>");
                }
                (code, class, message)
            })
        }
    }

    impl Drop for Side {
        fn drop(&mut self) {
            drop(self.repo.take());
            drop(std::fs::remove_dir_all(&self.root));
        }
    }

    fn c_path(path: &Path) -> CString {
        CString::new(path.to_str().unwrap()).unwrap()
    }

    fn big(len: usize) -> Vec<u8> {
        (0..len).map(|i| u8::try_from(i % 241).unwrap()).collect()
    }

    fn c_error(code: c_int) -> (c_int, c_int, String) {
        {
            // SAFETY: after init `git_error_last` never returns NULL, and its message is
            // NUL-terminated, both live until the next libgit2 call here, after the copy.
            let e = unsafe { ffi::git_error_last() };
            // SAFETY: as above.
            let klass = unsafe { (*e).klass };
            // SAFETY: as above.
            let message2 = unsafe { (*e).message };
            // SAFETY: as above.
            unsafe {
                (
                    code,
                    klass,
                    CStr::from_ptr(message2).to_string_lossy().into_owned(),
                )
            }
        }
    }

    fn c_done(code: c_int, id: &ffi::git_oid) -> Outcome {
        if code < 0 {
            return Err(c_error(code));
        }
        let mut out = [0u8; 65];
        // SAFETY: `id` is live; `out` has 65 writable bytes.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
        Ok(CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned())
    }

    fn rust_done(result: Result<Oid, RepositoryError>) -> Outcome {
        result
            .map(|id| id.as_ref().to_hex_string().unwrap())
            .map_err(|e| {
                (
                    e.raw_code(),
                    e.raw_class(),
                    e.message().to_string_lossy().into_owned(),
                )
            })
    }

    /// C's stream: opened on `side` with `hint`, written `pieces`, then committed.
    fn c_stream(side: &Side, hint: Option<&str>, pieces: &[&[u8]], commit: bool) -> Outcome {
        let hint = hint.map(|h| CString::new(h).unwrap());
        let mut stream = ptr::null_mut();
        // SAFETY: a writable slot, the live repository and a NULL or NUL-terminated
        // hint, copied.
        let rc = unsafe {
            ffi::git_blob_create_from_stream(
                &raw mut stream,
                side.raw(),
                hint.as_ref().map_or(ptr::null(), |h| h.as_ptr()),
            )
        };
        if rc < 0 {
            return Err(c_error(rc));
        }
        for piece in pieces {
            // SAFETY: the open stream's own `write` member, given a live slice.
            // SAFETY: as above, the open stream.
            let write = unsafe { (*stream).write }.unwrap();
            // SAFETY: as above, its own member, given `piece`, live for the call.
            let rc = unsafe { write(stream, piece.as_ptr().cast(), piece.len()) };
            assert_eq!(rc, 0);
        }
        if !commit {
            // SAFETY: the open stream's own `free` member, once.
            let free2 = unsafe { (*stream).free };
            // SAFETY: as above.
            unsafe { free2.unwrap()(stream) };
            return Ok(String::new());
        }
        let mut id = ffi::git_oid::default();
        // SAFETY: the open stream, consumed; a writable id.
        let rc = unsafe { ffi::git_blob_create_from_stream_commit(&raw mut id, stream) };
        c_done(rc, &id)
    }

    fn rust_stream(side: &Side, hint: Option<&str>, pieces: &[&[u8]], commit: bool) -> Outcome {
        let hint = hint.map(|h| CString::new(h).unwrap());
        let repo = side.repo.as_ref().unwrap().as_ref();
        let mut writer = match create_from_stream(repo, hint.as_deref()) {
            Ok(writer) => writer,
            Err(e) => return rust_done(Err(e)),
        };
        for piece in pieces {
            writer.write(piece).unwrap();
        }
        if !commit {
            drop(writer);
            return Ok(String::new());
        }
        rust_done(writer.commit())
    }

    fn c_run(side: &Side, step: Step) -> Outcome {
        let mut id = ffi::git_oid::default();
        let repo = side.raw();
        let outcome = match step {
            Step::Buffer(data) => {
                let data = data.to_vec();
                // SAFETY: a writable id, the live repository and a live buffer.
                let rc = unsafe {
                    ffi::git_blob_create_from_buffer(
                        &raw mut id,
                        repo,
                        data.as_ptr().cast(),
                        data.len(),
                    )
                };
                c_done(rc, &id)
            }
            Step::Big(len) => {
                let data = big(len);
                // SAFETY: as above.
                let rc = unsafe {
                    ffi::git_blob_create_from_buffer(
                        &raw mut id,
                        repo,
                        data.as_ptr().cast(),
                        data.len(),
                    )
                };
                c_done(rc, &id)
            }
            Step::Workdir(rel) => {
                let rel = CString::new(rel).unwrap();
                // SAFETY: a writable id, the live repository and a relative path.
                let rc =
                    unsafe { ffi::git_blob_create_from_workdir(&raw mut id, repo, rel.as_ptr()) };
                c_done(rc, &id)
            }
            Step::DiskInside(rel) => {
                let path = side.inside(rel);
                // SAFETY: a writable id, the live repository and a NUL-terminated path.
                let rc =
                    unsafe { ffi::git_blob_create_from_disk(&raw mut id, repo, path.as_ptr()) };
                c_done(rc, &id)
            }
            Step::DiskOutside(data) => {
                let path = side.outside(data);
                // SAFETY: as above.
                let rc =
                    unsafe { ffi::git_blob_create_from_disk(&raw mut id, repo, path.as_ptr()) };
                c_done(rc, &id)
            }
            Step::Stream(hint, pieces) => c_stream(side, hint, pieces, true),
            Step::StreamBig(len) => c_stream(side, None, &[&big(len)], true),
            Step::StreamDropped => c_stream(side, Some("a.txt"), &[b"discarded"], false),
        };
        side.normalised(outcome)
    }

    fn rust_run(side: &Side, step: Step) -> Outcome {
        let repo = side.repo.as_ref().unwrap().as_ref();
        let outcome = match step {
            Step::Buffer(data) => rust_done(create_from_buffer(repo, data)),
            Step::Big(len) => rust_done(create_from_buffer(repo, &big(len))),
            Step::Workdir(rel) => rust_done(create_from_workdir(repo, &CString::new(rel).unwrap())),
            Step::DiskInside(rel) => rust_done(create_from_disk(repo, &side.inside(rel))),
            Step::DiskOutside(data) => rust_done(create_from_disk(repo, &side.outside(data))),
            Step::Stream(hint, pieces) => rust_stream(side, hint, pieces, true),
            Step::StreamBig(len) => rust_stream(side, None, &[&big(len)], true),
            Step::StreamDropped => rust_stream(side, Some("a.txt"), &[b"discarded"], false),
        };
        side.normalised(outcome)
    }

    const CHILD: &str = "CRUSTIFY_EQUIV_BLOB_WRITING_CHILD";

    /// Runs `body` alone in a child process: the C side loads filter lists without the
    /// wrappers' ordering of libgit2's first, unsynchronised filter initialisation, so
    /// it must not race another test's first load (`TSan`).
    fn isolated(test: &str, body: fn()) {
        let name = test.strip_prefix("equiv::").unwrap_or(test);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn compare(c: Side, rust: Side) {
        compare_steps(c, rust, &script());
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the helper consumes what it reports, as its callers hand it over"
    )]
    fn compare_steps(c: Side, rust: Side, steps: &[Step]) {
        let mut failures = 0;
        for &step in steps {
            let (c_out, rust_out) = (c_run(&c, step), rust_run(&rust, step));
            failures += usize::from(c_out.is_err());
            assert_eq!(rust_out, c_out, "{step:?}");
        }
        assert!(failures > 0);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[cfg_attr(
        bsan,
        ignore = "its large inputs outlast the BorrowSanitizer gate, over an hour alone; the other variants run it"
    )]
    fn writes_match_c() {
        isolated(concat!(module_path!(), "::writes_match_c"), || {
            compare(Side::new("c", false), Side::new("rust", false));
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[cfg_attr(
        bsan,
        ignore = "its large inputs outlast the BorrowSanitizer gate, over an hour alone; the other variants run it"
    )]
    fn writes_into_bare_repositories_match_c() {
        isolated(
            concat!(module_path!(), "::writes_into_bare_repositories_match_c"),
            || compare(Side::new("bare-c", true), Side::new("bare-rust", true)),
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[cfg_attr(
        bsan,
        ignore = "its large inputs outlast the BorrowSanitizer gate, over an hour alone; the other variants run it"
    )]
    fn writes_without_paths_match_c() {
        isolated(
            concat!(module_path!(), "::writes_without_paths_match_c"),
            || compare(Side::pathless(), Side::pathless()),
        );
    }

    /// A bare repository whose `objects` directory is gone: no stream can open its
    /// temporary file. Only the streams are compared, which fail before loading the
    /// object database: libgit2 leaks the one it fails to load there
    /// (`git_repository_odb__weakptr` returns when reading the alternates fails without
    /// freeing it or its path), on both sides alike.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn writes_without_an_objects_directory_match_c() {
        isolated(
            concat!(
                module_path!(),
                "::writes_without_an_objects_directory_match_c"
            ),
            || {
                let (c, rust) = (Side::new("gone-c", true), Side::new("gone-rust", true));
                for side in [&c, &rust] {
                    std::fs::remove_dir_all(side.root.join("work/objects")).unwrap();
                }
                let streams: Vec<Step> = script()
                    .into_iter()
                    .filter(|s| {
                        matches!(
                            s,
                            Step::Stream(..) | Step::StreamBig(_) | Step::StreamDropped
                        )
                    })
                    .collect();
                compare_steps(c, rust, &streams);
            },
        );
    }
}
