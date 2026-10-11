# `Entry`, `EntryRef` and `EntryMut` have no `Debug`

- **item**: `git_config_entry` (type), `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `512be0487` (libgit2/config wave 1 batch 0)
- **class**: misc (API ergonomics). The existing surface is sound.

## Finding

`define_ctype!` generates no `Debug`, and the batch added none. An `EntryArc` printed only
its pointer (ffibox's `CArc` `Debug`). Its `EntryRef` handle, which carries all six field
getters, did not implement `Debug` at all. So `{:?}` on an entry's fields, or a derived
`Debug` on a type holding an `EntryRef`, did not compile. Wave 3/4 will lend `EntryRef`s
to `git_config_foreach` callbacks and `git_config_next` iterators, where printing an entry
is the first thing a caller reaches for.

The config module's own `ConfigRef`/`ConfigMut`, and the options records reviewed in wave 0,
implement `Debug`.

## Fix

`Debug` for `EntryRef` prints the six fields through the getters (`name`, `value`,
`backend_type`, `origin_path`, `level`, `include_depth`), as `Entry { .. }`. `Entry` and
`EntryMut` delegate to it. Nothing reads beyond what the getters already read, so the
output borrows nothing and adds no unsafe code.

## Tests

`config.rs::unit_tests::debug_and_thread_traits` (Miri): an `EntryRef`, an `EntryMut` and
an inline `Entry` over a Rust-owned fixture print the expected string. The fixture
includes a NULL value and a non-UTF-8 origin.
