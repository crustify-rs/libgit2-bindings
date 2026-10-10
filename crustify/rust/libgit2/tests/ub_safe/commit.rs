//! UB tests for `git2/commit.h`: the `Commit` owner, `CommitArc<'r>`, its lookups,
//! handles and object conversions; the `CommitArray` value held by its `CommitArrayVal`
//! owner, its handles, the commits it lends and moves in and out, and the
//! `git_commitarray_dispose` policy; the `Header` value; and the `CreateExtOptions`
//! value and its handles.
//!
//! Each filled array's buffer is exactly `len` commit pointers long (libgit2 detaches
//! a `git_vector` sized to them, `set_commits` allocates that many), so ASan flags a
//! read past them; every commit reference and buffer must be released exactly once,
//! whether the array is cleared, emptied by `take`, refilled or dropped (ASan: double
//! free, use after free; LSan: leak). Only the empty array is built without libgit2,
//! so little runs under Miri. Illegal uses are compile-fail cases and doctests on the
//! crate: a commit or an array outliving its repository, a lent commit outliving its
//! array, a `clone`, sending an array or a commit to another thread.

use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::config::config::Config;
use libgit2::libgit2::object::commit::{CommitArray, CommitArrayRef, CommitArrayVal};

use crate::fixture::commit_and_config::{
    EMPTY, MERGEDREPO, OpenRepo, TESTREPO, commit_id, commit_parents,
};
use crate::fixture::isolated;

fn ids(array: CommitArrayRef<'_, '_>) -> Vec<String> {
    array.iter().map(commit_id).collect()
}

const HEAD: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
const MERGED: [&str; 2] = [
    "3a34580a35add43a4cf361e8e9a30060a905c876",
    "e2809157a7766f272e4cfe26e61ef2678a5357ff",
];

/// The empty array, built, read, cleared, emptied and dropped without libgit2.
#[test]
fn empty_arrays_never_reach_libgit2() {
    let mut array: CommitArrayVal<'static> = CommitArray::new();
    assert!(array.as_ref().is_empty());
    assert_eq!(array.as_ref().len(), 0);
    assert!(array.as_ref().get(0).is_none() && array.as_ref().get(usize::MAX).is_none());
    assert_eq!(array.as_ref().iter().next_back().map(commit_id), None);
    array.as_mut().clear();
    array.as_mut().clear();
    assert!(array.as_mut().take().is_empty());
    assert!(array.as_mut().set_commits(Vec::new()).is_ok());
    assert_eq!(format!("{:?}", array.as_ref()), "[]");
    assert_eq!(format!("{:?}", array.as_mut()), "[]");
    let bare = array.into_inner();
    assert!(bare.as_ref().is_empty());
    assert!(
        CommitArray::from_commits(Vec::new())
            .unwrap()
            .as_ref()
            .is_empty()
    );
}

/// Filled arrays lend every commit in bounds, and release each reference once, through
/// `clear` and through the drop, reads after a clear included.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn filled_arrays_read_clear_and_release() {
    let repo = OpenRepo::open(MERGEDREPO);
    let mut array = commit_parents(&repo);
    let r = array.as_ref();
    assert_eq!(r.len(), 2);
    assert_eq!(ids(r), MERGED);
    assert!(r.get(2).is_none());
    assert_eq!(commit_id(r.get(1).unwrap()), MERGED[1]);
    assert_eq!(
        r.iter().rev().map(commit_id).collect::<Vec<_>>(),
        [MERGED[1], MERGED[0]]
    );
    assert_eq!(format!("{r:?}"), "[Commit { .. }, Commit { .. }]");

    array.as_mut().clear();
    assert!(array.as_ref().is_empty() && array.as_ref().get(0).is_none());
    array.as_mut().clear();

    // The same commits held by several arrays at once are counted references of the
    // repository's cache: releasing them in any order frees each once.
    let arrays: Vec<_> = (0..4).map(|_| commit_parents(&repo)).collect();
    for (i, a) in arrays.into_iter().enumerate().rev() {
        assert_eq!(ids(a.as_ref()), MERGED, "array {i}");
    }
    let empty = OpenRepo::open(EMPTY);
    let unborn = commit_parents(&empty);
    assert!(unborn.as_ref().is_empty());
}

/// Commits lent by an array and copied out of it outlive the array; commits taken out
/// of one array and moved into another keep their single references (ASan: a double
/// release; LSan: a leak).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn commits_move_between_arrays_and_outlive_them() {
    let repo = OpenRepo::open(MERGEDREPO);
    let mut first = commit_parents(&repo);
    let copied: Vec<_> = first.as_ref().iter().map(|c| c.to_commit()).collect();
    let taken = first.as_mut().take();
    assert!(first.as_ref().is_empty());
    drop(first);
    assert_eq!(taken.len(), 2);

    let mut second = CommitArray::from_commits(taken).unwrap();
    assert_eq!(ids(second.as_ref()), MERGED);
    // Replacing releases the old commits; the copies keep theirs.
    second.as_mut().set_commits(copied.clone()).unwrap();
    second
        .as_mut()
        .set_commits(copied.iter().rev().cloned().collect())
        .unwrap();
    assert_eq!(ids(second.as_ref()), [MERGED[1], MERGED[0]]);
    let lent = second.as_ref().get(0).unwrap().to_commit();
    drop(second);
    assert_eq!(commit_id(lent.as_ref()), MERGED[1]);

    // Many arrays of the same commits, refilled and emptied in turn.
    let mut arrays: Vec<_> = (0..8)
        .map(|_| CommitArray::from_commits(copied.clone()).unwrap())
        .collect();
    for (i, array) in arrays.iter_mut().enumerate() {
        if i % 2 == 0 {
            drop(array.as_mut().take());
        } else {
            array.as_mut().set_commits(vec![lent.clone()]).unwrap();
        }
    }
    drop(copied);
    for (i, array) in arrays.iter().enumerate() {
        assert_eq!(array.as_ref().len(), i % 2, "array {i}");
    }
}

/// Arrays released before their repository, after their commits were read: the
/// commits are released through the repository's cache, which is still live.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_go_before_their_repository() {
    let repo = OpenRepo::open(TESTREPO);
    let kept = commit_parents(&repo);
    let mut cleared = commit_parents(&repo);
    assert_eq!(ids(kept.as_ref()), [HEAD]);
    cleared.as_mut().clear();
    drop(cleared);
    drop(kept);
    drop(repo);
}

/// Dropping and clearing empty arrays on one thread while another performs the
/// process's first libgit2 initialisation races nothing: the empty array never
/// reaches libgit2's allocator (TSan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_drops_do_not_race_the_first_init() {
    isolated(
        concat!(module_path!(), "::empty_drops_do_not_race_the_first_init"),
        || {
            let barrier = Barrier::new(2);
            thread::scope(|s| {
                s.spawn(|| {
                    barrier.wait();
                    for _ in 0..200 {
                        let mut array: CommitArrayVal<'_> = CommitArray::new();
                        array.as_mut().clear();
                        drop(array.as_mut().take());
                        drop(array);
                    }
                });
                s.spawn(|| {
                    barrier.wait();
                    drop(Config::new().unwrap());
                });
            });
        },
    );
}

/// Arrays filled from repositories on several threads at once, each array staying on
/// its own thread, release their commits without racing (TSan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn arrays_on_their_own_threads() {
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                let repo = OpenRepo::open(MERGEDREPO);
                for _ in 0..8 {
                    let mut array = commit_parents(&repo);
                    assert_eq!(ids(array.as_ref()), MERGED);
                    let taken = array.as_mut().take();
                    array.as_mut().set_commits(taken).unwrap();
                    array.as_mut().clear();
                }
            });
        }
    });
}

/// The extra headers: the `Header` value (`git_commit_header`) and its handles.
///
/// A header is two borrowed string pointers. Its handles project each field out of a
/// pointer and rebuild a `&CStr` from it, so these tests exercise every getter and
/// setter on every holder Rust has for one (a local, a heap slot, an element of a run
/// that moves when it grows, a copy, a `static`), through reborrows, with strings that
/// live on the heap and are dropped right after their last read (ASan: a getter
/// reading past the string or after it), and across threads under the `Send` / `Sync`
/// the crate asserts (TSan). A misaligned projection or a write through the shared
/// handle would be flagged by Miri and ASan/UBSan. Headers that outlive their strings,
/// handles that outlive their header, aliased handles and setters on the shared handle
/// do not compile (`compile_fail/` and the type's doctests). Nothing here reaches
/// libgit2, which keeps no pointer to a header past a call, so every test runs under
/// Miri and there is no foreign write or retained pointer to race (BSan).
mod header {
    use std::ffi::{CStr, CString};
    use std::sync::Mutex;
    use std::thread;

    use libgit2::libgit2::object::commit::{Header, HeaderMut, HeaderRef};

    static STATIC: Header<'static> = Header::new(c"static", c"header");

    fn read<'h>(r: HeaderRef<'_, 'h>) -> (&'h CStr, &'h CStr) {
        (r.field(), r.value())
    }

    fn write<'h>(mut m: HeaderMut<'_, 'h>, field: &'h CStr, value: &'h CStr) {
        m.set_field(field);
        m.set_value(value);
    }

    #[test]
    fn every_field_round_trips_on_every_holder() {
        let strings: Vec<CString> = ["a", "", "multi\nline value", "x".repeat(4096).as_str()]
            .into_iter()
            .map(|s| CString::new(s).unwrap())
            .collect();
        let mut local = Header::new(c"f", c"v");
        let mut boxed = Box::new(Header::new(c"f", c"v"));
        let mut run = vec![Header::new(c"f", c"v")];
        for (i, pair) in strings.windows(2).enumerate() {
            let (f, v) = (pair[0].as_c_str(), pair[1].as_c_str());
            write(local.as_mut(), f, v);
            write(Header::as_mut(&mut boxed), f, v);
            // Growing the run moves its elements; each handle is taken afresh.
            run.push(Header::new(f, v));
            write(run[i].as_mut(), f, v);
            for r in [
                local.as_ref(),
                Header::as_ref(&boxed),
                run[i].as_ref(),
                run[i + 1].as_ref(),
            ] {
                assert_eq!(read(r), (f, v));
            }
            let copy = local;
            assert_eq!(read(copy.as_ref()), (f, v));
        }
        assert_eq!(read(STATIC.as_ref()), (c"static", c"header"));
    }

    /// Strings on the heap, freed right after the header's last use: every read lands
    /// inside a live allocation (ASan).
    #[test]
    fn reads_stay_inside_heap_strings() {
        for len in [0, 1, 7, 8, 15, 16, 255] {
            let field = CString::new("f".repeat(len)).unwrap();
            let value = CString::new("v".repeat(len + 1)).unwrap();
            let header = Header::new(&field, &value);
            assert_eq!(header.as_ref().field().to_bytes().len(), len);
            assert_eq!(header.as_ref().value().to_bytes().len(), len + 1);
            assert!(format!("{header:?}").len() > 2 * len);
            // The header is `Copy`: its last use is above; the strings go now.
            drop(field);
            drop(value);
        }
    }

    #[test]
    fn reborrows_keep_one_writer() {
        let mut header = Header::new(c"a", c"b");
        let mut m = header.as_mut();
        {
            let mut inner = m.as_mut();
            inner.set_field(c"inner");
            assert_eq!(inner.as_ref().field(), c"inner");
        }
        let shared = m.as_ref();
        assert_eq!(read(shared), (c"inner", c"b"));
        m.set_value(c"after");
        assert_eq!(read(header.as_ref()), (c"inner", c"after"));
    }

    #[test]
    fn copies_are_independent() {
        let a = Header::new(c"a", c"1");
        let mut b = a;
        let mut c = a.as_ref().to_header();
        b.as_mut().set_value(c"2");
        c.as_mut().set_field(c"c");
        assert_eq!(read(a.as_ref()), (c"a", c"1"));
        assert_eq!(read(b.as_ref()), (c"a", c"2"));
        assert_eq!(read(c.as_ref()), (c"c", c"1"));
    }

    /// A run of headers read from several threads at once, each element written by
    /// one thread through its own exclusive handle, and headers moved between threads
    /// under the asserted `Send` / `Sync` (TSan).
    #[test]
    fn headers_cross_threads() {
        let owned: Vec<CString> = (0..8)
            .map(|i| CString::new(format!("value {i}")).unwrap())
            .collect();
        let mut run: Vec<Header<'_>> = owned.iter().map(|v| Header::new(c"field", v)).collect();
        thread::scope(|s| {
            for (i, h) in run.iter_mut().enumerate() {
                let owned = &owned;
                s.spawn(move || {
                    let mut m = h.as_mut();
                    m.set_field(c"written");
                    assert_eq!(m.as_ref().value(), owned[i].as_c_str());
                });
            }
        });
        let run = &run;
        let seen = Mutex::new(0);
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for h in run {
                        let r: HeaderRef<'_, '_> = h.as_ref();
                        assert_eq!(r.field(), c"written");
                        assert!(r.value().to_bytes().starts_with(b"value "));
                    }
                    *seen.lock().unwrap() += 1;
                });
            }
            let one = run[3];
            s.spawn(move || assert_eq!(one.as_ref().value(), c"value 3"));
            let handle = run[5].as_ref();
            s.spawn(move || assert_eq!(handle.value(), c"value 5"));
        });
        assert_eq!(*seen.lock().unwrap(), 4);
    }
}

/// The commit owner: `CommitArc<'r>`, its constructors `Commit::lookup` and
/// `Commit::lookup_prefix`, its handles, and its conversions to and from objects.
///
/// A commit is counted and shared with its repository's cache, which hands the same
/// commit to every lookup of one id: every owner gives back its one reference through
/// `git_commit_free` exactly once, and every clone, `to_commit` and conversion takes or
/// moves exactly one. A miscount frees a commit the cache or another owner still
/// reads, which ASan flags, or leaks it, which LSan does. The tests share commits
/// between lookups, clones, conversions and arrays, drop them in every order, let the
/// cache evict commits owners still hold (a tiny cache limit) and run with the cache
/// off (a fresh commit per lookup). Every failing lookup must leave nothing behind, an
/// object of another type must never pass for a commit, and an id of another type
/// than the repository's must be refused before it reaches C. libgit2 copies the id it
/// is given: the tests overwrite and drop each id right after its lookup (BSan).
///
/// A commit is neither `Send` nor `Sync`, so there is no thread test (doctests); it,
/// its handles and its clones cannot outlive the repository, nor a handle its owner,
/// and no owner lends an exclusive handle (`compile_fail/`).
mod commit {
    use std::ffi::CString;

    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::object::commit::{Commit, CommitArc, CommitArray, CommitRef};
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryError};
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use crate::fixture::commit_and_config::commit_id;
    use crate::fixture::isolated;
    use crate::fixture::object_and_tree_entry::{
        enable_caching, object_view, open_testrepo, set_cache_max_size,
    };

    /// Commits of `testrepo.git`.
    const COMMITS: [&str; 8] = [
        "a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
        "be3563ae3f795b2b4353bcce3a527ad0a4f7f644",
        "c47800c7266a2be04c571c04d5a6614691ea99bd",
        "9fd738e8f7967c078dceed8190330fc8648ee56a",
        "4a202b346bb0fb0db7eff3cffeb3c70babbd2045",
        "5b5b025afb0b4c913b4c338a42934a3863bf3644",
        "8496071c1b46c854b31185ea97743be6a8774479",
        "763d71aadf09a7951596c9746c024e7eece7c7af",
    ];
    const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
    const BLOB: &str = "a8233120f6ad708f843d861ce2b7228ec4e3dec6";
    const TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";

    fn id(hex: &str) -> Oid {
        Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap()
    }

    fn hex(commit: &CommitArc<'_>) -> String {
        commit_id(commit.as_ref())
    }

    fn assert_failed(result: Result<CommitArc<'_>, RepositoryError>, code: i32, class: ErrorClass) {
        let e = result.unwrap_err();
        assert_eq!((e.raw_code(), e.class()), (code, Some(class)), "{e:?}");
    }

    /// Lookups by id and by prefix, clones and `to_commit` copies share one commit
    /// with the cache, and each owner releases its own reference, in every order. The
    /// id is overwritten and dropped right after each lookup.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn owners_share_one_commit_and_release_in_every_order() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        let mut held: Vec<CommitArc<'_>> = Vec::new();
        for (i, hex) in COMMITS.iter().enumerate() {
            // On the heap, so a read after the free is flagged.
            let mut oid = Box::new(id(hex));
            let full = Commit::lookup(r, Oid::as_ref(&oid)).unwrap();
            let short = Commit::lookup_prefix(r, Oid::as_ref(&oid), 7 + i).unwrap();
            Oid::as_mut(&mut oid).set_id([0xff; 32]);
            drop(oid);
            assert!(CommitArc::ptr_eq(&full, &short));
            let copy = full.as_ref().to_commit();
            let clone = short.clone();
            assert!(CommitArc::ptr_eq(&copy, &clone));
            match i % 3 {
                0 => drop((full, short)),
                1 => {
                    drop(short);
                    held.push(full);
                }
                _ => held.push(short),
            }
            held.push(copy);
            drop(clone);
        }
        for owner in held.iter().rev() {
            assert!(COMMITS.contains(&hex(owner).as_str()));
        }
        // Drop from both ends towards the middle.
        while let Some(last) = held.pop() {
            let id = hex(&last);
            if !held.is_empty() {
                let first = held.remove(0);
                drop(last);
                assert!(COMMITS.contains(&hex(&first).as_str()));
            }
            assert!(COMMITS.contains(&id.as_str()));
        }
    }

    /// With a tiny cache, commits are evicted while owners hold them; with the cache
    /// off, every lookup parses a fresh commit. Each owner keeps its commit alive
    /// either way.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn owners_survive_eviction_and_a_disabled_cache() {
        isolated(
            concat!(
                module_path!(),
                "::owners_survive_eviction_and_a_disabled_cache"
            ),
            || {
                set_cache_max_size(1);
                let repo = open_testrepo();
                let first: Vec<_> = COMMITS
                    .iter()
                    .map(|h| Commit::lookup(repo.as_ref(), id(h).as_ref()).unwrap())
                    .collect();
                let again: Vec<_> = COMMITS
                    .iter()
                    .map(|h| Commit::lookup(repo.as_ref(), id(h).as_ref()).unwrap())
                    .collect();
                for (a, b) in first.iter().zip(&again) {
                    assert_eq!(hex(a), hex(b));
                }
                drop(first);
                enable_caching(false);
                let fresh = Commit::lookup(repo.as_ref(), id(COMMITS[0]).as_ref()).unwrap();
                let other = Commit::lookup(repo.as_ref(), id(COMMITS[0]).as_ref()).unwrap();
                assert!(!CommitArc::ptr_eq(&fresh, &other));
                let array = CommitArray::from_commits(vec![fresh, other.clone()]).unwrap();
                drop(other);
                assert_eq!(
                    array.as_ref().iter().map(commit_id).collect::<Vec<_>>(),
                    [COMMITS[0]; 2]
                );
                drop(again);
            },
        );
    }

    /// Failing lookups leave nothing behind: objects of other types, missing and
    /// ambiguous ids, ids of another type, and a repository without an object database.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failing_lookups_leave_nothing() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        for other in [TREE, BLOB, TAG] {
            assert_failed(
                Commit::lookup(r, id(other).as_ref()),
                ErrorCode::NotFound.raw(),
                ErrorClass::Invalid,
            );
            assert_failed(
                Commit::lookup_prefix(r, id(other).as_ref(), 10),
                ErrorCode::NotFound.raw(),
                ErrorClass::Invalid,
            );
        }
        assert_failed(
            Commit::lookup(r, Oid::zero(OidType::Sha1).as_ref()),
            ErrorCode::NotFound.raw(),
            ErrorClass::Odb,
        );
        for len in [0, 1, 3] {
            assert_failed(
                Commit::lookup_prefix(r, id(COMMITS[0]).as_ref(), len),
                ErrorCode::Ambiguous.raw(),
                ErrorClass::Object,
            );
        }
        let wide = Oid::from_raw(&[0xa6; 32], OidType::Sha256).unwrap();
        assert_failed(Commit::lookup(r, wide.as_ref()), -1, ErrorClass::Invalid);
        assert_failed(
            Commit::lookup_prefix(r, wide.as_ref(), 64),
            -1,
            ErrorClass::Invalid,
        );
        let pathless = Repository::new().unwrap();
        assert!(Commit::lookup(pathless.as_ref(), id(COMMITS[0]).as_ref()).is_err());
        // The repository still answers.
        assert_eq!(
            hex(&Commit::lookup(r, id(COMMITS[0]).as_ref()).unwrap()),
            COMMITS[0]
        );
    }

    /// A commit seen and converted as an object, and back: the same commit, one
    /// reference moved or counted per owner; objects of other types are refused and
    /// handed back usable.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn object_conversions_move_one_reference() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        let object = Object::lookup(r, id(COMMITS[1]).as_ref(), None).unwrap();
        let viewed = CommitRef::from_object(object.as_ref()).unwrap();
        assert_eq!(commit_id(viewed), COMMITS[1]);
        let from_view = viewed.to_commit();
        let commit = Commit::try_from_object(object).unwrap();
        assert!(CommitArc::ptr_eq(&commit, &from_view));
        let as_object: ObjectArc<'_> = commit.as_ref().as_object().to_object();
        assert_eq!(
            object_view(&as_object),
            (COMMITS[1].into(), libgit2::ffi::GIT_OBJECT_COMMIT)
        );
        let back = Commit::into_object(commit);
        drop(from_view);
        assert_eq!(object_view(&back), object_view(&as_object));
        drop(as_object);
        let again = Commit::try_from_object(back).unwrap();
        assert_eq!(hex(&again), COMMITS[1]);

        for (other, kind) in [
            (TREE, ObjectType::Tree),
            (BLOB, ObjectType::Blob),
            (TAG, ObjectType::Tag),
        ] {
            let object = Object::lookup(r, id(other).as_ref(), Some(kind)).unwrap();
            assert!(CommitRef::from_object(object.as_ref()).is_none());
            let object = Commit::try_from_object(object).unwrap_err();
            assert_eq!(object_view(&object).0, other);
        }
        // A tag peeled to its commit converts.
        let tag = Object::lookup(r, id(TAG).as_ref(), None).unwrap();
        let peeled = tag.as_ref().peel(Some(ObjectType::Commit)).unwrap();
        drop(tag);
        let commit = Commit::try_from_object(peeled).unwrap();
        assert_eq!(hex(&commit), "e90810b8df3e80c413d903f631643c716887138d");
    }
}

/// The creation options: the `CreateExtOptions` value (`git_commit_create_ext_options`)
/// and its handles.
///
/// The options are a few borrowed pointers and a length. Their handles project each
/// field out of a pointer and rebuild a `&CStr` or a run of headers from it, so these
/// tests exercise every getter and setter on every holder Rust has for one (a local, a
/// heap slot, an element of a run that moves when it grows), through reborrows, with
/// strings and headers on the heap dropped right after their last read (ASan: a getter
/// reading past them or after them). libgit2 reads the options for one call and keeps
/// nothing: one test hands them to `git_commit_create_ext`, its strings and headers on
/// the heap, freed right after the call (ASan, BSan: a retained pointer). Options that
/// outlive their borrows, aliased handles and the raw seam without `unsafe` do not
/// compile (`compile_fail/` and the type's doctests). The options are neither `Send`
/// nor `Sync`, so there is no thread test. `CreateExtOptions::new` fills the options
/// through libgit2, so Miri skips every test here; the module's `unit_tests` run the
/// same projections under Miri over options built in Rust.
mod create_ext_options {
    use std::ffi::{CStr, CString};

    use libgit2::libgit2::object::commit::{
        CreateExtOptions, CreateExtOptionsMut, CreateExtOptionsRef, Header,
    };

    use crate::fixture::TempRepo;
    use crate::fixture::create_ext::{create, has_reference};

    type Fields<'o> = (
        Option<&'o CStr>,
        Option<&'o CStr>,
        Vec<(&'o CStr, &'o CStr)>,
    );

    fn read<'o>(r: CreateExtOptionsRef<'_, 'o>) -> Fields<'o> {
        let headers = r
            .extra_headers()
            .iter()
            .map(|h| (h.field(), h.value()))
            .collect();
        (r.update_ref(), r.message_encoding(), headers)
    }

    fn write<'o>(
        mut m: CreateExtOptionsMut<'_, 'o>,
        name: Option<&'o CStr>,
        encoding: Option<&'o CStr>,
        headers: &'o [Header<'o>],
    ) {
        m.set_update_ref(name);
        m.set_message_encoding(encoding);
        m.set_extra_headers(headers);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn every_field_round_trips_on_every_holder() {
        let strings: Vec<CString> = ["refs/heads/a", "", "UTF-16", &"x".repeat(1024)]
            .into_iter()
            .map(|s| CString::new(s).unwrap())
            .collect();
        let headers: Vec<Header<'_>> = strings.iter().map(|s| Header::new(s, s)).collect();
        let mut local = CreateExtOptions::new();
        let mut boxed = Box::new(CreateExtOptions::new());
        let mut run = vec![CreateExtOptions::new()];
        for i in 0..strings.len() {
            let (name, encoding) = (
                Some(strings[i].as_c_str()),
                strings.get(i + 1).map(|s| s.as_c_str()),
            );
            let hs = &headers[i..];
            write(local.as_mut(), name, encoding, hs);
            write(CreateExtOptions::as_mut(&mut boxed), name, encoding, hs);
            // Growing the run moves its elements; each handle is taken afresh.
            run.push(CreateExtOptions::new());
            write(run[i].as_mut(), name, encoding, hs);
            let expected = (
                name,
                encoding,
                hs.iter()
                    .map(|h| (h.as_ref().field(), h.as_ref().value()))
                    .collect::<Vec<_>>(),
            );
            for r in [
                local.as_ref(),
                CreateExtOptions::as_ref(&boxed),
                run[i].as_ref(),
            ] {
                assert_eq!(read(r), expected);
                assert_eq!(r.version(), 1);
                assert!(!r.has_signer());
            }
            assert_eq!(read(run[i + 1].as_ref()), (None, None, Vec::new()));
        }
        write(local.as_mut(), None, None, &[]);
        assert_eq!(read(local.as_ref()), (None, None, Vec::new()));
    }

    /// Strings and headers on the heap, freed right after the options' last use.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reads_stay_inside_heap_borrows() {
        for len in [0, 1, 7, 8, 15, 16, 255] {
            let name = CString::new("n".repeat(len)).unwrap();
            let headers: Vec<Header<'_>> =
                (0..len % 5).map(|_| Header::new(&name, &name)).collect();
            let mut options = CreateExtOptions::new();
            write(options.as_mut(), Some(&name), Some(&name), &headers);
            let (n, e, hs) = read(options.as_ref());
            assert_eq!(
                (n.unwrap().to_bytes().len(), e.unwrap().to_bytes().len()),
                (len, len)
            );
            assert_eq!(hs.len(), len % 5);
            assert!(format!("{options:?}").len() > len);
            drop(headers);
            drop(name);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reborrows_keep_one_writer() {
        let headers = [Header::new(c"f", c"v")];
        let mut options = CreateExtOptions::default();
        let mut m = options.as_mut();
        {
            let mut inner = m.as_mut();
            inner.set_extra_headers(&headers);
            assert_eq!(inner.as_ref().extra_headers().len(), 1);
        }
        let shared = m.as_ref();
        assert_eq!(read(shared).2, [(c"f", c"v")]);
        m.set_update_ref(Some(c"HEAD"));
        assert_eq!(options.as_ref().update_ref(), Some(c"HEAD"));
        assert_eq!(format!("{:?}", options.as_mut()), format!("{options:?}"));
    }

    /// libgit2 reads the options for the call only: the commit it writes carries the
    /// encoding and the headers, the reference it names is created, and nothing it was
    /// given is read after the call (the heap borrows are freed right away).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn libgit2_reads_the_options_for_the_call_only() {
        let repo = TempRepo::new("commit-create-ext-options");
        let name = CString::new("refs/heads/created").unwrap();
        let encoding = CString::new("ISO-8859-1").unwrap();
        let value = CString::new("one\ntwo").unwrap();
        let headers = vec![Header::new(c"x-a", &value), Header::new(c"x-b", c"")];
        let mut options = CreateExtOptions::new();
        write(options.as_mut(), Some(&name), Some(&encoding), &headers);
        let text = create(&repo, &options).unwrap();
        drop(headers);
        drop((value, encoding));
        assert!(
            text.contains("\nencoding ISO-8859-1\nx-a one\n two\nx-b \n\nmsg\n"),
            "{text}"
        );
        assert!(has_reference(&repo, &name));

        // The default options write a plain commit and move no reference.
        let text = create(&repo, &CreateExtOptions::new()).unwrap();
        assert!(text.ends_with("0000\n\nmsg\n"), "{text}");
    }

}

/// UB tests for writing and reading commit signatures: `create_with_signature` and
/// `extract_signature`.
///
/// libgit2 reads the contents, signature and field it is given for the call only, and
/// fills two fresh buffers it owns: every input is an exact-length heap string,
/// overwritten and freed right after the call (ASan flags a read past one, BSan a
/// pointer kept), and every buffer is released once (LSan: leak; ASan: double free). The
/// commits parsed and rejected along the way take libgit2's error paths: contents that
/// do not parse, a signed one without a blank line, a missing tree, a repository
/// without paths. An id of another type than the repository's, which libgit2 would
/// format out of bounds, and an empty field, which it would scan for forever, are
/// refused before the call. Each thread writes into its own repository at once (TSan).
mod signatures {
    use std::ffi::{CStr, CString};
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::object::commit::{create_with_signature, extract_signature};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::Repository;
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use crate::fixture::TempRepo;
    use crate::fixture::object_and_tree_entry::{repository, write_raw};

    const SIGNATURE: &str =
        "-----BEGIN SIGNATURE-----\nline one\n\nline three\n-----END SIGNATURE-----";

    /// The contents of a commit of `tree` with `parents`.
    fn content(tree: &Oid, parents: &[&Oid], message: &str) -> String {
        let mut s = format!("tree {}\n", tree.as_ref().to_hex_string().unwrap());
        for p in parents {
            s += &format!("parent {}\n", p.as_ref().to_hex_string().unwrap());
        }
        s + "author A U Thor <a@example.com> 1700000000 +0000\n\
             committer A U Thor <a@example.com> 1700000000 +0000\n\n"
            + message
    }

    /// `s` in a heap buffer of exactly its length, NUL included.
    fn exact(s: &str) -> Box<CStr> {
        CString::new(s).unwrap().into_boxed_c_str()
    }

    /// A bare repository holding the empty tree.
    fn repo_with_tree(tag: &str) -> (TempRepo, Oid) {
        let repo = TempRepo::new(&format!("signatures-{tag}"));
        let tree = write_raw(&repo, b"", ffi::GIT_OBJECT_TREE);
        (repo, tree)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signatures_round_trip() {
        let (temp, tree) = repo_with_tree("round-trip");
        let repo = repository(&temp);
        let root_content = content(&tree, &[], "root\n");
        let root = create_with_signature(repo, &exact(&root_content), None, None).unwrap();
        let unsigned = content(&tree, &[&root], "signed\n");
        for field in [None, Some("x-signature"), Some("gpgsig-sha256")] {
            let (mut text, mut sig) = (exact(&unsigned), exact(SIGNATURE));
            let f = field.map(exact);
            let id = create_with_signature(repo, &text, Some(&sig), f.as_deref()).unwrap();
            // libgit2 kept none of them.
            drop((
                std::mem::replace(&mut text, exact("")),
                std::mem::replace(&mut sig, exact("")),
            ));
            drop(f);
            let f = field.map(exact);
            let (signature, signed) = extract_signature(repo, id.as_ref(), f.as_deref()).unwrap();
            drop(f);
            assert_eq!(signature.as_ref().as_bytes(), SIGNATURE.as_bytes());
            assert_eq!(signed.as_ref().as_bytes(), unsigned.as_bytes());
            drop(signature);
            assert_eq!(signed.as_ref().len(), unsigned.len());
        }
        // Unsigned contents are written as they are, and have no signature.
        let id = create_with_signature(repo, &exact(&unsigned), None, Some(c"ignored")).unwrap();
        let e = extract_signature(repo, id.as_ref(), None).unwrap_err();
        assert_eq!(
            (e.code(), e.class()),
            (Some(ErrorCode::NotFound), Some(ErrorClass::Object))
        );
        assert_eq!(e.message(), c"this commit is not signed");
    }

    /// Every failure leaves nothing behind: contents that do not parse, a signed one
    /// without a blank line, a tree that does not exist, an object that is not a commit,
    /// an id that names nothing, and a repository without paths.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failures_leave_nothing_behind() {
        let (temp, tree) = repo_with_tree("failures");
        let repo = repository(&temp);
        let good = content(&tree, &[], "m\n");
        for bad in [
            "",
            "tree xyz\n",
            "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n",
        ] {
            let e = create_with_signature(repo, &exact(bad), Some(c"sig"), None).unwrap_err();
            assert_eq!(e.raw_code(), -1, "{bad:?}: {e:?}");
        }
        let headers_only = good.split_once("\n\n").unwrap().0.to_owned() + "\n";
        let e = create_with_signature(repo, &exact(&headers_only), Some(c"sig"), None).unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        assert_eq!(e.message(), c"malformed commit contents");
        let missing = Oid::from_raw(&[7; 20], OidType::Sha1).unwrap();
        let orphan = content(&missing, &[], "m\n");
        assert!(create_with_signature(repo, &exact(&orphan), None, None).is_err());
        let pathless = Repository::new().unwrap();
        assert!(create_with_signature(pathless.as_ref(), &exact(&good), None, None).is_err());

        let unended = headers_only.trim_end().to_owned() + "\ngpgsig a\n b";
        let id = create_with_signature(repo, &exact(&unended), None, None).unwrap();
        let e = extract_signature(repo, id.as_ref(), None).unwrap_err();
        assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Object)));
        assert_eq!(e.message(), c"malformed header");

        let blob = write_raw(&temp, b"not a commit", ffi::GIT_OBJECT_BLOB);
        let e = extract_signature(repo, blob.as_ref(), None).unwrap_err();
        assert_eq!(
            (e.code(), e.class()),
            (Some(ErrorCode::NotFound), Some(ErrorClass::Invalid))
        );
        let e = extract_signature(repo, missing.as_ref(), None).unwrap_err();
        assert_eq!(e.code(), Some(ErrorCode::NotFound));
        assert!(extract_signature(pathless.as_ref(), missing.as_ref(), None).is_err());
    }

    /// An id of another type than the repository's, at any type byte, and an empty
    /// field are refused before libgit2 reads anything.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn foreign_ids_and_empty_fields_are_refused() {
        let (temp, tree) = repo_with_tree("refused");
        let repo = repository(&temp);
        let id = create_with_signature(repo, &exact(&content(&tree, &[], "m\n")), Some(c"s"), None)
            .unwrap();
        let mut wide = Oid::from_raw(&id.as_ref().id(), OidType::Sha256).unwrap();
        for kind in [OidType::Sha256.raw() as u8, 0, 9, u8::MAX] {
            wide.as_mut().set_raw_kind(kind);
            let e = extract_signature(repo, wide.as_ref(), None).unwrap_err();
            assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
        }
        let e = extract_signature(repo, id.as_ref(), Some(c"")).unwrap_err();
        assert_eq!(e.message(), c"the signature header field name is empty");
        let (signature, _) = extract_signature(repo, id.as_ref(), Some(c"gpgsig")).unwrap();
        assert_eq!(signature.as_ref().as_bytes(), b"s");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signatures_on_many_threads() {
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let (temp, tree) = repo_with_tree(&format!("thread-{t}"));
                    let repo = repository(&temp);
                    let text = content(&tree, &[], &format!("thread {t}\n"));
                    let id =
                        create_with_signature(repo, &exact(&text), Some(c"sig"), None).unwrap();
                    let (sig, signed) = extract_signature(repo, id.as_ref(), None).unwrap();
                    assert_eq!(sig.as_ref().as_bytes(), b"sig");
                    assert_eq!(signed.as_ref().as_bytes(), text.as_bytes());
                });
            }
        });
    }
}

/// UB tests for signing through `CreateExtOptionsMut::set_signer`, which stores a
/// `SignatureCallback` in the options for `git_commit_create_ext` (driven through the
/// `create_ext` fixture until that routine is wrapped).
///
/// libgit2 calls the trampoline once per commit, with a builder lent for the call and
/// contents it frees when a header is added: the closure reads its copy of the contents
/// after adding headers (ASan: use after free), and its own state is written between
/// calls (BSan: nothing keeps a pointer into it). A refusal fails the commit with its
/// code, a panic with `GIT_EUSER`, kept until resumed. The options borrow the callback
/// for as long as they live (`compile_fail/`), replaced and cleared ones included, and
/// each thread signs in its own repository at once (TSan).
mod signers {
    use std::cell::{Cell, RefCell};
    use std::ffi::CStr;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::thread;

    use libgit2::libgit2::core::types::StreamError;
    use libgit2::libgit2::object::commit::{
        CommitBuilderMut, CreateExtOptions, Header, SignatureCallback,
    };
    use libgit2::libgit2::repository::repository::RepositoryRef;
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use crate::fixture::TempRepo;
    use crate::fixture::create_ext::create;

    /// What the closure is told to return, and what it saw.
    struct Signer {
        rc: Cell<i32>,
        calls: Cell<usize>,
        content: RefCell<Vec<u8>>,
    }

    impl Signer {
        fn new() -> Self {
            Self {
                rc: Cell::new(0),
                calls: Cell::new(0),
                content: RefCell::new(Vec::new()),
            }
        }

        /// Adds `signature` as `gpgsig`, then reads the contents again, and returns
        /// the code it is told to.
        fn sign(
            &self,
            mut builder: CommitBuilderMut<'_>,
            content: &CStr,
            signature: &CStr,
        ) -> Result<(), StreamError> {
            self.calls.set(self.calls.get() + 1);
            builder
                .add_header(c"gpgsig", signature)
                .map_err(StreamError::from)?;
            *self.content.borrow_mut() = content.to_bytes().to_vec();
            match self.rc.get() {
                rc if rc >= 0 => Ok(()),
                rc => Err(StreamError::new(ErrorClass::Callback, c"refused")
                    .with_code(ErrorCode::from_raw(rc).unwrap())),
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signers_sign_each_commit_once() {
        let repo = TempRepo::new("signer-each");
        let state = Signer::new();
        let mut f = |builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, content: &CStr| {
            state.sign(builder, content, c"-----BEGIN-----\nsig\n-----END-----")
        };
        let mut signer = SignatureCallback::new(&mut f);
        let headers = [Header::new(c"x-extra", c"before the signature")];
        let mut options = CreateExtOptions::new();
        options.as_mut().set_extra_headers(&headers);
        options.as_mut().set_signer(Some(&mut signer));
        let text = create(&repo, &options).unwrap();
        assert_eq!(state.calls.get(), 1);
        assert!(
            text.contains(
                "\nx-extra before the signature\ngpgsig -----BEGIN-----\n sig\n -----END-----\n"
            ),
            "{text}"
        );
        let unsigned = String::from_utf8(state.content.borrow().clone()).unwrap();
        assert!(!unsigned.contains("gpgsig") && unsigned.ends_with("\nmsg\n"));

        for (code, written) in [(1, true), (-30, false), (-1, false), (-7, false)] {
            state.rc.set(code);
            let result = create(&repo, &options);
            assert_eq!(result.as_ref().err(), (!written).then_some(&code), "{code}");
            assert_eq!(result.map(|t| t.contains("gpgsig")).unwrap_or(false), written);
        }
        drop(signer);
        assert_eq!(state.calls.get(), 5);
    }

    /// A replaced signer is not called again, a cleared one calls nothing, and both
    /// stay borrowed until the options are dropped.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn replaced_and_cleared_signers() {
        let repo = TempRepo::new("signer-replaced");
        let (first, second) = (Signer::new(), Signer::new());
        let mut f1 = |b: CommitBuilderMut<'_>, _: RepositoryRef<'_>, c: &CStr| first.sign(b, c, c"1");
        let mut f2 = |b: CommitBuilderMut<'_>, _: RepositoryRef<'_>, c: &CStr| second.sign(b, c, c"2");
        let (mut s1, mut s2) = (SignatureCallback::new(&mut f1), SignatureCallback::new(&mut f2));
        let mut options = CreateExtOptions::new();
        options.as_mut().set_signer(Some(&mut s1));
        options.as_mut().set_signer(Some(&mut s2));
        assert!(options.as_ref().has_signer());
        assert!(create(&repo, &options).unwrap().contains("\ngpgsig 2\n"));
        assert!(create(&repo, &options).unwrap().contains("\ngpgsig 2\n"));
        options.as_mut().set_signer(None);
        assert!(!options.as_ref().has_signer());
        assert!(!create(&repo, &options).unwrap().contains("gpgsig"));
        drop((s1, s2));
        assert_eq!((first.calls.get(), second.calls.get()), (0, 2));
    }

    /// A panic fails its commit with `GIT_EUSER` and every later one until it is
    /// resumed, once the options are gone; the callback then signs again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signer_panics_are_kept_until_resumed() {
        let repo = TempRepo::new("signer-panics");
        let state = Signer::new();
        let mut f = |builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, content: &CStr| {
            if state.calls.get() == 1 {
                state.calls.set(2);
                panic!("signing panicked");
            }
            state.sign(builder, content, c"ok")
        };
        let mut signer = SignatureCallback::new(&mut f);
        let mut options = CreateExtOptions::new();
        options.as_mut().set_signer(Some(&mut signer));
        assert!(create(&repo, &options).is_ok());
        assert_eq!(create(&repo, &options), Err(ErrorCode::User.raw()));
        assert_eq!(create(&repo, &options), Err(ErrorCode::User.raw()));
        let caught = catch_unwind(AssertUnwindSafe(|| signer.resume_panic()));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"signing panicked")
        );
        let mut options = CreateExtOptions::new();
        options.as_mut().set_signer(Some(&mut signer));
        assert!(create(&repo, &options).unwrap().contains("\ngpgsig ok\n"));
        drop(signer);
        assert_eq!(state.calls.get(), 3);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signers_on_many_threads() {
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let repo = TempRepo::new(&format!("signer-thread-{t}"));
                    let state = Signer::new();
                    let mut f =
                        |b: CommitBuilderMut<'_>, _: RepositoryRef<'_>, c: &CStr| state.sign(b, c, c"t");
                    let mut signer = SignatureCallback::new(&mut f);
                    let mut options = CreateExtOptions::new();
                    options.as_mut().set_signer(Some(&mut signer));
                    for _ in 0..4 {
                        assert!(create(&repo, &options).unwrap().contains("\ngpgsig t\n"));
                    }
                    drop(signer);
                    assert_eq!(state.calls.get(), 4);
                });
            }
        });
    }
}
