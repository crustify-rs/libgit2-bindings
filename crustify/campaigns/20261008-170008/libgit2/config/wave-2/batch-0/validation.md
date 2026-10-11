# Backend entry — wrap, type route

## Analysis and API

Verified the worklist's homogeneous type route and authored the requested
`config/sys/config.rs` home. Read the public layout, entry/list declarations,
file/memory/snapshot producers, list construction/duplication/destruction,
lookup/iterator/foreach consumers and field touchers. Queried Wavefront's type,
field-toucher, lifecycle-schema and dependency records; submitted the callback's
ownership and the internal list-entry release role using `--update`.

* `BackendEntry` / `BackendEntryRef` / `BackendEntryMut` use ffibox's compatible
  layout and pointer-wide handles. The embedded public entry is at offset zero;
  its nullable strings are immutable borrows of retained storage, not separately
  owned fields. The shared and exclusive field projections return existing safe
  `EntryRef` / `EntryMut` handles without referencing foreign struct storage.
* Lookup results acquire one reference to backing storage. Multiple results can
  alias the same entry: `BackendEntryArc` therefore makes no sole-owner, mutable,
  cloning or thread-safety claim. Iterator/foreach entries are only borrowed and
  must not release storage. No public standalone C constructor or up-ref exists.
* `BackendEntry::from_entry` safely consumes the already-acquired `EntryArc`
  produced by the existing safe config constructor/lookup. `into_entry` transfers
  it back without copying, changing its address or incrementing its reference.
  An arbitrary borrowed `EntryRef` cannot be promoted to this larger layout.
* `release_policy` exposes an ownership-compatible, stateless callable strategy,
  not an unbound function pointer. `release` consumes an acquired owner and
  recovers that owner's callback, never a callback saved from another allocation.
  `BackendEntryFree` delegates to the existing anchored `EntryFree` lifecycle
  implementation, which calls C's `git_config_entry_free`. The callback cannot
  be replaced safely because it determines the allocation/storage release pair.
* Added only `git_config_backend_entry` to the affected sys type allowlist;
  regenerated bindings and checked/tested the sys crate. No C or shim changed,
  no new standalone macro API or public unsafe API was added. Existing lower
  wrappers exposed no temporary backend-entry raw API requiring replacement.

## Builds and validation

Reused the immutable prepared libraries and `cargo-variant.sh`. Compiled C inputs
match campaign revision `0551dfd4ad989b6a3d5683c0d4cf326c6efef929` (empty diff
for sources, headers, dependencies and CMake inputs). `build.json` remains version
1 and matches setup commit `74e3b602a`. Cached compiler identities/flags match
clang 18.1.3 for plain/coverage/ASan+UBSan/TSan, and the prepared BSan clang 23.1.3
and plugin. Rust outputs and logs are worktree/agent-private; C coverage data was
redirected with `GCOV_PREFIX` into `/tmp/crustify-config-w2b0/`. A wrap with no
compiled C change does not require a C baseline rebuild.

* Workspace `cargo check`, `cargo clippy`, additional all-target clippy, and
  `cargo test` pass. Plain, ASan+UBSan and TSan workspace runs pass: 615 unit,
  389 equivalence, 648 safe-UB, 91 unsafe-UB and 293 doctests. One pre-existing
  unit test is ignored. Final focused sanitizer runs also cover the workset.
* Added two internal tests, one safe-UB integration test, one equivalence
  integration test and six compile-fail doctests. Tests cover both projections,
  reborrows, NULL borrowing seams, size/alignment/offset, address-preserving
  owner transfers, exact callback release counts, callback reentry, retained
  entries across refresh/parent destruction, NULL versus empty/non-UTF-8 values,
  includes, errors and multi-step release sequences on independent C/Rust inputs.
* Compile-fail evidence rejects owner escape, acquired-owner mutation/cloning,
  cross-thread transfer, mutable child/parent aliasing and releasing a borrow.
* Miri passes the Rust-only layout/projection/reborrow test. Foreign acquisition
  and release paths are explicitly excluded from Miri.
* BSan passes two tests individually by exact name under 300-second timeouts:
  `sys_config::backend_entries::backend_entry_retained_storage_and_release`
  (`ub_safe`) and
  `libgit2::config::sys::config::unit_tests::backend_entries::acquired_callback_and_owner_roundtrip`
  (`--lib`). No whole-binary or workspace BSan run was performed.
* The final static safety scan has no unsafe/raw smell or undocumented unsafe
  site in the authored wrapper/tests. Existing unrelated workspace findings
  remain out of scope. `git diff --check` passes.

## Coverage

Ran `cargo-variant.sh coverage test --workspace backend_entr` with the prepared
coverage library, inspected Rust JSON and private C gcov results. Every authored
wrapper method and lifecycle policy executes, including mutable projection in
unit tests; the authored wrapper block has no uncovered executable segment.

C dispatch calls `git_config_entry_free` 23 times (including the five custom
fixture releases), `git_config_list_entry_free` 18 times and list teardown 52
times. The latter two cover every executable block. File and snapshot acquisition
execute as well, including old-entry retention through file refresh.

Uncovered C paths are the NULL `git_config_entry_free` no-op (an owner is always
non-NULL), allocation/lock/refresh failure paths not forced by this type workset,
and memory-backend acquisition pending its higher-layer safe backend wrapper.
NULL origins and maximal scalar values are covered by the Rust-only fixture.
Impossible NULL inline-field/owner conversions and an absent required callback
are seam-invariant violations, not legal safe inputs used to force panic paths.

## Landing revalidation

The initial local push was rejected because batch 1 landed first (`d6a13729c`).
Rebased only this agent branch onto that wave tip and retained both batches' sys
allowlist additions. Repeated the workspace check/clippy/test runs, full ASan+UBSan
and TSan workspace runs, both exact BSan tests, Miri, coverage and the static scan
on the rebased tree; all passed. The counts above describe that combined tree.
