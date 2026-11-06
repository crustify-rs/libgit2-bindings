# The `SAFETY` comment of `CommitArrayDispose` sits outside the campaign scan's window

- **item**: `git_commitarray_dispose` (`CommitArrayDispose`'s `unsafe impl CDispose`),
  `crustify/rust/libgit2/libgit2/object/commit.rs`
- **affected commits**: `2e375c880` (repository wave 0 batch 2, which emitted the policy) and
  `e2fba68e0` (object wave 1 batch 1, which lengthened its comment to seven lines)
- **class**: conventions. Every `unsafe` site carries a `// SAFETY:` comment where the
  campaign's audit can find it.

## Finding

`crustify scan-unsafe` counts an `unsafe impl` line as documented only when a `SAFETY`
comment is on that line or within the six lines above it. The policy's comment started
seven lines above `unsafe impl<'r> CDispose<CommitArray<'r>> for CommitArrayDispose`, so
the scan counted the site among `commit.rs`'s four undocumented ones. Clippy's
`undocumented_unsafe_blocks` accepts it.

## Fix

The comment now says the same thing in six lines. Behaviour is unchanged. Re-running the
scan's rule over the batch's files leaves three undocumented sites in `commit.rs`, none of
them in this workset: the doctest of `CommitBuilderMut::from_ptr`, `add_header`'s split
`let rc =`, and `CreateExtOptions`' `CCell` impl. The other remaining sites are two in
`tests/equiv/commit.rs` outside the `create_options` module and one in
`tests/ub_safe/fixture.rs`'s `StreamWriter`.
