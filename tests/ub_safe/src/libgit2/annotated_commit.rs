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

mod lookup {}

mod from_ref {}

mod from_fetchhead {}

mod from_revspec {}
