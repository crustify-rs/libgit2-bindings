//! UB tests for the safe API of `git2/annotated_commit.h`: the four constructors and
//! the owner [`AnnotatedCommitBox`] they build. Each constructor runs on every kind of
//! input a repository can answer, edge cases included: ids of every object type and of
//! every id type (a SHA256 id in a SHA1 repository is the one that overflowed
//! libgit2's path buffer), names that peel to no commit, references of another
//! repository, and allocation failures after libgit2 built the commit. Each result is
//! released in every order its repository and references allow, on this thread or
//! another while its commit's other references (the repository's object cache) are in
//! use (`ASan`: overflow, use after free, double free; `LSan`: leak; `TSan`: the commit's
//! reference count).
//!
//! Outliving the repository, sharing an owner across threads (it is not `Sync`),
//! cloning one (there is no copy routine) and outliving one with a handle must not
//! compile: see the compile-fail doctests on [`AnnotatedCommit`].
//!
//! The allocation-failure tests install an allocator, a process-wide setting, so they
//! rerun alone in a child process.

use std::ffi::CString;
use std::thread;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::annotated_commit::{AnnotatedCommit, AnnotatedCommitBox};
use libgit2::libgit2::src::libgit2::oid::Oid;
use libgit2::libgit2::src::util::errors::ErrorClass;

use crate::fixture::annotated_commit::{
    CommitRepo, LONG_BRANCH, LONG_URL, hex, read, refuse, refused,
};
use crate::fixture::index::isolated_tolerating_leaks;
use crate::fixture::isolated;

/// `id`'s bytes under the type byte `kind`.
fn retyped(id: &ffi::git_oid, kind: u8) -> Oid {
    let mut oid = Oid::zeroed();
    oid.as_mut().set_raw_kind(kind);
    oid.as_mut().set_id(id.id);
    oid
}

fn oid(id: &ffi::git_oid) -> Oid {
    retyped(id, id.type_)
}

/// Every id type byte a caller can build: the named ones and unnamed ones around them.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the named id types are 1 and 2, and a const cannot call try_from"
)]
const KINDS: [u8; 5] = [
    0,
    ffi::GIT_OID_SHA1 as u8,
    ffi::GIT_OID_SHA256 as u8,
    3,
    0xFF,
];

/// Both object id types a repository can have.
const REPO_KINDS: [ffi::git_oid_t; 2] = [ffi::GIT_OID_SHA1, ffi::GIT_OID_SHA256];

mod lookup {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn every_object_and_id_type() {
        for own in REPO_KINDS {
            let repo = CommitRepo::new(&format!("lookup-kinds-{own}"), own);
            let mut held = Vec::new();
            for id in [&repo.first, &repo.second, &repo.tree, &repo.tag] {
                for kind in KINDS {
                    match AnnotatedCommit::lookup(repo.repo(), retyped(id, kind).as_ref()) {
                        Ok(commit) => {
                            assert_eq!(kind, u8::try_from(own).unwrap());
                            assert_eq!(read(&commit), (hex(id), None));
                            held.push(commit);
                        }
                        Err(e) if kind != u8::try_from(own).unwrap() => {
                            assert_eq!(e.class(), Some(ErrorClass::Invalid));
                            assert_eq!(
                                e.message(),
                                c"specified object ID type does not match repository object ID type"
                            );
                        }
                        Err(e) => assert_eq!(e.class(), Some(ErrorClass::Invalid), "{e}"),
                    }
                }
            }
            assert_eq!(held.len(), 2);
            let mut missing = oid(&repo.first);
            missing.as_mut().set_id([0xA5; 32]);
            let e = AnnotatedCommit::lookup(repo.repo(), missing.as_ref()).unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Odb));
            let zero = Oid::zeroed();
            assert!(AnnotatedCommit::lookup(repo.repo(), zero.as_ref()).is_err());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn results_drop_on_other_threads_while_the_cache_hands_out_more() {
        let repo = CommitRepo::new("lookup-threads", ffi::GIT_OID_SHA1);
        let id = oid(&repo.second);
        let batches: Vec<Vec<AnnotatedCommitBox<'_>>> = (0..4)
            .map(|_| {
                (0..8)
                    .map(|_| AnnotatedCommit::lookup(repo.repo(), id.as_ref()).unwrap())
                    .collect()
            })
            .collect();
        let expected = (hex(&repo.second), None);
        thread::scope(|s| {
            for batch in batches {
                s.spawn(move || drop(batch));
            }
            // This thread keeps taking references to the same cached commit meanwhile.
            for _ in 0..32 {
                let commit = AnnotatedCommit::lookup(repo.repo(), id.as_ref()).unwrap();
                assert_eq!(read(&commit), expected);
            }
        });
        // An owner moved to another thread and back.
        let commit = AnnotatedCommit::lookup(repo.repo(), id.as_ref()).unwrap();
        let mut commit = thread::scope(|s| s.spawn(move || commit).join().unwrap());
        let _ = commit.as_mut().as_ref();
        assert_eq!(read(&commit), expected);
    }
}

mod from_ref {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn every_reference_kind_outliving_the_reference() {
        let repo = CommitRepo::new("from-ref", ffi::GIT_OID_SHA1);
        let mut held = Vec::new();
        for name in [
            c"HEAD",
            c"refs/heads/main",
            c"refs/tags/v1",
            c"refs/tags/tree",
            LONG_BRANCH,
        ] {
            let reference = repo.reference(name);
            let result = AnnotatedCommit::from_ref(repo.repo(), reference.as_ref());
            drop(reference);
            match result {
                Ok(commit) => {
                    assert_eq!(read(&commit).1.as_deref(), name.to_str().ok());
                    held.push(commit);
                }
                Err(e) => {
                    assert_eq!(name, c"refs/tags/tree");
                    assert_eq!(e.class(), Some(ErrorClass::Object));
                }
            }
        }
        assert_eq!(held.len(), 4);
        // A reference of another repository peels there, and is looked up here; the
        // result outlives that repository.
        let other = CommitRepo::new("from-ref-other", ffi::GIT_OID_SHA1);
        let reference = other.reference(c"refs/heads/main");
        let commit = AnnotatedCommit::from_ref(repo.repo(), reference.as_ref()).unwrap();
        drop(reference);
        drop(other);
        assert_eq!(read(&commit).1.as_deref(), Some("refs/heads/main"));
        // A SHA256 reference peels to an id the SHA1 repository cannot hold: refused
        // (libgit2 overflowed its object path buffer with it).
        let wide = CommitRepo::new("from-ref-sha256", ffi::GIT_OID_SHA256);
        let reference = wide.reference(c"refs/heads/main");
        let e = AnnotatedCommit::from_ref(repo.repo(), reference.as_ref()).unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        drop(reference);
        thread::scope(|s| {
            s.spawn(move || drop((commit, held)));
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn frees_what_libgit2_built_when_the_name_copy_fails() {
        isolated(
            concat!(
                module_path!(),
                "::frees_what_libgit2_built_when_the_name_copy_fails"
            ),
            || {
                let repo = CommitRepo::new("from-ref-oom", ffi::GIT_OID_SHA1);
                {
                    let reference = repo.reference(LONG_BRANCH);
                    // The second 100-byte request is the reference name's copy; the
                    // first, the description's, succeeds (its failure leaks inside
                    // libgit2).
                    refuse(LONG_BRANCH.to_bytes_with_nul().len(), 2);
                    let e = AnnotatedCommit::from_ref(repo.repo(), reference.as_ref()).unwrap_err();
                    assert!(refused());
                    assert_eq!(e.class(), Some(ErrorClass::NoMemory));
                    // The same call succeeds once memory is back.
                    let commit =
                        AnnotatedCommit::from_ref(repo.repo(), reference.as_ref()).unwrap();
                    assert_eq!(
                        read(&commit),
                        (hex(&repo.first), LONG_BRANCH.to_str().ok().map(Into::into))
                    );
                }
                // libgit2 itself leaks its reference to the peeled commit on the failed
                // path (it returns before `git_object_free(peeled)`). The repository
                // stays open, so its object cache still reaches that commit, and
                // LeakSanitizer reports only what nothing reaches: the failed call's
                // annotated commit, had the wrapper not freed it.
                repo.keep_open();
            },
        );
    }

    /// The copy before the name's, the description's, fails inside libgit2's
    /// `annotated_commit_init`, which reports the failure with nothing in the out slot
    /// (and leaks the half-built commit, so `LeakSanitizer` is off in this child): the
    /// wrapper adopts nothing and reports the error, and the next call succeeds.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn description_copy_failure_adopts_nothing() {
        isolated_tolerating_leaks(
            concat!(module_path!(), "::description_copy_failure_adopts_nothing"),
            || {
                let repo = CommitRepo::new("from-ref-description-oom", ffi::GIT_OID_SHA1);
                let reference = repo.reference(LONG_BRANCH);
                refuse(LONG_BRANCH.to_bytes_with_nul().len(), 1);
                let e = AnnotatedCommit::from_ref(repo.repo(), reference.as_ref()).unwrap_err();
                assert!(refused());
                assert_eq!(e.class(), Some(ErrorClass::NoMemory));
                let commit = AnnotatedCommit::from_ref(repo.repo(), reference.as_ref()).unwrap();
                assert_eq!(
                    read(&commit),
                    (hex(&repo.first), LONG_BRANCH.to_str().ok().map(Into::into))
                );
                drop(commit);
                drop(reference);
                repo.keep_open();
            },
        );
    }
}

mod from_fetchhead {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn every_id_type_and_string() {
        for own in REPO_KINDS {
            let repo = CommitRepo::new(&format!("from-fetchhead-{own}"), own);
            let mut held = Vec::new();
            for id in [&repo.first, &repo.tree, &repo.tag] {
                for kind in KINDS {
                    for (branch, url) in [(c"main", c"/srv/r"), (c"", c""), (LONG_BRANCH, LONG_URL)]
                    {
                        let result = AnnotatedCommit::from_fetchhead(
                            repo.repo(),
                            branch,
                            url,
                            retyped(id, kind).as_ref(),
                        );
                        match result {
                            Ok(commit) => {
                                assert!(
                                    std::ptr::eq(id, &raw const repo.first)
                                        && kind == u8::try_from(own).unwrap()
                                );
                                assert_eq!(
                                    read(&commit),
                                    (hex(id), branch.to_str().ok().map(Into::into))
                                );
                                held.push(commit);
                            }
                            Err(e) => assert_eq!(e.class(), Some(ErrorClass::Invalid), "{e}"),
                        }
                    }
                }
            }
            assert_eq!(held.len(), 3);
            thread::scope(|s| {
                s.spawn(move || drop(held));
            });
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn frees_what_libgit2_built_when_a_copy_fails() {
        isolated(
            concat!(
                module_path!(),
                "::frees_what_libgit2_built_when_a_copy_fails"
            ),
            || {
                let repo = CommitRepo::new("from-fetchhead-oom", ffi::GIT_OID_SHA1);
                let id = oid(&repo.first);
                let branch = LONG_BRANCH.to_bytes_with_nul().len();
                // The remote URL's copy, the last allocation; then the reference name's,
                // the second copy of the branch name (the first is the description's,
                // whose failure leaks inside libgit2).
                for (len, nth) in [(LONG_URL.to_bytes_with_nul().len(), 1), (branch, 2)] {
                    refuse(len, nth);
                    let e = AnnotatedCommit::from_fetchhead(
                        repo.repo(),
                        LONG_BRANCH,
                        LONG_URL,
                        id.as_ref(),
                    )
                    .unwrap_err();
                    assert!(refused(), "{len}");
                    assert_eq!(e.class(), Some(ErrorClass::NoMemory));
                }
                let commit = AnnotatedCommit::from_fetchhead(
                    repo.repo(),
                    LONG_BRANCH,
                    LONG_URL,
                    id.as_ref(),
                )
                .unwrap();
                assert_eq!(
                    read(&commit),
                    (hex(&repo.first), LONG_BRANCH.to_str().ok().map(Into::into))
                );
            },
        );
    }
}

mod from_revspec {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn every_revision_kind() {
        for own in REPO_KINDS {
            let repo = CommitRepo::new(&format!("from-revspec-{own}"), own);
            let mut held = Vec::new();
            let first = CString::new(hex(&repo.first)).unwrap();
            let prefix = CString::new(&hex(&repo.first)[..7]).unwrap();
            let long = CString::new("x".repeat(4096)).unwrap();
            for spec in [
                c"HEAD",
                c"main",
                c"v1",
                c"main~1",
                c"v1^{commit}",
                c":/first",
                first.as_c_str(),
                prefix.as_c_str(),
                c"HEAD^{tree}",
                c"tree",
                c"main~2",
                c"nope",
                c"",
                c"main..feature",
                c"@{-1}",
                c"main@{0}",
                c"^",
                c"~",
                c"@",
                c":",
                c"HEAD^{/sec}",
                long.as_c_str(),
            ] {
                if let Ok(commit) = AnnotatedCommit::from_revspec(repo.repo(), spec) {
                    assert_eq!(read(&commit).1, None);
                    held.push(commit);
                }
            }
            assert!(held.len() >= 8, "{}", held.len());
            // Released in another order than built.
            held.reverse();
            drop(held);
        }
    }
}
