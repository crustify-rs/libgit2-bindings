# `MapType` lacks the `/// Handwritten` anchor

- **item**: `git_configmap_t` (type), `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `95f5f77cb` (libgit2/config wave 0 batch 1)
- **class**: conventions — anchors

## Finding

`MapType` is a hand-written Rust enum over the C `git_configmap_t` values (no ffibox
primitive represents it), and it carried only `/// Wraps: git_configmap_t`. The ffibox
guidance asks for a `/// Handwritten` anchor on every hand-written representation, and
the crate follows it for every other C enum: `ConfigLevel` in the same batch, `RefdbType`,
`PruneFlag`, `InitFlag`, `RepositoryState`, `GitFile` and the others. The campaign's
hand-written count missed this item.

## Fix

Added `/// Handwritten` under the `Wraps:` anchor.
