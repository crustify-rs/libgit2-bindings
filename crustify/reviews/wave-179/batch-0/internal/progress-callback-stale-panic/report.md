# A walk that unwinds leaves a stale panic in its `ProgressCallback`

- **Objective:** `internal`
- **Where:** `src/libgit2/src/libgit2/indexer.rs`, `ProgressCallback::walk_with` and
  `progress_with`, as landed by `4b00f9c6773f1c8791ca35c2ba1fa488520bf022`
  (`crustify: wrap git_indexer_progress_cb as ProgressCallback`).
- **Severity:** low. Behaviour only, no undefined behaviour. Reachable only through
  `walk_with`, which is `pub(crate)`. Today only the unit tests call it, but every future
  wrapper of a routine that takes the callback (`git_odb_write_pack`, `git_indexer_new`,
  the fetch callbacks) would go through it.

## What is wrong

When the closure panics inside C, the trampoline `progress_with` stores the panic in
`ProgressCallback::panic`. It returns `GIT_EUSER`, and then cancels every later call
without running the closure. `walk_with` takes the stored panic and resumes it only once
`walk` *returns*. If `walk` unwinds instead, the take is never reached and the panic stays
stored. A wrapper's own code can panic after libgit2 has returned the cancel, for example
an `unwrap`, an assertion or a nested callback's resumed panic.

The callback is still usable afterwards: it is borrowed `&mut` and comes back to its owner
once the unwind is caught. From then on, every later use is broken:

- `ProgressCallback::call` never runs the closure. The trampoline sees the stale panic,
  cancels, and `walk_with` then resumes the **old** panic from the unrelated call.
- A later `walk_with` (a new indexing) is cancelled at its first progress. The old panic
  then resumes from it, so a new indexing fails with a panic it never raised.

## Evidence

Unit test `a_walk_that_unwinds_leaves_no_stale_panic` in `indexer.rs`'s `unit_tests`, run
on the affected commit (wave 179's base, `f1f360f7b`), with the test added and the fix
left out:

```bash
cargo test -p libgit2 --lib a_walk_that_unwinds
```

```text
thread '...a_walk_that_unwinds_leaves_no_stale_panic' panicked at src/libgit2/src/libgit2/indexer.rs:944:13:
first call
thread '...a_walk_that_unwinds_leaves_no_stale_panic' panicked at src/libgit2/src/libgit2/indexer.rs:956:17:
the walk
thread '...a_walk_that_unwinds_leaves_no_stale_panic' panicked at src/libgit2/src/libgit2/indexer.rs:964:9:
assertion `left == right` failed
  left: None
 right: Some(Continue(()))
```

The third panic is the later `call`. Instead of returning `Continue`, it re-raised
`"first call"`, the closure panic of the walk that had already unwound. The closure ran
only once.

## Fix

`walk_with` now drops any stored panic before it starts the walk. Only a walk that
unwound can leave one behind, and that panic belonged to it: Rust was already unwinding
with the walk's own panic, which is what propagated. Within a walk, the behaviour is
unchanged: after the closure panics, later calls cancel without running it, and the
panic resumes once the walk returns. The reproducer is now the unit test above, which
passes.
