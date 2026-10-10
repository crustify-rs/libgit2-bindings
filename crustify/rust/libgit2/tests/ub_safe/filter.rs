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
//!
//! The blob forms, [`with_blob`], take an object handle that must be a blob: libgit2
//! reads only the blob's id, which each filter's `check` sees during the load and the
//! list does not keep. A probe filter (the fixture's, registered once and selected
//! where the `crustify-probe` attribute is set) records that id from its `check` and
//! keeps a payload of its own in the list, which its `cleanup` frees: the tests drop
//! the blob (and, for a blob of another repository, that repository) before the list,
//! so a list that kept the blob would read freed memory on its free (ASan), and a list
//! freed without its filters' cleanup leaks the payload (LSan). An object of another
//! type is refused before libgit2 reads it as a blob (ASan would flag the read of a
//! smaller object), and the edge cases recur with a blob: no filter for the path (no
//! list), a failing load, and an absolute path.

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

mod with_blob {
    use std::ffi::CStr;
    use std::path::PathBuf;
    use std::ptr::NonNull;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::filter::filter::{FilterList, FilterListBox};
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryRef};
    use libgit2::libgit2::util::errors::{ErrorBuf, ErrorClass};

    use crate::fixture::filter_probe::{register, seen};
    use crate::fixture::object_and_tree_entry::{object_view, write_blob};
    use crate::fixture::{filter_count, init, isolated};

    /// A fresh repository whose `.gitattributes` gives `*.probe` the probe and `*.id`
    /// libgit2's `ident` filter; freed and removed on drop.
    struct Workdir {
        path: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl Workdir {
        /// Registers the probe and primes it (a first list selecting it, loaded on this
        /// thread before any test loads its own), then makes the repository.
        fn new(tag: &str) -> Self {
            register();
            let wd = Self::make(tag);
            drop(FilterList::load(wd.repo(), c"a.probe", ffi::GIT_FILTER_TO_ODB, 0).unwrap());
            let _ = seen();
            wd
        }

        fn make(tag: &str) -> Self {
            init();
            let path = std::env::temp_dir().join(format!(
                "crustify-filter-ub-blob-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            let c_path = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
            let repo = Repository::init(&c_path, false).unwrap();
            std::fs::write(
                path.join(".gitattributes"),
                "*.probe crustify-probe\n*.id ident\n",
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

        /// A new blob of `content` in this repository, as an object, and its id.
        fn blob(&self, content: &[u8]) -> (ObjectArc<'_>, String) {
            let blob = write_blob(self.repo.as_ref().unwrap(), content);
            let id = object_view(&blob).0;
            (blob, id)
        }
    }

    impl Drop for Workdir {
        fn drop(&mut self) {
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn refused(result: Result<Option<FilterListBox<'_>>, ErrorBuf>) -> ErrorBuf {
        result.map(|_| ()).unwrap_err()
    }

    /// The probe's `check` sees the blob's id, and libgit2's `ident` filter (which has
    /// no `check`) nothing; the blob goes before the list, whose free hands the probe's
    /// payload back to its `cleanup`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn checks_see_the_id_and_the_list_keeps_no_blob() {
        isolated(
            concat!(
                module_path!(),
                "::checks_see_the_id_and_the_list_keeps_no_blob"
            ),
            || {
                let wd = Workdir::new("blob-freed");
                let _ = seen();
                for path in [c"a.probe", c"a.id"] {
                    for mode in [ffi::GIT_FILTER_TO_WORKTREE, ffi::GIT_FILTER_TO_ODB] {
                        let (blob, id) = wd.blob(b"$Id$\n");
                        let list =
                            FilterList::load_with_blob(wd.repo(), blob.as_ref(), path, mode, 0);
                        drop(blob);
                        let list = list.unwrap().unwrap();
                        assert_eq!(filter_count(&list), 1);
                        let expect = if path == c"a.probe" {
                            (vec![Some(id)], 1)
                        } else {
                            (vec![], 0)
                        };
                        assert_eq!(seen(), expect);
                        drop(list);
                        assert_eq!(seen().1, 0);
                    }
                }
            },
        );
    }

    /// A blob of another repository: only its id is read, and that repository may go
    /// before the list; the `_ext` form likewise, with options that are only copied
    /// (a commit that does not exist, which libgit2 skips, and a dangling `reserved`
    /// pointer, which it never reads).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn blob_of_another_repository() {
        isolated(
            concat!(module_path!(), "::blob_of_another_repository"),
            || {
                let wd = Workdir::new("blob-own");
                let other = Workdir::new("blob-other");
                let _ = seen();
                let (blob, id) = other.blob(b"elsewhere\n");
                let list = FilterList::load_with_blob(
                    wd.repo(),
                    blob.as_ref(),
                    c"x.probe",
                    ffi::GIT_FILTER_TO_ODB,
                    0,
                );
                let ext = {
                    let mut opts = ffi::git_filter_options {
                        version: 1,
                        flags: ffi::GIT_FILTER_ATTRIBUTES_FROM_COMMIT,
                        reserved: NonNull::<u64>::dangling().as_ptr().cast(),
                        ..Default::default()
                    };
                    opts.attr_commit_id.id[..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
                    FilterList::load_ext_with_blob(
                        wd.repo(),
                        blob.as_ref(),
                        c"a.probe",
                        ffi::GIT_FILTER_TO_ODB,
                        Some(&opts),
                    )
                };
                drop(blob);
                drop(other);
                let list = list.unwrap().unwrap();
                let ext = ext.unwrap().unwrap();
                assert_eq!(seen(), (vec![Some(id.clone()), Some(id)], 2));
                drop(list);
                assert_eq!(seen().1, 1);
                drop(ext);
                assert_eq!(seen().1, 0);
            },
        );
    }

    /// The `_ext` form with and without options: the probe sees the id each time.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn ext_checks_see_the_id_and_the_list_keeps_no_blob() {
        isolated(
            concat!(
                module_path!(),
                "::ext_checks_see_the_id_and_the_list_keeps_no_blob"
            ),
            || {
                let wd = Workdir::new("ext-blob-freed");
                let _ = seen();
                let opts = ffi::git_filter_options {
                    version: 1,
                    flags: ffi::GIT_FILTER_NO_SYSTEM_ATTRIBUTES,
                    ..Default::default()
                };
                for opts in [None, Some(&opts)] {
                    let (blob, id) = wd.blob(b"ext\n");
                    let list = FilterList::load_ext_with_blob(
                        wd.repo(),
                        blob.as_ref(),
                        c"a.probe",
                        ffi::GIT_FILTER_TO_WORKTREE,
                        opts,
                    );
                    drop(blob);
                    let list = list.unwrap().unwrap();
                    assert_eq!(seen(), (vec![Some(id)], 1));
                    drop(list);
                    assert_eq!(seen().1, 0);
                }
            },
        );
    }

    /// No filter for the path (no list), an unborn `HEAD` (a failed load) and an
    /// absolute path (refused before libgit2, so before any check): nothing is built,
    /// the probe sees nothing, and the blob stays the caller's, released once.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn edge_cases_build_nothing_and_leave_the_blob_alone() {
        isolated(
            concat!(
                module_path!(),
                "::edge_cases_build_nothing_and_leave_the_blob_alone"
            ),
            || {
                let wd = Workdir::new("blob-edges");
                let _ = seen();
                let (blob, _) = wd.blob(b"x");
                let head = ffi::git_filter_options {
                    version: 1,
                    flags: ffi::GIT_FILTER_ATTRIBUTES_FROM_HEAD,
                    ..Default::default()
                };
                for _ in 0..3 {
                    let blob = blob.as_ref();
                    assert!(
                        FilterList::load_with_blob(wd.repo(), blob, c"a.bin", 0, 0)
                            .unwrap()
                            .is_none()
                    );
                    assert!(
                        FilterList::load_ext_with_blob(wd.repo(), blob, c"a.bin", 0, None)
                            .unwrap()
                            .is_none()
                    );
                    let error = refused(FilterList::load_with_blob(
                        wd.repo(),
                        blob,
                        c"a.probe",
                        0,
                        ffi::GIT_FILTER_ATTRIBUTES_FROM_HEAD,
                    ));
                    assert_eq!(error.class(), Some(ErrorClass::Reference));
                    let error = refused(FilterList::load_ext_with_blob(
                        wd.repo(),
                        blob,
                        c"a.probe",
                        0,
                        Some(&head),
                    ));
                    assert_eq!(error.class(), Some(ErrorClass::Reference));
                    let error = refused(FilterList::load_with_blob(
                        wd.repo(),
                        blob,
                        c"/a.probe",
                        0,
                        0,
                    ));
                    assert_eq!(error.class(), Some(ErrorClass::Internal));
                    let error = refused(FilterList::load_ext_with_blob(
                        wd.repo(),
                        blob,
                        c"//a.probe",
                        0,
                        None,
                    ));
                    assert_eq!(error.class(), Some(ErrorClass::Internal));
                }
                assert_eq!(seen(), (vec![], 0));
                drop(blob);
            },
        );
    }

    /// An object that is not a blob (the commit, tree and tag of `testrepo.git`) is
    /// refused by both forms before libgit2 reads it as one, with a `GIT_ERROR_FILTER`
    /// error; the probe sees nothing and the object stays usable.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn only_blobs_are_read() {
        isolated(concat!(module_path!(), "::only_blobs_are_read"), || {
            let wd = Workdir::new("not-blobs");
            let _ = seen();
            let path = std::ffi::CString::new(format!(
                "{}/../../../tests/resources/testrepo.git",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            let fixture = Repository::open(&path).unwrap();
            let fixture = &fixture;
            let lookup = move |hex: &CStr, kind| {
                let id = Oid::from_string(hex, OidType::Sha1).unwrap();
                Object::lookup(fixture.as_ref(), id.as_ref(), Some(kind)).unwrap()
            };
            let others = [
                lookup(
                    c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
                    ObjectType::Commit,
                ),
                lookup(
                    c"944c0f6e4dfa41595e6eb3ceecdb14f50fe18162",
                    ObjectType::Tree,
                ),
                lookup(c"b25fa35b38051e4ae45d4222e795f9df2e43f1d1", ObjectType::Tag),
            ];
            for other in &others {
                for path in [c"a.probe", c"a.id", c"a.bin"] {
                    let error = refused(FilterList::load_with_blob(
                        wd.repo(),
                        other.as_ref(),
                        path,
                        0,
                        0,
                    ));
                    assert_eq!(error.class(), Some(ErrorClass::Filter));
                    assert_eq!(error.message(), c"the object is not a blob");
                    let error = refused(FilterList::load_ext_with_blob(
                        wd.repo(),
                        other.as_ref(),
                        path,
                        0,
                        None,
                    ));
                    assert_eq!(error.message(), c"the object is not a blob");
                }
                let _ = object_view(other);
            }
            assert_eq!(seen(), (vec![], 0));
            // A blob of that repository is taken.
            let blob = lookup(
                c"a8233120f6ad708f843d861ce2b7228ec4e3dec6",
                ObjectType::Blob,
            );
            let list = FilterList::load_with_blob(wd.repo(), blob.as_ref(), c"a.probe", 0, 0)
                .unwrap()
                .unwrap();
            assert_eq!(
                seen(),
                (
                    vec![Some("a8233120f6ad708f843d861ce2b7228ec4e3dec6".into())],
                    1
                )
            );
            drop(blob);
            drop(list);
            assert_eq!(seen().1, 0);
        });
    }
}
