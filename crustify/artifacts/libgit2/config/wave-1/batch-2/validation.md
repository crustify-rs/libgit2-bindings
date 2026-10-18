# Config strings, write eligibility and snapshots — wrap batch 2

## Analysis and safe boundary

Verified `symbol` route, `wrap` objective, and the three requested definitions in
`src/libgit2/config.c`; all three anchors are in the supplied Rust home. Read
public declarations, callers, dependency records and C implementations, including
writer selection/comparison, file and memory backend setters, snapshot backend
construction/open/iteration/free, entry duplication, config allocation/free and
repository-cache invalidation. Submitted complete pointer and lifecycle findings
for these three symbols through Wavefront; prior findings are unchanged.

* `ConfigMut::set_string` exclusively borrows a live configuration. `name` and
  `value` are nonnull NUL-terminated strings borrowed only during the call; C
  normalizes/copies/escapes them, not retaining the caller's storage. NULL value
  is an invalid C input that returns an error and is not exposed as a safe value.
  Empty and non-UTF-8 values remain supported. Backend errors and return codes
  are copied into the existing `ConfigError`, including read-only, non-unique-key
  and file-lock errors. The existing config repository lifetime remains intact.
* `ConfigMut::set_write_order` accepts `&[ConfigLevel]`. Despite the mutable C
  declaration, every C access to `levels` only reads its `len` inline elements;
  C stores indices, never the array pointer. The transparent scalar wrapper is
  projected only at the small FFI seam, with NULL for an empty slice. Missing,
  repeated, custom and extreme levels remain valid equality comparisons. The
  length is checked before FFI (`len < INT_MAX`); this also bounds the signed
  writer-comparator subtraction. Empty input disables all writers, not resets
  them to defaults. Existing read-only backends remain read-only.
* `ConfigRef::snapshot` works on sole and shared owners and exclusive reborrows.
  C allocates a fresh config, snapshots each backend, opens it by copying entries,
  and hands out a sole reference on success. Failure frees partial construction
  and leaves output NULL. Snapshot backends record a source pointer used only by
  construction/open; getters, iterators, subsequent snapshots and free never
  dereference that source afterwards. No repository is retained. The result is
  `ConfigBox<'static>`, using the existing stateless ffibox `ConfigFree` policy;
  it can outlive source and repository. C may refresh source backend caches during
  construction, but no Rust reference covers C bytes. No new Send/Sync assertion,
  raw public argument/return or public unsafe function was added.

### Observed C write-order limitation

`git_config_set_writeorder` changes entry priorities then calls `git_vector_sort`,
but never clears the vector's cached sorted flag. Backends were inserted using
`git_vector_insert_sorted`, so sorting returns immediately. Existing relative
writer order is retained even when array order changes; level eligibility changes
correctly. The safe method explicitly documents this and the equivalence sequence
checks both permutations. This wrap batch preserves the authoritative C
implementation/ABI; no native copy of the defective sorting algorithm was added.
Selecting a single level reliably selects that backend. Fixing the upstream C
sort invalidation is separate from this wrapper worklist.

## Reusable builds

No C source or shim changed. Compiled inputs match campaign revision
`0551dfd4ad989b6a3d5683c0d4cf326c6efef929`, and `crustify/build.json` matches the
orchestrator's version 1. CMake caches match clang 18.1.3 and each configured
instrument for plain/coverage/ASan+UBSan/TSan, and clang 23.1.3 with the prepared
BSan plugin/runtime. Reused immutable `/target/crustify/builds` libraries through
`cargo-variant.sh`; Rust targets/logs are worktree-private and C coverage uses
agent-private `GCOV_PREFIX` outputs. Only the affected sys function allowlist
adds the three required bindings; bindgen regeneration and sys build/tests pass.
A wrap-only batch without compiled C changes needs no C baseline rebuild.

## Tests and validation

Four new public-safe integration tests cover discarded/replaced string and array
storage, mutable/shared reborrows, empty inputs, byte values/escaping, copied
errors, read-only and locked backends, non-unique keys, independent per-thread
configs, shared-owner snapshots, snapshots of snapshots, and source/repository
first drops. Existing compile-fail lifetime/thread tests apply; two new doctests
reject writes through a shared handle and writes aliasing a live shared reborrow.
No new unsafe API means no `ub_unsafe` suite is required.

Three new equivalence integration tests use independently owned configs/files,
raw C on one side and public wrappers on the other. Multi-call sequences compare
codes/errors, reads, snapshots, file bytes and state transitions after each step.
They include both writer permutations, absent/duplicate/extreme levels, disabled
writers, unchanged writes, invalid names, non-UTF-8 strings, locked-file recovery,
multivar rejection, nested snapshots, and partial snapshot failure after one
backend was already copied. Path-dependent error text is normalized only for the
independently owned filename. Three inline unit tests check the length boundary
without allocating a giant slice, output-config allocation failure, and backend
snapshot allocation failure after output construction; later recovery proves the
source remains usable and the partial owner is released.

Validation results and coverage figures are recorded below after final runs.
Raw logs, scan output, coverage JSON and C gcov files remain in the ignored
`crustify/tmp/config-wave1-batch2/` directory.

* `cargo check --workspace`, `cargo clippy --workspace` and additional
  `cargo clippy --workspace --all-targets`: passed without new warnings.
* Plain and full ASan/UBSan workspace tests passed: 599 unit tests (one existing
  ignored), 376 equivalence, 630 public-safe UB, 91 public-unsafe UB, and 274
  doctests (42 merged plus 232 other doctests). The sys crate builds and its
  zero-runtime-test binary passes. ASan leak detection and UBSan halt-on-error
  were enabled.
* TSan: the final config-filtered workspace run passed, including all new tests
  and the four-thread writer/snapshot test. The final full workspace run is
  recorded on completion below. A preliminary full run was invalidated by a
  concurrent rebuild in the same TSan target: isolated tests trying to spawn
  `current_exe()` received ENOENT after their running binary had been replaced.
  That was runner interference, not a library sanitizer finding; the full rerun
  uses an unmodified target while its test binaries execute.
* BSan: the four original exact test names passed separately under 300-second
  timeouts: `config::writes_and_snapshots::string_inputs_and_order_slices_are_not_retained`
  and `config::writes_and_snapshots::snapshot_copies_survive_source_and_repository_drop`
  (ub_safe), and `config::writes_and_snapshots::multi_call_writes_order_and_snapshot_match_c`
  and `config::writes_and_snapshots::partial_snapshot_failure_matches_c_and_releases_partial_owner`
  (equiv). The added exact multivar/lock-error tests also pass (ub_safe and equiv).
  They exercise foreign writes, input reborrows, non-retained slices/strings,
  parent-first drops and copied errors. No BSan whole-binary/workspace gate ran.
* Miri compiled the public integration boundary; all four new public-safe tests
  are explicitly ignored because they call libgit2. This batch adds no layout,
  initialization or Rust reference-projection assumption executable without C.
  Illegal shared writes/aliasing are separately covered by compile-fail doctests.
* `crustify scan-unsafe . --json --sites unsafe_fns_pub_smell raw_ptr_pub_smell
  unsafe_fns_priv_smell raw_ptr_priv_smell ref_to_type_wrapper_smell`: no smell
  sites in `config/config.rs`. Every new unsafe block has a specific safety
  comment. The scanner's one pre-existing undocumented site in that TU is the
  unchanged private unit-fixture libc extern block, not an added wrapper site.
  `git diff --check` passes and ownership-store semantic comparison identifies
  only this worklist's three new records.

## Coverage

Using the prepared C coverage library and Rust `cargo llvm-cov`, after adding the
focused backend-allocation-failure test:

* C `git_config_snapshot`: 18/18 executable lines (100%), including output-config
  allocation failure, backend-snapshot failure, add/open-backend failure after
  partial construction, cleanup and success.
* C `git_config_set_writeorder`: 16/16 executable lines (100%), with matched,
  missing, duplicate, absent and zero-length levels. C's failed length assertion
  cannot be reached through the checked wrapper.
* C `git_config_set_string`: 10/12 executable lines (83.33%). The only missing
  lines are NULL-value error construction and return: `&CStr` excludes this
  invalid input. Read-only, successful setter, setter error and owner-cache
  invalidation branches are reached by the config-filtered workload (the last
  through existing repository fixtures).
* Rust `snapshot`: 15/15 regions; `set_string`: 12/12; `set_write_order`: 22/23.
  The remaining write-order region propagates the length error and requires an
  at-least-8-GiB level slice to invoke publicly; its validating helper is tested
  at both sides of INT_MAX and usize::MAX without allocating such a slice.
  The helper has 9/9 regions and status conversion 5/5. Snapshot's additional
  NULL-on-success fallback closure is unreachable with a conforming C backend:
  C always returns the freshly constructed config on success.

No compiled C change, unprepared instrument, new public unsafe API, or native
Rust port is hidden behind these coverage numbers.

Final TSan full-workspace rerun passed with `--test-threads=1` (tests that
internally spawn concurrent workers still do so): 599 unit tests, 376 equivalence,
630 ub_safe, 91 ub_unsafe and 274 doctests, with one existing ignored unit test.
The extra exact-name BSan backend-snapshot allocation-failure unit test also
passed under its 300-second timeout. All prepared instruments have been covered.

## Landing rebase

The initial local landing push correctly rejected non-fast-forward. Rebased only
the batch branch onto wave commit `512be0487` (configuration entry/map wrappers).
Resolved append-only allowlist and test-module conflicts by retaining both
batches; the ownership findings and implementation merged without conflict.
Used the newly available safe `ConfigRef::entry` and `EntryRef::value` getters
for this batch's public-safe numeric observations, replacing its temporary raw
lookup dependency. File attachment still uses the existing fixture because the
backend-attachment wrapper is not yet available at this wave tip.

All workspace checks, clippy and regressions, coverage, static safety scan and
exact-name BSan tests were rerun after this rebase; final logs carry the `-final`
suffix. Miri compiled the updated safe boundary and ignored its four foreign-call
tests as expected. Coverage and workset-smell results are unchanged.

After the first rebase, full plain/ASan/TSan workspace runs passed: 601 unit,
378 equiv, 632 ub_safe, 91 ub_unsafe and 282 doctests (42 merged plus 240 other),
with the same one ignored unit test. All seven exact BSan reruns passed.

The wave advanced again to `5a0f495b6` (safe file construction/value access), so
another non-fast-forward was handled by rebasing only the batch branch. Append
conflicts were resolved preserving all tests/allowlist additions; ownership was
resolved by keeping wave findings and resubmitting these three symbols through
Wavefront. CLI canonicalization reordered some existing records, but semantic
comparison confirms no other record changed. This batch's temporary backend
setup now uses safe `add_file_ondisk`; its public-safe reads, Rust-side equivalence
reads, and private numeric fixture use `get_i32`. Only the C-reference side keeps
raw constructor/getter calls. No new raw adoption or FFI fixture was introduced.

Validation after this last rebase is in `*-land` logs: workspace check, clippy
(including all targets), full plain and ASan/UBSan workspace regressions, all
config-filtered TSan tests, the same seven exact BSan cases, Miri's compiled public
boundary, coverage and the static safety scan. The prepared instruments' workset
obligations were rerun; no whole-workspace BSan invocation occurred.

Last-rebase results: all checks/clippy and full plain/ASan workspace tests passed
(603 unit, 381 equiv, 638 ub_safe, 91 ub_unsafe, 282 doctests; one ignored unit).
All config-filtered TSan binaries passed (25 unit, 28 equiv, 25 ub_safe and six
existing trace ub_unsafe tests). All seven exact BSan cases passed. Scan has no
workset smell sites; Miri compiled and explicitly ignored the four foreign-call
safe scenarios. Final C coverage remains snapshot 18/18, write-order 16/16 and
string setter 10/12 lines; Rust region results and documented exclusions are
unchanged. No unrelated semantic ownership changes remain.
