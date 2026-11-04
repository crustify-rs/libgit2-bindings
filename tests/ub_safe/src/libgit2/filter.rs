//! UB tests for `git2/filter.h`: the `git_filter_list` owner.
//!
//! `FilterListBox` releases through `git_filter_list_free`, which hands each filter's
//! state to its `cleanup` (libgit2's `crlf` filter keeps an allocation per list): every
//! list is freed once (`ASan` flags a double free, `LSan` a leak), whatever constructor built
//! it and whatever order the owners drop in, and always before the repository it
//! borrows (the borrow enforces it; outliving it, or reaching the repository
//! exclusively meanwhile, does not compile: `compile_fail/filter_list_*` and the
//! doctests on `FilterList`). The handles reborrow without aliasing an exclusive one.
//!
//! The list keeps a bare pointer to its repository, which libgit2's filters read while
//! the list is applied (`crlf` reads the repository's configuration): applying a list
//! between Rust's own shared reborrows of the repository, which fill its caches
//! through the same object, must not conflict (`BorrowSanitizer`; the apply is the
//! fixture's stand-in for the list's apply wrapper).
//!
//! Lists are neither `Send` nor `Sync`, so nothing here crosses threads with one;
//! several threads each loading and dropping lists of their own repository exercise
//! libgit2's filter registry (under its read lock) and allocator under `TSan`. libgit2
//! marks a filter initialised under that *read* lock the first time a list selects it;
//! the constructors initialise the built-in filters once, before their first load (see
//! `filter.rs`'s module docs), so threads loading their first lists at once in a fresh
//! process do not race (`TSan`). libgit2's own routines that load lists internally (the
//! other modules' fixtures add files to an index through raw calls) do not go through
//! that, so each test here runs alone in a child process.
//!
//! The blob forms, [`with_blob`], take an object handle that must be a blob: libgit2
//! reads only the blob's id, which each filter's `check` sees during the load and the
//! list does not keep. A probe filter (the fixture's, registered once and selected
//! where the `crustify-probe` attribute is set) records that id from its `check` and
//! keeps a payload of its own in the list, which its `cleanup` frees: the tests drop
//! the blob (and, for a blob of another repository, that repository) before the list,
//! so a list that kept the blob would read freed memory on its free (`ASan`), and a list
//! freed without its filters' cleanup leaks the payload (`LSan`). An object of another
//! type is refused before libgit2 reads it as a blob (`ASan` would flag the read of a
//! smaller object), and the edge cases recur with a blob: no filter for the path (no
//! list), a failing load, and an absolute path.

use std::ffi::CString;
use std::path::PathBuf;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::filter::FilterList;
use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox, RepositoryRef};
use libgit2::libgit2::src::util::errors::ErrorClass;

use crate::fixture::{init, isolated};

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
        drop(std::fs::remove_dir_all(&path));
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
}

impl Drop for Workdir {
    fn drop(&mut self) {
        // Every list borrowed the fixture, so is gone by now.
        drop(self.repo.take());
        drop(std::fs::remove_dir_all(&self.path));
    }
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

mod with_blob {}
