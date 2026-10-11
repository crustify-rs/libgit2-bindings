# Wave code not formatted with rustfmt

- **items**: `git_diff` (`object/commit.rs:2369-2372`, the consumer rewired by commit
  `5be7c5199`) and `git_index_conflict_iterator` (`tests/equiv/index.rs:1269` and
  `:1283`, commit `25e623071`)
- **class**: conventions (formatting)

## Finding

`rustfmt +stable --edition 2024 --check` failed on code this wave landed:

- `commit.rs`'s `changes()` built the diff with an over-long `match` over fully
  qualified `crate::libgit2::diff::diff::Diff::…` paths and a `}.map_err(…)` tail.
- The conflict-cursor equivalence fixture had a 104-column `git_index_conflict_add`
  call and a hand-wrapped `observe` signature.

## Fix

`commit.rs` imports `Diff` and the expression is in rustfmt's layout. rustfmt alone would
split the `git_index_conflict_add` line into `let rc =` / `unsafe { .. }`, which moves its
SAFETY comment out of reach (see `undocumented-unsafe-blocks`). The arguments are
hoisted into locals instead, so the line fits. All files the wave or this review touched
pass `rustfmt --check`.
