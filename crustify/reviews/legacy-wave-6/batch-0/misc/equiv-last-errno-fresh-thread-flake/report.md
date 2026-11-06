# `last_reads_each_threads_own_error` compares a fresh thread's start-up `errno`

- **item**: `git_error` (type) — the `last` equivalence harness,
  `crustify/rust/libgit2/tests/equiv/errors.rs`
- **affected commit**: `dd95a582a` (libgit2/util wave 1 batch 0, which added the
  harness) — the test flakes on the reviewed wave-0 tree
- **class**: misc — a nondeterministic equivalence test; no wrapper defect
- **reproducer**: none needed; `cargo test -p libgit2 --test equiv` failed 3 of 15
  runs on the reviewed tree (`last_reads_each_threads_own_error`), 0 of 40 after
  the fix

## Finding

`run(side, steps)` spawns a thread, takes each step and records `Seen { class,
message, errno }` after it; `compare` and `last_reads_each_threads_own_error` then
require every thread's `Seen`s to be identical, the reference run included. The
first steps of that test (`Str`, `Oom`) do not touch `errno`, so their `errno` field
is whatever the thread started with. A freshly spawned Rust thread's `errno` is not
always 0: under contention, start-up leaves `EAGAIN` (11) behind. A probe spawning
8 × 200 threads, each reading `errno` as its first action, observed one thread
starting with `errno == 11`; the failing assertion shows the same:

```
thread 'errors::last_reads_each_threads_own_error' panicked at libgit2/tests/equiv/errors.rs:566:13:
assertion `left == right` failed
  left: [Some(Seen { class: 12, message: [], errno: 11 }), Some(Seen { class: 1, message: [79, 117, 116, ...], errno: 11 }), Some(Seen { class: 0, message: [110, 111, 32, ...], errno: 0 }), ...
 right: [Some(Seen { class: 12, message: [], errno: 0 }),  Some(Seen { class: 1, message: [79, 117, 116, ...], errno: 0 }),  Some(Seen { class: 0, message: [110, 111, 32, ...], errno: 0 }), ...
```

(`errno` returns to 0 at the third step, `Clear`, which resets it on both sides.)
The C and Rust sides are equally exposed, so the comparison does not discriminate
between them; it fails whenever the eight worker threads and the reference thread
do not all start with the same `errno`.

## Fix

`run` zeroes the thread's `errno` before its first step, with a comment saying why,
so every recorded `errno` is the product of the steps alone; the test then passed
40/40 runs on this batch's tree. The wave-0 reviews of batch 1
(`misc/equiv-sequences-start-with-unspecified-errno`) and batch 2
(`misc/equiv-runners-read-errno-of-fresh-thread`) found the same flake concurrently
and landed the identical one-line fix first, batch 2's also covering the sibling
harness in `tests/equiv/sys_errors.rs`; on rebase this batch took the wave's
version, so its landing commit carries no change to `tests/equiv/errors.rs`.
