# config: git_transaction leaves for refs, git_config arrives from repository

## What the earlier plan decided

The schema-v1 schedule's `config` wave 0 was:

```text
wave 0: b0: git_config_backend_memory_options, git_config_level_t | b1: git_configmap_t, git_transaction
        | b2: git_config_parse_bool, git_config_parse_int32, git_config_parse_int64
```

and `git_config` itself was scheduled in `repository` wave 0 (batch 2, beside
`git_commitarray`).

## Evidence against it

`git_transaction` is defined in `src/libgit2/transaction.c`, a file `refs` lists; `config`
reaches it only through `include/git2/config.h`'s `git_config_lock`. `git_config` is
defined in `src/libgit2/config.h`, a file `config` lists. A wrap schedule holds only the items
its own config owns (wavefront `docs/scoping.md`, API mode):

```sh
python3 crustify/reviews/planner/subsystems/repository-schedule-type-ownership/emit_schedule.py config
/opt/venv/bin/wavefront /target --config crustify/wavefront/configs/libgit2/config/wavefront-config.json \
  query types --name git_transaction --imported-only
```

## The edit

`crustify/artifacts/libgit2/config/schedule.json` (schema v2) keeps 6 waves, 13 batches and 56
items. Wave 0 becomes:

```text
wave 0: b0: git_config, git_config_backend_memory_options | b1: git_config_level_t, git_configmap_t
        | b2: git_config_parse_bool, git_config_parse_int32, git_config_parse_int64
```

Waves 1 to 5 are unchanged. `git_config` keeps the home the v1 plan gave it,
`crustify/rust/libgit2/libgit2/config/config.rs`.

## What moves downstream

`config` has landed and is not relaunched; its artifact dirs keep the batch that bound
`git_transaction`, which is anchored at `crustify/rust/libgit2/libgit2/refs/transaction.rs`.
The pending `refs` schedule now holds `git_transaction` alone in wave 0 batch 5, and
`git_transaction_new`/`git_transaction_free` in wave 1 batch 4, all three already anchored. Batch 5
can be recorded as done without an agent. See also
`crustify/reviews/planner/subsystems/repository-schedule-type-ownership/`.
