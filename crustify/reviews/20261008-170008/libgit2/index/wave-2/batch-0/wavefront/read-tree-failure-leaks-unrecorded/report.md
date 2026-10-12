# The `git_index_read_tree` record understated what a failed read leaks

- **record**: `git_index_read_tree` (`ptr_args[0]`, `index`), `src/libgit2/index.c`
- **submitted by**: libgit2/index wave 2 batch 0 (`cb77d2e9b`)
- **resubmitted**: yes, through `wavefront … query symbols --name git_index_read_tree
  --update`, spliced into the base layout. Only this `note` line changed.

## Previous finding

> Exclusive: clears (frees) every entry and the tree cache, swaps in fresh entries; copies stat data of unchanged old entries. Leaks the new vector and map when the map insert fails (OOM).

## Evidence it is incomplete

- Every failure leaks, not just the out-of-memory map insert. `read_tree_cb` adds each
  entry it creates to the local `entries` vector. When `git_tree_walk` then fails
  (a refused name, a subtree whose object is gone), `git_index_read_tree` jumps to
  `cleanup:` (`index.c:3505`). There `git_vector_dispose(&entries)` frees the array but
  not the entries in it. LeakSanitizer on a temporary probe that reads a tree
  `["a", "q/r"]` whose `q` is gone into an empty index
  (`misc/read-tree-doc-omits-entries-leaked-on-failure/lsan.log`):

  ```
  Direct leak of 98 byte(s) in 1 object(s) allocated from:
      #3 index_entry_create /target/src/libgit2/index.c:993:10
      #4 read_tree_cb /target/src/libgit2/index.c:3430:6
      #5 tree_walk /target/src/libgit2/tree.c:1022:12
      #6 git_tree_walk /target/src/libgit2/tree.c:1048:10
      #7 git_index_read_tree /target/src/libgit2/index.c:3480:15
  ```

- `read_tree_cb` returns before `git_str_dispose(&path)` when `index_entry_create`
  refuses a name such as `.git` (`index.c:3430-3431`), leaking the joined path. The
  batch documented this in `IndexMut::read_tree`, but the store did not record it. The
  name is validated against the *index's* owner, not the tree's repository.
- The tree cache is dropped (`index->tree = NULL`) before the walk, so a failed read
  loses it while keeping the old entries.

## Corrected finding

> Exclusive: drops the tree cache first, then clears (frees) every entry and swaps in fresh ones; copies stat data of unchanged old entries. On any failure the old entries stay but every entry already created leaks (cleanup disposes the new vector without freeing its entries; the map-insert OOM path returns before cleanup, leaking vector and map too); read_tree_cb also leaks the joined path when index_entry_create refuses a name (.git), validated against the index owner.

`ptr_args[1]` (`tree`) is unchanged.
