# `commit_parents` said its commits outlive the repository

- **item**: `git_repository_commit_parents` (symbol),
  `crustify/rust/libgit2/libgit2/repository/repository.rs`:
  `RepositoryRef::commit_parents`; the seam doc of
  `CommitArrayRef::commit_ptrs` (`object/commit.rs`)
- **affected commit**: `28fd3b00e` (libgit2/repository wave 1 batch 2)
- **class**: documentation (a contract the next run inherits); no runtime change

## Finding

The wrapper's doc said "The commits outlive the repository", and `object/commit.rs`
"A commit outlives its repository: `git_commit_free` touches only the commit". Both
are true of *releasing* a commit and false of *using* one: a `git_commit` is a
`git_object`, whose `repo` field (`src/libgit2/object.h`) is the repository it was
parsed from, uncounted and never cleared by `git_repository_free`, and
`git_commit_owner`, `git_commit_tree` (`git_tree_lookup(.., commit->object.repo, ..)`),
`git_commit_parent` (`git_commit_lookup(.., commit->object.repo, ..)`),
`git_commit_nth_gen_ancestor`, `git_commit_amend`... read it. It is the shape the
config review found (`../../ub_safe/repository-config-keeps-freed-repository-for-includeif`):
free after the parent is fine, use after it is not.

Today the array hands out raw `*mut git_commit` only (`commit_ptrs`, a seam the
`object` subsystem's run replaces), so no safe path reaches the dangling pointer; the
doc, read as "the commits are independent of the repository", would have let that run
build a `Commit` handle without the repository's borrow.

## Fix

Doc only: `commit_parents` and the `object/commit.rs` module and seam docs now say the
array may be released after the repository but its commits reach it through
`object.repo`, so the commit pointers may be given to those routines only while the
repository lives and the `git_commit` handle must borrow it (as `Reference<'r>` and
`Odb<'r>` do), with the array of such handles carrying the borrow.
