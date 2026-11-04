To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: checkout::ProgressCallback's docs promise a call for every step

Hi Marius,

This is a documentation problem, not a soundness bug. It can be handled in public.

In `src/libgit2/src/libgit2/checkout.rs` (commit 3b654f097), the docs of
`checkout::ProgressCallback` say libgit2 calls it "once after each file it removes, writes or
resolves a conflict of, with that path ... and the count so far". libgit2 counts one step per
stale conflict that it removes from the index, but `checkout_remove_conflicts` never calls the
callback for them:

```c
git_vector_foreach(&data->remove_conflicts, i, conflict) {
    if (git_index_conflict_remove(data->index, conflict) < 0)
        return -1;
    data->completed_steps++;   /* no report_progress */
}
```

A forced tree checkout over an index that holds a conflict on `c.txt` therefore reports:

```text
calls: [(None, 0, 2), (Some("a.txt"), 2, 2)]
documented contract broken: 2 steps counted, completed counts reported [0, 2], documented [0, 1, 2]
```

A checkout that only removes conflicts reports nothing but `(None, 0, n)`. A progress bar
written against the docs would never finish. The wrapper itself matches C call for call.

The fix rewrites the docs to state that removed conflicts are counted but not reported. It
also adds equivalence tests against a C callback for that case, and for the symlink and
submodule calls. It is on the review branch (crustify/waves/wave-331). The report and a
standalone reproducer are under
`crustify/reviews/wave-331/batch-1/misc/progress-callback-docs-promise-every-step/`. I can send
the exact diff on request.

The skipped calls are libgit2's own behaviour. If you would rather raise them upstream, a
one-line `report_progress(data, conflict)` in `checkout_remove_conflicts` would make every
step visible, and I can prepare that patch too.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
