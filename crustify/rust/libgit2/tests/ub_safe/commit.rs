//! UB tests for `git2/commit.h`: the `Commit` owner, `CommitArc<'r>`, its lookups,
//! handles, getters and object conversions; the `CommitArray` value held by its `CommitArrayVal`
//! owner, its handles, the commits it lends and moves in and out, and the
//! `git_commitarray_dispose` policy; the `Header` value; the `CreateExtOptions` and
//! `CreateOptions` values and their handles; reading a commit in [`reading`]; writing
//! commits in [`creating`]; and the creation callback in [`create_callback`].
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
use libgit2::libgit2::object::commit::{CommitArray, CommitArrayRef, CommitArrayVal, CommitRef};

use crate::fixture::commit_and_config::{EMPTY, MERGEDREPO, OpenRepo, TESTREPO, commit_parents};
use crate::fixture::isolated;

/// The hex id of a commit, through [`CommitRef::id`].
pub fn commit_id(commit: CommitRef<'_, '_>) -> String {
    commit.id().to_hex_string().unwrap()
}

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
    use libgit2::libgit2::object::tree::Tree;
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryError};
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use super::commit_id;
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
        let commit = Commit::from_object(object).unwrap();
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
        let again = Commit::from_object(back).unwrap();
        assert_eq!(hex(&again), COMMITS[1]);

        for (other, kind) in [
            (TREE, ObjectType::Tree),
            (BLOB, ObjectType::Blob),
            (TAG, ObjectType::Tag),
        ] {
            let object = Object::lookup(r, id(other).as_ref(), Some(kind)).unwrap();
            assert!(CommitRef::from_object(object.as_ref()).is_none());
            let object = Commit::from_object(object).unwrap_err();
            assert_eq!(object_view(&object).0, other);
        }
        // A tag peeled to its commit converts.
        let tag = Object::lookup(r, id(TAG).as_ref(), None).unwrap();
        let peeled = tag.as_ref().peel(Some(ObjectType::Commit)).unwrap();
        drop(tag);
        let commit = Commit::from_object(peeled).unwrap();
        assert_eq!(hex(&commit), "e90810b8df3e80c413d903f631643c716887138d");
    }

    /// Parents, first-parent ancestors and trees, each derived from a commit whose owner
    /// is then dropped, and from those in turn, released in every order (ASan: a read of
    /// a commit freed under a derived owner; LSan: a reference never given back). The
    /// commits an array lends derive owners that outlive the array.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn derived_owners_outlive_what_they_came_from() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        let mut commits: Vec<CommitArc<'_>> = Vec::new();
        let mut trees = Vec::new();
        let mut base = Commit::lookup(r, id(COMMITS[0]).as_ref()).unwrap();
        for generation in 0..5 {
            let parent = base.as_ref().parent(0).unwrap();
            let ancestor = base.as_ref().nth_gen_ancestor(1).unwrap();
            assert!(CommitArc::ptr_eq(&parent, &ancestor));
            trees.push(base.as_ref().tree().unwrap());
            if base.as_ref().parent(1).is_ok() {
                commits.push(base.as_ref().parent(1).unwrap());
            }
            commits.push(base.as_ref().nth_gen_ancestor(0).unwrap());
            drop(base);
            if generation % 2 == 0 {
                drop(ancestor);
                base = parent;
            } else {
                commits.push(parent);
                base = ancestor;
            }
        }
        assert_eq!(hex(&base), COMMITS[6]);
        drop(base);
        for (i, commit) in commits.iter().enumerate() {
            assert!(COMMITS.contains(&hex(commit).as_str()), "{i}");
        }
        for tree in &trees {
            assert!(
                !tree
                    .as_ref()
                    .as_object()
                    .id()
                    .to_hex_string()
                    .unwrap()
                    .is_empty()
            );
        }
        // From the middle out, trees and commits interleaved.
        while !commits.is_empty() || !trees.is_empty() {
            if !commits.is_empty() {
                drop(commits.remove(commits.len() / 2));
            }
            if !trees.is_empty() {
                drop(trees.remove(0));
            }
        }

        let array = CommitArray::from_commits(vec![
            Commit::lookup(r, id(COMMITS[1]).as_ref()).unwrap(),
            Commit::lookup(r, id(COMMITS[3]).as_ref()).unwrap(),
        ])
        .unwrap();
        let derived: Vec<_> = array
            .as_ref()
            .iter()
            .flat_map(|c| [c.parent(0).unwrap(), c.nth_gen_ancestor(2).unwrap()])
            .collect();
        let tree = array.as_ref().get(0).unwrap().tree().unwrap();
        drop(array);
        assert_eq!(
            derived.iter().map(hex).collect::<Vec<_>>(),
            [COMMITS[3], COMMITS[4], COMMITS[4], COMMITS[5]]
        );
        drop(derived);
        assert_eq!(
            object_view(&Tree::into_object(tree)).1,
            libgit2::ffi::GIT_OBJECT_TREE
        );
    }

    /// With a one-byte cache every derived commit and tree is evicted while its owner
    /// holds it, and with the cache off each derivation parses afresh: each owner keeps
    /// its object alive either way.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn derived_owners_survive_eviction_and_a_disabled_cache() {
        isolated(
            concat!(
                module_path!(),
                "::derived_owners_survive_eviction_and_a_disabled_cache"
            ),
            || {
                set_cache_max_size(1);
                let repo = open_testrepo();
                let head = Commit::lookup(repo.as_ref(), id(COMMITS[0]).as_ref()).unwrap();
                let derived: Vec<_> = (0..6)
                    .map(|n| head.as_ref().nth_gen_ancestor(n).unwrap())
                    .collect();
                let trees: Vec<_> = derived.iter().map(|c| c.as_ref().tree().unwrap()).collect();
                drop(head);
                for (commit, expected) in derived.iter().zip([0, 1, 3, 4, 5, 6]) {
                    assert_eq!(hex(commit), COMMITS[expected]);
                }
                drop(derived);
                enable_caching(false);
                let head = Commit::lookup(repo.as_ref(), id(COMMITS[0]).as_ref()).unwrap();
                let a = head.as_ref().parent(0).unwrap();
                let b = head.as_ref().parent(0).unwrap();
                assert!(!CommitArc::ptr_eq(&a, &b));
                let itself = head.as_ref().nth_gen_ancestor(0).unwrap();
                assert!(CommitArc::ptr_eq(&itself, &head));
                drop((head, a));
                assert_eq!(hex(&b), COMMITS[1]);
                drop((trees, itself, b));
            },
        );
    }

    /// Refused derivations leave nothing behind (LSan) and the commit usable: a parent
    /// index past the last, ancestors past a root at every width, and the same through a
    /// commit whose repository has no further objects to give.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn refused_derivations_leave_nothing() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        let root = Commit::lookup(r, id(COMMITS[6]).as_ref()).unwrap();
        let merge = Commit::lookup(r, id(COMMITS[1]).as_ref()).unwrap();
        for n in [0, 1, 2, u32::MAX] {
            assert_failed(
                root.as_ref().parent(n),
                ErrorCode::NotFound.raw(),
                ErrorClass::Invalid,
            );
        }
        for n in [2, 3, u32::MAX] {
            assert_failed(
                merge.as_ref().parent(n),
                ErrorCode::NotFound.raw(),
                ErrorClass::Invalid,
            );
        }
        for n in [1, 2, 6, 1000, u32::MAX] {
            assert_failed(
                root.as_ref().nth_gen_ancestor(n),
                ErrorCode::NotFound.raw(),
                ErrorClass::Invalid,
            );
        }
        assert_failed(
            merge.as_ref().nth_gen_ancestor(5),
            ErrorCode::NotFound.raw(),
            ErrorClass::Invalid,
        );
        assert_eq!(hex(&root.as_ref().nth_gen_ancestor(0).unwrap()), COMMITS[6]);
        assert_eq!(
            hex(&merge.as_ref().nth_gen_ancestor(4).unwrap()),
            COMMITS[6]
        );
        assert!(root.as_ref().tree().is_ok());
    }
}

/// UB tests for the getters of `CommitRef`: `id`, `owner`, `message`, `message_raw`,
/// `message_encoding`, `raw_header`, `header_field`, `summary`, `time`, `time_offset`,
/// `tree_id`, `parent_count` and `parent_id`.
///
/// Every string and id a getter returns lives inside the commit, borrowed for as long
/// as the handle it came from: the tests read each result after every other owner of
/// the commit, and every other commit, is gone (ASan: a getter tied to the wrong
/// commit, or reading past a string), and, with the cache off, after the earlier
/// lookup of the same id, a distinct commit, is freed. `summary` writes its result into
/// the commit on first use, through a shared handle: it is read through other handles
/// and lent array commits. A header field is a fresh buffer the caller owns, which
/// outlives the commit and the repository (LSan: leak; ASan: double free). A field
/// holding a newline, which libgit2 would match across line ends and copy with a
/// negative length (UBSan: pointer overflow; ASan: an allocation of nearly
/// `usize::MAX` bytes), is refused before C. The repository `owner` hands back is the
/// one the commit was looked up through, which outlives the commit.
///
/// Results outliving their commit do not compile (the getters' doctests). Commits are
/// neither `Send` nor `Sync`, so there is no thread test. Every test calls libgit2, so
/// Miri skips them all.
mod getters {
    use std::ffi::{CStr, CString};

    use libgit2::ffi;
    use libgit2::libgit2::object::commit::{Commit, CommitArc, CommitArray};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use crate::fixture::TempRepo;
    use crate::fixture::isolated;
    use crate::fixture::object_and_tree_entry::{
        enable_caching, open_testrepo, repository, write_raw,
    };

    /// `testrepo.git`'s commits: `HEAD`, the merge, a root.
    const HEAD: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
    const MERGE: &str = "be3563ae3f795b2b4353bcce3a527ad0a4f7f644";
    const ROOT: &str = "8496071c1b46c854b31185ea97743be6a8774479";
    const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    fn id(hex: &str) -> Oid {
        Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap()
    }

    /// Everything the getters return of one commit, the strings still borrowed.
    struct View<'a> {
        id: String,
        tree: String,
        parents: Vec<String>,
        message: &'a CStr,
        raw: &'a CStr,
        encoding: Option<&'a CStr>,
        header: &'a CStr,
        summary: &'a CStr,
        time: (i64, i32),
    }

    fn view<'a>(commit: &'a CommitArc<'_>) -> View<'a> {
        let c = commit.as_ref();
        let count = c.parent_count();
        let parents = (0..count)
            .map(|n| c.parent_id(n).unwrap().to_hex_string().unwrap())
            .collect();
        assert!(c.parent_id(count).is_none() && c.parent_id(u32::MAX).is_none());
        View {
            id: c.id().to_hex_string().unwrap(),
            tree: c.tree_id().to_hex_string().unwrap(),
            parents,
            message: c.message(),
            raw: c.message_raw(),
            encoding: c.message_encoding(),
            header: c.raw_header(),
            summary: c.summary().unwrap(),
            time: (c.time(), c.time_offset()),
        }
    }

    /// Reads every byte of a view's strings.
    fn touch(v: &View<'_>) -> usize {
        let strs = [v.message, v.raw, v.header, v.summary];
        let n: usize = strs.iter().map(|s| s.to_bytes().len()).sum();
        n + v.encoding.map_or(0, |e| e.to_bytes().len()) + v.id.len() + v.tree.len()
    }

    /// The strings of commits held by one owner each, read after every other owner,
    /// lookup and id is gone; the summary is computed through one handle and read
    /// through another, and through a lent array commit.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn results_borrow_from_the_commit_they_read() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        let mut held = Vec::new();
        for hex in [HEAD, MERGE, ROOT] {
            let mut oid = Box::new(id(hex));
            let commit = Commit::lookup(r, Oid::as_ref(&oid)).unwrap();
            Oid::as_mut(&mut oid).set_id([0xff; 32]);
            drop(oid);
            let other = commit.clone();
            held.push(commit);
            drop(other);
        }
        let views: Vec<_> = held.iter().map(view).collect();
        assert_eq!(views[0].id, HEAD);
        assert_eq!(views[1].parents.len(), 2);
        assert!(views[2].parents.is_empty());
        assert_eq!(views[1].summary, c"Merge branch 'br2'");
        assert!(views.iter().map(touch).sum::<usize>() > 0);
        assert_eq!(views[0].time, (1_312_943_626, -420));
        let array = CommitArray::from_commits(held.clone()).unwrap();
        let lent = array.as_ref().get(1).unwrap();
        assert_eq!(lent.summary().unwrap().as_ptr(), views[1].summary.as_ptr());
        drop(views);
        drop(held);
        // The array's references keep the commits, and their strings, alive.
        let lent = array.as_ref().get(2).unwrap();
        assert_eq!(lent.message(), c"testing\n");
        assert_eq!(lent.summary(), Ok(c"testing"));
    }

    /// With the cache off every lookup parses a fresh commit: the results of the
    /// second stay valid once the first is freed, and the summaries are distinct.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn results_of_fresh_commits_outlive_earlier_ones() {
        isolated(
            concat!(
                module_path!(),
                "::results_of_fresh_commits_outlive_earlier_ones"
            ),
            || {
                enable_caching(false);
                let repo = open_testrepo();
                let first = Commit::lookup(repo.as_ref(), id(MERGE).as_ref()).unwrap();
                let second = Commit::lookup(repo.as_ref(), id(MERGE).as_ref()).unwrap();
                assert!(!CommitArc::ptr_eq(&first, &second));
                let a = first.as_ref().summary().unwrap().as_ptr();
                let kept = view(&second);
                assert_ne!(a, kept.summary.as_ptr());
                drop(first);
                assert_eq!(kept.summary, c"Merge branch 'br2'");
                assert!(touch(&kept) > 0);
            },
        );
    }

    /// Header fields are fresh buffers the caller owns: they outlive the commit and
    /// the repository. Fields holding a newline never reach C, including the two that
    /// overflow in it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn header_fields_are_owned_and_newlines_never_reach_c() {
        let repo = open_testrepo();
        let commit = Commit::lookup(repo.as_ref(), id(HEAD).as_ref()).unwrap();
        let mut fields = Vec::new();
        for name in [c"tree", c"parent", c"author", c"committer"] {
            fields.push(commit.as_ref().header_field(name).unwrap());
        }
        for name in [c"", c"tre", c"parents", c"gpgsig", c"encoding"] {
            let e = commit.as_ref().header_field(name).unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::NotFound), "{name:?}");
        }
        let overflowing: [&CStr; 3] = [
            c"tree 944c0f6e4dfa41595e6eb3ceecdb14f50fe18162\nparent",
            c"tree 944c0f6e4dfa41595e6eb3ceecdb14f50fe18162\nparent be3563ae3f795b2b4353bcce3a527ad0a4f7f644\nauthor",
            c"tree 944c0f6e4dfa41595e6eb3ceecdb14f50fe18162\nparent be3563ae3f795b2b4353bcce3a527ad0a4f7f644\nauthor Scott Chacon <schacon@gmail.com> 1312943626 -0700\ncommitter",
        ];
        for name in overflowing {
            let e = commit.as_ref().header_field(name).unwrap_err();
            assert_eq!(
                (e.code(), e.class()),
                (Some(ErrorCode::NotFound), Some(ErrorClass::Object))
            );
        }
        let owner = commit.as_ref().owner();
        drop(commit);
        // The owner is the repository's handle again, outliving the commit.
        let again = Commit::lookup(owner, id(HEAD).as_ref()).unwrap();
        assert_eq!(again.as_ref().owner().path(), repo.as_ref().path());
        drop(again);
        drop(repo);
        let read: Vec<_> = fields
            .iter()
            .map(|f| f.as_ref().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(read[0], "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162");
        assert_eq!(read[1], "be3563ae3f795b2b4353bcce3a527ad0a4f7f644");
        assert_eq!(read[2], "Scott Chacon <schacon@gmail.com> 1312943626 -0700");
    }

    /// Hand-made commits, each in an exact-length object: an `encoding` header, a
    /// multi-line header and a header whose last continuation line does not end, no
    /// message at all, leading newlines and whitespace-only lines in the message.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn crafted_commits_read_in_bounds() {
        let tmp = TempRepo::new("commit-getters");
        let who = "A U Thor <a@example.com> 1700000000 +0130";
        let head = format!("tree {EMPTY_TREE}\nauthor {who}\ncommitter {who}\n");
        let cases: [(String, &CStr, Option<&CStr>, &CStr); 5] = [
            (
                format!("{head}encoding ISO-8859-1\n\nLatin\n"),
                c"Latin\n",
                Some(c"ISO-8859-1"),
                c"Latin",
            ),
            (
                format!(
                    "{head}gpgsig -----BEGIN-----\n line\n -----END-----\nx y\n\n\n\nfirst  line\n\tcontinued\n  \nbody\n"
                ),
                c"first  line\n\tcontinued\n  \nbody\n",
                None,
                c"first  line continued",
            ),
            (format!("{head}foo bar\n baz"), c"", None, c""),
            (format!("{head}\n"), c"", None, c""),
            (format!("{head}\n \t\n  \n"), c" \t\n  \n", None, c""),
        ];
        let mut held = Vec::new();
        for (raw, message, encoding, summary) in &cases {
            let oid = write_raw(&tmp, raw.as_bytes(), ffi::GIT_OBJECT_COMMIT);
            let commit = Commit::lookup(repository(&tmp), oid.as_ref()).unwrap();
            let c = commit.as_ref();
            assert_eq!(
                (c.message(), c.message_encoding()),
                (*message, *encoding),
                "{raw:?}"
            );
            assert_eq!(c.summary(), Ok(*summary), "{raw:?}");
            assert_eq!((c.time(), c.time_offset()), (1_700_000_000, 90));
            assert_eq!(c.parent_count(), 0);
            assert!(raw.as_bytes().starts_with(c.raw_header().to_bytes()));
            held.push(commit);
        }
        let signed = held[1].as_ref();
        let sig = signed.header_field(c"gpgsig").unwrap();
        assert_eq!(
            sig.as_ref().as_bytes(),
            b"-----BEGIN-----\nline\n-----END-----"
        );
        assert_eq!(signed.header_field(c"x").unwrap().as_ref().as_bytes(), b"y");
        let malformed = held[2].as_ref().header_field(c"foo").unwrap_err();
        assert_eq!(malformed.message(), c"malformed header");
        assert_eq!(held[2].as_ref().message_raw(), c"");
        assert_eq!(held[0].as_ref().message_encoding(), Some(c"ISO-8859-1"));
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
/// nothing: one test hands them to `create_ext`, its strings and headers on the heap,
/// freed right after the call (ASan, BSan: a retained pointer). Options that
/// outlive their borrows, aliased handles and a signer that does not outlive the options
/// do not compile (`compile_fail/` and the type's doctests). The options are neither `Send`
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
        let text = create(&repo, &mut options).unwrap();
        drop(headers);
        drop((value, encoding));
        assert!(
            text.contains("\nencoding ISO-8859-1\nx-a one\n two\nx-b \n\nmsg\n"),
            "{text}"
        );
        assert!(has_reference(&repo, &name));

        // The default options write a plain commit and move no reference.
        let text = create(&repo, &mut CreateExtOptions::new()).unwrap();
        assert!(text.ends_with("0000\n\nmsg\n"), "{text}");
    }

}

/// The commit-on-`HEAD` options: the `CreateOptions` value (`git_commit_create_options`)
/// and its handles.
///
/// The options are a version, a one-bit flag, two borrowed signatures, a borrowed string,
/// a borrowed run of headers and a callback with its payload. Their handles project each
/// field out of a pointer — the flag through its bitfield's storage byte — and rebuild a
/// `SignatureRef`, a `&CStr` or a run of headers from it, so these tests exercise every
/// getter and setter on every holder Rust has for one (a local, a heap slot, an element
/// of a run that moves when it grows), through reborrows, with signatures (borrowed views
/// and owned boxes), strings and headers on the heap dropped right after their last read
/// (ASan: a getter reading past them or after them). libgit2 reads the options for one
/// call and keeps nothing: two tests hand them to `git_commit_create_from_tree` and
/// `git_commit_amend_from_tree` (through the `create_options` fixture until those are
/// wrapped), everything they borrow on the heap and freed right after the call (ASan,
/// BSan: a retained pointer), one with a signer whose closure writes its own state
/// between calls. Options that outlive their borrows, aliased handles and a signer that
/// does not outlive the options do not compile (`compile_fail/` and the type's doctests).
/// The options are neither `Send` nor `Sync`; each thread builds its own and commits in
/// its own repository at once (TSan). `CreateOptions::new` fills the options through
/// libgit2, so Miri skips every test here; the module's `unit_tests` run the same
/// projections under Miri over options built in Rust.
mod create_options {
    use std::cell::Cell;
    use std::ffi::{CStr, CString};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::thread;

    use libgit2::libgit2::core::types::{
        Signature, SignatureBox, SignatureRef, SignatureView, StreamError, Time,
    };
    use libgit2::libgit2::object::commit::{
        CommitBuilderMut, CreateOptions, CreateOptionsMut, CreateOptionsRef, Header,
        SignatureCallback,
    };
    use libgit2::libgit2::repository::repository::RepositoryRef;
    use libgit2::libgit2::util::errors::ErrorCode;

    use crate::fixture::TempRepo;
    use crate::fixture::create_options::{amend, create};

    /// What the getters read: the flag, the two signatures' names, the encoding and the
    /// headers.
    type Fields<'o> = (
        bool,
        Option<&'o CStr>,
        Option<&'o CStr>,
        Option<&'o CStr>,
        Vec<(&'o CStr, &'o CStr)>,
    );

    fn name<'o>(s: Option<SignatureRef<'o>>) -> Option<&'o CStr> {
        s.map(|s| s.name().unwrap())
    }

    fn read<'o>(r: CreateOptionsRef<'_, 'o>) -> Fields<'o> {
        let headers = r
            .extra_headers()
            .iter()
            .map(|h| (h.field(), h.value()))
            .collect();
        (
            r.allow_empty_commit(),
            name(r.author()),
            name(r.committer()),
            r.message_encoding(),
            headers,
        )
    }

    fn write<'o>(
        mut m: CreateOptionsMut<'_, 'o>,
        allow: bool,
        who: Option<SignatureRef<'o>>,
        encoding: Option<&'o CStr>,
        headers: &'o [Header<'o>],
    ) {
        m.set_allow_empty_commit(allow);
        m.set_author(who);
        m.set_committer(who);
        m.set_message_encoding(encoding);
        m.set_extra_headers(headers);
    }

    /// An owned signature of `name`, its strings and struct on libgit2's heap.
    fn owned(name: &CStr) -> SignatureBox {
        Signature::new(name, c"o@example.com", 7, 0).unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn every_field_round_trips_on_every_holder() {
        let strings: Vec<CString> = ["Name", "UTF-16", "x", &"y".repeat(1024)]
            .into_iter()
            .map(|s| CString::new(s).unwrap())
            .collect();
        let views: Vec<SignatureView<'_>> = strings
            .iter()
            .map(|s| SignatureView::new(s, s, Time::new(0, 0)))
            .collect();
        let boxes: Vec<SignatureBox> = strings.iter().map(|s| owned(s)).collect();
        let headers: Vec<Header<'_>> = strings.iter().map(|s| Header::new(s, s)).collect();
        let mut local = CreateOptions::new();
        let mut boxed = Box::new(CreateOptions::new());
        let mut run = vec![CreateOptions::new()];
        for i in 0..strings.len() {
            let allow = i % 2 == 0;
            let who = if allow {
                views[i].as_ref()
            } else {
                boxes[i].as_ref()
            };
            let encoding = strings.get(i + 1).map(|s| s.as_c_str());
            let hs = &headers[i..];
            write(local.as_mut(), allow, Some(who), encoding, hs);
            write(
                CreateOptions::as_mut(&mut boxed),
                allow,
                Some(who),
                encoding,
                hs,
            );
            // Growing the run moves its elements; each handle is taken afresh.
            run.push(CreateOptions::new());
            write(run[i].as_mut(), allow, Some(who), encoding, hs);
            let expected = (
                allow,
                Some(strings[i].as_c_str()),
                Some(strings[i].as_c_str()),
                encoding,
                hs.iter()
                    .map(|h| (h.as_ref().field(), h.as_ref().value()))
                    .collect::<Vec<_>>(),
            );
            for r in [
                local.as_ref(),
                CreateOptions::as_ref(&boxed),
                run[i].as_ref(),
            ] {
                assert_eq!(read(r), expected);
                assert_eq!(r.version(), 1);
                assert!(!r.has_signer());
                assert_eq!(r.author().unwrap(), who);
            }
            assert_eq!(
                read(run[i + 1].as_ref()),
                (false, None, None, None, Vec::new())
            );
        }
        write(local.as_mut(), false, None, None, &[]);
        assert_eq!(read(local.as_ref()), (false, None, None, None, Vec::new()));
    }

    /// Signatures, strings and headers on the heap, freed right after the options' last
    /// use.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reads_stay_inside_heap_borrows() {
        for len in [1, 7, 8, 15, 16, 255] {
            let name = CString::new("n".repeat(len)).unwrap();
            let view = SignatureView::new(&name, &name, Time::new(1, 1));
            let signature = owned(&name);
            let headers: Vec<Header<'_>> =
                (0..len % 5).map(|_| Header::new(&name, &name)).collect();
            let mut options = CreateOptions::new();
            write(
                options.as_mut(),
                true,
                Some(view.as_ref()),
                Some(&name),
                &headers,
            );
            options.as_mut().set_committer(Some(signature.as_ref()));
            let (allow, author, committer, encoding, hs) = read(options.as_ref());
            assert!(allow);
            assert_eq!(author.unwrap().to_bytes().len(), len);
            assert_eq!(committer.unwrap().to_bytes().len(), len);
            assert_eq!(encoding.unwrap().to_bytes().len(), len);
            assert_eq!(options.as_ref().committer().unwrap().when().seconds(), 7);
            assert_eq!(hs.len(), len % 5);
            assert!(format!("{options:?}").len() > 3 * len);
            drop((headers, signature, view));
            drop(name);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reborrows_keep_one_writer() {
        let headers = [Header::new(c"f", c"v")];
        let who = SignatureView::new(c"W", c"w@example.com", Time::new(0, 0));
        let mut options = CreateOptions::default();
        let mut m = options.as_mut();
        {
            let mut inner = m.as_mut();
            inner.set_extra_headers(&headers);
            inner.set_allow_empty_commit(true);
            assert_eq!(inner.as_ref().extra_headers().len(), 1);
        }
        let shared = m.as_ref();
        assert_eq!(read(shared).4, [(c"f", c"v")]);
        assert!(read(shared).0);
        m.set_author(Some(who.as_ref()));
        assert_eq!(name(options.as_ref().author()), Some(c"W"));
        assert_eq!(format!("{:?}", options.as_mut()), format!("{options:?}"));
    }

    /// libgit2 reads the options for the call only: the commits it writes carry the
    /// signatures, the encoding and the headers, empty commits are refused until
    /// allowed, an amend reads the same struct, and nothing it was given is read after
    /// the call (every borrow is on the heap and freed right away).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn libgit2_reads_the_options_for_the_call_only() {
        let repo = TempRepo::new("commit-create-options");
        let author_name = CString::new("A U Thor").unwrap();
        let author_mail = CString::new("author@example.com").unwrap();
        let author = Box::new(SignatureView::new(
            &author_name,
            &author_mail,
            Time::new(100, 60),
        ));
        let committer = owned(c"Committer");
        let encoding = CString::new("ISO-8859-1").unwrap();
        let value = CString::new("one\ntwo").unwrap();
        let headers = vec![Header::new(c"x-a", &value), Header::new(c"x-b", c"")];
        let mut options = CreateOptions::new();
        write(
            options.as_mut(),
            false,
            Some(SignatureView::as_ref(&author)),
            Some(&encoding),
            &headers,
        );
        options.as_mut().set_committer(Some(committer.as_ref()));
        // The empty tree on an unborn `HEAD` changes nothing.
        assert_eq!(create(&repo, &options), Err(ErrorCode::Generic.raw()));
        options.as_mut().set_allow_empty_commit(true);
        let text = create(&repo, &options).unwrap();
        drop(headers);
        drop((value, encoding, committer));
        assert!(
            text.contains(
                "\nauthor A U Thor <author@example.com> 100 +0100\n\
                 committer Committer <o@example.com> 7 +0000\n\
                 encoding ISO-8859-1\nx-a one\n two\nx-b \n\nmsg\n"
            ),
            "{text}"
        );

        // Amending keeps the amended commit's encoding and author where unset.
        let mut options = CreateOptions::new();
        options
            .as_mut()
            .set_committer(Some(SignatureView::as_ref(&author)));
        let text = amend(&repo, None, &options).unwrap();
        drop(author);
        drop((author_name, author_mail));
        assert!(
            text.contains(
                "\nauthor A U Thor <author@example.com> 100 +0100\n\
                 committer A U Thor <author@example.com> 100 +0100\n\
                 encoding ISO-8859-1\n\nmsg\n"
            ),
            "{text}"
        );
    }

    /// A signer stored in the options signs each commit created or amended with them,
    /// its closure writing its own state between calls; a refusal fails the commit with
    /// its code, a panic with `GIT_EUSER`, kept until resumed once the options are gone.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signers_sign_through_the_options() {
        let repo = TempRepo::new("commit-create-options-signer");
        let who = SignatureView::new(c"S", c"s@example.com", Time::new(5, 0));
        let calls = Cell::new(0);
        let mut f = |mut builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, content: &CStr| {
            calls.set(calls.get() + 1);
            builder
                .add_header(c"gpgsig", c"sig")
                .map_err(StreamError::from)?;
            // The copy of the contents outlives the buffer the header replaced.
            assert!(!content.to_bytes().windows(6).any(|w| w == b"gpgsig"));
            match calls.get() {
                3 => Err(StreamError::new(
                    libgit2::libgit2::util::errors::ErrorClass::Callback,
                    c"refused",
                )
                .with_code(ErrorCode::Ambiguous)),
                4 => panic!("signing panicked"),
                _ => Ok(()),
            }
        };
        let mut signer = SignatureCallback::new(&mut f);
        {
            let mut options = CreateOptions::new();
            let mut m = options.as_mut();
            m.set_allow_empty_commit(true);
            m.set_author(Some(who.as_ref()));
            m.set_committer(Some(who.as_ref()));
            m.set_signer(Some(&mut signer));
            assert!(create(&repo, &options).unwrap().contains("\ngpgsig sig\n"));
            assert!(amend(&repo, Some(c"amended\n"), &options).is_ok());
            assert_eq!(create(&repo, &options), Err(ErrorCode::Ambiguous.raw()));
            assert_eq!(create(&repo, &options), Err(ErrorCode::User.raw()));
            assert_eq!(amend(&repo, None, &options), Err(ErrorCode::User.raw()));
        }
        let caught = catch_unwind(AssertUnwindSafe(|| signer.resume_panic()));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"signing panicked")
        );
        let mut options = CreateOptions::new();
        options.as_mut().set_allow_empty_commit(true);
        options.as_mut().set_author(Some(who.as_ref()));
        options.as_mut().set_committer(Some(who.as_ref()));
        options.as_mut().set_signer(Some(&mut signer));
        assert!(create(&repo, &options).unwrap().contains("\ngpgsig sig\n"));
        drop(signer);
        assert_eq!(calls.get(), 5);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn options_on_many_threads() {
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let repo = TempRepo::new(&format!("commit-create-options-thread-{t}"));
                    let name = CString::new(format!("T{t}")).unwrap();
                    let who = SignatureView::new(&name, c"t@example.com", Time::new(t, 0));
                    let headers = [Header::new(c"x-thread", &name)];
                    let mut options = CreateOptions::new();
                    write(options.as_mut(), true, Some(who.as_ref()), None, &headers);
                    for _ in 0..4 {
                        let text = create(&repo, &options).unwrap();
                        assert!(text.contains(&format!("\nx-thread T{t}\n")), "{text}");
                    }
                });
            }
        });
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
/// `SignatureCallback` in the options for `create_ext` (driven through the `create_ext`
/// fixture, which reads the commit back).
///
/// libgit2 calls the trampoline once per commit, with a builder lent for the call and
/// contents it frees when a header is added: the closure reads its copy of the contents
/// after adding headers (ASan: use after free), and its own state is written between
/// calls (BSan: nothing keeps a pointer into it). A refusal fails the commit with its
/// code, a panic with `GIT_EUSER`, resumed from `create_ext`. The options borrow the
/// callback for as long as they live (`compile_fail/`), replaced and cleared ones
/// included, and each thread signs in its own repository at once (TSan).
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
        let text = create(&repo, &mut options).unwrap();
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
            let result = create(&repo, &mut options);
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
        assert!(create(&repo, &mut options).unwrap().contains("\ngpgsig 2\n"));
        assert!(create(&repo, &mut options).unwrap().contains("\ngpgsig 2\n"));
        options.as_mut().set_signer(None);
        assert!(!options.as_ref().has_signer());
        assert!(!create(&repo, &mut options).unwrap().contains("gpgsig"));
        drop((s1, s2));
        assert_eq!((first.calls.get(), second.calls.get()), (0, 2));
    }

    /// A panic fails its commit with `GIT_EUSER` inside libgit2, and resumes from
    /// `create_ext` once libgit2 has returned, nothing written; the callback, still
    /// lent to the options, then signs again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signer_panics_resume_from_create_ext() {
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
        assert!(create(&repo, &mut options).is_ok());
        let caught = catch_unwind(AssertUnwindSafe(|| create(&repo, &mut options)));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"signing panicked")
        );
        assert!(create(&repo, &mut options).unwrap().contains("\ngpgsig ok\n"));
        // Nothing is left to resume.
        signer.resume_panic();
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
                        assert!(create(&repo, &mut options).unwrap().contains("\ngpgsig t\n"));
                    }
                    drop(signer);
                    assert_eq!(state.calls.get(), 4);
                });
            }
        });
    }
}

/// UB tests for reading a commit: `CommitRef::author`, `committer` and `body`, and
/// `author_with_mailmap` / `committer_with_mailmap`.
///
/// The signatures and the body are borrowed from the commit: every string is read whole
/// through every holder of a shared commit (lookups, clones, an object view), and after
/// the other holders are gone (ASan: use after free). libgit2 computes the body on the
/// first call and stores it in the commit through the shared handle, as a `Cell` is
/// written: a body read earlier stays valid while later calls return the same string
/// (ASan; BSan: the write goes through C's pointer, never through a Rust reference). The
/// mailmap copies are owned and released once (LSan, ASan). Commits are neither `Send`
/// nor `Sync`: each thread reads commits of its own repository (TSan). A signature or a
/// body outliving its commit is a doctest. Every test calls libgit2, so Miri skips them.
mod reading {
    use std::ffi::{CStr, CString};
    use std::thread;

    use libgit2::libgit2::core::types::{SignatureRef, SignatureView, Time};
    use libgit2::libgit2::object::commit::{Commit, CommitRef, create};
    use libgit2::libgit2::object::mailmap::Mailmap;
    use libgit2::libgit2::object::tree::Tree;
    use libgit2::libgit2::oid::oid::{Oid, OidType};

    use crate::fixture::TempRepo;
    use crate::fixture::object_and_tree_entry::repository;

    pub(super) const EMPTY_TREE: &CStr = c"4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    /// A signature's strings and time, copied out whole.
    fn signature(s: SignatureRef<'_>) -> (Vec<u8>, Vec<u8>, i64, i32) {
        let when = s.when();
        (
            s.name().unwrap().to_bytes().to_vec(),
            s.email().unwrap().to_bytes().to_vec(),
            when.seconds(),
            when.offset_minutes(),
        )
    }

    fn everything(c: CommitRef<'_, '_>) -> (Vec<u8>, Vec<u8>, i64, i32, Vec<u8>) {
        let (name, email, seconds, offset) = signature(c.author());
        let committer = signature(c.committer());
        assert_eq!(committer.0, name);
        let body = c.body().map(|b| b.to_bytes().to_vec()).unwrap_or_default();
        (name, email, seconds, offset, body)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signatures_and_bodies_are_read_through_every_holder() {
        let temp = TempRepo::new("commit-read-holders");
        let repo = repository(&temp);
        let tree = Tree::lookup(
            repo,
            Oid::from_string(EMPTY_TREE, OidType::Sha1)
                .unwrap()
                .as_ref(),
        );
        let tree = tree.unwrap();
        for len in [0usize, 1, 63, 64, 4097] {
            let name = CString::new("n".repeat(len + 1))
                .unwrap()
                .into_boxed_c_str();
            let email = CString::new(format!("{}@x", "e".repeat(len))).unwrap();
            let sig = SignatureView::new(&name, &email, Time::new(len as i64, -(len as i32)));
            let body = "b".repeat(len);
            let message = CString::new(format!("summary\n\n{body}\n")).unwrap();
            let id = create(
                repo,
                None,
                sig.as_ref(),
                sig.as_ref(),
                None,
                &message,
                tree.as_ref(),
                &[],
            )
            .unwrap();
            drop((name, email, message));
            let first = Commit::lookup(repo, id.as_ref()).unwrap();
            let second = Commit::lookup(repo, id.as_ref()).unwrap();
            let expected = everything(first.as_ref());
            assert_eq!(expected.4, body.as_bytes());
            let clone = first.clone();
            let early = clone.as_ref().body();
            let object = Commit::into_object(first);
            let view = CommitRef::from_object(object.as_ref()).unwrap();
            assert_eq!(everything(view), expected);
            assert_eq!(everything(second.as_ref()), expected);
            drop(object);
            assert_eq!(everything(clone.as_ref()), expected);
            drop(second);
            assert_eq!(early.map(|b| b.to_bytes().len()).unwrap_or(0), len);
            let author = clone.as_ref().author();
            let late = clone.as_ref().body();
            assert_eq!(signature(author).0.len(), len + 1);
            assert_eq!(late.map(|b| b.to_bytes().len()).unwrap_or(0), len);
        }
    }

    /// The body is stored in the commit on first use: earlier results stay valid while
    /// later calls, through any holder, return the same string.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn bodies_are_cached_in_the_shared_commit() {
        let temp = TempRepo::new("commit-body-cache");
        let repo = repository(&temp);
        let tree = Tree::lookup(
            repo,
            Oid::from_string(EMPTY_TREE, OidType::Sha1)
                .unwrap()
                .as_ref(),
        );
        let tree = tree.unwrap();
        let sig = SignatureView::new(c"A", c"a@x", Time::new(0, 0));
        let id = create(
            repo,
            None,
            sig.as_ref(),
            sig.as_ref(),
            None,
            c"s\n\n body \n",
            tree.as_ref(),
            &[],
        )
        .unwrap();
        let commit = Commit::lookup(repo, id.as_ref()).unwrap();
        let other = Commit::lookup(repo, id.as_ref()).unwrap();
        let bodies: Vec<&CStr> = (0..8)
            .map(|i| {
                if i % 2 == 0 {
                    commit.as_ref()
                } else {
                    other.as_ref()
                }
                .body()
                .unwrap()
            })
            .collect();
        for b in &bodies {
            assert_eq!(b.to_bytes(), b"body");
        }
        let none = create(
            repo,
            None,
            sig.as_ref(),
            sig.as_ref(),
            None,
            c"only\n",
            tree.as_ref(),
            &[],
        )
        .unwrap();
        let none = Commit::lookup(repo, none.as_ref()).unwrap();
        for _ in 0..4 {
            assert!(none.as_ref().body().is_none());
        }
        assert_eq!(bodies[7].to_bytes(), b"body");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mailmap_copies_are_owned() {
        let temp = TempRepo::new("commit-mailmap");
        let repo = repository(&temp);
        let tree = Tree::lookup(
            repo,
            Oid::from_string(EMPTY_TREE, OidType::Sha1)
                .unwrap()
                .as_ref(),
        );
        let tree = tree.unwrap();
        let sig = SignatureView::new(c"Old", c"old@x", Time::new(7, 90));
        let id = create(
            repo,
            None,
            sig.as_ref(),
            sig.as_ref(),
            None,
            c"m\n",
            tree.as_ref(),
            &[],
        )
        .unwrap();
        let commit = Commit::lookup(repo, id.as_ref()).unwrap();
        let text = b"New Name <new@x> <old@x>\n".to_vec().into_boxed_slice();
        let mailmap = Mailmap::from_buffer(&text).unwrap();
        drop(text);
        let mut copies = Vec::new();
        for i in 0..6 {
            let map = (i % 3 != 0).then(|| mailmap.as_ref());
            copies.push(if i % 2 == 0 {
                commit.as_ref().author_with_mailmap(map).unwrap()
            } else {
                commit.as_ref().committer_with_mailmap(map).unwrap()
            });
        }
        drop(mailmap);
        drop(commit);
        for (i, copy) in copies.iter().enumerate() {
            let expected: &[u8] = if i % 3 != 0 { b"New Name" } else { b"Old" };
            assert_eq!(copy.as_ref().name().unwrap().to_bytes(), expected);
            assert_eq!(copy.as_ref().when().offset_minutes(), 90);
        }
        let clones: Vec<_> = copies.iter().map(Clone::clone).collect();
        drop(copies);
        assert_eq!(clones.len(), 6);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn threads_read_commits_of_their_own_repositories() {
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let temp = TempRepo::new(&format!("commit-read-thread-{t}"));
                    let repo = repository(&temp);
                    let tree = Tree::lookup(
                        repo,
                        Oid::from_string(EMPTY_TREE, OidType::Sha1)
                            .unwrap()
                            .as_ref(),
                    )
                    .unwrap();
                    let sig = SignatureView::new(c"T", c"t@x", Time::new(t as i64, 0));
                    for i in 0..4 {
                        let message = CString::new(format!("s\n\nbody {t} {i}\n")).unwrap();
                        let id = create(
                            repo,
                            None,
                            sig.as_ref(),
                            sig.as_ref(),
                            None,
                            &message,
                            tree.as_ref(),
                            &[],
                        )
                        .unwrap();
                        let commit = Commit::lookup(repo, id.as_ref()).unwrap();
                        assert_eq!(commit.as_ref().author().when().seconds(), t as i64);
                        let body = format!("body {t} {i}");
                        assert_eq!(commit.as_ref().body().unwrap().to_bytes(), body.as_bytes());
                        assert!(commit.as_ref().author_with_mailmap(None).is_ok());
                    }
                });
            }
        });
    }
}

/// UB tests for writing commits: `create`, `create_from_array`, `create_buffer`,
/// `create_ext` and `CommitRef::amend`.
///
/// libgit2 reads every input for the call only: the message, the encoding and the
/// reference name are exact-length heap strings, and the signatures are views over
/// heap strings, all freed right after the call (ASan: a read past one; BSan: a pointer
/// kept); the parents' pointers are passed as a run of exactly their number (ASan), or
/// as that many variadic arguments, up to the most `create_from_array` takes. Every
/// buffer `create_buffer` fills is released once (LSan, ASan). A signature whose offset
/// is `i32::MIN` minutes is refused before libgit2 would negate it (UBSan: a signed
/// overflow), and a tree or parent of another repository before libgit2 would assert or
/// drop it. Commits are neither `Send` nor `Sync`: each thread writes into its own
/// repository at once, without moving a reference (TSan; reference writes reach glibc's
/// timezone state, which TSan cannot see the lock of). Every test calls libgit2, so Miri
/// skips them all.
mod creating {
    use std::ffi::{CStr, CString};
    use std::thread;

    use libgit2::libgit2::core::types::{SignatureView, Time};
    use libgit2::libgit2::object::commit::{
        Commit, CommitArc, CommitRef, CreateExtOptions, Header, MAX_ARRAY_PARENTS, create,
        create_buffer, create_ext, create_from_array,
    };
    use libgit2::libgit2::object::tree::Tree;
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::util::errors::ErrorClass;

    use super::reading::EMPTY_TREE;
    use crate::fixture::object_and_tree_entry::repository;
    use crate::fixture::{TempRepo, isolated};

    fn heap(s: &str) -> Box<CStr> {
        CString::new(s).unwrap().into_boxed_c_str()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn inputs_are_read_for_the_call_only() {
        let temp = TempRepo::new("commit-create-inputs");
        let repo = repository(&temp);
        let tree = Tree::lookup(
            repo,
            Oid::from_string(EMPTY_TREE, OidType::Sha1)
                .unwrap()
                .as_ref(),
        );
        let tree = tree.unwrap();
        let mut parents = Vec::new();
        for len in [0usize, 1, 15, 16, 1000] {
            let (name, email) = (heap(&"n".repeat(len + 1)), heap(&"e".repeat(len + 1)));
            let sig = SignatureView::new(&name, &email, Time::new(1 << 40, 14 * 60 + 59));
            let message = heap(&"m".repeat(len));
            let encoding = heap(&"E".repeat(len + 1));
            let refs: Vec<CommitRef<'_, '_>> =
                parents.iter().map(|c: &CommitArc<'_>| c.as_ref()).collect();
            let buf = create_buffer(
                repo,
                sig.as_ref(),
                sig.as_ref(),
                Some(&encoding),
                &message,
                tree.as_ref(),
                &refs,
            )
            .unwrap();
            let id = create(
                repo,
                None,
                sig.as_ref(),
                sig.as_ref(),
                Some(&encoding),
                &message,
                tree.as_ref(),
                &refs,
            )
            .unwrap();
            drop(refs);
            drop((name, email, message, encoding));
            assert!(buf.as_ref().len() > len);
            parents.push(Commit::lookup(repo, id.as_ref()).unwrap());
            drop(buf);
        }
        // The variadic form, with every count it takes.
        let sig = SignatureView::new(c"A", c"a@x", Time::new(0, 0));
        let p = parents[0].as_ref();
        let one = create_from_array(
            repo,
            None,
            sig.as_ref(),
            sig.as_ref(),
            None,
            c"1\n",
            tree.as_ref(),
            [p],
        )
        .unwrap();
        let all = create_from_array(
            repo,
            None,
            sig.as_ref(),
            sig.as_ref(),
            None,
            c"16\n",
            tree.as_ref(),
            [p; MAX_ARRAY_PARENTS],
        )
        .unwrap();
        let none = create_from_array(
            repo,
            None,
            sig.as_ref(),
            sig.as_ref(),
            None,
            c"0\n",
            tree.as_ref(),
            [],
        )
        .unwrap();
        for id in [one, all, none] {
            assert!(Commit::lookup(repo, id.as_ref()).is_ok());
        }
    }

    /// The options' strings and headers live on the heap, freed right after the call;
    /// amending reads the commit's own parts when none is given.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn options_and_amends_read_for_the_call_only() {
        let temp = TempRepo::new("commit-create-ext-amend");
        let repo = repository(&temp);
        let tree = Tree::lookup(
            repo,
            Oid::from_string(EMPTY_TREE, OidType::Sha1)
                .unwrap()
                .as_ref(),
        );
        let tree = tree.unwrap();
        let sig = SignatureView::new(c"A", c"a@x", Time::new(5, -5));
        let (field, value, encoding) = (heap("x-field"), heap("v\nw"), heap("UTF-16"));
        let headers = vec![Header::new(&field, &value); 3];
        let mut options = CreateExtOptions::new();
        options.as_mut().set_extra_headers(&headers);
        options.as_mut().set_message_encoding(Some(&encoding));
        let message = heap("ext\n\nbody\n");
        let id = create_ext(
            repo,
            sig.as_ref(),
            sig.as_ref(),
            &message,
            tree.as_ref(),
            &[],
            Some(options.as_mut()),
        )
        .unwrap();
        drop(headers);
        drop((field, value, encoding, message));
        let commit = Commit::lookup(repo, id.as_ref()).unwrap();
        assert_eq!(commit.as_ref().body().unwrap().to_bytes(), b"body");
        let other = SignatureView::new(&heap("B"), &heap("b@x"), Time::new(6, 6))
            .as_ref()
            .to_signature()
            .unwrap();
        let amended = commit
            .as_ref()
            .amend(
                None,
                None,
                Some(other.as_ref()),
                None,
                Some(&heap("amended\n")),
                None,
            )
            .unwrap();
        drop(other);
        let amended = Commit::lookup(repo, amended.as_ref()).unwrap();
        assert_eq!(amended.as_ref().author(), commit.as_ref().author());
        assert_eq!(amended.as_ref().committer().name(), Some(c"B"));
        // The extra headers are not carried over (libgit2 amends with the encoding
        // only); everything else is the commit's own.
        let copy = commit
            .as_ref()
            .amend(None, None, None, None, None, Some(tree.as_ref()))
            .unwrap();
        assert_ne!(copy, id);
        let copy = Commit::lookup(repo, copy.as_ref()).unwrap();
        assert_eq!(copy.as_ref().body(), commit.as_ref().body());
        assert_eq!(copy.as_ref().committer(), commit.as_ref().committer());
        let plain = create(
            repo,
            None,
            sig.as_ref(),
            sig.as_ref(),
            None,
            c"p\n",
            tree.as_ref(),
            &[],
        )
        .unwrap();
        let plain_commit = Commit::lookup(repo, plain.as_ref()).unwrap();
        let same = plain_commit
            .as_ref()
            .amend(None, None, None, None, None, Some(tree.as_ref()));
        assert_eq!(same.unwrap(), plain);
    }

    /// Inputs libgit2 cannot take are refused before it runs: an offset it would negate
    /// out of range (UBSan), a tree or parent of another repository.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn unwritable_inputs_are_refused_before_c() {
        let (a, b) = (
            TempRepo::new("commit-refuse-a"),
            TempRepo::new("commit-refuse-b"),
        );
        let (ra, rb) = (repository(&a), repository(&b));
        let empty = Oid::from_string(EMPTY_TREE, OidType::Sha1).unwrap();
        let (ta, tb) = (
            Tree::lookup(ra, empty.as_ref()).unwrap(),
            Tree::lookup(rb, empty.as_ref()).unwrap(),
        );
        let good = SignatureView::new(c"A", c"a@x", Time::new(0, 0));
        let min = SignatureView::new(c"A", c"a@x", Time::new(0, i32::MIN));
        let (g, m) = (good.as_ref(), min.as_ref());
        let pb = create(rb, None, g, g, None, c"b\n", tb.as_ref(), &[]).unwrap();
        let pb = Commit::lookup(rb, pb.as_ref()).unwrap();
        for (author, committer) in [(m, g), (g, m), (m, m)] {
            let e =
                create(ra, None, author, committer, None, c"x\n", ta.as_ref(), &[]).unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
            let e =
                create_buffer(ra, author, committer, None, c"x\n", ta.as_ref(), &[]).unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
            let e = create_from_array(ra, None, author, committer, None, c"x\n", ta.as_ref(), [])
                .unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
            let e = create_ext(ra, author, committer, c"x\n", ta.as_ref(), &[], None).unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
            let e = pb
                .as_ref()
                .amend(None, Some(author), Some(committer), None, None, None)
                .unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
        }
        let e = create(ra, None, g, g, None, c"x\n", tb.as_ref(), &[]).unwrap_err();
        assert_eq!(
            e.message(),
            c"invalid argument: 'git_tree_owner(tree) == repo'"
        );
        let e = create(ra, None, g, g, None, c"x\n", ta.as_ref(), &[pb.as_ref()]).unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        let e = create_from_array(ra, None, g, g, None, c"x\n", ta.as_ref(), [pb.as_ref()])
            .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn threads_write_into_their_own_repositories() {
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let temp = TempRepo::new(&format!("commit-create-thread-{t}"));
                    let repo = repository(&temp);
                    let tree = Tree::lookup(
                        repo,
                        Oid::from_string(EMPTY_TREE, OidType::Sha1)
                            .unwrap()
                            .as_ref(),
                    )
                    .unwrap();
                    let sig = SignatureView::new(c"T", c"t@x", Time::new(t as i64, 0));
                    let mut tip = None;
                    for i in 0..4 {
                        let message = CString::new(format!("{t} {i}\n")).unwrap();
                        let parents: Vec<_> =
                            tip.iter().map(|c: &CommitArc<'_>| c.as_ref()).collect();
                        let id = create(
                            repo,
                            None,
                            sig.as_ref(),
                            sig.as_ref(),
                            None,
                            &message,
                            tree.as_ref(),
                            &parents,
                        )
                        .unwrap();
                        let buf = create_buffer(
                            repo,
                            sig.as_ref(),
                            sig.as_ref(),
                            None,
                            &message,
                            tree.as_ref(),
                            &parents,
                        )
                        .unwrap();
                        assert!(!buf.as_ref().is_empty());
                        tip = Some(Commit::lookup(repo, id.as_ref()).unwrap());
                    }
                });
            }
        });
    }

    /// Moving references, the one path of these that writes them, alone in a child.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn references_are_moved_from_heap_names() {
        isolated(
            concat!(module_path!(), "::references_are_moved_from_heap_names"),
            references_are_moved_from_heap_names_body,
        );
    }

    fn references_are_moved_from_heap_names_body() {
        let temp = TempRepo::new("commit-create-refs");
        let repo = repository(&temp);
        let tree = Tree::lookup(
            repo,
            Oid::from_string(EMPTY_TREE, OidType::Sha1)
                .unwrap()
                .as_ref(),
        );
        let tree = tree.unwrap();
        let sig = SignatureView::new(c"A", c"a@x", Time::new(0, 0));
        let name = heap("refs/heads/topic");
        let root = create(
            repo,
            Some(&name),
            sig.as_ref(),
            sig.as_ref(),
            None,
            c"r\n",
            tree.as_ref(),
            &[],
        )
        .unwrap();
        drop(name);
        let root = Commit::lookup(repo, root.as_ref()).unwrap();
        let name = heap("refs/heads/topic");
        let child = create_from_array(
            repo,
            Some(&name),
            sig.as_ref(),
            sig.as_ref(),
            None,
            c"c\n",
            tree.as_ref(),
            [root.as_ref()],
        )
        .unwrap();
        let child = Commit::lookup(repo, child.as_ref()).unwrap();
        let amended = child
            .as_ref()
            .amend(Some(&name), None, None, None, Some(c"c2\n"), None)
            .unwrap();
        drop(name);
        let reference = std::fs::read_to_string(temp.path.join("refs/heads/topic")).unwrap();
        assert_eq!(reference.trim(), amended.as_ref().to_hex_string().unwrap());
    }
}

/// UB tests for the commit creation callback, `CreateCallback`, through its public
/// [`call`](CreateCallback::call), which runs the `extern "C"` trampoline libgit2 calls.
///
/// The trampoline lends the closure handles over what it is given, for the call only:
/// the closure reads every string and parent whole (ASan), and its own state is written
/// between calls (BSan: nothing keeps a pointer into it); the strings are heap ones,
/// freed after the call. An id it returns is copied out whole (Miri cannot run it: the
/// trampoline reads the tree's repository through libgit2). A panic resumes from `call`
/// without crossing the trampoline, and leaves the callback usable. Handles outliving
/// the call and a callback crossing threads are doctests; the callback under libgit2's
/// own caller, `git_rebase_commit`, is unit-tested beside it.
mod create_callback {
    use std::cell::RefCell;
    use std::ffi::{CStr, CString};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use libgit2::libgit2::core::types::{SignatureRef, SignatureView, StreamError, Time};
    use libgit2::libgit2::object::commit::{Commit, CommitRef, CreateCallback, create};
    use libgit2::libgit2::object::tree::{Tree, TreeRef};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::util::errors::ErrorClass;

    use super::reading::EMPTY_TREE;
    use crate::fixture::TempRepo;
    use crate::fixture::object_and_tree_entry::repository;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn arguments_are_lent_for_the_call() {
        let temp = TempRepo::new("commit-create-cb");
        let repo = repository(&temp);
        let tree = Tree::lookup(
            repo,
            Oid::from_string(EMPTY_TREE, OidType::Sha1)
                .unwrap()
                .as_ref(),
        );
        let tree = tree.unwrap();
        let sig = SignatureView::new(c"A", c"a@x", Time::new(0, 0));
        let mut commits = Vec::new();
        for i in 0..5 {
            let message = CString::new(format!("{i}\n")).unwrap();
            let id = create(
                repo,
                None,
                sig.as_ref(),
                sig.as_ref(),
                None,
                &message,
                tree.as_ref(),
                &[],
            )
            .unwrap();
            commits.push(Commit::lookup(repo, id.as_ref()).unwrap());
        }
        let seen = RefCell::new(Vec::new());
        let mut f = |author: SignatureRef<'_>,
                     committer: SignatureRef<'_>,
                     encoding: Option<&CStr>,
                     message: &CStr,
                     tree: TreeRef<'_, '_>,
                     parents: &[CommitRef<'_, '_>]| {
            let mut s = seen.borrow_mut();
            s.push((
                author.name().unwrap().to_bytes().to_vec(),
                committer.email().unwrap().to_bytes().to_vec(),
                encoding.map(|e| e.to_bytes().to_vec()),
                message.to_bytes().to_vec(),
                tree.id().to_hex_string().unwrap(),
                parents.iter().filter(|p| p.body().is_none()).count(),
            ));
            match s.len() % 3 {
                0 => Ok(None),
                1 => Ok(Some(parents.first().map_or_else(
                    || Oid::zero(OidType::Sha1),
                    |p| p.id().to_oid(),
                ))),
                _ => Err(StreamError::new(ErrorClass::Callback, c"no")),
            }
        };
        let mut callback = CreateCallback::new(&mut f);
        for n in 0..=commits.len() {
            let (name, email) = (
                CString::new("N".repeat(n + 1)).unwrap().into_boxed_c_str(),
                CString::new("e".repeat(n + 1)).unwrap().into_boxed_c_str(),
            );
            let author = SignatureView::new(&name, &email, Time::new(n as i64, 0));
            let message = CString::new("m".repeat(n * 100))
                .unwrap()
                .into_boxed_c_str();
            let encoding = (n % 2 == 0).then(|| CString::new("E".repeat(n)).unwrap());
            let parents: Vec<_> = commits[..n].iter().map(|c| c.as_ref()).collect();
            let result = callback.call(
                author.as_ref(),
                author.as_ref(),
                encoding.as_deref(),
                &message,
                tree.as_ref(),
                &parents,
            );
            drop((name, email, message, encoding));
            match (n + 1) % 3 {
                0 => assert!(result.unwrap().is_none()),
                1 => {
                    let id = result.unwrap().unwrap();
                    assert_eq!(u32::from(id.as_ref().raw_kind()), OidType::Sha1.raw());
                }
                _ => assert_eq!(result.unwrap_err().message(), c"no"),
            }
        }
        drop(callback);
        let seen = seen.into_inner();
        assert_eq!(seen.len(), commits.len() + 1);
        for (n, s) in seen.iter().enumerate() {
            assert_eq!(s.0.len(), n + 1);
            assert_eq!(s.3.len(), n * 100);
            assert_eq!(s.5, n);
            assert_eq!(s.4, EMPTY_TREE.to_str().unwrap());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn panics_resume_and_leave_the_callback_usable() {
        let temp = TempRepo::new("commit-create-cb-panic");
        let repo = repository(&temp);
        let tree = Tree::lookup(
            repo,
            Oid::from_string(EMPTY_TREE, OidType::Sha1)
                .unwrap()
                .as_ref(),
        );
        let tree = tree.unwrap();
        let sig = SignatureView::new(c"A", c"a@x", Time::new(0, 0));
        let mut calls = 0;
        let mut f = |_: SignatureRef<'_>,
                     _: SignatureRef<'_>,
                     _: Option<&CStr>,
                     _: &CStr,
                     _: TreeRef<'_, '_>,
                     _: &[CommitRef<'_, '_>]| {
            calls += 1;
            if calls == 2 {
                panic!("callback {calls}");
            }
            Ok::<_, StreamError>(None)
        };
        let mut callback = CreateCallback::new(&mut f);
        let call = |cb: &mut CreateCallback<'_>| {
            cb.call(sig.as_ref(), sig.as_ref(), None, c"m", tree.as_ref(), &[])
        };
        assert!(call(&mut callback).unwrap().is_none());
        let caught = catch_unwind(AssertUnwindSafe(|| call(&mut callback)));
        assert_eq!(
            caught
                .unwrap_err()
                .downcast_ref::<String>()
                .map(String::as_str),
            Some("callback 2")
        );
        assert!(call(&mut callback).unwrap().is_none());
        assert!(format!("{callback:?}").starts_with("CreateCallback { panicked: false"));
        drop(callback);
        assert_eq!(calls, 3);
    }
}
