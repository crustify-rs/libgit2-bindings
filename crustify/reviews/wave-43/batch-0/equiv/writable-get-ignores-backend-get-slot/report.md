# `BackendMut::get` skipped a writable backend's own `get` callback

- **item**: `git_config_backend` (type), field accessor `BackendMut::get`
  (`Field: git_config_backend.get`), `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `e797d4624` (the review wave's base; landed by `beeda6a07`,
  libgit2/config wave 4 batch 0)
- **class**: equivalence defect: return value and error differ from C's dispatch of the
  same slot

## Finding

For a backend whose `readonly` flag is zero, the wrapper never called the backend's
`get` callback. It took a snapshot instead and looked the key up there:

```rust
if !self.as_ref().is_readonly() {
    return self.snapshot()?.as_mut().get(key);
}
```

The doc comment justified this as avoiding "C's file-get callback returning an
uninitialized status after refresh failure". `config_file_get`
(`src/libgit2/config_file.c:340`) has no such path. `config_file_refresh` returns its
error, and `config_file_get` returns it without writing `*out`.
`config_file_is_modified` sets `*modified = 0` before anything can fail. The defect
the earlier entry wrapper works around is in the *caller*, `get_entry`
(`src/libgit2/config.c:811`), which forms `&be->entry` from the unwritten slot. This
wrapper never reads the slot after a failure, so that defect cannot reach it.

The detour cost two things:

1. A writable backend that implements `get` but not `snapshot` could not answer a
   lookup at all. C dispatches the `get` slot; the wrapper failed with `GIT_ERROR`.
   The `BackendBox` adoption contract documents NULL slots as permitted, so such a
   backend is within contract. The error also carried the thread's previous message
   (`no error` below; see `../../misc/missing-backend-slot-error-is-stale`).
2. Every lookup on a writable backend copied the whole backend: a snapshot
   allocation plus one entry per variable, then a discard.

No built-in backend reachable from safe code today is writable (memory and snapshot
backends set `readonly = 1`). So the divergence shows through the documented ffibox
adoption seam, or through the file backends that `git_config_add_backend` will hand
over in wave 5.

## Reproducer

`src/main.rs` builds two independent instances of a minimal writable C-ABI backend.
Its `get` forwards to an opened in-memory backend holding `a.b = value`, and its
`snapshot` slot is NULL. The C side calls the instance's `get` slot directly. The
Rust side adopts the other instance with `BackendBox::from_c`, within the documented
contract, and calls `BackendMut::get`. The program compares the results.

```sh
CRUSTIFY_LIBGIT2_BUILD_DIR=<build> LD_LIBRARY_PATH=<build> cargo run --offline
```

`<build>` must export `git_config_backend_from_string` (wave 4 batch 0 made it
`GIT_EXTERN`). The shared prebuilds under `/target/crustify/builds` predate that change.
This review used a private plain build of the reviewed tree,
`/tmp/crustify-review-config-w4-b0/plain`.

On `e797d4624` (exit 101):

```
c    Ok([118, 97, 108, 117, 101])
rust Err((-1, "no error"))

thread 'main' panicked at src/main.rs:97:5:
assertion `left == right` failed: BackendMut::get diverges from the backend's get slot
  left: Err((-1, "no error"))
 right: Ok([118, 97, 108, 117, 101])
```

On the fixed tree it prints two equal `Ok` lines and exits 0.

## Fix

`BackendMut::get` now dispatches the backend's own `get` slot for every backend, as C
does. It still reads `out` only after the callback reports success. The doc and
`SAFETY` comments now give the real contract. `snapshot()` is unchanged.

Regressions:

- `tests/equiv/sys_config.rs`, `backends::writable_get_dispatches_its_own_slot_like_c`:
  the reproducer's backend, compared through `compare_get` for a multivar, a
  valueless key, a missing key and a repeated lookup.
- `unit_tests::backends::writable_get_dispatches_its_own_slot_and_missing_open_cleans_up`
  (inline): a mock writable backend counts `get` calls. Lookups succeed without a
  snapshot slot and reach the callback once each. A snapshot taken afterwards answers
  without calling it.
