# `Commit::try_from_object` breaks the naming its three sibling conversions share

- item: `git_commit` (wave 1 batch 1, `crustify/rust/libgit2/libgit2/object/commit.rs`)
- kind: conventions — naming
- affected commit: `e2fba68e0`, reviewed at `38e6d1b3e`

## Finding

The wave wrapped the four typed objects with the same conversion surface: a borrowed
`XRef::from_object(ObjectRef) -> Option<XRef>`, and an owned
`X::from_object(ObjectArc<'r>) -> Result<XArc<'r>, ObjectArc<'r>>` with its inverse
`X::into_object`. `Blob` (batch 0), `Tag` and `Tree` (batch 3) name the owned form
`from_object`; batch 1 named the identical signature on `Commit` `try_from_object`. A
consumer converting objects generically — or reading one wrapper's docs and writing code
for another — meets two names for one operation, and the crate's own docs cross-reference
them inconsistently.

## Fix

`Commit::try_from_object` is now `Commit::from_object`, matching the other three (the
`Result` return already says the conversion can fail, as it does for them). Module and
type docs, the inline unit test, `tests/ub_safe/commit.rs` and `tests/equiv/commit.rs`
follow. No behaviour change.
