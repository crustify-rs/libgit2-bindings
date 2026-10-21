# repository: the types it does not own leave its schedule

## What the earlier plan decided

The schema-v1 schedule gave `repository` 3 waves, 18 batches and 98 items. Its wave 0 opened
with seven type batches holding types other subsystems define:

```text
wave 0: b0: git_annotated_commit | b1: git_checkout_options | b2: git_commitarray, git_config
        | b3: git_index | b4: git_odb, git_path_fs | b5: git_path_gitfile, git_refdb
        | b6: git_refdb_t, git_reference | b7: git_repository | ...
```

## Evidence against it

None of these nine types is defined or published by a file `repository` lists:

| type | defined in | owner (lists the file) |
|---|---|---|
| `git_annotated_commit` | `src/libgit2/annotated_commit.h` | `refs` |
| `git_checkout_options` | `include/git2/checkout.h` | `workdir` |
| `git_commitarray` | `include/git2/commit.h` | `object` |
| `git_config` | `src/libgit2/config.h` | `config` |
| `git_index` | `src/libgit2/index.h` | `index` |
| `git_odb` | `src/libgit2/odb.h` | `odb` |
| `git_refdb` | `src/libgit2/refdb.h` | `refs` |
| `git_refdb_t` | `include/git2/refdb.h` | `refs` |
| `git_reference` | `src/libgit2/refs.h` | `refs` |

`include/git2/types.h`, which `repository` lists, only forward-declares the opaque ones. In API
mode an imported item is a dependency of the units that reach it, not a unit; the config whose
files own it schedules it (wavefront `docs/scoping.md`, API mode). Rerun the subsystem's
schedule from its own configuration:

```sh
python3 crustify/reviews/planner/subsystems/repository-schedule-type-ownership/emit_schedule.py repository
/opt/venv/bin/wavefront /target --config crustify/wavefront/configs/libgit2/repository/wavefront-config.json \
  query types --name git_config --imported-only
```

## The edit

`crustify/artifacts/libgit2/repository/schedule.json` (schema v2) has 3 waves, 12 batches and 89
items. Wave 0 keeps only `repository`'s own types:

```text
wave 0: b0: git_path_fs, git_path_gitfile | b1: git_repository
        | b2: git_repository_init_flag_t, git_repository_init_mode_t
        | b3: git_repository_item_t, git_repository_open_flag_t | b4: git_repository_state_t, git_worktree
        | b5: git_worktree_prune_options, git_worktree_prune_t
```

Waves 1 and 2 are unchanged. The homes of the items that stay are unchanged.

## What moves downstream

`repository` has landed, and its work is not relaunched: its artifact dirs under
`crustify/artifacts/libgit2/repository/wave-*/` keep the batches that bound these nine types,
and the Rust tree keeps their anchors at the homes the v1 plan gave them. Each type now sits in
its owner's schedule, at the same home:

- `config` (landed) wave 0 now holds `git_config`, and `index` (landed) wave 0 holds
  `git_index`. Both have filled anchors.
- `object` (landed) wave 2 now holds `git_commitarray`, beside `git_commit_create_options`.
- Pending: `refs` wave 0 holds `git_annotated_commit`, `git_refdb`, `git_refdb_t` and
  `git_reference`; `odb` wave 0 holds `git_odb`; `workdir` wave 2 holds
  `git_checkout_options`. All are anchored already, so their binders skip them.
  `refs` wave 0 batch 2 (`git_refdb_t`, `git_reference`) and `workdir` wave 2 batch 0
  (`git_checkout_options`) hold nothing else; the orchestrator can mark them done instead of
  launching them.

## Question for the user

Should batches whose every item is already anchored be launched at all, or recorded as done
without an agent?
