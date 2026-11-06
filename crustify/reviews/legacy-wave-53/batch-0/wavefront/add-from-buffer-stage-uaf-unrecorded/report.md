# The `git_index_add_from_buffer` record omitted the conflict-stage use after free

- **record**: `git_index_add_from_buffer` (`ptr_args[1]`, `source_entry`),
  `src/libgit2/index.c`
- **submitted by**: libgit2/index wave 2 batch 0 (`cb77d2e9b`)
- **resubmitted**: yes, through `wavefront … query symbols --name git_index_add_from_buffer
  --update`. The tool re-sorted the store, so the record was spliced back into the base
  layout. Only this `note` line changed in `crustify/wavefront/ownership-store.json`.

## Previous finding

> Read for the call: mode must be blob/exec/link; copied with its path, id and file_size replaced by the blob's.

## Evidence it is incomplete

The batch that submitted this note found, and refused in Rust, a use after free that
depends only on this argument. `index_entry_dup` copies the entry's flags, stage bits
included. `index_insert` then inserts the copy at that stage. Next,
`git_index__conflict_to_reuc(index, entry->path)` (`index.c:1560`) finds the copy as the
path's conflict: `git_index_conflict_remove` moves it to the REUC records and frees it.
Finally, `git_tree_cache_invalidate_path(index->tree, entry->path)` (`index.c:1563`)
reads the freed path. The batch's commit message and `IndexMut::add_from_buffer`'s doc
describe this. The regression is
`index.rs::unit_tests::entries::buffers_refuse_conflict_stages_libgit2_would_free_while_adding`.
The store did not record it, though it is the argument's most important ownership
constraint, and "copied with its path" suggests the copy's stage is harmless.

## Corrected finding

> Read for the call: mode must be blob/exec/link; copied with its path, stage bits included, its id and file_size replaced by the blob's. A stage 1-3 entry is a use after free once inserted (owner, valid mode and length, valid path): git_index__conflict_to_reuc treats the new entry as the path's conflict, moves it to REUC and frees it, then git_tree_cache_invalidate_path reads entry->path (index.c:1563).

The other facets, and `ptr_args[0]` / `ptr_args[2]`, are unchanged. The added
precondition ("once inserted") is the one the equivalence report
`equiv/add-from-buffer-stage-refusal-replaces-c-errors` narrows the Rust refusal to.
