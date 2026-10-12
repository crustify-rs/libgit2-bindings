# `IndexMut::find_prefix` missed matching paths in a case-insensitive index

- **item**: `git_index_find_prefix`, wrapped as
  `crustify/rust/libgit2/libgit2/index/index.rs::IndexMut::find_prefix` (libgit2/index
  wave 1 batch 2, `05a56b569`)
- **affected commit**: `3e694ec8b`
- **class**: libgit2 defect that the wrapper carried into Rust; the documented contract
  was not met.

## Finding

The wrapper documents "the position of the first entry whose path starts with `prefix` …;
`None` when no path does … The prefix is compared byte for byte, even when the index
ignores case". `git_index_find_prefix` (`src/libgit2/index.c`) runs `index_find` with
the index's search function. In an `IGNORE_CASE` index that is a case-folded binary
search. It then checks only the entry it lands on, with the byte-wise `git__prefixcmp`.
Entries that differ only in case sort together in folded order. So a byte-wise match
sorted after a case variant is never looked at.

With entries `["A/x", "a/y"]` in an index that ignores case, the search for `"a/"` lands
on `"A/x"`. That entry fails the byte comparison, and C returns `GIT_ENOTFOUND`. The
wrapper returned `None`, even though `"a/y"` starts with `"a/"`. The same index returns
`Some(0)` for `"A/"`. Case-sensitive indexes are not affected: there the landing entry is
the first one at or after the prefix.

## Fix

In an index that ignores case (`IndexRef::capabilities`), `find_prefix` now scans the
entries in index order. It returns the first one whose path starts with `prefix` byte for
byte, through the existing `path_at_starts_with`. That is the documented contract. The
case-sensitive path still uses `git_index_find_prefix` and backs up over conflict stages
as before. The doc states both divergences.

Regressions:

- `index.rs::unit_tests::symbols::find_prefix_scans_case_insensitive_indexes`: C's
  `GIT_ENOTFOUND` for `"a/"`, then the wrapper's positions for `"a/"`, `"A/"`, `"a/y"`,
  `""`, a conflicted `"b"` (its first stage), `"B"` and a missing prefix.
- `tests/equiv/index.rs::assert_same_finds`: in a case-insensitive index, the wrapper is
  compared with the first byte-wise match among C's entries, and with any position C
  finds, which it must not exceed. `in_memory_sequence_matches` runs this through its
  case-insensitive step.
