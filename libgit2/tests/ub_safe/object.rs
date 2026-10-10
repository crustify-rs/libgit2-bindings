//! UB tests for `git2/object.h`: the `git_object` owner, `ObjectArc<'r>`, its
//! constructors `Object::lookup`, `Object::lookup_prefix`, `ObjectRef::peel` and
//! `ObjectRef::lookup_bypath`, and its handles.
//!
//! An object is counted, and shared with its repository's cache, which hands the same
//! object to every lookup of one id (blobs aside, which it does not keep): every owner
//! gives back its one reference through `git_object_free` exactly once, and every clone
//! takes one through `git_object_dup`.
//! A miscount frees an object the cache or another owner still reads, which ASan
//! flags, or leaks it, which LSan does. The tests share objects between lookups and
//! clones, drop them in every order, let the cache evict objects owners still hold (a
//! tiny cache limit) and run with the cache off (a fresh object per lookup), and C
//! reads each survivor back. Every failing lookup must leave nothing behind, and an id
//! of another type than the repository's, which libgit2 would format out of bounds,
//! must be refused before it reaches C. libgit2 copies the id it is given: the tests
//! overwrite and free each id right after its lookup (BSan). A peel or a lookup by
//! path derives one object from another through the repository: the derived object
//! owns one reference of its own, whether it is a fresh object, a cached one or the
//! very object it was derived from, and outlives the object it came from, the trees
//! read on the way, and cache eviction.
//!
//! An object is neither `Send` nor `Sync`, so there is no thread test (its owner and
//! shared handle staying on their thread is a doctest each); it, its handles and its
//! clones cannot outlive the repository, nor can a handle outlive its owner, and no
//! owner lends an exclusive handle (`compile_fail/` and doctests).
//!
//! The id options, `ObjectIdOptions<'f>`, are Rust storage that libgit2 reads in place,
//! holding a filter list it applies through them: the tests set every object and id
//! type and a list (empty, or `crlf`) through the safe setters, have C hash content
//! through the options (the fixture's `object_id`, standing in for the
//! `git_object_id_from_buffer` wrapper) between Rust's own shared and exclusive
//! reborrows of them (BorrowSanitizer), and drop the options before the list and the
//! list before its repository, which the borrows enforce: outliving the list, or
//! reaching it otherwise while the options hold it, does not compile
//! (`compile_fail/object_id_options_*` and the doctests on `ObjectIdOptions`). A list
//! read back through the options is the one lent, and C applies it while it lives
//! (ASan). Loading a filter list initialises libgit2's filters unsynchronised (see
//! `filter.rs`), so those tests run alone in a child process.
//!
//! Every test calls libgit2, so Miri skips them all.

use std::ffi::{CStr, CString};

use libgit2::ffi;
use libgit2::libgit2::core::types::ObjectType;
use libgit2::libgit2::filter::filter::FilterList;
use libgit2::libgit2::object::object::{Object, ObjectArc, ObjectIdOptions, ObjectIdOptionsMut};
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::{
    Repository, RepositoryBox, RepositoryError, RepositoryRef,
};
use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

use crate::fixture::object_and_tree_entry::{
    enable_caching, object_view, open_testrepo, repository, set_cache_max_size, write_raw,
};
use crate::fixture::{TempRepo, isolated, object_id};

const COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
const BLOB: &str = "a8233120f6ad708f843d861ce2b7228ec4e3dec6";
const TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";

/// Commits of `testrepo.git` and their trees.
const MANY: [&str; 16] = [
    "d07b0f9a8c89f1d9e74dc4fce6421dec5ef8a659",
    "2d59075e0681f540482d4f6223a68e0fef790bc7",
    "258f0e2a959a364e40ed6603d5d44fbb24765b10",
    "1b8cbad43e867676df601306689fe7c3def5e689",
    "763d71aadf09a7951596c9746c024e7eece7c7af",
    "ae90f12eea699729ed24555e40b9fd669da12a12",
    "a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
    "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162",
    "e90810b8df3e80c413d903f631643c716887138d",
    "53fc32d17276939fc79ed05badaef2db09990016",
    "6dcf9bf7541ee10456529833502442f385010c3d",
    "bed08a0b30b72a9d4aed7f1af8c8ca124e8d64b9",
    "be3563ae3f795b2b4353bcce3a527ad0a4f7f644",
    "1810dff58d8a660512d4832e740f692884338ccd",
    "c47800c7266a2be04c571c04d5a6614691ea99bd",
    "75057dd4114e74cca1d750d0aee1647c903cb60a",
];

fn id(hex: &str) -> Oid {
    Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap()
}

fn hex(object: &ObjectArc<'_>) -> String {
    object_view(object).0
}

/// Whether `owner` is the repository `repo` holds: one `git_repository` lends one copy
/// of its path, which another repository over the same directory does not share.
fn is_owner(owner: RepositoryRef<'_>, repo: &RepositoryBox) -> bool {
    owner.path().map(CStr::as_ptr) == repo.as_ref().path().map(CStr::as_ptr)
}

fn assert_failed(result: Result<ObjectArc<'_>, RepositoryError>, code: i32, class: ErrorClass) {
    let e = result.unwrap_err();
    assert_eq!((e.raw_code(), e.class()), (code, Some(class)), "{e:?}");
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn lookups_and_clones_share_one_object() {
    let repo = open_testrepo();
    let r = repo.as_ref();
    let cases = [
        (COMMIT, ObjectType::Commit),
        (TREE, ObjectType::Tree),
        (BLOB, ObjectType::Blob),
        (TAG, ObjectType::Tag),
    ];
    for (hex_id, kind) in cases {
        let any = Object::lookup(r, id(hex_id).as_ref(), None).unwrap();
        let typed = Object::lookup(r, id(hex_id).as_ref(), Some(kind)).unwrap();
        let short = Object::lookup_prefix(r, id(hex_id).as_ref(), 7, Some(kind)).unwrap();
        // The cache keeps no blob by default: each lookup parses a fresh one.
        let cached = kind != ObjectType::Blob;
        assert_eq!(ObjectArc::ptr_eq(&any, &typed), cached);
        assert_eq!(ObjectArc::ptr_eq(&any, &short), cached);
        assert_eq!(object_view(&any), (hex_id.to_owned(), kind.raw()));
        assert!(is_owner(any.as_ref().owner(), &repo));
        let clone = typed.clone();
        let copy = short.as_ref().to_object();
        drop((any, typed, short));
        assert_eq!(hex(&clone), hex_id);
        drop(clone);
        assert_eq!(hex(&copy), hex_id);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn clone_chains_release_every_reference_in_any_order() {
    let repo = open_testrepo();
    for order in 0..3 {
        let first = Object::lookup(repo.as_ref(), id(TREE).as_ref(), None).unwrap();
        let mut chain = vec![first];
        for i in 0..16 {
            let next = if i % 2 == 0 {
                chain.last().unwrap().clone()
            } else {
                chain.last().unwrap().as_ref().to_object()
            };
            chain.push(next);
        }
        match order {
            0 => chain.reverse(),
            1 => chain.rotate_left(7),
            _ => {}
        }
        while let Some(object) = chain.pop() {
            assert_eq!(hex(&object), TREE);
            drop(object);
            if let Some(survivor) = chain.first() {
                assert_eq!(hex(survivor), TREE);
            }
        }
    }
}

/// Every failure leaves no object behind (LSan), and its code and class.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn failures_leave_nothing_behind() {
    let repo = open_testrepo();
    let r = repo.as_ref();
    let not_found = ErrorCode::NotFound.raw();
    let ambiguous = ErrorCode::Ambiguous.raw();
    assert_failed(
        Object::lookup(r, id(COMMIT).as_ref(), Some(ObjectType::Tree)),
        not_found,
        ErrorClass::Invalid,
    );
    assert_failed(
        Object::lookup(r, id(&"de".repeat(20)).as_ref(), None),
        not_found,
        ErrorClass::Odb,
    );
    assert_failed(
        Object::lookup(r, Oid::zero(OidType::Sha1).as_ref(), None),
        not_found,
        ErrorClass::Odb,
    );
    for len in [0, 1, 3] {
        assert_failed(
            Object::lookup_prefix(r, id(COMMIT).as_ref(), len, None),
            ambiguous,
            ErrorClass::Object,
        );
    }
    // `1810` and `3d35` each start two objects, of the same type and not.
    for (prefix, kind) in [
        ("1810", None),
        ("1810", Some(ObjectType::Tree)),
        ("3d35", Some(ObjectType::Blob)),
    ] {
        let prefix = Oid::from_prefix(prefix.as_bytes(), OidType::Sha1).unwrap();
        assert_failed(
            Object::lookup_prefix(r, prefix.as_ref(), 4, kind),
            ambiguous,
            ErrorClass::Odb,
        );
    }
    let blob = id(BLOB);
    assert_failed(
        Object::lookup_prefix(r, blob.as_ref(), 7, Some(ObjectType::Commit)),
        not_found,
        ErrorClass::Invalid,
    );

    // A repository without a git directory has no object database to load.
    let pathless = Repository::new().unwrap();
    assert_failed(
        Object::lookup(pathless.as_ref(), id(COMMIT).as_ref(), None),
        not_found,
        ErrorClass::Invalid,
    );
    assert_failed(
        Object::lookup_prefix(pathless.as_ref(), id(COMMIT).as_ref(), 7, None),
        not_found,
        ErrorClass::Invalid,
    );

    // Ids of another type, at every length, never reach C.
    let mut wide = Oid::from_raw(&id(COMMIT).as_ref().id(), OidType::Sha256).unwrap();
    for kind in [2, 0, 3, 0x7f, u8::MAX] {
        wide.as_mut().set_raw_kind(kind);
        assert_failed(
            Object::lookup(r, wide.as_ref(), None),
            -1,
            ErrorClass::Invalid,
        );
        for len in [0, 4, 7, 39, 40, 41, 64, 65, usize::MAX] {
            assert_failed(
                Object::lookup_prefix(r, wide.as_ref(), len, None),
                -1,
                ErrorClass::Invalid,
            );
        }
    }
    // The repository still answers.
    assert_eq!(
        hex(&Object::lookup(r, id(COMMIT).as_ref(), None).unwrap()),
        COMMIT
    );
}

/// Prefix lengths past the full id are clamped to it; odd lengths mask the last digit.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn prefix_lengths_from_four_to_past_the_end() {
    let repo = open_testrepo();
    let full = Object::lookup(repo.as_ref(), id(COMMIT).as_ref(), None).unwrap();
    for len in [4, 5, 7, 8, 21, 39, 40, 41, 64, 4096, usize::MAX] {
        let mut probe = id(COMMIT);
        if len < 40 {
            // Garbage past the prefix: it must be ignored.
            let mut bytes = probe.as_ref().id();
            bytes[len.div_ceil(2)..20].fill(0x5a);
            if len % 2 == 1 {
                bytes[len / 2] |= 0x0f;
            }
            probe = Oid::from_raw(&bytes[..20], OidType::Sha1).unwrap();
        }
        let found = Object::lookup_prefix(repo.as_ref(), probe.as_ref(), len, None).unwrap();
        assert!(ObjectArc::ptr_eq(&found, &full), "len {len}");
    }
}

/// libgit2 copies the id it is given: each one is overwritten and freed right after
/// its lookup, before the objects are read back and looked up again.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn lookups_retain_no_id() {
    let repo = open_testrepo();
    let mut held = Vec::new();
    for (i, hex_id) in MANY.iter().enumerate() {
        let mut probe = Box::new(id(hex_id));
        let object = if i % 2 == 0 {
            Object::lookup(repo.as_ref(), probe.as_ref().as_ref(), None)
        } else {
            Object::lookup_prefix(repo.as_ref(), probe.as_ref().as_ref(), 10, None)
        }
        .unwrap();
        probe.as_mut().as_mut().set_id([0xee; 32]);
        drop(probe);
        held.push((hex_id, object));
    }
    for (hex_id, object) in &held {
        assert_eq!(hex(object), **hex_id);
        let again = Object::lookup(repo.as_ref(), id(hex_id).as_ref(), None).unwrap();
        assert!(ObjectArc::ptr_eq(&again, object));
    }
}

/// Two repositories over the same directory have two caches, and so two objects for
/// one id, each reaching its own repository.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn each_repository_has_its_own_objects() {
    let (a, b) = (open_testrepo(), open_testrepo());
    let from_a = Object::lookup(a.as_ref(), id(COMMIT).as_ref(), None).unwrap();
    let from_b = Object::lookup(b.as_ref(), id(COMMIT).as_ref(), None).unwrap();
    assert!(!ObjectArc::ptr_eq(&from_a, &from_b));
    assert!(is_owner(from_a.as_ref().owner(), &a));
    assert!(is_owner(from_b.as_ref().owner(), &b));
    assert!(!is_owner(from_a.as_ref().owner(), &b));
    drop(from_a);
    drop(a);
    assert_eq!(hex(&from_b), COMMIT);
}

/// With a cache limit of one byte, the cache evicts objects as fast as they come,
/// while owners still hold them: the owners' references keep them alive.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owners_outlive_cache_eviction() {
    isolated(
        concat!(module_path!(), "::owners_outlive_cache_eviction"),
        || {
            let repo = open_testrepo();
            set_cache_max_size(1);
            for round in 0..4 {
                let held: Vec<_> = MANY
                    .iter()
                    .map(|h| Object::lookup(repo.as_ref(), id(h).as_ref(), None).unwrap())
                    .collect();
                let clones: Vec<_> = held.iter().rev().map(ObjectArc::clone).collect();
                for (object, h) in held.iter().zip(MANY) {
                    assert_eq!(hex(object), h, "round {round}");
                }
                drop(held);
                for (object, h) in clones.iter().rev().zip(MANY) {
                    assert_eq!(hex(object), h, "round {round}");
                }
            }
        },
    );
}

/// With caching off, every lookup parses a fresh object, which its owners alone
/// count; dropping the last frees it.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn without_a_cache_each_lookup_is_a_fresh_object() {
    isolated(
        concat!(
            module_path!(),
            "::without_a_cache_each_lookup_is_a_fresh_object"
        ),
        || {
            let repo = open_testrepo();
            enable_caching(false);
            let first = Object::lookup(repo.as_ref(), id(TREE).as_ref(), None).unwrap();
            let second = Object::lookup(repo.as_ref(), id(TREE).as_ref(), None).unwrap();
            assert!(!ObjectArc::ptr_eq(&first, &second));
            let clone = first.clone();
            assert!(ObjectArc::ptr_eq(&first, &clone));
            drop(first);
            assert_eq!(hex(&clone), TREE);
            drop(second);
            assert_eq!(hex(&clone), TREE);
        },
    );
}

/// An object the database holds but libgit2 cannot parse fails its lookup, through
/// every constructor and type filter, and its half-parsed object is freed (a tree's
/// entries included): ASan flags a bad free, LSan a leak. A well-formed one beside it
/// is still found.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn unparsable_objects_fail_and_leave_nothing_behind() {
    let repo = TempRepo::new("object-unparsable");
    let r = repository(&repo);
    let corrupt = [
        (
            b"tree nope\n".as_slice(),
            ffi::GIT_OBJECT_COMMIT,
            ObjectType::Commit,
        ),
        (b"100644 one\0short", ffi::GIT_OBJECT_TREE, ObjectType::Tree),
        (
            b"100644 a\0aaaaaaaaaaaaaaaaaaaa100644 b",
            ffi::GIT_OBJECT_TREE,
            ObjectType::Tree,
        ),
        (b"object 1234\n", ffi::GIT_OBJECT_TAG, ObjectType::Tag),
    ];
    for (data, raw, kind) in corrupt {
        let id = write_raw(&repo, data, raw);
        for result in [
            Object::lookup(r, id.as_ref(), None),
            Object::lookup(r, id.as_ref(), Some(kind)),
            Object::lookup_prefix(r, id.as_ref(), 12, None),
        ] {
            let e = result.unwrap_err();
            assert_eq!(e.raw_code(), -1, "{e:?}");
        }
    }
    let fine = write_raw(&repo, b"fine\n", ffi::GIT_OBJECT_BLOB);
    let blob = Object::lookup(r, fine.as_ref(), Some(ObjectType::Blob)).unwrap();
    assert_eq!(object_view(&blob).1, ffi::GIT_OBJECT_BLOB);
}

/// What a type mismatch reports, for the record: libgit2's wording.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn type_mismatch_message() {
    let repo = open_testrepo();
    let e = Object::lookup(repo.as_ref(), id(BLOB).as_ref(), Some(ObjectType::Tag)).unwrap_err();
    assert_eq!(
        e.message(),
        c"the requested type does not match the type in the ODB"
    );
    assert_eq!(e.code(), Some(ErrorCode::NotFound));
}

/// Nested tag `b25fa35` (`refs/tags/test`), of tag `7b43849`, of commit `e90810b`.
const NESTED_TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";
const INNER_TAG: &str = "7b4384978d2493e851f9cca7858815fac9b10980";
/// Commit `763d71a` (`refs/heads/subtrees`), whose tree nests `ab/de/fgh/1.txt`.
const SUBTREES: &str = "763d71aadf09a7951596c9746c024e7eece7c7af";

/// Peels and lookups by path count one reference each on what they find, which may
/// be the object they started from (a peel to its own type), a cached object another
/// owner holds, or a fresh one: every chain is released in every order, after the
/// object it was derived from, and C reads each survivor back.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn derived_objects_count_their_own_reference() {
    let repo = open_testrepo();
    let tag = Object::lookup(repo.as_ref(), id(NESTED_TAG).as_ref(), None).unwrap();
    let same = tag.as_ref().peel(Some(ObjectType::Tag)).unwrap();
    assert!(ObjectArc::ptr_eq(&same, &tag));
    // `None` peels through the whole tag chain, to the commit.
    let commit = tag.as_ref().peel(None).unwrap();
    assert!(!ObjectArc::ptr_eq(&commit, &tag));
    assert_eq!(object_view(&commit).1, ffi::GIT_OBJECT_COMMIT);
    let inner = Object::lookup(repo.as_ref(), id(INNER_TAG).as_ref(), None).unwrap();
    let commit_again = inner.as_ref().peel(Some(ObjectType::Commit)).unwrap();
    assert!(ObjectArc::ptr_eq(&commit, &commit_again));
    assert!(ObjectArc::ptr_eq(
        &commit,
        &tag.as_ref().peel(Some(ObjectType::Commit)).unwrap()
    ));
    let tree = commit.as_ref().peel(None).unwrap();
    let tree_direct = tag.as_ref().peel(Some(ObjectType::Tree)).unwrap();
    assert!(ObjectArc::ptr_eq(&tree, &tree_direct));
    let looked_up = Object::lookup(repo.as_ref(), id(&hex(&tree)).as_ref(), None).unwrap();
    assert!(ObjectArc::ptr_eq(&tree, &looked_up));
    let readme = tag.as_ref().lookup_bypath(c"readme.txt", None).unwrap();
    let readme_blob = tree
        .as_ref()
        .lookup_bypath(c"readme.txt", Some(ObjectType::Blob))
        .unwrap();
    assert_eq!(object_view(&readme), object_view(&readme_blob));
    assert_eq!(object_view(&readme).1, ffi::GIT_OBJECT_BLOB);
    // Blobs are not cached: each lookup by path parses a fresh one.
    assert!(!ObjectArc::ptr_eq(&readme, &readme_blob));

    let mut chain = vec![
        tag,
        same,
        inner,
        commit,
        commit_again,
        tree,
        tree_direct,
        looked_up,
        readme,
        readme_blob,
    ];
    let views: Vec<_> = chain.iter().map(object_view).collect();
    for order in 0..3 {
        let copies: Vec<_> = chain.iter().map(|o| o.as_ref().to_object()).collect();
        match order {
            0 => chain.reverse(),
            1 => chain.rotate_left(3),
            _ => {}
        }
        while let Some(object) = chain.pop() {
            let _ = object_view(&object);
        }
        chain = copies;
        assert_eq!(chain.iter().map(object_view).collect::<Vec<_>>(), views);
    }
    drop(chain);
    drop(repo);
}

/// Every refused peel and lookup by path leaves nothing behind (LSan), the trees and
/// tags read on the way included; the error is libgit2's, with its code.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn refused_derivations_leave_nothing_behind() {
    let repo = open_testrepo();
    let blob = Object::lookup(repo.as_ref(), id(BLOB).as_ref(), None).unwrap();
    let tree = Object::lookup(repo.as_ref(), id(TREE).as_ref(), None).unwrap();
    let commit = Object::lookup(repo.as_ref(), id(SUBTREES).as_ref(), None).unwrap();
    let tag = Object::lookup(repo.as_ref(), id(NESTED_TAG).as_ref(), None).unwrap();
    let kinds = [
        None,
        Some(ObjectType::Commit),
        Some(ObjectType::Tree),
        Some(ObjectType::Blob),
        Some(ObjectType::Tag),
    ];
    for kind in kinds {
        if kind != Some(ObjectType::Blob) {
            assert_failed(blob.as_ref().peel(kind), -12, ErrorClass::Object);
        }
        if kind != Some(ObjectType::Tree) {
            assert_failed(tree.as_ref().peel(kind), -12, ErrorClass::Object);
        }
        if matches!(kind, Some(ObjectType::Blob | ObjectType::Tag)) {
            assert_failed(commit.as_ref().peel(kind), -12, ErrorClass::Object);
        }
    }
    assert_failed(
        tag.as_ref().peel(Some(ObjectType::Blob)),
        -19,
        ErrorClass::Object,
    );
    for path in [
        c"",
        c"/",
        c"nope",
        c"ab/nope",
        c"ab/de/fgh/nope",
        c"README/",
        c"ab//de",
    ] {
        for kind in kinds {
            assert_failed(
                commit.as_ref().lookup_bypath(path, kind),
                -3,
                ErrorClass::Tree,
            );
            assert_failed(tag.as_ref().lookup_bypath(path, kind), -3, ErrorClass::Tree);
            assert_failed(
                blob.as_ref().lookup_bypath(path, kind),
                -12,
                ErrorClass::Object,
            );
        }
    }
    for (path, kind) in [
        (c"README", ObjectType::Tree),
        (c"ab", ObjectType::Blob),
        (c"ab/de/fgh/1.txt", ObjectType::Commit),
    ] {
        let e = commit.as_ref().lookup_bypath(path, Some(kind)).unwrap_err();
        assert_eq!(e.code(), Some(ErrorCode::InvalidSpec), "{e:?}");
        assert_eq!(e.class(), Some(ErrorClass::Object));
    }
    let deep = commit
        .as_ref()
        .lookup_bypath(c"ab/de/fgh/1.txt", Some(ObjectType::Blob))
        .unwrap();
    assert_eq!(object_view(&deep).1, ffi::GIT_OBJECT_BLOB);
    drop((blob, tree, commit, tag));
    assert_eq!(hex(&deep), "1f67fc4386b2d171e0d21be1c447e12660561f9b");
    drop(deep);
    drop(repo);
}

/// Derived objects outlive the eviction of what they came from and of the subtrees
/// read on the way: with a one-byte cache, every peel step and path component is a
/// fresh parse, which the result alone keeps alive.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn derived_objects_outlive_cache_eviction() {
    isolated(
        concat!(module_path!(), "::derived_objects_outlive_cache_eviction"),
        || {
            let repo = open_testrepo();
            set_cache_max_size(1);
            for round in 0..3 {
                let tag = Object::lookup(repo.as_ref(), id(NESTED_TAG).as_ref(), None).unwrap();
                let tree = tag.as_ref().peel(Some(ObjectType::Tree)).unwrap();
                let commit = Object::lookup(repo.as_ref(), id(SUBTREES).as_ref(), None).unwrap();
                let deep = commit
                    .as_ref()
                    .lookup_bypath(c"ab/de/fgh/1.txt", None)
                    .unwrap();
                let sub = commit
                    .as_ref()
                    .lookup_bypath(c"ab/de", Some(ObjectType::Tree))
                    .unwrap();
                let from_sub = sub.as_ref().lookup_bypath(c"fgh/1.txt", None).unwrap();
                drop(tag);
                drop(commit);
                assert_eq!(object_view(&tree).1, ffi::GIT_OBJECT_TREE, "round {round}");
                assert_eq!(hex(&deep), hex(&from_sub));
                drop(sub);
                assert_eq!(hex(&from_sub), "1f67fc4386b2d171e0d21be1c447e12660561f9b");
            }
        },
    );
}

/// `a\nb\n` and `a\r\nb\r\n` as blobs, as `git hash-object --no-filters` hashes them.
const LF: &str = "422c2b7ab3b3c668038da977e4e93a5fc623169c";
const CRLF: &str = "c30dea8a3641ea99b125d04d599d843712292759";

/// Every object and id type, and the defaults, reach C through the options, which
/// hashes or refuses each; Rust's reborrows of the options interleave with C's reads
/// of them (BorrowSanitizer), and the getters read back what the setters wrote.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn id_options_are_read_by_c_between_reborrows() {
    crate::fixture::init();
    let mut opts = ObjectIdOptions::new();
    let blob = object_id(&mut opts, b"hello\n").unwrap();
    assert_eq!(blob, "ce013625030ba8dba906f756967f9e9ca394464a");
    assert_eq!(
        object_id(&mut opts, b"").unwrap(),
        "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
    );
    let kinds = [
        None,
        Some(ObjectType::Commit),
        Some(ObjectType::Tree),
        Some(ObjectType::Blob),
        Some(ObjectType::Tag),
    ];
    for kind in kinds {
        for oid in [None, Some(OidType::Sha1), Some(OidType::Sha256)] {
            // A shared reborrow before the exclusive one, which ends it.
            let _ = opts.as_ref().filters();
            {
                let mut m: ObjectIdOptionsMut<'_, '_> = opts.as_mut();
                m.set_object_type(kind);
                m.as_mut().set_oid_type(oid);
                assert_eq!(
                    (m.as_ref().object_type(), m.as_ref().oid_type()),
                    (kind, oid)
                );
            }
            let result = object_id(&mut opts, b"hello\n");
            let read = opts.as_ref();
            assert_eq!((read.object_type(), read.oid_type()), (kind, oid));
            let id = result.unwrap();
            let hex_len = if oid == Some(OidType::Sha256) { 64 } else { 40 };
            assert_eq!(id.len(), hex_len);
            assert_eq!(
                id == blob,
                oid != Some(OidType::Sha256) && matches!(kind, None | Some(ObjectType::Blob))
            );
        }
    }
    assert!(opts.as_ref().filters().is_none());
    assert!(!format!("{opts:?}").is_empty());
}

/// A list lent to the options is applied by C through them (CRLF to LF before the
/// blob is hashed), read back while they hold it, ignored for other object types and
/// when empty; the options go before the lists, the lists before their repository,
/// and a second list may replace the first while both outlive the options (ASan,
/// LSan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn id_options_apply_the_list_they_borrow() {
    isolated(
        concat!(module_path!(), "::id_options_apply_the_list_they_borrow"),
        || {
            crate::fixture::init();
            let path =
                std::env::temp_dir().join(format!("crustify-id-options-ub-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            let c_path = CString::new(path.to_str().unwrap()).unwrap();
            let repo = Repository::init(&c_path, false).unwrap();
            std::fs::write(path.join(".gitattributes"), "*.txt text\n").unwrap();
            let mut crlf = FilterList::load(repo.as_ref(), c"a.txt", ffi::GIT_FILTER_TO_ODB, 0)
                .unwrap()
                .unwrap();
            let mut empty = FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
            {
                let mut opts = ObjectIdOptions::new();
                assert_eq!(object_id(&mut opts, b"a\r\nb\r\n").unwrap(), CRLF);
                opts.as_mut().set_filters(Some(crlf.as_mut()));
                for _ in 0..3 {
                    assert!(opts.as_ref().filters().is_some());
                    assert_eq!(object_id(&mut opts, b"a\r\nb\r\n").unwrap(), LF);
                }
                opts.as_mut().set_object_type(Some(ObjectType::Commit));
                let commit = object_id(&mut opts, b"a\r\nb\r\n").unwrap();
                assert!(commit != LF && commit != CRLF);
                opts.as_mut().set_object_type(Some(ObjectType::Blob));
                assert_eq!(object_id(&mut opts, b"a\r\nb\r\n").unwrap(), LF);
                assert_eq!(object_id(&mut opts, b"").unwrap().len(), 40);
                opts.as_mut().set_filters(Some(empty.as_mut()));
                assert_eq!(object_id(&mut opts, b"a\r\nb\r\n").unwrap(), CRLF);
                opts.as_mut().set_filters(None);
                assert!(opts.as_ref().filters().is_none());
            }
            // The options are gone: the lists are the caller's again.
            assert_eq!(crate::fixture::filter_count(&crlf), 1);
            assert_eq!(crate::fixture::filter_count(&empty), 0);
            drop(crlf);
            drop(empty);
            drop(repo);
            let _ = std::fs::remove_dir_all(&path);
        },
    );
}

mod kinds_short_ids_and_contents {
    //! UB tests for `ObjectRef::kind` (`git_object_type`), `ObjectRef::short_id`
    //! (`git_object_short_id`), `ObjectType::name` / `from_name`
    //! (`git_object_type2string` / `_string2type`), `type_is_valid` and
    //! `content_is_valid` (`git_object_rawcontent_is_valid`).
    //!
    //! A short id is a fresh `git_buf` of libgit2's allocator, which the returned owner
    //! releases once (ASan: double free; LSan: leak), and which outlives the object, the
    //! repository and the object database libgit2 loaded for it. A content check hashes
    //! and parses exactly the bytes it is lent: each content sits in an exact-length
    //! heap allocation (ASan: a read past it), every prefix of well-formed contents is
    //! checked, and the bytes are rewritten between checks (BSan: libgit2 retains no
    //! pointer into them); the object it parses them into is freed before it returns
    //! (LSan). The first checks of a process race to initialise libgit2 (TSan). A name
    //! is read up to its NUL, from an exact-length allocation too.
    //!
    //! Objects are neither `Send` nor `Sync`, so `kind` and `short_id` stay on one
    //! thread; `ObjectType` is plain data.

    use std::ffi::CString;
    use std::thread;

    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::object::object::{Object, ObjectArc, content_is_valid, type_is_valid};
    use libgit2::libgit2::oid::oid::OidType;

    use super::{BLOB, COMMIT, MANY, TAG, TREE, id};
    use crate::fixture::isolated;
    use crate::fixture::object_and_tree_entry::open_testrepo;

    /// Short ids of many objects, each looked up twice, outlive the objects and the
    /// repository, and are dropped in an order unrelated to their objects'.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn short_ids_outlive_their_objects() {
        let repo = open_testrepo();
        let mut ids = Vec::new();
        let mut objects: Vec<ObjectArc<'_>> = Vec::new();
        for hex in MANY.iter().chain(&[COMMIT, TREE, BLOB, TAG]) {
            for _ in 0..2 {
                let object = Object::lookup(repo.as_ref(), id(hex).as_ref(), None).unwrap();
                let short = object.as_ref().short_id().unwrap();
                assert!(hex.starts_with(short.as_ref().to_str().unwrap()));
                assert!(short.as_ref().len() >= 7);
                ids.push(short);
                objects.push(object);
            }
        }
        let kinds: Vec<_> = objects.iter().map(|o| o.as_ref().kind()).collect();
        assert!(kinds.contains(&ObjectType::Tag) && kinds.contains(&ObjectType::Blob));
        drop(objects);
        drop(repo);
        for (i, short) in ids.into_iter().enumerate().rev() {
            if i % 3 == 0 {
                assert_eq!(short.as_ref().to_str().unwrap().len(), short.as_ref().len());
            }
        }
    }

    /// A tag of each id type, its prefixes, and a long tree.
    fn contents() -> Vec<(Vec<u8>, ObjectType, OidType)> {
        let mut contents = Vec::new();
        for (hex, oid_type) in [
            (COMMIT.to_owned(), OidType::Sha1),
            ("ab".repeat(32), OidType::Sha256),
        ] {
            let tag = format!("object {hex}\ntype commit\ntag v1\ntagger T <t@x> 0 +0000\n\nm\n");
            let commit =
                format!("tree {hex}\nauthor A <a@x> 0 +0000\ncommitter C <c@x> 0 +0000\n\nm\n");
            for (sample, kind) in [(tag, ObjectType::Tag), (commit, ObjectType::Commit)] {
                for len in 0..=sample.len() {
                    contents.push((sample.as_bytes()[..len].to_vec(), kind, oid_type));
                }
            }
        }
        let mut tree = Vec::new();
        for i in 0..2000 {
            tree.extend_from_slice(format!("100644 file-{i:04}\0").as_bytes());
            tree.extend_from_slice(&[0x5a; 20]);
        }
        contents.push((tree, ObjectType::Tree, OidType::Sha1));
        contents
    }

    /// Every content in an exact-length allocation, checked as each type; then
    /// rewritten in place and checked again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn content_checks_read_only_their_bytes() {
        crate::fixture::init();
        let mut valid = 0;
        for (content, kind, oid_type) in contents() {
            let mut exact: Box<[u8]> = content.into_boxed_slice();
            for as_kind in [
                ObjectType::Commit,
                ObjectType::Tree,
                ObjectType::Blob,
                ObjectType::Tag,
            ] {
                let verdict = content_is_valid(&exact, as_kind, oid_type).unwrap();
                valid += usize::from(verdict && as_kind == kind);
            }
            if let Some(first) = exact.first_mut() {
                *first ^= 0x20;
                let _ = content_is_valid(&exact, kind, oid_type).unwrap();
            }
        }
        assert!(valid >= 5, "{valid}");
    }

    /// Small contents, checked twice with the bytes rewritten between: libgit2 keeps no
    /// pointer into them (BSan), and gives the same verdicts.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn content_checks_retain_no_pointer() {
        crate::fixture::init();
        let mut buf = format!("object {COMMIT}\ntype commit\ntag v1\n").into_bytes();
        assert_eq!(
            content_is_valid(&buf, ObjectType::Tag, OidType::Sha1),
            Ok(true)
        );
        buf[0] = b'O';
        assert_eq!(
            content_is_valid(&buf, ObjectType::Tag, OidType::Sha1),
            Ok(false)
        );
        buf[0] = b'o';
        assert_eq!(
            content_is_valid(&buf, ObjectType::Tag, OidType::Sha1),
            Ok(true)
        );
        assert_eq!(
            content_is_valid(&[], ObjectType::Tree, OidType::Sha1),
            Ok(true)
        );
    }

    /// Many threads make the first calls at once, in a process where nothing has
    /// initialised libgit2, and keep checking contents and names (TSan).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn first_checks_race_initialisation() {
        isolated(
            concat!(module_path!(), "::first_checks_race_initialisation"),
            || {
                thread::scope(|s| {
                    for i in 0..8 {
                        s.spawn(move || {
                            let tag = format!("object {COMMIT}\ntype commit\ntag v{i}\n");
                            for _ in 0..50 {
                                let kind = ObjectType::Tag;
                                assert_eq!(
                                    content_is_valid(tag.as_bytes(), kind, OidType::Sha1),
                                    Ok(true)
                                );
                                assert_eq!(content_is_valid(b"x", kind, OidType::Sha1), Ok(false));
                                assert_eq!(ObjectType::from_name(c"tag"), Some(kind));
                                assert_eq!(kind.name(), "tag");
                                assert!(type_is_valid(kind.raw()) && !type_is_valid(0));
                            }
                        });
                    }
                });
            },
        );
    }

    /// Names of every length up to a few KiB, in exact-length allocations: libgit2
    /// stops at the NUL.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn names_are_read_up_to_their_nul() {
        for len in (0..64).chain([1000, 4096]) {
            for seed in ["commit", "tree", "blob", "tag", "x"] {
                let name: String = seed.chars().cycle().take(len).collect();
                let name = CString::new(name).unwrap();
                let found = ObjectType::from_name(&name);
                assert_eq!(
                    found.is_some(),
                    len >= seed.len() && seed != "x",
                    "{name:?}"
                );
            }
        }
    }
}

/// An object's id lives in the object, as long as any owner of it does; its owner is
/// the repository handle it was looked up through, which outlives the object and looks
/// further objects up into the same cache. An id written over after its lookup does not
/// change it (BSan: libgit2 copied it).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn ids_and_owners_borrow_from_their_object() {
    let repo = open_testrepo();
    for (hex_id, kind) in [
        (COMMIT, ObjectType::Commit),
        (TREE, ObjectType::Tree),
        (BLOB, ObjectType::Blob),
        (TAG, ObjectType::Tag),
    ] {
        let mut probe = id(hex_id);
        let object = Object::lookup(repo.as_ref(), probe.as_ref(), Some(kind)).unwrap();
        probe.as_mut().set_id([0x5a; 32]);
        let clone = object.clone();
        let handle = clone.as_ref();
        let object_id = handle.id();
        drop(object);
        assert_eq!(object_id.to_hex_string().unwrap(), hex_id);
        assert_eq!(object_id.kind(), Some(OidType::Sha1));
        let owner = handle.owner();
        drop(clone);
        assert!(is_owner(owner, &repo));
        let again = Object::lookup(owner, id(hex_id).as_ref(), None).unwrap();
        assert_eq!(again.as_ref().id(), id(hex_id).as_ref());
    }
    // A prefix lookup's object holds the full id; derived objects their own.
    let short = Object::lookup_prefix(repo.as_ref(), id(COMMIT).as_ref(), 7, None).unwrap();
    assert_eq!(short.as_ref().id().to_hex_string().unwrap(), COMMIT);
    let tree = short.as_ref().peel(Some(ObjectType::Tree)).unwrap();
    assert_eq!(tree.as_ref().id().to_hex_string().unwrap(), TREE);
    let readme = tree.as_ref().lookup_bypath(c"README", None).unwrap();
    drop((short, tree));
    assert_eq!(readme.as_ref().id().to_hex_string().unwrap(), BLOB);
    assert!(is_owner(readme.as_ref().owner(), &repo));
}

/// Objects of a repository without paths are never built, and those of a temporary
/// repository report it as their owner after it wrote them.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owners_of_written_objects() {
    let temp = TempRepo::new("owners");
    let blob = write_raw(&temp, b"owned", ffi::GIT_OBJECT_BLOB);
    let repo = repository(&temp);
    let object = Object::lookup(repo, blob.as_ref(), None).unwrap();
    assert_eq!(object.as_ref().id(), blob.as_ref());
    let owner = object.as_ref().owner();
    assert_eq!(owner.path(), repo.path());
    assert_eq!(
        owner.path().map(CStr::as_ptr),
        repo.path().map(CStr::as_ptr)
    );
    let pathless = Repository::new().unwrap();
    assert!(Object::lookup(pathless.as_ref(), blob.as_ref(), None).is_err());
}
