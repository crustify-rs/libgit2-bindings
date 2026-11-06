# A checkout nested in a callback drops the outer checkout's panic

- **objective**: `misc` (a panic the API promises to resume is lost, and a panicked closure
  runs again)
- **where**: `CheckoutOptionsRef::walk_with` in `src/libgit2/src/libgit2/checkout.rs`, added
  by `50789cf60` (wave 334, batch 0). `Worktree::add` in `src/libgit2/src/libgit2/worktree.rs`
  reaches it, and the `git_checkout_*` wrappers (wave 356) will too.
- **severity**: low. Not unsound: no reference is aliased, and nothing outlives what it
  borrows. But a panic disappears without a trace, and the outer call reports success after a
  closure failed.
- **status**: fixed, with a unit regression test

## What is wrong

`walk_with` wraps the C call of every routine that checks out with the options. Before the
call, it drops whatever the panic slots of the stored Rust callbacks hold, treating it as
stale: left by an earlier call that handed the options to C directly. After the call, it
resumes what the slots hold.

The module docs, `set_notify_callback` and the `ub_safe` test
`an_add_the_closure_makes_with_its_own_options_finds_it_busy` all allow a closure to reach
its own options through interior mutability and check out with them again. Such a nested
call goes through the same `walk_with` while the outer walk is still running. At that point
the slots can hold a panic of the *outer* checkout, so they are not stale. Take the case of a
progress closure that panicked earlier in the checkout, before the performance-data closure
started a nested checkout. Then the nested `walk_with`:

1. drops the outer checkout's progress panic;
2. runs the nested checkout, where the progress trampoline no longer finds a panic, so it
   calls the closure that already panicked again. `set_progress_callback`'s docs promise
   that a panic "skips the closure for the rest of the checkout".

When the nested call returns, the outer `walk_with` finds the slot empty, and
`Worktree::add` returns `Ok`.

Only the progress-then-performance-data order loses a panic this way. A notification panic
cancels the checkout before either other callback runs. A performance-data panic comes last.
A panic raised by the nested checkout itself resumes inside the closure that started it,
where its trampoline catches it again, so it is not lost.

## Evidence

The reproducer in this directory (`#![forbid(unsafe_code)]`) builds a repository with one
file. Its progress closure panics once, on the first file. Its performance-data closure adds
a second worktree with the same `AddOptions`, borrowed from a `RefCell`. The outer
`Worktree::add` should resume the progress panic.

On `50789cf60`:

```console
$ cd crustify/reviews/legacy-wave-335/batch-0/misc/nested-add-loses-progress-panic
$ LD_LIBRARY_PATH=/target/libgit2-bindings/crustify/builds/plain cargo run
thread 'main' panicked at src/main.rs:78:13:
progress panics once
progress calls: [(None, 0, 1), (Some("a.txt"), 1, 1), (None, 0, 1), (Some("a.txt"), 1, 1)]
nested add succeeded: Some(true)
outer add returned Ok(()): the progress panic was lost, and the closure that panicked was called again during the same checkout
$ echo $?
1
```

The last two progress calls belong to the nested add. They reach the closure that had
already panicked.

With the fix:

```console
progress calls: [(None, 0, 1), (Some("a.txt"), 1, 1)]
nested add succeeded: Some(true)
outer add resumed: Some("progress panics once")
$ echo $?
0
```

## Fix

A walk is nested when one of the stored callbacks is busy. Its closure is then running, so
the walk is a checkout that this closure started with the same options. A nested
`walk_with` no longer clears the slots. It notes which slots already hold a panic, leaves
them alone, and resumes only the panics that its own checkout raised. The outer checkout's
panic stays in place: the nested checkout skips that closure, as the rest of the outer
checkout does, and the outer walk resumes the panic. A walk that is not nested still drops
stale panics first. `stored_panics` became `stored_slots`, which also returns each callback's
busy flag.

Regression test (unit, runs under Miri):
`checkout::unit_tests::a_nested_walk_leaves_the_outer_walks_panics_to_it`. A progress
closure panics in an outer walk, and the performance-data closure runs a nested walk with
the same options. The test checks that the outer walk resumes the panic, the nested walk
resumes nothing and skips the panicked closure, and the next walk runs the closure again.
