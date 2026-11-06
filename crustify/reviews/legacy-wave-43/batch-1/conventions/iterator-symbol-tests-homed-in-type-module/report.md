# The workset's inline tests stayed in `config/sys/config.rs`

- **items**: `git_config_iterator_new`, `git_config_iterator_glob_new`,
  `git_config_multivar_iterator_new` (`ConfigRef::iterator` / `iterator_matching` /
  `multivar_iterator`) and `git_config_next` (`IteratorMut::advance_entry`), all homed
  in `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `8491945f1` (libgit2/config wave 4 batch 1), reviewed at
  `e797d4624`
- **class**: conventions (test placement)

## Finding

The conventions put a translated unit's inline tests in the `#[cfg(test)] mod
unit_tests` beside it. The batch moved the three constructors from `config/sys/config.rs`
(where wave 3's type batch had put them) to their scheduled home, `config/config.rs`, and
added `advance_entry` there. Their inline tests did not move with them:

- `config::sys::config::unit_tests::iterators::allocation_failures::constructors_report_allocation_failure`
  (`sys/config.rs:1824-1934` at the reviewed commit) exercises only the three
  constructors. It also brought a second copy of the "libc allocator that refuses after N
  grants" fixture, which `config.rs`'s `unit_tests` already has (`Refusing`,
  `ALLOWED_ALLOCATIONS`, `install_refusing`, `isolated`).
- The batch added its own unit test for `advance_entry`,
  `iterator_public_entry_dispatch_and_release` (`sys/config.rs:1782`), to the type
  module, next to the type's adoption fixture, not beside the method.

A reader or a test filter looking under `config::config::unit_tests` for the
constructors' or `git_config_next`'s internal evidence found none.

## Fix

Both tests now live in `config::config::unit_tests::iterators`:

- `constructors_report_allocation_failure` runs the same six cases with the module's
  existing `isolated` child, `install_refusing` and `ALLOWED_ALLOCATIONS`. The
  duplicated `Counting` allocator is gone. Each recovery step now advances through the
  public `advance_entry`.
- `advance_entry_dispatches_statuses_and_lends` replaces the fixture test with its own
  adopted Rust allocation (`Scripted`), whose `next` replays a status script. It still
  checks what the old test checked (dispatch to the allocation's own callback, a lent
  and never acquired entry, end of iteration, one release by the allocation's own
  `free`). It also covers what the old one could not: a failure status keeps its code
  and thread error, a positive status is reported as `GIT_ERROR`, and the iterator stays
  usable after a failure and after the end.

`sys/config.rs` keeps the tests of the `git_config_iterator` type (layout, handles,
adoption and release, the value filter). Both files pass
`rustfmt +stable --edition 2024 --check`.
