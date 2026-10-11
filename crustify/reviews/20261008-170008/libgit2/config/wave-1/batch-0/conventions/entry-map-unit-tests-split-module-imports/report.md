# Entry/map tests split `unit_tests`' imports and were not rustfmt-clean

- **items**: `git_config_entry`, `git_configmap` (types),
  `crustify/rust/libgit2/libgit2/config/config.rs`, `tests/ub_safe/config.rs`
- **affected commit**: `512be0487` (libgit2/config wave 1 batch 0)
- **class**: conventions (test placement and formatting)

## Finding

The batch inserted its two inline tests (`mapping_layout_borrows_clone_and_seams`,
`entry_layout_and_borrowed_fields_without_c`) right after `use super::*;` in
`config.rs`'s `mod unit_tests`. That left the module's other imports
(`ErrorCode`, `Alloc`, `Command`, the atomics, ...) after the new tests, so the module read
as two halves. The insertion also left blank-line runs that `rustfmt +stable --edition
2024 --check` rejects (two hunks in `config.rs`, one in `tests/ub_safe/config.rs` before
`mod entry_and_map_types`). Both files were rustfmt-clean before the batch, except one
line from wave 1 batch 2 that this review leaves to that batch's reviewer.

The wave-0 review treated the same pattern (a type's inline test split from the single
`unit_tests` module) as a conventions fix:
`../../../wave-0/batch-0/conventions/map-type-tests-outside-config-suites`.

## Fix

The module's imports are joined under `use super::*;`, ahead of the entry/map tests, and
the stray blank lines are removed. The wave's symbol review (`1317ac5cb`), landed first,
removed the same blank lines and batch 2's hunk; this fix rebased onto it. Both
`config.rs` and `tests/ub_safe/config.rs` are rustfmt-clean. No test changed behaviour.
