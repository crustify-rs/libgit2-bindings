To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: IndexerProgress docs misstate what libgit2's indexer resets

Hi Marius,

This is a documentation problem, not a security or soundness issue.

The docs of `IndexerProgress` (`src/libgit2/src/libgit2/indexer.rs`, commit 9f243dd2e) say
two things about libgit2 that are false: that the indexer "resets every counter when it reads a
pack's header", and that the zeroed value is how "the indexer leaves it before a pack's header
arrives". The campaign's ownership store recorded the same claim for `git_indexer_progress`.

In `src/libgit2/indexer.c:915-920`, `git_indexer_append` sets `total_objects` from the header
and zeroes the five object and delta counters after it. It never writes `received_bytes`,
which the transports count (`transports/smart_protocol.c:664`, `transports/local.c:506`), and
it writes nothing before the header. The zeroed state comes from the caller:

```c
git_indexer_progress stats = { 0 };          /* git_packbuilder_write */
memset(stats, 0, sizeof(git_indexer_progress)); /* git_smart__download_pack */
```

This matters to the next wrappers of `git_indexer_append` and `git_indexer_commit`. If a
caller trusts the old docs and reuses a stale progress, `received_bytes` keeps its old value.

Corrected docs and a corrected store note are on the review branch (crustify/waves/wave-173),
with the full report under `crustify/reviews/wave-173/batch-0/misc/`. I can send the diff on
request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
