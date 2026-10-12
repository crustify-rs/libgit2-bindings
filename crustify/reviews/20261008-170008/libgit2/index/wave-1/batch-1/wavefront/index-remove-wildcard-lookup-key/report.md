# The `git_index_remove` record's `ptr_args[0]` note was incomplete

- **record**: `git_index_remove` (`ptr_args[0]`, `index`), `src/libgit2/index.c`
- **submitted by**: libgit2/index wave 1 batch 3 (`7f4d85bd7`)
- **resubmitted**: yes, through `wavefront … query symbols --name git_index_remove --update`. The
  tool re-sorted part of the store, so the record was spliced into the base layout:
  only this `note` line changed in `crustify/wavefront/ownership-store.json`.

## Previous finding

> Removes and frees one entry (deferred to index->deleted while readers>0); stage -1 matches any stage.

## Evidence it is incomplete

"stage -1 matches any stage" is incomplete. `git_index_remove`
(`src/libgit2/index.c:1740-1763`) first builds the lookup key with
`GIT_INDEX_ENTRY_STAGE_SET(&remove_key, -1)`, which stores stage 3. It removes that key
from `entries_map` before `index_find(…, GIT_INDEX_STAGE_ANY)` picks the entry to free,
and the binary search picks stage 2 of a three-way conflict. Afterwards the kept stage 3
entry is missing from `git_index_get_bypath`. The raw probe in
`index.rs::unit_tests::edits::removal_at_any_stage_takes_the_first_and_keeps_lookups`
shows `[a:1, a:3]` left, with only stage 1 found by path. Report:
`misc/remove-any-stage-drops-a-lookup-key`.

## Corrected finding

> Removes and frees one entry (deferred to index->deleted while readers>0). Stage -1 (GIT_INDEX_STAGE_ANY) removes whichever stage index_find's bsearch meets (stage 2 of a three-way conflict), and first drops the key (path, stage 3) from entries_map, since GIT_INDEX_ENTRY_STAGE_SET(-1) stores stage 3: a kept stage-3 entry then vanishes from git_index_get_bypath. Wrappers should resolve the wildcard to a concrete stage.

The other facets, and `ptr_args[1]` (`path`), are unchanged.
