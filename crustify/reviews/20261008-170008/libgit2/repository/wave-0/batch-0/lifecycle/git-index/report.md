# `git_index`'s typed constructors (`git_index_new_ext`, `git_index_open_ext`) were not wrapped

- **item**: `git_index` (type), `crustify/rust/libgit2/libgit2/index/index.rs`
- **affected commit**: `b423332e3` (libgit2/repository wave 0 batch 3; reviewed at `65285ef7b`)
- **class**: lifecycle gap: two of the type's four public allocating constructors had
  no safe form, and so no SHA-256 index could be built in Rust

## Finding

libgit2 builds a sole-reference `git_index` through four public routines:
`git_index_new` / `git_index_open` and their `_ext` forms, which take a
`git_index_options { version, oid_type }` and are the only way to build an index whose
object ids are SHA-256 (`git_index_open_ext`, `index.c:441`: the type is asserted
valid and stored; `git_index_new` / `git_index_open` are the `_ext` forms with NULL
options, i.e. `GIT_OID_DEFAULT`). Batch 3's wavefront findings record all four as
constructors (`query types --name git_index --lifecycle-ops`), but the wrapper emitted
only `Index::new` and `Index::open`, documenting "Its object ids are SHA1"; the sibling
types of the wave got their typed forms (`Odb::with_oid_type` / `open_with_oid_type`,
`Repository::with_oid_type`). An index file written by a SHA-256 repository could not
be opened on its own, and no in-memory SHA-256 index could be built.

## Fix

`Index::with_oid_type(OidType)` wraps `git_index_new_ext` and
`Index::open_with_oid_type(&CStr, OidType)` wraps `git_index_open_ext`, each over a
private `git_index_options` of the current version (as `odb.rs` builds its
`git_odb_options`), sharing `open`'s refusal of the empty path (the `git_filebuf`
NULL-`memcpy` divergence). `git_index_new_ext`, `git_index_open_ext`,
`git_index_options` and `GIT_INDEX_OPTIONS_VERSION` joined the `libgit2-sys`
allowlists.

Evidence: `tests/equiv/index.rs::oid_types` (each type's in-memory and on-disk index
against `git_index_new_ext` / `git_index_open_ext` step by step: entries added, the
file written byte for byte alike, reopened as its own type, refused as the other, and
read by `open` only when SHA-1; the failure cases alike),
`tests/ub_safe/index.rs::typed_indexes_are_built_and_released_once`, and the inline
`unit_tests` (`options_carry_this_version_and_the_type`, the typed forms in the
empty-path, uninitialisable-library and allocation-failure sweeps).
