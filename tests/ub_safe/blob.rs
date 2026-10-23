//! UB tests for `git2/blob.h`: `data_is_binary` (`git_blob_data_is_binary`), the
//! filter flags in [`filter_flags`], the blob owner in [`owner`], the filter options
//! in [`filter_options`], and reading and filtering a blob in [`reading`].
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

    use crate::fixture::blob::set_blob_cache_limit;

    /// A copy of the content of the blob an owner holds.
    fn content(blob: &BlobArc<'_>) -> Vec<u8> {
        blob.as_ref().content().to_vec()
    }
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

    /// [`BlobRef::filter`] of `blob` as `path`, with `opts` (or none): the filtered
    /// bytes, or the code it failed with.
    fn filter(
        blob: &libgit2::libgit2::object::blob::BlobArc<'_>,
        path: &std::ffi::CStr,
        opts: Option<&mut FilterOptions>,
    ) -> Result<Vec<u8>, std::ffi::c_int> {
        let out = blob.as_ref().filter(path, opts.map(|o| o.as_ref()));
        out.map(|b| b.as_ref().as_bytes().to_vec())
            .map_err(|e| e.raw_code())
    }
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
            "{}/c/tests/resources/crlf.git",
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

/// UB tests for the blob writers: `create_from_buffer`, `create_from_workdir`,
/// `create_from_disk`, and `create_from_stream` with its `BlobWriter`.
///
/// libgit2 reads each input for the call only: every buffer and path is an exact-length
/// heap allocation, overwritten and freed right after the call (ASan flags a read past
/// it, BSan a pointer libgit2 kept). A writer owns libgit2's blob stream, its file buffer
/// and temporary file, which committing or dropping releases once (ASan: double free;
/// LSan: leak; the temporary file must be gone either way), whatever was written, past
/// the stream's 2 MiB buffer included. Repositories without paths, bare ones, missing
/// files, directories and symbolic links take every failure path. The writers that load
/// filters order libgit2's first, unsynchronised initialisation of its built-in filters
/// among themselves, not with raw loads elsewhere in this binary, so each test loading
/// filters runs alone in a child: one of them writes blobs through filters on several
/// threads at once (TSan). A writer, like its
/// repository, stays on one thread, and outlives neither (doctests).
mod writing {
    use std::ffi::{CStr, CString};
    use std::io::Write;
    use std::path::PathBuf;
    use std::thread;

    use libgit2::libgit2::object::blob::{
        create_from_buffer, create_from_disk, create_from_stream, create_from_workdir,
    };
    use libgit2::libgit2::oid::oid::Oid;
    use libgit2::libgit2::repository::repository::{
        Repository, RepositoryBox, RepositoryError, RepositoryRef,
    };
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use crate::fixture::isolated;

    /// `git hash-object` of `hello\n` and of nothing.
    const HELLO: &str = "ce013625030ba8dba906f756967f9e9ca394464a";
    const EMPTY: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

    /// Attributes selecting the CRLF and `ident` filters.
    const ATTRIBUTES: &[u8] = b"*.txt text\n*.id ident\n";

    /// A fresh repository in a temporary directory, removed when dropped.
    struct Work {
        dir: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl Work {
        fn new(tag: &str, bare: bool) -> Self {
            let dir =
                std::env::temp_dir().join(format!("crustify-blob-ub-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let repo = Repository::init(&cstring(&dir), bare).unwrap();
            if !bare {
                std::fs::write(dir.join(".gitattributes"), ATTRIBUTES).unwrap();
            }
            Self {
                dir,
                repo: Some(repo),
            }
        }

        fn repo(&self) -> RepositoryRef<'_> {
            self.repo.as_ref().unwrap().as_ref()
        }

        /// Writes `data` at `rel` in the working directory; its absolute path.
        fn file(&self, rel: &str, data: &[u8]) -> CString {
            let path = self.dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, data).unwrap();
            cstring(&path)
        }

        /// The temporary files of open blob writers.
        fn streamed(&self) -> usize {
            let objects = self.dir.join(".git/objects");
            let objects = if objects.exists() {
                objects
            } else {
                self.dir.join("objects")
            };
            std::fs::read_dir(objects)
                .unwrap()
                .filter(|e| {
                    let name = e.as_ref().unwrap().file_name();
                    name.to_string_lossy().starts_with("streamed")
                })
                .count()
        }
    }

    impl Drop for Work {
        fn drop(&mut self) {
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn cstring(path: &std::path::Path) -> CString {
        CString::new(path.to_str().unwrap()).unwrap()
    }

    /// `s` in a heap buffer of exactly its length, NUL included.
    fn exact(s: &str) -> Box<CStr> {
        CString::new(s).unwrap().into_boxed_c_str()
    }

    fn hex(id: &Oid) -> String {
        id.as_ref().to_hex_string().unwrap()
    }

    #[track_caller]
    fn assert_failed<T: std::fmt::Debug>(
        result: Result<T, RepositoryError>,
        code: Option<ErrorCode>,
        class: ErrorClass,
    ) {
        let e = result.unwrap_err();
        assert_eq!((e.code(), e.class()), (code, Some(class)), "{e:?}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn buffers_are_read_whole_and_only_during_the_call() {
        let work = Work::new("buffers", true);
        let repo = work.repo();
        assert_eq!(hex(&create_from_buffer(repo, &[]).unwrap()), EMPTY);
        assert_eq!(hex(&create_from_buffer(repo, &Vec::new()).unwrap()), EMPTY);
        let sizes: &[usize] = if cfg!(bsan) {
            &[0, 1, 7, 4096]
        } else {
            &[0, 1, 7, 4096, 65_537, 3 << 20]
        };
        for &len in sizes {
            let mut data: Box<[u8]> = (0..len).map(|i| (i % 253) as u8).collect();
            let id = create_from_buffer(repo, &data).unwrap();
            let again = create_from_buffer(repo, &data.clone()).unwrap();
            data.fill(0xa5);
            drop(data);
            assert_eq!(id, again, "{len}");
        }
        let mut hello = b"hello\n".to_vec().into_boxed_slice();
        let id = create_from_buffer(repo, &hello).unwrap();
        hello.fill(0);
        assert_eq!(hex(&id), HELLO);
        assert_ne!(create_from_buffer(repo, b"a\0b").unwrap(), id);
    }

    /// Repositories without paths have no object database, working directory or
    /// `objects` directory; bare ones no working directory: each writer fails cleanly.
    /// The working-directory and disk writers initialise the built-in filters first, so
    /// this runs alone in a child too.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn pathless_and_bare_repositories_fail_cleanly() {
        isolated(
            concat!(
                module_path!(),
                "::pathless_and_bare_repositories_fail_cleanly"
            ),
            || {
                let pathless = Repository::new().unwrap();
                let bare = Work::new("bare-fail", true);
                let outside = Work::new("bare-fail-file", false);
                let file = outside.file("plain.bin", b"data");
                let repo = pathless.as_ref();
                assert_failed(
                    create_from_buffer(repo, b"x"),
                    Some(ErrorCode::NotFound),
                    ErrorClass::Invalid,
                );
                assert_failed(
                    create_from_disk(repo, &file),
                    Some(ErrorCode::Generic),
                    ErrorClass::Invalid,
                );
                for repo in [pathless.as_ref(), bare.repo()] {
                    let e = create_from_workdir(repo, c"plain.bin").unwrap_err();
                    assert_eq!(e.message(), c"repository has no working directory");
                    assert_eq!(e.class(), Some(ErrorClass::Repository));
                }
                assert_failed(
                    create_from_stream(repo, None),
                    Some(ErrorCode::NotFound),
                    ErrorClass::Invalid,
                );
                assert_failed(
                    create_from_stream(repo, Some(c"hint.txt")),
                    Some(ErrorCode::NotFound),
                    ErrorClass::Invalid,
                );
                // A bare repository writes files from anywhere, unfiltered.
                let id = create_from_disk(bare.repo(), &file).unwrap();
                assert_eq!(id, create_from_buffer(bare.repo(), b"data").unwrap());

                // Without an `objects` directory a writer has nowhere for its temporary file.
                let gone = Work::new("objects-gone", true);
                std::fs::remove_dir_all(gone.dir.join("objects")).unwrap();
                let e = create_from_stream(gone.repo(), None).unwrap_err();
                assert_eq!(e.class(), Some(ErrorClass::Os), "{e:?}");
            },
        );
    }

    /// Working-directory files go through the filters their paths select; directories,
    /// missing files and absolute paths fail; a symbolic link is stored as its target.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn workdir_files_are_read_through_their_filters() {
        isolated(
            concat!(
                module_path!(),
                "::workdir_files_are_read_through_their_filters"
            ),
            || {
                let work = Work::new("workdir", false);
                let repo = work.repo();
                work.file("a.txt", b"one\r\ntwo\r\n");
                work.file("sub/b.id", b"$Id: stale $\n");
                work.file("c.bin", b"one\r\ntwo\r\n");
                let crlf = create_from_workdir(repo, &exact("a.txt")).unwrap();
                assert_eq!(crlf, create_from_buffer(repo, b"one\ntwo\n").unwrap());
                let ident = create_from_workdir(repo, &exact("sub/b.id")).unwrap();
                assert_eq!(ident, create_from_buffer(repo, b"$Id$\n").unwrap());
                let raw = create_from_workdir(repo, &exact("c.bin")).unwrap();
                assert_eq!(raw, create_from_buffer(repo, b"one\r\ntwo\r\n").unwrap());

                std::os::unix::fs::symlink("a.txt", work.dir.join("link")).unwrap();
                let link = create_from_workdir(repo, c"link").unwrap();
                assert_eq!(link, create_from_buffer(repo, b"a.txt").unwrap());

                assert_failed(
                    create_from_workdir(repo, c"sub"),
                    Some(ErrorCode::Directory),
                    ErrorClass::Odb,
                );
                let e = create_from_workdir(repo, c"missing.txt").unwrap_err();
                assert_eq!(e.code(), Some(ErrorCode::NotFound), "{e:?}");
                let absolute = work.file("abs.txt", b"x\r\n");
                assert_failed(
                    create_from_workdir(repo, &absolute),
                    Some(ErrorCode::Generic),
                    ErrorClass::Internal,
                );
            },
        );
    }

    /// A file inside the working directory goes through its filters, by absolute or
    /// relative path; one outside it does not.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn disk_files_are_filtered_inside_the_workdir_only() {
        isolated(
            concat!(
                module_path!(),
                "::disk_files_are_filtered_inside_the_workdir_only"
            ),
            || {
                let work = Work::new("disk", false);
                let outside = Work::new("disk-outside", true);
                let repo = work.repo();
                let inside = work.file("a.txt", b"one\r\n");
                let elsewhere = outside.file("a.txt", b"one\r\n");
                let filtered = create_from_buffer(repo, b"one\n").unwrap();
                assert_eq!(create_from_disk(repo, &inside).unwrap(), filtered);
                let mut path = inside.into_bytes();
                let unfiltered = create_from_disk(repo, &elsewhere).unwrap();
                path.fill(b'/');
                assert_eq!(unfiltered, create_from_buffer(repo, b"one\r\n").unwrap());
                assert_failed(
                    create_from_disk(repo, &cstring(&work.dir)),
                    Some(ErrorCode::Directory),
                    ErrorClass::Odb,
                );
                let e = create_from_disk(repo, c"/nonexistent/crustify/file").unwrap_err();
                assert_eq!(e.code(), Some(ErrorCode::NotFound), "{e:?}");
            },
        );
    }

    /// Writers commit what was written, past the 2 MiB buffer too, through the filters
    /// of their hint path; dropping one discards it. Each temporary file lives exactly as
    /// long as its writer, and several writers interleave on one repository.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn writers_commit_or_discard_what_was_written() {
        isolated(
            concat!(
                module_path!(),
                "::writers_commit_or_discard_what_was_written"
            ),
            || {
                let work = Work::new("writers", false);
                let repo = work.repo();
                let chunk: Vec<u8> = (0..65_537u32).map(|i| (i % 249) as u8).collect();
                let rounds = if cfg!(bsan) { 2 } else { 34 };
                let mut big = create_from_stream(repo, None).unwrap();
                let mut small = create_from_stream(repo, Some(&exact("a.txt"))).unwrap();
                let dropped = create_from_stream(repo, Some(c"dropped.txt")).unwrap();
                assert_eq!(work.streamed(), 3);
                let mut all = Vec::new();
                for _ in 0..rounds {
                    let mut piece = chunk.clone();
                    big.write(&piece).unwrap();
                    all.extend_from_slice(&piece);
                    piece.fill(0);
                    small.write(b"line\r\n").unwrap();
                }
                big.write(&[]).unwrap();
                drop(dropped);
                assert_eq!(work.streamed(), 2);
                let id = big.commit().unwrap();
                assert_eq!(id, create_from_buffer(repo, &all).unwrap());
                let lines = small.commit().unwrap();
                let expected = b"line\n".repeat(rounds);
                assert_eq!(lines, create_from_buffer(repo, &expected).unwrap());
                assert_eq!(work.streamed(), 0);

                let mut empty = create_from_stream(repo, None).unwrap();
                empty.flush().unwrap();
                assert_eq!(hex(&empty.commit().unwrap()), EMPTY);
                let mut unwritten = create_from_stream(repo, Some(c"x.txt")).unwrap();
                writeln!(unwritten, "hello").unwrap();
                assert_eq!(hex(&unwritten.commit().unwrap()), HELLO);
                assert_eq!(work.streamed(), 0);
            },
        );
    }

    /// Each thread writes blobs into its own repository through filters at once: the
    /// built-in filters are initialised once, before any of their loads (TSan). Alone in
    /// a child, where nothing else loads a filter list first.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn filtered_writes_on_many_threads() {
        isolated(
            concat!(module_path!(), "::filtered_writes_on_many_threads"),
            || {
                thread::scope(|s| {
                    for t in 0..4 {
                        s.spawn(move || {
                            let work = Work::new(&format!("threads-{t}"), false);
                            let repo = work.repo();
                            let path = work.file("a.txt", b"x\r\n");
                            let expected = create_from_buffer(repo, b"x\n").unwrap();
                            let ids = match t % 3 {
                                0 => create_from_workdir(repo, c"a.txt"),
                                1 => create_from_disk(repo, &path),
                                _ => {
                                    let mut w = create_from_stream(repo, Some(c"a.txt")).unwrap();
                                    w.write(b"x\r\n").unwrap();
                                    w.commit()
                                }
                            };
                            assert_eq!(ids.unwrap(), expected);
                        });
                    }
                });
            },
        );
    }
}

/// Reading a blob: `BlobRef::content`, `size`, `id`, `owner` and `is_binary`, and
/// `BlobRef::filter`.
///
/// The content is borrowed in place from the blob, which holds the object database's
/// copy of it: every byte is read back, for blobs of many sizes (empty, around the 8000
/// bytes `is_binary` looks at, and past libgit2's stream buffer), through every holder
/// (lookups, clones, `to_blob`, an object view), and after the other holders of a shared
/// blob are gone (ASan: a slice longer than its allocation, a use after free). The id
/// and the repository are borrowed the same way. `filter` copies the content into a
/// buffer it owns, which is read after the blob and its repository are gone and
/// released once (ASan, LSan); filter lists race on their first load, so those tests run
/// in isolated children. Each thread reads blobs of its own repository at once (TSan).
/// Content outliving its blob and a repository outliving its borrow are doctests. Every
/// test calls libgit2, so Miri skips them all.
mod reading {
    use std::ffi::{CStr, CString};
    use std::thread;

    use libgit2::libgit2::object::blob::{
        Blob, BlobRef, FilterFlag, FilterOptions, create_from_buffer, data_is_binary,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::Repository;

    use crate::fixture::blob::set_blob_cache_limit;
    use crate::fixture::object_and_tree_entry::repository;
    use crate::fixture::{TempRepo, init, isolated};

    /// Bytes `0, 1, 2, ...` wrapping at 251, `len` of them, with a NUL at `nul` if any.
    fn bytes(len: usize, nul: Option<usize>) -> Vec<u8> {
        let mut v: Vec<u8> = (0..len).map(|i| (i % 251) as u8 | 0x20).collect();
        if let Some(i) = nul.filter(|&i| i < len) {
            v[i] = 0;
        }
        v
    }

    /// Everything a blob lends, read whole: `(id hex, size, content, binary?)`.
    fn read(blob: BlobRef<'_, '_>) -> (String, u64, Vec<u8>, bool) {
        let content = blob.content();
        assert_eq!(content.len() as u64, blob.size());
        let sum: u64 = content.iter().map(|&b| u64::from(b)).sum();
        assert!(sum <= 255 * content.len() as u64);
        (
            blob.id().to_hex_string().unwrap(),
            blob.size(),
            content.to_vec(),
            blob.is_binary(),
        )
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[cfg_attr(
        bsan,
        ignore = "its large inputs outlast the BorrowSanitizer gate, over an hour alone; the other variants run it"
    )]
    fn contents_are_read_whole_through_every_holder() {
        let temp = TempRepo::new("blob-read-holders");
        let repo = repository(&temp);
        for (len, nul) in [
            (0, None),
            (1, None),
            (7, Some(3)),
            (4096, None),
            (8000, Some(7999)),
            (8001, Some(8000)),
            (70_000, None),
            (3 << 20, Some(1)),
        ] {
            let data = bytes(len, nul);
            let id = create_from_buffer(repo, &data).unwrap();
            let first = Blob::lookup(repo, id.as_ref()).unwrap();
            let second = Blob::lookup(repo, id.as_ref()).unwrap();
            let (clone, copy) = (first.clone(), second.as_ref().to_blob());
            let expected = (
                id.as_ref().to_hex_string().unwrap(),
                len as u64,
                data.clone(),
                data_is_binary(&data[..len.min(8000)]),
            );
            assert_eq!(read(first.as_ref()), expected, "{len}");
            assert_eq!(first.as_ref().owner().path(), repo.path());
            let object = Blob::into_object(first);
            let view = BlobRef::from_object(object.as_ref()).unwrap();
            assert_eq!(read(view), expected);
            drop(object);
            drop(second);
            // The copies outlive the holders they came from.
            assert_eq!(read(clone.as_ref()), expected);
            assert_eq!(read(copy.as_ref()), expected);
            let content = copy.as_ref().content();
            drop(clone);
            assert_eq!(content, &data[..]);
        }
    }

    /// With blobs cached, lookups share one blob: its content stays readable through the
    /// last holder, whichever goes first, and through the cache's own reference.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shared_cached_blobs_lend_their_content() {
        isolated(
            concat!(module_path!(), "::shared_cached_blobs_lend_their_content"),
            shared_cached_blobs_lend_their_content_body,
        );
    }

    fn shared_cached_blobs_lend_their_content_body() {
        init();
        set_blob_cache_limit(1 << 20);
        let temp = TempRepo::new("blob-read-cached");
        let repo = repository(&temp);
        let data = bytes(5000, None);
        let id = create_from_buffer(repo, &data).unwrap();
        for order in 0..3 {
            let holders: Vec<_> = (0..3)
                .map(|_| Blob::lookup(repo, id.as_ref()).unwrap())
                .collect();
            let mut holders: Vec<_> = holders.into_iter().map(Some).collect();
            for k in 0..3 {
                let i = (k + order) % 3;
                let blob = holders[i].take().unwrap();
                assert_eq!(blob.as_ref().content(), &data[..]);
                drop(blob);
                for other in holders.iter().flatten() {
                    assert_eq!(other.as_ref().content(), &data[..]);
                    assert_eq!(other.as_ref().id().id()[..20], id.as_ref().id()[..20]);
                }
            }
            let again = Blob::lookup(repo, id.as_ref()).unwrap();
            assert_eq!(again.as_ref().content(), &data[..]);
        }
    }

    /// `filter`'s buffer is a copy of its own: read after the blob and the repository
    /// are gone, and released once, whichever options and paths produced it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn filtered_buffers_outlive_their_blob() {
        isolated(
            concat!(module_path!(), "::filtered_buffers_outlive_their_blob"),
            filtered_buffers_outlive_their_blob_body,
        );
    }

    fn filtered_buffers_outlive_their_blob_body() {
        init();
        let path = CString::new(format!(
            "{}/c/tests/resources/crlf.git",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let oid = |hex: &CStr| Oid::from_string(hex, OidType::Sha1).unwrap();
        let repo = Repository::open(&path).unwrap();
        let ident_commit = oid(c"b8986fec0f7bde90f78ac72706e782d82f24f2f0");
        let mut opts = FilterOptions::new();
        opts.as_mut()
            .set_flags(FilterFlag::AttributesFromCommit | FilterFlag::NoSystemAttributes);
        opts.as_mut().set_attr_commit_id(ident_commit.as_ref());
        let mut kept = Vec::new();
        for (blob, as_path) in [
            (c"a9a2e8913c1dbe2812fac5e6b4e0a4bd5d0d5966", c"file.crlf"),
            (c"85340755cfe5e28c2835781978bb1cece91b3d0f", c"file.txt"),
            (
                c"055c8729cdcc372500a08db659c045e16c4409fb",
                c"ident.identlf",
            ),
            (
                c"055c8729cdcc372500a08db659c045e16c4409fb",
                c"raw/ident.bin",
            ),
        ] {
            let blob = Blob::lookup(repo.as_ref(), oid(blob).as_ref()).unwrap();
            for opts in [None, Some(opts.as_ref())] {
                let as_path = CString::new(as_path.to_bytes()).unwrap().into_boxed_c_str();
                let out = blob.as_ref().filter(&as_path, opts).unwrap();
                drop(as_path);
                kept.push((blob.as_ref().content().to_vec(), out));
            }
        }
        // The id-expanding filter changed the third blob, under the options only.
        assert_eq!(
            kept[5].1.as_ref().as_bytes(),
            b"$Id: 055c8729cdcc372500a08db659c045e16c4409fb $\n"
        );
        let copies: Vec<_> = kept
            .iter()
            .map(|(_, b)| b.as_ref().as_bytes().to_vec())
            .collect();
        drop(repo);
        for ((_, buf), copy) in kept.iter().zip(&copies) {
            assert_eq!(buf.as_ref().as_bytes(), &copy[..]);
        }
        // Released in reverse.
        while kept.pop().is_some() {}
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn threads_read_blobs_of_their_own_repositories() {
        init();
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let temp = TempRepo::new(&format!("blob-read-thread-{t}"));
                    let repo = repository(&temp);
                    for len in [0, 10, 9000] {
                        let data = bytes(len + t, Some(len / 2));
                        let id = create_from_buffer(repo, &data).unwrap();
                        let blob = Blob::lookup(repo, id.as_ref()).unwrap();
                        assert_eq!(blob.as_ref().content(), &data[..]);
                        assert_eq!(
                            blob.as_ref().is_binary(),
                            data_is_binary(&data[..data.len().min(8000)])
                        );
                    }
                });
            }
        });
    }
}
