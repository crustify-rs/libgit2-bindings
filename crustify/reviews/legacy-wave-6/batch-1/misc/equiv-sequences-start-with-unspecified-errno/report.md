# The `git_error_last` equivalence sequences compare a fresh thread's unspecified `errno`

- **items**: the `tests/equiv/errors.rs` suite of this workset's home TU (the
  `last` sequences added by wave 1 batch 0, `git_error_last`)
- **affected**: `run()` in `crustify/rust/libgit2/tests/equiv/errors.rs`, used by
  `last_reads_as_c_does_after_each_step`, `last_reads_messages_of_every_size`,
  `last_reads_each_threads_own_error`, `last_agrees_once_it_pins_the_runtime`,
  `last_reads_allocation_failures_as_c_does`
- **affected commit**: `dd95a582a` (wave 1 batch 0)
- **class**: misc — a flaky equivalence test; no defect in the wrapped code

## Finding

Each sequence runs its steps on a fresh thread, once reading the last error through
C and once through the safe `last`, and records `(class, message, errno)` after every
step; the two sides are compared step by step, and the first steps are also asserted
against `errno == 0`. The thread's `errno` is never initialised: the first step's
`errno` is whatever the thread started with, and before any step that sets it, the
comparison depends on that.

A freshly spawned Rust thread does not always start with `errno == 0`. A probe
spawning 6 400 threads that do nothing but read `errno` on entry:

```rust
fn errno() -> i32 { unsafe { *libc::__errno_location() } }
// 200 rounds × 32 threads: std::thread::spawn(errno).join()
{0: 6399, 11: 1}
```

One thread in 6 400 starts with `EAGAIN` (11), left by thread setup; it has nothing
to do with libgit2. The plain workspace run hit it once during this review:

```
thread 'errors::last_reads_each_threads_own_error' panicked at libgit2/tests/equiv/errors.rs:639:13:
assertion `left == right` failed
```

(the C-side expectation, computed on one thread, differed from a Rust-side run on a
thread that started with `errno == 11`), and the coverage-instrumented run hit the
same pattern in the sibling `tests/equiv/sys_errors.rs::threads_keep_their_own_errors`
(`Seen { ret: Some(0), errno: 11, ... }` against `errno: 0`), whose `run()` is built
the same way.

## Fix

`run()` now sets the fresh thread's `errno` to 0 before the first step, so both sides
start from the same known state; the steps that set `errno` (`Errno(..)`, `Os(..)`,
`Clear`) are compared exactly as before. Three consecutive plain runs and the
ASan/UBSan, TSan and coverage runs pass.

The sibling suite `tests/equiv/sys_errors.rs` (wave 0 batch 2, under review in
parallel) has the same gap in its `run()`; it belongs to that batch and is left for
its review, noted in `REVIEW.md`.
