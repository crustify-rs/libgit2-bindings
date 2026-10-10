//! UB tests for `git2/tree.h`: the enum `WalkMode` (`git_treewalk_mode`, unsigned).
//!
//! `WalkMode` is a fieldless enum over C's `git_treewalk_mode`. The only safe way in
//! from a raw value is `from_raw` / `try_from`, which must reject every value without
//! a variant rather than produce an invalid discriminant (which Miri and UBSan would
//! flag on the next read); an `as` cast to it does not compile (`compile_fail/`). It
//! owns and points at nothing and none of these tests calls libgit2, so they also run
//! under Miri; the sweep strides under BSan and Miri.
//!
//! The tree builder's tests are in [`builder`].

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
    //! handles, `clear`, and the `git_treebuilder_free` policy.
    //!
    //! Every entry a builder holds is its own allocation, which `clear` and the drop must
    //! each free exactly once, however the builder was filled (ASan: double free, use
    //! after free; LSan: leak). A builder keeps a bare pointer to its repository, which
    //! libgit2 follows while inserting and writing, across the Rust reborrows of that
    //! repository made in between (BSan: a retained foreign pointer). Builders are not
    //! `Send`, so the threaded test gives each thread its own repository (TSan: the
    //! runtime pin and allocator under concurrent constructors). Nothing here runs under
    //! Miri, which cannot call libgit2. Illegal uses are compile-fail doctests on the crate
    //! and files under `compile_fail/`: a builder outliving its repository or a handle its
    //! owner, aliased exclusive handles, sending or sharing a builder, a `clone`.

    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::Barrier;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::object::tree::{TreeBuilder, TreeBuilderBox, TreeBuilderMut};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    use crate::fixture::init;
    use crate::fixture::tree::{count, id, insert, mode_of, write};

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
}
