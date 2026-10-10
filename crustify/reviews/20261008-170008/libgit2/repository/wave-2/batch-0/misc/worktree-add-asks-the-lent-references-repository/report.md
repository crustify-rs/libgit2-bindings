# `Worktree::add` let a branch be checked out in two working trees when the reference was another repository's

- **item**: `git_worktree_add` (symbol),
  `crustify/rust/libgit2/libgit2/repository/worktree.rs`: `Worktree::add`, with the
  `reference` field of `git_worktree_add_options` (`AddOptionsRef::reference` /
  `AddOptionsMut::set_reference`, wave 1 batch 1) as the way a reference of another
  repository reaches it
- **affected commit**: `9468d5210` (libgit2/repository wave 2 batch 0, which wrapped
  `git_worktree_add`); reviewed at `16d2fcaae`
- **class**: a libgit2 defect copied into the safe API (wrong repository asked by the
  "already checked out" check); reproducer in `src/main.rs` (`cargo run` exits 1 on the
  affected commit, `run-affected.log`; 0 once fixed), no sanitizer involved

## Finding

`git_worktree_add` (`worktree.c:336`) takes the options' `ref` as the branch the new
worktree checks out, and refuses one that another working tree already holds:

```c
	if (git_branch_is_checked_out(ref)) {
		git_error_set(GIT_ERROR_WORKTREE, "reference %s is already checked out", ...
```

`git_branch_is_checked_out` (`branch.c`) walks the working trees of
`git_reference_owner(branch)`, the repository the reference was *read through*, not of
`repo`, the repository the worktree is *added to*. Everything else the routine does
with the reference is by name (`git_reference_is_branch`, `git_reference_dup`,
`git_refdb_init(refdb, git_reference_name(ref), ..)`), which is why the wrapper's doc
could say that a reference of another repository "lends only its name": the branch of
that name in `repo` is what the new worktree's `HEAD` points at and what
`git_checkout_head` checks out. The check, though, concerns the other repository's
working trees, so:

- with `repo`'s branch `feature` checked out in one of `repo`'s working trees (added
  with the other repository's `refs/heads/feature`), a second add with that reference
  succeeds, and two of `repo`'s working trees hold `feature` (`git worktree add`
  refuses this; `git_worktree_add` refuses it with `repo`'s own reference);
- with the other repository's `feature` checked out in *its* working tree, an add to
  `repo`, whose `feature` is free, is refused "already checked out".

Measured on the C (`tests/equiv/worktree.rs::add_options::adds_with_another_repositorys_reference_match_c`,
step `x4`, and the unit test below): a second worktree on the checked-out branch, both
`worktrees/*/HEAD` reading `ref: refs/heads/feature`. The reproducer does the same
through the safe API alone (the fixture's commit and lookup go through `ffi`, as no
safe `git_odb_write` / `git_reference_lookup` exists in this wave).

The batch's `Worktree::add` passed the options through unchanged, so the safe API
inherited both halves of the defect; its doc described the lent reference as
contributing its name only, which is true of the checkout and false of the check.

## Fix

`Worktree::add` now asks whether the reference's owner (`git_reference_owner`) is
`repo`; when it is another repository, it looks the reference's name up in `repo`
(`git_reference_lookup`, a documented raw seam until the reference routines are
wrapped) and gives libgit2 a copy of the options with that reference in place of the
lent one, so `git_branch_is_checked_out` walks `repo`'s working trees. When `repo` has
no reference of that name (or the lookup fails otherwise), the options go through as
given and libgit2 fails as it would (`GIT_EUNBORNBRANCH` from the checkout, after the
worktree's files are written), which the equivalence test keeps comparing against C.
The looked-up reference is released after the call (ASan/LSan:
`tests/ub_safe/worktree.rs::adding::lent_references_are_read_for_the_call_only`).

Regression: `worktree.rs::unit_tests::adding::add_checks_the_branch_in_the_repository_added_to`
(Rust refuses the second worktree and allows the add to a third repository whose
branch is free, C does the reverse, shown alongside). Equivalence of the unaffected
cases (a lent name `repo` lacks, a lent tag, a lent free branch):
`tests/equiv/worktree.rs::add_options::adds_with_another_repositorys_reference_match_c`,
whose last step records the divergence. The module docs of `worktree.rs` list the
defect with the four the wave 1 wrappers correct; `Worktree::add`'s doc describes
what a lent reference contributes.
