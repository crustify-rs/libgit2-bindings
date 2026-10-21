# object: git_filter_list leaves for filter, git_commitarray arrives from repository

## What the earlier plan decided

The schema-v1 schedule gave `object` 4 waves, 21 batches and 173 items, with wave 0:

```text
wave 0: b0: git_blob_filter_flag_t, git_commit_header | b1: git_commitbuilder, git_filter_list
        | b2: git_mailmap, git_message_trailer | b3: git_object, git_tree_entry
        | b4: git_tree_update_t, git_treebuilder | b5: git_treewalk_mode
        | b6: git_blob_data_is_binary, git_tag_name_is_valid
```

and wave 2 batch 0 holding `git_commit_create_options` alone. `git_commitarray` was
scheduled in `repository` wave 0.

## Evidence against it

`git_filter_list` is defined in `src/libgit2/filter.c`, a file `filter` lists; `object`
reaches it only through a field of `include/git2/object.h`'s `git_object_id_options`. `git_commitarray` is defined in
`include/git2/commit.h`, which `object` lists. A wrap schedule holds only the items its own
config owns (wavefront `docs/scoping.md`, API mode):

```sh
python3 crustify/reviews/planner/subsystems/repository-schedule-type-ownership/emit_schedule.py object
/opt/venv/bin/wavefront /target --config crustify/wavefront/configs/libgit2/object/wavefront-config.json \
  query types --name git_filter_list --imported-only
```

## The edit

`crustify/artifacts/libgit2/object/schedule.json` (schema v2) has 4 waves, 20 batches and 173
items. Without `git_filter_list`, wave 0's types pack into one batch fewer:

```text
wave 0: b0: git_blob_filter_flag_t, git_commit_header | b1: git_commitbuilder, git_mailmap
        | b2: git_message_trailer, git_object | b3: git_tree_entry, git_tree_update_t
        | b4: git_treebuilder, git_treewalk_mode | b5: git_blob_data_is_binary, git_tag_name_is_valid
```

and wave 2 batch 0 is `git_commit_create_options, git_commitarray`, where `git_commitarray`
first has its dependencies. Waves 1 and 3 are unchanged; every item keeps its v1 home.

## What moves downstream

`object` has landed and is not relaunched. `git_filter_list`, already anchored at
`crustify/rust/libgit2/libgit2/filter/filter.rs`, now sits in the pending `filter` schedule's
wave 0 batch 0, beside `git_filter_flag_t`; `filter`'s lifecycle routines
`git_filter_list_free`, `git_filter_list_load`, `git_filter_list_new` and
`git_filter_list_load_ext` are anchored too, and their binders skip them.
