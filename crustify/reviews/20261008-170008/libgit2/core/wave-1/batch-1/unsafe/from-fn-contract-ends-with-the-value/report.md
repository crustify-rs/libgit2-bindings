# `TraceCallback::from_fn`'s obligation ended with the value that adopted the function

- item: `git_trace_cb` (`TraceCallback`, `crustify/rust/libgit2/libgit2/core/trace.rs`)
- kind: safe-boundary defect — an `unsafe fn` whose `# Safety` contract is too weak
  for the calls the type exists to make
- affected commit: `2b63f91ae` (libgit2/core wave 1 batch 1), reviewed at `4dfa6b86a`
- fixed in: the Rust-tree commit of this review (doc contract; no code change)

## Finding

`TraceCallback::from_fn(f)` is the one `unsafe` entry point of the batch. Its
contract read:

> For as long as this value or a copy of it exists, `f` must be callable with any
> `TraceLevel` and any NUL-terminated string live for the call, from any thread,
> concurrently and reentrantly, with no obligation beyond `TraceFn`'s, and must not
> unwind. Those are the calls `call` and libgit2 make.

The obligation was tied to the lifetime of `TraceCallback` values, but the type's
purpose is to be handed to libgit2: `TraceCallback::raw()` is documented as "the C
value, as `git_trace_set` takes it", and `git_trace_set` stores the bare function
pointer in `git_trace__data.callback` (`src/libgit2/trace.c:18`), from where
`git_trace__write_fmt` (`src/libgit2/trace.h:24`) calls it on every traced operation,
on whatever thread runs it, for as long as it stays installed. No `TraceCallback`
value needs to exist at that point — `raw()` takes `self` by value and returns a
plain `Option<fn>`, and the type is `Copy`, so nothing ties libgit2's calls to any
value — yet the contract said the obligation ends when the last value is gone.

The gap is not reachable through this batch's own safe API (`call` needs a value), but
it is the contract the next batch builds on: `git_trace_set`'s safe wrapper is planned
as `set_trace(level, Option<TraceCallback>)` (the ub_safe fixture `set_trace` already
stands in for it), a *safe* function, which cannot add obligations of its own. On the
contract as written, this program honours every stated obligation and is still UB:

```rust
// A C tracer that appends to a buffer the caller owns.
let buffer = CString::new(vec![0u8; 4096]).unwrap();
install_sink(buffer.as_ptr());                       // C-side global the tracer writes
let cb = unsafe { TraceCallback::from_fn(c_tracer_into_sink) }; // obligation: while `cb` lives
set_trace(TraceLevel::Debug, Some(cb));              // the planned safe wrapper
drop(cb);                                            // the last value is gone...
drop(buffer);                                        // ...so, per the contract, so is the obligation
remote.connect(..);                                  // libgit2 traces: write into freed memory
```

The ub_safe fixture already assumed the stronger rule ("a `TraceCallback`, safe to
call from any thread for the life of the process") without the contract granting it.

## Fix

`from_fn`'s `# Safety` now binds `f` for the rest of the process and says why the
obligation cannot end with the value (`Copy`; `as_fn` hands the function out again;
`raw` feeds `git_trace_set`, after which libgit2 calls `f` with no `TraceCallback`
left). The type-level doc says the same in one line. Every existing adopter in the
tests (static or thread-local state only) already satisfied the stronger rule, so no
test changed meaning.

## Regression evidence

`tests/ub_unsafe/trace.rs::from_fn_installed_function_outlives_every_callback_value`:
adopts `c_logging`, installs the raw function with `git_trace_set`, lets the
`TraceCallback` go out of scope, makes libgit2 trace three connections, and checks
the function received all three `Connecting to remote` messages — the calls the
contract now covers explicitly. Clean under plain, ASan/UBSan, TSan and BSan.
