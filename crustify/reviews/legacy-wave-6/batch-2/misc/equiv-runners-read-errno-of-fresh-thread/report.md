# Equivalence runners read `errno` on a fresh thread that can start with `EAGAIN`

Affected: `run` in `crustify/rust/libgit2/tests/equiv/sys_errors.rs` (this workset,
util wave 0 batch 2). `run` in `crustify/rust/libgit2/tests/equiv/errors.rs` (util
wave 1 batch 0) has the same defect; the concurrent review batch 1 landed its fix
(`bf39b60f0`) while this one was under way, so it is not patched again here.

## Finding

Both runners spawn a fresh thread per side, run the step sequence on it and record
`errno` after every step, then compare the C and Rust sides step by step. They
assumed a new thread starts with `errno == 0`. On this environment (Rust 1.99 /
nightly 2026-09, glibc 2.39) a freshly spawned thread starts with `errno == EAGAIN`
every few hundred spawns while other threads are being created and torn down: std's
stack-overflow bookkeeping (`library/std/src/sys/pal/unix/stack_overflow/thread_info.rs`,
`set_current_info`) takes a global `Mutex` at every thread start, and a contended
`futex` wait that returns `EAGAIN` goes through `libc::syscall`, which stores it in
the new thread's `errno` before the user closure runs.

Under `cargo test`, the equivalence binary runs its tests on parallel threads and
spawns child processes, so the churn is there, and the first step of a sequence
that does not write `errno` (`Str`, `Oom`) is recorded with `errno == 11` on one
side and `0` on the other. Observed failure rate across 40 runs of the `equiv`
binary on the reviewed commit: 9 failures, in `sys_errors::threads_keep_their_own_errors`
(4), `sys_errors::messages_grow_and_shrink` (1), `errors::last_reads_each_threads_own_error`
(3) and `errors::last_reads_messages_of_every_size` (1). One trace:

```
---- sys_errors::threads_keep_their_own_errors stdout ----
thread 'sys_errors::threads_keep_their_own_errors' panicked at libgit2/tests/equiv/sys_errors.rs:315:13:
assertion `left == right` failed
  left: [Seen { ret: Some(0), errno: 11, class: 12, message: [] }, Seen { ret: None, errno: 11, class: 1, message: [79, 117, 116, ...] }, Seen { ret: None, errno: 0, class: 0, message: [110, 111, 32, 101, 114, 114, 111, 114] }, ...
 right: [Seen { ret: Some(0), errno: 0, class: 12, message: [] }, Seen { ret: None, errno: 0, class: 1, message: [79, 117, 116, ...] }, ...
```

The wrappers are not at fault: an `LD_PRELOAD` shim over `syscall` showed the
`EAGAIN` futex (`op 137`, `FUTEX_WAIT_BITSET|PRIVATE`) issued by the fresh thread's
own tid before its closure ran, with no libgit2 involved. The standalone reproducer
beside this report (pure std and libc) exits non-zero once a fresh thread starts with
a non-zero `errno`, typically within 100 ms:

```
$ cargo run -q
thread ThreadId(1194) started with errno 11
1 of 236 fresh threads started with a non-zero errno, in 68.321534ms
$ echo $?
1
```

## Fix

The runner sets `errno` to 0 at the top of the spawned closure, before the first
step, with a comment stating why (as batch 1 did for `errors.rs`). 40 further runs
of the `equiv` binary, with both fixes, passed.
