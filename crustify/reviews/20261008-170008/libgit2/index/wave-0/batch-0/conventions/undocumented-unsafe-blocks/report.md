# Unsafe blocks whose SAFETY comment does not reach them

- **items**: `git_index_iterator` (`tests/equiv/index.rs:1178`, commit `12159d066`),
  `git_diff` (`diff/diff.rs:213`, commit `5be7c5199`), `git_pathspec`
  (`index/pathspec.rs:632` and `tests/equiv/pathspec.rs:94`, commit `ab190ae80`)
- **class**: conventions (every unsafe block carries a SAFETY comment)

## Finding

Four unsafe blocks in the workset had their `// SAFETY:` comment above a statement that
rustfmt had split, so the comment sat above `assert_eq!(`, `let rc =`, `let _ =` or
`NonNull::new(` rather than above the `unsafe` line:

```rust
// SAFETY: writable output slot and live index; ...
assert_eq!(
    unsafe { ffi::git_index_iterator_new(&mut raw, c_index.0) },
    0
);
```

`crustify scan-unsafe` counts all four as undocumented (`undocumented_unsafe.per_file`:
`diff/diff.rs` 1, `index/pathspec.rs` 1, `tests/equiv/index.rs` 1,
`tests/equiv/pathspec.rs` 1). The first also fails the workspace lint
`clippy::undocumented_unsafe_blocks` under `cargo clippy --workspace --all-targets`.
Batches 0, 2, 5, 6 and 7 of this wave each reported that failure in their
`validation.md` and left it, since it was not theirs.

## Fix

Each block now stands on its own line under its comment: the result is bound first
(`let rc = unsafe { .. }; assert_eq!(rc, 0);`), or the long argument is hoisted into a
local. Nothing else changes. `cargo clippy --workspace --all-targets` is clean, and the
scan reports no undocumented site in the workset's files.
