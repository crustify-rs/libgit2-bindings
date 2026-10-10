//! UB tests for `git2/tree.h`: the enum `WalkMode` (`git_treewalk_mode`, unsigned).
//!
//! `WalkMode` is a fieldless enum over C's `git_treewalk_mode`. The only safe way in
//! from a raw value is `from_raw` / `try_from`, which must reject every value without
//! a variant rather than produce an invalid discriminant (which Miri and UBSan would
//! flag on the next read); an `as` cast to it does not compile (`compile_fail/`). It
//! owns and points at nothing and none of these tests calls libgit2, so they also run
//! under Miri; the sweep strides under BSan and Miri.
//!
//! The tree builder's tests are in [`builder`], the tree entry's in [`tree_entry`], the
//! tree update's in [`tree_update`].

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
    //! subtree, a tree of the builder's own repository or of another, a SHA256
    //! repository's tree seeding a SHA1 repository's builder, and objects that are not
    //! trees, refused before libgit2 reads them as one. The builder keeps nothing of the
    //! tree, so dropping the tree right after the call is fine (ASan).
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

    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::Barrier;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::object::tree::{TreeBuilder, TreeBuilderBox, TreeBuilderMut};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
    use libgit2::libgit2::util::errors::ErrorClass;

    use crate::fixture::init;
    use crate::fixture::tree::{count, entries, id, insert, mode_of, write};

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
            assert_eq!(
                insert(builder, &name, &id(i + 1), ffi::GIT_FILEMODE_COMMIT),
                0
            );
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
            "{}/../../../tests/resources/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        Repository::open(&path).unwrap()
    }

    /// The object `hex` names in `repo`, of any type.
    fn lookup<'r>(repo: &'r RepositoryBox, hex: &str, kind: OidType) -> ObjectArc<'r> {
        let id = Oid::from_string(&CString::new(hex).unwrap(), kind).unwrap();
        Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap()
    }

    /// Seeds a builder of `target` with `source`, drops `source` at once, then checks
    /// that the builder holds a copy of each entry.
    fn seeded<'r>(target: &'r RepositoryBox, source: ObjectArc<'_>) -> TreeBuilderBox<'r> {
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

    /// An object that is not a tree is refused before libgit2 reads it as one (ASan would
    /// flag the read of a smaller object): the commit of `NESTED`, a blob and a tag, with
    /// a `GIT_ERROR_TREE` error; the object stays usable, and nothing is built.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn only_trees_seed_builders() {
        let source = open("testrepo.git");
        let target = TempRepo::new("not-trees");
        for hex in [
            "763d71aadf09a7951596c9746c024e7eece7c7af",
            "a8233120f6ad708f843d861ce2b7228ec4e3dec6",
            "b25fa35b38051e4ae45d4222e795f9df2e43f1d1",
        ] {
            let object = lookup(&source, hex, OidType::Sha1);
            for _ in 0..2 {
                let e =
                    TreeBuilder::from_tree(target.repo().as_ref(), object.as_ref()).unwrap_err();
                assert_eq!(e.class(), Some(ErrorClass::Tree), "{hex}");
                assert_eq!(e.message(), c"the object is not a tree");
            }
            let _ = object.as_ref().to_object();
        }
        let tree = lookup(&source, NESTED, OidType::Sha1);
        assert_eq!(count(&seeded(target.repo(), tree)), 4);
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
    //! Every test calls libgit2, so Miri skips them all.

    use std::ffi::{CStr, CString};
    use std::sync::Barrier;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::object::tree::{TreeEntry, TreeEntryBox};
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

    fn tree<'r>(repo: RepositoryRef<'r>) -> ObjectArc<'r> {
        let id = Oid::from_string(TREE, OidType::Sha1).unwrap();
        Object::lookup(repo, id.as_ref(), Some(ObjectType::Tree)).unwrap()
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
        let root = Object::lookup(repository(&repo), id.as_ref(), Some(ObjectType::Tree)).unwrap();
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
        raw.extend([0xde; 20]);
        let id = write_raw(&repo, &raw, ffi::GIT_OBJECT_TREE);
        let tree = Object::lookup(repository(&repo), id.as_ref(), Some(ObjectType::Tree)).unwrap();
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
            let e = tree.as_ref().lookup_bypath(path, None).unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Odb), "{path:?}: {e:?}");
        }
        let leaf = TreeEntry::bypath(tree.as_ref(), c"leaf").unwrap();
        assert_eq!(entry_view(&leaf).0.as_c_str(), c"leaf");
        let object = tree
            .as_ref()
            .lookup_bypath(c"leaf", Some(ObjectType::Blob))
            .unwrap();
        assert_eq!(object_view(&object).0, entry_view(&leaf).1);
        // The missing subtree, as an object: not found.
        let e = tree.as_ref().lookup_bypath(c"sub", None).unwrap_err();
        assert_eq!(e.code(), Some(ErrorCode::NotFound), "{e:?}");
    }

    /// An object that is not a tree is refused before libgit2 reads it as one: a
    /// commit, a blob and a tag (ASan would flag the read of a smaller object).
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
            for path in [c"README", c"", c"ab/de"] {
                let e = TreeEntry::bypath(object.as_ref(), path).unwrap_err();
                assert_eq!(e.code(), Some(ErrorCode::InvalidSpec), "{hex:?} {path:?}");
                assert_eq!(e.class(), Some(ErrorClass::Tree));
                assert_eq!(e.message(), c"the object is not a tree");
            }
        }
        // The tree itself is, and the same lookup through the peeled commit agrees.
        let root = tree(repo.as_ref());
        let commit_id =
            Oid::from_string(c"763d71aadf09a7951596c9746c024e7eece7c7af", OidType::Sha1).unwrap();
        let commit = Object::lookup(repo.as_ref(), commit_id.as_ref(), None).unwrap();
        let peeled = commit.as_ref().peel(Some(ObjectType::Tree)).unwrap();
        assert!(ObjectArc::ptr_eq(&peeled, &root));
        let a = TreeEntry::bypath(root.as_ref(), c"ab/c/3.txt").unwrap();
        let b = TreeEntry::bypath(peeled.as_ref(), c"ab/c/3.txt").unwrap();
        assert_eq!(entry_view(&a), entry_view(&b));
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
