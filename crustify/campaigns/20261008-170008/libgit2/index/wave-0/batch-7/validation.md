# git_pathspec_match_list — wrap/type

## Analysis

- Public forward declaration only; no public fields or inline-value constructor.
- `pathspec_match_alloc` initializes list storage, increments the compiled pathspec
  reference count and records the string/diff discriminator. No list refcount,
  public clone, initializer or field-only disposer exists.
- `git_pathspec_match_{index,tree,workdir}` copy matching names into the list pool;
  iterators release source references before returning. Results outlive sources.
- `git_pathspec_match_diff` stores borrowed `git_diff_delta *` without retaining the
  diff; the Rust owner and handles carry the source borrow lifetime.
- `pathspec_build_failure_array` copies each failed pattern into the list pool.
  The comment in `pathspec_match_alloc` claiming failures point into the pathspec
  is stale, but its refcount is nevertheless incremented and must be released.
- `git_pathspec_match_list_free` frees arrays, pool, retained pathspec reference,
  and list; it never frees delta pointers. Allocation and every ABI remain C-owned.
- Bounds and discriminator checks in public getters return NULL rather than read
  invalid entries, including `SIZE_MAX`. C returns negative status on failures,
  cleans any partial list, and nulls the output; early failures leave it untouched.
- The diff wrapper landed at `5be7c5199`; constructors accept its safe `DiffRef`
  and getters return `DiffDeltaRef`, with the source lifetime carried by the list.
- The scheduled pathspec wrapper did not land. The four constructors therefore
  retain a narrow `*mut git_pathspec` seam with an explicit `# Safety` contract:
  live initialized heap pathspec, caller-held counted reference, no concurrent
  mutation, and an original owner that permits shared ownership. C retains an
  independent counted reference. Replace the seam with the shared pathspec owner
  (and typed flags) when that dependency lands. A shared handle from a sole box
  must not escape as an independently retained reference.
- No Send/Sync claims, callbacks or internal-field projections. Public unsafe
  API is limited to those four constructor seams.
  Findings submitted and checked through Wavefront, including lifecycle reverse lookup.

## Build reuse

Reused immutable `/target/crustify/builds/{asan-ubsan,tsan,bsan,coverage}` and the
campaign `crustify/rust/scripts/cargo-variant.sh`. The worktree and build source
checkout have identical C/include/dependency/CMake trees; build.json version 1 is
identical. CMake caches confirm clang 18 and the configured ASan+UBSan, TSan, gcov,
and BSan clang/plugin flags. Only binding allowlists changed, not compiled C/shims.
Build outputs and logs are batch-local; C coverage uses a private GCOV_PREFIX.
A full C baseline is unnecessary for this wrap-only, unchanged-C batch.

## Validation

Logs and instrument outputs are batch-local under
`/tmp/crustify-index-wave0-batch7/` (not shared build directories).

- The affected sys crate was regenerated, built and tested under ASan/UBSan.
- `cargo check --workspace`, `cargo clippy --workspace`, and
  `cargo test --workspace` pass against the matching ASan/UBSan build, including
  leak detection and five compile-fail ownership/lifetime doctests.
- Public boundary evidence: two `ub_safe` tests for borrowed getters, copied
  strings, bounds and shared/exclusive reborrows; five `ub_unsafe` tests covering
  all four constructor contracts, parsed/generated diff storage, empty and
  populated sources, large failure arrays, early errors and source release order.
  No raw FFI round-trip is classified as `ub_unsafe`: adoption is inline `unit`.
- Three equivalence tests compare independently owned C/Rust sources through
  multi-call flag sequences, statuses/error messages, output entries/failures,
  delta content, bounds and release order. Two inline unit tests cover layout,
  Clone of borrowed handles and raw owner transfer/release.
- Miri: the Rust-only layout/null-adoption test passes; foreign calls are excluded.
- BSan: exactly three selected tests pass (never a whole suite): copied index
  paths/retained patterns, parsed diff borrows, and generated diff borrows.
- TSan: the worklist tests pass in parallel. The unrestricted workspace run
  reports an inherited `git_filter_list__load` race at `src/libgit2/filter.c:569`
  outside this workset. The serialized full workspace retry passes; its log is
  `tsan-serial.log`. This does not dismiss the separate parallel filter finding.
- Static safety scan: no undocumented unsafe blocks in this module; the only
  public unsafe/raw sites are the four documented unavailable-pathspec seams.
  The private raw/unsafe `finish` helper adopts the FFI output with the proven
  allocation and delta-lifetime contract. No wrapper-reference/projection smell.
- Optional all-target clippy found and fixed one safety-comment placement in this
  batch's test macro. It also reports an inherited comment placement at
  `tests/equiv/index.rs:1178`; required workspace clippy and this batch's unsafe
  integration target pass. The unrelated iterator test is not modified.

## Coverage

The campaign coverage build ran the worklist tests with a private GCOV_PREFIX.
Rust `index/pathspec.rs`: 118/119 executable lines (99.16%), 190/191 regions
(99.48%), 20/21 functions (95.24%). The only uncovered closure is the defensive
success-with-NULL fallback; valid C constructors return a non-null list on success.

C `src/libgit2/pathspec.c`: 87.66% lines / 96.75% branches executed for the TU;
the match-list region (line 298 onward) covers 221/237 executable lines (93.25%).
All public worklist functions execute. Focused additions cover literal `!` names,
already-used patterns, renamed files' old/new paths and generated-diff pointer
sharing. Remaining list-region lines are allocation-failure cleanup, exceptional
iterator/filesystem failures, and private freeing of NULL. NULL list getters and
NULL source assertions are outside the non-null Rust handle/caller contracts.
The configured pool initializer cannot fail for the constructor's fixed size.
Other uncovered TU code belongs to the independent matches-path API, not this list.

No compiled C or shim changes were made by this batch.
