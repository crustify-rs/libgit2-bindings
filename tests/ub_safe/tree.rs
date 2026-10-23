//! UB tests for `git2/tree.h`: the enum `WalkMode` (`git_treewalk_mode`, unsigned).
//!
//! `WalkMode` is a fieldless enum over C's `git_treewalk_mode`. The only safe way in
//! from a raw value is `from_raw` / `try_from`, which must reject every value without
//! a variant rather than produce an invalid discriminant (which Miri and UBSan would
//! flag on the next read); an `as` cast to it does not compile (`compile_fail/`). It
//! owns and points at nothing and none of these tests calls libgit2, so they also run
//! under Miri; the sweep strides under BSan and Miri.
//!
//! The tree object's tests are in [`tree_object`], the tree builder's in [`builder`], its
//! routines' and the walk callback's in [`routines`], the tree entry's in [`tree_entry`],
//! the tree update's in [`tree_update`], the tree's getters, walk and the builder's
//! filter in [`tree_reads`].

use std::collections::HashSet;
use std::hint::black_box;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::object::tree::{UnnamedWalkMode, WalkMode};

/// Every small value, a stride across the rest of `git_treewalk_mode`, and its ends.
fn probes() -> impl Iterator<Item = ffi::git_treewalk_mode> {
    let stride = if cfg!(bsan) || cfg!(miri) { 97 } else { 1 };
    (0..=4096)
        .step_by(stride)
        .chain([0, 1, 2])
        .chain((0..=ffi::git_treewalk_mode::MAX).step_by(1 << 24))
        .chain([ffi::git_treewalk_mode::MAX - 1, ffi::git_treewalk_mode::MAX])
}

#[test]
fn from_raw_yields_only_valid_modes() {
    let mut seen = HashSet::new();
    for raw in probes() {
        match WalkMode::from_raw(black_box(raw)) {
            Some(mode) => {
                // Matching on the value reads its discriminant.
                let back = match black_box(mode) {
                    WalkMode::PreOrder => ffi::GIT_TREEWALK_PRE,
                    WalkMode::PostOrder => ffi::GIT_TREEWALK_POST,
                    _ => unreachable!(),
                };
                assert_eq!(back, raw);
                assert_eq!(WalkMode::try_from(raw), Ok(mode));
                assert!(!format!("{mode:?}").is_empty());
                seen.insert(mode);
            }
            None => {
                let err = WalkMode::try_from(raw).unwrap_err();
                assert_eq!(err.raw(), raw);
                assert!(!err.to_string().is_empty());
            }
        }
    }
    assert_eq!(seen.len(), 2);
}

#[test]
fn option_niche_holds() {
    for raw in probes() {
        let mode = black_box(WalkMode::from_raw(raw));
        assert_eq!(mode.map(WalkMode::raw).unwrap_or(raw), raw);
        let result = black_box(WalkMode::try_from(raw));
        assert_eq!(
            result
                .map(WalkMode::raw)
                .unwrap_or_else(UnnamedWalkMode::raw),
            raw
        );
    }
}

#[test]
fn conversions_agree_across_threads() {
    thread::scope(|s| {
        let handles: Vec<_> = [WalkMode::PreOrder, WalkMode::PostOrder]
            .into_iter()
            .chain((0..4).filter_map(WalkMode::from_raw))
            .map(|mode| s.spawn(move || (mode, WalkMode::from_raw(mode.raw()))))
            .collect();
        for h in handles {
            let (mode, back) = h.join().unwrap();
            assert_eq!(back, Some(mode));
        }
        let unnamed = s.spawn(|| WalkMode::try_from(2).unwrap_err());
        assert_eq!(unnamed.join().unwrap().raw(), 2);
    });
}

mod builder {
    //! UB tests for `git2/tree.h`: the `TreeBuilder` owned by its `TreeBuilderBox`, its
    //! handles, `clear`, and the `git_treebuilder_free` policy; and `TreeBuilder::from_tree`
    //! on every shape of tree object: the empty tree, flat trees, a tree holding a
    //! subtree, a tree of the builder's own repository or of another, and a SHA256
    //! repository's tree seeding a SHA1 repository's builder; an object that is not a
    //! tree has no tree handle to seed one with. The builder keeps nothing of the tree, so
    //! dropping the tree right after the call is fine (ASan).
    //!
    //! Every entry a builder holds is its own allocation, which `clear` and the drop must
    //! each free exactly once, however the builder was filled (ASan: double free, use
    //! after free; LSan: leak). A builder keeps a bare pointer to its repository, which
    //! libgit2 follows while inserting and writing, across the Rust reborrows of that
    //! repository made in between (BSan: a retained foreign pointer). Builders are not
    //! `Send`, so the threaded test gives each thread its own repository (TSan: the
    //! runtime pin and allocator under concurrent constructors). Nothing here runs under
    //! Miri, which cannot call libgit2. Illegal uses are compile-fail doctests on the crate
    //! (sending or sharing a builder, among others) and files under `compile_fail/`: a
    //! builder outliving its repository or a handle its owner, aliased exclusive handles,
    //! a `clone`.

    use std::ffi::{CString, c_int};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::Barrier;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::FileMode;
    use libgit2::libgit2::object::object::Object;
    use libgit2::libgit2::object::tree::{
        Tree, TreeArc, TreeBuilder, TreeBuilderBox, TreeBuilderMut, TreeRef,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    use crate::fixture::init;
    use crate::fixture::tree::{count, entries, id, mode_of};

    const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    /// A fresh bare repository in a temporary directory, removed on drop after the
    /// repository it holds.
    struct TempRepo {
        path: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl TempRepo {
        fn new(tag: &str) -> Self {
            init();
            let path =
                std::env::temp_dir().join(format!("crustify-tree-ub-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            let repo = Repository::init(&Self::c_path(&path), true).unwrap();
            Self {
                path,
                repo: Some(repo),
            }
        }

        fn c_path(path: &std::path::Path) -> CString {
            CString::new(path.as_os_str().as_bytes()).unwrap()
        }

        fn repo(&self) -> &RepositoryBox {
            self.repo.as_ref().unwrap()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Inserts `n` submodule entries `e0`.. (ids `01..`, never the refused zero id), which
    /// libgit2 does not check against the object database.
    fn fill(builder: &mut TreeBuilderBox<'_>, n: u8) {
        for i in 0..n {
            let name = CString::new(format!("e{i}")).unwrap();
            let id = id(i + 1);
            builder
                .as_mut()
                .insert(&name, id.as_ref(), FileMode::Commit)
                .unwrap();
        }
    }

    /// The hex id of the tree the builder writes, or the code of its failure.
    fn write(builder: &mut TreeBuilderBox<'_>) -> Result<String, c_int> {
        match builder.as_mut().write() {
            Ok(id) => Ok(id.as_ref().to_hex_string().unwrap()),
            Err(e) => Err(e.raw_code()),
        }
    }

    /// The length follows every insertion, replacement and clear, through the owner and
    /// every handle form, and agrees with C's count.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lengths_follow_the_entries() {
        let repo = TempRepo::new("lengths");
        let mut builder = TreeBuilder::new(repo.repo().as_ref()).unwrap();
        assert!(builder.as_ref().is_empty());
        for n in [1u8, 7, 64, 200] {
            fill(&mut builder, n);
            assert_eq!(builder.as_ref().len(), usize::from(n));
            assert_eq!(builder.as_mut().as_ref().len(), count(&builder));
            fill(&mut builder, n);
            assert_eq!(builder.as_ref().len(), usize::from(n));
            let mut handle: TreeBuilderMut<'_, '_> = builder.as_mut();
            handle.as_mut().clear();
            assert!(handle.as_ref().is_empty());
        }
    }

    /// Empty builders are built, cleared through every handle form and released, many
    /// over one repository, in any order.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn empty_builders_build_clear_and_release() {
        init();
        let repo = Repository::new().unwrap();
        let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
        builder.as_mut().clear();
        {
            let mut m = builder.as_mut();
            m.clear();
            let mut again: TreeBuilderMut<'_, '_> = m.as_mut();
            again.clear();
            assert_eq!(format!("{:?}", again.as_ref()), "TreeBuilder { .. }");
            m.clear();
            assert_eq!(format!("{m:?}"), "TreeBuilder { .. }");
        }
        let r = builder.as_ref();
        let copy = r;
        assert_eq!(format!("{copy:?}"), format!("{r:?}"));
        assert_eq!(count(&builder), 0);
        drop(builder);

        let mut builders: Vec<_> = (0..8)
            .map(|_| TreeBuilder::new(repo.as_ref()).unwrap())
            .collect();
        builders.swap(1, 6);
        while let Some(b) = builders.pop() {
            drop(b);
            if let Some(first) = builders.first_mut() {
                first.as_mut().clear();
            }
        }
    }

    /// Every entry is freed once: replaced in place, cleared, refilled, and released with
    /// the builder, entries still held.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn builders_free_every_entry() {
        let temp = TempRepo::new("entries");
        let mut builder = TreeBuilder::new(temp.repo().as_ref()).unwrap();
        fill(&mut builder, 16);
        assert_eq!(count(&builder), 16);
        // The same names again replace each entry's id in place.
        fill(&mut builder, 16);
        assert_eq!(count(&builder), 16);
        assert_eq!(mode_of(&builder, c"e3"), Some(ffi::GIT_FILEMODE_COMMIT));

        builder.as_mut().clear();
        assert_eq!(count(&builder), 0);
        assert_eq!(mode_of(&builder, c"e3"), None);
        builder.as_mut().clear();

        fill(&mut builder, 40);
        assert_eq!(count(&builder), 40);
        drop(builder);

        // A builder dropped full, never cleared.
        let mut full = TreeBuilder::new(temp.repo().as_ref()).unwrap();
        fill(&mut full, 3);
    }

    /// A cleared builder writes the empty tree, whatever it held, and stays usable.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn cleared_builders_write_the_empty_tree() {
        let temp = TempRepo::new("clear-write");
        let mut builder = TreeBuilder::new(temp.repo().as_ref()).unwrap();
        assert_eq!(write(&mut builder).as_deref(), Ok(EMPTY_TREE));
        fill(&mut builder, 5);
        let full = write(&mut builder).unwrap();
        assert_ne!(full, EMPTY_TREE);
        builder.as_mut().clear();
        assert_eq!(write(&mut builder).as_deref(), Ok(EMPTY_TREE));
        fill(&mut builder, 5);
        assert_eq!(write(&mut builder), Ok(full));
    }

    /// A builder follows its repository pointer while the repository is reborrowed and
    /// used through Rust between its operations: the odb libgit2 loads lazily through the
    /// builder's write is the one a later Rust borrow reads, and the other way round.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn builders_follow_their_repository_across_reborrows() {
        let temp = TempRepo::new("reborrow");
        let repo = temp.repo();
        let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
        assert!(repo.as_ref().path().is_some());
        fill(&mut builder, 2);
        // The builder's write loads the repository's odb through the pointer it kept.
        let written = write(&mut builder).unwrap();
        let odb = repo.as_ref().odb().unwrap();
        drop(odb);
        builder.as_mut().clear();
        fill(&mut builder, 2);
        assert_eq!(write(&mut builder), Ok(written));
        let second = TreeBuilder::new(repo.as_ref()).unwrap();
        assert!(repo.as_ref().path().is_some());
        drop(builder);
        drop(second);
    }

    /// Builders of two repositories, interleaved, each write into their own.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn builders_of_several_repositories() {
        let a = TempRepo::new("several-a");
        let b = TempRepo::new("several-b");
        let mut in_a = TreeBuilder::new(a.repo().as_ref()).unwrap();
        let mut in_b = TreeBuilder::new(b.repo().as_ref()).unwrap();
        fill(&mut in_a, 3);
        fill(&mut in_b, 3);
        assert_eq!(write(&mut in_a), write(&mut in_b));
        drop(in_a);
        in_b.as_mut().clear();
        assert_eq!(write(&mut in_b).as_deref(), Ok(EMPTY_TREE));
    }

    /// Threads build, fill, write and release builders at once, each over its own
    /// repository opened on one directory: the constructors' runtime pin and libgit2's
    /// allocator race nothing.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn builders_on_threads() {
        const THREADS: usize = 4;
        let temp = TempRepo::new("threads");
        let path = TempRepo::c_path(&temp.path);
        let barrier = Barrier::new(THREADS);
        let ids: Vec<String> = thread::scope(|s| {
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    s.spawn(|| {
                        let repo = Repository::open(&path).unwrap();
                        barrier.wait();
                        let mut written = None;
                        for _ in 0..8 {
                            let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
                            fill(&mut builder, 4);
                            written = Some(write(&mut builder).unwrap());
                            builder.as_mut().clear();
                        }
                        written.unwrap()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(ids.windows(2).all(|w| w[0] == w[1]));
    }

    /// Opens the fixture repository `name` under `tests/resources`, read-only.
    fn open(name: &str) -> RepositoryBox {
        init();
        let path = CString::new(format!(
            "{}/c/tests/resources/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        Repository::open(&path).unwrap()
    }

    /// The tree `hex` names in `repo`.
    fn lookup<'r>(repo: &'r RepositoryBox, hex: &str, kind: OidType) -> TreeArc<'r> {
        let id = Oid::from_string(&CString::new(hex).unwrap(), kind).unwrap();
        Tree::lookup(repo.as_ref(), id.as_ref()).unwrap()
    }

    /// Seeds a builder of `target` with `source`, drops `source` at once, then checks
    /// that the builder holds a copy of each entry.
    fn seeded<'r>(target: &'r RepositoryBox, source: TreeArc<'_>) -> TreeBuilderBox<'r> {
        let expected = entries(&source);
        let builder = TreeBuilder::from_tree(target.as_ref(), source.as_ref()).unwrap();
        drop(source);
        assert_eq!(count(&builder), expected.len());
        for (name, mode) in &expected {
            assert_eq!(mode_of(&builder, name), Some(*mode), "{name:?}");
        }
        builder
    }

    /// `testrepo.git`'s `HEAD` tree: three blobs.
    const FLAT: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
    /// A `testrepo.git` tree holding three blobs and the subtree `ab`.
    const NESTED: &str = "ae90f12eea699729ed24555e40b9fd669da12a12";
    /// `testrepo_256.git`'s `HEAD` tree: three blobs, with SHA256 ids.
    const FLAT_256: &str = "f2a108f86a3b4fd9ad75ed55e9cb3cb46e348fca3b9dba3db64f7c9f64b8a736";

    /// Flat and nested trees, seeding a builder of another repository, are copied whole:
    /// the builder writes the very tree back, after its source is gone (ASan: use after
    /// free of the tree; LSan: a leaked copy).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn trees_seed_builders_of_another_repository() {
        let source = open("testrepo.git");
        let target = TempRepo::new("another");
        for hex in [FLAT, NESTED] {
            let mut builder = seeded(target.repo(), lookup(&source, hex, OidType::Sha1));
            assert_eq!(write(&mut builder).as_deref(), Ok(hex));
            builder.as_mut().clear();
            assert_eq!(count(&builder), 0);
            assert_eq!(write(&mut builder).as_deref(), Ok(EMPTY_TREE));
        }
        // The source repository may go before the builders seeded from it.
        let tree = lookup(&source, NESTED, OidType::Sha1);
        let builder = seeded(target.repo(), tree);
        drop(source);
        assert_eq!(count(&builder), 4);
        assert_eq!(mode_of(&builder, c"ab"), Some(ffi::GIT_FILEMODE_TREE));
    }

    /// A tree of the builder's own repository: the empty tree it wrote, then a tree with
    /// entries, each builder released with its entries still held.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn trees_seed_builders_of_their_own_repository() {
        let temp = TempRepo::new("own");
        let repo = temp.repo();
        let mut empty = TreeBuilder::new(repo.as_ref()).unwrap();
        assert_eq!(write(&mut empty).as_deref(), Ok(EMPTY_TREE));
        drop(empty);

        let builder = seeded(repo, lookup(repo, EMPTY_TREE, OidType::Sha1));
        assert_eq!(count(&builder), 0);
        drop(builder);

        let source = open("testrepo.git");
        let mut builder = seeded(repo, lookup(&source, NESTED, OidType::Sha1));
        // Written into `repo`, the tree is now its own.
        let written = write(&mut builder).unwrap();
        drop(builder);
        let again = seeded(repo, lookup(repo, &written, OidType::Sha1));
        let mut copies: Vec<_> = (0..4)
            .map(|_| seeded(repo, lookup(repo, &written, OidType::Sha1)))
            .collect();
        copies[2].as_mut().clear();
        drop(again);
        drop(copies);
    }

    /// A SHA256 repository's tree seeds a SHA1 repository's builder: the ids are copied as
    /// they are, and writing truncates each to SHA1's 20 bytes, within the id's storage;
    /// the tree written is malformed but nothing is read or written out of bounds.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn trees_of_another_object_id_type_stay_in_bounds() {
        let source = open("testrepo_256.git");
        let target = TempRepo::new("sha256");
        let mut builder = seeded(target.repo(), lookup(&source, FLAT_256, OidType::Sha256));
        let written = write(&mut builder).unwrap();
        assert_ne!(written, FLAT_256);
        assert_eq!(written.len(), 40);
        builder.as_mut().clear();
        assert_eq!(write(&mut builder).as_deref(), Ok(EMPTY_TREE));
    }

    /// An object that is not a tree has no tree handle, so libgit2 never reads one as a
    /// tree (ASan would flag the read of a smaller object): the commit of `NESTED`, a blob
    /// and a tag stay objects, usable. The tree object itself seeds a builder through
    /// either view of it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn only_trees_seed_builders() {
        let source = open("testrepo.git");
        let target = TempRepo::new("not-trees");
        let object = |hex: &str| {
            let id = Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap();
            Object::lookup(source.as_ref(), id.as_ref(), None).unwrap()
        };
        for hex in [
            "763d71aadf09a7951596c9746c024e7eece7c7af",
            "a8233120f6ad708f843d861ce2b7228ec4e3dec6",
            "b25fa35b38051e4ae45d4222e795f9df2e43f1d1",
        ] {
            let object = object(hex);
            assert!(TreeRef::from_object(object.as_ref()).is_none(), "{hex}");
            let object = Tree::from_object(object).unwrap_err();
            let _ = object.as_ref().to_object();
        }
        let tree = object(NESTED);
        let viewed = TreeRef::from_object(tree.as_ref()).unwrap();
        let builder = TreeBuilder::from_tree(target.repo().as_ref(), viewed).unwrap();
        assert_eq!(count(&builder), 4);
        let tree = Tree::from_object(tree).unwrap();
        assert_eq!(count(&seeded(target.repo(), tree)), 4);
    }
}

mod routines {
    //! UB tests for `git2/tree.h`'s tree-builder routines over the safe API: `get`,
    //! `insert` / `insert_entry`, `remove` and `write`, and the walk callback
    //! (`WalkCallback`, `git_treewalk_cb`).
    //!
    //! Each entry is the builder's own allocation, its name inline past the struct: an
    //! entry replaced in place, removed, cleared or dropped with the builder must be freed
    //! exactly once, and an entry lent by `get` or `insert_entry` must be the live one
    //! (ASan: use after free, double free, a name read out of bounds; LSan: a leak). The
    //! tests insert names of every length up to the longest a tree stores, and refused
    //! inserts of every kind, which must leave the builder as it was. Inserting looks the
    //! object up through the repository pointer the builder kept, across Rust reborrows of
    //! that repository (BSan: a retained foreign pointer). An entry is `Sync`: an entry the
    //! builder lends is copied from several threads at once while the builder stays
    //! shared (TSan). The walk callback lends each entry and root for its call only, and
    //! catches a panic of its closure; one closure may run a callback of its own (a
    //! re-entered callback). Lent entries outliving their borrow, or held across a change
    //! of the builder, do not compile (`compile_fail/` and doctests). Every test calls
    //! libgit2, so Miri skips them all.

    use std::ffi::{CStr, CString, c_int};
    use std::os::unix::ffi::OsStrExt;
    use std::panic::{self, AssertUnwindSafe};
    use std::path::PathBuf;
    use std::thread;

    use libgit2::libgit2::core::types::FileMode;
    use libgit2::libgit2::object::object::ObjectArc;
    use libgit2::libgit2::object::tree::{
        Tree, TreeBuilder, TreeBuilderBox, TreeEntry, TreeEntryBox, TreeEntryRef, WalkAction,
        WalkCallback,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
    use libgit2::libgit2::util::errors::{ErrorBuf, ErrorClass};

    use crate::fixture::init;
    use crate::fixture::object_and_tree_entry::{entry_view, object_view, write_blob};
    use crate::fixture::tree::{count, id, mode_of};

    /// A fresh bare repository in a temporary directory, removed on drop after the
    /// repository it holds.
    struct TempRepo {
        path: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl TempRepo {
        fn new(tag: &str) -> Self {
            init();
            let path =
                std::env::temp_dir().join(format!("crustify-tree-ub-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            let repo = Repository::init(&Self::c_path_of(&path), true).unwrap();
            Self {
                path,
                repo: Some(repo),
            }
        }

        fn c_path_of(path: &std::path::Path) -> CString {
            CString::new(path.as_os_str().as_bytes()).unwrap()
        }

        fn c_path(&self) -> CString {
            Self::c_path_of(&self.path)
        }

        fn repo(&self) -> &RepositoryBox {
            self.repo.as_ref().unwrap()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// The id of an object an owner holds.
    fn id_of(object: &ObjectArc<'_>) -> Oid {
        let (hex, _) = object_view(object);
        Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap()
    }

    /// C's name, hex id and mode of a lent entry, read from a copy.
    fn view(entry: TreeEntryRef<'_>) -> (CString, String, u32) {
        entry_view(&entry.to_tree_entry().unwrap())
    }

    fn hex(id: &Oid) -> String {
        id.as_ref().to_hex_string().unwrap()
    }

    /// A blob and the empty tree of `temp`, written through it.
    fn objects(temp: &TempRepo) -> (Oid, Oid) {
        let blob = id_of(&write_blob(temp.repo(), b"content\n"));
        let tree = TreeBuilder::new(temp.repo().as_ref())
            .unwrap()
            .as_mut()
            .write()
            .unwrap();
        (blob, tree)
    }

    /// Entries are inserted, looked up, replaced in place, removed and cleared, over
    /// names of every length a tree stores; each step frees what it drops exactly once,
    /// and every lent entry is the live one.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn entries_are_inserted_replaced_and_removed() {
        let temp = TempRepo::new("tree-routines");
        let (blob, empty) = objects(&temp);
        let mut builder = TreeBuilder::new(temp.repo().as_ref()).unwrap();
        let lengths: Vec<usize> = if cfg!(bsan) {
            vec![1, 2, 255, 256, 65535]
        } else {
            (1..=300).chain([4095, 4096, 65534, 65535]).collect()
        };
        let names: Vec<CString> = lengths
            .iter()
            .map(|&n| CString::new(vec![b'f'; n]).unwrap())
            .collect();
        for name in &names {
            let entry = builder
                .as_mut()
                .insert_entry(name, blob.as_ref(), FileMode::Blob)
                .map(view)
                .unwrap();
            assert_eq!(entry, (name.clone(), hex(&blob), ffi_mode(FileMode::Blob)));
        }
        assert_eq!(count(&builder), names.len());
        // Every name again, as a tree and then an executable: replaced in place.
        for name in &names {
            builder
                .as_mut()
                .insert(name, empty.as_ref(), FileMode::Tree)
                .unwrap();
            let mut m = builder.as_mut();
            let entry = m
                .insert_entry(name, blob.as_ref(), FileMode::BlobExecutable)
                .unwrap();
            assert_eq!(view(entry).2, ffi_mode(FileMode::BlobExecutable));
        }
        assert_eq!(count(&builder), names.len());
        let shared = builder.as_ref();
        let lent: Vec<_> = names.iter().map(|n| shared.get(n).unwrap()).collect();
        for (entry, name) in lent.iter().zip(&names) {
            assert_eq!(view(*entry).0, *name);
        }
        assert!(shared.get(c"absent").is_none());
        let written = builder.as_mut().write().unwrap();

        // Every other entry removed; removing it again fails, naming it.
        for name in names.iter().step_by(2) {
            builder.as_mut().remove(name).unwrap();
            let e = builder.as_mut().remove(name).unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Tree));
            assert!(
                e.message()
                    .to_bytes()
                    .starts_with(b"failed to remove entry: file isn't in the tree - f")
            );
            assert!(builder.as_ref().get(name).is_none());
        }
        assert_eq!(count(&builder), names.len() / 2);
        assert_ne!(builder.as_mut().write().unwrap(), written);
        // Put back, the same tree is written.
        for name in names.iter().step_by(2) {
            builder
                .as_mut()
                .insert(name, blob.as_ref(), FileMode::BlobExecutable)
                .unwrap();
        }
        assert_eq!(builder.as_mut().write().unwrap(), written);
        builder.as_mut().clear();
        assert!(builder.as_mut().remove(&names[0]).is_err());
        builder
            .as_mut()
            .insert(&names[0], blob.as_ref(), FileMode::Link)
            .unwrap();
        assert_eq!(mode_of(&builder, &names[0]), Some(ffi_mode(FileMode::Link)));
        // Dropped with an entry held.
    }

    fn ffi_mode(mode: FileMode) -> u32 {
        mode.raw()
    }

    /// Every insert libgit2 refuses, and the over-long names Rust refuses, fail with
    /// their error and leave the builder as it was: no entry added or changed, nothing
    /// leaked.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn refused_inserts_leave_the_builder_unchanged() {
        let temp = TempRepo::new("tree-refused");
        let (blob, empty) = objects(&temp);
        let mut builder = TreeBuilder::new(temp.repo().as_ref()).unwrap();
        builder
            .as_mut()
            .insert(c"kept", blob.as_ref(), FileMode::Blob)
            .unwrap();
        let missing = id(0x42);
        let zero = Oid::zero(OidType::Sha1);
        let long = CString::new(vec![b'l'; 65536]).unwrap();
        let cases: [(&CStr, &Oid, FileMode, ErrorClass); 12] = [
            (c"", &blob, FileMode::Blob, ErrorClass::Tree),
            (c".", &blob, FileMode::Blob, ErrorClass::Tree),
            (c"..", &blob, FileMode::Blob, ErrorClass::Tree),
            (c".git", &blob, FileMode::Blob, ErrorClass::Tree),
            (c"a/b", &blob, FileMode::Blob, ErrorClass::Tree),
            (c"/", &blob, FileMode::Blob, ErrorClass::Tree),
            (c"x", &blob, FileMode::Unreadable, ErrorClass::Tree),
            (c"x", &zero, FileMode::Commit, ErrorClass::Tree),
            (c"x", &missing, FileMode::Blob, ErrorClass::Tree),
            (c"x", &blob, FileMode::Tree, ErrorClass::Tree),
            (c"kept", &empty, FileMode::Blob, ErrorClass::Tree),
            (&long, &blob, FileMode::Blob, ErrorClass::Invalid),
        ];
        for (name, id, mode, class) in cases {
            for e in [
                builder.as_mut().insert(name, id.as_ref(), mode),
                builder
                    .as_mut()
                    .insert_entry(name, id.as_ref(), mode)
                    .map(|_| ()),
            ] {
                let e: ErrorBuf = e.unwrap_err();
                assert_eq!(e.class(), Some(class), "{name:?} {mode:?}: {e:?}");
            }
            assert_eq!(count(&builder), 1);
            assert_eq!(mode_of(&builder, c"kept"), Some(ffi_mode(FileMode::Blob)));
        }
        assert!(builder.as_ref().get(c"x").is_none());
    }

    /// An id of another type than the repository's never reaches libgit2's object lookup,
    /// whose loose backend formats it into a path sized for the repository's type: a
    /// SHA-256 id in a SHA-1 repository wrote 24 bytes past that path's buffer (ASan: a
    /// heap buffer overflow, with the short objects path of a temporary directory), and
    /// a SHA-1 id in a SHA-256 repository left part of it unwritten. Both insert forms
    /// refuse it for every mode libgit2 looks up, leaving the builder as it was; a
    /// submodule's commit, never looked up, and the mode and zero-id refusals, which
    /// libgit2 makes first, keep libgit2's answers.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn ids_of_another_type_are_refused_before_the_lookup() {
        let temp = TempRepo::new("tree-wide");
        let (blob, _) = objects(&temp);
        let mut builder = TreeBuilder::new(temp.repo().as_ref()).unwrap();
        builder
            .as_mut()
            .insert(c"kept", blob.as_ref(), FileMode::Blob)
            .unwrap();
        let wide = Oid::from_raw(&[0xAB; 32], OidType::Sha256).unwrap();
        let looked_up = [
            FileMode::Blob,
            FileMode::BlobExecutable,
            FileMode::Link,
            FileMode::Tree,
        ];
        for mode in looked_up {
            for name in [c"x", c"kept"] {
                for e in [
                    builder.as_mut().insert(name, wide.as_ref(), mode),
                    builder
                        .as_mut()
                        .insert_entry(name, wide.as_ref(), mode)
                        .map(|_| ()),
                ] {
                    let e: ErrorBuf = e.unwrap_err();
                    assert_eq!(e.class(), Some(ErrorClass::Invalid), "{mode:?}: {e:?}");
                    assert_eq!(
                        e.message(),
                        c"specified object ID type does not match repository object ID type"
                    );
                }
            }
            assert_eq!(count(&builder), 1);
            assert_eq!(mode_of(&builder, c"kept"), Some(ffi_mode(FileMode::Blob)));
        }
        // libgit2's earlier refusals keep their own errors.
        let e = builder
            .as_mut()
            .insert(c"x", wide.as_ref(), FileMode::Unreadable)
            .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Tree));
        let zero = Oid::zero(OidType::Sha256);
        let e = builder
            .as_mut()
            .insert(c"x", zero.as_ref(), FileMode::Blob)
            .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Tree));
        // A submodule's commit is stored unchecked, as libgit2 stores it.
        builder
            .as_mut()
            .insert(c"sub", wide.as_ref(), FileMode::Commit)
            .unwrap();
        assert_eq!(count(&builder), 2);
        drop(builder);

        // The converse, through a SHA-256 repository's builder.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/c/tests/resources/testrepo_256.git"
        );
        let repo = Repository::open(&CString::new(path).unwrap()).unwrap();
        let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
        let e = builder
            .as_mut()
            .insert(c"x", blob.as_ref(), FileMode::Blob)
            .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        assert_eq!(count(&builder), 0);
    }

    /// Entries lent by `get` and `insert_entry` copy out into entries of their own, which
    /// outlive the builder and its repository.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lent_entries_copy_out() {
        let temp = TempRepo::new("tree-copies");
        let (blob, _) = objects(&temp);
        let mut builder = TreeBuilder::new(temp.repo().as_ref()).unwrap();
        let from_insert: TreeEntryBox = {
            let mut m = builder.as_mut();
            let entry = m.insert_entry(c"a", blob.as_ref(), FileMode::Blob).unwrap();
            entry.to_tree_entry().unwrap()
        };
        let from_get = builder.as_ref().get(c"a").unwrap().to_tree_entry().unwrap();
        builder.as_mut().remove(c"a").unwrap();
        drop(builder);
        drop(temp);
        let expected = (c"a".to_owned(), hex(&blob), ffi_mode(FileMode::Blob));
        assert_eq!(entry_view(&from_insert), expected);
        assert_eq!(entry_view(&from_get.clone()), expected);
    }

    /// Copies of the entries a builder lends cross to other threads, which read and drop
    /// them while this thread replaces, removes and clears the builder's entries: a copy
    /// shares nothing with the builder's storage.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lent_entry_copies_cross_threads() {
        let temp = TempRepo::new("tree-threads");
        let mut builder = TreeBuilder::new(temp.repo().as_ref()).unwrap();
        let names: Vec<CString> = (0..4)
            .map(|i| CString::new(format!("s{i}")).unwrap())
            .collect();
        for (i, name) in (1u8..).zip(&names) {
            builder
                .as_mut()
                .insert(name, id(i).as_ref(), FileMode::Commit)
                .unwrap();
        }
        let copies: Vec<TreeEntryBox> = names
            .iter()
            .map(|n| builder.as_ref().get(n).unwrap().to_tree_entry().unwrap())
            .collect();
        let expected: Vec<_> = copies.iter().map(entry_view).collect();
        let views: Vec<_> = thread::scope(|s| {
            let readers: Vec<_> = copies
                .into_iter()
                .map(|copy| {
                    s.spawn(move || {
                        let view = entry_view(&copy);
                        let again = copy.clone();
                        drop(copy);
                        (view, entry_view(&again))
                    })
                })
                .collect();
            for (i, name) in (10u8..).zip(&names) {
                builder
                    .as_mut()
                    .insert(name, id(i).as_ref(), FileMode::Commit)
                    .unwrap();
                builder.as_mut().remove(name).unwrap();
            }
            builder.as_mut().clear();
            readers.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for ((view, again), expected) in views.iter().zip(&expected) {
            assert_eq!(view, expected);
            assert_eq!(again, expected);
        }
        assert_eq!(expected[2].0, c"s2".to_owned());
    }

    /// Inserting looks each object up through the repository pointer the builder kept,
    /// loading the object database on first use, while Rust reborrows the repository in
    /// between: a shared handle reads its paths and takes its odb, then the builder
    /// inserts, writes and inserts again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn inserts_reach_the_repository_across_reborrows() {
        let temp = TempRepo::new("tree-reborrow");
        let repo = temp.repo();
        let blob = id_of(&write_blob(repo, b"x"));
        let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
        assert!(repo.as_ref().path().is_some());
        builder
            .as_mut()
            .insert(c"a", blob.as_ref(), FileMode::Blob)
            .unwrap();
        let odb = repo.as_ref().odb().unwrap();
        let first = builder.as_mut().write().unwrap();
        drop(odb);
        assert!(repo.as_ref().path().is_some());
        builder
            .as_mut()
            .insert(c"b", first.as_ref(), FileMode::Tree)
            .unwrap();
        assert!(
            builder
                .as_mut()
                .insert(c"c", first.as_ref(), FileMode::Blob)
                .is_err()
        );
        let second = builder.as_mut().write().unwrap();
        assert_ne!(first, second);
    }

    /// A repository without a git directory has no object database to load: `write`
    /// fails, as does inserting anything libgit2 looks up, and the builder stays usable.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn writes_without_an_object_database_fail() {
        init();
        let repo = Repository::new().unwrap();
        let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
        builder
            .as_mut()
            .insert(c"sub", id(1).as_ref(), FileMode::Commit)
            .unwrap();
        for _ in 0..2 {
            let e = builder.as_mut().write().unwrap_err();
            assert!(e.raw_code() < 0);
            assert!(!e.message().is_empty());
        }
        let e = builder
            .as_mut()
            .insert(c"blob", id(2).as_ref(), FileMode::Blob)
            .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Tree));
        assert_eq!(count(&builder), 1);
        builder.as_mut().remove(c"sub").unwrap();
    }

    /// The walk callback lends entries borrowed from a builder or owned, and roots, for the
    /// call only; the closure copies what it keeps. Each answer comes back as given, and
    /// `stopped` tells a stop.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn walk_callbacks_lend_entries_for_the_call() {
        let temp = TempRepo::new("tree-walk-callback");
        let (blob, _) = objects(&temp);
        let mut builder = TreeBuilder::new(temp.repo().as_ref()).unwrap();
        builder
            .as_mut()
            .insert(c"file", blob.as_ref(), FileMode::Blob)
            .unwrap();
        let tree_id = builder.as_mut().write().unwrap();
        let tree = Tree::lookup(temp.repo().as_ref(), tree_id.as_ref()).unwrap();
        let owned = TreeEntry::bypath(tree.as_ref(), c"file").unwrap();

        let mut kept: Vec<(CString, TreeEntryBox)> = Vec::new();
        let answers = [WalkAction::Continue, WalkAction::Skip, WalkAction::Stop];
        let mut next = answers.iter().copied().cycle();
        let mut f = |root: &CStr, entry: TreeEntryRef<'_>| {
            kept.push((root.to_owned(), entry.to_tree_entry().unwrap()));
            next.next().unwrap()
        };
        let mut callback = WalkCallback::new(&mut f);
        let lent = builder.as_ref().get(c"file").unwrap();
        for (i, (root, entry)) in [
            (c"", lent),
            (c"dir/", owned.as_ref()),
            (c"dir/\xff/", lent),
            (c"", owned.as_ref()),
        ]
        .into_iter()
        .enumerate()
        {
            let answer = callback.call(root, entry);
            assert_eq!(answer, answers[i % 3]);
            assert_eq!(callback.stopped(), answer == WalkAction::Stop);
        }
        assert!(format!("{callback:?}").starts_with("WalkCallback { stopped: false"));
        drop(callback);
        drop(builder);
        drop(owned);
        drop(tree);
        assert_eq!(kept.len(), 4);
        for (root, entry) in &kept {
            assert!(root.to_bytes().is_empty() || root.to_bytes().ends_with(b"/"));
            assert_eq!(entry_view(entry).0, c"file".to_owned());
        }
    }

    /// A panic of the closure stays out of C and resumes from `call`; the callback is
    /// usable afterwards. A closure may run a walk callback of its own, with the entry it
    /// was lent.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn walk_callbacks_resume_panics_and_nest() {
        let temp = TempRepo::new("tree-walk-panic");
        let mut builder = TreeBuilder::new(temp.repo().as_ref()).unwrap();
        builder
            .as_mut()
            .insert(c"sub", id(9).as_ref(), FileMode::Commit)
            .unwrap();
        let entry = builder.as_ref().get(c"sub").unwrap();

        let mut calls = 0;
        let mut f = |_: &CStr, _: TreeEntryRef<'_>| {
            calls += 1;
            if calls == 1 {
                panic!("first call panics");
            }
            WalkAction::Skip
        };
        let mut callback = WalkCallback::new(&mut f);
        let panicked = panic::catch_unwind(AssertUnwindSafe(|| callback.call(c"", entry)));
        assert_eq!(
            panicked.unwrap_err().downcast_ref::<&str>(),
            Some(&"first call panics")
        );
        assert!(callback.stopped());
        assert_eq!(callback.call(c"", entry), WalkAction::Skip);
        assert!(!callback.stopped());
        drop(callback);
        assert_eq!(calls, 2);

        let mut depth: c_int = 0;
        let mut outer = |root: &CStr, entry: TreeEntryRef<'_>| {
            depth += 1;
            let mut inner = |r: &CStr, e: TreeEntryRef<'_>| {
                assert_eq!(view(e).0, c"sub".to_owned());
                assert_eq!(r, c"inner/");
                WalkAction::Stop
            };
            let mut nested = WalkCallback::new(&mut inner);
            assert_eq!(nested.call(c"inner/", entry), WalkAction::Stop);
            assert_eq!(root, c"outer/");
            WalkAction::Continue
        };
        let mut callback = WalkCallback::new(&mut outer);
        assert_eq!(callback.call(c"outer/", entry), WalkAction::Continue);
        drop(callback);
        assert_eq!(depth, 1);
    }

    /// Builders on threads, each over its own repository, insert, look up, remove and
    /// write at once.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn routines_on_threads() {
        let temp = TempRepo::new("tree-routine-threads");
        let path = temp.c_path();
        let written: Vec<Oid> = thread::scope(|s| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let path = &path;
                    s.spawn(move || {
                        let repo = Repository::open(path).unwrap();
                        let mut builder: TreeBuilderBox<'_> =
                            TreeBuilder::new(repo.as_ref()).unwrap();
                        for i in 0..8u8 {
                            let name = CString::new(format!("t{i}")).unwrap();
                            builder
                                .as_mut()
                                .insert(&name, id(i + 1).as_ref(), FileMode::Commit)
                                .unwrap();
                            assert!(builder.as_ref().get(&name).is_some());
                        }
                        builder.as_mut().remove(c"t0").unwrap();
                        builder.as_mut().write().unwrap()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(written.windows(2).all(|w| w[0] == w[1]));
    }
}

mod tree_entry {
    //! UB tests for `git2/tree.h`'s tree entries: the owned `TreeEntryBox` and its
    //! handles.
    //!
    //! An owned entry is one block of libgit2's allocator, its name inline right past the
    //! struct: `git_tree_entry_free` releases it once and `git_tree_entry_dup` copies it
    //! whole into a new one. A double free, a leaked copy, or a copy that reads or writes
    //! past the name (an off-by-one in the inline name) is flagged by ASan (LSan included):
    //! the tests copy entries from the box and from its shared handle, drop chains of
    //! copies in every order, copy names of every length a tree allows (1 to 65535 bytes),
    //! and let copies outlive the tree and the repository they came from, which an owned
    //! entry does not reach.
    //!
    //! An entry is `Send` and `Sync`: these tests copy one entry from many threads at once
    //! and move copies to other threads to drop them (TSan). A handle outliving its owner
    //! and aliased exclusive handles do not compile (`compile_fail/` and doctests).
    //!
    //! The getters lend the entry's name and id, which live inside its block: they are
    //! read in full for names of every length (ASan: a read past the block), outlive the
    //! tree and repository the entry came from, and are read from many threads at once
    //! with the ordering between entries (TSan). An entry's object outlives the entry,
    //! and a name or object outliving what keeps it alive does not compile
    //! (`compile_fail/`). A `TreeBuilderFilter` lends each entry for its call only
    //! (doctests), and a panicking closure resumes after the call.
    //!
    //! Every test calls libgit2, so Miri skips them all.

    use std::cmp::Ordering;
    use std::ffi::{CStr, CString};
    use std::sync::Barrier;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::{FileMode, ObjectType};
    use libgit2::libgit2::object::object::Object;
    use libgit2::libgit2::object::tree::{
        Tree, TreeArc, TreeBuilder, TreeBuilderFilter, TreeEntry, TreeEntryBox, TreeEntryRef,
        TreeRef,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::RepositoryRef;
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use crate::fixture::TempRepo;
    use crate::fixture::object_and_tree_entry::{
        entry_view, object_view, open_testrepo, repository, write_raw, write_tree,
    };

    /// Commit `763d71a`'s tree, which nests `ab/de/fgh/1.txt`.
    const TREE: &CStr = c"ae90f12eea699729ed24555e40b9fd669da12a12";
    const PATHS: [&CStr; 6] = [
        c"README",
        c"ab",
        c"ab/",
        c"ab/c/3.txt",
        c"ab/de/fgh",
        c"ab/de/fgh/1.txt",
    ];

    fn tree<'r>(repo: RepositoryRef<'r>) -> TreeArc<'r> {
        let id = Oid::from_string(TREE, OidType::Sha1).unwrap();
        Tree::lookup(repo, id.as_ref()).unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copies_outlive_the_tree_and_the_repository() {
        let repo = open_testrepo();
        let root = tree(repo.as_ref());
        let entries: Vec<TreeEntryBox> = PATHS
            .iter()
            .map(|p| TreeEntry::bypath(root.as_ref(), p).unwrap())
            .collect();
        let expected: Vec<_> = entries.iter().map(entry_view).collect();
        let copies: Vec<_> = entries
            .iter()
            .map(|e| e.as_ref().to_tree_entry().unwrap())
            .collect();
        let clones: Vec<_> = entries.iter().map(TreeEntryBox::clone).collect();
        drop(root);
        drop(repo);
        drop(entries);
        for ((copy, clone), view) in copies.iter().zip(&clones).zip(&expected) {
            assert_eq!(&entry_view(copy), view);
            assert_eq!(&entry_view(clone), view);
        }
        assert_eq!(expected[1].0.as_c_str(), c"ab");
        assert_eq!(expected[1].2, ffi::GIT_FILEMODE_TREE);
        assert_eq!(expected[5].0.as_c_str(), c"1.txt");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copy_chains_release_every_copy_in_any_order() {
        let repo = open_testrepo();
        let root = tree(repo.as_ref());
        let original = TreeEntry::bypath(root.as_ref(), c"ab/de/fgh/1.txt").unwrap();
        let expected = entry_view(&original);
        for order in 0..3 {
            let mut chain = vec![original.clone()];
            for i in 0..16 {
                let last = chain.last().unwrap();
                let next = match i % 3 {
                    0 => last.clone(),
                    1 => last.try_clone().unwrap(),
                    _ => last.as_ref().to_tree_entry().unwrap(),
                };
                chain.push(next);
            }
            match order {
                0 => chain.reverse(),
                1 => chain.rotate_left(5),
                _ => {}
            }
            while let Some(entry) = chain.pop() {
                assert_eq!(entry_view(&entry), expected);
            }
        }
        assert_eq!(entry_view(&original), expected);
    }

    /// Names of 1 to 65535 bytes, the most a tree entry's 16-bit length holds, and of
    /// every byte a name may hold, copied whole: an off-by-one in the inline name leaves
    /// the block (ASan).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn names_of_every_length_copy_whole() {
        let repo = TempRepo::new("tree-entry-names");
        let lens = [
            1usize, 2, 7, 8, 9, 15, 16, 17, 255, 256, 4095, 4096, 65534, 65535,
        ];
        let mut names: Vec<CString> = lens
            .iter()
            .map(|&n| CString::new(vec![b'n'; n]).unwrap())
            .collect();
        names.push(CString::new((1u8..=255).filter(|b| *b != b'/').collect::<Vec<_>>()).unwrap());
        let id = write_tree(&repo, b"payload\n", &names);
        let root = Tree::lookup(repository(&repo), id.as_ref()).unwrap();
        for name in &names {
            let entry = TreeEntry::bypath(root.as_ref(), name).unwrap();
            let copy = entry.as_ref().to_tree_entry().unwrap();
            drop(entry);
            let clone = copy.clone();
            drop(copy);
            assert_eq!(&entry_view(&clone).0, name);
        }
    }

    /// Many threads copy one shared entry at once, through the owner and its shared
    /// handle, and move their copies to other threads that read and drop them (TSan).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn entries_cross_threads() {
        let repo = open_testrepo();
        let root = tree(repo.as_ref());
        let shared = TreeEntry::bypath(root.as_ref(), c"ab/c/3.txt").unwrap();
        let expected = entry_view(&shared);
        drop(root);
        drop(repo);

        const THREADS: usize = 8;
        let barrier = Barrier::new(THREADS);
        let copies: Vec<TreeEntryBox> = thread::scope(|s| {
            let workers: Vec<_> = (0..THREADS)
                .map(|i| {
                    let (shared, barrier) = (&shared, &barrier);
                    s.spawn(move || {
                        let handle = shared.as_ref();
                        barrier.wait();
                        let mut mine = Vec::new();
                        for j in 0..64 {
                            let copy = if (i + j) % 2 == 0 {
                                shared.clone()
                            } else {
                                handle.to_tree_entry().unwrap()
                            };
                            mine.push(copy);
                        }
                        mine.truncate(4);
                        mine
                    })
                })
                .collect();
            workers
                .into_iter()
                .flat_map(|w| w.join().unwrap())
                .collect()
        });

        let views: Vec<_> = copies
            .into_iter()
            .map(|copy| thread::spawn(move || entry_view(&copy)))
            .collect();
        for view in views {
            assert_eq!(view.join().unwrap(), expected);
        }
        let moved = thread::spawn(move || {
            let view = entry_view(&shared);
            drop(shared);
            view
        });
        assert_eq!(moved.join().unwrap(), expected);
    }

    /// The shared handle is `Copy` and outlives no owner; several may coexist.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shared_handles_coexist() {
        let repo = open_testrepo();
        let root = tree(repo.as_ref());
        let mut entry = TreeEntry::bypath(root.as_ref(), c"README").unwrap();
        let a = entry.as_ref();
        let b = a;
        let (x, y) = (a.to_tree_entry().unwrap(), b.to_tree_entry().unwrap());
        assert_eq!(entry_view(&x), entry_view(&y));
        let m = entry.as_mut();
        assert_eq!(format!("{m:?}"), "TreeEntry { .. }");
        assert_eq!(format!("{:?}", m.as_ref()), "TreeEntry { .. }");
    }

    /// Lookups by path own their entry alone: every failure, at any depth, leaves
    /// nothing behind (LSan), every success is released once (ASan), and an entry
    /// found through a subtree outlives the tree, the repository and the subtrees
    /// libgit2 read on the way.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn path_lookups_own_their_entry_alone() {
        let repo = open_testrepo();
        let root = tree(repo.as_ref());
        let misses: [(&CStr, ErrorCode); 8] = [
            (c"", ErrorCode::NotFound),
            (c"/", ErrorCode::NotFound),
            (c"nope", ErrorCode::NotFound),
            (c"ab/nope", ErrorCode::NotFound),
            (c"ab/de/fgh/nope", ErrorCode::NotFound),
            (c"README/", ErrorCode::NotFound),
            (c"ab/c/3.txt/x", ErrorCode::NotFound),
            (c"ab//de", ErrorCode::NotFound),
        ];
        for (path, code) in misses {
            let e = TreeEntry::bypath(root.as_ref(), path).unwrap_err();
            assert_eq!(e.code(), Some(code), "{path:?}: {e:?}");
            assert_eq!(e.class(), Some(ErrorClass::Tree), "{path:?}");
        }
        let deep = TreeEntry::bypath(root.as_ref(), c"ab/de/fgh/1.txt").unwrap();
        let dir = TreeEntry::bypath(root.as_ref(), c"ab/de/").unwrap();
        let again = TreeEntry::bypath(root.as_ref(), c"ab/de/fgh/1.txt").unwrap();
        assert_ne!(deep.as_c_ptr(), again.as_c_ptr());
        drop(root);
        drop(repo);
        assert_eq!(entry_view(&deep), entry_view(&again));
        assert_eq!(entry_view(&deep).0.as_c_str(), c"1.txt");
        assert_eq!(entry_view(&dir).0.as_c_str(), c"de");
        assert_eq!(entry_view(&dir).2, ffi::GIT_FILEMODE_TREE);
        drop(again);
        let copy = deep.clone();
        drop(deep);
        assert_eq!(entry_view(&copy).0.as_c_str(), c"1.txt");
    }

    /// A tree whose subtree entry names an object the database lacks: the lookups
    /// through it fail with libgit2's "object not found" error (code `-1` from
    /// `git_tree_entry_bypath`, `GIT_ENOTFOUND` from the odb through
    /// `ObjectRef::lookup_bypath`'s peel), leaving nothing behind (LSan); the entry
    /// itself is still found, and a blob entry beside it looked up.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn missing_subtrees_fail_with_libgit2s_error() {
        let repo = TempRepo::new("tree-entry-missing-subtree");
        let blob = write_raw(&repo, b"leaf\n", ffi::GIT_OBJECT_BLOB);
        // Entries in git's order (libgit2 searches them by bisection).
        let mut raw = b"100644 leaf\0".to_vec();
        raw.extend(&blob.as_ref().id()[..20]);
        raw.extend(b"40000 sub\0");
        raw.extend([0xDE; 20]);
        let id = write_raw(&repo, &raw, ffi::GIT_OBJECT_TREE);
        let tree = Tree::lookup(repository(&repo), id.as_ref()).unwrap();
        // The entry itself, with or without a trailing `/`, needs no subtree.
        let sub = TreeEntry::bypath(tree.as_ref(), c"sub").unwrap();
        assert_eq!(entry_view(&sub).1, "de".repeat(20));
        assert_eq!(entry_view(&sub).2, ffi::GIT_FILEMODE_TREE);
        assert_eq!(
            entry_view(&TreeEntry::bypath(tree.as_ref(), c"sub/").unwrap()),
            entry_view(&sub)
        );
        drop(sub);
        for path in [c"sub/x", c"sub/a/b"] {
            let e = TreeEntry::bypath(tree.as_ref(), path).unwrap_err();
            assert_eq!(e.raw_code(), -1, "{path:?}: {e:?}");
            assert_eq!(e.class(), Some(ErrorClass::Odb));
            let e = tree
                .as_ref()
                .as_object()
                .lookup_bypath(path, None)
                .unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Odb), "{path:?}: {e:?}");
        }
        let leaf = TreeEntry::bypath(tree.as_ref(), c"leaf").unwrap();
        assert_eq!(entry_view(&leaf).0.as_c_str(), c"leaf");
        let object = tree
            .as_ref()
            .as_object()
            .lookup_bypath(c"leaf", Some(ObjectType::Blob))
            .unwrap();
        assert_eq!(object_view(&object).0, entry_view(&leaf).1);
        // The missing subtree, as an object: not found.
        let e = tree
            .as_ref()
            .as_object()
            .lookup_bypath(c"sub", None)
            .unwrap_err();
        assert_eq!(e.code(), Some(ErrorCode::NotFound), "{e:?}");
    }

    /// An object that is not a tree has no tree handle, so libgit2 never reads one as a
    /// tree: a commit, a blob and a tag (ASan would flag the read of a smaller object).
    /// The tree itself is one, and the same lookup through the peeled commit agrees.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn only_trees_are_looked_into() {
        let repo = open_testrepo();
        for hex in [
            c"763d71aadf09a7951596c9746c024e7eece7c7af",
            c"a8233120f6ad708f843d861ce2b7228ec4e3dec6",
            c"b25fa35b38051e4ae45d4222e795f9df2e43f1d1",
        ] {
            let id = Oid::from_string(hex, OidType::Sha1).unwrap();
            let object = Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap();
            assert!(TreeRef::from_object(object.as_ref()).is_none(), "{hex:?}");
        }
        let root = tree(repo.as_ref());
        let commit_id =
            Oid::from_string(c"763d71aadf09a7951596c9746c024e7eece7c7af", OidType::Sha1).unwrap();
        let commit = Object::lookup(repo.as_ref(), commit_id.as_ref(), None).unwrap();
        let peeled = commit.as_ref().peel(Some(ObjectType::Tree)).unwrap();
        let peeled = Tree::from_object(peeled).unwrap();
        assert!(TreeArc::ptr_eq(&peeled, &root));
        let a = TreeEntry::bypath(root.as_ref(), c"ab/c/3.txt").unwrap();
        let b = TreeEntry::bypath(peeled.as_ref(), c"ab/c/3.txt").unwrap();
        assert_eq!(entry_view(&a), entry_view(&b));
    }

    /// Names of every length a tree allows, lent by the getter, read whole and kept past
    /// the tree and the repository, with the id, mode and type beside them.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_lend_from_the_entry() {
        let repo = TempRepo::new("tree-entry-getters");
        let lens = [1usize, 2, 7, 8, 9, 16, 17, 255, 256, 4096, 65535];
        let names: Vec<CString> = lens
            .iter()
            .map(|&n| CString::new(vec![b'g'; n]).unwrap())
            .collect();
        let id = write_tree(&repo, b"payload\n", &names);
        let root = Tree::lookup(repository(&repo), id.as_ref()).unwrap();
        let entries: Vec<TreeEntryBox> = names
            .iter()
            .map(|n| TreeEntry::bypath(root.as_ref(), n).unwrap())
            .collect();
        let blob = entries[0].as_ref().to_object(repository(&repo)).unwrap();
        assert_eq!(blob.as_ref().kind(), ObjectType::Blob);
        drop(blob);
        drop(root);
        drop(repo);
        let views: Vec<_> = entries.iter().map(entry_view).collect();
        for (entry, (name, hex, raw)) in entries.iter().zip(&views) {
            let handle = entry.as_ref();
            assert_eq!(handle.name(), name.as_c_str());
            assert_eq!(&handle.id().to_hex_string().unwrap(), hex);
            assert_eq!(handle.filemode_raw(), *raw);
            assert_eq!(handle.filemode(), FileMode::Blob);
            assert_eq!(handle.kind(), ObjectType::Blob);
        }
        for pair in entries.windows(2) {
            assert_eq!(
                pair[0].as_ref().cmp_in_tree(pair[1].as_ref()),
                Ordering::Less
            );
        }
    }

    /// Many threads read every getter of shared entries at once, and order them (TSan).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_cross_threads() {
        let repo = open_testrepo();
        let root = tree(repo.as_ref());
        let entries: Vec<TreeEntryBox> = PATHS
            .iter()
            .map(|p| TreeEntry::bypath(root.as_ref(), p).unwrap())
            .collect();
        drop(root);
        drop(repo);
        let expected: Vec<_> = entries.iter().map(entry_view).collect();
        const THREADS: usize = 8;
        let barrier = Barrier::new(THREADS);
        thread::scope(|s| {
            for _ in 0..THREADS {
                let (entries, expected, barrier) = (&entries, &expected, &barrier);
                s.spawn(move || {
                    barrier.wait();
                    for _ in 0..32 {
                        for (entry, (name, hex, raw)) in entries.iter().zip(expected) {
                            let handle = entry.as_ref();
                            assert_eq!(handle.name(), name.as_c_str());
                            assert_eq!(&handle.id().to_hex_string().unwrap(), hex);
                            assert_eq!(handle.filemode_raw(), *raw);
                            let _ = (handle.filemode(), handle.kind());
                            for other in entries {
                                let _ = handle.cmp_in_tree(other.as_ref());
                            }
                        }
                    }
                });
            }
        });
    }

    /// An entry's object outlives the entry and the tree it came from, looked up in its
    /// own repository or in another holding it; a repository lacking it fails cleanly.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn objects_outlive_their_entries() {
        let repo = open_testrepo();
        let other = open_testrepo();
        let empty = TempRepo::new("tree-entry-objects");
        let root = tree(repo.as_ref());
        let mut objects = Vec::new();
        for path in PATHS {
            let entry = TreeEntry::bypath(root.as_ref(), path).unwrap();
            objects.push(entry.as_ref().to_object(repo.as_ref()).unwrap());
            objects.push(entry.as_ref().to_object(other.as_ref()).unwrap());
            let e = entry.as_ref().to_object(repository(&empty)).unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::NotFound));
        }
        drop(root);
        let views: Vec<_> = objects.iter().map(object_view).collect();
        for pair in views.chunks(2) {
            assert_eq!(pair[0], pair[1]);
        }
    }

    /// A filter called directly on entries of every kind; a closure that panics on one
    /// call resumes the panic and leaves the filter usable for the next.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn filters_lend_entries_for_the_call() {
        let repo = open_testrepo();
        let root = tree(repo.as_ref());
        let entries: Vec<TreeEntryBox> = PATHS
            .iter()
            .map(|p| TreeEntry::bypath(root.as_ref(), p).unwrap())
            .collect();
        let mut seen = Vec::new();
        let mut calls = 0;
        let mut pick = |entry: TreeEntryRef<'_>| {
            calls += 1;
            if calls == 3 {
                panic!("third call");
            }
            seen.push(entry.name().to_owned());
            entry.kind() == ObjectType::Tree
        };
        let mut filter = TreeBuilderFilter::new(&mut pick);
        let mut verdicts = Vec::new();
        for entry in &entries {
            let call = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                filter.call(entry.as_ref())
            }));
            verdicts.push(call.ok());
        }
        assert_eq!(
            format!("{filter:?}"),
            "TreeBuilderFilter { panicked: false, .. }"
        );
        drop(filter);
        assert_eq!(
            verdicts,
            [
                Some(false),
                Some(true),
                None,
                Some(false),
                Some(true),
                Some(false)
            ]
        );
        assert_eq!(seen.len(), PATHS.len() - 1);
        // A builder seeded from the tree is untouched by the filter's calls.
        let builder = TreeBuilder::from_tree(repo.as_ref(), root.as_ref()).unwrap();
        assert_eq!(builder.as_ref().len(), 4);
    }
}

mod tree_update {
    //! UB tests for `git2/tree.h`: the `TreeUpdate` value (`git_tree_update`), its
    //! handles and accessors. An update is plain Rust-owned storage with a borrowed path,
    //! so the dangers are a path read after its string is gone or past its end, an id
    //! handle that outlives or aliases the update, and races on shared updates; none of
    //! these tests calls libgit2, so they run under Miri as well as ASan, UBSan, TSan and
    //! BSan.
    //!
    //! Paths of every length up to a few hundred bytes are heap strings, stored and read
    //! back whole through every write path (ASan: bounds; Miri: provenance of the stored
    //! pointer). The exclusive id handle writes the struct in place while the update is
    //! reborrowed around it (Miri, BSan: reborrows). Updates are shared across threads,
    //! read concurrently and copied out to threads that keep their copies (TSan).
    //! Illegal uses are compile-fail doctests on the crate and files under
    //! `compile_fail/`: an update outliving its path, a stored path outliving the update's
    //! use, aliased handles, a handle or id handle outliving its update.

    use std::ffi::{CStr, CString};
    use std::hint::black_box;
    use std::sync::Barrier;
    use std::thread;

    use ffibox::CZeroable;
    use libgit2::libgit2::core::types::FileMode;
    use libgit2::libgit2::object::tree::{TreeUpdate, TreeUpdateMut, UpdateAction};
    use libgit2::libgit2::oid::oid::{Oid, OidType};

    const MODES: [FileMode; 6] = [
        FileMode::Unreadable,
        FileMode::Tree,
        FileMode::Blob,
        FileMode::BlobExecutable,
        FileMode::Link,
        FileMode::Commit,
    ];

    /// A path of `len` bytes, `/`-separated components of letters.
    fn path(len: usize) -> CString {
        let bytes = (0..len)
            .map(|i| {
                if i % 7 == 6 {
                    b'/'
                } else {
                    b'a' + (i % 26) as u8
                }
            })
            .collect::<Vec<_>>();
        CString::new(bytes).unwrap()
    }

    /// An id of type `kind` whose bytes count up from `seed`.
    fn id(kind: OidType, seed: u8) -> Oid {
        let mut id = Oid::zero(kind);
        id.as_mut()
            .set_id(core::array::from_fn(|i| seed.wrapping_add(i as u8)));
        id
    }

    /// Every path length, stored through each constructor and setter, reads back whole,
    /// and a shorter path stored over a longer one is read to its own end.
    #[test]
    fn paths_of_every_length_read_back_whole() {
        let stride = if cfg!(bsan) || cfg!(miri) { 31 } else { 1 };
        let longest = 300;
        let paths: Vec<CString> = (0..=longest).step_by(stride).map(path).collect();
        let short = CString::new("x").unwrap();
        for (i, p) in paths.iter().enumerate() {
            let mode = MODES[i % MODES.len()];
            let mut up = TreeUpdate::upsert(p, id(OidType::Sha1, i as u8).as_ref(), mode);
            assert_eq!(black_box(up.as_ref().path()), p.as_c_str());
            assert_eq!(up.as_ref().filemode(), mode);
            let mut gone = TreeUpdate::remove(p);
            assert_eq!(
                black_box(gone.as_ref().path()).to_bytes().len(),
                p.as_bytes().len()
            );
            assert!(!format!("{gone:?}").is_empty());

            let other = &paths[paths.len() - 1 - i];
            up.as_mut().set_path(other);
            assert_eq!(up.as_ref().path(), other.as_c_str());
            gone.as_mut().set_path(&short);
            assert_eq!(gone.as_ref().path(), c"x");
            gone.as_mut().set_path(p);
            assert_eq!(gone.as_ref().to_tree_update().as_ref().path(), p.as_c_str());
        }
    }

    /// The exclusive id handle rewrites the update's id in place, every type byte
    /// included, while the update is reborrowed around it; the shared id handle reads
    /// what it wrote, and the other fields are untouched.
    #[test]
    fn id_handles_rewrite_in_place() {
        let mut update = TreeUpdate::upsert(c"dir/f", Oid::zeroed().as_ref(), FileMode::Blob);
        let kinds = if cfg!(miri) {
            vec![0, 1, 2, 3, 255]
        } else {
            (0..=255).collect()
        };
        for kind in kinds {
            let mut m: TreeUpdateMut<'_, '_> = update.as_mut();
            {
                let mut again = m.as_mut();
                let mut oid = again.id_mut();
                oid.set_raw_kind(kind);
                oid.set_id([kind; 32]);
                assert!(oid.id_mut().set_elem(31, !kind));
            }
            let shared = m.as_ref();
            let oid = shared.id();
            assert_eq!(oid.raw_kind(), kind);
            assert_eq!(oid.id()[..31], [kind; 31]);
            assert_eq!(oid.id()[31], !kind);
            assert_eq!(oid.kind(), OidType::from_raw(kind.into()));
            assert_eq!(shared.path(), c"dir/f");
            assert_eq!(shared.filemode(), FileMode::Blob);
            m.set_id(id(OidType::Sha256, kind).as_ref());
            assert_eq!(
                update.as_ref().id().id(),
                id(OidType::Sha256, kind).as_ref().id()
            );
        }
    }

    /// Copies, by `Copy` and through a handle, are values of their own: changing one
    /// changes no other, and each keeps reading its borrowed path.
    #[test]
    fn copies_are_independent() {
        let a = CString::new("a/b/c").unwrap();
        let b = CString::new("d").unwrap();
        let original = TreeUpdate::upsert(&a, id(OidType::Sha1, 9).as_ref(), FileMode::Link);
        let mut copies = vec![original; 4];
        copies.push(original.as_ref().to_tree_update());
        for (i, copy) in copies.iter_mut().enumerate() {
            let mut m = copy.as_mut();
            match i % 4 {
                0 => m.set_path(&b),
                1 => m.set_action(UpdateAction::Remove),
                2 => m.set_filemode(FileMode::Commit),
                _ => m.id_mut().set_raw_kind(0),
            }
            assert_ne!(*copy, original);
        }
        assert_eq!(original.as_ref().path(), a.as_c_str());
        assert_eq!(original.as_ref().action(), UpdateAction::Upsert);
        assert_eq!(copies[0].as_ref().path(), c"d");
        assert_eq!(copies[1].as_ref().path(), c"a/b/c");
        assert_eq!(copies[4].as_ref().path(), c"d");
    }

    /// A slice of updates, the array C reads, is read through shared handles of every
    /// element at once and rewritten element by element.
    #[test]
    fn slices_of_updates() {
        let paths: Vec<CString> = (1..=16).map(path).collect();
        let mut updates: Vec<TreeUpdate<'_>> =
            paths.iter().map(|p| TreeUpdate::remove(p)).collect();
        let handles: Vec<_> = updates.iter().map(TreeUpdate::as_ref).collect();
        for (h, p) in handles.iter().zip(&paths) {
            assert_eq!(h.path(), p.as_c_str());
            assert_eq!(h.action(), UpdateAction::Remove);
        }
        for (i, update) in updates.iter_mut().enumerate() {
            let mut m = update.as_mut();
            m.set_action(UpdateAction::Upsert);
            m.set_filemode(MODES[i % MODES.len()]);
            m.set_id(id(OidType::Sha1, i as u8).as_ref());
        }
        for (i, update) in updates.iter().enumerate() {
            assert_eq!(update.as_ref().filemode(), MODES[i % MODES.len()]);
            assert_eq!(update.as_ref().id().id()[0], i as u8);
        }
    }

    /// Updates are shared by many threads that read them at once and copy them out to
    /// threads of their own, which read and change their copies; the originals stay as
    /// they were (TSan).
    #[test]
    fn updates_cross_threads() {
        const THREADS: usize = 8;
        let paths: Vec<CString> = (1..=THREADS).map(|n| path(n * 9)).collect();
        let updates: Vec<TreeUpdate<'_>> = paths
            .iter()
            .enumerate()
            .map(|(i, p)| {
                TreeUpdate::upsert(p, id(OidType::Sha1, i as u8).as_ref(), FileMode::Blob)
            })
            .collect();
        let barrier = Barrier::new(THREADS);
        let rounds = if cfg!(miri) { 2 } else { 64 };
        thread::scope(|s| {
            let readers: Vec<_> = (0..THREADS)
                .map(|t| {
                    let (updates, paths, barrier) = (&updates[..], &paths, &barrier);
                    s.spawn(move || {
                        barrier.wait();
                        let mut mine = Vec::new();
                        for r in 0..rounds {
                            let i = (t + r) % THREADS;
                            let handle = updates[i].as_ref();
                            assert_eq!(handle.path(), paths[i].as_c_str());
                            assert_eq!(handle.id().id()[0], i as u8);
                            mine.push(handle.to_tree_update());
                        }
                        mine
                    })
                })
                .collect();
            let copies: Vec<Vec<TreeUpdate<'_>>> =
                readers.into_iter().map(|r| r.join().unwrap()).collect();
            let writers: Vec<_> = copies
                .into_iter()
                .map(|mut mine| {
                    s.spawn(move || {
                        for copy in &mut mine {
                            let mut m = copy.as_mut();
                            m.set_action(UpdateAction::Remove);
                            m.set_path(c"elsewhere");
                        }
                        mine.iter()
                            .all(|c| c.as_ref().path() == c"elsewhere" as &CStr)
                    })
                })
                .collect();
            for w in writers {
                assert!(w.join().unwrap());
            }
        });
        for (i, update) in updates.iter().enumerate() {
            assert_eq!(update.as_ref().action(), UpdateAction::Upsert);
            assert_eq!(update.as_ref().path(), paths[i].as_c_str());
        }
    }
}

mod tree_object {
    //! UB tests for `git2/tree.h`'s `git_tree`: its owner `TreeArc<'r>`, the
    //! constructors `Tree::lookup` and `Tree::lookup_prefix`, the handles,
    //! `TreeRef::to_tree`, and the conversions from and to an object
    //! (`Tree::from_object`, `Tree::into_object`, `TreeRef::from_object`,
    //! `TreeRef::as_object`).
    //!
    //! A tree is counted, and shared with its repository's cache, which hands the same
    //! object to every lookup of its id, as a tree or as an object: every owner gives back
    //! its one reference through `git_tree_free` (or `git_object_free`, once handed over
    //! as an object) exactly once, and every clone takes one through `git_tree_dup`. A
    //! miscount frees a tree the cache or another owner still reads (ASan), or leaks it
    //! with its entries and raw data (LSan). The tests share trees between lookups,
    //! clones and conversions, drop them in every order, let the cache evict trees owners
    //! still hold and run without a cache, and read entries borrowed from each survivor.
    //! Every failing lookup must leave nothing behind, a half-parsed tree's entries
    //! included, and an id of another type than the repository's must be refused before
    //! it reaches C. libgit2 copies the id it is given (BSan: the tests overwrite and free
    //! it right after), and a tree keeps a bare pointer to its repository, which looking
    //! up a path through its subtrees follows across Rust reborrows of that repository
    //! (BSan). Trees are not `Send`, so the threaded test gives each thread its own
    //! repository (TSan).
    //!
    //! A tree cannot outlive its repository, nor a handle (or a tree viewed from an
    //! object) its owner, and no owner lends an exclusive handle (`compile_fail/` and
    //! doctests). Every test calls libgit2, so Miri skips them all.

    use std::ffi::CString;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::object::tree::{Tree, TreeArc, TreeEntry, TreeRef};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryError};
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use crate::fixture::object_and_tree_entry::{
        enable_caching, entry_view, object_view, open_testrepo, repository, set_cache_max_size,
        write_raw,
    };
    use crate::fixture::tree::entries;
    use crate::fixture::{TempRepo, isolated};

    /// Trees of `testrepo.git`: `HEAD`'s, commit `763d71a`'s (nesting `ab/de/fgh`), its
    /// subtrees `ab`, `ab/c`, `ab/de` and `ab/de/fgh`, and more commits' trees.
    const TREES: [&str; 9] = [
        "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162",
        "ae90f12eea699729ed24555e40b9fd669da12a12",
        "f1425cef211cc08caa31e7b545ffb232acb098c3",
        "9a03079b8a8ee85a0bee58bf9be3da8b62414ed4",
        "b6361fc6a97178d8fc8639fdeed71c775ab52593",
        "3259a6bd5b57fb9c1281bb7ed3167b50f224cb54",
        "53fc32d17276939fc79ed05badaef2db09990016",
        "1810dff58d8a660512d4832e740f692884338ccd",
        "181037049a54a1eb5fab404658a3a250b44335d7",
    ];
    const NESTED: &str = "ae90f12eea699729ed24555e40b9fd669da12a12";
    const COMMIT: &str = "763d71aadf09a7951596c9746c024e7eece7c7af";
    const BLOB: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";
    const TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";

    fn id(hex: &str) -> Oid {
        Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap()
    }

    /// C's hex id of the tree an owner holds, read through another counted reference
    /// handed over as an object.
    fn hex(tree: &TreeArc<'_>) -> String {
        let (hex, kind) = object_view(&Tree::into_object(tree.clone()));
        assert_eq!(kind, ffi::GIT_OBJECT_TREE);
        hex
    }

    fn assert_failed(result: Result<TreeArc<'_>, RepositoryError>, code: i32, class: ErrorClass) {
        let e = result.unwrap_err();
        assert_eq!((e.raw_code(), e.class()), (code, Some(class)), "{e:?}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_clones_and_conversions_share_one_tree() {
        let repo = open_testrepo();
        let r = repo.as_ref();
        for hex_id in TREES {
            let tree = Tree::lookup(r, id(hex_id).as_ref()).unwrap();
            let short = Tree::lookup_prefix(r, id(hex_id).as_ref(), 7).unwrap();
            let object = Object::lookup(r, id(hex_id).as_ref(), None).unwrap();
            let viewed = TreeRef::from_object(object.as_ref()).unwrap().to_tree();
            let adopted = Tree::from_object(object.clone()).unwrap();
            assert!(TreeArc::ptr_eq(&tree, &short));
            assert!(TreeArc::ptr_eq(&tree, &viewed) && TreeArc::ptr_eq(&tree, &adopted));
            let expected = entries(&tree);
            let clone = tree.clone();
            let copy = short.as_ref().to_tree();
            let as_object = Tree::into_object(adopted);
            drop((tree, short, object, viewed));
            assert_eq!(hex(&clone), hex_id);
            drop(clone);
            assert_eq!(entries(&copy), expected);
            drop(copy);
            assert_eq!(object_view(&as_object).0, hex_id);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reference_chains_release_every_reference_in_any_order() {
        let repo = open_testrepo();
        for order in 0..3 {
            let first = Tree::lookup(repo.as_ref(), id(NESTED).as_ref()).unwrap();
            let expected = entries(&first);
            let mut chain = vec![first];
            for i in 0..16 {
                let last = chain.last().unwrap();
                let next = match i % 3 {
                    0 => last.clone(),
                    1 => last.as_ref().to_tree(),
                    // Handed over as an object and back: one reference all the way.
                    _ => Tree::from_object(Tree::into_object(last.clone())).unwrap(),
                };
                chain.push(next);
            }
            match order {
                0 => chain.reverse(),
                1 => chain.rotate_left(7),
                _ => {}
            }
            while let Some(tree) = chain.pop() {
                assert_eq!(entries(&tree), expected);
                drop(tree);
                if let Some(survivor) = chain.first() {
                    assert_eq!(hex(survivor), NESTED);
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
        for other in [COMMIT, BLOB, TAG] {
            assert_failed(
                Tree::lookup(r, id(other).as_ref()),
                not_found,
                ErrorClass::Invalid,
            );
            assert_failed(
                Tree::lookup_prefix(r, id(other).as_ref(), 9),
                not_found,
                ErrorClass::Invalid,
            );
        }
        assert_failed(
            Tree::lookup(r, id(&"de".repeat(20)).as_ref()),
            not_found,
            ErrorClass::Odb,
        );
        assert_failed(
            Tree::lookup(r, Oid::zero(OidType::Sha1).as_ref()),
            not_found,
            ErrorClass::Odb,
        );
        for len in [0, 1, 3] {
            assert_failed(
                Tree::lookup_prefix(r, id(NESTED).as_ref(), len),
                ErrorCode::Ambiguous.raw(),
                ErrorClass::Object,
            );
        }
        // `1810` starts two trees.
        let prefix = Oid::from_prefix(b"1810", OidType::Sha1).unwrap();
        assert_failed(
            Tree::lookup_prefix(r, prefix.as_ref(), 4),
            ErrorCode::Ambiguous.raw(),
            ErrorClass::Odb,
        );

        let pathless = Repository::new().unwrap();
        assert_failed(
            Tree::lookup(pathless.as_ref(), id(NESTED).as_ref()),
            not_found,
            ErrorClass::Invalid,
        );
        assert_failed(
            Tree::lookup_prefix(pathless.as_ref(), id(NESTED).as_ref(), 7),
            not_found,
            ErrorClass::Invalid,
        );

        // Ids of another type, at every length, never reach C.
        let mut wide = Oid::from_raw(&id(NESTED).as_ref().id(), OidType::Sha256).unwrap();
        for kind in [2, 0, 3, 0x7F, u8::MAX] {
            wide.as_mut().set_raw_kind(kind);
            assert_failed(Tree::lookup(r, wide.as_ref()), -1, ErrorClass::Invalid);
            for len in [0, 4, 7, 39, 40, 41, 64, 65, usize::MAX] {
                assert_failed(
                    Tree::lookup_prefix(r, wide.as_ref(), len),
                    -1,
                    ErrorClass::Invalid,
                );
            }
        }
        assert_eq!(hex(&Tree::lookup(r, id(NESTED).as_ref()).unwrap()), NESTED);
    }

    /// Trees the database holds but libgit2 cannot parse fail their lookup, through both
    /// constructors, and the half-parsed tree is freed with the entries it had (ASan: bad
    /// free; LSan: leak). The empty tree and a one-entry tree beside them are found, and
    /// their entries read.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn unparsable_trees_fail_and_leave_nothing_behind() {
        let repo = TempRepo::new("tree-unparsable");
        let r = repository(&repo);
        let blob = write_raw(&repo, b"leaf\n", ffi::GIT_OBJECT_BLOB);
        let mut entry = b"100644 leaf\0".to_vec();
        entry.extend(&blob.as_ref().id()[..20]);
        let mut corrupt: Vec<Vec<u8>> = vec![
            b"100644 one\0short".to_vec(),
            b"100644 a\0aaaaaaaaaaaaaaaaaaaa100644 b".to_vec(),
            b"10x644 a\0aaaaaaaaaaaaaaaaaaaa".to_vec(),
            b"100644a\0aaaaaaaaaaaaaaaaaaaa".to_vec(),
            b"100644 \0aaaaaaaaaaaaaaaaaaaa".to_vec(),
            b"100644 no-nul".to_vec(),
        ];
        // Many good entries, then a truncated one: the entries array has grown.
        let mut long = Vec::new();
        for i in 0..40 {
            long.extend(format!("100644 f{i:02}\0").as_bytes());
            long.extend(&blob.as_ref().id()[..20]);
        }
        long.extend(b"100644 last\0trunc");
        corrupt.push(long);
        for data in &corrupt {
            let id = write_raw(&repo, data, ffi::GIT_OBJECT_TREE);
            for result in [
                Tree::lookup(r, id.as_ref()),
                Tree::lookup_prefix(r, id.as_ref(), 12),
            ] {
                let e = result.unwrap_err();
                assert_eq!(e.raw_code(), -1, "{e:?}");
            }
        }
        let empty = write_raw(&repo, b"", ffi::GIT_OBJECT_TREE);
        assert!(entries(&Tree::lookup(r, empty.as_ref()).unwrap()).is_empty());
        let one = write_raw(&repo, &entry, ffi::GIT_OBJECT_TREE);
        let tree = Tree::lookup_prefix(r, one.as_ref(), 40).unwrap();
        assert_eq!(
            entries(&tree),
            [(c"leaf".to_owned(), ffi::GIT_FILEMODE_BLOB)]
        );
    }

    /// libgit2 copies the id it is given: each one is overwritten and freed right after
    /// its lookup, before the trees are read back and looked up again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_retain_no_id() {
        let repo = open_testrepo();
        let mut held = Vec::new();
        for (i, hex_id) in TREES.into_iter().enumerate() {
            let mut probe = Box::new(id(hex_id));
            let tree = if i % 2 == 0 {
                Tree::lookup(repo.as_ref(), probe.as_ref().as_ref())
            } else {
                Tree::lookup_prefix(repo.as_ref(), probe.as_ref().as_ref(), 10)
            }
            .unwrap();
            probe.as_mut().as_mut().set_id([0xEE; 32]);
            drop(probe);
            held.push((hex_id, tree));
        }
        for (hex_id, tree) in &held {
            assert_eq!(hex(tree), *hex_id);
            let again = Tree::lookup(repo.as_ref(), id(hex_id).as_ref()).unwrap();
            assert!(TreeArc::ptr_eq(&again, tree));
        }
    }

    /// A tree keeps a bare pointer to its repository, which a lookup by path through its
    /// subtrees follows, while Rust reborrows the repository between the lookups.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn trees_reach_their_repository_across_reborrows() {
        let repo = open_testrepo();
        let tree = Tree::lookup(repo.as_ref(), id(NESTED).as_ref()).unwrap();
        let viewed = Object::lookup(repo.as_ref(), id(NESTED).as_ref(), None).unwrap();
        for round in 0..4 {
            let _ = repo.as_ref();
            let deep = TreeEntry::bypath(tree.as_ref(), c"ab/de/fgh/1.txt").unwrap();
            let r = repo.as_ref();
            let through_view = TreeRef::from_object(viewed.as_ref()).unwrap();
            let again = TreeEntry::bypath(through_view, c"ab/de/fgh/1.txt").unwrap();
            assert_eq!(entry_view(&deep), entry_view(&again), "round {round}");
            let blob = tree
                .as_ref()
                .as_object()
                .lookup_bypath(c"ab/de/fgh/1.txt", Some(ObjectType::Blob))
                .unwrap();
            assert_eq!(object_view(&blob).0, entry_view(&deep).1);
            let ab = entry_view(&TreeEntry::bypath(tree.as_ref(), c"ab").unwrap()).1;
            let sub = Tree::lookup(r, id(&ab).as_ref()).unwrap();
            assert_eq!(entries(&sub).len(), 3);
        }
    }

    /// With a cache limit of one byte, the cache evicts trees as fast as they come, while
    /// owners still hold them: the owners' references keep them, and their entries,
    /// alive.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn owners_outlive_cache_eviction() {
        isolated(
            concat!(module_path!(), "::owners_outlive_cache_eviction"),
            || {
                let repo = open_testrepo();
                set_cache_max_size(1);
                for round in 0..4 {
                    let held: Vec<_> = TREES
                        .iter()
                        .map(|h| Tree::lookup(repo.as_ref(), id(h).as_ref()).unwrap())
                        .collect();
                    let expected: Vec<_> = held.iter().map(entries).collect();
                    let clones: Vec<_> = held.iter().rev().map(|t| t.as_ref().to_tree()).collect();
                    drop(held);
                    for (tree, e) in clones.iter().rev().zip(&expected) {
                        assert_eq!(&entries(tree), e, "round {round}");
                    }
                }
            },
        );
    }

    /// With caching off, every lookup parses a fresh tree, which its owners alone count;
    /// dropping the last frees it, with its entries and raw data.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn without_a_cache_each_lookup_is_a_fresh_tree() {
        isolated(
            concat!(
                module_path!(),
                "::without_a_cache_each_lookup_is_a_fresh_tree"
            ),
            || {
                let repo = open_testrepo();
                enable_caching(false);
                let first = Tree::lookup(repo.as_ref(), id(NESTED).as_ref()).unwrap();
                let second = Tree::lookup_prefix(repo.as_ref(), id(NESTED).as_ref(), 8).unwrap();
                assert!(!TreeArc::ptr_eq(&first, &second));
                let expected = entries(&first);
                let clone = first.clone();
                drop(first);
                assert_eq!(entries(&clone), expected);
                drop(second);
                assert_eq!(entries(&clone), expected);
            },
        );
    }

    /// Each thread opens its own repository and looks its trees up, clones, converts and
    /// reads them: the shared machinery underneath must not race.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn threads_with_their_own_repositories() {
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let repo = open_testrepo();
                    for round in 0..8 {
                        let hex_id = TREES[(t + round) % TREES.len()];
                        let tree = Tree::lookup(repo.as_ref(), id(hex_id).as_ref()).unwrap();
                        let object = Tree::into_object(tree.clone());
                        let back = Tree::from_object(object).unwrap();
                        assert!(TreeArc::ptr_eq(&tree, &back));
                        assert_eq!(hex(&tree), hex_id);
                    }
                });
            }
        });
    }

    /// The handles: a shared handle is `Copy`, viewed as an object it is the same
    /// object, and it formats opaquely.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shared_handles_coexist() {
        let repo = open_testrepo();
        let tree = Tree::lookup(repo.as_ref(), id(NESTED).as_ref()).unwrap();
        let a = tree.as_ref();
        let b = a;
        assert!(TreeArc::ptr_eq(&a.to_tree(), &b.to_tree()));
        let object: ObjectArc<'_> = a.as_object().to_object();
        assert_eq!(object_view(&object).0, NESTED);
        assert_eq!(format!("{b:?}"), "Tree { .. }");
    }
}

mod tree_reads {
    //! UB tests for `git2/tree.h`'s tree reading routines: the id and repository of a
    //! tree, its entries by index, name and id and in order, the walk over it and its
    //! subtrees, and a tree builder's filter (`remove_if`).
    //!
    //! A tree's entries are an array the tree owns, their names pointing into the raw
    //! object data it keeps: every lent entry, name and id is read in full while only one
    //! owner keeps the tree, and copied out to outlive it and its repository (ASan: a
    //! read past or after the tree's storage). Lookups by name probe names of every
    //! length around the 16 bits a tree stores one in, past it included, and lookups by
    //! id probe ids of every type byte and tail (ASan: a read past the name or the id).
    //! The getters returning nothing on a miss are checked at both ends of the index
    //! range.
    //!
    //! A walk looks each subtree up in the repository and frees it once walked: the
    //! closure copies what it keeps, walks the same tree again from inside, looks other
    //! trees up and drops clones of the walked tree meanwhile, under a cache that evicts
    //! as fast as trees come (ASan: an entry or root used after its subtree is freed). A
    //! panic resumes after the walk, which leaves the tree usable, and a walk failing on
    //! a missing or mistyped subtree leaves nothing behind (LSan). A filter frees each
    //! entry it removes right after its call: the closure copies entries out, panics,
    //! and drives another builder meanwhile (ASan, LSan). Walks and filters run on
    //! threads, each over its own repository (TSan).
    //!
    //! An entry, id or repository outliving what keeps it alive, a closure reaching the
    //! builder it filters, or the repository's exclusive handle from a walk do not
    //! compile (`compile_fail/` and doctests). Every test calls libgit2, so Miri skips
    //! them all.

    use std::ffi::{CStr, CString};
    use std::ops::ControlFlow;
    use std::panic::{self, AssertUnwindSafe};
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::{FileMode, ObjectType};
    use libgit2::libgit2::object::tree::{
        Tree, TreeArc, TreeBuilder, TreeBuilderBox, TreeBuilderFilter, TreeEntryBox, TreeEntryRef,
        TreeRef, WalkAction, WalkCallback, WalkMode,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::RepositoryBox;
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode, last};

    use crate::fixture::object_and_tree_entry::{
        entry_view, open_testrepo, repository, set_cache_max_size, write_raw,
    };
    use crate::fixture::tree::{entries, id};
    use crate::fixture::{TempRepo, isolated};

    /// Trees of `testrepo.git`: `HEAD`'s, commit `763d71a`'s (nesting `ab/de/fgh`), its
    /// subtrees `ab`, `ab/c`, `ab/de` and `ab/de/fgh`, and more commits' trees.
    const TREES: [&str; 9] = [
        "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162",
        "ae90f12eea699729ed24555e40b9fd669da12a12",
        "f1425cef211cc08caa31e7b545ffb232acb098c3",
        "9a03079b8a8ee85a0bee58bf9be3da8b62414ed4",
        "b6361fc6a97178d8fc8639fdeed71c775ab52593",
        "3259a6bd5b57fb9c1281bb7ed3167b50f224cb54",
        "53fc32d17276939fc79ed05badaef2db09990016",
        "1810dff58d8a660512d4832e740f692884338ccd",
        "181037049a54a1eb5fab404658a3a250b44335d7",
    ];
    const NESTED: &str = "ae90f12eea699729ed24555e40b9fd669da12a12";

    fn sha1(hex: &str) -> Oid {
        Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap()
    }

    /// An entry's name, hex id and raw mode, read through its getters.
    fn view(entry: TreeEntryRef<'_>) -> (CString, String, u32) {
        let id = entry.id().to_hex_string().unwrap();
        (entry.name().to_owned(), id, entry.filemode_raw())
    }

    /// Every getter agrees with the iterator on every tree, each lent entry read in full
    /// with only one owner left; the copies outlive the tree and the repository.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_lend_entries_for_the_tree_borrow() {
        let mut copies: Vec<(TreeEntryBox, (CString, String, u32))> = Vec::new();
        {
            let repo = open_testrepo();
            for hex in TREES {
                let tree = Tree::lookup(repo.as_ref(), sha1(hex).as_ref()).unwrap();
                // A second owner, dropped before the reads: the first keeps the tree.
                let other = tree.clone();
                let t: TreeRef<'_, '_> = other.as_ref();
                assert_eq!(t.id().to_hex_string().unwrap(), hex);
                assert_eq!(t.owner().path(), repo.as_ref().path());
                drop(other);
                let t = tree.as_ref();
                let all: Vec<_> = t.iter().collect();
                assert_eq!(all.len(), t.len());
                assert!(!t.is_empty());
                for (i, entry) in all.iter().enumerate() {
                    let seen = view(*entry);
                    assert_eq!(t.get(i).map(view), Some(seen.clone()));
                    let named = t.get_by_name(entry.name()).unwrap();
                    assert_eq!(view(named), seen);
                    // The first entry naming the same object.
                    let first = all.iter().position(|e| e.id() == entry.id()).unwrap();
                    assert_eq!(view(t.get_by_id(entry.id()).unwrap()), view(all[first]));
                    copies.push((entry.to_tree_entry().unwrap(), seen));
                }
                for i in [t.len(), t.len() + 1, usize::MAX / 2, usize::MAX] {
                    assert!(t.get(i).is_none());
                }
                assert_eq!(t.iter().rev().count(), t.len());
                assert_eq!(entries(&tree).len(), t.len());
                let owner = t.owner();
                drop(tree);
                // The repository outlives the tree it was read from.
                assert_eq!(owner.path(), repo.as_ref().path());
            }
        }
        for (copy, seen) in &copies {
            assert_eq!(view(copy.as_ref()), *seen);
            assert_eq!(entry_view(copy), *seen);
        }
    }

    /// Names around every length a tree stores, past 65535 bytes included, where libgit2
    /// truncates the length it searches with: each lookup reads within the name, finds
    /// only the entry of the very same bytes, and an overlong one leaves libgit2's
    /// "path too long" error.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn names_of_every_length_are_looked_up_in_bounds() {
        let temp = TempRepo::new("tree-reads-names");
        let lengths = [1usize, 2, 255, 256, 4096, 65534, 65535];
        let name = |len: usize, last: u8| {
            let mut bytes = vec![b'n'; len];
            *bytes.last_mut().unwrap() = last;
            CString::new(bytes).unwrap()
        };
        let mut builder = TreeBuilder::new(repository(&temp)).unwrap();
        for (i, &len) in lengths.iter().enumerate() {
            let entry_id = id(i as u8 + 1);
            builder
                .as_mut()
                .insert(&name(len, b'x'), entry_id.as_ref(), FileMode::Commit)
                .unwrap();
        }
        let written = builder.as_mut().write().unwrap();
        drop(builder);
        let tree = Tree::lookup(repository(&temp), written.as_ref()).unwrap();
        let t = tree.as_ref();
        assert_eq!(t.len(), lengths.len());
        for &len in &lengths {
            let exact = name(len, b'x');
            let found = t.get_by_name(&exact).unwrap();
            assert_eq!(found.name().count_bytes(), len);
            assert!(t.get_by_name(&name(len, b'y')).is_none());
            // The entry's own bytes, one longer or one shorter: no entry of the others'.
            let mut longer = exact.as_bytes().to_vec();
            longer.push(b'x');
            assert!(t.get_by_name(&CString::new(longer).unwrap()).is_none());
            let shorter = &exact.as_bytes()[..len - 1];
            assert!(t.get_by_name(&CString::new(shorter).unwrap()).is_none());
        }
        assert!(t.get_by_name(c"").is_none());
        // 65536 bytes search with a 16-bit length of 0, and the others wrap likewise.
        for len in [65536, 65537, 65536 + 255, 131071, 131072, 200_000] {
            assert!(t.get_by_name(&name(len, b'x')).is_none(), "{len}");
            let e = last();
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
            assert_eq!(e.message(), c"tree entry path too long");
        }
    }

    /// Ids of every type byte, and SHA-1 ids with a non-zero tail, are compared within
    /// the `git_oid` and match nothing; the tree's own ids match their first entry.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn ids_of_every_kind_are_compared_in_bounds() {
        let repo = open_testrepo();
        let tree = Tree::lookup(repo.as_ref(), sha1(NESTED).as_ref()).unwrap();
        let t = tree.as_ref();
        for entry in t {
            let raw = entry.id().id();
            let mut probe = Oid::from_raw(&raw[..20], OidType::Sha1).unwrap();
            assert!(t.get_by_id(probe.as_ref()).is_some());
            let mut tail = raw;
            tail[31] = 1;
            probe.as_mut().set_id(tail);
            assert!(t.get_by_id(probe.as_ref()).is_none());
            probe.as_mut().set_id(raw);
            for kind in [0u8, 2, 3, 0x7F, u8::MAX] {
                probe.as_mut().set_raw_kind(kind);
                assert!(t.get_by_id(probe.as_ref()).is_none(), "{kind}");
            }
            let wide = Oid::from_raw(&raw, OidType::Sha256).unwrap();
            assert!(t.get_by_id(wide.as_ref()).is_none());
        }
        for probe in [
            Oid::zero(OidType::Sha1),
            Oid::zero(OidType::Sha256),
            id(0xEE),
        ] {
            assert!(t.get_by_id(probe.as_ref()).is_none());
        }
    }

    /// One walk's visits: the path under the root, and a copy of the entry.
    type Visit = (String, TreeEntryBox);

    fn path_of(root: &CStr, entry: TreeEntryRef<'_>) -> String {
        format!(
            "{}{}",
            root.to_str().unwrap(),
            entry.name().to_str().unwrap()
        )
    }

    /// Walks `tree` in `mode`, copying each entry out, answering `answer`.
    fn walk_copying(
        tree: TreeRef<'_, '_>,
        mode: WalkMode,
        mut answer: impl FnMut(&str, TreeEntryRef<'_>) -> WalkAction,
    ) -> (ControlFlow<()>, Vec<Visit>) {
        let mut visits = Vec::new();
        let mut f = |root: &CStr, entry: TreeEntryRef<'_>| {
            let path = path_of(root, entry);
            visits.push((path.clone(), entry.to_tree_entry().unwrap()));
            answer(&path, entry)
        };
        let mut callback = WalkCallback::new(&mut f);
        let walked = tree.walk(mode, &mut callback).unwrap();
        drop(callback);
        (walked, visits)
    }

    /// Under a cache evicting as fast as trees come, a walk's subtrees are freed as soon
    /// as walked: the closure copies what it keeps, walks the tree again from inside,
    /// looks other trees up and drops clones of the walked tree. Stopping at every visit
    /// in both orders ends the walk there; the copies outlive the walks.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn walks_lend_entries_for_the_call() {
        isolated(
            concat!(module_path!(), "::walks_lend_entries_for_the_call"),
            || {
                let repo = open_testrepo();
                set_cache_max_size(1);
                let tree = Tree::lookup(repo.as_ref(), sha1(NESTED).as_ref()).unwrap();
                let mut kept = Vec::new();
                for mode in [WalkMode::PreOrder, WalkMode::PostOrder] {
                    let (walked, all) =
                        walk_copying(tree.as_ref(), mode, |_, _| WalkAction::Continue);
                    assert_eq!(walked, ControlFlow::Continue(()));
                    assert!(all.iter().any(|(p, _)| p == "ab/de/fgh/1.txt"));
                    for stop in 0..all.len() {
                        let mut n = 0;
                        let (walked, visits) = walk_copying(tree.as_ref(), mode, |_, _| {
                            n += 1;
                            if n > stop {
                                WalkAction::Stop
                            } else {
                                WalkAction::Continue
                            }
                        });
                        assert_eq!(walked, ControlFlow::Break(()));
                        assert_eq!(visits.len(), stop + 1);
                        kept.extend(visits);
                    }
                    kept.extend(all);
                }

                // From inside: a nested walk of the same tree, lookups and clones.
                let mut inner_visits = 0;
                let mut outer = |root: &CStr, entry: TreeEntryRef<'_>| {
                    let clone = tree.clone();
                    let mut inner = |r: &CStr, e: TreeEntryRef<'_>| {
                        inner_visits += 1;
                        let _ = path_of(r, e);
                        WalkAction::Skip
                    };
                    let mut nested = WalkCallback::new(&mut inner);
                    let walked = clone.as_ref().walk(WalkMode::PreOrder, &mut nested);
                    assert_eq!(walked.unwrap(), ControlFlow::Continue(()));
                    drop(clone);
                    if entry.kind() == ObjectType::Tree {
                        let sub = Tree::lookup(repo.as_ref(), entry.id()).unwrap();
                        assert_eq!(sub.as_ref().id().id(), entry.id().id());
                        assert!(!sub.as_ref().is_empty());
                    }
                    let _ = path_of(root, entry);
                    WalkAction::Continue
                };
                let mut callback = WalkCallback::new(&mut outer);
                let walked = tree.as_ref().walk(WalkMode::PostOrder, &mut callback);
                assert_eq!(walked.unwrap(), ControlFlow::Continue(()));
                drop(callback);
                assert!(inner_visits > 0);
                drop(tree);
                drop(repo);
                for (path, entry) in &kept {
                    assert!(path.ends_with(entry.as_ref().name().to_str().unwrap()));
                }
            },
        );
    }

    /// A panic of the closure stops the walk, resumes once libgit2 has returned, and
    /// leaves the tree and the callback usable; skips keep the walk out of subtrees,
    /// which are then never looked up.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn walks_resume_panics_and_skip() {
        let repo = open_testrepo();
        let tree = Tree::lookup(repo.as_ref(), sha1(NESTED).as_ref()).unwrap();
        for mode in [WalkMode::PreOrder, WalkMode::PostOrder] {
            let mut calls = 0;
            let mut f = |_: &CStr, entry: TreeEntryRef<'_>| {
                calls += 1;
                if calls == 3 {
                    panic!("walk panicked at {:?}", entry.name());
                }
                WalkAction::Continue
            };
            let mut callback = WalkCallback::new(&mut f);
            let caught =
                panic::catch_unwind(AssertUnwindSafe(|| tree.as_ref().walk(mode, &mut callback)));
            assert!(caught.is_err());
            assert!(callback.stopped());
            let walked = tree.as_ref().walk(mode, &mut callback).unwrap();
            assert_eq!(walked, ControlFlow::Continue(()));
            assert!(!callback.stopped());
            drop(callback);
            assert!(calls > 3);
        }
        let (walked, visits) = walk_copying(tree.as_ref(), WalkMode::PreOrder, |_, entry| {
            if entry.kind() == ObjectType::Tree {
                WalkAction::Skip
            } else {
                WalkAction::Continue
            }
        });
        assert_eq!(walked, ControlFlow::Continue(()));
        assert_eq!(visits.len(), tree.as_ref().len());
        assert!(visits.iter().all(|(path, _)| !path.contains('/')));
    }

    /// Walks reaching a subtree the object database lacks, or whose object is a blob,
    /// fail with the lookup's error after the visits before it, leaving nothing behind
    /// (LSan).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn walks_into_missing_or_mistyped_subtrees_fail_cleanly() {
        let temp = TempRepo::new("tree-reads-broken");
        let blob = write_raw(&temp, b"leaf\n", ffi::GIT_OBJECT_BLOB);
        for target in [[0x77u8; 20].as_slice(), &blob.as_ref().id()[..20]] {
            let mut data = b"160000 a\0".to_vec();
            data.extend_from_slice(&[0x11; 20]);
            data.extend_from_slice(b"40000 broken\0");
            data.extend_from_slice(target);
            let broken = write_raw(&temp, &data, ffi::GIT_OBJECT_TREE);
            let tree = Tree::lookup(repository(&temp), broken.as_ref()).unwrap();
            for mode in [WalkMode::PreOrder, WalkMode::PostOrder] {
                let mut visits = Vec::new();
                let mut f = |root: &CStr, entry: TreeEntryRef<'_>| {
                    visits.push(path_of(root, entry));
                    WalkAction::Continue
                };
                let mut callback = WalkCallback::new(&mut f);
                let e = tree.as_ref().walk(mode, &mut callback).unwrap_err();
                assert!(!callback.stopped());
                drop(callback);
                assert_eq!(e.code(), Some(ErrorCode::NotFound), "{e:?}");
                let want: &[&str] = match mode {
                    WalkMode::PreOrder => &["a", "broken"],
                    _ => &["a"],
                };
                assert_eq!(visits, want);
            }
        }
    }

    /// A builder seeded with `TREES[1]`'s entries, of `repo`.
    fn seeded(repo: &RepositoryBox) -> TreeBuilderBox<'_> {
        let tree = Tree::lookup(repo.as_ref(), sha1(NESTED).as_ref()).unwrap();
        TreeBuilder::from_tree(repo.as_ref(), tree.as_ref()).unwrap()
    }

    /// Whether the entry's id starts with an even byte: what the filters below remove.
    fn even(entry: TreeEntryRef<'_>) -> bool {
        entry.id().id()[0].is_multiple_of(2)
    }

    /// Filters free each entry they remove right after its call, whatever the closure
    /// copies out or drives meanwhile (another builder); a panicking closure keeps the
    /// rest and resumes after the filter.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn filters_free_what_they_remove() {
        let repo = open_testrepo();
        let mut builder = seeded(&repo);
        let all = builder.as_ref().len();
        let mut other = seeded(&repo);
        let mut copies = Vec::new();
        let mut f = |entry: TreeEntryRef<'_>| {
            copies.push(entry.to_tree_entry().unwrap());
            let name = entry.name().to_owned();
            other.as_mut().remove(&name).unwrap();
            even(entry)
        };
        builder
            .as_mut()
            .remove_if(&mut TreeBuilderFilter::new(&mut f));
        assert_eq!(copies.len(), all);
        assert!(other.as_ref().is_empty());
        for copy in &copies {
            let kept = builder.as_ref().get(copy.as_ref().name()).is_some();
            assert_eq!(kept, !even(copy.as_ref()));
        }
        let removed = copies.iter().filter(|c| even(c.as_ref())).count();
        assert_eq!(builder.as_ref().len(), all - removed);

        // A panic on the second call: the first entry goes, the rest stay.
        let mut builder = seeded(&repo);
        let mut calls = 0;
        let mut boom = |_: TreeEntryRef<'_>| {
            calls += 1;
            assert!(calls <= 2, "called after the panic");
            if calls == 2 {
                panic!("filter panicked");
            }
            true
        };
        let mut filter = TreeBuilderFilter::new(&mut boom);
        let caught =
            panic::catch_unwind(AssertUnwindSafe(|| builder.as_mut().remove_if(&mut filter)));
        assert!(caught.is_err());
        drop(filter);
        assert_eq!(builder.as_ref().len(), all - 1);

        // Removing everything, twice: the second filter sees nothing.
        let mut everything = |_: TreeEntryRef<'_>| true;
        let mut filter = TreeBuilderFilter::new(&mut everything);
        builder.as_mut().remove_if(&mut filter);
        builder.as_mut().remove_if(&mut filter);
        drop(filter);
        assert!(builder.as_ref().is_empty());
    }

    /// The written tree of a filtered builder lists what the filter kept.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn filtered_builders_write_what_they_kept() {
        let temp = TempRepo::new("tree-reads-filter-write");
        let mut builder = TreeBuilder::new(repository(&temp)).unwrap();
        for i in 0..32u8 {
            let name = CString::new(format!("e{i:02}")).unwrap();
            builder
                .as_mut()
                .insert(&name, id(i + 1).as_ref(), FileMode::Commit)
                .unwrap();
        }
        let mut odd = |entry: TreeEntryRef<'_>| entry.name().to_bytes()[2] % 2 == 1;
        builder
            .as_mut()
            .remove_if(&mut TreeBuilderFilter::new(&mut odd));
        let written = builder.as_mut().write().unwrap();
        drop(builder);
        let tree: TreeArc<'_> = Tree::lookup(repository(&temp), written.as_ref()).unwrap();
        let names: Vec<_> = tree.as_ref().iter().map(|e| e.name().to_owned()).collect();
        let want: Vec<_> = (0..32)
            .step_by(2)
            .map(|i| CString::new(format!("e{i:02}")).unwrap())
            .collect();
        assert_eq!(names, want);
    }

    /// Walks, reads and filters on threads, each over its own repository.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reads_walks_and_filters_on_threads() {
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let repo = open_testrepo();
                    for round in 0..4 {
                        let hex = TREES[(t + round) % TREES.len()];
                        let tree = Tree::lookup(repo.as_ref(), sha1(hex).as_ref()).unwrap();
                        let names: Vec<_> = tree.as_ref().iter().map(view).collect();
                        let mode = [WalkMode::PreOrder, WalkMode::PostOrder][round % 2];
                        let (walked, visits) =
                            walk_copying(tree.as_ref(), mode, |_, _| WalkAction::Continue);
                        assert_eq!(walked, ControlFlow::Continue(()));
                        assert!(visits.len() >= names.len());
                        let mut builder =
                            TreeBuilder::from_tree(repo.as_ref(), tree.as_ref()).unwrap();
                        let mut first = true;
                        let mut f = |_: TreeEntryRef<'_>| std::mem::take(&mut first);
                        builder
                            .as_mut()
                            .remove_if(&mut TreeBuilderFilter::new(&mut f));
                        assert_eq!(builder.as_ref().len(), names.len() - 1);
                    }
                });
            }
        });
    }
}

mod updates {
    //! UB tests for `Tree::create_updated` (`git_tree_create_updated`).
    //!
    //! libgit2 reads the caller's array of updates in place, sorting pointers to its
    //! elements, and each update's path through a bare pointer, for the call only: the
    //! paths sit in exact-length heap buffers (ASan: a read past their NUL) that the
    //! tests rewrite and free right after the call (BSan: a retained foreign pointer),
    //! and the arrays range from empty to thousands of updates. It counts a reference
    //! on the baseline tree and builds a builder and a subtree reference per level of
    //! each path, all released whatever the outcome (ASan, LSan): every refusal it
    //! makes (duplicates, D/F conflicts, missing removals, invalid names, missing
    //! objects) runs, beside the inputs the wrapper refuses before C (an id or a
    //! baseline of another object id type, which C formats out of bounds, and an
    //! over-long path component). The baseline is dropped before the result is used.
    //! Each thread updates trees of a repository of its own, as a repository is not
    //! `Sync` (TSan). Every test calls libgit2, so Miri skips them all.

    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::thread;

    use libgit2::libgit2::core::types::FileMode;
    use libgit2::libgit2::object::tree::{Tree, TreeBuilder, TreeEntry, TreeUpdate, UpdateAction};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{
        InitFlag, InitOptions, Repository, RepositoryBox,
    };
    use libgit2::libgit2::util::errors::ErrorClass;

    use crate::fixture::init;

    /// Commit `763d71a`'s tree in `testrepo.git`: `README`, `ab/` three levels deep.
    const BASE: &str = "ae90f12eea699729ed24555e40b9fd669da12a12";
    const README: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";
    const OTHER: &str = "a8233120f6ad708f843d861ce2b7228ec4e3dec6";

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
            let path = std::env::temp_dir().join(format!(
                "crustify-tree-update-ub-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            let source = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/c/tests/resources/testrepo.git"
            );
            copy_dir(Path::new(source), &path);
            let repo = Repository::open(&CString::new(path.as_os_str().as_bytes()).unwrap());
            Self {
                path,
                repo: Some(repo.unwrap()),
            }
        }

        fn repo(&self) -> &RepositoryBox {
            self.repo.as_ref().unwrap()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn id(hex: &str) -> Oid {
        Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap()
    }

    /// Overwrites a string's bytes in place, keeping its length, then frees it.
    fn scribble(s: CString) {
        let mut bytes = s.into_bytes();
        bytes.fill(b'#');
        drop(bytes);
    }

    /// Updates over paths in heap buffers of their exact length, rewritten and freed
    /// after each call; the baseline is dropped before the written tree is read.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn paths_are_read_for_the_call_only() {
        let sandbox = Sandbox::new("paths");
        let repo = sandbox.repo();
        let (readme, other) = (id(README), id(OTHER));
        for n in [0usize, 1, 2, 16, 300] {
            let paths: Vec<CString> = (0..n)
                .map(|i| {
                    CString::new(format!("d{}/{}/f{i}", i % 3, "x".repeat(i % 7 + 1))).unwrap()
                })
                .collect();
            let mut updates: Vec<TreeUpdate<'_>> = paths
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let blob = if i % 2 == 0 { &readme } else { &other };
                    TreeUpdate::upsert(p, blob.as_ref(), FileMode::Blob)
                })
                .collect();
            // Reversed: libgit2 sorts its own pointers into the array.
            updates.reverse();
            let base = Tree::lookup(repo.as_ref(), id(BASE).as_ref()).unwrap();
            let written = Tree::create_updated(repo.as_ref(), Some(base.as_ref()), &updates);
            drop(base);
            drop(updates);
            let written = written.unwrap();
            let tree = Tree::lookup(repo.as_ref(), written.as_ref()).unwrap();
            for p in paths {
                assert!(TreeEntry::bypath(tree.as_ref(), &p).is_ok());
                scribble(p);
            }
        }
    }

    /// Every refusal, libgit2's and the wrapper's, releases what the call built.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[cfg_attr(
        bsan,
        ignore = "its large inputs outlast the BorrowSanitizer gate, over an hour alone; the other variants run it"
    )]
    fn refusals_release_everything() {
        let sandbox = Sandbox::new("refusals");
        let repo = sandbox.repo();
        let (readme, other) = (id(README), id(OTHER));
        let missing = id("dededededededededededededededededededede");
        let wide = Oid::from_raw(&[7; 32], OidType::Sha256).unwrap();
        let long = CString::new(format!("dir/{}", "n".repeat(70_000))).unwrap();
        let sets: Vec<Vec<TreeUpdate<'_>>> = vec![
            vec![
                TreeUpdate::upsert(c"x", readme.as_ref(), FileMode::Blob),
                TreeUpdate::upsert(c"x", other.as_ref(), FileMode::Blob),
            ],
            vec![TreeUpdate::upsert(
                c"README/x",
                readme.as_ref(),
                FileMode::Blob,
            )],
            vec![TreeUpdate::upsert(c"ab", readme.as_ref(), FileMode::Blob)],
            vec![TreeUpdate::remove(c"ab/de/fgh/missing")],
            vec![
                TreeUpdate::upsert(c"ab/de/fgh/new", readme.as_ref(), FileMode::Blob),
                TreeUpdate::upsert(c"ab/de/fgh/zz/.git", readme.as_ref(), FileMode::Blob),
            ],
            vec![TreeUpdate::upsert(c"a//b", readme.as_ref(), FileMode::Blob)],
            vec![TreeUpdate::upsert(
                c"new/deep/x",
                missing.as_ref(),
                FileMode::Blob,
            )],
            vec![TreeUpdate::upsert(
                c"new",
                other.as_ref(),
                FileMode::Unreadable,
            )],
            vec![TreeUpdate::upsert(c"wide/x", wide.as_ref(), FileMode::Blob)],
            vec![TreeUpdate::upsert(&long, readme.as_ref(), FileMode::Commit)],
        ];
        for (i, updates) in sets.iter().enumerate() {
            let base = Tree::lookup(repo.as_ref(), id(BASE).as_ref()).unwrap();
            let result = Tree::create_updated(repo.as_ref(), Some(base.as_ref()), updates);
            assert!(result.is_err(), "set {i}");
            let unbased = Tree::create_updated(repo.as_ref(), None, updates);
            // Without a baseline, nothing is in the way of sets 1 and 2.
            assert_eq!(unbased.is_ok(), matches!(i, 1 | 2), "set {i}");
        }
        // A removal reads no id, whatever its type.
        let mut removal = TreeUpdate::remove(c"README");
        removal.as_mut().set_id(wide.as_ref());
        removal.as_mut().set_action(UpdateAction::Remove);
        let base = Tree::lookup(repo.as_ref(), id(BASE).as_ref()).unwrap();
        assert!(Tree::create_updated(repo.as_ref(), Some(base.as_ref()), &[removal]).is_ok());
    }

    /// A baseline of another object id type is refused before libgit2 would look its
    /// subtrees up in a repository of the other type; one of the same type but another
    /// repository is looked up in the repository given.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn baselines_of_other_repositories_stay_in_bounds() {
        let sandbox = Sandbox::new("foreign-base");
        let repo = sandbox.repo();
        let path = sandbox.path.with_extension("sha256");
        let _ = std::fs::remove_dir_all(&path);
        let mut opts = InitOptions::new();
        opts.as_mut().set_flags(InitFlag::Bare | InitFlag::Mkdir);
        opts.as_mut().set_oid_type(Some(OidType::Sha256));
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let wide = Repository::init_ext(&c_path, opts.as_mut()).unwrap();
        let mut builder = TreeBuilder::new(wide.as_ref()).unwrap();
        let empty = builder.as_mut().write().unwrap();
        drop(builder);
        let wide_tree = Tree::lookup(wide.as_ref(), empty.as_ref()).unwrap();
        let e = Tree::create_updated(repo.as_ref(), Some(wide_tree.as_ref()), &[]).unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        let readme = id(README);
        let updates = [TreeUpdate::upsert(c"a/b", readme.as_ref(), FileMode::Blob)];
        let e = Tree::create_updated(wide.as_ref(), None, &updates).unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));

        let other = Sandbox::new("other-base");
        let base = Tree::lookup(other.repo().as_ref(), id(BASE).as_ref()).unwrap();
        let updates = [TreeUpdate::upsert(
            c"ab/c/new",
            readme.as_ref(),
            FileMode::Blob,
        )];
        let written = Tree::create_updated(repo.as_ref(), Some(base.as_ref()), &updates).unwrap();
        drop(base);
        drop(other);
        assert!(Tree::lookup(repo.as_ref(), written.as_ref()).is_ok());
        drop(wide_tree);
        drop(wide);
        let _ = std::fs::remove_dir_all(&path);
    }

    /// Threads update trees at once, each in a repository of its own.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn updates_on_threads() {
        let sandboxes: Vec<Sandbox> = (0..4)
            .map(|i| Sandbox::new(&format!("thread-{i}")))
            .collect();
        let paths: Vec<CString> = sandboxes
            .iter()
            .map(|s| CString::new(s.path.as_os_str().as_bytes()).unwrap())
            .collect();
        thread::scope(|s| {
            for (t, path) in paths.iter().enumerate() {
                s.spawn(move || {
                    let repo = Repository::open(path).unwrap();
                    let readme = id(README);
                    for k in 0..8 {
                        let name = CString::new(format!("t{t}/k{k}/file")).unwrap();
                        let updates = [
                            TreeUpdate::upsert(&name, readme.as_ref(), FileMode::Blob),
                            TreeUpdate::remove(c"README"),
                        ];
                        let base = Tree::lookup(repo.as_ref(), id(BASE).as_ref()).unwrap();
                        let written =
                            Tree::create_updated(repo.as_ref(), Some(base.as_ref()), &updates)
                                .unwrap();
                        assert!(Tree::lookup(repo.as_ref(), written.as_ref()).is_ok());
                    }
                });
            }
        });
    }
}
