# The `git_index_write` record's `ptr_args[0]` note was incomplete

- **record**: `git_index_write` (`ptr_args[0]`, `index`), `src/libgit2/index.c`
- **submitted by**: libgit2/index wave 1 batch 3 (`7f4d85bd7`)
- **resubmitted**: yes, through `wavefront … query symbols --name git_index_write --update`. The
  tool re-sorted part of the store, so the record was spliced into the base layout:
  only this `note` line changed in `crustify/wavefront/ownership-store.json`.

## Previous finding

> Writes the file through a lock; counts a reference for the call. Sorts entries/REUC, sets extended flags, resets racily-clean entries' file_size (owner with workdir only), stamp/checksum/dirty. Frees no entry.

## Evidence it is incomplete

The record lists the racily-clean reset among the routine's effects, but not how it is
done. `truncate_racily_clean` (`src/libgit2/index.c:770-822`) runs before the write when
the owner has a working directory. It passes the paths of the candidate entries (not
`UPTODATE`, not gitlinks, at or after the read stamp) to `git_diff_index_to_workdir` as
a pathspec, which `git_pathspec__vinit` parses. An index file can hold an entry with an
empty path (`read_entry` accepts it). Beside another candidate, that empty pattern makes
`parse_optimized_patterns` read one byte past the entry's allocation. The wrapper relied
on the record's "frees no entry" and checked nothing. ASan trace and
`#![forbid(unsafe_code)]` reproducer: `ub_safe/index-write-empty-path-overread`.

## Corrected finding

> Writes the file through a lock; counts a reference for the call. Sorts entries/REUC, sets extended flags, resets racily-clean entries' file_size (owner with workdir only), stamp/checksum/dirty. Frees no entry. Before writing, truncate_racily_clean passes the paths of entries neither UPTODATE nor gitlinks and newer than the read stamp to git_diff_index_to_workdir as a pathspec: an empty-path entry (an index file can hold one) beside another is parsed and parse_optimized_patterns reads one byte past it (UB); wrappers must refuse that mix.

The ownership, borrow, nullability and mutability facets are unchanged.
