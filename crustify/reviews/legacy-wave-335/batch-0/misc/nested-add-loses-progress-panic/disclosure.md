To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: a checkout nested in a callback drops the outer checkout's panic

Hi Marius,

This is a correctness bug, not a soundness bug, so it can be handled in public.

In `src/libgit2/src/libgit2/checkout.rs` (commit 50789cf60), `CheckoutOptionsRef::walk_with`
wraps the C call of every routine that checks out with the options, `Worktree::add` for now.
It drops any panic the stored callbacks hold before the call, as stale, and resumes what they
hold after it. A closure may reach its own options through a `RefCell` and check out with them
again, as the docs allow. That nested walk then drops a panic of the outer checkout that is
still in flight:

```rust
let mut on_progress = |path: Option<&CStr>, _, _| {
    if path.is_some() && !panicked.replace(true) { panic!("progress panics once"); }
};
let mut on_perfdata = |_: CheckoutPerfdataRef<'_>| {
    add("inner", &opts.borrow());   // same options, nested
};
// ... store both, then:
add("outer", &opts.borrow())
```

```text
progress calls: [(None, 0, 1), (Some("a.txt"), 1, 1), (None, 0, 1), (Some("a.txt"), 1, 1)]
outer add returned Ok(()): the progress panic was lost, and the closure that panicked was called again during the same checkout
```

The outer add reports success, and the panicked closure runs again in the same checkout. The
docs promise that the closure is skipped for the rest of the checkout and that the panic
resumes once libgit2 returns.

The fix treats a walk as nested while one of the stored callbacks is busy. A nested walk
leaves the panics already held to the outer walk, and resumes only those its own checkout
raised. A unit regression test runs under Miri. The fix is on the review branch
(crustify/waves/wave-335). The report and a standalone reproducer are under
`crustify/reviews/legacy-wave-335/batch-0/misc/nested-add-loses-progress-panic/`. I can send the
exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
