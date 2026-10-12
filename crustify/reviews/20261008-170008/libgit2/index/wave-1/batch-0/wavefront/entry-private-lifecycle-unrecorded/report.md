# `git_index_entry` had no lifecycle roles recorded

- **record**: the lifecycle routines of `git_index_entry` (`include/git2/index.h`):
  `index_entry_free`, `index_entry_create`, `index_entry_dup` (`src/libgit2/index.c`)
  and `index_entry_dup_pool` (`src/libgit2/merge.c`)
- **submitted by**: libgit2/index wave 1 batch 0 (`81e92a1ae`), which submitted the type
  record and its `path` field but no symbol record
- **resubmitted**: yes, through `wavefront … query symbols --name <routine> --update`
  for each of the four. `--update` re-sorted the whole store, so the four new records
  were spliced into HEAD's layout (`indent=1`, `ensure_ascii`). The store diff is 250
  insertions and no deletions.

## Previous finding

`query types --name git_index_entry --lifecycle-ops` and
`query symbols --lifetime-for git_index_entry` both listed nothing:

```json
{"type": "git_index_entry", "dropped_by": [], "fields_disposed_by": [],
 "cloned_by": [], "constructed_by": []}
```

All four routines had `submitted: false`, with every pointer slot pending. The type's own
`path` note names `index_entry_create` and `index_entry_free` as the allocator and
releaser of the entries an index holds, but no record carried those roles.

## Evidence it is incomplete

The sibling type batch for `git_index_reuc_entry` (wave 1 batch 1) recorded its private
`reuc_entry_alloc` / `index_entry_reuc_init` / `index_entry_reuc_free`. The entry
routines meet the schema's rule for recording a private primitive, which is more than one
caller (`query symbols --callers`):

| routine | role | callers |
|---|---|---|
| `index_entry_free` | dropper (`git__free` of the `entry_internal` block, path inline, id zeroed first; NULL is a no-op) | 9: removal, replacement, read/read_tree, `index_free_deleted`, failed additions |
| `index_entry_create` | `alloc` constructor through `out` (calloc of struct + path; untouched on an invalid path, size overflow or OOM) | 5: `index_entry_init`, `index_entry_dup`, `index_entry_dup_nocache`, `add_repo_as_submodule`, … |
| `index_entry_dup` | deep cloner of `src` (create, then `index_entry_cpy` keeps the new path) | 5: `git_index_add`, `git_index_add_from_buffer`, `git_index_conflict_add`, `git_index__fill`, … |
| `index_entry_dup_pool` | deep cloner of `src` into caller storage, path in a `git_pool` (NULL `src` copies nothing) | 2: `merge_diff_from_index_entries`, `merge_diff_list_insert_unmodified` |

These are the routines behind the type's Rust design: entries a caller builds are only
read by `index_entry_dup`, never retained, and entries an index holds are allocated and
freed only by the index. That is why `IndexEntry<'p>` has no teardown and index-owned
entries are only lent.

## Corrected finding

Submitted with full pointer blocks: `index_entry_free` (`for: entry`, `is_dropper`),
`index_entry_create` (`for: out`, `is_constructor.alloc`; `repo` mutable, because the
`core.protectHFS` / `protectNTFS` configmap lookups cache into it), `index_entry_dup` and
`index_entry_dup_pool` (`for: src`, `is_cloner.deep`). `--lifetime-for git_index_entry`
now reads:

```json
{"dropped_by": ["index_entry_free"], "cloned_by": ["index_entry_dup", "index_entry_dup_pool"],
 "constructed_by": ["index_entry_create"]}
```

Not recorded, on purpose:

- `git_index_entry__init_from_stat` fills only the stat fields (not path, id or flags),
  so it is not a constructor.
- `index_entry_cpy` copies fields into an existing entry, keeping its path, so it produces
  no new object.
- `index_entry_init` and `index_entry_dup_nocache` each have one caller, and both call
  the recorded `index_entry_create`.

No Rust change follows: the public API still hands out no entry to free.
