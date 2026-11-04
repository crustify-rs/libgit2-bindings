To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: wavefront notes on git_checkout_notify_cb's NULL sides miss two cases

Hi Marius,

This is an analysis-record problem, not a soundness bug. It can be handled in public.

Wave 332 (commit b694c3653) submitted pointer findings for `git_checkout_notify_cb` to
wavefront's ownership store. The properties are right: each `git_diff_file` side is borrowed
for the call, read only, and nullable. Their notes, which binders read as the reasoning,
miss cases:

- workdir (`arg4`) is said to be NULL only "when the path is missing on disk". But
  `checkout_action_with_wd_dir` (checkout.c:612-617) passes NULL while a directory stands
  at the path:

  ```c
  checkout_notify(data, GIT_CHECKOUT_NOTIFY_DIRTY, delta, NULL);
  checkout_notify(data, GIT_CHECKOUT_NOTIFY_UNTRACKED, NULL, wd);
  ```

  The side can also be a directory (mode 040000, path ending in `/`), not only a file.
- baseline and target (`arg2`, `arg3`) are also NULL for the DIRTY notification of a file the
  index tracks outside the checkout's diff (checkout.c:413-415), not only for untracked and
  ignored items.

The corrected notes have been resubmitted with the same properties. The update file and the
report are under `crustify/reviews/wave-333/batch-0/wavefront/ownership/` on the review
branch (crustify/waves/wave-333). I can send the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
