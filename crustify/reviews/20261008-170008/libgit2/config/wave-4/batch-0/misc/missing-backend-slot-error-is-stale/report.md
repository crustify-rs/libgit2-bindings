# Calls through an absent backend slot reported an unrelated earlier error

- **item**: `git_config_backend` (type): `BackendOpen::call`, `BackendMut::{get,
  iterator, snapshot, set, set_multivar, delete, delete_multivar, lock}`,
  `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `e797d4624` (landed by `beeda6a07`)
- **class**: misc: wrong error contents (Rust-only path, no C oracle)

## Finding

Each dispatch checks its callback slot before calling it, which is right: C would call
through NULL. But the refusal was built by

```rust
fn backend_error() -> ConfigError {
    ConfigError::from_libgit2(ffi::GIT_ERROR)
}
```

and `ConfigError::from_libgit2` captures the thread's *last* libgit2 error. Nothing set
one, so the caller got whatever an earlier, unrelated call left behind. A fresh thread
got `no error`, which is how the equivalence reproducer
`../../equiv/writable-get-ignores-backend-get-slot` printed it. After a failed parse
on the same thread, the refusal claimed to be that parse error. The backend
template returned by `Backend::new()` reaches every one of these paths from safe code,
since all of its slots are NULL.

This is the same pattern earlier config reviews fixed for silent C failures
(`wave-0/batch-1/misc/parse-int64-suffix-tail-error-is-stale`,
`wave-1/batch-1/misc/delete-readonly-error-is-stale`).

## Fix

`missing_slot(slot)` sets a `GIT_ERROR_CONFIG` error,
`config backend does not implement '<slot>'`, before building the `ConfigError`. Each
dispatch names its own C slot (`open`, `get`, `iterator`, `snapshot`, `set`,
`set_multivar`, `del`, `del_multivar`, `lock`, `unlock`).

Regressions:

- `unit_tests::backends::backend_missing_slots_reject_before_ffi_dispatch` seeds an
  unrelated `Odb` error before each call on the template. It asserts code `GIT_ERROR`,
  class `Config` and the slot-naming message, and does the same for a mock missing
  `unlock` and a pending mock missing `open`.
- `tests/ub_safe/sys_config.rs`,
  `backends::backend_template_refuses_every_slot_and_snapshot_chains_outlive_sources`,
  through the safe public API.
