# A `FETCH_HEAD` entry of an id and a tab alone inherits the previous entry's merge marker, undocumented

- **item**: `git_repository_fetchhead_foreach` (symbol),
  `crustify/rust/libgit2/libgit2/repository/repository.rs`:
  `RepositoryRef::for_each_fetchhead`'s doc, and `FetchheadEntry::is_merge`
- **affected commit**: `9468d5210` (libgit2/repository wave 2 batch 0); reviewed at
  `16d2fcaae`
- **class**: documentation omits a C behaviour the wrapper passes through; no code
  change (the behaviour is libgit2's and not a defect the wrapper should mask),
  equivalence evidence in the test named below

## Finding

`fetchhead_ref_parse` (`fetchhead.c:170`) sets `*is_merge` in two places only: to 1 for a
line without a tab (the old loose-ref format), and from the marker field when there is
text after the first tab. A line of an id, a tab and nothing else takes neither path,
and `git_repository_fetchhead_foreach` keeps one `is_merge` across lines (initially 0),
so such an entry is reported with the marker of the entry before it: "not for merge"
as a first entry or after a `not-for-merge` line, "for merge" after an unmarked line.
The wrapper's doc described the marker as "not marked `not-for-merge`", which such a
line is, and said nothing of the carry-over. The C coverage of
`fetchhead_ref_parse:215` (the `*line` test's false arm with a tab present) was reached
by no test.

## Fix

`for_each_fetchhead`'s doc now states it. Equivalence:
`tests/equiv/repository.rs::head_and_lookups::fetchhead_entry_without_a_marker_keeps_the_previous_one`
compares the three cases against a C callback and asserts the inherited values.
