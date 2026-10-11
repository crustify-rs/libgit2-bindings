# The `diff_not_sync` compile-fail case passes only on toolchains with `rust-src`

- **item**: `git_diff` (type), `tests/ub_safe/compile_fail/diff_not_sync.rs` / `.stderr`
- **affected commit**: `5be7c5199` (libgit2/index wave 0 batch 0)
- **class**: misc (test suite)

## Finding

The trybuild case checks that `Diff<'static>` is not `Sync`. Its blessed `.stderr`
contains the source snippet of `core::marker::PhantomData`:

```
note: required because it appears within the type `PhantomData<(*const (), &'static ())>`
 --> $RUST/core/src/marker.rs
  |
  | pub struct PhantomData<T: PointeeSized>;
  |            ^^^^^^^^^^^
```

rustc prints that snippet only when the toolchain ships the standard library sources.
The nightly toolchain used by the sanitizer variants has them, but the stable toolchain
of the `plain` variant does not. So `cargo-variant.sh plain test --workspace` failed
`ub_safe::compile_fail` with a stderr mismatch (the actual output stops at
`--> $RUST/core/src/marker.rs`), while the ASan/UBSan and TSan runs passed. Batches 5
and 6 of this wave hit the mismatch and recorded it in their `validation.md`, but left it
because it was not theirs. The plain workspace suite was therefore red at the wave tip.

## Fix

The check is now two `compile_fail,E0277` doctests on `Diff`: one for the layout, as
before, and one for `DiffRef`, the shared handle that would actually cross threads.
Doctests pin the error code, not rustc's note chain, which is how this crate already
checks the other not-`Send`/`Sync` claims (`DiffBox`, `IndexBox`, `PathspecArc`, …).
The trybuild pair is removed. The doctests fail to compile with E0277 under both the
stable and the nightly toolchains, and `plain test -p libgit2 --test ub_safe --
compile_fail` passes again.
