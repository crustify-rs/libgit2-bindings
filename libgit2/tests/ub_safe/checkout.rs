//! UB tests for the safe API of `git2/checkout.h`: [`CheckoutOptions`], its handles,
//! and what its setters borrow.
//!
//! The options store pointers to the strings and path arrays their setters borrow, and
//! read them back through the getters; every source here is an allocation of exactly
//! its length (`CString`, `StrArrayView`'s pointer array, `StrArray::from_strs`'
//! copies), so ASan flags a read past one, and every borrowed source is released after
//! the options, exactly once (ASan, LSan). The handles reborrow while the options keep
//! those pointers (BSan). Options are built on several threads while another
//! initialises libgit2 for the first time: `git_checkout_options_init` touches no
//! global state, so nothing races (TSan). Illegal borrows are in `compile_fail/`, and
//! in the type's doctests.
//!
//! The baseline index is borrowed from its owner, an `IndexBox` or an `IndexArc`, and
//! the baseline tree from its `TreeArc`: real checkouts read them (ASan), libgit2 counts
//! a reference of its own on the tree for each call, which it must give back (LSan),
//! and the owners release them only after the options. The checkout routines and the
//! repository are not wrapped yet: [`repo`] stands in for them, as `fixture` does for
//! other missing constructors.
//!
//! Every test builds its options through `git_checkout_options_init`, so Miri skips
//! them; the inline unit tests run the accessors under Miri over a Rust-built value.

use std::ffi::{CStr, CString};
use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::core::global;
use libgit2::libgit2::core::strarray::{StrArray, StrArrayView};
use libgit2::libgit2::index::index::{Index, IndexArc};
use libgit2::libgit2::object::tree::TreeArc;
use libgit2::libgit2::workdir::checkout::{
    CheckoutOptions, CheckoutOptionsMut, CheckoutOptionsRef,
};

use crate::fixture::isolated;

/// `git_checkout_strategy_t`'s `GIT_CHECKOUT_FORCE`, from `git2/checkout.h`: the `-sys`
/// crate does not bind the enum.
const GIT_CHECKOUT_FORCE: u32 = 1 << 1;

/// `n` distinct strings of lengths 1, 2, 3, ...
fn strings(n: usize) -> Vec<CString> {
    (1..=n)
        .map(|i| CString::new(vec![b'a' + (i % 26) as u8; i]).unwrap())
        .collect()
}

/// Every byte the getters reach.
fn read_all(o: CheckoutOptionsRef<'_, '_>) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = o.paths().iter().map(|s| s.to_bytes().to_vec()).collect();
    for s in [
        o.target_directory(),
        o.ancestor_label(),
        o.our_label(),
        o.their_label(),
    ] {
        out.push(s.map_or(b"-".to_vec(), |s| s.to_bytes().to_vec()));
    }
    out.push(format!("{o:?}").into_bytes());
    out
}

fn set_strings<'o>(mut m: CheckoutOptionsMut<'_, 'o>, s: &'o [CString]) {
    m.set_target_directory(Some(&s[0]));
    m.set_ancestor_label(Some(&s[1]));
    m.set_our_label(Some(&s[2]));
    m.set_their_label(Some(&s[3]));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn defaults_then_every_setter() {
    let mut options = CheckoutOptions::new();
    let o = options.as_ref();
    assert_eq!(o.version(), 1);
    assert_eq!(
        (
            o.strategy(),
            o.dir_mode(),
            o.file_mode(),
            o.file_open_flags()
        ),
        (0, 0, 0, 0)
    );
    assert!(!o.disable_filters() && o.notify_flags() == 0 && o.paths().is_empty());
    assert!(o.baseline().is_none() && o.baseline_index().is_none());
    assert!(o.notify_callback().0.is_none() && o.progress_callback().0.is_none());
    assert!(o.perfdata_callback().0.is_none());
    assert_eq!((o.target_directory(), o.our_label()), (None, None));

    for round in 0..64u32 {
        let mut m = options.as_mut();
        m.set_strategy(round.rotate_left(round));
        m.set_disable_filters(round % 2 == 0);
        m.set_dir_mode(round * 7);
        m.set_file_mode(u32::MAX - round);
        m.set_file_open_flags(i32::MIN + round as i32);
        m.set_notify_flags(!round);
        let o = m.as_ref();
        assert_eq!(o.strategy(), round.rotate_left(round));
        assert_eq!(o.disable_filters(), round % 2 == 0);
        assert_eq!((o.dir_mode(), o.file_mode()), (round * 7, u32::MAX - round));
        assert_eq!(
            (o.file_open_flags(), o.notify_flags()),
            (i32::MIN + round as i32, !round)
        );
        assert_eq!(o.version(), 1);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn borrowed_strings_and_paths_from_every_source() {
    let owned_strs = strings(5);
    let owned = StrArray::from_strs(&owned_strs).unwrap();
    let view_strs = strings(3);
    let view = StrArrayView::new(&view_strs);
    let empty = StrArrayView::default();
    let labels = strings(8);
    let mut options = CheckoutOptions::new();
    for round in 0..16 {
        let mut m = options.as_mut();
        match round % 4 {
            0 => m.set_paths(owned.as_ref()),
            1 => m.set_paths(view.as_ref()),
            2 => m.set_paths(empty.as_ref()),
            _ => m.clear_paths(),
        }
        set_strings(m.as_mut(), &labels[round % 4..]);
        if round % 3 == 0 {
            m.set_our_label(Some(c"static"));
            m.set_target_directory(None);
        }
        let got = read_all(options.as_ref());
        let paths: Vec<_> = options
            .as_ref()
            .paths()
            .iter()
            .map(CStr::to_bytes)
            .collect();
        match round % 4 {
            0 => assert_eq!(
                paths,
                owned_strs.iter().map(|s| s.as_bytes()).collect::<Vec<_>>()
            ),
            1 => assert_eq!(
                paths,
                view_strs.iter().map(|s| s.as_bytes()).collect::<Vec<_>>()
            ),
            _ => assert!(paths.is_empty()),
        }
        assert!(got.len() >= 5);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn borrowed_sources_are_released_after_the_options() {
    let strs = strings(4);
    let mut owned = StrArray::from_strs(&strs).unwrap();
    let label = CString::new("mine").unwrap();
    {
        let mut options = CheckoutOptions::new();
        options.as_mut().set_paths(owned.as_ref());
        options.as_mut().set_our_label(Some(&label));
        assert_eq!(options.as_ref().paths().len(), 4);
    }
    // The options are gone: the array may change, and every source be released, once.
    owned.as_mut().truncate(1);
    assert_eq!(owned.as_ref().len(), 1);
    drop(owned);
    drop(label);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn getters_outlive_the_handles_and_the_options() {
    let strs = strings(4);
    let view = StrArrayView::new(&strs);
    let (label, paths) = {
        let mut options = CheckoutOptions::new();
        set_strings(options.as_mut(), &strs);
        options.as_mut().set_paths(view.as_ref());
        let label = options.as_ref().their_label();
        // The array header lives in the options, so the strings are read here.
        let paths: Vec<Vec<u8>> = options
            .as_ref()
            .paths()
            .iter()
            .map(|s| s.to_bytes().to_vec())
            .collect();
        (label, paths)
    };
    assert_eq!(label, Some(strs[3].as_c_str()));
    assert_eq!(paths.len(), 4);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reborrows_keep_the_retained_pointers_valid() {
    let strs = strings(6);
    let view = StrArrayView::new(&strs[..3]);
    let mut options = CheckoutOptions::new();
    let mut outer = options.as_mut();
    outer.set_paths(view.as_ref());
    set_strings(outer.as_mut(), &strs[2..]);
    for _ in 0..8 {
        let mut inner = outer.as_mut();
        let before = read_all(inner.as_ref());
        // Rust reads the sources the options point at, then writes through a reborrow.
        assert_eq!(strs[2].as_bytes(), before[3]);
        inner.as_mut().set_strategy(1);
        assert_eq!(read_all(inner.as_ref())[..7], before[..7]);
        let shared = outer.as_ref();
        let again = shared;
        assert_eq!(read_all(again), read_all(shared));
    }
    assert_eq!(options.as_ref().paths().len(), 3);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn options_on_threads_while_libgit2_initialises() {
    isolated(
        concat!(
            module_path!(),
            "::options_on_threads_while_libgit2_initialises"
        ),
        || {
            const THREADS: usize = 4;
            let shared = strings(4);
            let start = Barrier::new(THREADS + 1);
            thread::scope(|s| {
                s.spawn(|| {
                    start.wait();
                    let runtime = global::init().unwrap();
                    let owned = StrArray::from_strs(&shared).unwrap();
                    assert_eq!(owned.as_ref().len(), 4);
                    drop(owned);
                    drop(runtime);
                });
                for t in 0..THREADS {
                    let (shared, start) = (&shared, &start);
                    s.spawn(move || {
                        start.wait();
                        let view = StrArrayView::new(shared);
                        for round in 0..32 {
                            let mut options = CheckoutOptions::new();
                            let mut m = options.as_mut();
                            m.set_strategy((t * round) as u32);
                            m.set_paths(view.as_ref());
                            set_strings(m.as_mut(), shared);
                            assert_eq!(read_all(options.as_ref())[..4].len(), 4);
                        }
                    });
                }
            });
        },
    );
}

/// Stands in for the repository, tree and checkout wrappers the crate does not have
/// yet; replace it once they land.
#[allow(unsafe_code)]
mod repo {
    use std::ffi::CString;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::ptr;
    use std::sync::OnceLock;

    use libgit2::ffi;
    use libgit2::libgit2::index::index::{IndexArc, IndexBox};
    use libgit2::libgit2::object::tree::TreeArc;
    use libgit2::libgit2::workdir::checkout::CheckoutOptions;

    /// A non-bare repository whose working directory and index hold `old` (`a.txt`
    /// "old"), with a second tree, `new` (`a.txt` "new", `c.txt`); released on drop.
    pub struct Repo {
        pub root: PathBuf,
        repo: *mut ffi::git_repository,
        pub old: Tree,
        pub new: Tree,
    }

    /// One of the repository's trees.
    #[derive(Clone, Copy)]
    pub struct Tree(*mut ffi::git_tree);

    impl Repo {
        pub fn new(tag: &str) -> Self {
            // libgit2 marks a filter initialised on its first load outside the registry
            // lock, so concurrent first loads race (TSan): the first repository is
            // built alone.
            static PRIMED: OnceLock<()> = OnceLock::new();
            PRIMED.get_or_init(|| drop(Self::build("prime")));
            Self::build(tag)
        }

        fn build(tag: &str) -> Self {
            // SAFETY: initialising the library has no precondition.
            assert!(unsafe { ffi::git_libgit2_init() } >= 1);
            let root = std::env::temp_dir().join(format!(
                "crustify-checkout-ub-safe-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let work = root.join("work");
            fs::create_dir_all(&work).unwrap();
            let c_work = CString::new(work.as_os_str().as_bytes()).unwrap();
            let mut repo = ptr::null_mut();
            let mut index = ptr::null_mut();
            // SAFETY: live slots and a NUL-terminated path, read for the calls; the
            // index reference is released below.
            unsafe {
                assert_eq!(ffi::git_repository_init(&mut repo, c_work.as_ptr(), 0), 0);
                assert_eq!(ffi::git_repository_index(&mut index, repo), 0);
            }
            let tree = |files: &[(&str, &str)]| {
                for (path, content) in files {
                    fs::write(work.join(path), content).unwrap();
                    let path = CString::new(*path).unwrap();
                    // SAFETY: the index is live and `path` NUL-terminated.
                    let rc = unsafe { ffi::git_index_add_bypath(index, path.as_ptr()) };
                    assert_eq!(rc, 0);
                }
                let mut id = ffi::git_oid::default();
                let mut tree = ptr::null_mut();
                // SAFETY: live out-parameters and a live index and repository.
                unsafe {
                    assert_eq!(ffi::git_index_write_tree(&mut id, index), 0);
                    assert_eq!(ffi::git_tree_lookup(&mut tree, repo, &id), 0);
                }
                Tree(tree)
            };
            let new = tree(&[("a.txt", "new\n"), ("c.txt", "c\n")]);
            fs::remove_file(work.join("c.txt")).unwrap();
            let old = tree(&[("a.txt", "old\n")]);
            // SAFETY: the index and tree are live; the reference is released once.
            unsafe {
                assert_eq!(ffi::git_index_read_tree(index, old.0), 0);
                assert_eq!(ffi::git_index_write(index), 0);
                ffi::git_index_free(index);
            }
            Self {
                root,
                repo,
                old,
                new,
            }
        }

        /// The working directory's `a.txt`.
        pub fn a(&self) -> String {
            fs::read_to_string(self.root.join("work/a.txt")).unwrap()
        }

        /// The repository's index, one more counted reference to it.
        pub fn index(&self) -> IndexArc {
            let mut index = ptr::null_mut();
            // SAFETY: the repository is live; the result is a new reference to its index,
            // which the repository also keeps, released by `git_index_free`.
            unsafe {
                assert_eq!(ffi::git_repository_index(&mut index, self.repo), 0);
                IndexArc::from_c(index).unwrap()
            }
        }

        /// One more counted reference to `tree`, owned by the safe wrapper, which
        /// borrows the repository.
        pub fn tree(&self, tree: Tree) -> TreeArc<'_> {
            let mut copy = ptr::null_mut();
            // SAFETY: the tree is live; `git_tree_dup` counts one more reference on it,
            // given to the owner and released by `git_tree_free`; the repository outlives
            // the owner, which borrows `self`.
            unsafe {
                assert_eq!(ffi::git_tree_dup(&mut copy, tree.0), 0);
                TreeArc::from_c(copy).unwrap()
            }
        }

        /// Fills `index` with `tree`'s entries.
        pub fn read_tree(&self, index: &mut IndexBox, tree: Tree) {
            // SAFETY: the index is exclusively borrowed, and the tree live.
            let rc = unsafe { ffi::git_index_read_tree(index.as_c_ptr(), tree.0) };
            assert_eq!(rc, 0);
        }

        /// Checks `tree` out with `options`, as `git_checkout_tree`.
        pub fn checkout(&self, tree: Tree, options: &CheckoutOptions<'_>) -> i32 {
            // `#[repr(transparent)]` over `git_checkout_options`, as documented.
            let options: *const ffi::git_checkout_options = ptr::from_ref(options).cast();
            // SAFETY: the repository and tree are live, and the options' safe setters
            // stored only borrows live for this call.
            unsafe { ffi::git_checkout_tree(self.repo, tree.0.cast(), options) }
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            // SAFETY: the trees and repository are the references `build` took.
            unsafe {
                ffi::git_tree_free(self.old.0);
                ffi::git_tree_free(self.new.0);
                ffi::git_repository_free(self.repo);
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_boxed_baseline_index_is_read_then_released_after_the_options() {
    let repo = repo::Repo::new("index-box");
    let mut index = Index::new().unwrap();
    repo.read_tree(&mut index, repo.old);
    {
        let mut options = CheckoutOptions::new();
        options.as_mut().set_baseline_index(Some(index.as_ref()));
        assert!(options.as_ref().baseline_index().is_some());
        // The working directory matches the baseline, so the update is safe.
        assert_eq!(repo.checkout(repo.new, &options), 0);
        assert_eq!(repo.a(), "new\n");
        // Back to `old`, which the baseline matches: `a.txt` reads as a local change,
        // and is kept.
        assert_eq!(repo.checkout(repo.old, &options), 0);
        assert_eq!(repo.a(), "new\n");
        options.as_mut().set_baseline_index(None);
        assert!(options.as_ref().baseline_index().is_none());
    }
    // The options are gone: the index may change, and be released, once.
    repo.read_tree(&mut index, repo.new);
    drop(index);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_shared_baseline_index_outlives_the_options_and_the_repository() {
    let repo = repo::Repo::new("index-arc");
    let index: IndexArc = repo.index();
    let other: IndexArc = Index::new().unwrap().into();
    let mut options = CheckoutOptions::new();
    let mut m = options.as_mut();
    m.set_strategy(0);
    m.set_baseline_index(Some(other.as_ref()));
    // Replaced before use: only the repository's index is read.
    m.set_baseline_index(Some(index.as_ref()));
    assert_eq!(repo.checkout(repo.new, &options), 0);
    // The checkout updated that same index, so going back is safe too.
    assert_eq!(repo.checkout(repo.old, &options), 0);
    assert_eq!(repo.a(), "old\n");
    // The options' last use ends their borrows; the other index goes first.
    drop(other);
    // The index reference outlives the repository that also kept it.
    drop(repo);
    drop(index);
}

/// A baseline tree borrowed from its owner is read by real checkouts, through every
/// owner of the same tree (a lookup's and its clones'), and replaced or cleared without
/// a release; libgit2 counts a reference of its own on it for each call and gives it
/// back (LSan). The options' borrows end before the trees are released (ASan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_baseline_tree_is_read_then_released_after_the_options() {
    let repo = repo::Repo::new("baseline-tree");
    let old = repo.tree(repo.old);
    let clone = old.clone();
    let new = repo.tree(repo.new);
    {
        let mut options = CheckoutOptions::new();
        options.as_mut().set_baseline(Some(old.as_ref()));
        let read = options.as_ref().baseline().unwrap().to_tree();
        assert!(TreeArc::ptr_eq(&read, &old));
        drop(read);
        // The working directory matches the baseline, so the update is safe.
        assert_eq!(repo.checkout(repo.new, &options), 0);
        assert_eq!(repo.a(), "new\n");
        // The baseline is now stale: `new`'s files read as local changes, kept.
        options.as_mut().set_baseline(Some(clone.as_ref()));
        assert_eq!(repo.checkout(repo.old, &options), 0);
        assert_eq!(repo.a(), "new\n");
        options.as_mut().set_baseline(Some(new.as_ref()));
        assert_eq!(repo.checkout(repo.old, &options), 0);
        assert_eq!(repo.a(), "old\n");
        // `None` means `HEAD`'s, which is unborn: every dirty file conflicts.
        options.as_mut().set_baseline(None);
        assert!(options.as_ref().baseline().is_none());
        std::fs::write(repo.root.join("work/a.txt"), "dirty\n").unwrap();
        assert_eq!(
            repo.checkout(repo.new, &options),
            libgit2::ffi::GIT_ECONFLICT
        );
    }
    drop(old);
    assert_eq!(repo.a(), "dirty\n");
    drop((clone, new));
}

/// Two options share one baseline tree while a handle borrowed from them is read, then
/// both are dropped before the tree, and the tree before the repository.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn options_sharing_a_baseline_tree_are_dropped_before_it() {
    let repo = repo::Repo::new("baseline-shared");
    let tree = repo.tree(repo.old);
    {
        let mut a = CheckoutOptions::new();
        let mut b = CheckoutOptions::new();
        a.as_mut().set_baseline(Some(tree.as_ref()));
        b.as_mut().set_baseline(a.as_ref().baseline());
        let from_b = b.as_ref().baseline().unwrap();
        assert!(TreeArc::ptr_eq(&from_b.to_tree(), &tree));
        b.as_mut().set_strategy(GIT_CHECKOUT_FORCE);
        assert_eq!(repo.checkout(repo.new, &b), 0);
        assert_eq!(repo.a(), "new\n");
        // Against `a`'s baseline, `old`, the forced `new` reads as a local change, kept.
        assert_eq!(repo.checkout(repo.old, &a), 0);
        assert_eq!(repo.a(), "new\n");
    }
    drop(tree);
    drop(repo);
}
