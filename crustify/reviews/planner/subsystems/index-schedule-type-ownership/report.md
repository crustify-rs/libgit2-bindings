# index: git_diff and git_diff_delta leave for diff, git_index arrives from repository

## What the earlier plan decided

The schema-v1 schedule's `index` wave 0 opened with a batch of two types `diff` defines,
`b0: git_diff, git_diff_delta`, and `git_index` itself was scheduled in `repository` wave 0
(batch 3).

## Evidence against it

`git_diff` is defined in `src/libgit2/diff.h` and `git_diff_delta` in
`include/git2/diff.h`, both files `diff` lists; `index` reaches them through
`include/git2/pathspec.h`'s `git_pathspec_match_diff`. `git_index` is defined in
`src/libgit2/index.h`, which `index` lists. A wrap schedule holds only the items its own config
owns (wavefront `docs/scoping.md`, API mode):

```sh
python3 crustify/reviews/planner/subsystems/repository-schedule-type-ownership/emit_schedule.py index
/opt/venv/bin/wavefront /target --config crustify/wavefront/configs/libgit2/index/wavefront-config.json \
  query types --name git_diff --imported-only
```

## The edit

`crustify/artifacts/libgit2/index/schedule.json` (schema v2) has 3 waves, 16 batches and 92
items: wave 0 batch 0 is `git_index`, in place of `git_diff, git_diff_delta`. The other
batches are unchanged, and every item keeps its v1 home.

`crustify/wavefront/configs/libgit2/index/wavefront-config.json` also names
`git_note_foreach_cb` (`include/git2/notes.h`) in `out_of_scope.items`. `index` lists
`notes.h` only for `git_note_iterator`. `history` implements `notes.c` and its only
consumer of the callback, `git_note_foreach`, and the v1 plan scheduled the callback there.
It stays there.

## What moves downstream

`index` has landed and is not relaunched. `git_diff` and `git_diff_delta`, anchored at
`crustify/rust/libgit2/libgit2/diff/diff.rs`, now sit in the pending `diff` schedule (wave 0
batch 1 and wave 2 batch 0), where their binders skip them; so do the `git_diff_*` symbols
`index` already bound (`git_diff_free`, `git_diff_from_buffer`, `git_diff_num_deltas`,
`git_diff_from_buffer_ext`, `git_diff_get_delta`, `git_diff_index_to_index`,
`git_diff_index_to_workdir`, `git_diff_tree_to_index`, `git_diff_tree_to_tree`,
`git_diff_tree_to_workdir`, `git_diff_tree_to_workdir_with_index`) and `git_diff_file`.
