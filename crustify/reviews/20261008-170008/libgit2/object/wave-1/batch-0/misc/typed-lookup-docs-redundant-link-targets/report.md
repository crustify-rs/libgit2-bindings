# Typed lookups' docs: redundant explicit link targets warn under `cargo doc`

- items: `git_commit` (`Commit::lookup`, `Commit::lookup_prefix`), `git_tag`
  (`Tag::lookup`), `git_tree` (`Tree::lookup`, and the `TreeEntry::bypath` docs batch 3
  rewrote)
- kind: misc — documentation
- affected commits: `e2fba68e0`, `47a8e7a36`, reviewed at `38e6d1b3e`

## Finding

`cargo doc -p libgit2 --no-deps` prints `rustdoc::redundant_explicit_links` warnings on
seven doc links of the workset's constructors: each spells
`[`ErrorCode::NotFound`](crate::libgit2::util::errors::ErrorCode::NotFound)` (or
`ErrorCode::Ambiguous`, `ObjectType::Tag` / `ObjectType::Tree`) in a module that imports
the type, so the explicit target is the one rustdoc resolves anyway (`commit.rs` 507, 565,
568; `tag.rs` 375, 376; `tree.rs` 1162, 1163, 1849 at the reviewed commit). The earlier
object review left the crate's docs warning-free.

## Fix

The links keep their text and drop the explicit target. `cargo doc` now warns only for
two sites outside this workset: `blob.rs:1245` (`create_from_workdir`'s docs, a
symbol-batch item: `[0]` read as a link) and `filter/filter.rs:63`.
