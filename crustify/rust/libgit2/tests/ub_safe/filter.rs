//! UB tests for `git2/filter.h`: the `git_filter_list` owner.
//!
//! `FilterListBox` releases through `git_filter_list_free`, which hands each filter's
//! state to its `cleanup` (libgit2's `crlf` filter keeps an allocation per list): every
//! list is freed once (ASan flags a double free, LSan a leak), whatever constructor built
//! it and whatever order the owners drop in, and always before the repository it
//! borrows (the borrow enforces it; outliving it, or reaching the repository
//! exclusively meanwhile, does not compile: `compile_fail/filter_list_*` and the
//! doctests on `FilterList`). The handles reborrow without aliasing an exclusive one.
//!
//! The list keeps a bare pointer to its repository, which libgit2's filters read while
//! the list is applied (`crlf` reads the repository's configuration): applying a list
//! between Rust's own shared reborrows of the repository, which fill its caches
//! through the same object, must not conflict (BorrowSanitizer; the apply is the
//! fixture's stand-in for the list's apply wrapper).
//!
//! Lists are neither `Send` nor `Sync`, so nothing here crosses threads with one;
//! several threads each loading and dropping lists of their own repository exercise
//! libgit2's filter registry (under its read lock) and allocator under TSan. libgit2
//! marks a filter initialised under that *read* lock the first time a list selects it;
//! the constructors initialise the built-in filters once, before their first load (see
//! `filter.rs`'s module docs), so threads loading their first lists at once in a fresh
//! process do not race (TSan). libgit2's own routines that load lists internally (the
//! other modules' fixtures add files to an index through raw calls) do not go through
//! that, so each test here runs alone in a child process.

use std::ffi::CString;
use std::path::PathBuf;
use std::sync::Barrier;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::core::types::ObjectType;
use libgit2::libgit2::filter::filter::{FilterList, FilterListBox, FilterListMut, FilterListRef};
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryRef};
use libgit2::libgit2::util::errors::ErrorClass;

use crate::fixture::{apply, filter_count, init, isolated};

/// A fresh repository whose working directory's `.gitattributes` gives `*.txt` the
/// `crlf` filter (to CRLF in the working directory), `*.id` the `ident` one and
/// `*.both` both; freed and removed on drop.
struct Workdir {
    path: PathBuf,
    repo: Option<RepositoryBox>,
}

impl Workdir {
    fn new(tag: &str) -> Self {
        init();
        let path =
            std::env::temp_dir().join(format!("crustify-filter-ub-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let repo = Repository::init(&c_path, false).unwrap();
        std::fs::write(
            path.join(".gitattributes"),
            "*.txt text eol=crlf\n*.id ident\n*.both text eol=crlf ident\n",
        )
        .unwrap();
        Self {
            path,
            repo: Some(repo),
        }
    }

    fn repo(&self) -> RepositoryRef<'_> {
        self.repo.as_ref().unwrap().as_ref()
    }

    fn load(
        &self,
        path: &std::ffi::CStr,
        mode: ffi::git_filter_mode_t,
    ) -> Option<FilterListBox<'_>> {
        FilterList::load(self.repo(), path, mode, 0).unwrap()
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        // Every list borrowed the fixture, so is gone by now.
        drop(self.repo.take());
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn shared(r: FilterListRef<'_, '_>) -> String {
    format!("{r:?}")
}

fn exclusive(mut m: FilterListMut<'_, '_>) -> String {
    let inner = format!("{:?}", m.as_mut());
    assert_eq!(shared(m.as_ref()), inner);
    format!("{m:?}")
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn handles_reborrow_and_drop_once() {
    isolated(
        concat!(module_path!(), "::handles_reborrow_and_drop_once"),
        || {
            let wd = Workdir::new("handles");
            let mut list = wd.load(c"a.both", ffi::GIT_FILTER_TO_WORKTREE).unwrap();
            let r = list.as_ref();
            let copy = r;
            assert_eq!(shared(r), shared(copy));
            assert_eq!(exclusive(list.as_mut()), "FilterList { .. }");
            let mut m = list.as_mut();
            let _ = exclusive(m.as_mut());
            let _ = exclusive(m);
            assert_eq!(filter_count(&list), 2);
            drop(list);
        },
    );
}

/// Every constructor, with filter state or without, the lists alive together and
/// dropped out of order; applying a list in between writes its filters' state.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn constructors_build_lists_freed_once() {
    isolated(
        concat!(module_path!(), "::constructors_build_lists_freed_once"),
        || {
            let wd = Workdir::new("constructors");
            let repo = wd.repo();
            let mut lists: Vec<FilterListBox<'_>> = Vec::new();
            for mode in [
                ffi::GIT_FILTER_TO_WORKTREE,
                ffi::GIT_FILTER_TO_ODB,
                7,
                u32::MAX,
            ] {
                for flags in [0, ffi::GIT_FILTER_ALLOW_UNSAFE, u32::MAX] {
                    lists.push(FilterList::new(repo, mode, flags).unwrap());
                }
            }
            for path in [c"a.txt", c"a.id", c"a.both", c"sub/dir/b.txt", c"../c.id"] {
                for mode in [ffi::GIT_FILTER_TO_WORKTREE, ffi::GIT_FILTER_TO_ODB] {
                    lists.push(FilterList::load(repo, path, mode, 0).unwrap().unwrap());
                    let mut opts = ffi::git_filter_options {
                        version: 1,
                        flags: ffi::GIT_FILTER_NO_SYSTEM_ATTRIBUTES,
                        ..Default::default()
                    };
                    lists.push(
                        FilterList::load_ext(repo, path, mode, Some(&opts))
                            .unwrap()
                            .unwrap(),
                    );
                    opts.flags |= ffi::GIT_FILTER_ATTRIBUTES_FROM_COMMIT;
                    opts.attr_commit_id.id[0] = 0x5a;
                    lists.push(
                        FilterList::load_ext(repo, path, mode, Some(&opts))
                            .unwrap()
                            .unwrap(),
                    );
                    lists.push(
                        FilterList::load_ext(repo, path, mode, None)
                            .unwrap()
                            .unwrap(),
                    );
                }
            }
            // Paths no filter applies to: no list.
            for path in [c"a.bin", c"", c"txt"] {
                assert!(
                    FilterList::load(repo, path, ffi::GIT_FILTER_TO_ODB, 0)
                        .unwrap()
                        .is_none()
                );
                assert!(FilterList::load_ext(repo, path, 0, None).unwrap().is_none());
            }
            for list in lists.iter_mut().step_by(3) {
                if filter_count(list) > 0 {
                    apply(list, b"$Id$\na\nb\n").unwrap();
                }
            }
            // Drop from the back, the front, every other one, then the rest.
            drop(lists.pop());
            drop(lists.remove(0));
            let mut i = 0;
            lists.retain(|_| {
                i += 1;
                i % 2 == 0
            });
            drop(lists);
        },
    );
}

/// Failing loads leave nothing behind (LSan): an unborn `HEAD` fails
/// `GIT_FILTER_ATTRIBUTES_FROM_HEAD`, and an absolute path is refused.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn failed_loads_build_nothing() {
    isolated(
        concat!(module_path!(), "::failed_loads_build_nothing"),
        || {
            let wd = Workdir::new("failures");
            for _ in 0..4 {
                let error = FilterList::load(
                    wd.repo(),
                    c"a.txt",
                    ffi::GIT_FILTER_TO_ODB,
                    ffi::GIT_FILTER_ATTRIBUTES_FROM_HEAD,
                )
                .map(|_| ())
                .unwrap_err();
                assert_eq!(error.class(), Some(ErrorClass::Reference));
                let opts = ffi::git_filter_options {
                    version: 1,
                    flags: ffi::GIT_FILTER_ATTRIBUTES_FROM_HEAD,
                    ..Default::default()
                };
                assert!(FilterList::load_ext(wd.repo(), c"a.id", 0, Some(&opts)).is_err());
                let error = FilterList::load(wd.repo(), c"/a.txt", 0, 0)
                    .map(|_| ())
                    .unwrap_err();
                assert_eq!(error.class(), Some(ErrorClass::Internal));
            }
        },
    );
}

/// A shorter-lived repository borrow is fine for a list built for a longer one: the
/// owner is covariant in `'r`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owners_shorten_their_repository_borrow() {
    isolated(
        concat!(module_path!(), "::owners_shorten_their_repository_borrow"),
        || {
            let wd = Workdir::new("shorten");
            fn shorten<'s, 'l: 's>(list: FilterListBox<'l>) -> FilterListBox<'s> {
                list
            }
            let mut list = shorten(wd.load(c"a.txt", ffi::GIT_FILTER_TO_WORKTREE).unwrap());
            assert_eq!(apply(&mut list, b"a\n").unwrap(), b"a\r\n");
        },
    );
}

/// The list keeps a pointer to its repository, through which `crlf` reads the
/// repository's configuration while it applies; Rust's shared reborrows of the
/// repository in between reach the same object and fill its caches (attributes,
/// configuration) through it. Nothing may conflict (BorrowSanitizer), and every
/// application sees the same repository.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn retained_repository_across_rust_reborrows() {
    isolated(
        concat!(
            module_path!(),
            "::retained_repository_across_rust_reborrows"
        ),
        || {
            let wd = Workdir::new("reborrows");
            std::fs::write(wd.path.join("a.txt"), "a\r\nb\r\n").unwrap();
            let mut to_wd = wd.load(c"a.txt", ffi::GIT_FILTER_TO_WORKTREE).unwrap();
            let mut to_odb = wd.load(c"a.txt", ffi::GIT_FILTER_TO_ODB).unwrap();
            for _ in 0..3 {
                let repo = wd.repo();
                let id = repo.hash_file(c"a.txt", ObjectType::Blob, None).unwrap();
                assert_eq!(apply(&mut to_wd, b"a\nb\n").unwrap(), b"a\r\nb\r\n");
                let again = wd.repo();
                assert!(again.workdir().is_some());
                assert_eq!(apply(&mut to_odb, b"a\r\nb\r\n").unwrap(), b"a\nb\n");
                assert_eq!(
                    again.hash_file(c"a.txt", ObjectType::Blob, None).unwrap(),
                    id
                );
                let fresh = FilterList::load(repo, c"a.txt", ffi::GIT_FILTER_TO_ODB, 0).unwrap();
                drop(fresh);
            }
            drop(to_odb);
            drop(to_wd);
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn threads_load_and_drop_their_own_lists() {
    isolated(
        concat!(module_path!(), "::threads_load_and_drop_their_own_lists"),
        || {
            threads_load(4, 8);
        },
    );
}

/// In a fresh process, so that these are the first lists loaded: the threads start
/// together, and each builds its repository (pinning the runtime) and loads at once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn first_loads_on_threads_do_not_race() {
    isolated(
        concat!(module_path!(), "::first_loads_on_threads_do_not_race"),
        || threads_load(6, 1),
    );
}

fn threads_load(threads: usize, rounds: usize) {
    let start = Barrier::new(threads);
    thread::scope(|s| {
        for t in 0..threads {
            let start = &start;
            s.spawn(move || {
                start.wait();
                let wd = Workdir::new(&format!("threads-{threads}-{t}"));
                for i in 0..rounds {
                    let mode = if i % 2 == 0 {
                        ffi::GIT_FILTER_TO_WORKTREE
                    } else {
                        ffi::GIT_FILTER_TO_ODB
                    };
                    let mut list = wd.load(c"a.both", mode).unwrap();
                    let empty = FilterList::new(wd.repo(), mode, 0).unwrap();
                    apply(&mut list, b"$Id$\nx\n").unwrap();
                    drop(empty);
                    drop(list);
                }
            });
        }
    });
}
