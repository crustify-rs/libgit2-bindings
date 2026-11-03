# `IndexerProgress` docs and its ownership-store note misstate what the indexer resets

- **Objective:** `misc` (documentation, and the type's note in the ownership store)
- **Where:** `src/libgit2/src/libgit2/indexer.rs`, the module doc (`# Progress`) and the doc of
  `IndexerProgress`; `crustify/wavefront/ownership-store.json`, the `_comment_user` of
  `git_indexer_progress` (`include/git2/indexer.h`), all as landed by `9f243dd2e`
- **Verdict:** wrong statements about C behaviour. No API or behaviour is affected, so there is
  no reproducer.

## What is wrong

1. The module doc said: "the indexer resets every counter when it reads a pack's header and
   advances them as it hashes objects and resolves deltas".
2. The type doc said that `IndexerProgress::default` / `zeroed` gives "every counter at 0, as the
   indexer leaves it before a pack's header arrives".
3. The ownership store recorded: "git_indexer_append resets every counter on the pack header and
   increments them afterwards".

None of these matches `c/src/libgit2/indexer.c`:

- on the pack header, `git_indexer_append` (lines 915-920) sets `total_objects` to the header's
  object count and zeroes `indexed_objects`, `received_objects`, `local_objects`,
  `total_deltas` and `indexed_deltas`;
- it never writes `received_bytes`. The transports count it
  (`transports/smart_protocol.c:664`, `transports/local.c:506`);
- it writes nothing before the header arrives. The zeroed state comes from the caller:
  `git_packbuilder_write` (`pack-objects.c:1425`, `= { 0 }`) and `git_smart__download_pack`
  (`transports/smart_protocol.c:691`, `memset`). `local_download_pack`
  (`transports/local.c:602-605`) zeroes only `total_objects`, `indexed_objects`,
  `received_objects` and `received_bytes`.

This matters to the wrappers that come next in this module (`git_indexer_append`,
`git_indexer_commit`, the progress callback). Read the old way, the docs suggest that a caller
can pass a stale progress and the indexer will clear it. It will not clear `received_bytes`,
and before the header arrives it clears nothing.

## Rerun

```bash
sed -n 913,921p c/src/libgit2/indexer.c
grep -n received_bytes c/src/libgit2/indexer.c   # no output
sed -n 1425p c/src/libgit2/pack-objects.c; sed -n 691p c/src/libgit2/transports/smart_protocol.c
sed -n 602,605p c/src/libgit2/transports/local.c
wavefront c --config <campaign config> query types --name git_indexer_progress   # findings._comment_user
```

## Fix

- The module doc now says that the caller zeroes the progress, that the indexer sets
  `total_objects` from the header and resets the five other object and delta counters, and that
  `received_bytes` is left to the transport.
- The type doc now says the zeroed value is the state callers zero a progress to before handing
  it to libgit2.
- The note in the ownership store was corrected and resubmitted with
  `query types --name git_indexer_progress --update`. It now records what the header resets,
  that `received_bytes` belongs to the transports, and which callers zero the progress. The
  type has no pointer fields, so no `ptr` finding changed.
