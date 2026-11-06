# PCRE2 release policies break the module's policy declaration pattern

- **item**: `git_config_iterator` (type): `value_filter::RegexFree` and
  `value_filter::MatchDataFree`, `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `0de2b59b4` (libgit2/config wave 3 batch 0)
- **class**: conventions (declaration style)

## Finding

Every other release policy in the crate (`BackendEntryFree`, `IteratorFree`, `EntryFree`
and the rest) has the same shape: doc comment, a blank `///`, the anchor lines, then
`#[derive(Clone, Copy, Debug, Default)]` on a unit struct. The two PCRE2 policies put
`#[derive(Clone, Copy, Default)]` above the doc comment and left out `Debug`. Their
`Wraps:` anchor also ran on directly from the description, with no blank line between.

## Fix

Both now follow the module's pattern: doc, blank line, `Wraps:` anchor, then
`#[derive(Clone, Copy, Debug, Default)]`. Nothing else changed. They remain private to
`value_filter`, and the PCRE2 routines they bind are unchanged.
