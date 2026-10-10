//! UB tests for `git2/blob.h`: `data_is_binary` (`git_blob_data_is_binary`), the
//! filter flags in [`filter_flags`], the blob owner in [`owner`] and the filter options
//! in [`filter_options`].
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

/// The blob owner, `BlobArc<'r>`, its constructors `Blob::lookup` and
/// `Blob::lookup_prefix`, its conversions from and to `ObjectArc<'r>` and
/// `ObjectRef<'_, 'r>`, and its handles.
///
/// A blob is counted on its `git_object`, which `git_blob_dup` and `git_blob_free`
/// increment and decrement, and the repository's cache keeps one only when a blob
/// cache limit is set: every owner gives back its one reference exactly once, and
/// every clone, `to_blob` and conversion takes or moves exactly one. A miscount frees a
/// blob (and its reference to the object database's copy of the content) that another
/// owner or the cache still reads, which ASan flags when C reads the content back, or
/// leaks it, which LSan does. The tests share blobs between lookups, clones and object
/// conversions, drop them in every order, run with blobs cached and evicted under
/// their owners (isolated children), and fail every lookup they can: wrong type,
/// missing, ambiguous, no object database, and an id of another type than the
/// repository's, which must be refused before C formats it out of bounds. libgit2
/// copies the id it is given: the tests overwrite and free each id right after its
/// lookup (BSan). The content is read back from heap buffers of exactly its length,
/// empty and large included (ASan).
///
/// A blob is neither `Send` nor `Sync`, so there is no thread test (doctests); it, its
/// handles and its clones cannot outlive the repository, nor a handle its owner, nor a
/// blob borrowed from an object that object (`compile_fail/` and doctests).
///
/// Every test calls libgit2, so Miri skips them all.
mod owner {
    use std::ffi::CString;

    use libgit2::ffi;
    use libgit2::libgit2::object::blob::{Blob, BlobArc, BlobRef};
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryError};
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use crate::fixture::blob::{content, set_blob_cache_limit};
    use crate::fixture::object_and_tree_entry::{
        object_view, open_testrepo, repository, set_cache_max_size, write_raw,
    };
    use crate::fixture::{TempRepo, isolated};

    const COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
    const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
    /// `README` of `testrepo.git`, "hey there\n".
    const README: &str = "a8233120f6ad708f843d861ce2b7228ec4e3dec6";
    /// The blobs of `testrepo.git`'s history, and their contents.
    const BLOBS: [(&str, &[u8]); 4] = [
        (README, b"hey there\n"),
        ("3697d64be941a53d4ae8f6a271e4e3fa56b022cc", b"hi\nbye!\n"),
        ("a71586c1dfe8a71c6cbf6c129f404c5642ff31bd", b"my new file\n"),
        ("45b983be36b73c0788dc9cbcb76cbb80fc7bb057", b"hi\n"),
    ];

    fn id(hex: &str) -> Oid {
        Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap()
    }

    /// The hex id of the object a blob is, read by C.
    fn hex(blob: &BlobArc<'_>) -> String {
        object_view(&Blob::into_object(blob.clone())).0
    }

    fn assert_failed(result: Result<BlobArc<'_>, RepositoryError>, code: i32, class: ErrorClass) {
        let e = result.unwrap_err();
        assert_eq!((e.raw_code(), e.class()), (code, Some(class)), "{e:?}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_and_clones_count_their_own_reference() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        for (hex_id, data) in BLOBS {
            let full = Blob::lookup(r, id(hex_id).as_ref()).unwrap();
            let short = Blob::lookup_prefix(r, id(hex_id).as_ref(), 7).unwrap();
            // The cache keeps no blob by default: each lookup parses a fresh one.
            assert!(!BlobArc::ptr_eq(&full, &short));
            let clone = full.clone();
            let copy = short.as_ref().to_blob();
            assert!(BlobArc::ptr_eq(&clone, &full));
            assert!(BlobArc::ptr_eq(&copy, &short));
            drop((full, short));
            assert_eq!(content(&clone), data);
            assert_eq!(hex(&clone), hex_id);
            drop(clone);
            assert_eq!(content(&copy), data);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn clone_chains_release_every_reference_in_any_order() {
        let repo = open_testrepo();
        for order in 0..3 {
            let first = Blob::lookup(repo.as_ref(), id(README).as_ref()).unwrap();
            let mut chain = vec![first];
            for i in 0..16 {
                let last = chain.last().unwrap();
                let next = match i % 3 {
                    0 => last.clone(),
                    1 => last.as_ref().to_blob(),
                    _ => {
                        let object = Blob::into_object(last.clone());
                        Blob::from_object(object).unwrap()
                    }
                };
                chain.push(next);
            }
            match order {
                0 => chain.reverse(),
                1 => chain.rotate_left(7),
                _ => {}
            }
            while let Some(blob) = chain.pop() {
                assert_eq!(content(&blob), b"hey there\n");
                drop(blob);
                if let Some(survivor) = chain.first() {
                    assert_eq!(content(survivor), b"hey there\n");
                }
            }
        }
    }

    /// Objects of other types stay objects, untouched and still owned; an object of
    /// type blob becomes a blob, borrowed or owned, and back, on one count.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn object_conversions_move_one_reference() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        for other in [COMMIT, TREE] {
            let object = Object::lookup(r, id(other).as_ref(), None).unwrap();
            assert!(BlobRef::from_object(object.as_ref()).is_none());
            let object = Blob::from_object(object).unwrap_err();
            assert_eq!(object_view(&object).0, other);
        }
        let tree = Object::lookup(r, id(TREE).as_ref(), None).unwrap();
        let readme = tree.as_ref().lookup_bypath(c"README", None).unwrap();
        drop(tree);
        let borrowed = BlobRef::from_object(readme.as_ref()).unwrap();
        let owned = borrowed.to_blob();
        let object = borrowed.as_object().to_object();
        drop(readme);
        assert_eq!(content(&owned), b"hey there\n");
        let blob = Blob::from_object(object).unwrap();
        assert!(BlobArc::ptr_eq(&blob, &owned));
        drop(owned);
        let object = Blob::into_object(blob);
        assert_eq!(object_view(&object).0, README);
        let objects: Vec<ObjectArc<'_>> = (0..4).map(|_| object.clone()).collect();
        let blobs: Vec<_> = objects
            .into_iter()
            .map(|o| Blob::from_object(o).unwrap())
            .collect();
        drop(object);
        for blob in blobs.iter().rev() {
            assert_eq!(content(blob), b"hey there\n");
        }
    }

    /// Every failure leaves no blob behind (LSan), and its code and class.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failures_leave_nothing_behind() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        let not_found = ErrorCode::NotFound.raw();
        let ambiguous = ErrorCode::Ambiguous.raw();
        for other in [COMMIT, TREE] {
            assert_failed(
                Blob::lookup(r, id(other).as_ref()),
                not_found,
                ErrorClass::Invalid,
            );
            assert_failed(
                Blob::lookup_prefix(r, id(other).as_ref(), 7),
                not_found,
                ErrorClass::Invalid,
            );
        }
        assert_failed(
            Blob::lookup(r, id(&"de".repeat(20)).as_ref()),
            not_found,
            ErrorClass::Odb,
        );
        assert_failed(
            Blob::lookup(r, Oid::zero(OidType::Sha1).as_ref()),
            not_found,
            ErrorClass::Odb,
        );
        for len in [0, 1, 3] {
            assert_failed(
                Blob::lookup_prefix(r, id(README).as_ref(), len),
                ambiguous,
                ErrorClass::Object,
            );
        }
        // `3d35` starts two blobs.
        let prefix = Oid::from_prefix(b"3d35", OidType::Sha1).unwrap();
        assert_failed(
            Blob::lookup_prefix(r, prefix.as_ref(), 4),
            ambiguous,
            ErrorClass::Odb,
        );

        let pathless = Repository::new().unwrap();
        assert_failed(
            Blob::lookup(pathless.as_ref(), id(README).as_ref()),
            not_found,
            ErrorClass::Invalid,
        );
        assert_failed(
            Blob::lookup_prefix(pathless.as_ref(), id(README).as_ref(), 7),
            not_found,
            ErrorClass::Invalid,
        );

        // Ids of another type, at every length, never reach C.
        let mut wide = Oid::from_raw(&id(README).as_ref().id(), OidType::Sha256).unwrap();
        for kind in [2, 0, 3, 0x7f, u8::MAX] {
            wide.as_mut().set_raw_kind(kind);
            assert_failed(Blob::lookup(r, wide.as_ref()), -1, ErrorClass::Invalid);
            for len in [0, 4, 7, 39, 40, 41, 64, 65, usize::MAX] {
                assert_failed(
                    Blob::lookup_prefix(r, wide.as_ref(), len),
                    -1,
                    ErrorClass::Invalid,
                );
            }
        }
        assert_eq!(
            content(&Blob::lookup(r, id(README).as_ref()).unwrap()),
            b"hey there\n"
        );
    }

    /// libgit2 copies the id it is given: each one is overwritten and freed right
    /// after its lookup, before the blobs are read back.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_retain_no_id() {
        let repo = open_testrepo();
        let mut held = Vec::new();
        for (i, (hex_id, data)) in BLOBS.iter().enumerate() {
            let mut probe = Box::new(id(hex_id));
            let blob = if i % 2 == 0 {
                Blob::lookup(repo.as_ref(), probe.as_ref().as_ref())
            } else {
                Blob::lookup_prefix(repo.as_ref(), probe.as_ref().as_ref(), 10)
            }
            .unwrap();
            probe.as_mut().as_mut().set_id([0xee; 32]);
            drop(probe);
            held.push((data, blob));
        }
        for (data, blob) in &held {
            assert_eq!(content(blob), **data);
        }
    }

    /// Blobs of every size, the empty one included, read back from exactly their
    /// length; a blob outlives the object it came from and every other owner.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn contents_of_every_size_read_back_exactly() {
        let repo = TempRepo::new("blob-sizes");
        let boxed = repository(&repo);
        let sizes = [0usize, 1, 7, 8000, 8001, 1 << 16];
        let mut blobs = Vec::new();
        for (i, &len) in sizes.iter().enumerate() {
            let data: Vec<u8> = (0..len).map(|b| (b * 31 + i) as u8).collect();
            let id = write_raw(&repo, &data, ffi::GIT_OBJECT_BLOB);
            let blob = Blob::lookup(boxed, id.as_ref()).unwrap();
            assert_eq!(content(&blob), data, "{len}");
            blobs.push((data, blob));
        }
        for (data, blob) in blobs.iter().rev() {
            assert_eq!(content(blob), *data);
        }
    }

    /// With blobs cached, every lookup of one id shares the cache's blob, and owners
    /// keep blobs the cache evicts under them (a one-byte cache limit).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn cached_blobs_are_shared_and_outlive_eviction() {
        isolated(
            concat!(
                module_path!(),
                "::cached_blobs_are_shared_and_outlive_eviction"
            ),
            || {
                let repo = open_testrepo();
                set_blob_cache_limit(1 << 20);
                let r = repo.as_ref();
                let first = Blob::lookup(r, id(README).as_ref()).unwrap();
                let second = Blob::lookup_prefix(r, id(README).as_ref(), 8).unwrap();
                assert!(BlobArc::ptr_eq(&first, &second));
                let mut first = first;
                assert!(first.get_mut().is_none());
                drop((first, second));

                set_cache_max_size(1);
                for round in 0..4 {
                    let held: Vec<_> = BLOBS
                        .iter()
                        .map(|(h, _)| Blob::lookup(r, id(h).as_ref()).unwrap())
                        .collect();
                    let clones: Vec<_> = held.iter().rev().map(BlobArc::clone).collect();
                    for (blob, (_, data)) in held.iter().zip(BLOBS) {
                        assert_eq!(content(blob), data, "round {round}");
                    }
                    drop(held);
                    for (blob, (_, data)) in clones.iter().rev().zip(BLOBS) {
                        assert_eq!(content(blob), data, "round {round}");
                    }
                }
            },
        );
    }
}

/// The blob filter options, `FilterOptions`, and their handles.
///
/// The options are a plain value: their accessors project fields of Rust-owned
/// storage, and libgit2 reads them in place and copies them for one call. The tests
/// write every field through every handle, including the embedded id through its own
/// handle (Miri), keep a dangling `reserved` pointer that nothing may read (ASan, Miri),
/// and hand options libgit2 refuses (every bad version) and accepts (every flag bit) to
/// `git_blob_filter`, rewriting them between calls (BSan: libgit2 keeps no pointer into
/// them). They are `Send` and `Sync`: copies cross threads, and one value is read from
/// many at once (TSan).
mod filter_options {
    use std::ffi::{CStr, CString};
    use std::ptr::NonNull;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::object::blob::{Blob, FilterFlag, FilterFlags, FilterOptions};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::Repository;

    use crate::fixture::blob::filter;
    use crate::fixture::{init, isolated};

    /// `crlf.git`: a CRLF text blob, a binary one, one holding `$Id$`, and the commit
    /// whose `.gitattributes` gives `*.identlf` the `ident` attribute.
    const ALL_CRLF: &str = "a9a2e8913c1dbe2812fac5e6b4e0a4bd5d0d5966";
    const BINARY: &str = "85340755cfe5e28c2835781978bb1cece91b3d0f";
    const IDENT: &str = "055c8729cdcc372500a08db659c045e16c4409fb";
    const IDENT_COMMIT: &str = "b8986fec0f7bde90f78ac72706e782d82f24f2f0";

    fn id(hex: &str) -> Oid {
        Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap()
    }

    /// Options as C spells them, with a `reserved` pointer nothing may read.
    fn with_dangling_reserved() -> FilterOptions {
        FilterOptions::from(ffi::git_blob_filter_options {
            version: 1,
            flags: ffi::GIT_BLOB_FILTER_CHECK_FOR_BINARY,
            reserved: NonNull::<u64>::dangling().as_ptr().cast(),
            attr_commit_id: ffi::git_oid::default(),
        })
    }

    /// Every field through every handle, and copies that stay apart; the `reserved`
    /// pointer is tested and cleared, never read. Runs under Miri.
    #[test]
    fn fields_through_every_handle() {
        let mut opts = with_dangling_reserved();
        let copy = opts;
        {
            let mut m = opts.as_mut();
            let mut again = m.as_mut();
            again.set_version(i32::MIN);
            again.set_flags(FilterFlags::from_bits_retain(u32::MAX));
            let mut oid = again.attr_commit_id_mut();
            oid.set_raw_kind(u8::MAX);
            oid.set_id([0xff; 32]);
            assert!(m.as_ref().has_reserved());
            m.clear_reserved();
            let r = m.as_ref();
            assert_eq!(r.version(), i32::MIN);
            assert_eq!(r.flags().bits(), u32::MAX);
            assert_eq!(r.attr_commit_id().raw_kind(), u8::MAX);
            assert!(!r.has_reserved());
        }
        let r = copy.as_ref();
        assert_eq!((r.version(), r.flags().bits()), (1, 1));
        assert!(r.has_reserved());
        assert_eq!(r.attr_commit_id().id(), [0; 32]);
        // The borrowed id reads the options in place; a copy of it outlives them.
        let mut owned = Oid::zero(OidType::Sha1);
        {
            let moved = opts;
            let id = moved.as_ref().attr_commit_id();
            owned.as_mut().set_raw_kind(id.raw_kind());
            owned.as_mut().set_id(id.id());
        }
        assert_eq!(owned.as_ref().id(), [0xff; 32]);
        assert_eq!(owned.as_ref().raw_kind(), u8::MAX);
        let back: ffi::git_blob_filter_options = copy.into();
        assert_eq!(back.version, 1);
        assert_eq!(format!("{:?}", copy.as_ref()), format!("{copy:?}"));
    }

    /// One value read from many threads at once, and copies written on each.
    #[test]
    fn options_cross_threads() {
        let mut shared = with_dangling_reserved();
        shared.as_mut().clear_reserved();
        thread::scope(|s| {
            let handles: Vec<_> = FilterFlag::ALL
                .into_iter()
                .map(|flag| {
                    let shared = &shared;
                    let r = shared.as_ref();
                    s.spawn(move || {
                        let mut mine = *shared;
                        let mut m = mine.as_mut();
                        m.set_flags(r.flags() | flag);
                        m.attr_commit_id_mut().set_raw_kind(flag.raw() as u8);
                        assert!(mine.as_ref().flags().contains(flag));
                        assert_eq!(r.version(), 1);
                        mine
                    })
                })
                .collect();
            for (h, flag) in handles.into_iter().zip(FilterFlag::ALL) {
                let mine = h.join().unwrap();
                assert_eq!(mine.as_ref().attr_commit_id().raw_kind(), flag.raw() as u8);
            }
        });
        assert_eq!(shared.as_ref().flags(), FilterFlags::OPTIONS_DEFAULT);
    }

    /// libgit2 refuses every version but 1, read as an `unsigned int`, before it reads
    /// anything else; it accepts every flag bit, and never reads `reserved`. The same
    /// options are rewritten between calls.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn libgit2_reads_options_in_place_for_the_call() {
        isolated(
            concat!(
                module_path!(),
                "::libgit2_reads_options_in_place_for_the_call"
            ),
            libgit2_reads_options_in_place_for_the_call_body,
        );
    }

    fn libgit2_reads_options_in_place_for_the_call_body() {
        init();
        let path = CString::new(format!(
            "{}/../../../tests/resources/crlf.git",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let repo = Repository::open(&path).unwrap();
        let r = repo.as_ref();
        let crlf = Blob::lookup(r, id(ALL_CRLF).as_ref()).unwrap();
        let binary = Blob::lookup(r, id(BINARY).as_ref()).unwrap();
        let ident = Blob::lookup(r, id(IDENT).as_ref()).unwrap();
        let as_txt: &CStr = c"file.txt";

        let mut boxed = Box::new(with_dangling_reserved());
        let opts = &mut *boxed;
        for version in [0, -1, 2, i32::MIN, i32::MAX] {
            opts.as_mut().set_version(version);
            assert_eq!(
                filter(&crlf, as_txt, Some(&mut *opts)),
                Err(-1),
                "{version}"
            );
        }
        opts.as_mut().set_version(FilterOptions::VERSION);
        let plain = filter(&crlf, as_txt, None).unwrap();
        assert_eq!(filter(&crlf, as_txt, Some(&mut *opts)).unwrap(), plain);
        assert_eq!(filter(&binary, as_txt, Some(&mut *opts)).unwrap(), b"");
        opts.as_mut().set_flags(FilterFlags::empty());
        assert!(
            !filter(&binary, as_txt, Some(&mut *opts))
                .unwrap()
                .is_empty()
        );

        opts.as_mut()
            .set_flags(FilterFlags::from_bits_retain(u32::MAX));
        opts.as_mut().set_attr_commit_id(id(IDENT_COMMIT).as_ref());
        let expanded = filter(&ident, c"ident.identlf", Some(&mut *opts)).unwrap();
        assert_eq!(
            expanded,
            b"$Id: 055c8729cdcc372500a08db659c045e16c4409fb $\n"
        );
        opts.as_mut()
            .set_flags(FilterFlag::NoSystemAttributes | FilterFlag::AttributesFromCommit);
        opts.as_mut().attr_commit_id_mut().set_id([0xee; 32]);
        let mut fresh = FilterOptions::new();
        fresh.as_mut().set_flags(FilterFlags::empty());
        assert_eq!(
            filter(&ident, c"ident.identlf", Some(&mut fresh)).unwrap(),
            b"$Id$\n"
        );
        drop(boxed);
        assert_eq!(
            filter(&ident, c"ident.identlf", Some(&mut fresh)).unwrap(),
            b"$Id$\n"
        );
    }
}
