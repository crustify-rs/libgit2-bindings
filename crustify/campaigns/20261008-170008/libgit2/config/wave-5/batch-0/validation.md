# libgit2/config wave 5 batch 0 — symbol wrap

## Worklist and ownership

Verified the homogeneous symbol route and authoritative `wrap` objective.
Definitions/declarations, dependency records, callers, backend construction/free,
config instance refcounts, level views, iterator callbacks and Rust consumers were
inspected. Findings for all three items were submitted through Wavefront `--update`.

- `git_config_init_backend` already has its sole anchor on `Backend::new`, emitted
  by the backend type batch. Reused that safe initialized constructor; no duplicate
  wrapper or anchor. Its template owns no resources and cannot be registered.
- `ConfigMut::add_backend`, in the assigned `sys/config.rs`, consumes
  `UnopenedBackend`. C opens exactly once and takes ownership only on success.
  On failure the allocation-specific ffibox policy frees the input, even when C
  already opened it. Invalid priorities and absent open callbacks are refused.
  Priorities in `0..i32::MAX` avoid comparator overflow. Optional repositories are
  typed borrows lasting for the config's lifetime. The existing cfg identity
  remains an identity, not a dereferenceable parent reference.
- `BackendMut::foreach_match`, in the assigned `config.rs`, takes an exclusive
  backend borrow and the existing synchronous `ForeachCallback`. Entry borrows
  cannot escape, same-backend reentry is rejected by borrowing, independent
  nested walks work, stop codes remain distinct from errors, and panic resumption
  occurs only after both regex and iterator cleanup.
- No new public unsafe API, raw public arguments, Send/Sync assertion, owner
  policy, shim, C change, or Rust ABI export. Only the two needed function names
  were added to the agent-owned sys allowlist; bindings regenerated during builds.

## C-defect avoidance and regression evidence

C's backend walk leaks a successfully compiled regex if iterator construction
fails, hides negative next errors, and treats positive next errors as successful
output. A private borrowed ABI adapter prepares an ffibox iterator before C
compiles its regex and records terminal errors. C still performs name matching,
callback dispatch and regex disposal. Rust drops the actual iterator once, also
on regex failure or callback panic. Iterator construction now precedes regex
compilation; its actual error code is retained instead of C's hard-coded `-1`.
These deliberate divergences are covered by inline corrected-behavior tests;
unaffected return/error, entry field/order, callback, stop and state behavior is
compared with independently owned C inputs in two multi-call equivalence tests.

Nullable entry names remain accepted by unfiltered callbacks. Filtering rejects
such names with Config-class `GIT_EINVALIDSPEC` before C's matcher can dereference
NULL. The inline regression exercises both distributions.

Miri and exact-name BSan caught a raw-tag invalidation during adapter setup in the
initial draft. Setup now projects its self-pointer field through the same raw
provenance that C retains. The Rust-only lending/provenance regression and the
foreign-boundary walk regression pass both instruments after this correction.

Four new public safe-UB tests cover ownership transfer, duplicate rejection,
replacement, failed open, priority bounds, copied inputs, retained entries,
snapshots, optional repository lifetime, shared/exclusive reborrows, regex
patterns, empty/valueless/non-UTF-8 entries, stop extremes, panic recovery and
independent reentry. Four inline tests cover private adoption/release counts,
missing slots/version, safe allocator-failure cleanup, iterator errors, nullable
names, and Rust-only adapter layout/provenance. Three compile-fail doctests cover
registration state, moved owners and callback aliasing. Existing init equivalence
and type/handle lifetime tests are reused. No new unsafe-UB suite is appropriate.

## Reused builds and final regressions

C inputs match the orchestrator's current `/target` tree at
`7634e18ffb0e9f182d6da1ad9c1b0222644bbdcc`; tracked source/header/dependency/CMake
content SHA-256 is
`e62914a00b0fa122fe003f48ca727f5b73c12c2bd87bd660952d63589e9d8293`.
`build.json` version is 1. All five reusable CMake caches match the prescribed
compiler and instrumentation: clang 18.1.3 except the supplied BSan compiler and
plugin. These current shared libraries export the memory constructors added in
wave 4. Reused `/target/crustify/builds` and `cargo-variant.sh` without modification.
Rust targets, logs and coverage outputs are agent-private. C gcov writes are
redirected with private `GCOV_PREFIX`, and gcno notes copied privately for analysis.
No C rebuild or full C baseline is needed for this C-unchanged wrap batch.

Final commands all exit zero:

- Workspace `cargo check`, `cargo clippy --all-targets`, and full `cargo test`.
- Full matching ASan/UBSan and TSan workspace tests: 636 unit (one existing
  ignored), 409 equivalence, 672 safe-UB, 92 existing unsafe-UB, and 317 doctests
  (43 merged plus 274 others) pass. Sys layout checks compile successfully.
- BSan: seven new foreign-write/retained-pointer tests run individually by exact
  name under 600-second timeouts: the four safe-UB tests and the registration
  release-count, corrected-walk-error, and Rust-only adapter unit tests. No
  BSan workspace or whole-binary run. All pass without a sanitizer report.
- Miri: Rust-only `backend_walk_adapter_borrowed_bases_and_lending` passes;
  foreign tests are explicitly ignored. Callback lifetime/thread/alias rejection
  is enforced by existing and new compile-fail tests.
- `crustify scan-unsafe . --json --sites all`: no new workset unsafe/raw smell.
  The existing public iterator setter at `sys/config.rs:720` is untouched. New
  unsafe operations are narrow documented FFI/layout/private callback seams;
  Clippy's undocumented-unsafe-block deny passes. `git diff --check` passes.

Logs, full profiles and C traces are under `/tmp/crustify-config-w5-b0`.

## Coverage and remaining C limitations

Final `backend_` workset coverage includes the existing backend constructor/init
regressions. `coverage-summary.json` records aggregated executable regions across
integration/unit binaries: registration 36/36, backend walk plus callback closure
63/63, adapter next 43/43, iterator 8/8, and free 2/2. Inspection prompted focused
error/lending/name tests; no Rust executable workset region remains uncovered.

Scheduled C executable lines: add_backend 15/15; init_backend 4/4; backend walk
18/20. The only uncovered walk lines (602–603) are its leaking iterator-failure
branch, deliberately unreachable through the prepared adapter. NULL assertions
and unsupported init versions are unreachable through the safe constructors.

A transitive pre-existing C limitation is not corrected here: if writers-vector
allocation fails *after* readers insertion in `git_config__add_instance`, C leaves
a dangling reader. Deliberate allocator-failure tests stop before that defective
partial-insertion path (instance, entry, and readers growth failures are covered).
Like the previously documented C list-copy allocation failures, this path is not
intentionally triggered through the unsafe allocator override in the workset
regressions; this validation does not claim recovery from that C defect.
