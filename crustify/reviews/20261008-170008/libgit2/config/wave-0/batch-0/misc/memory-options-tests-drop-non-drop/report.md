# Memory-options tests trip `clippy::drop_non_drop`

- **item**: `git_config_backend_memory_options` (type),
  `crustify/rust/libgit2/libgit2/config/sys/config.rs`,
  `crustify/rust/libgit2/tests/ub_safe/sys_config.rs`
- **affected commit**: `831989948` (libgit2/config wave 0 batch 0)
- **class**: misc — lint warnings

## Finding

On the review-wave base, `cargo clippy --workspace --all-targets` printed three warnings,
all in this workset:

```
warning: call to `std::mem::drop` with a value that does not implement `Drop`. Dropping such a type only extends its contained lifetimes
   --> libgit2/libgit2/config/sys/config.rs:292:9
  --> libgit2/tests/ub_safe/sys_config.rs:40:5
  --> libgit2/tests/ub_safe/sys_config.rs:44:5
```

`BackendMemoryOptions` has no `Drop` (it holds only borrows), so `drop(options)` does
nothing at run time. The tests meant to show that a clone is still readable once the
original is gone, but the `drop` calls proved nothing beyond what moving the value already
does, and they kept `-D warnings` from passing.

## Fix

In `unit_tests::rust_only_reborrows_and_clone` the original is now cleared and read back
next to the clone. In `tests/ub_safe/sys_config.rs::memory_options_reborrow_and_replace` the
original is moved into an inner block that returns the clone, so it really goes out of
scope before the clone is read. `cargo clippy --workspace --all-targets -- -D warnings` is
now clean.
