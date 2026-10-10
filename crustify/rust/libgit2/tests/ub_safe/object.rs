//! UB tests for `git2/object.h`: the `git_object` owner, `ObjectArc<'r>`, its
//! constructors `Object::lookup` and `Object::lookup_prefix`, and its handles.
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
//! overwrite and free each id right after its lookup (BSan).
//!
//! An object is neither `Send` nor `Sync`, so there is no thread test (its owner and
//! shared handle staying on their thread is a doctest each); it, its handles and its
//! clones cannot outlive the repository, nor can a handle outlive its owner, and no
//! owner lends an exclusive handle (`compile_fail/` and doctests).
//!
//! Every test calls libgit2, so Miri skips them all.

use std::ffi::CString;

use libgit2::ffi;
use libgit2::libgit2::core::types::ObjectType;
use libgit2::libgit2::object::object::{Object, ObjectArc};
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::{Repository, RepositoryError};
use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

use crate::fixture::object_and_tree_entry::{
    enable_caching, object_owner, object_view, open_testrepo, repository, set_cache_max_size,
    write_raw,
};
use crate::fixture::{TempRepo, isolated};

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
        assert_eq!(object_owner(&any), repo.as_c_ptr());
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
    assert_eq!(object_owner(&from_a), a.as_c_ptr());
    assert_eq!(object_owner(&from_b), b.as_c_ptr());
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
