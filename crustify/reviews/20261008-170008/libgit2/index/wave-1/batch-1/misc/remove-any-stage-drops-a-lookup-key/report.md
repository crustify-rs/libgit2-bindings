# `IndexMut::remove(path, Stage::Any)` passed on libgit2's wildcard, which removes an arbitrary stage and drops another stage's lookup key

- **item**: `git_index_remove`, wrapped as
  `crustify/rust/libgit2/libgit2/index/index.rs::IndexMut::remove` (libgit2/index wave 1
  batch 3, `7f4d85bd7`)
- **affected commit**: `3e694ec8b`
- **class**: libgit2 defect that the wrapper carried into Rust; the documented contract
  was not met. No memory-safety impact was found.

## Finding

The wrapper's doc promised to remove "the entry at `path` at `stage`, or at the first stage
found for `Stage::Any`". It passed `Stage::Any` (`-1`) straight to `git_index_remove`
(`src/libgit2/index.c:1740-1763`), which does two things with it:

1. `GIT_INDEX_ENTRY_STAGE_SET(&remove_key, -1)` stores `-1 & 3`, which is stage **3**. So
   `git_index_entrymap_remove(&index->entries_map, &remove_key)` drops the path's stage 3
   entry from the path lookup table, whichever entry is removed afterwards.
2. `index_find(…, GIT_INDEX_STAGE_ANY)` returns whatever stage the binary search meets
   first. For a three-way conflict that is stage 2, not the first stage.

So on a conflict `a` with stages 1, 2 and 3, C removes stage 2. Stage 3 stays in the
entry vector, but `git_index_get_bypath(index, "a", 3)` no longer finds it. The same goes
for libgit2's own lookups through that table (`git_index_get_bypath(…, -1)` also keys on
stage 3). Nothing frees the orphaned entry early: the table only loses a key. No safe
wrapper reads the table yet (`git_index_get_bypath` is wave 2). Once one does, a
`remove(…, Stage::Any)` silently hides an entry that `entry_count`, `find` and the written
file still show.

`index.rs::unit_tests::edits::removal_at_any_stage_takes_the_first_and_keeps_lookups`
shows both points through the raw call. The remaining entries are `[a:1, a:3]`, and
`git_index_get_bypath` finds stages `[1]` only, not `[1, 3]`.

## Fix

`remove` now resolves `Stage::Any` in Rust before calling libgit2: the stage of the first
entry at `path`, which `find` (`git_index_find`) finds. `git_index_remove` then gets that
concrete stage, removes that entry and drops only its key. With no entry at `path`, the
wildcard is still passed. libgit2 then reports `GIT_ENOTFOUND` ("index does not contain a
at stage -1") as before, and its table holds no entry at `path` that could lose its key. In
a case-insensitive index, `find` folds case, as `git_index_remove` does. The doc states
the divergence.

`ffi::git_index_get_bypath` was added to the `-sys` function allowlist for the
regression, ahead of its wave 2 wrapper.

Regression: `index.rs::unit_tests::edits::removal_at_any_stage_takes_the_first_and_keeps_lookups`.
It covers C's behaviour, then the wrapper removing stages 1, 2 and 3 in turn with the
table kept whole, the `GIT_ENOTFOUND` error, and a case-insensitive index.
`tests/equiv/index/edits.rs::removal_sequences_match` now compares `Stage::Any` steps
against C given the stage `git_index_find` reports (`c_first_stage`). It used to compare
them against C's wildcard. The resulting entries, REUC records and written files are
compared as before.
