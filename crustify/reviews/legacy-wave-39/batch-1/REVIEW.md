# Review: libgit2 / config / wave 2 / batch 1 (the wave's symbols)

Reviewed commit `3a5814a4e` (the review-wave base), covering the five `symbol` items of
the wave. Batch 1 (`d6a13729c`) wrapped `git_config_get_mapped`
(`ConfigRef::get_mapped`), `git_config_lookup_map_value` (`lookup_map_value`) and the
`git_config_foreach_cb` callback (`ForeachCallback`). It reused the two items wave 1
had already emitted: `git_config_entry_free` (`EntryFree`, the `EntryArc` policy) and
`git_config_get_entry` (`ConfigRef::entry`). All of them live in `config/config.rs`.

Config has waves 0–5 (`schedule.json`), so wave 2 is not the sub-campaign's last and
there is no BSan gate here. BSan ran only on the one reproducer emitted, alone.

| step | outcome |
|---|---|
| lifecycle discovery | Not applicable: no `type` or `raw-lifetime` items. `git_config_entry_free` keeps its single anchor on `EntryFree` (`impl_cdrop!`), and no wrapper duplicates it. |
| wavefront findings | 1 report, resubmitted: `wavefront/map-lookup-null-value-and-output-notes`. `lookup_map_value`'s `value` note omitted the NULL `%s` miss report, its `out` note claimed a failed parse may write the output (it never does), and `get_mapped`'s plan still reached the first. The three other records (callback, entry_free, get_entry) were checked against the C and hold. |
| safe boundary | No defect. The workset adds no public `unsafe fn` and no public raw pointer, so there is no `ub_unsafe` suite. `scan-unsafe --sites`: the only config site is `raw_ptr_derefs_outside_impl` at the trampoline's payload reconstruction (`foreach_with`), the necessary private callback seam with its exclusivity/liveness SAFETY comment. After the fixes: unsafe blocks unchanged (one removed in `get_mapped`, one added in `restore_last`), undocumented sites 44 → 44. |
| UB | No sanitizer-provable defect. Checked by hand: `ForeachCallback` keeps the closure exclusively borrowed (no reentry, no escape of the higher-ranked `EntryRef`, `!Send`), catches panics inside the `extern "C"` trampoline and resumes them only after C returns, and refuses to call the closure again after a stop. `EntryArc` cannot clone or mutate. Snapshot entries pin their list, not the dropped snapshot. Tables passed to C are transparent `ConfigMap` slices whose String rules always have a string. The one C UB reachable from safe code (`printf("%s", NULL)`) is defined by glibc and invisible to every prepared sanitizer: filed under misc. The fixed tree is clean under ASan/UBSan (LSan on) and TSan, and Miri runs the inline seam tests. |
| equivalence | 1 report, fixed: `equiv/spent-backend-failure-reformats-os-error` (reproducer included). When `entry` / `get_mapped` spent a lower backend's reread failure, they restored the previous error with `set`. For `GIT_ERROR_OS`, `set` appends `": "` and the `errno` description, and an unknown class was not restored at all. A private `restore_last` now copies the error verbatim through `git_error_set_str`. Regression: `equiv::config::mapping_callbacks::spent_backend_failures_keep_any_previous_error`. |
| internal | Nothing further to fix. |
| conventions | 1 report, fixed: `conventions/workset-unit-tests-split-module-imports`. The batch's inline tests were inserted ahead of `unit_tests`' `use` block again. |
| misc | 1 report, fixed: `misc/valueless-map-miss-formats-null`. A valueless setting with no boolean-true rule reached `git_error_set("failed to map '%s'", NULL)`. `lookup_map_value` now reports that miss itself, with glibc's wording. `get_mapped` is composed from `entry` + `lookup_map_value`, as C composes it, which also removes its duplicated reread/restore code. Regression: `unit_tests::valueless_misses_never_reach_c_formatting`. |
| coverage | See below. |

## Coverage

Measured with the `coverage` variant (`cargo llvm-cov` over the config-filtered `lib`,
`equiv` and `ub_safe` tests, plus C gcov through a private `GCOV_PREFIX`).

- **Rust**, `config/config.rs`: 1473/1489 lines and 2798/2820 regions. The translation
  batch measured 1441/1459 and 2726/2751. No region in the workset code is uncovered:
  `entry`, `reread_through`, `restore_last` (both branches; the equivalence regression
  also restores an undefined class, 4242), `get_mapped`, both `lookup_map_value` paths, and every
  `ForeachCallback` / trampoline branch. The old uncovered region, restoring an unknown
  raw class, is gone because the restore no longer branches on known classes.
- **C**: `git_config_entry_free` 6/6, `git_config_get_entry` 2/2,
  `git_config_get_mapped` 7/7 (driven by the equivalence references; the safe wrapper no
  longer calls it) and `git_config_lookup_map_value` 20/20 lines. `get_entry` is at
  25/28. Its remaining lines are the `GET_NO_ERRORS` / `GET_NO_MISSING` branches, which
  only the internal `git_config__lookup_entry` uses.

## Validation of the landed tree

`cargo check --workspace` and `cargo clippy --workspace --all-targets` produce no
warnings. `cargo test --workspace` ran under plain, ASan/UBSan and TSan (`-Zbuild-std`).

- Base, plain: `lib` 615 passed / 1 ignored, `equiv` 389, `ub_safe` 648, `ub_unsafe`
  91, doctests 43 + 250.
- Fixed tree, under all three: `lib` 616 / 1 ignored, `equiv` 390, `ub_safe` 648,
  `ub_unsafe` 91, doctests 43 + 250, all clean.
- Miri on the config inline tests: 10 passed, 20 ignored (they call libgit2).
- The reproducer exits 101 on `3a5814a4e` and 0 on the fixed tree. Run alone under BSan,
  it exits 0 with no report.
- With only `restore_last` reverted, the new equivalence regression fails on its OS-class
  case.
- `rustfmt +stable --edition 2024 --check` passes on the changed files, and
  `git diff --check` is clean.

## Noted, not changed

- **`get_mapped` no longer calls `git_config_get_mapped`.** It composes the same two
  steps C does (`get_entry` + `lookup_map_value` + entry release), through the safe
  entry acquisition. The equivalence suites still compare it with C's routine.
- **`ForeachCallback::walk_with` is a safe `pub(crate)` seam that hands out raw
  pointers.** Misusing them needs `unsafe` in the caller's closure, and its doc states
  the obligations. The wave-3 foreach wrappers are its intended users.
- **C's foreach routines swallow a callback's `GIT_ITEROVER` (-31).** This matters for
  the wave-3 wrappers built on `ForeachCallback`, not for `call`. `stopped()` already
  keeps the closure's own code, so those wrappers can report it.
