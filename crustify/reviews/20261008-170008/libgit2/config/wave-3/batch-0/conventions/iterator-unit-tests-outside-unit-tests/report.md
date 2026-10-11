# Iterator inline tests in a second `mod iterator_unit_tests`

- **item**: `git_config_iterator` (type), `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `0de2b59b4` (libgit2/config wave 3 batch 0)
- **class**: conventions (test placement)

## Finding

The conventions put a translated unit's inline tests in one
`#[cfg(test)] mod unit_tests` beside it. The batch appended a second top-level
`#[cfg(test)] mod iterator_unit_tests` after the file's `mod unit_tests`, which already
held the memory-options tests and the wave-2 `backend_entries` submodule. Test filters
and readers looking under `config::sys::config::unit_tests` missed the three iterator
tests. The file also again ended with something after `mod unit_tests`, which wave 2's
type review had just fixed
(`../../../wave-2/batch-0/conventions/backend-entry-items-after-unit-tests`).

## Fix

The tests now live in `unit_tests::iterators`, next to `unit_tests::backend_entries`,
with their own fixture and imports (`use super::*;` first). `mod unit_tests` is the last
item of the file again. No test body changed. The new allocation-failure test went into
the same submodule. The file passes `rustfmt +stable --edition 2024 --check`.
