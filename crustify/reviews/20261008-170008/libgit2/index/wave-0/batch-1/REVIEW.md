# Review: libgit2 / index / wave 0 / batch 1 (the wave's symbols)

Reviewed commit `ab3dabdcc` (the review wave's base). It covers the one `symbol` item the
wave landed, from wave 0 batch 8 (`6c09dba0d`):

| item | Rust | anchor |
|---|---|---|
| `git_index_matched_path_cb` | `MatchedPathCallback<'f>` (+ `MatchedPathResult`) in `crustify/rust/libgit2/libgit2/index/index.rs` | `Wraps:` + `Handwritten` |

The wrapper was re-read against the typedef and all three C invokers:
`index_apply_to_all` (`git_index_remove_all`) and `apply_each_file` (`git_index_add_all`
/ `git_index_update_all`, `src/libgit2/index.c:3693-3925`). The paths it checked:

- argument and result passing: `None` exactly for NULL; every `int` preserved;
  positive codes skip, negative codes abort and are returned unchanged;
- after a stop the closure is not called again, and each scan resets that state;
- a panic is contained at the C ABI and resumed after C has cleaned up;
- the handle is `!Send` / `!Sync`;
- borrowck prevents same-handle reentry, while nested different handles are allowed;
- the `&mut` payload derived from `scan_with`'s `&mut self`, with no Rust reborrow while
  C holds it.

Wave 0 is not index's last wave (`schedule.json`: waves 0-2), so BSan ran only on the
reproducer emitted here.

| step | outcome |
|---|---|
| lifecycle discovery | Not applicable: a callback, with no lifecycle primitives. Neither string nor the payload transfers ownership in any caller. |
| wavefront findings | 2 defects, resubmitted (only the record's two `note` lines changed, without the tool's re-sort). `wavefront/matched-pathspec-null-for-unusable-patterns`: `ptr_args[1]` said NULL "for an empty pathspec". C passes NULL whenever no pattern survives parsing, and an empty entry beside others is over-read. `wavefront/remove-all-path-freed-by-index-mutation`: `ptr_args[0]` said the index releases `path` "later". A callback that mutates the scanned index frees `entry->path` at once, shown by a temporary raw ASan probe (`asan-probe.log`). No safe path reaches it today. The add/remove/update wrappers must keep the index out of the closure's reach. `ptr_args[2]` holds. |
| safe boundary | 1 report, fixed: `unsafe/matched-path-trampoline-outside-impl`. The trampoline was a free function. It is now `MatchedPathCallback::trampoline`, casting to `Self`, and `raw_ptr_derefs_outside_impl` goes 16 → 15. The workset adds no public `unsafe fn` and no public raw pointer. The crate-private `scan_with` seam's contract (synchronous, unretained, strings immutable) was re-read and holds. There is no `ub_unsafe` suite to review. |
| UB | No true positive in the workset, checked by hand: the payload's `&mut` aliasing under nested `call`s; a panic payload outliving a stopped scan; a scan closure panicking after a stored panic (dropped with the handle); closure arguments escaping (rejected by the `compile_fail` doctests); cross-thread use. Miri passes the Rust-only tests (1 inline, 3 `ub_safe`). ASan/UBSan passes the whole workspace. One libgit2 defect outside the workset surfaced through this batch's new test and is filed under misc. |
| equivalence | No defect. `tests/equiv/index.rs::matched_path_callback` gains three steps, compared with C's reference callback on returns, errors, arguments and index state: `["", ""]`, `["#a", "\nb", "\r\nb"]` (no usable pattern) and `["#a", "c"]`. |
| internal | Nothing beyond the misc test defect below. |
| conventions | None to fix. Anchors, naming, `SAFETY` comments (`undocumented_unsafe` unchanged at 49) and rustfmt on the touched files all check out. `tests/equiv/index.rs` was not rustfmt-clean at the base, so it was edited by hand. |
| misc | 3 reports, fixed. (1) `misc/matched-pathspec-none-doc-omits-unusable-patterns`: the doc said the pattern is `None` only when no patterns were supplied, so it is now precise, with an inline test. (2) `misc/matched-path-workdir-tests-race-the-first-filter-load`: the two `add_all` / `update_all` inline tests raced libgit2's first filter load under TSan (exit 66), and they now run through `isolated`. (3) `misc/pathspec-empty-entry-overread`: `Pathspec::new(["a", ""])` makes libgit2 read past the empty string (`parse_optimized_patterns`). It has a `#![forbid(unsafe_code)]` ASan reproducer, aborting on the base and exiting 0 on the fix (BSan, alone: exit 0). `Pathspec::new` now refuses the mix through the new `pub(crate) pathspec::patterns_are_parsable`, for the later scan, diff and checkout wrappers to reuse. Regressions: `tests/ub_safe/pathspec.rs::empty_patterns_beside_others_are_refused` and `pathspec.rs::unit_tests::mixed_empty_patterns_are_refused_before_libgit2`. `git_pathspec` belongs to the wave's type batch, and the fix is filed here because this batch found it. |
| coverage | See below. |

## Coverage

Rust (`cargo llvm-cov`, `coverage` variant, the `matched_path` tests of the lib, `equiv`
and `ub_safe` binaries): every line from `MatchedPathResult` to the end of
`MatchedPathCallback::trampoline` is covered.

C (gcov of the `coverage` build, private `GCOV_PREFIX`): in `index_apply_to_all` and
`apply_each_file`, every callback-facing line is reached: the call, skip, abort and
proceed paths of both invokers. Still uncovered are lines that belong to the
not-yet-wrapped add/remove/update routines, not to the callback:

- bare-repository and pathspec-init failures;
- `GIT_INDEX_ADD_CHECK_PATHSPEC` and `GIT_INDEX_ADD_FORCE`;
- the repository-less `update_all`;
- the `git_str_sets` OOM break;
- `index_apply_to_all`'s `NONE` / `UPDATE` / default actions, which no public routine
  passes.

## Validation

| run | result |
|---|---|
| `cargo check --workspace`, `cargo clippy --workspace` | clean |
| `cargo clippy --workspace --all-targets` | fails on 1 site that predates this batch, `tests/equiv/index.rs:1182` (`IndexIterator`, the type batch's item) |
| `cargo test --workspace --no-fail-fast` (plain) | 673 + 428 + 703 + 93 + 44 + 312 tests pass. `ub_safe::compile_fail` fails on 1 case that predates this batch, `diff_not_sync.rs` (`git_diff`, the type batch's item): the stable toolchain omits the `marker.rs` snippet its `.stderr` recorded. It passes on nightly. |
| `cargo-variant.sh asan-ubsan test --workspace` (LSan on) | every test passes, `compile_fail` included |
| `cargo-variant.sh tsan test -p libgit2 --lib --test equiv --test ub_safe -- index:: pathspec` | 34 + 26 + 38 tests pass |
| Miri (`cargo +nightly miri test`, `matched_path`) | 1 inline + 3 `ub_safe` tests pass; 3 that call libgit2 are ignored |
| BSan | only the `misc/pathspec-empty-entry-overread` reproducer, alone: exit 0 |
