# Review: libgit2 / config / wave 4 / batch 0 (the wave's types)

Reviewed commit `e797d4624` (the review-wave base), rebased onto `074307ec6` (the wave's
symbol review) before landing. It covers the one `type` item that wave 4 batch 0
(`beeda6a07`) landed:

| item | wrapper | home |
|---|---|---|
| `git_config_backend` | `Backend` / `BackendRef` / `BackendMut` (`define_ctype!`), `BackendBox = CBox<Backend, BackendFree>`, `BackendFree` (the allocation's `free` slot), construction phase `UnopenedBackend` with consuming `open` (`BackendOpen`), constructors `Backend::new` (`git_config_init_backend`), `Backend::from_string` / `from_values` (`git_config_backend_from_string` / `_from_values`), dispatch `get` / `iterator` / `snapshot` / `set` / `set_multivar` / `delete` / `delete_multivar` / `lock` → `BackendTransaction` / `BackendUnlock`, metadata `version` / `readonly_raw` / `config_identity`, and the typed `IteratorRef::backend` / `unsafe IteratorMut::set_backend` that replaced wave 3's raw seam | `config/sys/config.rs` |

Config has waves 0–5 (`schedule.json`), so wave 4 is not the sub-campaign's last wave and
this review runs no BSan gate. Its one reproducer was run alone under BSan.

| step | outcome |
|---|---|
| lifecycle discovery | No Rust gap. Every `git_config_backend **` in `include/git2` was checked: the two memory constructors (wrapped, adopted into `UnopenedBackend`) and `git_config_add_backend` (takes ownership; wave 5). Non-public producers reachable from Rust are the `snapshot` slot (`git_config_backend_snapshot`, wrapped by `BackendMut::snapshot`, which opens the result at once) and `git_config_backend_from_file` (only through `git_config_add_file_ondisk`). The only releaser is the allocation's own `free` slot (`BackendFree`). There is no clone or up-ref. `init` builds an inert template (`Backend::new`), never adopted. |
| wavefront findings | 1 report, resubmitted (`wavefront/backend-snapshot-and-file-constructors-unrecorded`): `git_config_backend_snapshot` and `git_config_backend_from_file` had no lifetime record. The 14 field notes were checked against `config_mem.c`, `config_snapshot.c`, `config_file.c` and `config.c:384` (the only `cfg` write) and are correct. |
| safe boundary | No defect. The public unsafe surface is `IteratorMut::set_backend`. Its `# Safety` (callback compatibility; liveness is typed by `BackendRef<'c>`) matches C: built-in `next`/`free` never read the slot. `ub_unsafe/sys_config.rs` covers it. Raw adoption stays the ffibox seam (`BackendBox::from_c`). `as_ptr` / `as_mut_ptr` / `from_ptr` are `pub(crate)`. `Backend` has no `CZeroable` and no safe raw constructor, so safe code cannot install callbacks. Scan smells in the file are unchanged: `unsafe_fns_pub_smell` 1 (`set_backend`), `wrapper_newtypes_undeclared` 1 (wave 3's `Filtered` payload). |
| UB | No true positive. Checked: entries from `get` hold a list reference and outlive the backend and snapshots (`ub_safe` drops the parent first). Snapshots read their source only during the immediate open, so snapshot chains survive their sources. `open` runs at most once (it consumes `UnopenedBackend`). An iterator borrows the backend exclusively, and a transaction cannot reborrow it. No `Send`/`Sync` is asserted. ASan/UBSan (LSan on), TSan and Miri (the Rust-only backend tests) are clean. |
| equivalence | 1 report, fixed (`equiv/writable-get-ignores-backend-get-slot`): `BackendMut::get` took a snapshot instead of calling a writable backend's own `get`, so it failed for backends without a `snapshot` slot. The stated reason (an uninitialised status in `config_file_get`) does not hold: the defect is in `config.c`'s `get_entry`, which this wrapper does not use. New `equiv` tests: the reproducer's backend, compared through `compare_get`. Memory options (`backend_type`, `origin_path`) through both constructors, compared per entry and per iteration. A snapshot of a snapshot after its source is freed. C's empty-key open failure (code and error). |
| internal | 1 report, fixed (`internal/transaction-rollback-panics-in-drop`): a failed rollback panicked in `Drop`, which aborts during unwinding. It is now discarded, as `git_transaction_free` does. |
| conventions | 2 reports, fixed: `conventions/backend-unit-tests-outside-unit-tests` (two extra top-level test modules) and `conventions/backend-safety-comments-outside-lexical-window` (5 → 0 lexically undocumented sites in the file). |
| misc | 2 reports, fixed: `misc/missing-backend-slot-error-is-stale` (refusals carried the thread's previous error) and `misc/backend-phase-types-lack-debug`. |
| coverage | See below. |

Rebase note: the symbol review (`074307ec6`) moved the iterator allocation-failure test
into `config.rs`'s `unit_tests` and reused its refusing allocator there. This review's
new backend allocation test does the same. It lives in
`config/sys/config.rs`'s `unit_tests::backends` and imports `isolated`,
`install_refusing` and `ALLOWED_ALLOCATIONS` from `config.rs`'s test module, which is
now `pub(crate)` with those three items `pub(crate)` (test-only code).

C defect noted, not reported as a Rust finding: `git_config_backend_from_string` /
`_from_values` leak the copied `backend_type` and `origin_path` when the later copy
of the input (`git_str_set` / `git_strlist_copy`) fails (`config_mem.c:357`, `:378`).
LSan confirms it: refusing the 5th allocation leaks "in-memory" and "origin", 34 bytes
in 4 objects. The allocation test stops before that allocation.

Hand-off for wave 5: `git_config_add_backend` calls the backend's `open` itself. Its
wrapper should take an `UnopenedBackend`: an already opened memory `BackendBox` would
be parsed again, which duplicates every entry.

## Coverage

Measured with the `coverage` variant (`cargo llvm-cov` over the `backend` / `sys_config`
filtered `lib`, `equiv`, `ub_safe` and `ub_unsafe` tests) and C gcov from a private
coverage build (`/tmp/crustify-review-config-w4-b0/coverage`, counters cleared first).

- **Rust**, the backend block of `config/sys/config.rs` (from `define_ctype!(Backend…)`
  to `mod unit_tests`): 202 of 204 lines with counts are executed. Still uncovered: the
  two `runtime_pinned()` failure returns in `from_string` / `from_values`, which need
  libgit2's initialisation to fail.
- **C**: `config_mem.c` rose from 86.21 % to 91.63 % (the new allocation test covers
  `config_backend_new`'s refusals: shell, list, both strings). Still uncovered: the
  leaking input-copy failures above, which are deliberately skipped. Also the parse
  callbacks' allocation failures, and `config_memory_unlock`, which no transaction can
  reach because `config_memory_lock` always fails. `config_snapshot.c` is at 88.68 %.
  Its gaps are mutex and list-duplication failures, `config_snapshot_unlock` (same
  reason), the NULL `free` guard (owners are non-NULL), and the source-iterator failure
  in `config_snapshot_open`, which no built-in source returns.

## Validation of the landed tree

No C, header, CMake, bindgen allowlist or `build.json` changed in this review. The
shared prebuilds under `/target/crustify/builds` do not export
`git_config_backend_from_string` / `_from_values`, which wave 4 batch 0 made
`GIT_EXTERN`. So all five variants were built privately at
`/tmp/crustify-review-config-w4-b0/<variant>` from this tree with `build.json` v1's
configure lines (`-j24`). Rust outputs went to agent-private `CARGO_TARGET_DIR`s there.
The C baseline was not rerun (no C change in this review).

- `cargo check --workspace`, `cargo clippy --workspace --all-targets`: clean.
  `rustfmt +stable --edition 2024 --check` is clean on the touched files.
- `cargo test --workspace`, plain and ASan/UBSan (LSan on): lib 632 (1 ignored, the
  existing one), equiv 407, ub_safe 668, ub_unsafe 92, sys 43, doctests 271, all
  passing, no sanitizer report. The ASan/UBSan run on the wave tip before any fix was
  equally clean (630 / 403 / 666 / 92 / 43 / 271).
- TSan (`-Zbuild-std`): the workset's `sys_config` / `config::sys::config` tests across
  `lib`, `equiv`, `ub_safe`, `ub_unsafe` (17 + 11 + 12 + 1), all passing.
- Miri: `config::sys::config` unit tests, 10 passing, 7 ignored (they call C). The
  passing ones include the Rust-only backend tests (layout, stale identity,
  transactions, failed rollback on drop).
- BSan: only the reproducer `equiv/writable-get-ignores-backend-get-slot`, alone, on the
  fixed tree: clean, exits 0.
