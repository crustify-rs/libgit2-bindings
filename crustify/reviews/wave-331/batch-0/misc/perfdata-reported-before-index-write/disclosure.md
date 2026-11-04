To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: perfdata docs say the callback reports only successful checkouts

Hi Marius,

This is a documentation problem, not a soundness bug. It can be handled in public.

In `src/libgit2/src/libgit2/checkout.rs` (commit eda4ae102), the `# Performance data`
module docs and `CheckoutOptionsMut::set_perfdata_callback` say that libgit2 reports the
counts "once a checkout succeeds". libgit2 calls `perfdata_cb` before it writes the index
(`src/libgit2/checkout.c:2680-2686`), and that write can still fail the checkout:

```c
	if (data.opts.perfdata_cb)
		data.opts.perfdata_cb(&data.perfdata, data.opts.perfdata_payload);
cleanup:
	if (!error && data.index != NULL && ...)
		error = git_index_write(data.index);
```

With `.git/index.lock` present, a checkout calls the callback once and then returns
`GIT_ELOCKED` (-14). A consumer who takes a report as proof of success is misled. The
counts themselves are valid, so this is not UB.

The fix is on the review branch (crustify/waves/wave-331). It corrects both doc comments and
the matching wavefront ownership note, and adds an equivalence test that reproduces the
locked-index case:
`cargo test --test equiv perfdata_reads_match_c_on_links_update_only_and_a_failed_index_write`.
The report is under `crustify/reviews/wave-331/batch-0/misc/`. I can send the exact diff
on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
