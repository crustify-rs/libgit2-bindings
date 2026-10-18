# `git_worktree_add`'s record did not say which repository its "already checked out" check asks

- **item**: `git_worktree_add` (symbol), the `opts` argument's note in
  `crustify/wavefront/ownership-store.json`
- **affected commit**: `9468d5210` (libgit2/repository wave 2 batch 0, which submitted
  the record); reviewed at `16d2fcaae`
- **class**: incomplete analysis finding (a hazard of the `ref` field the record did
  not name), resubmitted with `query symbols --name git_worktree_add --update`

## Finding

The record said of `opts` that its "ref is git_reference_dup'ed, checkout options
copied and read for the inner git_checkout_head", which reads as if the reference
contributed its name alone. It also reaches `git_branch_is_checked_out(ref)`
(`worktree.c:353`), which walks the working trees of `git_reference_owner(ref)`
(`branch.c`, through `git_repository_foreach_worktree`), the repository the reference
was read through: for a reference of another repository, the refusal "reference %s is
already checked out" concerns the wrong repository's working trees (evidence:
`../misc/worktree-add-asks-the-lent-references-repository/`, and the unit test
`worktree.rs::unit_tests::adding::add_checks_the_branch_in_the_repository_added_to`,
whose C half adds a second worktree on a branch `repo` already has checked out). The
wrapper written from the record inherited the defect.

## Fix

The note now states which repository the check asks and what that does to a lent
reference, and that the wrapper substitutes `repo`'s own reference of the name when it
has one. The other six records of the workset (`git_repository_fetchhead_foreach`,
`git_repository_init_ext`, `git_repository_new_ext`, the three `*_options_init`) were
read against the C and hold: the `FETCH_HEAD` walk's `gitdir`-only read and
synchronous callback, `init_ext`'s writes to `opts->flags` and the live repository a
late failure leaves in `*out`, `new_ext`'s read-only options and
`GIT_ASSERT_ARG` on an unnamed type, the template copies of the init routines.
