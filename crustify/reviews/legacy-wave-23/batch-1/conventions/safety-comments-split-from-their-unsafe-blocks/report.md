# `// SAFETY:` comments separated from their `unsafe` blocks

- **items**: the tests of `git_repository_set_head_detached`,
  `git_repository_set_head_detached_from_annotated`, `git_repository_set_workdir`,
  `git_repository_fetchhead_foreach_cb` and `git_repository_head_for_worktree`
  (`tests/equiv/repository.rs`, the inline `unit_tests::heads_and_parts` of
  `repository/repository.rs`)
- **affected commits**: `28fd3b00e`, `f591b52ca` (libgit2/repository wave 1 batches 2
  and 3)
- **class**: convention (every unsafe block carries a specific `// SAFETY:` comment,
  which `crustify scan-unsafe` reads on the line before the block)

## Finding

Six unsafe blocks had their comment, but not on the line before the block: rustfmt
had moved the `unsafe { .. }` of a long `let x = unsafe { .. };` onto its own line
after `let x =`, or the block sat inside a tuple expression, so `crustify
scan-unsafe` counted them as undocumented (`undocumented_unsafe.per_file`:
`tests/equiv/repository.rs` 5, `libgit2/repository/repository.rs` 1 in the wave's
scan). `clippy::undocumented_unsafe_blocks` accepts them, so the lint did not catch
it. (`tests/equiv/worktree.rs` has one more of the kind, in the `git_worktree_add_options`
tests, which belong to the type review.)

## Fix

Each site binds its long arguments (or the routine) to locals first so that
`let rc = unsafe { .. };` fits on one line under its comment, or hoists the block
out of the tuple into its own statement. The workset's sites are gone from the scan
(35 undocumented in the wave's scan, 29 after).
