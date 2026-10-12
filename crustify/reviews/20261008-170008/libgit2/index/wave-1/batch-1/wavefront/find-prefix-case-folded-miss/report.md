# The `git_index_find_prefix` record's `ptr_args[1]` note was incomplete

- **record**: `git_index_find_prefix` (`ptr_args[1]`, `index`), `src/libgit2/index.c`
- **submitted by**: libgit2/index wave 1 batch 2 (`05a56b569`)
- **resubmitted**: yes, through `wavefront … query symbols --name git_index_find_prefix --update`. The
  tool re-sorted part of the store, so the record was spliced into the base layout:
  only this `note` line changed in `crustify/wavefront/ownership-store.json`.

## Previous finding

> Entry vector sorted (index_find), then read. No GIT_ASSERT_ARG on index or prefix. When an entry's path equals the prefix and it has several stages, the bsearch stops at an arbitrary stage, not the first one the header promises (the wrapper backs up).

## Evidence it is incomplete

The record covers the conflict-stage case, but not case-insensitive indexes. There
`index_find` searches in folded order, and only the landed entry is compared with the
byte-wise `git__prefixcmp`. A matching path sorted after a case variant is missed:
`GIT_ENOTFOUND` for `"a/"` in `["A/x", "a/y"]`. The raw call in
`index.rs::unit_tests::symbols::find_prefix_scans_case_insensitive_indexes` shows it.
Report: `misc/find-prefix-misses-case-variants`.

## Corrected finding

> Entry vector sorted (index_find), then read. No GIT_ASSERT_ARG on index or prefix. When an entry's path equals the prefix and it has several stages, the bsearch stops at an arbitrary stage, not the first one the header promises (the wrapper backs up). In an ignore_case index the bsearch runs in folded order but the landed entry is compared byte for byte (git__prefixcmp), so a matching path after a case variant is missed (GIT_ENOTFOUND for 'a/' in ['A/x','a/y']).

The other facets, and `ptr_args[0]` / `ptr_args[2]`, are unchanged.
