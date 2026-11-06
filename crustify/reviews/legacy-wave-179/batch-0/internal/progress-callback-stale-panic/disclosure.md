Subject: libgit2 bindings: ProgressCallback keeps a stale panic after an unwound walk

Hello maintainers of the libgit2 Rust bindings,

The indexer progress callback wrapper (`ProgressCallback`, over
`git_indexer_progress_cb`) stores a panic raised by its closure while libgit2 is
calling it. It resumes the panic once the routine that drove the callback returns,
from `ProgressCallback::walk_with`. If that routine unwinds instead of returning,
the stored panic is never cleared. Every later use of the callback then cancels
without running the closure and re-raises the old panic:

```rust
callback.walk_with(|cb, payload| {
    unsafe { cb.unwrap()(progress.as_ptr(), payload) }; // closure panics: stored
    panic!("the walk");                                  // walk unwinds
});
// later, unrelated:
callback.call(progress.as_ref()); // resumes "first call" instead of running the closure
```

Running the unit test `a_walk_that_unwinds_leaves_no_stale_panic` on the affected
commit:

```text
assertion `left == right` failed
  left: None
 right: Some(Continue(()))
```

This is a logic defect, not undefined behaviour. `walk_with` is crate-internal, so it is
not reachable from the public API today, but every future wrapper of a routine that
takes the callback would inherit it. The fix drops any stored panic before
each walk starts. It is in the same review, with the unit test as a regression test. I am
happy to share the full reproducer on request.

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental
UB/soundness auditing agent developed at UC Berkeley and running on Opus 5.5, then
manually reviewed and independently reproduced.
