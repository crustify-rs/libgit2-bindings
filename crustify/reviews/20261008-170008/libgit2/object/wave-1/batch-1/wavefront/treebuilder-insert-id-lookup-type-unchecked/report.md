# `git_treebuilder_insert`'s `id` finding omits that the lookup takes the id's type unchecked

- **item**: `git_treebuilder_insert`, `ptr_args[3]` (`id`)
- **submitted by**: libgit2/object wave 1 batch 7 (`84c213038`)
- **resubmitted**: `wavefront . --config crustify/wavefront/configs/wavefront-config.json
  query symbols --name git_treebuilder_insert --update …` (note only; every other
  facet unchanged)

## Previous finding

> Asserted non-NULL; zero id refused; looked up in the ODB unless mode is COMMIT or
> strict object creation is off; copied by value (git_oid_cpy), not retained.

## Why it was incomplete

The lookup is `check_entry` → `git_object__is_valid` → `git_odb_read_header`
(`src/libgit2/tree.c:495`, `src/libgit2/object.c:598`), which never compares the id's
type with `bld->repo`'s; the loose backend's `object_file_name`
(`src/libgit2/odb_loose.c:82`) sizes the path for the repository's type and
`git_oid_pathfmt` writes the id's own hex size. The record therefore carried no
caller obligation on the id's type, and the safe wrapper was written without one: a
SHA-256 id in a SHA-1 repository overflows the heap (see
`ub_safe/tree-builder-insert-formats-foreign-id-out-of-bounds`, with the ASan trace),
and a SHA-1 id in a SHA-256 repository leaves part of the path uninitialised.

## Corrected finding

The note now states the lookup condition (any mode but `COMMIT`, after the mode and
zero-id refusals, with strict object creation on), that the id's type is not checked
against the repository's, the overflow and the uninitialised read it causes, and the
obligation: an id of the builder's repository type for a looked-up mode, which the
Rust wrapper enforces with `id_fits` before FFI.
