//! UB tests for `git2/tag.h`: `name_is_valid` (`git_tag_name_is_valid`), and the tag
//! object's owner and handles in [`tag_object`].
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

mod tag_object {
    //! UB tests for `git2/tag.h`'s `git_tag`: its owner `TagArc<'r>`, the constructors
    //! `Tag::lookup` and `Tag::lookup_prefix`, the handles, `TagRef::to_tag`, and the
    //! conversions from and to an object (`Tag::from_object`, `Tag::into_object`,
    //! `TagRef::from_object`, `TagRef::as_object`).
    //!
    //! A tag is counted, and shared with its repository's cache, which hands the same
    //! object to every lookup of its id, as a tag or as an object: every owner gives back
    //! its one reference through `git_tag_free` (or `git_object_free`, once handed over as
    //! an object) exactly once, and every clone takes one through `git_tag_dup`. A miscount
    //! frees a tag the cache or another owner still reads (ASan), or leaks it (LSan). The
    //! tests share tags between lookups, clones and conversions, drop them in every order,
    //! let the cache evict tags owners still hold and run without a cache. Every failing
    //! lookup must leave nothing behind, a half-parsed tag included (its name, tagger and
    //! message), and an id of another type than the repository's must be refused before
    //! it reaches C. libgit2 copies the id it is given (BSan: the tests overwrite and free
    //! it right after), and a tag keeps a bare pointer to its repository, which its target
    //! lookup follows across Rust reborrows of that repository (BSan). Tags are not
    //! `Send`, so the threaded test gives each thread its own repository (TSan).
    //!
    //! A tag cannot outlive its repository, nor a handle its owner, and no owner lends an
    //! exclusive handle (`compile_fail/` and doctests). Every test calls libgit2, so Miri
    //! skips them all.

    use std::ffi::CString;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::object::tag::{Tag, TagArc, TagRef};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryError};
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use crate::fixture::object_and_tree_entry::{
        enable_caching, object_view, open_testrepo, repository, set_cache_max_size, write_raw,
    };
    use crate::fixture::{TempRepo, isolated};

    /// Every tag of `testrepo.git`: `test` (of tag `e90810b`), `e90810b`, `hard_tag`,
    /// `taggerless` and `annotated_tag_to_blob`.
    const TAGS: [&str; 5] = [
        "b25fa35b38051e4ae45d4222e795f9df2e43f1d1",
        "7b4384978d2493e851f9cca7858815fac9b10980",
        "849a5e34a26815e821f865b8479f5815a47af0fe",
        "4a23e2e65ad4e31c4c9db7dc746650bfad082679",
        "521d87c1ec3aef9824daf6d96cc0ae3710766d91",
    ];
    const COMMIT: &str = "e90810b8df3e80c413d903f631643c716887138d";
    const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
    const BLOB: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";

    fn id(hex: &str) -> Oid {
        Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap()
    }

    /// C's hex id and type of the tag an owner holds, read through another counted
    /// reference handed over as an object.
    fn view(tag: &TagArc<'_>) -> (String, ffi::git_object_t) {
        object_view(&Tag::into_object(tag.clone()))
    }

    fn hex(tag: &TagArc<'_>) -> String {
        let (hex, kind) = view(tag);
        assert_eq!(kind, ffi::GIT_OBJECT_TAG);
        hex
    }

    fn assert_failed(result: Result<TagArc<'_>, RepositoryError>, code: i32, class: ErrorClass) {
        let e = result.unwrap_err();
        assert_eq!((e.raw_code(), e.class()), (code, Some(class)), "{e:?}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_clones_and_conversions_share_one_tag() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        for hex_id in TAGS {
            let tag = Tag::lookup(r, id(hex_id).as_ref()).unwrap();
            let short = Tag::lookup_prefix(r, id(hex_id).as_ref(), 7).unwrap();
            let object = Object::lookup(r, id(hex_id).as_ref(), None).unwrap();
            let viewed = TagRef::from_object(object.as_ref()).unwrap().to_tag();
            let adopted = Tag::from_object(object.clone()).unwrap();
            assert!(TagArc::ptr_eq(&tag, &short));
            assert!(TagArc::ptr_eq(&tag, &viewed) && TagArc::ptr_eq(&tag, &adopted));
            assert_eq!(
                object_view(&object),
                (hex_id.to_owned(), ffi::GIT_OBJECT_TAG)
            );
            let clone = tag.clone();
            let copy = short.as_ref().to_tag();
            let as_object = Tag::into_object(adopted);
            drop((tag, short, object, viewed));
            assert_eq!(hex(&clone), hex_id);
            drop(clone);
            assert_eq!(hex(&copy), hex_id);
            drop(copy);
            assert_eq!(object_view(&as_object).0, hex_id);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reference_chains_release_every_reference_in_any_order() {
        let repo = open_testrepo();
        for order in 0..3 {
            let first = Tag::lookup(repo.as_ref(), id(TAGS[0]).as_ref()).unwrap();
            let mut chain = vec![first];
            for i in 0..16 {
                let last = chain.last().unwrap();
                let next = match i % 3 {
                    0 => last.clone(),
                    1 => last.as_ref().to_tag(),
                    // Handed over as an object and back: one reference all the way.
                    _ => Tag::from_object(Tag::into_object(last.clone())).unwrap(),
                };
                chain.push(next);
            }
            match order {
                0 => chain.reverse(),
                1 => chain.rotate_left(7),
                _ => {}
            }
            while let Some(tag) = chain.pop() {
                assert_eq!(hex(&tag), TAGS[0]);
                drop(tag);
                if let Some(survivor) = chain.first() {
                    assert_eq!(hex(survivor), TAGS[0]);
                }
            }
        }
    }

    /// Every failure leaves no object behind (LSan), with its code and class; objects of
    /// other types are looked up, and released, by libgit2 before it refuses them.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failures_leave_nothing_behind() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        let not_found = ErrorCode::NotFound.raw();
        for other in [COMMIT, TREE, BLOB] {
            assert_failed(
                Tag::lookup(r, id(other).as_ref()),
                not_found,
                ErrorClass::Invalid,
            );
            assert_failed(
                Tag::lookup_prefix(r, id(other).as_ref(), 9),
                not_found,
                ErrorClass::Invalid,
            );
        }
        assert_failed(
            Tag::lookup(r, id(&"de".repeat(20)).as_ref()),
            not_found,
            ErrorClass::Odb,
        );
        assert_failed(
            Tag::lookup(r, Oid::zero(OidType::Sha1).as_ref()),
            not_found,
            ErrorClass::Odb,
        );
        for len in [0, 1, 3] {
            assert_failed(
                Tag::lookup_prefix(r, id(TAGS[0]).as_ref(), len),
                ErrorCode::Ambiguous.raw(),
                ErrorClass::Object,
            );
        }
        // `1810` starts two objects, neither a tag.
        let prefix = Oid::from_prefix(b"1810", OidType::Sha1).unwrap();
        assert_failed(
            Tag::lookup_prefix(r, prefix.as_ref(), 4),
            ErrorCode::Ambiguous.raw(),
            ErrorClass::Odb,
        );

        // A repository without a git directory has no object database to load.
        let pathless = Repository::new().unwrap();
        assert_failed(
            Tag::lookup(pathless.as_ref(), id(TAGS[0]).as_ref()),
            not_found,
            ErrorClass::Invalid,
        );
        assert_failed(
            Tag::lookup_prefix(pathless.as_ref(), id(TAGS[0]).as_ref(), 7),
            not_found,
            ErrorClass::Invalid,
        );

        // Ids of another type, at every length, never reach C.
        let mut wide = Oid::from_raw(&id(TAGS[0]).as_ref().id(), OidType::Sha256).unwrap();
        for kind in [2, 0, 3, 0x7f, u8::MAX] {
            wide.as_mut().set_raw_kind(kind);
            assert_failed(Tag::lookup(r, wide.as_ref()), -1, ErrorClass::Invalid);
            for len in [0, 4, 7, 39, 40, 41, 64, 65, usize::MAX] {
                assert_failed(
                    Tag::lookup_prefix(r, wide.as_ref(), len),
                    -1,
                    ErrorClass::Invalid,
                );
            }
        }
        // The repository still answers.
        assert_eq!(hex(&Tag::lookup(r, id(TAGS[1]).as_ref()).unwrap()), TAGS[1]);
    }

    /// Tags the database holds but libgit2 cannot parse fail their lookup, through both
    /// constructors, and the half-parsed tag is freed with whatever the parse had
    /// allocated (its name, a tagger it could not parse): ASan flags a bad free, LSan a
    /// leak. A well-formed tag without tagger or message beside them is found.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn unparsable_tags_fail_and_leave_nothing_behind() {
        let repo = TempRepo::new("tag-unparsable");
        let r = repository(&repo);
        let target = format!("object {COMMIT}\ntype commit\n");
        let corrupt = [
            "object 1234\n".to_owned(),
            format!("object {COMMIT}\ntype nope\ntag x\n"),
            format!("{target}tag no-newline"),
            format!("{target}tag x\ntagger nobody\n\nmessage\n"),
            format!("{target}tag x\ntagger A <a@b> 0 +0000\nno blank line"),
            // Each field check in turn: truncated after the id, a misspelt `type`, a type
            // cut short, truncated after the type, a misspelt `tag`.
            format!("object {COMMIT}\n"),
            format!("object {COMMIT}\nkind commit\ntag x\n"),
            format!("object {COMMIT}\ntype comm"),
            format!("{target}ta"),
            format!("{target}name x\n"),
        ];
        for data in &corrupt {
            let id = write_raw(&repo, data.as_bytes(), ffi::GIT_OBJECT_TAG);
            for result in [
                Tag::lookup(r, id.as_ref()),
                Tag::lookup_prefix(r, id.as_ref(), 12),
            ] {
                let e = result.unwrap_err();
                assert_eq!(e.raw_code(), -1, "{data:?}: {e:?}");
            }
        }
        let bare = write_raw(
            &repo,
            format!("{target}tag bare\n").as_bytes(),
            ffi::GIT_OBJECT_TAG,
        );
        let tag = Tag::lookup(r, bare.as_ref()).unwrap();
        assert_eq!(view(&tag).1, ffi::GIT_OBJECT_TAG);
        let full = write_raw(
            &repo,
            format!("{target}tag full\ntagger A <a@b> 0 +0000\n\nmessage\n").as_bytes(),
            ffi::GIT_OBJECT_TAG,
        );
        let tag = Tag::lookup_prefix(r, full.as_ref(), 40).unwrap();
        assert_eq!(view(&tag).1, ffi::GIT_OBJECT_TAG);
        // A header line past the tagger, before the blank line and the message.
        let more = write_raw(
            &repo,
            format!("{target}tag more\ntagger A <a@b> 0 +0000\nextra x\n\nmessage\n").as_bytes(),
            ffi::GIT_OBJECT_TAG,
        );
        let tag = Tag::lookup(r, more.as_ref()).unwrap();
        assert_eq!(view(&tag).1, ffi::GIT_OBJECT_TAG);
    }

    /// libgit2 copies the id it is given: each one is overwritten and freed right after
    /// its lookup, before the tags are read back and looked up again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_retain_no_id() {
        let repo = open_testrepo();
        let mut held = Vec::new();
        for (i, hex_id) in TAGS.iter().enumerate() {
            let mut probe = Box::new(id(hex_id));
            let tag = if i % 2 == 0 {
                Tag::lookup(repo.as_ref(), probe.as_ref().as_ref())
            } else {
                Tag::lookup_prefix(repo.as_ref(), probe.as_ref().as_ref(), 10)
            }
            .unwrap();
            probe.as_mut().as_mut().set_id([0xee; 32]);
            drop(probe);
            held.push((hex_id, tag));
        }
        for (hex_id, tag) in &held {
            assert_eq!(hex(tag), **hex_id);
            let again = Tag::lookup(repo.as_ref(), id(hex_id).as_ref()).unwrap();
            assert!(TagArc::ptr_eq(&again, tag));
        }
    }

    /// A tag keeps a bare pointer to its repository, which peeling it to its target
    /// follows, while Rust reborrows the repository between the lookups and the peels.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn tags_reach_their_repository_across_reborrows() {
        let repo = open_testrepo();
        let tags: Vec<_> = TAGS
            .iter()
            .map(|h| Tag::lookup(repo.as_ref(), id(h).as_ref()).unwrap())
            .collect();
        for (tag, hex_id) in tags.iter().zip(TAGS) {
            let _ = repo.as_ref();
            let target = tag.as_ref().as_object().peel(None).unwrap();
            let r = repo.as_ref();
            let again = Object::lookup(r, id(&object_view(&target).0).as_ref(), None).unwrap();
            // The cache keeps commits, not blobs: a blob target is parsed afresh.
            let blob = object_view(&target).1 == ffi::GIT_OBJECT_BLOB;
            assert_eq!(ObjectArc::ptr_eq(&target, &again), !blob, "{hex_id}");
            assert_eq!(object_view(&target), object_view(&again));
            let peeled = tag
                .as_ref()
                .as_object()
                .peel(Some(ObjectType::Tag))
                .unwrap();
            assert_eq!(Tag::from_object(peeled).map(|t| hex(&t)).unwrap(), hex_id);
        }
    }

    /// With a cache limit of one byte, the cache evicts tags as fast as they come, while
    /// owners still hold them: the owners' references keep them alive.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn owners_outlive_cache_eviction() {
        isolated(
            concat!(module_path!(), "::owners_outlive_cache_eviction"),
            || {
                let repo = open_testrepo();
                set_cache_max_size(1);
                for round in 0..4 {
                    let held: Vec<_> = TAGS
                        .iter()
                        .map(|h| Tag::lookup(repo.as_ref(), id(h).as_ref()).unwrap())
                        .collect();
                    let clones: Vec<_> = held.iter().rev().map(|t| t.as_ref().to_tag()).collect();
                    drop(held);
                    for (tag, h) in clones.iter().rev().zip(TAGS) {
                        assert_eq!(hex(tag), h, "round {round}");
                    }
                }
            },
        );
    }

    /// With caching off, every lookup parses a fresh tag, which its owners alone count;
    /// dropping the last frees it, with its name, tagger and message.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn without_a_cache_each_lookup_is_a_fresh_tag() {
        isolated(
            concat!(
                module_path!(),
                "::without_a_cache_each_lookup_is_a_fresh_tag"
            ),
            || {
                let repo = open_testrepo();
                enable_caching(false);
                for hex_id in TAGS {
                    let first = Tag::lookup(repo.as_ref(), id(hex_id).as_ref()).unwrap();
                    let second = Tag::lookup_prefix(repo.as_ref(), id(hex_id).as_ref(), 8).unwrap();
                    assert!(!TagArc::ptr_eq(&first, &second));
                    let clone = first.clone();
                    drop(first);
                    assert_eq!(hex(&clone), hex_id);
                    drop(second);
                    assert_eq!(hex(&clone), hex_id);
                }
            },
        );
    }

    /// Each thread opens its own repository and looks its tags up, clones and converts
    /// them: the shared machinery underneath (the runtime, the allocator, the packs'
    /// mapping) must not race.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn threads_with_their_own_repositories() {
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let repo = open_testrepo();
                    for round in 0..8 {
                        let hex_id = TAGS[(t + round) % TAGS.len()];
                        let tag = Tag::lookup(repo.as_ref(), id(hex_id).as_ref()).unwrap();
                        let object = Tag::into_object(tag.clone());
                        let back = Tag::from_object(object).unwrap();
                        assert!(TagArc::ptr_eq(&tag, &back));
                        assert_eq!(hex(&tag), hex_id);
                    }
                });
            }
        });
    }

    /// The handles: a shared handle is `Copy`, viewed as an object it is the same
    /// pointer, and it formats opaquely.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shared_handles_coexist() {
        let repo = open_testrepo();
        let tag = Tag::lookup(repo.as_ref(), id(TAGS[2]).as_ref()).unwrap();
        let a = tag.as_ref();
        let b = a;
        assert!(TagArc::ptr_eq(&a.to_tag(), &b.to_tag()));
        let object = a.as_object().to_object();
        assert_eq!(object_view(&object).0, TAGS[2]);
        assert_eq!(format!("{b:?}"), "Tag { .. }");
    }
}
