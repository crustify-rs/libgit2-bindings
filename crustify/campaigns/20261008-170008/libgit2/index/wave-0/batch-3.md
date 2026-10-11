# Index wave 0, batch 3 — type / wrap

## Surface and ownership

- `git_index_entry_flag_t`: `EntryFlags`, a resource-free transparent enum-integer
  value. EXTENDED/VALID combine without discarding name, stage, or future bits.
  No macro is published as an independent API.
- `git_index_iterator`: ffibox `IndexIterator` / `IndexIteratorRef` /
  `IndexIteratorMut`, with `IndexIteratorBox` and stateless `IndexIteratorFree`.
  The public API keeps the C type opaque; private `index`, `snap`, and `cur` fields
  are not exposed. There is no C clone or in-place initializer.
- `IndexIterator::new(&IndexArc)` calls `git_index_iterator_new`. The C snapshot
  sorts and copies the pointer vector, increments index rc/readers, and owns a
  counted reference. Entry deletion is deferred while readers exist. Its drop
  releases the vector, decrements readers, releases the index, then frees the
  iterator. Failure balances partially acquired resources and leaves output alone.
- Requiring `IndexArc` prevents retaining a reference while an `IndexBox` still
  claims sole ownership. The caller's index may be dropped before the iterator.
  No runtime keepalive field, custom Drop, Send/Sync implementation, or public
  unsafe function is needed. Macro-generated ffibox escape hatches remain seams.
- `git_index_iterator_next` is bound only to observe constructed C state in
  equivalence tests; its safe wrapper and dependent entry representation belong to
  their scheduled worksets. No lower-layer iterator raw consumer needed replacement.

Evidence: `include/git2/index.h`, `src/libgit2/index.h`, `src/libgit2/index.c`
(new/next/free, snapshot new/release, remove/clear/free), `src/util/vector.c`
(dup/dispose), and `tests/libgit2/index/tests{,256}.c`. Wavefront API dependency
closure is opaque; inspected implementation dependencies separately. Ownership and
constructor/destructor findings submitted using `wavefront --update`.

## Validation

Reusable builds match C revision `0551dfd4ad989b6a3d5683c0d4cf326c6efef929`,
`build.json` version 1, Clang 18.1.3 (BSan Clang 23.1.3), and prepared flags.
No C or compiled-shim source changed. Used the orchestrator's
`/target/crustify/rust/scripts/cargo-variant.sh`, explicit worktree manifest, and
agent-private target/log directories under `/tmp/crustify-index-b3/`.

- Regenerated and tested `libgit2-sys` against ASan/UBSan.
- Workspace `cargo check`, `cargo clippy`, and `cargo test`: pass using ASan/UBSan.
  Existing manifest/style warnings are outside this changeset.
- ASan/UBSan and TSan: new unit, UB-safe, and equivalence tests pass. Scenarios
  include empty/populated snapshots, independent indexes, multiple snapshots,
  both parent/child drop orders, shared/mutable reborrows, replacement after
  snapshot creation, repeated end-of-iteration, and both allocation failure sites.
- BSan: only exact `ub_safe` test
  `index::iterator_snapshot_owners_and_reborrows`, under a timeout: pass.
- Miri: exact Rust-only unit test
  `libgit2::index::index::unit_tests::entry_flags_value_operations_and_layout`:
  pass. Foreign iterator operations cannot run under Miri.
- Four compile-fail doctests reject an escaping iterator borrow, aliasing a
  mutable reborrow, moving an iterator across threads, and constructing from a
  sole-owner index borrow. No new unsafe public API, hence no `ub_unsafe` suite.
- `crustify scan-unsafe` with enumerated sites: no new unsafe/raw smell in the
  authored surface; no undocumented unsafe block in the additions.

## Broader TSan observation after integration

An extra parallel `index::` TSan run after rebasing onto `6c09dba0d` reported
an out-of-workset C filter-initialization race: `filter.c:569` reads
`fdef->initialized` while `filter.c:344` writes it under the registry read lock.
The two stacks are the inherited `matched_path::foreign_scan_payload_and_panic_cleanup`
and `matched_path::workdir_scan_skip_abort_and_removed_paths` unit tests; neither
access belongs to the flags or iterator API. No unrelated C/test fix is included.
The complete batch-3 workset passes its focused TSan rerun. The broader diagnostic
is retained at `/tmp/crustify-index-b3/rebase2/tsan.log` for campaign review.

## Coverage

Ran index tests against the coverage C library and Rust LLVM instrumentation.
`GCOV_PREFIX` redirected C counters into agent-private directories, leaving the
reusable build immutable. Inspected `llvm-cov gcov` and Rust JSON reports.

- All nine authored flag/constructor functions execute; every reported region in
  their combined unit/integration instances is covered, including constructor
  success and error.
- C constructor: both calloc failure and snapshot-vector allocation failure,
  plus successful empty/nonempty creation. Snapshot new/release exercise all
  executable lines and both snapshot failure/success paths.
- C next: successful entries and repeated exhaustion; C free: normal teardown.
- Uncovered paths are argument-assertion failures (safe non-null handles/output
  slots exclude them) and `git_index_iterator_free(NULL)` (ffibox owners are
  non-null). The Rust success-with-null assertion is unreachable under the
  inspected C constructor contract. No environment-dependent path was needed.
