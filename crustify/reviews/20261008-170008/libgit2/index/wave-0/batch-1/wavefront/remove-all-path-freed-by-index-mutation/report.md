# The callback record said the index releases `path` "later", but a callback that mutates the index frees it at once

- **record**: `git_index_matched_path_cb` (`ptr_args[0]`, `path`), callback in
  `include/git2/index.h`
- **submitted by**: libgit2/index wave 0 batch 8 (`6c09dba0d`)
- **resubmitted**: yes, together with
  `wavefront/matched-pathspec-null-for-unusable-patterns`. Only the two `note` lines of
  the record changed.

## Previous finding

> apply_each_file borrows delta->old_file.path; index_apply_to_all borrows entry->path.
> Non-null NUL string, immutable during callback only; diff/index release later. No
> transfer or retention.

## Evidence it is incomplete

"diff/index release later" is true of `apply_each_file`. There the path is a copy in the
diff's pool (`diff_delta__alloc`, `src/libgit2/diff_generate.c:47`), released by
`git_diff_free` after the scan. In `index_apply_to_all` (`src/libgit2/index.c:3835-3900`,
which serves `git_index_remove_all`), `path` is `entry->path`. That buffer is allocated
inline with the entry (`index_entry_create`, `:991-998`). The scan takes no reader count,
so if the callback removes or replaces that entry, `index_remove_entry` (`:565-570`)
calls `index_entry_free` immediately. The callback could do that through
`git_index_remove*`, `git_index_add*` or `git_index_clear`, or through another reference
to the same refcounted index. After that, the borrowed `path` dangles for the rest of the
call. If the callback then returns 0, C itself reads the freed entry as well
(`git_str_sets(&path, entry->path)`, `:3869`).

A temporary probe (not kept) showed this under the `asan-ubsan` prebuild. It was a raw
C-ABI callback that called `git_index_remove(index, path, 0)`, then `strlen(path)`, and
returned 1 (skip), during `git_index_remove_all` on an in-memory index. The trace is in `asan-probe.log`,
symbolized with `llvm-symbolizer-18`:

- the read happened in the callback, called from `index_apply_to_all` (`index.c:3859`, the
  `cb(entry->path, …)` call)
  via `git_index_remove_all` (`:3910`);
- the buffer was freed by `git__free` ← `index_entry_free` (`:347`) ←
  `index_remove_entry` (`:569`) ← `git_index_remove` (`:1758`) inside the callback;
- the buffer had been allocated by `index_entry_create` (`:993`) ← `index_entry_dup` ←
  `git_index_add` (`:1734`).

No safe Rust path reaches this today: `MatchedPathCallback` is driven only through
`call` (strings the caller lends) and the crate-private `scan_with` seam. The seam's
contract already requires the scan to "keep both input strings immutable for each call".
The `git_index_remove_all` / `git_index_update_all` / `git_index_add_all` wrappers
scheduled in a later wave read this record to decide what the closure may capture. With
the old note they could have let the closure reach the scanned index through a second
`IndexArc` or a repository's `index()`.

## Corrected finding

> apply_each_file borrows delta->old_file.path (diff-owned, freed by git_diff_free after
> the scan); index_apply_to_all borrows entry->path, allocated inline with the index
> entry. Non-null NUL string, immutable during callback only; no transfer or retention.
> index_apply_to_all takes no reader count, so a callback that removes or replaces
> entries of the scanned index (git_index_remove*/add*/clear, also through another
> reference to the refcounted index) frees entry->path at once (index_remove_entry ->
> index_entry_free): wrappers must keep the index unreachable from the callback.

The facets are unchanged (borrowed, non-null, immutable string).
