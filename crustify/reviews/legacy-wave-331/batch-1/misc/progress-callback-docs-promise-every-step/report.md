# `checkout::ProgressCallback`'s docs promise a call for every step

- **objective**: `misc` (documentation contradicts libgit2's observable behaviour)
- **where**: `src/libgit2/src/libgit2/checkout.rs`, the `# Progress callbacks` module docs
  and the `ProgressCallback` type docs, added by `3b654f097` (wave 330, batch 1)
- **severity**: low. No unsoundness: a closure that trusts the docs, for example a
  progress bar waiting for `completed == total`, never sees the checkout finish.
- **status**: fixed (docs), with regression equivalence tests

## What is wrong

The docs say libgit2 calls the callback once with the total, "then once after each file it
removes, writes or resolves a conflict of, with that path ... and the count so far". The
module docs say "once per file it removes or writes and per conflict it resolves".

libgit2 does not call it for every step. `git_checkout_iterator`
(`c/src/libgit2/checkout.c:2638-2643`) counts `CHECKOUT_ACTION__REMOVE_CONFLICT` in
`total_steps`. `checkout_remove_conflicts` (`checkout.c:2278-2290`) removes each stale
conflict from the index and increments `completed_steps`, but never calls
`report_progress`. These removals run right after the file removals and before any write.
So the next call's count skips them. A checkout that only removes conflicts calls back
once, for the zero baseline, and never reports a completed step.

The wrapper matches C exactly. The equivalence tests below run a C callback beside it, and
the `ownership-store.json` note for `git_checkout_progress_cb` does not claim one call per
step. Only the Rust docs overstate the contract.

## Evidence

The reproducer in this directory makes a repository with one file, and adds a content
conflict on `c.txt` to the index. It then runs a forced `git_checkout_tree` with a
`ProgressCallback`, through the options' raw seam, as the batch's equivalence tests do.

```console
$ cd crustify/reviews/legacy-wave-331/batch-1/misc/progress-callback-docs-promise-every-step
$ LD_LIBRARY_PATH=/target/libgit2-bindings/crustify/builds/plain cargo run
calls: [(None, 0, 2), (Some("a.txt"), 2, 2)]
documented contract broken: 2 steps counted, completed counts reported [0, 2], documented [0, 1, 2]
$ echo $?
1
```

The removal of the `c.txt` conflict is step 1, and it is never reported. With the equiv
fixture's five conflicted paths and nothing else to do, the closure sees only
`[(None, 0, 5)]`, the same as the C callback beside it.

## Fix

The docs now say that conflicts written out are reported, and that conflicts removed from
the index are counted but not reported. They also say the next call's count skips those
removals, and that the last call need not report every step completed.

Regression and coverage tests in `tests/equiv/src/libgit2/checkout.rs`:

- `a_progress_callback_sees_removed_conflicts_as_a_c_callback`: for a checkout that only
  removes conflicts, both callbacks see `[(None, 0, 5)]`. Beside updates, the first
  reported count is 6. The last call reaches the total, with `total + 1 - 5` calls.
- `a_progress_callback_sees_symlinks_and_submodules_as_a_c_callback`: covers the
  symlink and submodule calls (`checkout.c:1900`, `1921`), which no test reached before.

`compare_progress` now takes a fixture-preparing function instead of a `conflict` flag.
