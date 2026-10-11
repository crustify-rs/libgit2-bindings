# Review: libgit2 / config / wave 0 / batch 0 (the wave's types)

Reviewed commit `b83f28c34` (the review-wave base). It covers the four `type` items that
the wave's batches 0 and 1 landed:

| item | wrapper | home |
|---|---|---|
| `git_config_backend_memory_options` | `BackendMemoryOptions<'o>` value, `…Ref` / `…Mut` handles (batch 0) | `config/sys/config.rs` |
| `git_config_level_t` | `ConfigLevel`, a transparent integer newtype (batch 0) | `config/config.rs` |
| `git_configmap_t` | `MapType`, a closed checked enum (batch 1) | `config/config.rs` |
| `git_transaction` | `Transaction<'r>` / `TransactionRef` / `TransactionMut`, `TransactionBox<'r>`, `ConfigTransaction<'c, 'r>` (batch 1) | `refs/transaction.rs` |

Config has waves 0–5 (`schedule.json`), so this is not the sub-campaign's last wave and
there is no BSan gate here. The one reproducer emitted checks equivalence (it compares
return codes; nothing for a sanitizer to catch), so BSan had nothing to run either.

| step | outcome |
|---|---|
| lifecycle discovery | No gap. `git_transaction` has three constructors and one releaser: `git_transaction_new` (`Transaction::new`), `git_config_lock` → `git_transaction_config_new` (`ConfigMut::lock`), and `git_transaction_free` (`TransactionFree`), which picks pool teardown or `git__free` by C's private discriminator. A grep for `git_transaction **` over `include/git2` finds only those two public routines; commit is a symbol. `git_config_backend_memory_options` has only `_init` (`BackendMemoryOptions::new`) and the `_INIT` macro; C never frees, clones or retains it (`config_backend_new` strdups both strings). The two enums have no lifecycle. `query symbols --lifetime-for` agrees for all four. |
| wavefront findings | No defect. The type records (memory options' borrowed nullable strings; transaction's borrowed `repo` / `cfg`, owned `db`) and the lifecycle-routine records were read against the C and hold. |
| safe boundary | No defect. The workset adds no public `unsafe fn` and no raw pointer on the public surface; the `pub(crate)` `from_ptr` / `as_ptr` / `as_mut_ptr` are seams. `crustify scan-unsafe`: no `*_smell` or `*_outside_impl` site in the three files before or after. Counters changed only by the six new `unsafe impl Send/Sync` (`unsafe_impls` 216 → 222); undocumented sites stayed at 44, none in the workset. |
| UB | No candidate survived. Checked by hand: the memory options' covariance in `'o`, with the `Mut` handle invariant and outlives constraints carrying the stored borrows; getters lending `'a ≤ 'o`; clone sharing immutable borrows; the transaction owner and handle variance; `ConfigTransaction` holding the exclusive `ConfigMut` (no relock, no alias, no outliving the config, all compile-fail tests), its field drop order (unlock before the borrow ends; `git_config_unlock` never dereferences `cfg`), its private raw owner never lent as a reference transaction, and `mem::forget` (it leaks a lock; the next lock fails cleanly with `GIT_ELOCKED`, no UB). Plain, ASan/UBSan (with LSan) and TSan (`-Zbuild-std`) are clean on the whole workspace before and after, apart from the intermittent glibc `tzset` report in tests outside the workset (see misc). Miri on the workset's Rust-only inline tests: 6 passed. |
| equivalence | 1 report, fixed: `equiv/config-lock-discards-error-code`. `ConfigMut::lock` returned an `ErrorBuf`, losing `GIT_EREADONLY` / `GIT_ELOCKED`; it now returns the new `ConfigError` (code plus error). Reproducer included. Regression test: `tests/equiv/transaction.rs::config_lock_codes_match_c` (7 steps: empty, held, free, then two backends where the higher-priority `APP` file is the writer, held, free). Each step compares code, class, message and which files are locked. |
| internal | Nothing to fix in the private implementation. New inline test `refs::transaction::unit_tests::out_of_memory::constructors_report_allocation_failure`: both constructors under a refusing allocator report `NoMemory`, leave no lock file, and work again afterwards. |
| conventions | 2 reports, fixed. `conventions/map-type-tests-outside-config-suites`: `MapType`'s ub_safe/equiv tests were in the transaction suites, and its inline test was in a second module after an item. Both moved to the config suites and into the single `unit_tests`. `conventions/map-type-missing-handwritten-anchor`: added the anchor. |
| misc | 4 reports. `misc/memory-options-tests-drop-non-drop` (fixed): the only three `clippy` warnings on the base were `drop_non_drop`, all in this workset. `misc/memory-options-not-send-sync-or-debug` (fixed): added `Send`/`Sync` for the options and their handles, as `TreeUpdate` / `FilterOptions` have, plus `Debug`; covered by a TSan test, an inline test and a `compile_fail,E0597` doctest. `misc/refdb-open-frees-uninitialised-pointer-on-oom` (documented): a libgit2 crash on allocation failure in `git_refdb_open`, reached through `Transaction::new`, now documented on the wrapper. `misc/equiv-reference-writers-race-on-tzset` (fixed, outside the workset): two equivalence tests from earlier sub-campaigns wrote references on shared threads and failed the TSan gate on glibc's `tzset` in two of three runs. They now run in child processes. |
| coverage | Rust (`cargo llvm-cov`, `coverage` variant, line coverage): `config/sys/config.rs` 100 %, `config/config.rs` 99.7 %, `refs/transaction.rs` 97.8 %. Uncovered: the `!runtime_pinned()` returns of `Transaction::new` and `ConfigTransaction::lock` (unreachable, because a repository or config exists only once the runtime is pinned) and failure formatting in test helpers. C (gcov of the `coverage` build, private `GCOV_PREFIX`): `git_transaction_config_new`, `git_transaction_free`, `git_config_lock`, `git_config_unlock` and `git_config_backend_memory_options_init` are fully covered. In `git_transaction_new`, `git_pool_init`'s failure branch (it cannot fail) and the refdb failure branch (`transaction.c:99`) remain; the latter crashes in C, see the misc report above. The new OOM test covers the `git_pool_mallocz` failure branch. |

## Validation of the landed tree

Rebased onto batch 1's `210d569ac`. `cargo check --workspace` and `cargo clippy --workspace
--all-targets -- -D warnings` are clean (the base had 3 warnings). `cargo test --workspace`
under plain, ASan/UBSan (LSan on) and TSan (`-Zbuild-std`) gives `lib` 596 passed / 1
ignored, `equiv` 373, `ub_safe` 626 (`compile_fail` included), `ub_unsafe` 91, doctests
230, all clean. Before the isolation fix in
`misc/equiv-reference-writers-race-on-tzset`, TSan failed `equiv` intermittently on
glibc's `tzset`. After it, the TSan `equiv` binary passed three consecutive runs. Miri on
the workset's inline tests: 6 passed, 4 ignored (they call libgit2).

## Noted, not changed

- **`git_config_lock` leaks the backend lock on OOM.** If `backend->lock` succeeds and
  `git_transaction_config_new` then fails to allocate, C returns without unlocking. The
  file backend's lock stays held until the backend is freed. This is a C defect, reachable
  only by allocation failure; the wrapper reports the error.
- **Custom backends without `lock`.** `git_config_lock` calls `backend->lock`
  unconditionally on the highest writable backend. No public wrapper can add a custom
  backend yet. The wave that wraps `git_config_add_backend` should refuse a writable
  backend with a NULL `lock` / `unlock`.
- **`Transaction::new` keeps `ErrorBuf`.** `git_repository_refdb` collapses every refdb
  error to `-1`, so its code carries nothing beyond `GIT_ERROR`.
- **String getters lend `'a`, not `'o`.** `BackendMemoryOptionsRef::backend_type` /
  `origin_path` could lend the stored borrow's own `'o`. Lending `'a` is sound, only
  narrower, and changing it is an API widening rather than a defect.
