# `BackendEntry`, `BackendEntryRef` and `BackendEntryMut` have no `Debug`

- **item**: `git_config_backend_entry` (type), `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `a6e60a4d5` (libgit2/config wave 2 batch 0)
- **class**: misc (API ergonomics). The existing surface is sound.

## Finding

`define_ctype!` generates no `Debug`, and the batch added none. `{:?}` on a
`BackendEntryArc` printed only its pointer (ffibox's `CArc` `Debug`). A
`BackendEntryRef` did not implement `Debug` at all, so a derived `Debug` on anything
holding one did not compile. Every other wrapper in the module and its parent does
implement `Debug`: `BackendMemoryOptions` (wave 0 review), `Entry` (wave 1 review,
`misc/entry-handles-lack-debug`), `ConfigMap` and `Config`.

## Fix

`Debug` for `BackendEntryRef` prints `BackendEntry { entry: Entry { .. }, .. }`. The
inner entry comes through the existing `entry()` projection and `EntryRef`'s `Debug`.
The `free` callback is elided (`finish_non_exhaustive`): its address is neither portable
nor meaningful, and reading the slot is all `release_policy` needs. `BackendEntry` and
`BackendEntryMut` delegate to it. The impls add no unsafe code.

## Tests

- `sys/config.rs::unit_tests::backend_entries::debug_prints_the_entry_only` (also under
  Miri): shared, exclusive, reborrowed and inline-value forms over a Rust-owned fixture
  print the expected string, including a NULL value, a NULL origin and `u32::MAX` depth.
- `tests/ub_safe/sys_config.rs::backend_entries::backend_entry_debug_reads_retained_storage`
  (ASan/UBSan, TSan): formats two acquired entries after their config and files are gone,
  then again after a sibling owner is released. Every string comes from the retained
  list.
