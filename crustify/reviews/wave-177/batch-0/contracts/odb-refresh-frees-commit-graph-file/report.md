# `Odb`'s thread-safety docs name `git_odb_set_commit_graph` as the only safe-API hazard to the commit-graph file, but `git_odb_refresh` frees it too

- **items**: `git_odb_set_commit_graph` (bound as the `unsafe fn
  OdbRef::clear_commit_graph`) and `git_odb_refresh` (bound as the safe `OdbRef::refresh`),
  with the `Send`/`Sync` justification of `Odb` and the module docs' *Threads* section,
  `src/libgit2/src/libgit2/odb.rs`
- **affected commit**: `b043039e2` (wave 176 batch 2)
- **objective**: `contracts`. The justification's reasoning is incomplete. It does not hold
  for the future wrappers of the commit walks, which it leaves without the obligation. No safe
  path breaks it today, because nothing in the crate reads the commit-graph file, so there is
  no reproducer. The evidence is libgit2's source, cited below.

## What is wrong

Batch 2 made `clear_commit_graph` unsafe because of this hazard: "The routine frees the graph
under the odb's lock, but libgit2's revision walk reads the graph's file after that lock is
released, so a walk running meanwhile would read freed memory." `Odb`'s `Send` justification
and the *Threads* section present it, with `git_odb_write_multi_pack_index`, as the only
routines that "break that locking".

The same free happens on a path the safe API reaches all the time:

- `git_odb_refresh` (`c/src/libgit2/odb.c:1684`) calls `git_commit_graph_refresh(db->cgraph)`
  under the odb's lock (`odb.c:1708`);
- `git_commit_graph_refresh` (`c/src/libgit2/commit_graph.c:520`) frees `cgraph->file`
  (`git_commit_graph_file_free`) when `git_commit_graph_file_needs_refresh` reports that the
  file changed on disk;
- `git_odb_refresh` runs on every lookup that misses before it looks again: `git_odb_read`
  (`odb.c:1172`), `git_odb_read_prefix` (`:1334`), `git_odb__read_header_or_object` (`:1052`),
  `git_odb_exists_ext` without `GIT_ODB_LOOKUP_NO_REFRESH` (`:799`),
  `git_odb_exists_prefix` (`:885`), and `git_odb__freshen` (`:769`), which `git_odb_write`
  calls for an object the odb lacks. These are `OdbRef::refresh`, `read`, `read_prefix`,
  `read_header`, `exists`, `exists_ext`, `exists_prefix` and `write`, all safe and all callable
  from any thread through `Odb: Sync`;
- the reader is the one batch 2 names: `git_commit_list_parse`
  (`c/src/libgit2/commit_list.c:180`) takes the file from `git_odb__get_commit_graph_file`
  (`odb.c:708`), which releases the lock before returning, and then reads it with
  `git_commit_graph_entry_find` / `_entry_parent`.

So in C, a revision walk on one thread races any odb lookup miss on another whenever
`objects/info/commit-graph` is rewritten (by `git commit-graph write`, `git gc`, or a
fetch). That is a libgit2 race. It contradicts `docs/threading.md`'s "thread-safe to use from
multiple threads simultaneously" for the odb.

The crate stays sound today only because no wrapper reaches `git_commit_list_parse`. The
revision walk, `git_merge_base*` and `git_graph_*` are not bound, and `git_merge_bases` is
only allowlisted, for a unit test. The justification gave its readers the wrong rule, though:
that freeing the commit-graph is dangerous only through `git_odb_set_commit_graph`, which the
`unsafe` contract fences off. Nothing told the wrappers of the walks that they must not
overlap another thread's use of the odb.

## Fix

Documentation and justification only. The commit-graph free is the same as before, and
`clear_commit_graph` stays `unsafe`: its free is unconditional, while `refresh`'s depends on
the file changing on disk.

- The module docs' *Threads* section gains a paragraph that names `git_odb_refresh` as the
  third path that frees the file, lists the safe methods that reach it, cites
  `git_commit_list_parse` as the reader past the lock, and states the obligation of the
  wrappers of the commit walks (`git_revwalk_*`, `git_merge_base*`, `git_graph_*`, and what
  calls them): a walk must not overlap another thread's use of its odb.
- `Odb`'s `Send` justification says why `git_odb_refresh`'s free keeps it sound today:
  only a commit walk reads the file past the lock, and no safe method reaches one.
- `OdbRef::refresh` and `OdbRef::clear_commit_graph` cross-reference the hazard.
