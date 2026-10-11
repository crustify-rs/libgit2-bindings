# `Backend`, `UnopenedBackend` and `BackendTransaction` had no `Debug`

- **item**: `git_config_backend` (type), `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `e797d4624` (landed by `beeda6a07`)
- **class**: misc (Rust-only trait surface)

## Finding

`BackendRef` and `BackendMut` print the backend's metadata. The value type `Backend`
(the template from `Backend::new()`), the construction phase `UnopenedBackend` and the
`BackendTransaction` guard did not implement `Debug`. In this TU, `BackendEntry`,
`Iterator` and `BackendMemoryOptions` all format their values. Earlier config reviews
filed the same gap for other handles
(`wave-1/batch-0/misc/entry-handles-lack-debug`,
`wave-2/batch-0/misc/backend-entry-handles-lack-debug`). Without these impls a caller
cannot `{:?}` a pending backend or a transaction, or derive `Debug` on a struct that
holds one.

## Fix

`Backend` formats like its shared handle. `UnopenedBackend` prints
`UnopenedBackend(Backend { .. })`, and `BackendTransaction` prints
`BackendTransaction { backend: Backend { .. }, .. }`. None of them prints callback
addresses. Asserted in `unit_tests::backends`
(`backend_rust_layout_and_stale_identity_are_not_dereferenced`,
`failed_rollback_on_drop_is_discarded`,
`backend_missing_slots_reject_before_ffi_dispatch`) and in the `ub_safe` template test.
