# `IndexMut::read_tree` documented only some of libgit2's leaks, and its tests avoided the others

- **item**: `git_index_read_tree`, `crustify/rust/libgit2/libgit2/index/index.rs`
  (`IndexMut::read_tree`)
- **affected commit**: `cb77d2e9b` (libgit2/index wave 2 batch 0)
- **class**: misc (documentation and test coverage of a C leak reachable through the safe
  API; no undefined behaviour, and the Rust side matches C)

## Finding

The doc said:

> libgit2 leaks the joined path of a tree entry whose name it refuses (`read_tree_cb`,
> `src/libgit2/index.c`), and the entries it read when it runs out of memory filling its
> path table (`git_index_read_tree`).

`git_index_read_tree` leaks the entries it already read on **every** failure.
`read_tree_cb` appends each new entry to a local vector. When `git_tree_walk` fails,
`cleanup:` (`index.c:3505`) calls `git_vector_dispose(&entries)`, which frees the array
alone. Any failing read leaks: a missing subtree after a blob, or a refused name after an
accepted one. Both tests of failing reads happened to fail before the first entry. The
broken tree in `ub_safe::index::entries::tree_reads_free_previous_contents_once` is
`["q/r"]`. The refused tree in `refused_tree_entry_names_leave_the_index_as_it_was` lists
`.git` before `ok`. So LeakSanitizer never saw the leak, and the doc and tests implied the
other failures were clean.

LeakSanitizer on a temporary probe (`asan-ubsan` variant, LSan on) reading a tree
`["a", "q/r"]` whose `q` object was deleted, into an empty index (`lsan.log`):

```
==2507535==ERROR: LeakSanitizer: detected memory leaks
Direct leak of 98 byte(s) in 1 object(s) allocated from:
    #2 git__calloc /target/src/util/alloc.c:31:13
    #3 index_entry_create /target/src/libgit2/index.c:993:10
    #4 read_tree_cb /target/src/libgit2/index.c:3430:6
    #5 tree_walk /target/src/libgit2/tree.c:1022:12
    #6 git_tree_walk /target/src/libgit2/tree.c:1048:10
    #7 git_index_read_tree /target/src/libgit2/index.c:3480:15
    #8 <libgit2::libgit2::index::index::IndexMut>::read_tree crustify/rust/libgit2/libgit2/index/index.rs:2141:25
SUMMARY: AddressSanitizer: 98 byte(s) leaked in 1 allocation(s).
```

The wrapper cannot prevent it. Whether the walk fails depends on the object database at
the time of the call and on libgit2's private path validation (`git_path_is_valid`,
against the index's owner), which is not exported.

## Fix

- The doc now says that whenever the read fails, libgit2 leaks the entries read before
  the failure (refused names and missing subtrees after a blob included), as well as the
  joined path of a refused name.
- New test `ub_safe::index::entries::failed_tree_reads_after_entries_leave_the_index_as_it_was`
  reads a tree `["a", "b", "q/r"]` with `q` gone, twice, in a child with LeakSanitizer off
  (`isolated_tolerating_leaks`, as the refused-name test does). It checks that the index
  keeps its previous entries.
- The wavefront record was corrected too
  (`wavefront/read-tree-failure-leaks-unrecorded`).
