# The wave's BSan gate timed out on a 65k-step pure-Rust `FileMode` probe

- **item**: `git_filemode_t` (type), test
  `crustify/rust/libgit2/tests/equiv/types.rs::from_raw_names_exactly_the_c_modes`
  (libgit2/core wave 0 batch 2, `c3702db9a`)
- **affected commit**: `ab5de9104` (the review wave's base)
- **class**: gate failure; no UB and no wrong answer — BorrowSanitizer never finished
  the test, so the orchestrator's gate could not pass

## Finding

The orchestrator's BSan gate on the translated wave
(`crustify/campaigns/20261008-170008/libgit2/core/review-wave-0/bsan-gate.log`) ended
with `rc=124` after its 2400 s `timeout`, having completed only the unit-test binary
(`185 passed`); the `equiv` binary's last lines are

```
test types::from_raw_names_exactly_the_c_modes has been running for over 60 seconds
```

with every other long test (`oid::comparisons_match_c`, `oid::formatters_match_c`,
`oid::seeded_random_inputs_match_c`, `oid::shortener_adds_match_c`,
`sys_errors::messages_grow_and_shrink`) reporting `ok` before the kill. The test
iterates `0..=0o177777` (65 536 values) plus three extremes and calls
`FileMode::from_raw` / `TryFrom` on each, comparing against the `MODES` table: no FFI,
but BorrowSanitizer instruments every step and its GC safepoints stall on loops of
tens of thousands of steps (the same stall the campaign recorded for a 65k
`from_raw` probe in util wave 0). The sibling FFI test in the same file,
`parsed_trees_keep_any_16_bit_mode`, already took a stride of 97 under `cfg(bsan)`;
this one did not.

Reproduced on the base: the test alone under `cargo-variant.sh bsan` was still
running when a 300 s `timeout` killed it (`bsan-repro.log` excerpt below), while the
whole plain run of the file takes well under a second.

```
running 1 test
test types::from_raw_names_exactly_the_c_modes has been running for over 60 seconds
rc=124 elapsed=300s
```

## Fix

The probe takes the same stride under BorrowSanitizer as its sibling
(`if cfg!(bsan) { 97 } else { 1 }`), always adding the six named modes so every
variant is still checked; every other variant keeps the full sweep. With the fix the
whole workspace passes under BSan in 506 s (`equiv` 319 s, the unit tests 11 s), inside
the gate's 2400 s.
