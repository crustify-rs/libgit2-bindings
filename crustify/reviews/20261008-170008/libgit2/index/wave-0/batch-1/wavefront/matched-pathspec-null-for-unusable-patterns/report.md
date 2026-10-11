# The callback record said `matched_pathspec` is NULL only for an empty pathspec

- **record**: `git_index_matched_path_cb` (`ptr_args[1]`, `matched_pathspec`), callback in
  `include/git2/index.h`
- **submitted by**: libgit2/index wave 0 batch 8 (`6c09dba0d`)
- **resubmitted**: yes, through `wavefront … query symbols --name git_index_matched_path_cb
  --update`. In `crustify/wavefront/ownership-store.json`, only the two `note` lines of
  this record changed (this report and
  `wavefront/remove-all-path-freed-by-index-mutation`). The tool did not re-sort the
  store.

## Previous finding

> git_pathspec__match initializes match to NULL for an empty pathspec, otherwise lends the
> parsed pattern. Read-only NUL string valid during callback; git_pathspec__clear releases
> it after the synchronous scan.

## Evidence it is incomplete

"Empty pathspec" reads as "no strings supplied". `git_pathspec__match`
(`src/libgit2/pathspec.c:208-214`) writes NULL and returns `true` for an empty **parsed**
vector. Besides a NULL, empty or all-empty array (`git_pathspec_is_empty`, which is
never parsed), `git_pathspec__vinit` (`:80-96`) leaves that vector empty when every entry
is dropped by `git_attr_fnmatch__parse`'s `GIT_ENOTFOUND`
(`src/libgit2/attr_file.c:741-745`): a `#` comment, a leading `\n` or `\r\n`. So for
`["#x"]`, C matches every path and passes NULL, even though a pattern was supplied. The
Rust doc inherited this omission (`misc/matched-pathspec-none-doc-omits-unusable-patterns`,
which holds the observed table).
`index.rs::unit_tests::matched_path::unusable_patterns_match_every_path_without_a_pattern`
pins the behaviour through `git_index_remove_all`.

The record also did not say that an empty entry next to a non-empty one is parsed:
`parse_optimized_patterns` (`attr_file.c:842`) reads `pattern[1]`, one byte past the
empty string's terminator, before the entry is dropped (`misc/pathspec-empty-entry-overread`,
with an ASan reproducer). The add/remove/update wrappers that consume this callback must
refuse that mix as `Pathspec::new` now does.

## Corrected finding

> git_pathspec__match initializes match to NULL and returns true for an empty pattern
> vector: a NULL/empty/all-empty strarray (git_pathspec_is_empty, never parsed), or one
> whose every entry git_attr_fnmatch__parse drops with GIT_ENOTFOUND ('#' comment,
> leading '\n' or '\r\n'), so every path then matches with NULL. An empty entry beside
> a non-empty one is parsed too, and parse_optimized_patterns reads the byte past its
> NUL (UB) before dropping it: wrappers must refuse that mix
> (pathspec::patterns_are_parsable). Otherwise lends the parsed pattern (pool-owned, '!'
> and trailing '/' stripped). Read-only NUL string valid during callback;
> git_pathspec__clear releases it after the synchronous scan.

The ownership, borrow, nullability and mutability facets are unchanged. `ptr_args[2]`
(the payload) was re-read against all three callers and is still accurate.
