# Review: libgit2 / config / wave 5 / batch 0 (the wave's symbols)

Reviewed commit `5fc58a580` (the review wave's base). It covers the three `symbol` items
that wave 5 batch 0 (`e532e6312`) landed or skipped:

| item | Rust | home | anchor |
|---|---|---|---|
| `git_config_add_backend` | `ConfigMut::add_backend(UnopenedBackend, level, Option<RepositoryRef<'r>>, force)` | `config/sys/config.rs` | `Wraps:` |
| `git_config_backend_foreach_match` | `BackendMut::foreach_match(Option<&CStr>, &mut ForeachCallback)`, through the private adapter `PreparedBackendWalk` | `config/config.rs` | `Wraps:` |
| `git_config_init_backend` | `Backend::new` (wave 4 batch 0's type run; not wrapped again) | `config/sys/config.rs` | `Wraps:` |

Both wrappers were re-read against `config.c:290-391` and `:584-625`. `add_backend` refuses
priorities outside `0..i32::MAX` (as `add_file` does) and a missing `open` slot. Otherwise
it hands C the exclusive config and the construction-phase backend, and gives up ownership
only on success. On failure it frees the backend through the allocation's own `free`, which
is what C's callers do (`git_config_add_file_ondisk`, `git_config_snapshot`, the CLI). C
opens the backend itself, so taking an `UnopenedBackend` also rules out parsing a memory
backend twice (the wave-4 hand-off). `foreach_match` builds the backend's iterator in Rust
first, then hands C a stack adapter whose `next` forwards to it. C still compiles and
matches the regex and dispatches the callback. Rust releases the iterator once, on every
path, and reports `next` failures that C would hide.

Config has waves 0–5 (`schedule.json`), so wave 5 is the sub-campaign's last wave and this
review owns its BSan gate, which covers everything config landed (see below).

| step | outcome |
|---|---|
| lifecycle discovery | Not applicable: a symbol-route batch. `git_config_init_backend` is a lifecycle routine scheduled as a symbol. Its policy is wave 4's `Backend::new`, an inert template, which was re-read against `GIT_INIT_STRUCTURE_FROM_TEMPLATE` (`common.h:31`): the version check comes before the copy, and `Backend::new` passes the current version. It was not wrapped again. |
| wavefront findings | No defect. The three records were checked against the C. In `add_backend`: ownership moves only on a non-negative return, the caller frees on failure even after `open` succeeded, `cfg` is written at `config.c:384`, and the repository goes only to `open`. In the walk: the regex leaks when the iterator fails (`:601-603`), and nothing is retained. In `init_backend`: an unsupported version leaves the storage unchanged. The store is unchanged. |
| safe boundary | Clean. The workset adds no public `unsafe fn`, no public raw pointer, and no `Send`/`Sync`. `crustify scan-unsafe --sites all` finds no `*_smell` site from the workset. The config files keep their earlier sites: `IteratorMut::set_backend` (wave 4, its `# Safety` was reviewed then), the wave-2 `foreach_with` trampoline deref, and two `#[repr(C)]` payloads (`Filtered`, and this wave's `PreparedBackendWalk`), which are inventory only. `undocumented_unsafe` stays at 45. There is no `ub_unsafe` suite to review. |
| UB | No true positive. Candidates checked by hand. Registration: forced replacement and duplicate levels while entries, snapshots and level views are still held. `add_backend` on writable level views (shared views are `ConfigArc` and cannot reach it) and on snapshots. Comparator overflow against a HIGHEST view's internal level `-1`. The `'r` repository borrow (invariant through `ConfigMut`). The adapter: the self-pointer's provenance, entries lent across C's callback, panics (the iterator is dropped before the panic resumes), regex failure before `next`, nameless entries under a pattern, and reentry, which the borrow checker rejects. All three compile-fail doctests fail for the intended reason (E0308, E0382, E0499, checked by compiling their bodies). ASan/UBSan (LSan on) and TSan are clean on the whole workspace. The BSan gate is below. |
| equivalence | No defect. The documented divergences (priority refusal, iterator built before the regex, the iterator's own error code, surfaced `next` errors, nameless entries) are reachable only through allocator failure or a custom backend, which needs the unsafe adoption seam. Two existing tests were strengthened. `equiv::config::backend_walk` now builds both backends with memory options and also compares each lent entry's `origin_path` and `include_depth`. `equiv::sys_config::registration` now compares the full `git_config_foreach` listing (order, values, backend types, levels) with `ConfigRef::foreach` after every registration step, not just five `get_i32` lookups. |
| internal | 1 report, fixed: `internal/backend-walk-miri-test-bypasses-production-setup`. The adapter's only Miri test rebuilt `foreach_match`'s setup in its own body, so putting the draft's provenance defect back into production passed every suite but BSan. The setup is now `PreparedBackendWalk::run`, shared by `foreach_match` and the test. The test drives it through the vtable slots in C's order, and Miri fails it once the defect is put back. |
| conventions | No defect. Names (`add_backend`, `foreach_match`), receivers (`ConfigMut`, `BackendMut`), homes and anchors match the batch. Every `SAFETY` comment fits `scan-unsafe`'s window. The touched files were rustfmt-clean at the base (`rustfmt +stable`) and still are. `cargo clippy --workspace --all-targets` is clean. |
| misc | None. |
| coverage | See below. |

C defect noted, not reported as a Rust finding (only an injected allocator reaches it,
through the unsafe `set_allocator`). If `git_config__add_instance` inserts the entry into
`readers` and then fails to grow `writers`, it frees the entry (`config.c:314`) and leaves
it in `readers`. `config_free` then reads it (`config.c:77`). Confirmed under ASan by
granting the readers growth in `backend_registration_allocator_failures_release_owned_inputs`
(the test refuses that growth, which is why it stops one allocation earlier):
`heap-use-after-free ... READ of size 8` in `config_free` (`config.c:77`), freed by
`git__free` in `git_config__add_instance` (`config.c:314`) called from
`git_config_add_backend` (`config.c:386`).

## Coverage

Rust (`cargo llvm-cov`, `coverage` variant; the workset's `lib`, `equiv`, `ub_safe` and
`ub_unsafe` tests): every line with counts in `ConfigMut::add_backend`, `BackendMut::foreach_match`
and `PreparedBackendWalk` (`run`, `iterator`, `next`, `free`) is executed. The regions are
covered when the unit-test and integration-test instances of the crate are combined.

C (gcov of the shared `coverage` build, private `GCOV_PREFIX`): `git_config_add_backend`
15/15 lines, `git_config_init_backend` 4/4, `git_config_backend_foreach_match` 18/20. The
two uncovered lines (`config.c:602-603`) are the leaking iterator-failure return. The
adapter makes them unreachable on purpose: its iterator slot cannot fail. The unsupported
init version and the NULL assertions are unreachable through the safe constructors.

## BSan gate (the sub-campaign's)

One `--no-run` build, then one `cargo-variant.sh bsan test` per test binary (`libgit2-sys`,
`lib`, `ub_safe`, `ub_unsafe`, `equiv`), all in parallel, each under `timeout 2400`, with no
`--skip` beyond the script's `compile_fail`. Each gate ran from a snapshot of its tree
(`git archive` or a `tar` of the working tree, plus `git init`, `include/`, and the builds
and `tests/` links), with its own target directory. Logs: `bsan-gate-before/` and
`bsan-gate-after/`.

- **Wave tip `5fc58a580`, before any fix.** Every binary exited 0 with no BorrowSanitizer
  report: `libgit2-sys` 0 tests, `lib` 633 passed and 4 ignored (49 s), `ub_unsafe` 92
  passed (69 s), `ub_safe` 666 passed and 5 ignored (472 s), `equiv` 401 passed and 8
  ignored (1545 s). Nothing to triage. Every ignored test carries an earlier wave's
  `cfg_attr(bsan, ignore)` marker or is the 2 GiB blob scan.
- **The tree landed here.** Same result: `lib` 633 passed and 4 ignored (48 s), `ub_unsafe`
  92 (69 s), `ub_safe` 666 and 5 ignored (465 s), `equiv` 401 and 8 ignored (1393 s). No
  report. That covers all ten workset tests, including the refactored adapter test.
  The snapshot was checked to match the landed `crustify/rust` and `include` byte for byte
  (`diff -r`).

This review added no BSan marker. The draft's provenance defect, put back as a mutation
for the internal report, was caught by BSan on
`ub_safe config::backend_walk::backend_walk_patterns_stops_panics_and_independent_reentry`
run alone. The fixed code passes that test.

## Validation

On the landed tree:

- `cargo check --workspace`, `cargo clippy --workspace --all-targets` and
  `cargo test --workspace` (plain; `lib` 636 passed and 1 ignored, `equiv` 409, `ub_safe` 672,
  `ub_unsafe` 92, doctests 43 + 274, `compile_fail`): clean.
- ASan/UBSan (LSan on) and TSan (`-Zbuild-std`) on every workspace test binary with
  `--skip compile_fail`, doctests included: clean.
- Miri on `backend_walk_adapter_borrowed_bases_and_lending` under Stacked Borrows and
  `-Zmiri-tree-borrows`: passes. With the draft's defect put back it fails, which it did
  not on the base.
- BSan as above.
- `crustify scan-unsafe --sites all`: no smell site from the workset, `undocumented_unsafe`
  45 → 45.

No C changed, so no C baseline was rerun. The shared prebuilds and `cargo-variant.sh` were
reused unmodified. Private outputs are under `/tmp/crustify-config-rw5-b0`.
