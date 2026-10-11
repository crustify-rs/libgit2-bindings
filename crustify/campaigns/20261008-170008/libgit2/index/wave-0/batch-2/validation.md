# Index wave 0, batch 2 — type wrappers

## Analysis and representation

- Confirmed `type` route, `wrap` objective. No C or shim changes.
- `git_index_conflict_iterator_new` allocates only a cursor and stores the index
  pointer **without** a refcount increment or snapshot. `next` advances its private
  cursor and may sort the index. `free` releases only the cursor, accepting NULL.
  `checkout_conflicts_foreach` keeps the index live for the complete traversal.
- `ConflictIterator<'index>` and its shared/exclusive handles preserve the opaque
  C layout. Its `CBox` owns the cursor; an exclusive index borrow preserves the
  parent's lifetime and excludes mutations. No public fields, clone, shared-index
  constructor, or Send/Sync assertions. The lifetime parameter requires a
  hand-written `CCell`; allocation and destruction still use ffibox policies.
- Extended flags are resource-free, combinable integer bitmasks; the on-disk mask
  is the union of intent-to-add and skip-worktree, not another independent bit.
  `EntryExtendedFlags` retains unknown bits and preserves the enum's ABI.
- Submitted type, constructor, destructor, and traversal ownership records through
  Wavefront. The following symbol wave owns the safe `next` wrapper; its raw binding
  here is used only to observe constructor state in equivalence tests.

## Reusable build verification

Reused `crustify/rust/scripts/cargo-variant.sh` and immutable shared C builds. The
C inputs match `/target` at base `d8c416f228be423a85ce7189b83dfe517a3685ff`:

- `src`: `993eeb771a95b8e99847b80cfa5d1a4920609ce9`
- `include`: `4578a125d4c12bc9beb173470dc90017a7767f7e`
- `deps`: `3b32821a9342ca6673869ec5dca0368724eb4dd2`
- build.json version 1: `007b1966268a4bd775eca571013b6cef15068223`

CMake caches match the configured compilers and instrumentation: clang 18 for
plain, ASan/UBSan, TSan, gcov; the bsan toolchain's clang/plugin for BSan. All logs,
Cargo output, and gcov data are agent-private (`/tmp/crustify-index-w0b2` and this
worktree's target directories); shared libraries were not rebuilt or modified.

## Validation

Using the variant runner:

- `plain check --workspace`: pass.
- `plain clippy --workspace`: pass. The stronger `--all-targets` check passed
  for this batch before the snapshot-iterator sibling landed. After that rebase,
  it reports the sibling's existing `git_index_iterator_new` call in the
  equivalence test (comment precedes `assert_eq!`, not its nested unsafe block);
  no added workset block lacks its safety comment. This unrelated test is unchanged.
- `asan-ubsan test -p libgit2-sys`: pass (bindings regenerated).
- `asan-ubsan test --workspace`: pass, including all eight new compile-fail
  doctests. Focused equivalence tests rerun after the coverage addition: pass.
- TSan focused `conflict_cursor` tests: pass. Initial parallel workspace TSan
  reports an existing glibc timezone race between the config enumeration and
  iterator tests, both through `RepositoryRef::set_head` -> `git_signature_now`
  -> `mktime` -> `tzset_internal`. No index-batch frame is involved. Other test
  modules already document this uninstrumented glibc-lock issue. All serial
  workspace test binaries passed (646 unit, 409 equiv, 676 ub_safe, 92 ub_unsafe;
  one pre-existing unit test ignored). Explicit scoped-thread flag tests still
  create their own threads. The serial run's final rustdoc step encountered stale
  bindings because a wave rebase landed during its execution; regenerated and
  reran doctests separately on the integrated tree: all passed. Final
  workspace ASan/UBSan, check/clippy, focused TSan, exact BSan, Miri, and the
  safety scan also passed after integration.
- BSan, only exact test
  `index::conflict_cursor::retained_parent_and_reborrows`: pass.
- Miri, `conflict_cursor_handle_layout` and
  `extended_flags_preserve_bits_and_combine`: pass. No FFI under Miri.
- Static `crustify scan-unsafe --json --sites ...`: pass; none of the requested
  unsafe/raw/reference smell sites belong to the added wrapper code.

No public unsafe API was added beyond ffibox's lifecycle seam, so there is no new
`ub_unsafe` integration suite. Internal adoption and allocation-failure tests live
beside the wrapper. Safe integration tests cover owners and reborrows, and
compile-fail tests reject parent-first drop, aliasing, shared-index construction,
escaping handles, lifetime variance, and cross-thread ownership.

## Coverage

Coverage runner filters `conflict_cursor` and `extended_flags`, with
`GCOV_PREFIX=/tmp/crustify-index-w0b2/gcov`, followed by inspection of llvm-cov JSON
and a private copy of `index.c.gcno`/`.gcda` with `llvm-cov-18 gcov -b -c`:

- All 13 new Rust production functions executed, including success and OOM paths.
- C conflict iterator constructor: 85% blocks (all executable lines covered).
  The remaining branches are NULL-argument assertions excluded by the safe API.
- C conflict iterator destructor: 100% blocks, including internal NULL no-op test.
- C `next`, used to compare independently constructed cursors: 80% blocks after
  adding a normal entry to cover skipping nonconflicts. Remaining branches are
  invalid NULL arguments and the internal error return, unreachable for the
  well-formed index under the wrapper's exclusive borrow.
- Defensive Rust success-with-NULL adoption failure cannot be produced by this C
  constructor. No invalid-C-contract tests were added to force that state.

Flags are tested exhaustively over all 16-bit entry-field values plus 32-bit
unknown bits. Repeated create/traverse/drop sequences compare missing conflict
sides, termination, and parent state; an allocation-counting unit test verifies
cursor teardown leaves the index intact and frees each cursor exactly once.
