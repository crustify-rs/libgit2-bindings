# Mapping/callback unit tests again split `unit_tests`' imports

- **items**: `git_config_foreach_cb`, `git_config_get_mapped` (inline tests of
  libgit2/config wave 2 batch 1, `d6a13729c`),
  `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `3a5814a4e`
- **class**: conventions (test placement)

## Finding

The batch inserted its three inline tests
(`callback_trampoline_stops_panics_nulls_and_reuses`,
`callback_foreign_iteration_cleans_up_before_resuming_panic`,
`mapped_snapshot_construction_failure_reports_oom`) and the `callback_entry` fixture at
the top of `mod unit_tests`, ahead of its `use super::*;` block. That left a blank line
after `mod unit_tests {` and put the module's imports halfway down, after
`get_i64_suffix_tail_reports_its_own_error`. Wave 1's type review fixed the same split
(`../../../wave-1/batch-0/conventions/entry-map-unit-tests-split-module-imports`).

## Fix

The import block now opens the module again, followed by the batch's fixture and tests.
No test changed. The file passes `rustfmt +stable --edition 2024 --check`.
