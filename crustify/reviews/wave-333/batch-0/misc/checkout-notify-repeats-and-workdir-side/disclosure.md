To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: NotifyCallback's docs promise one call per path and a workdir side missing only off disk

Hi Marius,

This is a documentation problem, not a soundness bug. It can be handled in public.

In `src/libgit2/src/libgit2/checkout.rs` (commit b694c3653), `NotifyCallback` says libgit2
calls it "once per conflicting, dirty, updated, untracked or ignored path". It also says
`CheckoutNotification::workdir` is `None` "for a path missing on disk". libgit2 does neither.
Where a directory stands in place of a file that the baseline and the target share,
`checkout_action_with_wd_dir` (checkout.c:612-617) does two things. It notifies the file as
DIRTY with a NULL workdir side, though the directory is on disk. Then it notifies the
directory as UNTRACKED, and the workdir walk notifies it again:

```text
(DIRTY,     "a.txt",  [baseline "a.txt", target "a.txt", workdir None    ])
(UNTRACKED, "a.txt/", [None,             None,           "a.txt/"        ])
(UNTRACKED, "a.txt/", [None,             None,           "a.txt/"        ])
```

Under GIT_CHECKOUT_FORCE, `a.txt` is also notified as UPDATED, with the directory as its
workdir side. A closure that trusts the docs and keeps one entry per path, or that reads
`workdir: None` as "nothing on disk", decides wrongly.

The fix rewrites those docs. It also adds a unit test and an equivalence test (C callbacks
against `NotifyCallback`) that pin this behaviour, along with ignored files, which no test
reached before. Both are on the review branch (crustify/waves/wave-333). The report is under
`crustify/reviews/wave-333/batch-0/misc/`. I can send the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
