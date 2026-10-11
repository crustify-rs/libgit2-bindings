# index wave 0 / batch 0 — diff types (wrap)

## Surface and ownership

- Verified the homogeneous **type** route. `git_diff` remains opaque and C-owned;
  only public lifecycle operations are exposed. All eight public constructors are
  bound: six generators with safe default-option APIs, and both buffer parsers.
- `Diff<'repo>`, `DiffRef`, `DiffMut`, `DiffBox`, and non-clonable `DiffArc` use
  ffibox. Generated results keep an uncounted repository alive by lifetime; tree
  and index input snapshots need only survive the constructor. No public up-ref,
  clone, init-in-place, or sole-reference query exists. Internal addref is not
  exported as an API. Neither owners nor handles assert Send/Sync.
- Deltas have no independent public allocation/release/copy operation. Their
  scalar fields and embedded files are read-only output. Minimal `DiffFile`
  projections are included as the necessary by-value dependency, using existing
  `OidRef`; nullable path strings borrow the delta's parent. No Rust reference
  covers a C struct.
- Parsed patches copy paths, hunk lines and binary data. Their retained parse
  context **does not copy the original buffer**, but its input cursors are never
  dereferenced after parsing: subsequent operations use the copies and teardown
  only clears/releases the context. The parsed owner can outlive the input.
- One documented public unsafe cut remains: `generate_with_options`, pending the
  options/callback wrappers. Typed repository/tree/index inputs are used; nested
  options, payload lifetime, reentry and unwinding obligations are explicit.
  `DiffError` preserves even positive progress-callback cancellation codes.
- Replaced the commit subsystem's temporary raw diff construction/count/free path.
  Wavefront type, pointer, and lifecycle findings were submitted with `--update`.

## Build reuse and validation

No C or shim implementation changed. Compared the worktree's C/header/dependency
and build.json inputs with `/target`; no differences. Reused the immutable
orchestrator libraries (build.json version 1): Clang 18.1.3 for ASan/UBSan,
TSan and gcov; the configured BSan compiler/plugin for BSan. All output and
coverage writes are agent-private under `crustify/tmp/index-wave0-batch0`;
`GCOV_PREFIX` keeps the shared build immutable. Bindgen regenerated the minimal
allowlist extension and the sys crate was checked/tested with the workspace.

Commands use `crustify/rust/scripts/cargo-variant.sh`:

- `asan-ubsan check --workspace`: passed.
- `asan-ubsan clippy --workspace`: passed (existing manifest warnings). The
  stronger `--all-targets` check passed before rebase; after integrating batch 3
  it flags that batch's unmodified `tests/equiv/index.rs:1178` for a safety comment
  outside, rather than immediately inside, a multiline assertion.
- `asan-ubsan test --workspace`: passed, including sys, doctests and compile-fail.
- `tsan test --workspace diff::`: passed, including independent thread-local diffs.
- Full `tsan test --workspace`: observed an unrelated report in glibc
  `tzset_internal` / `signature.c:172`, between existing config tests
  `iterator_entries::public_entries_interleaved_views_snapshots_and_reparse` and
  `enumeration::enumeration_repository_level_and_snapshot_configs`, both calling
  `RepositoryRef::set_head`. No diff code is in either stack. The unfiltered run
  and a serial workspace replay are retained in the private logs; this batch does
  not modify the unrelated signature/config paths or suppress that report.
  After rebase onto `12159d066`, a full run with `TZ=UTC` instead reported the
  newly integrated raw index callback fixture's filter initialization race:
  `index.rs`'s `foreign_scan_payload_and_panic_cleanup` calls `git_index_add_all`
  while `blob.rs`'s `absolute_filter_paths_are_refused_before_c` initializes built-in
  filters; the conflicting C read is `git_filter_lookup` (`filter.c:360`). Neither
  stack contains this workset. Scoped diff tests remain clean. The first serial
  replay was invalidated when another build replaced its subprocess executable;
  a post-rebase serial replay without concurrent builds of that variant passed
  the entire workspace (`tsan test --workspace -- --test-threads=1`).
- BSan, separately, under 420-second timeouts, **only** exact tests
  `ub_safe::diff::parsed_copy_survives_input_and_owner_reborrows` and
  `ub_unsafe::diff::generate_with_options_contract`: passed.
- `cargo +nightly miri test -p libgit2 --lib
  diff::diff::unit_tests::delta_and_file_projection_layout`: passed; no foreign calls.
- `crustify scan-unsafe <workdir> --json --sites all`: passed. Remaining workset
  smells are the documented raw-options cut and private constructor-result
  adoption helper; their raw ownership obligations cannot be asserted safely by
  a caller lacking the C constructor result. No field-reference or public raw
  pointer smell was introduced.

There are three inline unit tests, three equivalence tests, four safe UB tests,
one suite for the public unsafe method, four trybuild cases and three compile-fail
owner doctests. Fixtures use safe constructors except direct independently owned
C reference-side objects in equivalence tests. Tests cover both object-id types,
empty/invalid/concatenated/trailing-text patches, drop and reborrow ordering,
all generator kinds, callback skip/cancel codes (positive and negative), bounds,
and copied field/path contents after input destruction.

## Coverage

`coverage test -p libgit2 diff:: --json --output-path <private>/coverage.json`,
with C gcov data inspected via `llvm-cov-18 gcov -b -c -f`:

- Rust `diff/diff.rs`: **60/60 functions, 333/335 lines, 576/578 regions**.
  The two missed lines are runtime-initialization failure exits; the runtime is
  pinned in these processes (global initialization failure has its own existing
  subsystem tests).
- C `git_diff_free`, `git_diff_num_deltas`, `git_diff_get_delta`, both buffer
  constructors, and both generated/parsed destructors: **100% line coverage**.
- Six C generators: respectively **96.15%, 90%, 86.21%, 95.83%, 80%, 92.31%**
  for tree/tree, tree/index, index/workdir, tree/workdir, tree/workdir-with-index,
  index/index. Generated allocator: **88.46%**; parsed allocator: **79.17%**.
- Residual C paths include allocation and repository/index I/O failures,
  case-insensitive index reordering, and index-write behavior enabled only by
  nondefault options. No allocator fault injection or nondeterministic filesystem
  failure was added. Private diff algorithms and other public symbols outside
  this lifecycle/field workset are not claimed as covered.
