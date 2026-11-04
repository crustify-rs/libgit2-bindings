//! UB tests for the safe API of `git2/checkout.h`: [`CheckoutOptions`], its handles,
//! and what its setters borrow.
//!
//! The options store pointers to the strings and path arrays their setters borrow, and
//! read them back through the getters; every source here is an allocation of exactly
//! its length (`CString`, `StrArrayView`'s pointer array, `StrArray::from_strs`'
//! copies), so `ASan` flags a read past one, and every borrowed source is released after
//! the options, exactly once (`ASan`, `LSan`). The handles reborrow while the options keep
//! those pointers (`BSan`). Options are built on several threads while another
//! initialises libgit2 for the first time: `git_checkout_options_init` touches no
//! global state, so nothing races (`TSan`). Illegal borrows are in `compile_fail/`, and
//! in the type's doctests.
//!
//! The baseline index is borrowed from its owner, an `IndexBox` or an `IndexArc`, and
//! the baseline tree from its `TreeArc`: real checkouts read them (`ASan`), libgit2 counts
//! a reference of its own on the tree for each call, which it must give back (`LSan`),
//! and the owners release them only after the options. The checkout routines and the
//! repository are not wrapped yet: [`repo`] stands in for them, as `fixture` does for
//! other missing constructors.
//!
//! Every test builds its options through `git_checkout_options_init`, so Miri skips
//! them; the inline unit tests run the accessors under Miri over a Rust-built value.
//!
//! [`CheckoutPerfdata`] holds three plain counters, which Rust values are built, copied
//! and read through without C: its handles reborrow and are read and written in turn
//! (Miri, `BSan`), copies cross threads and a shared value is read from several at once
//! (`TSan`, for its `Send` and `Sync`). A handle outliving its value, or a shared handle
//! beside the exclusive one, is in `compile_fail/`.

use std::ffi::{CStr, CString};
use std::sync::Barrier;
use std::thread;

use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::checkout::{
    CheckoutOptions, CheckoutOptionsMut, CheckoutOptionsRef, CheckoutPerfdata, CheckoutPerfdataMut,
    CheckoutPerfdataRef,
};
use libgit2::libgit2::src::libgit2::index::{Index, IndexArc};
use libgit2::libgit2::src::libgit2::libgit2 as global;
use libgit2::libgit2::src::libgit2::strarray::{StrArray, StrArrayView};
use libgit2::libgit2::src::libgit2::tree::TreeArc;

use crate::fixture::isolated;

/// `git_checkout_strategy_t`'s `GIT_CHECKOUT_FORCE`, from `git2/checkout.h`: the `-sys`
/// crate does not bind the enum.
const GIT_CHECKOUT_FORCE: u32 = 1 << 1;

/// `n` distinct strings of lengths 1, 2, 3, ...
fn strings(n: usize) -> Vec<CString> {
    (1..=n)
        .map(|i| CString::new(vec![b'a' + u8::try_from(i % 26).unwrap(); i]).unwrap())
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
        m.set_file_open_flags(i32::MIN + round.cast_signed());
        m.set_notify_flags(!round);
        let o = m.as_ref();
        assert_eq!(o.strategy(), round.rotate_left(round));
        assert_eq!(o.disable_filters(), round % 2 == 0);
        assert_eq!((o.dir_mode(), o.file_mode()), (round * 7, u32::MAX - round));
        assert_eq!(
            (o.file_open_flags(), o.notify_flags()),
            (i32::MIN + round.cast_signed(), !round)
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
                owned_strs.iter().map(CString::as_bytes).collect::<Vec<_>>()
            ),
            1 => assert_eq!(
                paths,
                view_strs.iter().map(CString::as_bytes).collect::<Vec<_>>()
            ),
            _ => assert!(paths.is_empty(), "{paths:?}"),
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
                            m.set_strategy(u32::try_from(t * round).unwrap());
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
#[expect(
    unsafe_code,
    reason = "stands in for the repository, tree and checkout wrappers the crate does not have yet"
)]
mod repo {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::{fs, ptr};

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::checkout::CheckoutOptions;
    use libgit2::libgit2::src::libgit2::index::{IndexArc, IndexBox};
    use libgit2::libgit2::src::libgit2::tree::TreeArc;

    /// A non-bare repository whose working directory and index hold `old` (`a.txt`
    /// "old"), with a second tree, `new` (`a.txt` "new", `c.txt`); released on drop.
    #[expect(
        clippy::struct_field_names,
        reason = "the fields mirror the names of what they hold"
    )]
    pub(super) struct Repo {
        pub root: PathBuf,
        repo: *mut ffi::git_repository,
        pub old: Tree,
        pub new: Tree,
    }

    /// One of the repository's trees.
    #[derive(Clone, Copy)]
    pub(super) struct Tree(*mut ffi::git_tree);

    impl Repo {
        pub(super) fn new(tag: &str) -> Self {
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
            drop(fs::remove_dir_all(&root));
            let work = root.join("work");
            fs::create_dir_all(&work).unwrap();
            let c_work = CString::new(work.as_os_str().as_bytes()).unwrap();
            let mut repo = ptr::null_mut();
            let mut index = ptr::null_mut();
            // SAFETY: live slots and a NUL-terminated path, read for the calls; the
            // index reference is released below.
            unsafe {
                assert_eq!(
                    ffi::git_repository_init(&raw mut repo, c_work.as_ptr(), 0),
                    0
                );
            };
            // SAFETY: as above.
            unsafe { assert_eq!(ffi::git_repository_index(&raw mut index, repo), 0) };
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
                unsafe { assert_eq!(ffi::git_index_write_tree(&raw mut id, index), 0) };
                // SAFETY: as above.
                unsafe { assert_eq!(ffi::git_tree_lookup(&raw mut tree, repo, &raw const id), 0) };
                Tree(tree)
            };
            let new = tree(&[("a.txt", "new\n"), ("c.txt", "c\n")]);
            fs::remove_file(work.join("c.txt")).unwrap();
            let old = tree(&[("a.txt", "old\n")]);
            // SAFETY: the index and tree are live; the reference is released once.
            unsafe { assert_eq!(ffi::git_index_read_tree(index, old.0), 0) };
            // SAFETY: as above.
            unsafe { assert_eq!(ffi::git_index_write(index), 0) };
            // SAFETY: as above.
            unsafe { ffi::git_index_free(index) };
            Self {
                root,
                repo,
                old,
                new,
            }
        }

        /// The working directory's `a.txt`.
        pub(super) fn a(&self) -> String {
            fs::read_to_string(self.root.join("work/a.txt")).unwrap()
        }

        /// The repository's index, one more counted reference to it.
        pub(super) fn index(&self) -> IndexArc {
            let mut index = ptr::null_mut();
            // SAFETY: the repository is live; the result is a new reference to its index,
            // which the repository also keeps, released by `git_index_free`.
            unsafe { assert_eq!(ffi::git_repository_index(&raw mut index, self.repo), 0) };
            // SAFETY: as above.
            unsafe { IndexArc::from_c(index).unwrap() }
        }

        /// One more counted reference to `tree`, owned by the safe wrapper, which
        /// borrows the repository.
        #[expect(
            clippy::unused_self,
            reason = "a method of the fixture, as its callers use it"
        )]
        pub(super) fn tree(&self, tree: Tree) -> TreeArc<'_> {
            let mut copy = ptr::null_mut();
            // SAFETY: the tree is live; `git_tree_dup` counts one more reference on it,
            // given to the owner and released by `git_tree_free`; the repository outlives
            // the owner, which borrows `self`.
            unsafe { assert_eq!(ffi::git_tree_dup(&raw mut copy, tree.0), 0) };
            // SAFETY: as above.
            unsafe { TreeArc::from_c(copy).unwrap() }
        }

        /// Fills `index` with `tree`'s entries.
        #[expect(
            clippy::unused_self,
            reason = "a method of the fixture, as its callers use it"
        )]
        pub(super) fn read_tree(&self, index: &mut IndexBox, tree: Tree) {
            // SAFETY: the index is exclusively borrowed, and the tree live.
            let rc = unsafe { ffi::git_index_read_tree(index.as_c_ptr(), tree.0) };
            assert_eq!(rc, 0);
        }

        /// Checks `tree` out with `options`, as `git_checkout_tree`.
        pub(super) fn checkout(&self, tree: Tree, options: &CheckoutOptions<'_>) -> i32 {
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
            unsafe { ffi::git_tree_free(self.old.0) };
            // SAFETY: as above.
            unsafe { ffi::git_tree_free(self.new.0) };
            // SAFETY: as above.
            unsafe { ffi::git_repository_free(self.repo) };
            drop(fs::remove_dir_all(&self.root));
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
/// back (`LSan`). The options' borrows end before the trees are released (`ASan`).
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

fn perfdata_counts(r: CheckoutPerfdataRef<'_>) -> (usize, usize, usize) {
    (r.mkdir_calls(), r.stat_calls(), r.chmod_calls())
}

fn bump(mut m: CheckoutPerfdataMut<'_>) {
    let r = m.as_ref();
    let (mkdir, stat, chmod) = perfdata_counts(r);
    m.set_mkdir_calls(mkdir + 1);
    m.set_stat_calls(stat + 1);
    m.set_chmod_calls(chmod + 1);
}

#[test]
fn perfdata_handles_reborrow_and_read_their_writes() {
    let mut d = CheckoutPerfdata::zeroed();
    let mut m = d.as_mut();
    bump(m.as_mut());
    assert_eq!(perfdata_counts(m.as_ref()), (1, 1, 1));
    bump(m.as_mut());
    m.set_stat_calls(usize::MAX);
    let shared = m.as_ref();
    let again = shared;
    assert_eq!(perfdata_counts(again), (2, usize::MAX, 2));
    assert_eq!(perfdata_counts(d.as_ref()), (2, usize::MAX, 2));
    let copy = d.as_ref().copied();
    d.as_mut().set_stat_calls(5);
    bump(d.as_mut());
    assert_eq!(perfdata_counts(copy.as_ref()), (2, usize::MAX, 2));
    assert_eq!(perfdata_counts(d.as_ref()), (3, 6, 3));
}

#[test]
fn perfdata_copies_cross_threads_and_shared_values_are_read_concurrently() {
    let mut d = CheckoutPerfdata::default();
    d.as_mut().set_mkdir_calls(7);
    d.as_mut().set_stat_calls(11);
    let shared = &d;
    let handle = d.as_ref();
    let barrier = Barrier::new(4);
    thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| {
                barrier.wait();
                for _ in 0..100 {
                    assert_eq!(perfdata_counts(shared.as_ref()), (7, 11, 0));
                }
            });
            s.spawn(|| {
                barrier.wait();
                for _ in 0..100 {
                    assert_eq!(perfdata_counts(handle), (7, 11, 0));
                }
            });
        }
    });
    let moved = thread::spawn(move || {
        let mut d = d;
        bump(d.as_mut());
        d
    })
    .join()
    .unwrap();
    assert_eq!(perfdata_counts(moved.as_ref()), (8, 12, 1));
    assert_eq!(
        perfdata_counts(d.as_ref()),
        (7, 11, 0),
        "the thread moved a copy"
    );
    let mut exclusive = moved;
    let m = exclusive.as_mut();
    thread::scope(|s| {
        s.spawn(move || bump(m));
    });
    assert_eq!(perfdata_counts(exclusive.as_ref()), (9, 13, 2));
}

/// The `ProgressCallback` tests: it is called through its `extern "C"` trampoline, as
/// libgit2 calls it, and lends its closure each path for the call only. The paths come
/// from allocations of exactly their length, released right after the call (`ASan`), and
/// the closures copy what they keep. A panic never crosses the trampoline: it resumes from
/// `call`, after which the callback runs its closure again. Callbacks run on several
/// threads at once, each over its own closure and sharing only a lock (`TSan`). Escaping
/// paths, a closure read while lent, and a callback sent to another thread are in
/// `compile_fail/`. None of this reaches C, so Miri runs it.
mod progress_callback {
    use std::cell::Cell;
    use std::ffi::{CStr, CString};
    use std::panic::{self, AssertUnwindSafe};
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::thread;

    use libgit2::libgit2::src::libgit2::checkout::ProgressCallback;

    type Seen = Vec<(Option<CString>, usize, usize)>;

    /// A type-erased progress closure.
    type DynProgress<'a> = dyn FnMut(Option<&CStr>, usize, usize) + 'a;

    fn record(seen: &mut Seen) -> impl FnMut(Option<&CStr>, usize, usize) + '_ {
        |path, completed, total| seen.push((path.map(CStr::to_owned), completed, total))
    }

    #[test]
    fn paths_from_every_source_are_lent_for_the_call() {
        let mut seen = Seen::new();
        let mut f = record(&mut seen);
        let mut callback = ProgressCallback::new(&mut f);
        callback.call(None, 0, 4);
        // A heap string of exactly its length, freed right after the call.
        let owned = CString::new("dir/a.txt").unwrap();
        callback.call(Some(&owned), 1, 4);
        drop(owned);
        // A string inside a larger buffer, and a static one.
        let buffer = b"x/b.txt\0trailing".to_vec();
        callback.call(Some(CStr::from_bytes_until_nul(&buffer).unwrap()), 2, 4);
        drop(buffer);
        callback.call(Some(c""), 3, 4);
        callback.call(Some(c"\xff\xfe"), 4, 4);
        drop(callback);
        drop(f);
        assert_eq!(seen.len(), 5);
        assert_eq!(seen[1].0.as_deref(), Some(c"dir/a.txt"));
        assert_eq!(seen[2].0.as_deref(), Some(c"x/b.txt"));
        assert_eq!(seen[4], (Some(c"\xff\xfe".to_owned()), 4, 4));
    }

    #[test]
    fn every_closure_holder() {
        fn plain(_: Option<&CStr>, completed: usize, total: usize) {
            assert!(completed <= total);
        }
        let mut item = plain;
        ProgressCallback::new(&mut item).call(None, 0, 1);

        let mut count = 0;
        let mut boxed: Box<DynProgress<'_>> = Box::new(|_, _, _| count += 1);
        let mut callback = ProgressCallback::new(&mut boxed);
        callback.call(None, 0, 2);
        callback.call(Some(c"a"), 1, 2);
        drop(callback);
        drop(boxed);
        assert_eq!(count, 2);

        // A closure owning its state, lent to the callback and called again after it.
        let len = Rc::new(Cell::new(0));
        let mut owned = {
            let len = Rc::clone(&len);
            let mut paths = Vec::new();
            move |path: Option<&CStr>, _, _| {
                paths.extend(path.map(CStr::to_owned));
                len.set(paths.len());
            }
        };
        let mut callback = ProgressCallback::new(&mut owned);
        callback.call(Some(c"one"), 1, 1);
        drop(callback);
        owned(Some(c"two"), 2, 2);
        drop(owned);
        assert_eq!((len.get(), Rc::strong_count(&len)), (2, 1));
    }

    #[test]
    fn a_closure_calls_another_callback_reentrantly() {
        let mut inner_seen = Seen::new();
        let mut outer_calls = 0;
        {
            let mut inner = record(&mut inner_seen);
            let mut outer = |path: Option<&CStr>, completed: usize, total: usize| {
                outer_calls += 1;
                // A callback within a callback, over the outer call's lent path.
                let mut nested = ProgressCallback::new(&mut inner);
                nested.call(path, completed * 10, total * 10);
            };
            let mut callback = ProgressCallback::new(&mut outer);
            callback.call(None, 0, 1);
            let path = CString::new("nested").unwrap();
            callback.call(Some(&path), 1, 1);
        }
        assert_eq!(outer_calls, 2);
        assert_eq!(
            inner_seen,
            [(None, 0, 10), (Some(c"nested".to_owned()), 10, 10)]
        );
    }

    #[test]
    fn a_panic_resumes_from_call_and_the_callback_runs_again() {
        let mut calls = Vec::new();
        let mut f = |path: Option<&CStr>, completed: usize, _: usize| {
            calls.push(completed);
            assert_ne!(completed, 1, "at {:?}", path.map(CStr::to_owned));
        };
        let mut callback = ProgressCallback::new(&mut f);
        callback.call(None, 0, 2);
        let path = CString::new("p").unwrap();
        let caught = panic::catch_unwind(AssertUnwindSafe(|| callback.call(Some(&path), 1, 2)));
        drop(path);
        // The closure's own panic payload, its message included, comes back unchanged.
        let message = caught.unwrap_err().downcast::<String>().unwrap();
        assert!(message.contains("at Some(\"p\")"), "{message}");
        assert_eq!(
            format!("{callback:?}"),
            "ProgressCallback { panicked: false, .. }"
        );
        callback.call(None, 2, 2);
        drop(callback);
        assert_eq!(calls, [0, 1, 2]);
    }

    #[test]
    fn callbacks_on_threads_share_only_a_lock() {
        let shared = Mutex::new(Vec::new());
        thread::scope(|s| {
            for t in 0..4_usize {
                let shared = &shared;
                s.spawn(move || {
                    let mut local = 0;
                    let mut f = |path: Option<&CStr>, completed: usize, _: usize| {
                        local += completed;
                        let path = path.map(CStr::to_owned);
                        shared.lock().unwrap().push((t, path, completed));
                    };
                    let mut callback = ProgressCallback::new(&mut f);
                    for step in 0..50 {
                        let path = CString::new(format!("t{t}/{step}")).unwrap();
                        callback.call(Some(&path), step, 50);
                    }
                    drop(callback);
                    assert_eq!(local, (0..50).sum::<usize>());
                });
            }
        });
        let shared = shared.into_inner().unwrap();
        assert_eq!(shared.len(), 200);
        assert!(
            shared
                .iter()
                .all(|(t, p, c)| p.as_deref().unwrap().to_str().unwrap() == format!("t{t}/{c}"))
        );
    }
}

/// The `NotifyCallback` tests: it is called through its `extern "C"` trampoline, as
/// libgit2 calls it, and lends its closure each notification for the call only. The sides
/// are diff files libgit2 parsed from a patch, freed right after the calls, and the paths
/// allocations of exactly their length, freed after theirs (`ASan`); the closures copy
/// what they keep. A break comes back as a cancel, and a panic never crosses the
/// trampoline: it resumes from `call`, after which the callback runs its closure again.
/// Callbacks run on several threads at once, each over its own closure and sharing only a
/// lock (`TSan`). Escaping notifications, a closure read while lent, and a callback sent
/// to another thread are in `compile_fail/`. Only the parsed sides reach C, so Miri runs
/// the rest.
mod notify_callback {
    use std::ffi::{CStr, CString};
    use std::ops::ControlFlow;
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::Mutex;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::checkout::{
        CheckoutNotification, CheckoutPerfdata, CheckoutPerfdataRef, NotifyCallback,
        PerfdataCallback,
    };
    use libgit2::libgit2::src::libgit2::diff::{Diff, DiffFileRef};

    const CONFLICT: u32 = ffi::GIT_CHECKOUT_NOTIFY_CONFLICT;
    const UPDATED: u32 = ffi::GIT_CHECKOUT_NOTIFY_UPDATED;

    /// A modification of `file`, then a rename of `old` to `new`.
    const PATCH: &[u8] = b"diff --git a/file b/file\nindex 1111111..2222222 100644\n\
        --- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n\
        diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n";

    /// A notification as owned data: the reason, the path and each side's path.
    type Seen = (u32, Option<CString>, [Option<CString>; 3]);

    fn seen(n: CheckoutNotification<'_>) -> Seen {
        let side = |f: Option<DiffFileRef<'_>>| f.and_then(|f| f.path()).map(CStr::to_owned);
        let sides = [side(n.baseline), side(n.target), side(n.workdir)];
        (n.why, n.path.map(CStr::to_owned), sides)
    }

    fn bare(why: u32, path: Option<&CStr>) -> CheckoutNotification<'_> {
        CheckoutNotification {
            why,
            path,
            baseline: None,
            target: None,
            workdir: None,
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn sides_libgit2_owns_are_lent_for_the_call() {
        let mut log = Vec::new();
        let mut record = |n: CheckoutNotification<'_>| {
            log.push(seen(n));
            ControlFlow::Continue(())
        };
        let mut callback = NotifyCallback::new(&mut record);
        let input = PATCH.to_vec();
        let diff = Diff::from_buffer(&input).unwrap();
        drop(input);
        for i in 0..diff.as_ref().len() {
            let delta = diff.as_ref().get(i).unwrap();
            let (old, new) = (delta.old_file(), delta.new_file());
            // As libgit2 lends them: the baseline and target sides, the old file again
            // as the workdir side, then no side at all.
            let full = CheckoutNotification {
                why: CONFLICT,
                path: old.path(),
                baseline: Some(old),
                target: Some(new),
                workdir: Some(old),
            };
            assert_eq!(callback.call(full), ControlFlow::Continue(()));
            assert_eq!(
                callback.call(bare(UPDATED, new.path())),
                ControlFlow::Continue(())
            );
        }
        drop(diff);
        drop(callback);
        let some = |s: &CStr| Some(s.to_owned());
        assert_eq!(
            log,
            [
                (
                    CONFLICT,
                    some(c"file"),
                    [some(c"file"), some(c"file"), some(c"file")]
                ),
                (UPDATED, some(c"file"), [None, None, None]),
                (
                    CONFLICT,
                    some(c"old"),
                    [some(c"old"), some(c"new"), some(c"old")]
                ),
                (UPDATED, some(c"new"), [None, None, None]),
            ]
        );
    }

    #[test]
    fn paths_from_every_source_are_lent_for_the_call() {
        let mut log = Vec::new();
        let mut record = |n: CheckoutNotification<'_>| {
            log.push(seen(n));
            ControlFlow::Continue(())
        };
        let mut callback = NotifyCallback::new(&mut record);
        // A heap string of exactly its length, freed right after the call.
        let owned = CString::new("dir/a.txt").unwrap();
        assert!(callback.call(bare(UPDATED, Some(&owned))).is_continue());
        drop(owned);
        // A string inside a larger buffer, a static one, and none.
        let buffer = b"x/b.txt\0trailing".to_vec();
        let inner = CStr::from_bytes_until_nul(&buffer).unwrap();
        assert!(callback.call(bare(UPDATED, Some(inner))).is_continue());
        drop(buffer);
        assert!(callback.call(bare(0, Some(c"\xff\xfe"))).is_continue());
        assert!(callback.call(bare(u32::MAX, None)).is_continue());
        drop(callback);
        let paths: Vec<_> = log.into_iter().map(|(why, path, _)| (why, path)).collect();
        assert_eq!(
            paths,
            [
                (UPDATED, Some(c"dir/a.txt".to_owned())),
                (UPDATED, Some(c"x/b.txt".to_owned())),
                (0, Some(c"\xff\xfe".to_owned())),
                (u32::MAX, None),
            ]
        );
    }

    #[test]
    fn every_closure_holder_and_its_verdict() {
        fn refuse_conflicts(n: CheckoutNotification<'_>) -> ControlFlow<()> {
            if n.why == CONFLICT {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
        let mut item = refuse_conflicts;
        let mut callback = NotifyCallback::new(&mut item);
        assert!(callback.call(bare(CONFLICT, None)).is_break());
        assert!(callback.call(bare(UPDATED, None)).is_continue());

        let mut count = 0;
        let mut boxed: Box<dyn FnMut(CheckoutNotification<'_>) -> ControlFlow<()> + '_> =
            Box::new(|_| {
                count += 1;
                ControlFlow::Break(())
            });
        let mut callback = NotifyCallback::new(&mut boxed);
        assert!(callback.call(bare(UPDATED, None)).is_break());
        // A break is no panic: the next call runs the closure again.
        assert!(callback.call(bare(UPDATED, None)).is_break());
        drop(callback);
        drop(boxed);
        assert_eq!(count, 2);

        // A closure owning its state, lent to the callback and called again after it.
        let mut owned = {
            let mut paths = Vec::new();
            move |n: CheckoutNotification<'_>| {
                paths.extend(n.path.map(CStr::to_owned));
                if paths.len() < 2 {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            }
        };
        let mut callback = NotifyCallback::new(&mut owned);
        assert!(callback.call(bare(UPDATED, Some(c"one"))).is_continue());
        drop(callback);
        assert!(owned(bare(UPDATED, Some(c"two"))).is_break());
    }

    #[test]
    fn a_closure_calls_other_callbacks_reentrantly() {
        let mut inner_log = Vec::new();
        let mut counts = 0;
        {
            let mut inner = |n: CheckoutNotification<'_>| {
                inner_log.push(seen(n));
                ControlFlow::Break(())
            };
            let mut perfdata = |_: CheckoutPerfdataRef<'_>| {
                counts += 1;
            };
            let mut outer = |n: CheckoutNotification<'_>| {
                // A callback within a callback, over the outer call's lent notification.
                let verdict = NotifyCallback::new(&mut inner).call(n);
                let zero = CheckoutPerfdata::default();
                PerfdataCallback::new(&mut perfdata).call(zero.as_ref());
                verdict
            };
            let mut callback = NotifyCallback::new(&mut outer);
            let path = CString::new("nested").unwrap();
            assert!(callback.call(bare(CONFLICT, Some(&path))).is_break());
        }
        assert_eq!(
            inner_log,
            [(CONFLICT, Some(c"nested".to_owned()), [None, None, None])]
        );
        assert_eq!(counts, 1);
    }

    #[test]
    fn a_panic_resumes_from_call_and_the_callback_runs_again() {
        let mut calls = Vec::new();
        let mut f = |n: CheckoutNotification<'_>| {
            calls.push(n.why);
            assert_ne!(n.why, CONFLICT, "at {:?}", n.path.map(CStr::to_owned));
            ControlFlow::Continue(())
        };
        let mut callback = NotifyCallback::new(&mut f);
        assert!(callback.call(bare(UPDATED, None)).is_continue());
        let path = CString::new("p").unwrap();
        let caught = panic::catch_unwind(AssertUnwindSafe(|| {
            callback.call(bare(CONFLICT, Some(&path)))
        }));
        drop(path);
        // The closure's own panic payload, its message included, comes back unchanged.
        let message = caught.unwrap_err().downcast::<String>().unwrap();
        assert!(message.contains("at Some(\"p\")"), "{message}");
        assert_eq!(
            format!("{callback:?}"),
            "NotifyCallback { panicked: false, .. }"
        );
        assert!(callback.call(bare(UPDATED, None)).is_continue());
        drop(callback);
        assert_eq!(calls, [UPDATED, CONFLICT, UPDATED]);
    }

    #[test]
    fn callbacks_on_threads_share_only_a_lock() {
        let shared = Mutex::new(Vec::new());
        thread::scope(|s| {
            for t in 0..4_u32 {
                let shared = &shared;
                s.spawn(move || {
                    let mut breaks = 0;
                    let mut f = |n: CheckoutNotification<'_>| {
                        let path = n.path.map(CStr::to_owned);
                        shared.lock().unwrap().push((t, path, n.why));
                        if n.why.is_multiple_of(2) {
                            ControlFlow::Continue(())
                        } else {
                            breaks += 1;
                            ControlFlow::Break(())
                        }
                    };
                    let mut callback = NotifyCallback::new(&mut f);
                    for why in 0..50 {
                        let path = CString::new(format!("t{t}/{why}")).unwrap();
                        let verdict = callback.call(bare(why, Some(&path)));
                        assert_eq!(verdict.is_break(), !why.is_multiple_of(2));
                    }
                    drop(callback);
                    assert_eq!(breaks, 25);
                });
            }
        });
        let shared = shared.into_inner().unwrap();
        assert_eq!(shared.len(), 200);
        assert!(
            shared
                .iter()
                .all(|(t, p, w)| p.as_deref().unwrap().to_str().unwrap() == format!("t{t}/{w}"))
        );
    }
}

/// The `PerfdataCallback` tests: it is called through its `extern "C"` trampoline, as
/// libgit2 calls it, and lends its closure the counters for the call only: those of Rust
/// values, of copies and of exclusive handles (Miri, `BSan`). A panic never crosses the
/// trampoline and resumes from `call`. Callbacks run on several threads at once (`TSan`).
/// Escaping counters and a callback sent to another thread are in `compile_fail/`. None of
/// this reaches C, so Miri runs it.
mod perfdata_callback {
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::Mutex;
    use std::thread;

    use libgit2::libgit2::src::libgit2::checkout::{
        CheckoutPerfdata, CheckoutPerfdataRef, PerfdataCallback,
    };

    fn counts(mkdir: usize, stat: usize, chmod: usize) -> CheckoutPerfdata {
        let mut counts = CheckoutPerfdata::default();
        let mut m = counts.as_mut();
        m.set_mkdir_calls(mkdir);
        m.set_stat_calls(stat);
        m.set_chmod_calls(chmod);
        counts
    }

    #[test]
    fn counters_from_every_holder_are_lent_for_the_call() {
        let mut seen = Vec::new();
        let mut record = |d: CheckoutPerfdataRef<'_>| seen.push(d.copied());
        let mut callback = PerfdataCallback::new(&mut record);
        let mut value = counts(1, 2, 3);
        callback.call(value.as_ref());
        // Through an exclusive handle's shared reborrow, then a write, then again.
        {
            let mut exclusive = value.as_mut();
            callback.call(exclusive.as_ref());
            exclusive.set_stat_calls(usize::MAX);
            callback.call(exclusive.as_ref());
        }
        // A boxed copy, freed right after the call.
        let boxed = Box::new(value.as_ref().copied());
        callback.call(boxed.as_ref().as_ref());
        drop(boxed);
        drop(callback);
        assert_eq!(
            seen,
            [
                counts(1, 2, 3),
                counts(1, 2, 3),
                counts(1, usize::MAX, 3),
                counts(1, usize::MAX, 3)
            ]
        );
    }

    #[test]
    fn every_closure_holder() {
        fn check(d: CheckoutPerfdataRef<'_>) {
            assert!(d.mkdir_calls() <= d.stat_calls());
        }
        let mut item = check;
        PerfdataCallback::new(&mut item).call(counts(1, 2, 0).as_ref());

        let mut total = 0;
        let mut boxed: Box<dyn FnMut(CheckoutPerfdataRef<'_>) + '_> =
            Box::new(|d| total += d.stat_calls());
        let mut callback = PerfdataCallback::new(&mut boxed);
        callback.call(counts(0, 5, 0).as_ref());
        callback.call(counts(0, 7, 0).as_ref());
        drop(callback);
        drop(boxed);
        assert_eq!(total, 12);
    }

    #[test]
    fn a_panic_resumes_from_call_and_the_callback_runs_again() {
        let mut calls = 0;
        let mut f = |d: CheckoutPerfdataRef<'_>| {
            calls += 1;
            assert_ne!(d.chmod_calls(), 1, "chmod");
        };
        let mut callback = PerfdataCallback::new(&mut f);
        let one = counts(0, 0, 1);
        let caught = panic::catch_unwind(AssertUnwindSafe(|| callback.call(one.as_ref())));
        let message = caught.unwrap_err().downcast::<String>().unwrap();
        assert!(message.contains("chmod"), "{message}");
        assert_eq!(
            format!("{callback:?}"),
            "PerfdataCallback { panicked: false, .. }"
        );
        callback.call(counts(0, 0, 2).as_ref());
        drop(callback);
        assert_eq!(calls, 2);
    }

    #[test]
    fn callbacks_on_threads_share_only_a_lock() {
        let shared = Mutex::new(0_usize);
        let value = counts(1, 2, 3);
        thread::scope(|s| {
            for _ in 0..4 {
                let (shared, value) = (&shared, &value);
                s.spawn(move || {
                    let mut f = |d: CheckoutPerfdataRef<'_>| {
                        *shared.lock().unwrap() += d.stat_calls();
                    };
                    let mut callback = PerfdataCallback::new(&mut f);
                    for _ in 0..50 {
                        // A value shared by every thread, lent to each call.
                        callback.call(value.as_ref());
                    }
                });
            }
        });
        assert_eq!(shared.into_inner().unwrap(), 4 * 50 * 2);
    }
}
