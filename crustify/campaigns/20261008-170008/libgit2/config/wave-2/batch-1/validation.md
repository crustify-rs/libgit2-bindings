# Config mapping and borrowed callbacks — wave 2 batch 1

## Analysis and surface

Verified the homogeneous `symbol` route (the typedef is a callback, not a type)
and the requested `config/config.rs` home. Inspected declarations, invokers,
lookup/parser dependency closure, file/memory/snapshot entry acquisition, list
retention and release, mapping field access, and existing Rust consumers. Queried
Wavefront records/callees and submitted findings for exactly the five worklist
items. Existing entry findings were clarified with current lifetime vocabulary.

* `git_config_entry_free` and `git_config_get_entry` already have filled anchors
  from wave 1: reuse `EntryFree`, `EntryArc`, and `ConfigRef::entry`, without
  duplicating their lifecycle or constructor wrappers. Release consumes one
  acquired backend reference; iterator/callback entries must never be released.
* `ConfigRef::get_mapped` borrows the configuration, NUL key, and typed mapping
  slice, returning only an owned scalar or error. C retains no table pointer or
  mapping string, and releases its own temporary entry reference. Keys are
  normalized and highest-priority entries win. Like the existing entry wrapper,
  refresh is checked with the multivar iterator and C mapping runs on a snapshot:
  direct C lookup forms an uninitialized backend-entry-derived pointer when
  refresh fails. Regression evidence covers that failure, recovery, and the
  preservation of previous errors when only a lower-priority backend fails.
  Snapshot costs, eager lower-backend refresh differences, and previously
  documented defective C snapshot-copy allocation-failure paths remain inherited
  limitations, explicitly documented on the method. No C implementation changed.
* `lookup_map_value` pins the runtime and takes a typed slice and `Option<&CStr>`.
  NULL/valueless means boolean true, whereas empty text means false. Ordered
  boolean/string matches return `map_value`; integer matches return parsed input.
  Strings compare case-insensitively. Empty tables, parse failures, extremes and
  non-UTF-8 strings are tested. C's speculative parser last-error state on success
  is preserved and compared, not silently cleared.
* `ForeachCallback` lends `EntryRef` through a higher-ranked closure and returns
  `ControlFlow<NonZero<c_int>>`, retaining positive and negative C stop codes.
  All actual invokers lend iterator-owned immutable entries: no ownership forks
  are needed. The Rust-owned payload is borrowed exclusively and synchronously;
  it cannot cross threads or reenter itself. The private trampoline catches
  panics and stops; the Rust caller resumes after C iterator cleanup. Repeated
  calls after a stop cannot overwrite a pending panic. Independent nested
  callbacks are permitted. No layout references to C entries are formed.

No public unsafe API, raw pointer argument/return, or Send/Sync assertion was
added. ffibox's existing entry handles/owner/policy are reused. The callback is
handwritten because it owns a Rust closure/panic state, not C-allocated storage.
The older mapping-table equivalence test now calls the safe mapping API on its
Rust side instead of projecting a raw Rust-side table into C.

Bindings gained only `git_config_get_mapped`, the callback typedef, and
`git_config_foreach` for the internal foreign-invocation regression. The sys
crate was regenerated, built and tested; no shim or compiled C input changed.

## Builds and regressions

C inputs match campaign revision `0551dfd4ad989b6a3d5683c0d4cf326c6efef929`:
`git diff` finds no source/header/dependency/CMake changes. Build version is 1;
CMake caches match clang 18.1.3 and the prepared instrumentation flags (BSan uses
its supplied compiler/plugin). Reused immutable `/target/crustify/builds` and
`cargo-variant.sh`. Rust outputs are private to this worktree and logs are under
`/tmp/crustify-config-w2b1/logs`; C coverage writes use private `GCOV_PREFIX`
directories. This wrap-only batch requires no full C baseline rebuild.

* Workspace check, clippy, and additional all-target clippy pass without warnings.
* Full plain and ASan/UBSan workspace runs pass: 613 unit tests (one existing
  ignored test), 388 equivalence, 647 ub_safe, 91 existing ub_unsafe, and 287
  doctests (43 merged plus 244 others). The sys crate passes too.
* Full TSan workspace tests pass with the same counts as plain/ASan, using the
  matching C library and including concurrent read-only mapping calls on scoped
  threads.
* Miri passes the Rust-only callback seam test: borrowed fixture validity,
  stop-code handling, suppressed repeated invocation, panic capture/resumption,
  reuse, NULL rejection and Debug. Foreign-call tests are marked ignored there.
* BSan runs only six exact tests individually under 300-second timeouts: the
  ub_safe table-borrow, refresh-failure, lower-backend error and parent-drop/nested
  callback tests, plus the internal foreign-iteration panic and empty-snapshot
  OOM tests. All pass. No whole-binary/workspace BSan gate was run.
* Three new internal unit tests, five safe-UB tests, three equivalence tests and
  three compile-fail doctests cover the workset. Independent C/Rust configs,
  mapping tables and entry acquisitions compare multi-call state, values, errors,
  callback observations and cleanup. The empty-snapshot OOM test exercises safe
  construction failure without entering defective C file-entry-copy paths.
* Static safety scan and site enumeration find no workset public/private unsafe
  or raw-pointer smell, and no undocumented unsafe in the changed config files.
  The reported raw dereference outside an impl is the necessary private callback
  payload reconstruction, with its specific exclusive/liveness safety contract.
  `git diff --check` passes. Semantic ownership-store comparison confirms only
  the five scheduled symbol records changed.

## Coverage

Ran config-filtered unit/safe/equivalence suites with Rust llvm-cov and the
prepared C coverage library, inspecting Rust JSON regions and private C gcov.
The shared Rust config TU has 1441/1459 lines and 2726/2751 regions covered
(98.77% and 99.09%, including older wrappers/tests). Every new wrapper/callback
method and ordinary/error branch executes, including failed runtime pinning,
empty-snapshot construction failure, and restoration of absent or present stale
errors. The sole uncovered workset region is restoration of an unknown raw
error class: safe error setters accept only the typed known classes.

C worklist execution is complete by executable lines:
`git_config_entry_free` 6/6, `git_config_get_entry` 2/2,
`git_config_get_mapped` 7/7, and `git_config_lookup_map_value` 20/20.
Impossible discriminator/NULL-string-rule combinations are excluded by
`ConfigMap` construction. Previously documented C snapshot-copy OOM defects
are not deliberately triggered through the new safe API.
