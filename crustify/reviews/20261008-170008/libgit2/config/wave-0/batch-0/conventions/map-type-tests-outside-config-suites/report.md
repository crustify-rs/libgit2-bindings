# `MapType` tests live outside the config TU's suites, and its inline tests sit in a second module after an item

- **item**: `git_configmap_t` (type), `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `95f5f77cb` (libgit2/config wave 0 batch 1)
- **class**: conventions — test homing

## Finding

`MapType` is homed in `config/config.rs`, but batch 1 put its tests elsewhere:

- `tests/ub_safe/transaction.rs::mapping_modes_are_checked` and
  `tests/equiv/transaction.rs::mapping_mode_values_match_c_behavior` test only `MapType`
  (and `git_config_lookup_map_value`), yet sat in the *transaction* suites. Both suites
  must be `tests/<suite>/<the tested TU>.rs`, and `tests/{ub_safe,equiv}/config.rs` exist.
- Inside `config.rs`, the enum, its `impl`s and the `ConfigMut::lock` method came *after*
  the file's `#[cfg(test)] mod unit_tests`, and its inline test sat in a second module,
  `mod map_type_unit_tests`. The conventions name one inline `mod unit_tests` beside the
  translated unit, and every other TU in the crate ends with that single module.

## Fix

The enum, its `From` impl and `ConfigMut::lock` now come before the test module.
`checked_mapping_modes` moved into `unit_tests` and `map_type_unit_tests` is gone. The two
integration tests moved, unchanged apart from a local `use`, to
`tests/ub_safe/config.rs::mapping_modes_are_checked` and
`tests/equiv/config.rs::mapping_mode_values_match_c_behavior`. The transaction suites no
longer import `MapType`.
