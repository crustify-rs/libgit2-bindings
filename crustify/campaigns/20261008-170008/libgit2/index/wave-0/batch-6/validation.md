# Pathspec lifecycle and flags — type / wrap

## Analysis and implementation

Verified the homogeneous type route for `git_pathspec` and
`git_pathspec_flag_t`. Read the public/private headers, complete pathspec TU,
index's inline-storage consumers, revwalk's uncounted retained pointer,
match-list retention/release paths, string-array wrapper and existing consumers.
Wavefront type, field-toucher, dependency and lifecycle queries informed the
analysis; findings for both types and four lifecycle functions were submitted
through `--update` (no direct ownership-store edits).

- The public pathspec is opaque; C exclusively owns layout, allocation and free.
  `new` copies input strings/prefix into its vector/pool, with no input keepalive.
- `PathspecBox` owns the constructor's sole counted reference. `PathspecArc`
  represents a reference shared with C lists. Both use ffibox's stateless
  `PathspecFree` down-ref policy. No public up-ref, deep copy or sole-count query
  exists, so neither owner is cloneable and shared owners have no mutable handle.
- Canonical ffibox shared/exclusive handles carry owner borrows; no references to
  foreign object bodies, public fields, or new Send/Sync assertions were added.
- Private `__init`/`__clear` serve C's inline storage and are deliberately not
  wrapped. The future revwalk wrapper must express its uncounted keepalive.
- `PathspecFlags` is a transparent bitset using generated constants. Every
  combination, including both case flags and unknown bits, is preserved; C masks
  only the known flags. Its Rust-only representation needs no ffibox owner.
- The lower-layer string-array test now constructs its Rust-side pathspecs through
  the safe constructor, retaining raw C construction only as the independent
  equivalence reference.

No C/header/shim changed. Existing constructor/free/flags bindings sufficed. The
three list operations this batch first allowlisted to validate real C retention
were also added by the match-list batch that landed meanwhile, so after the
rebase the allowlist diff is empty. `git_pathspec_matches_path` stays deferred to
the symbol route.

## Rebase onto the wave tip: cut seams replaced

The first landing attempt lost the race to four batches, among them the
match-list wrapper (`a8fb7fc05`) homed in the same `pathspec.rs`. Its four list
constructors (`MatchList::from_index`/`from_tree`/`from_workdir`/`from_diff`)
carried a documented temporary seam awaiting this type: `pub unsafe fn` taking
`*mut ffi::git_pathspec` and raw `u32` flags. With the wrapper available they are
now safe `pub fn`s taking `&PathspecArc` and `PathspecFlags`:

- The list takes its own counted reference, so the argument is the shared owner;
  a sole `PathspecBox` must be converted first (new `compile_fail,E0308` doctest),
  keeping `CBox`'s sole-reference claim true. C only reads the compiled patterns
  and atomically bumps their count, which a shared owner permits.
- The match-list suites drop their raw `git_pathspec_new` fixture
  (`tests/support/pathspec_match_list.rs` now uses `Pathspec::new`) and the
  `ub_safe` raw list fixture. Its `ub_unsafe::pathspec` suite tested only these
  four functions; with no public unsafe API left it moved into
  `ub_safe::pathspec::match_list` under `#![forbid(unsafe_code)]`, superseding
  the two fixture-based duplicates. The `equiv` comparisons are unchanged apart
  from calling the safe constructors (raw C calls still get independent owners).
- Module docs merged; my `counted_c_list_outlives_rust_owner` and the list's
  raw-adoption unit test build lists through the safe constructor.
- Ownership-store records (2 types, 4 lifecycle symbols) were spliced into the
  wave tip's store unchanged (no overlap with the other batches' records).

## Evidence

Three public `ub_safe` integration tests exercise copied short-lived and non-UTF8
inputs, empty/skipped patterns, owned/borrowed arrays, shared/exclusive handles and
reborrows, sole-to-shared conversion, release and independent concurrent owners.
Six compile-fail doctests reject escaping borrows, conflicting reborrows, shared
mutation/cloning and thread transfer; a seventh rejects a sole owner at a
retaining matcher. No new public unsafe API (four were removed) means no
`ub_unsafe` suite.

Two `equiv` integration tests compare all flags and repeated C observations of
independently compiled objects from separately owned input strings across 12 pattern sets, 18 paths, all 64 named-bit
combinations and an unknown high bit. These observe the safe constructor through
C's matching operation, not a duplicate safe matching wrapper.

Inline `unit_tests` cover flags/layout, raw adoption/release seams, real C-list
retention after dropping the Rust owner, initialization failure, and single-fault
injection across 20 constructor allocation positions. Live-allocation assertions
prove partial construction is cleaned up. Objects are built through safe
constructors except the explicitly tested raw adoption round-trip.

## Instruments and build reuse

Used the immutable prepared `/target/crustify/builds` variants and supplied
`cargo-variant.sh` runner. Source/header/dependency/CMake tree hash matches the
prepared source: `6baf6d7311ed838ced029eaf1e6dd05eabae2100399ea42e59d5d2750116e977`
(`git ls-tree -r HEAD src include deps cmake CMakeLists.txt | sha256sum`).
`build.json` is version 1 and matches byte-for-byte; cached flags match Clang
18.1.3 for ASan/UBSan, TSan and coverage, and the prepared Clang 23 BSan toolchain.
No C rebuild or full C baseline was necessary. Logs/output directories are
agent-specific; `GCOV_PREFIX` redirects C counters away from the shared build.

Passed workset gates:

- Workspace `cargo check` and `cargo clippy --all-targets`.
- Full workspace ASan/UBSan and TSan `cargo test --workspace` runs passed
  with `--test-threads=1`, including compile-fail tests and doctests.
- All focused ASan/UBSan and TSan tests, including constructor failure cleanup,
  counted retention and concurrently created/dropped independent owners.
- BSan **only** three exact tests, separately under 600-second timeouts:
  `pathspec::copied_inputs_and_reborrows`,
  `pathspec::empty_owned_and_borrowed_inputs`, and
  `libgit2::index::pathspec::unit_tests::counted_c_list_outlives_rust_owner`.
- Miri: Rust-only flag algebra/layout; foreign calls intentionally excluded.
- `crustify scan-unsafe --json --sites all`: no workset smells or undocumented
  unsafe. Generated ffibox seams remain the intended interop boundary.

Initial parallel workspace runs found unrelated sanitizer diagnostics: an
OpenSSL `libcrypto` leak in existing
`repository::repository_owner::first_constructors_race_to_initialise` (its exact
rerun passed), and TSan's libc `tzset_internal` report between existing config
integration tests calling repository-set-head/signature-now. Neither stack uses
the new pathspec API. These observations are retained in the logs, not suppressed
or fixed outside this worklist. An intermediate serial retry was invalidated by
a concurrent rebuild deleting its running test executable, causing child-spawn
ENOENT; the final full runs use frozen sources without concurrent recompilation
of their target directory.

## Coverage

Campaign coverage build plus Rust llvm coverage reaches every region of
`Pathspec::new` across its library/test instantiations, including runtime-init and
C-construction failure. The TU's 17 measured functions are all executed (169/171
lines including tests; only failed-test assertion branches remain uncovered).
C coverage: `git_pathspec_new`, `__init`, `__clear`, `__vfree`, `pathspec_free` and
prefix construction have 100% line coverage; `git_pathspec_free` has 80% (the
NULL no-op is unreachable from non-null ffibox owners). Both nonlast and final
reference releases are exercised. `__vinit` has 95.83%: its vector-insert failure
is unreachable after reserving capacity for the entire input array. The null
array branch of `is_empty` is not needed by safe array handles; empty arrays are
covered. Internal match out-parameters and unrelated workdir/tree/diff list
operations belong to later symbol batches.

Raw logs, ownership-query evidence, scan JSON, coverage JSON and private gcov
reports: `/tmp/crustify-index-wave0-batch6/`.

## Revalidation after the rebase

On the rebased tree (logs in `/tmp/crustify-index-wave0-batch6/rebase/`):

- `cargo check --workspace --all-targets` passes. `cargo clippy --workspace
  --all-targets` reports one pre-existing error outside this batch:
  `tests/equiv/index.rs:1178` (unsafe block missing a safety comment, from
  `25e623071`); nothing in this changeset.
- Full workspace ASan/UBSan and TSan `cargo test --workspace -- --test-threads=1`
  pass (lib 666, equiv 425, ub_safe 698, ub_unsafe 93, doctests 44/307). The
  ASan binaries were built just before a comment-only inlining of a private
  helper; the focused ASan pathspec/strarray suites were rerun on the final tree
  and pass, and TSan was built from the final tree.
- Plain `cargo test --workspace --no-fail-fast` passes except trybuild's
  `diff_not_sync.rs` (diff batch): stable rustc omits the `PhantomData` source
  snippet that the nightly-blessed `.stderr` contains. It passes under the
  nightly sanitizer runs, and nothing here touches it.
- BSan, exact names only: `pathspec::copied_inputs_and_reborrows`,
  `pathspec::match_list::retained_patterns_and_copied_index_paths_survive_parents`
  and `libgit2::index::pathspec::unit_tests::counted_c_list_outlives_rust_owner`
  all pass.
- Miri: the two Rust-only layout/flag tests pass; foreign calls are ignored.
- Coverage: `pathspec.rs` has 289/292 lines and 493/499 regions covered. The gaps
  are the unreachable NULL-on-success closure in `finish`, the test allocator's
  malloc failure, and the assertion message arms. The C lifecycle functions of
  this worklist keep the coverage reported above.
- `crustify scan-unsafe --sites all`: no undocumented unsafe. The only smells in
  `pathspec.rs` are the match-list batch's private `finish` (unchanged); the
  four public unsafe constructors are gone.

