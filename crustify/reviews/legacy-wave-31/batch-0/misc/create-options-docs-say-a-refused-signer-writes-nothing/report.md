# `CreateOptions` docs say a refused signer writes nothing, and read the empty-commit check as the tree routines'

- **item**: `git_commit_create_options` (`CreateOptionsMut::set_signer`,
  `CreateOptionsRef::allow_empty_commit`), in
  `crustify/rust/libgit2/libgit2/object/commit.rs`
- **affected commit**: `8d42785bd` (libgit2/object wave 2 batch 0), reviewed at `d6a6d1f95`
- **class**: documentation defect

## Finding

`CreateOptionsMut::set_signer` documented the signer's failure as "an error fails the
call, nothing written". That holds for the tree routines (`git_commit_create_from_tree`,
`git_commit_amend_from_tree`), but not for the two routines that commit the index, which
the same docs name as readers of these options. `src/libgit2/commit.c`:

```c
/* git_commit_create_from_stage */
if (!opts.allow_empty_commit &&
    (error = check_for_empty_commit(repo, NULL, index)) < 0)
	goto done;

if (git_index_write_tree(&tree_id, index) < 0 ||      /* writes the tree objects */
    git_tree_lookup(&tree, repo, &tree_id) < 0) { ... }

error = create_from_tree(out, repo, tree, message, &opts);  /* signs, then writes */
```

`git_commit_amend_from_stage` does the same before `git_commit_amend_from_tree`. So when
the signer refuses (or panics, `GIT_EUSER`), no commit is written and `HEAD` does not
move, but the index's tree is already in the object database, and stays there. A wave-3
wrapper of these routines written from the options' docs would promise callers a
side-effect-free failure.

`allow_empty_commit` described the check as "when the tree is the one `HEAD` points
at", which only fits the tree routines: `git_commit_create_from_stage` diffs `HEAD`'s
tree against the index (`check_for_empty_commit(repo, NULL, index)`), before writing
anything.

The batch's equivalence and UB tests drove only `git_commit_create_from_tree` and
`git_commit_amend_from_tree`, so neither the stage routines' reading of the options nor
this effect was observed.

## Evidence

`tests/equiv/commit.rs::create_options::commits_created_and_amended_with_the_options_match_c`
now also runs `git_commit_create_from_stage` and `git_commit_amend_from_stage` on both
sides, with the repository's index read from one of the side's trees and, for some steps,
an entry added that makes a tree no object database holds yet (its id is computed in a
third repository). Each step's outcome now includes whether that tree is written. For the
step `Stage(1, true)` with a signer refusing with `-7`:

```
(-7, 26, "signing refused", Some(7), 6, true)
 ^code ^class GIT_ERROR_CALLBACK   ^HEAD unchanged ^calls ^staged tree written
```

and the test asserts `(expected.0, expected.5) == (-7, true)` for that step, while the
step before (`Stage(0, false)`, refused as unchanged with `GIT_EUNCHANGED`, `-38`) leaves
it unwritten.

## Fix

`set_signer` now says an error fails the call before the commit is written or `HEAD`
moves, and that the routines committing the index have written its tree into the object
database by then, and leave it there. `allow_empty_commit` now says the tree compared is
the index's for `git_commit_create_from_stage`, and that the refusal comes before
anything is written. No behaviour changes.
