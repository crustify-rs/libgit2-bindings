To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: SAFETY comment on `CZeroable for IndexerProgress` cites a reset libgit2 does not do

Hi Marius,

This is a documentation problem in an `unsafe impl`, not a soundness bug. No input reaches
undefined behaviour, so it can be handled in public.

In `src/libgit2/src/libgit2/indexer.rs` (commit 9f243dd2e), the justification of
`unsafe impl CZeroable for IndexerProgress {}` says all-zero is "the state the indexer resets a
progress to when it reads a pack's header". libgit2 does not do this. On the header,
`git_indexer_append` sets `total_objects` to the header's object count and zeroes only the
five counters after it (`src/libgit2/indexer.c:915-920`). It never writes `received_bytes`,
which the transports accumulate:

```c
stats->total_objects = total_objects;
stats->indexed_objects = 0;
/* ... received_objects, local_objects, total_deltas, indexed_deltas */
```

The impl is still sound. Every field is an `unsigned int` or a `size_t`, so all-zero is valid
for every accessor, and there is no teardown. The comment should rest on that, and on the
callers that zero a progress before handing it over (`git_packbuilder_write`,
`git_smart__download_pack`).

The corrected comment is on the review branch (crustify/waves/wave-173), with the full report
under `crustify/reviews/wave-173/batch-0/contracts/`. I can send the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
