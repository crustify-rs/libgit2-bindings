# Review: libgit2 / config / wave 1 / batch 1 (the wave's symbols)

Reviewed commit `d79771f6f` (the review-wave base). It covers the 28 `symbol` items the
wave's batches landed or reused. Batch 1 (`5a0f495b6`) wrapped file construction, typed
access, deletes, multivars, views and search. Batch 2 (`760d8dc91`) wrapped `set_string`,
`set_writeorder` and `snapshot`. Four lifecycle items were already emitted by wave 0 and
reused: `git_config_new` / `git_config_free` (`ConfigFree`), `git_config_lock`
(`ConfigMut::lock`) and `git_config_backend_memory_options_init`
(`BackendMemoryOptions::new`). All live in `config/config.rs`, `config/sys/config.rs`
and, for the transaction owner, `refs/transaction.rs`.

Config has waves 0–5 (`schedule.json`), so wave 1 is not the sub-campaign's last and
there is no BSan gate here. The one reproducer emitted checks an error value, not a
memory error, so there was nothing for BSan to run.

| step | outcome |
|---|---|
| lifecycle discovery | Not applicable: no `type` or `raw-lifetime` items. The four lifecycle routines in the worklist each carry one anchor on the strategy that calls them (`ConfigFree`'s `CDrop`/`CNew`, `ConfigMut::lock` → `ConfigTransaction`, `BackendMemoryOptions::new`), and no wrapper duplicates them. |
| wavefront findings | 1 report, resubmitted: `wavefront/retained-borrows-recorded-as-transient`. `git_config_lock`'s `config` (stored as `tx->cfg`) and `git_config_add_file_ondisk`'s `repo` (stored as `b->repo`) were recorded as borrowed for the call only (`arg:<own>`). Now `arg:out` and `arg:config`, with notes citing the retention. The other 26 records were read against the C and hold. |
| safe boundary | No defect. The workset adds no public `unsafe fn` and no raw pointer on the public surface, so there is no `ub_unsafe` suite to add. `crustify scan-unsafe --sites all`: no `*_smell` or `*_outside_impl` site in the three files, before or after. Three SAFETY comments fell outside the scanner's lexical window because of a `let rc =` line split (`set_string`, `set_bool`, one equiv call) and one was missing on the test-only libc `extern` block. Rebinding the pointer first brings the workset files to 0 undocumented sites (crate total 47 → 44). |
| UB | No true positive. Checked by hand: `get_string`'s borrowed result (C returns it only when every writer is `readonly`, i.e. snapshot backends, whose entry list the backend keeps; the wrapper copies it before any other call). Level views keep their parent and its `'r` borrowed, with an exclusive parent for writable views, so no backend is locked twice. `ConfigArc` cannot become a box (no `CRefClone`), so a shared view never yields `ConfigMut`. Snapshots are `'static` (`config_snapshot_open` ignores `repo`; the source is used only during construction). `ConfigMut` is invariant in `'r`, so no shorter-lived repository reaches a `'static` config. `add_file`'s priority range keeps both C comparators from overflowing. `set_write_order` passes a read-only slice and checks its length before C. Repository-owned configs are reachable only shared, so `set_string`'s owner-cache clear never runs from safe Rust. The fixed tree is clean under ASan/UBSan (LSan on), TSan and Miri. |
| equivalence | No equivalence defect where the wrapper diverges from C. The one stale-error finding copies C exactly; see misc. Hardened: the wave-0 owner tests now compare C with the wrappers instead of C with C (see conventions). |
| internal | Nothing to fix in the private implementation beyond what the misc fix touches. |
| conventions | 2 reports, fixed. `conventions/ondisk-names-transliterate-c`: renamed `Config::open_ondisk` → `Config::open` and `ConfigMut::add_file_ondisk` → `ConfigMut::add_file`. `conventions/config-tests-bypass-the-wrappers`: the ub_safe suites still drove configs through raw `get_int32` / `set_int32` / `write_through_file` fixtures, including a write through a shared repository config that safe code cannot make. The equiv owner tests ran the same raw steps on both sides. Fixtures removed; both suites use the safe API. |
| misc | 3 reports, fixed. `misc/delete-readonly-error-is-stale` (reproducer included): `git_config_delete_entry` / `_multivar` return `GIT_EREADONLY` without setting an error, so the wrappers returned a stale or "no error" `ConfigError`. They now clear the error and report `cannot delete '<name>': the configuration is read-only`. `misc/transaction-lacks-set-string`: `ConfigTransaction` could stage every write but a string; added `set_string`. `misc/delete-docs-misdescribe-multivars`: corrected the copy-pasted delete/multivar docs (`delete_entry` refuses multivars). |
| coverage | See below. |

## Coverage

Measured with the `coverage` variant (`cargo llvm-cov`, plus C gcov through a private
`GCOV_PREFIX`) over the config-filtered tests.

- **Rust**, `config/config.rs`, comparing the tree after the misc fixes with the final
  tree: lines 998/1020 → 1076/1091, regions 1874/1902 → 2049/2070.
  `config/sys/config.rs` stays at 100 %. New focused tests:
  - `unit_tests::without_runtime_fails_cleanly` now also reaches the unpinned-runtime
    returns of `open`, `open_default`, `find_global` / `_system` / `_xdg` and
    `parse_path`.
  - `unit_tests::opening_routines_report_allocation_failure` covers allocation failure
    in both handles' `open_level` / `open_global`, plus `open`, `open_default` and
    `add_file`, and a failed backend entry after the new config was built.
  - `unit_tests::write_order_refuses_oversized_slices` drives the public
    `set_write_order` with an `INT_MAX`-long, lazily zeroed slice.
  - `equiv::file_access::missing_level_views_match_c` covers missing-level views through
    both handles.
  - `equiv::file_access::isolated_search_and_default_failures_match_c` runs in a child
    process with `$HOME` holding a malformed `.gitconfig` and an empty system search
    path. It covers `open_default` failing and `find_system` / `find_xdg` not found.

  Still uncovered: the seven NULL-on-success fallbacks after `ConfigBox::from_c`, which a
  conforming C callee never takes, and two test-helper lines.
- **C**, at 100 % of lines: `git_config_add_file_ondisk`, `_open_ondisk`, `_open_level`,
  `_open_global` and `_open_default` (up from 93 / 89 / 73 / 88 / 81 %). All the others
  in the worklist were already there, apart from `git_config_new` (gcov reports 82 %,
  but every line has a count; it miscounts the alloc-check macro) and
  `git_config_set_string` (9/12). Its remaining lines are the NULL-value error, which
  `&CStr` excludes, and the repository-owner cache clear, which only a repository-owned
  config reaches; safe Rust holds those only through a shared `ConfigArc`, which cannot
  write.

## Validation of the landed tree

`cargo check --workspace`, `cargo clippy --workspace --all-targets` (no warnings) and
`cargo test --workspace` were run under plain, ASan/UBSan (LSan on) and TSan
(`-Zbuild-std`). The base, run under plain only, gave `lib` 603 passed / 1 ignored,
`equiv` 381, `ub_safe` 638, `ub_unsafe` 91, doctests 42 + 240. The fixed tree gives `lib` 607 / 1 ignored,
`equiv` 383, `ub_safe` 639 (`compile_fail` included), `ub_unsafe` 91, doctests 42 + 240,
all clean under all three. Miri on the workset's inline tests: 12 passed, 18 ignored
(they call libgit2). The reproducer exits 101 on `d79771f6f` and 0 on the fixed tree.

## Noted, not changed

- **`get_i64` and `parse_i64` word C's silent suffix-tail failure differently.**
  `get_i64` says `failed to parse configuration value as an integer`, because it does
  not have the value. `parse_i64` quotes the value, as C does for its other failures.
  Both are correct and documented. Making them match would cost a second lookup on the
  failure path, which could also race a refresh.
- **`git_config_set_writeorder` does not re-sort existing writers.** C never clears the
  vector's sorted flag. Batch 2 documented this and kept it, as a wrap must.
- **`git_config_lock` leaks the backend lock on OOM** (already noted in the wave-0
  review): C does not unlock when `git_transaction_config_new` fails.
- **Shared views are read-only by design.** `ConfigRef::open_level` / `open_global`
  return a `ConfigArc`, where C's view would be writable. That is what keeps a second
  lock of a shared backend out of safe code. A writable view needs the exclusive handle.
