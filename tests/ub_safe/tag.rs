//! UB tests for `git2/tag.h`: `name_is_valid` (`git_tag_name_is_valid`), the tag
//! object's owner and handles in [`tag_object`], and the routines over a repository's
//! tags and the foreach callback in [`repository_tags`].
//!
//! libgit2 reads the name up to its NUL, copies it into a reference name in its
//! allocator, and writes the verdict into a Rust local. Each name sits in an
//! exact-length heap buffer (a `CString`), so `ASan` flags any read past its NUL, and the
//! copy must be released (`LSan`). The name is rewritten between calls (`BSan`: libgit2
//! keeps no pointer into it), and many threads check names at once, racing the first
//! call's initialisation of libgit2 (`TSan`).

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
    //! frees a tag the cache or another owner still reads (`ASan`), or leaks it (`LSan`). The
    //! tests share tags between lookups, clones and conversions, drop them in every order,
    //! let the cache evict tags owners still hold and run without a cache. Every failing
    //! lookup must leave nothing behind, a half-parsed tag included (its name, tagger and
    //! message), and an id of another type than the repository's must be refused before
    //! it reaches C. libgit2 copies the id it is given (`BSan`: the tests overwrite and free
    //! it right after), and a tag keeps a bare pointer to its repository, which its target
    //! lookup follows across Rust reborrows of that repository (`BSan`). Tags are not
    //! `Send`, so the threaded test gives each thread its own repository (`TSan`).
    //!
    //! A tag cannot outlive its repository, nor a handle its owner, and no owner lends an
    //! exclusive handle (`compile_fail/` and doctests). Every test calls libgit2, so Miri
    //! skips them all.
    //!
    //! The getters (`id`, `name`, `message`, `tagger`, `target_id`, `target_kind`) lend
    //! what the tag holds for as long as the handle's borrow: the tests read every getter
    //! of every tag while other owners of the same tag come and go, and after the cache
    //! evicted it, as long as one owner remains (`ASan`); none outlives the last owner
    //! (doctests). A tagger is `Send` and `Sync` while the tag is not: threads read one
    //! tag's tagger at once while the tag's owner stays on its thread (`TSan`). `target`
    //! and `peel` hand out owners of their own that outlive the tag (`ASan`, `LSan`), looked
    //! up through the repository the tag keeps, across Rust reborrows of it (`BSan`), as
    //! `owner` hands that repository back.

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

    /// What [`read`] reads: id, name, message length, tagger string lengths and time,
    /// target id.
    type Read = (String, Vec<u8>, Option<usize>, Option<(usize, i64)>, String);

    /// Every getter's value, read through an owner while the others are dropped.
    fn read(tag: TagRef<'_, '_>) -> Read {
        let tagger = tag.tagger().map(|t| {
            let len = t.name().map_or(0, std::ffi::CStr::count_bytes)
                + t.email().map_or(0, std::ffi::CStr::count_bytes);
            (len, t.when().seconds())
        });
        let _ = tag.target_kind();
        (
            tag.id().to_hex_string().unwrap(),
            tag.name().to_bytes().to_vec(),
            tag.message().map(std::ffi::CStr::count_bytes),
            tagger,
            tag.target_id().to_hex_string().unwrap(),
        )
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_lend_what_the_last_owner_keeps() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        for hex_id in TAGS {
            let first = Tag::lookup(r, id(hex_id).as_ref()).unwrap();
            let expected = read(first.as_ref());
            let second = first.clone();
            let name = second.as_ref().name();
            let message = second.as_ref().message();
            let tagger = second.as_ref().tagger();
            let target_id = second.as_ref().target_id();
            let tag_id = second.as_ref().id();
            drop(first);
            let as_object = Tag::into_object(second.clone());
            assert_eq!(name.to_bytes(), expected.1);
            assert_eq!(message.map(std::ffi::CStr::count_bytes), expected.2);
            assert_eq!(tagger.map(|t| t.when().seconds()), expected.3.map(|t| t.1));
            assert_eq!(target_id.to_hex_string().unwrap(), expected.4);
            assert_eq!(tag_id.to_hex_string().unwrap(), hex_id);
            drop(as_object);
            assert_eq!(read(second.as_ref()), expected);
        }
    }

    /// With the cache emptied of the tag, the getters read the tag the owner keeps.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_survive_cache_eviction() {
        isolated(
            concat!(module_path!(), "::getters_survive_cache_eviction"),
            || {
                let repo = open_testrepo();
                let tag = Tag::lookup(repo.as_ref(), id(TAGS[0]).as_ref()).unwrap();
                let expected = read(tag.as_ref());
                set_cache_max_size(1);
                for hex_id in TAGS.iter().cycle().take(64) {
                    drop(Tag::lookup(repo.as_ref(), id(hex_id).as_ref()).unwrap());
                }
                assert_eq!(read(tag.as_ref()), expected);
            },
        );
    }

    /// `target` and `peel` hand out owners of their own, which outlive the tag; they
    /// look the target up through the tag's repository, reborrowed meanwhile, which
    /// `owner` hands back.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn targets_and_peels_outlive_their_tag() {
        let repo = open_testrepo();
        let mut kept: Vec<ObjectArc<'_>> = Vec::new();
        for hex_id in TAGS {
            let tag = Tag::lookup(repo.as_ref(), id(hex_id).as_ref()).unwrap();
            let owner = tag.as_ref().owner();
            assert_eq!(owner.path(), repo.as_ref().path());
            let target = tag.as_ref().target().unwrap();
            let peeled = owner.path().map(|_| tag.as_ref().peel().unwrap()).unwrap();
            assert_eq!(object_view(&target).1, tag.as_ref().target_kind().raw());
            drop(tag);
            assert_ne!(object_view(&peeled).1, ffi::GIT_OBJECT_TAG);
            kept.extend([target, peeled]);
        }
        let views: Vec<_> = kept.iter().map(object_view).collect();
        kept.reverse();
        drop(kept);
        assert_eq!(views.len(), 2 * TAGS.len());
    }

    /// A tagger is `Send` and `Sync`: threads read one tag's tagger, and its strings, at
    /// once, while the tag's owner stays on the main thread.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn taggers_are_read_from_many_threads() {
        let repo = open_testrepo();
        let tag = Tag::lookup(repo.as_ref(), id(TAGS[0]).as_ref()).unwrap();
        let tagger = tag.as_ref().tagger().unwrap();
        let expected = tagger.to_signature().unwrap();
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(move || {
                    for _ in 0..64 {
                        assert_eq!(tagger.name(), Some(c"Vicent Marti"));
                        assert_eq!(tagger.email(), Some(c"tanoku@gmail.com"));
                        assert_eq!(tagger.when().seconds(), 1_281_578_440);
                    }
                });
            }
        });
        assert_eq!(tagger, expected.as_ref());
        drop(tag);
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

    /// Every failure leaves no object behind (`LSan`), with its code and class; objects of
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
        for kind in [2, 0, 3, 0x7F, u8::MAX] {
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
    /// allocated (its name, a tagger it could not parse): `ASan` flags a bad free, `LSan` a
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
            probe.as_mut().as_mut().set_id([0xEE; 32]);
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

mod repository_tags {
    //! UB tests for `create_lightweight`, `create_from_buffer`, `delete`, `list`,
    //! `list_match` (`git_tag_create_lightweight`, `_create_from_buffer`, `_delete`,
    //! `_list`, `_list_match`) and `ForeachCallback` (`git_tag_foreach_cb`).
    //!
    //! Names, patterns and buffers sit in exact-length allocations (`ASan`: a read past
    //! their NUL) and are rewritten after each call (`BSan`: libgit2 keeps no pointer into
    //! them). A listing is an array and strings of libgit2's allocator that the returned
    //! owner releases once (`ASan`, `LSan`), and that outlives the repository. A buffer
    //! naming the zero id as its target, which libgit2 would answer by freeing an
    //! uninitialised pointer, and buffers libgit2 would leak part of (a malformed
    //! tagger, no message, an invalid reference name), never reach libgit2 (`ASan`,
    //! `LSan`). A failed listing leaks in libgit2 itself, so that test runs
    //! with `LeakSanitizer` off in its child. Each thread lists tags of a repository of
    //! its own, the first calls of a fresh process racing to initialise libgit2 (`TSan`);
    //! tests that write references run in a child, on one thread, since glibc's
    //! `tzset` (the reflog's time) is reported by `TSan` against any concurrent caller.
    //!
    //! A callback's name and id cannot outlive the call, nor the callback cross threads
    //! (doctests); a panicking closure resumes after the call.
    //!
    //! Then `create`, `create_annotation` and `foreach` (`git_tag_create`,
    //! `git_tag_annotation_create`, `git_tag_foreach`). A tagger reaches libgit2 as a
    //! borrowed `const git_signature *` it only prints: a view over strings the test
    //! rewrites and frees right after, an owned signature, and the tagger of a tag that
    //! is released after the call (`ASan`, `BSan`); names and messages up to a megabyte sit
    //! in exact-length buffers. `foreach` lends the closure each name and id for its
    //! call; the closure breaks with codes of either sign, panics, re-enters `foreach`
    //! with a callback of its own, and writes the reference database during the walk
    //! (deleting and creating tags), which libgit2 tolerates, failing the walk when a
    //! tag it was about to visit is gone. Walks run on threads at once, each over a
    //! repository of its own (`TSan`); tests that write references run in a child.

    use std::ffi::{CStr, CString, c_int};
    use std::num::NonZero;
    use std::ops::ControlFlow;
    use std::path::{Path, PathBuf};
    use std::thread;

    use libgit2::libgit2::core::types::{Signature, SignatureView, Time};
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::object::tag::{
        ForeachCallback, Tag, create, create_annotation, create_from_buffer, create_lightweight,
        delete, foreach, list, list_match,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidRef, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryRef};
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use crate::fixture::index::isolated_tolerating_leaks;
    use crate::fixture::{init, isolated};

    const COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// A private copy of `testrepo.git`, opened; removed on drop after the repository.
    struct Sandbox {
        path: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl Sandbox {
        fn new(tag: &str) -> Self {
            init();
            let path =
                std::env::temp_dir().join(format!("crustify-tag-ub-{}-{tag}", std::process::id()));
            drop(std::fs::remove_dir_all(&path));
            let source = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/c/tests/resources/testrepo.git"
            );
            copy_dir(Path::new(source), &path);
            let repo = Repository::open(&CString::new(path.to_str().unwrap()).unwrap()).unwrap();
            Self {
                path,
                repo: Some(repo),
            }
        }

        fn repo(&self) -> &RepositoryBox {
            self.repo.as_ref().unwrap()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            drop(self.repo.take());
            drop(std::fs::remove_dir_all(&self.path));
        }
    }

    fn commit(repo: &RepositoryBox) -> ObjectArc<'_> {
        let id = Oid::from_string(&CString::new(COMMIT).unwrap(), OidType::Sha1).unwrap();
        Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap()
    }

    /// Overwrites a string's bytes in place, keeping its length.
    fn scribble(s: CString) {
        let mut bytes = s.into_bytes();
        bytes.fill(b'#');
        drop(bytes);
    }

    /// Tags of names of many lengths are created, listed, matched and deleted; the
    /// listings outlive the repository, and every argument is rewritten after its call.
    /// In a child: writing a reference reads the time through glibc's `tzset`, which
    /// `TSan` reports against any concurrent caller.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn tags_are_created_listed_and_deleted() {
        isolated(
            concat!(module_path!(), "::tags_are_created_listed_and_deleted"),
            || {
                let sandbox = Sandbox::new("lifecycle");
                let repo = sandbox.repo();
                let target = commit(repo);
                let names: Vec<String> = [1usize, 2, 15, 16, 17, 100, 200]
                    .iter()
                    .map(|&n| "t".repeat(n))
                    .collect();
                for name in &names {
                    let c_name = CString::new(name.as_str()).unwrap();
                    let id =
                        create_lightweight(repo.as_ref(), &c_name, target.as_ref(), false).unwrap();
                    assert_eq!(id.as_ref().to_hex_string().unwrap(), COMMIT);
                    scribble(c_name);
                    let buffer = CString::new(format!(
                        "object {COMMIT}\ntype commit\ntag a{name}\ntagger T <t@x> 0 +0000\n\nm\n"
                    ))
                    .unwrap();
                    let id = create_from_buffer(repo.as_ref(), &buffer, false).unwrap();
                    assert!(!id.as_ref().is_zero());
                    scribble(buffer);
                }
                drop(target);
                let pattern = CString::new("t*").unwrap();
                let matched = list_match(repo.as_ref(), &pattern).unwrap();
                scribble(pattern);
                let all = list(repo.as_ref()).unwrap();
                assert_eq!(matched.as_ref().len(), names.len() + 2);
                assert_eq!(all.as_ref().len(), 7 + 2 * names.len());
                for name in &names {
                    let c_name = CString::new(name.as_str()).unwrap();
                    delete(repo.as_ref(), &c_name).unwrap();
                    let e = delete(repo.as_ref(), &c_name).unwrap_err();
                    assert_eq!(e.code(), Some(ErrorCode::NotFound));
                    scribble(c_name);
                }
                drop(sandbox);
                let listed: Vec<&CStr> = all.as_ref().iter().collect();
                assert!(listed.contains(&c"test"));
                assert!(
                    matched
                        .as_ref()
                        .iter()
                        .all(|n| n.to_bytes().starts_with(b"t"))
                );
            },
        );
    }

    /// Buffers naming the zero id, and buffers malformed after the tag's name, are
    /// refused before libgit2 frees an uninitialised pointer or leaks what it parsed.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn refused_buffers_never_reach_libgit2() {
        let sandbox = Sandbox::new("refused");
        let repo = sandbox.repo();
        let zero = "0".repeat(40);
        let buffers = [
            format!("object {zero}\ntype commit\ntag z\ntagger T <t@x> 0 +0000\n\nm\n"),
            format!("object {zero}\ntype tree\ntag z\n"),
            format!("object {COMMIT}\ntype commit\ntag v\ntagger nobody\n\nm\n"),
            format!("object {COMMIT}\ntype commit\ntag v\ntagger T <t@x> 0 +0000\nno blank line"),
            format!("object {COMMIT}\ntype commit\ntag v\ntagger T <t@x> notatime +0000\n\nm\n"),
            format!("object {COMMIT}\ntype commit\ntag bad..name\n\nm\n"),
        ];
        let classes = [
            ErrorClass::Odb,
            ErrorClass::Odb,
            ErrorClass::Invalid,
            ErrorClass::Tag,
            ErrorClass::Invalid,
            ErrorClass::Reference,
        ];
        for (buffer, class) in buffers.into_iter().zip(classes) {
            let buffer = CString::new(buffer).unwrap();
            for force in [false, true] {
                let e = create_from_buffer(repo.as_ref(), &buffer, force).unwrap_err();
                assert_eq!((e.raw_code(), e.class()), (-1, Some(class)), "{e:?}");
            }
            scribble(buffer);
        }
        assert_eq!(list(repo.as_ref()).unwrap().as_ref().len(), 7);
    }

    /// A dangling symbolic tag fails the listing with libgit2's error, and the next
    /// listing, once it is gone, succeeds; libgit2 leaks the names it collected.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failed_listings_report_errors() {
        isolated_tolerating_leaks(
            concat!(module_path!(), "::failed_listings_report_errors"),
            || {
                let sandbox = Sandbox::new("dangling");
                let dangling = sandbox.path.join("refs/tags/dangling");
                std::fs::write(&dangling, "ref: refs/heads/gone\n").unwrap();
                let repo = sandbox.repo();
                let e = list(repo.as_ref()).unwrap_err();
                assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Reference)));
                let e = list_match(repo.as_ref(), c"d*").unwrap_err();
                assert_eq!(e.raw_code(), -1);
                std::fs::remove_file(&dangling).unwrap();
                assert_eq!(list(repo.as_ref()).unwrap().as_ref().len(), 7);
            },
        );
    }

    /// The callback called directly with names of many lengths, each in an exact-length
    /// allocation rewritten after the call; a break and a panic each come back.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn callbacks_lend_names_and_ids_for_the_call() {
        init();
        let id = Oid::from_string(&CString::new(COMMIT).unwrap(), OidType::Sha1).unwrap();
        let mut seen = Vec::new();
        let mut collect = |name: &CStr, id: OidRef<'_>| {
            seen.push((name.to_bytes().len(), id.to_oid()));
            if name.to_bytes().ends_with(b"stop") {
                return ControlFlow::Break(NonZero::new(-7).unwrap());
            }
            ControlFlow::Continue(())
        };
        let mut callback = ForeachCallback::new(&mut collect);
        for len in [0usize, 1, 10, 255, 4096] {
            let name = CString::new(format!("refs/tags/{}", "n".repeat(len))).unwrap();
            assert_eq!(callback.call(&name, id.as_ref()), ControlFlow::Continue(()));
            scribble(name);
        }
        let stop = callback.call(c"refs/tags/stop", id.as_ref());
        assert_eq!(stop, ControlFlow::Break(NonZero::new(-7).unwrap()));
        drop(callback);
        assert_eq!(seen.len(), 6);
        assert!(seen.iter().all(|(_, seen_id)| *seen_id == id));

        let mut boom = |_: &CStr, _: OidRef<'_>| -> ControlFlow<NonZero<c_int>> { panic!("boom") };
        let mut callback = ForeachCallback::new(&mut boom);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callback.call(c"refs/tags/x", id.as_ref())
        }));
        assert!(caught.is_err());
        assert_eq!(
            format!("{callback:?}"),
            "ForeachCallback { stopped: Some(-7), panicked: false, .. }"
        );
    }

    /// Threads of a fresh process list and match tags at once, each in a repository of its
    /// own, racing libgit2's initialisation and allocator (`TSan`); the tags are created
    /// and deleted on the main thread, before and after, since writing a reference reads
    /// the time through glibc's `tzset`, which `TSan` reports against any concurrent caller.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn tags_on_threads() {
        isolated(concat!(module_path!(), "::tags_on_threads"), || {
            let sandboxes: Vec<Sandbox> = (0..4)
                .map(|i| Sandbox::new(&format!("thread-{i}")))
                .collect();
            for (i, sandbox) in sandboxes.iter().enumerate() {
                let repo = sandbox.repo();
                let target = commit(repo);
                for j in 0..8 {
                    let name = CString::new(format!("t{i}-{j}")).unwrap();
                    create_lightweight(repo.as_ref(), &name, target.as_ref(), false).unwrap();
                }
            }
            let paths: Vec<CString> = sandboxes
                .iter()
                .map(|s| CString::new(s.path.to_str().unwrap()).unwrap())
                .collect();
            thread::scope(|s| {
                for (i, path) in paths.iter().enumerate() {
                    s.spawn(move || {
                        let repo = Repository::open(path).unwrap();
                        for j in 0..8 {
                            let name = CString::new(format!("t{i}-{j}")).unwrap();
                            assert_eq!(list_match(repo.as_ref(), &name).unwrap().as_ref().len(), 1);
                        }
                        assert_eq!(list(repo.as_ref()).unwrap().as_ref().len(), 15);
                    });
                }
            });
            for (i, sandbox) in sandboxes.iter().enumerate() {
                for j in 0..8 {
                    let name = CString::new(format!("t{i}-{j}")).unwrap();
                    delete(sandbox.repo().as_ref(), &name).unwrap();
                }
                assert_eq!(list(sandbox.repo().as_ref()).unwrap().as_ref().len(), 7);
            }
        });
    }

    /// Every tag's full name, as one walk sees them.
    fn walked(repo: RepositoryRef<'_>) -> Vec<CString> {
        let mut names = Vec::new();
        let mut f = |name: &CStr, _: OidRef<'_>| {
            names.push(name.to_owned());
            ControlFlow::Continue(())
        };
        let mut callback = ForeachCallback::new(&mut f);
        assert_eq!(foreach(repo, &mut callback), Ok(ControlFlow::Continue(())));
        drop(callback);
        names
    }

    /// Annotated tags and tag objects, with taggers of every form, names and messages of
    /// many lengths; every argument is rewritten or released after its call. In a child:
    /// writing a reference reads the time through glibc's `tzset`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn annotated_tags_borrow_their_arguments_for_the_call() {
        isolated(
            concat!(
                module_path!(),
                "::annotated_tags_borrow_their_arguments_for_the_call"
            ),
            || {
                let sandbox = Sandbox::new("annotated");
                let repo = sandbox.repo();
                let target = commit(repo);
                let owned = Signature::new(c"Owned", c"owned@example.com", 1, -60).unwrap();
                for (i, len) in [0usize, 1, 40, 4096, 1 << 20].into_iter().enumerate() {
                    let name = CString::new(format!("a{i}-{}", "n".repeat(len.min(200)))).unwrap();
                    let message = CString::new("m".repeat(len)).unwrap();
                    let (who, mail) = (
                        CString::new("w".repeat(len + 1)).unwrap(),
                        CString::new(format!("{}@example.com", "e".repeat(len))).unwrap(),
                    );
                    let view =
                        SignatureView::new(&who, &mail, Time::new(i64::try_from(i).unwrap(), 0));
                    let id = create(
                        repo.as_ref(),
                        &name,
                        target.as_ref(),
                        view.as_ref(),
                        &message,
                        false,
                    )
                    .unwrap();
                    let again = create_annotation(
                        repo.as_ref(),
                        &name,
                        target.as_ref(),
                        view.as_ref(),
                        &message,
                    )
                    .unwrap();
                    assert_eq!(id, again);
                    scribble(who);
                    scribble(mail);
                    let e = create(
                        repo.as_ref(),
                        &name,
                        target.as_ref(),
                        owned.as_ref(),
                        &message,
                        false,
                    )
                    .unwrap_err();
                    assert_eq!(e.code(), Some(ErrorCode::Exists));
                    let replaced = create(
                        repo.as_ref(),
                        &name,
                        target.as_ref(),
                        owned.as_ref(),
                        &message,
                        true,
                    )
                    .unwrap();
                    scribble(name);
                    scribble(message);
                    let tag = Tag::lookup(repo.as_ref(), replaced.as_ref()).unwrap();
                    assert_eq!(tag.as_ref().message().map(CStr::count_bytes), Some(len));
                    assert_eq!(tag.as_ref().tagger().unwrap(), owned.as_ref());
                }
                // A tagger borrowed from a tag released right after the call.
                let parsed = Tag::lookup(
                    repo.as_ref(),
                    Oid::from_string(c"b25fa35b38051e4ae45d4222e795f9df2e43f1d1", OidType::Sha1)
                        .unwrap()
                        .as_ref(),
                )
                .unwrap();
                let id = create(
                    repo.as_ref(),
                    c"parsed",
                    parsed.as_ref().as_object(),
                    parsed.as_ref().tagger().unwrap(),
                    c"of a tag\n",
                    false,
                )
                .unwrap();
                drop(parsed);
                let tag = Tag::lookup(repo.as_ref(), id.as_ref()).unwrap();
                assert_eq!(tag.as_ref().tagger().unwrap().name(), Some(c"Vicent Marti"));
                assert_eq!(walked(repo.as_ref()).len(), 13);
            },
        );
    }

    /// The closure breaks with codes of either sign, panics, re-enters `foreach`, and
    /// deletes and creates tags during the walk. In a child, which writes references.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn walks_lend_names_and_ids_and_survive_their_closure() {
        isolated(
            concat!(
                module_path!(),
                "::walks_lend_names_and_ids_and_survive_their_closure"
            ),
            || {
                let sandbox = Sandbox::new("walks");
                let repo = sandbox.repo();
                let r = repo.as_ref();
                assert_eq!(walked(r).len(), 7);
                for code in [1, -1, c_int::MAX, c_int::MIN, ErrorCode::User.raw()] {
                    let mut calls = 0;
                    let mut f = |_: &CStr, _: OidRef<'_>| {
                        calls += 1;
                        ControlFlow::Break(NonZero::new(code).unwrap())
                    };
                    let mut callback = ForeachCallback::new(&mut f);
                    let flow = foreach(r, &mut callback).unwrap();
                    drop(callback);
                    assert_eq!(flow, ControlFlow::Break(NonZero::new(code).unwrap()));
                    assert_eq!(calls, 1);
                }

                // Re-entered: each tag walks every tag again.
                let mut nested = 0;
                let mut outer = |_: &CStr, _: OidRef<'_>| {
                    nested += walked(r).len();
                    ControlFlow::Continue(())
                };
                let mut callback = ForeachCallback::new(&mut outer);
                assert_eq!(foreach(r, &mut callback), Ok(ControlFlow::Continue(())));
                drop(callback);
                assert_eq!(nested, 49);

                // A panic resumes after the walk, which leaves nothing behind.
                let mut boom =
                    |_: &CStr, _: OidRef<'_>| -> ControlFlow<NonZero<c_int>> { panic!("boom") };
                let mut callback = ForeachCallback::new(&mut boom);
                let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    foreach(r, &mut callback)
                }));
                assert!(caught.is_err());
                drop(callback);

                // Writes during the walk: the first call deletes every later tag and
                // creates new ones; libgit2 fails the walk at the first tag it misses.
                let target = commit(repo);
                let mut first = true;
                let mut seen = 0;
                let mut writer = |name: &CStr, id: OidRef<'_>| {
                    seen += 1;
                    let _ = id.to_oid();
                    if std::mem::take(&mut first) {
                        for other in walked(r) {
                            if other.as_c_str() != name {
                                let short = &other.to_bytes()["refs/tags/".len()..];
                                delete(r, &CString::new(short).unwrap()).unwrap();
                            }
                        }
                        create_lightweight(r, c"zz-new", target.as_ref(), false).unwrap();
                    }
                    ControlFlow::Continue(())
                };
                let mut callback = ForeachCallback::new(&mut writer);
                let result = foreach(r, &mut callback);
                drop(callback);
                assert!(seen >= 1);
                if let Err(e) = result {
                    assert_eq!(e.code(), Some(ErrorCode::NotFound));
                }
                assert_eq!(walked(r).len(), 2);
            },
        );
    }

    /// Threads walk at once, each over a repository of its own, the first walks of a
    /// fresh process racing to initialise libgit2.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn walks_on_threads() {
        isolated(concat!(module_path!(), "::walks_on_threads"), || {
            let sandboxes: Vec<Sandbox> = (0..4)
                .map(|i| Sandbox::new(&format!("walk-thread-{i}")))
                .collect();
            let paths: Vec<CString> = sandboxes
                .iter()
                .map(|s| CString::new(s.path.to_str().unwrap()).unwrap())
                .collect();
            thread::scope(|s| {
                for path in &paths {
                    s.spawn(move || {
                        let repo = Repository::open(path).unwrap();
                        for _ in 0..8 {
                            assert_eq!(walked(repo.as_ref()).len(), 7);
                        }
                    });
                }
            });
        });
    }
}
