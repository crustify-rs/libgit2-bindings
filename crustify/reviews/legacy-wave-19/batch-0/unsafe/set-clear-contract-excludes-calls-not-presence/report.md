# `trace::set` / `trace::clear` excluded other threads' *calls* into libgit2, not their *presence* in it

- item: `git_trace_set` (`trace::set`, `trace::clear`,
  `crustify/rust/libgit2/libgit2/core/trace.rs`)
- kind: safe-boundary defect — an `unsafe fn` whose `# Safety` contract is ambiguous
  about the one situation it exists to exclude
- affected commit: `943038c1d` (libgit2/core wave 2 batch 0), reviewed at `6ccd53a97`
- fixed in: the Rust-tree commit of this review (doc contract plus one regression
  test; no code change)

## Finding

Both wrappers must stay `unsafe`: `git_trace_set` writes `git_trace__data.level` and
`.callback` (`src/libgit2/trace.c:16-21`), a plain global that `git_trace`
(`src/libgit2/trace.h:36-46`) reads without a lock from whichever thread runs a traced
operation — the level, then the callback for its NULL check, then the callback again
in `git_trace__write_fmt` (`trace.h:24`). A write concurrent with such a read is a C11
data race, and a `clear` landing between the check and the second read makes libgit2
call NULL. The wrapper cannot serialise against those readers (they are inside C), so
the obligation falls on the caller.

The contract as landed read:

> No other thread may call into libgit2 — through this crate or otherwise — while this
> runs: […] A traced call on the same thread, `callback` included, may.

"Call into libgit2 while this runs" names the act of *entering* libgit2 during the
call. The realistic race is the other way round: a thread that entered
`git_remote_connect` *before* `set` began and is still inside it when `set` writes
(the HTTP client traces at `httpclient.c:1109`, `:1308`, `:1158` over a connection
that can take seconds). That thread does not "call into libgit2 while this runs" on a
strict reading — it called in earlier — yet it is exactly the reader the wrapper must
exclude. The same text is also silent on what makes a *later* entry safe: a thread
that enters libgit2 after `set` returns without any synchronisation with the writing
thread has no happens-before edge to the write, which is still a data race by the
C11 definition (the `GIT_MEMORY_BARRIER` after the write at `trace.c:20` is a fence
on the writer side only; `git_trace` has none). The module doc already said the right
thing ("no other thread may be inside libgit2 meanwhile"); the function contracts,
which `unsafe` callers actually read, did not.

The batch's own tests assumed the stronger reading throughout (every `// SAFETY:` in
`tests/ub_unsafe/trace.rs::config` says "no other thread is inside libgit2" or "every
other thread that called into libgit2 has been joined"), so no test changed meaning.

The `common.rs` option setters (`set_mwindow_size` and the others, core wave 0) carry
the same "may call into libgit2 … while this runs" sentence; they are outside this
workset and are left as they are, noted here for their next review. For them the
window is narrower (their readers are short routines), but the wording gap is the same.

## Fix

`set`'s `# Safety` now reads: no other thread may be *inside* libgit2 while this runs,
"whether it entered libgit2 before this call began or during it", and a thread that
calls into libgit2 afterwards must do so after synchronising with this call (a join, a
lock, a spawn); the calling thread itself may be inside libgit2, in a traced call or
in `callback`. `clear`'s refers to `set`'s with the same two clauses and keeps its
NULL-call explanation.

## Regression evidence

`tests/ub_unsafe/trace.rs::config::set_and_clear_while_a_traced_thread_waits_outside_libgit2`:
a worker thread connects (and is traced), reports through a channel, and then stays
alive but parked outside libgit2 while the main thread calls `set` and `clear`; each
reconfiguration is handed to the worker through the channel before it connects again,
four rounds, checking that exactly one `Connecting to remote` message reaches the
tracer after each `set` and none after each `clear`. That is the situation the new
wording admits and the old one left ambiguous (a thread that *has been* inside libgit2
is fine; one that *is* inside is not), and the channel is the synchronisation the new
wording requires; TSan would flag the round if it were not enough. Clean under plain,
ASan/UBSan, TSan and BSan.
