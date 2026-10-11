# `ParentCallback::walk_with` reports a failed walk after libgit2 wrote the commit and moved the reference

- **item**: `git_commit_parent_callback` (callback), `crustify/rust/libgit2/libgit2/object/sys/commit.rs`
- **affected**: the crate-private `ParentCallback::walk_with`, the seam the
  `git_commit_create_from_callback` wrapper is to run its parents through (the public
  `ParentCallback::call` is unaffected)
- **affected commit**: `846e7d391` (libgit2/object wave 1 batch 5; reviewed at `38e6d1b3e`)
- **class**: internal defect — an error result for an operation that took effect

## Finding

When the closure panicked, or returned an id of another type than the repository's,
the trampoline could not stop libgit2 (anything but NULL is a parent, NULL ends the
list and the commit is written), so it handed out the repository's zero id once, then
NULL, and `walk_with` returned the refusal (or resumed the panic) afterwards. The
module docs relied on strict object creation to make libgit2 fail on that zero parent
"before writing anything". With `GIT_OPT_ENABLE_STRICT_OBJECT_CREATION` turned off,
`validate_tree_and_parents` checks no parent: libgit2 writes a commit whose parent is
the zero id and, when the reference to update is unborn, creates it pointing at that
commit, while `walk_with` reports `GIT_ERROR_INVALID` (or panics). The batch's own test
`refusals_end_the_list_without_strict_creation` pinned the written commit; a caller
told the call failed would not know a reference moved.

The new unit test, on the affected commit (strict creation off, in a child, updating
the unborn `refs/heads/main`):

```
thread '...::failed_walks_write_nothing_without_strict_creation' panicked at libgit2/libgit2/object/sys/commit.rs:606:17:
libgit2 ran: Some(Ok(Oid { kind: Sha1, id: 52c9316dbb8b77a9b90a60cb23a1f79725a84021000000000000000000000000 }))
```

and a probe of the same state found `refs/heads/main` = `52c9316d…` and that commit's
parents `["0000000000000000000000000000000000000000"]`.

## Fix

`walk_with` now asks the closure for every parent first, from index 0 until `None`,
refusing an id of another type than the repository's there; a panic propagates from
it. Only then does `walk` run, with a trampoline over the collected ids, which can
neither fail nor panic, so libgit2 writes the commit with exactly those parents or
fails on its own; a refusal or a panic never lets it run, whatever its options. The
zero-id stop, its state (`kind`, `failure`, `stopped`) and the `Failure` enum are gone;
the trampoline only keeps a panic of `call`'s closure, and `Debug` prints `panicked`.
The closure is now asked for its parents before libgit2 looks up the reference to
update, rather than after (documented on `walk_with`).

Regression tests (inline `unit_tests`):

- `failed_walks_write_nothing_without_strict_creation` (replacing
  `refusals_end_the_list_without_strict_creation`): a refusal and a panic, strict
  creation off, updating an unborn branch: libgit2 never runs, nothing is written, the
  branch stays unborn. Fails on the affected commit as above.
- `walks_collect_every_parent_first` (also under Miri): every parent is collected
  before the walk, which reads each in turn and then NULL; a panic stops it before
  the walk.
- `ids_of_another_type_are_refused`, `walks_propagate_panics_before_c` (renamed from
  `walks_resume_panics_after_c`), `walks_give_libgit2_each_parent` and
  `walks_updating_a_reference_check_its_tip` hold as before.
