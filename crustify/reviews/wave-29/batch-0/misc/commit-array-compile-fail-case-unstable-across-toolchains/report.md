# `ub_safe::compile_fail`: `commit_array_outlives_repository` fails under the sanitizer toolchains

- item: `git_commit` (wave 1 batch 1, which gave `CommitArray` its commits' `'r` and added
  `tests/ub_safe/compile_fail/commit_array_outlives_repository.rs`)
- kind: misc — test suite
- affected commit: `e2fba68e0`, reviewed at `38e6d1b3e`

## Finding

`scripts/cargo-variant.sh asan-ubsan test --workspace` on the wave tip fails in
`ub_safe::compile_fail` (and, without `--no-fail-fast`, never runs `ub_unsafe` or the
doctests):

```
test tests/ub_safe/compile_fail/commit_array_outlives_repository.rs ... mismatch
EXPECTED:
7 | fn store<'r>(mut array: CommitArrayMut<'_, 'static>, commit: CommitArc<'r>) {
  |          --  ---------                               ------ `commit` is a reference that is only valid in the function body
ACTUAL OUTPUT:
7 | fn store<'r>(mut array: CommitArrayMut<'_, 'static>, commit: CommitArc<'r>) {
  |          --  ---------                               ------ `commit` is only valid in the function body
```

The case's `E0521` label is worded differently by the stable toolchain (which recorded
the `.stderr`) and the nightly one the ASan and TSan variants build with; the two
`E0505` errors in the same case match on both. Same class as the earlier
`object/wave-0/batch-0/misc/tree-builder-compile-fail-case-unstable-across-toolchains`.

## Fix

The `store` check moved to a `compile_fail,E0521` doctest on
`CommitArrayMut::set_commits` (doctests compare the error code only), with a sentence
saying why; the trybuild case keeps the two `E0505` checks, its `.stderr` re-recorded.
Both pass on the plain (stable) and on the ASan/UBSan and TSan (nightly) variants.
