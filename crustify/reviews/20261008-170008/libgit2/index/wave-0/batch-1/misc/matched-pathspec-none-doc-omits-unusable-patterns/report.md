# `MatchedPathCallback` said the pattern is `None` only when no patterns were supplied

- **item**: `git_index_matched_path_cb` (callback), wrapped as
  `crustify/rust/libgit2/libgit2/index/index.rs::MatchedPathCallback` (libgit2/index
  wave 0 batch 8, `6c09dba0d`)
- **affected commit**: `ab3dabdcc` (the review wave's base)
- **class**: documentation defect. The trampoline passes C's pointer through unchanged
  (`None` exactly when C passes NULL), so only the contract text was wrong.

## Finding

The wrapper's doc said:

> `matched_pathspec` is the parsed pattern matching it, or `None` when no patterns were
> supplied.

All three C callers get the pattern from `git_pathspec__match`
(`src/libgit2/pathspec.c:196-230`). It sets `*matched_pathspec = NULL` and returns
`true` whenever the *parsed* pattern vector is empty. That happens for an empty or
all-empty array, which `git_pathspec_is_empty` keeps from being parsed at all. It also
happens when `git_pathspec__vinit` (`:67-103`) drops every entry because
`git_attr_fnmatch__parse` answers it with `GIT_ENOTFOUND`. Per
`src/libgit2/attr_file.c:741-745`, those entries are a `#` comment and a pattern starting
with `\n` or `\r\n`. So with patterns supplied but none of them usable, libgit2 matches
**every** path and hands the callback NULL. If a usable pattern is in the list, the
unusable ones are ignored and the usable one filters as usual.

A consumer who trusted the old sentence could reason "I passed patterns, so the second
argument is always `Some`", and so would miss that a pathspec such as `[""]` or
`["#generated"]` matches the whole index. In `remove_all`, that means removing every
entry.

Observed on the `plain` prebuild through `git_index_remove_all` on an in-memory index
holding `a` and `b`, with a callback that records its arguments and skips:

| pathspec | paths seen, with pattern |
|---|---|
| `[""]`, `["", ""]` | `a` (None), `b` (None) |
| `["#a", "\nb", "\r\nb"]` | `a` (None), `b` (None) |
| `["#a", "b"]` | `b` (`Some("b")`) |

A first draft of the regression test also passed `""` next to the comment patterns. ASan
aborted in libgit2 on it, a 1-byte over-read in `parse_optimized_patterns`. That defect is
filed and fixed separately as `misc/pathspec-empty-entry-overread`, and the tests here do
not mix empty and non-empty patterns.

## Fix

The doc now reads: `matched_pathspec` is the parsed pattern matching the path, or `None`
when the pathspec holds no usable pattern (it is empty, its entries are all empty, or
each is a `#` comment or starts with a line break), and then every path matches.

Regressions:

- `index.rs::unit_tests::matched_path::unusable_patterns_match_every_path_without_a_pattern`
  asserts the table above through `scan_with` and `git_index_remove_all`.
- `tests/equiv/index.rs::matched_path_callback::c_scan_matches_public_callback_results_arguments_and_mutations`
  adds three steps, `["", ""]`, `["#a", "\nb", "\r\nb"]` and `["#a", "c"]`, before the
  steps that empty the index. For each step it compares C's reference callback with the public
  `MatchedPathCallback::call` on returns, errors, arguments and index state.

The Wavefront record carried the same omission:
`wavefront/matched-pathspec-null-for-unusable-patterns`.
