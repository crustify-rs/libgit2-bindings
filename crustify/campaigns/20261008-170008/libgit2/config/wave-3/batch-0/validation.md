# git_config_iterator — type / wrap

## Contract and implementation

- Public base layout is unchanged; complete enclosing allocations have unique
  ownership and allocation-specific `next`/`free` callbacks. No clone or inline
  initializer exists. `IteratorBox<'c>` uses ffibox's sole-owner policy and lends
  pointer-only shared/exclusive handles. No Send/Sync claim is made.
- C's all-iterator stores an uncounted config pointer. Owners carry its borrow;
  advancement lends a backend entry only through the exclusive reborrow. The
  next advance can free the previous backend iterator/list. Entry callbacks must
  not be called on these unacquired borrows.
- All four published fields have getters; flags have exclusive mutation. Callback
  getters are typed stateless dispatch tokens, never detached function pointers.
  The nullable backend remains a documented higher-layer raw gap; its unsafe
  replacement contract covers both keepalive and subclass compatibility.
- Normal/glob construction, multivar name selection, and release still call C.
  Regex-filtered multivar iteration uses a Rust-owned ABI-compatible enclosing
  payload with the **same system PCRE2 engine and flags** as the configured C
  build. PCRE2 code/match-data allocations use private ffibox release policies.
  Inputs remain arbitrary C-string bytes, including non-UTF-8.
- C's regex multivar path calls `strlen(NULL)` for a valueless entry
  (`config.c:1110`, `regexp.c:112`). The safe filter skips such entries; explicit
  empty values still match. C's failed-regex path also omits freeing its normalized
  name. The wrapper cleans up the unfiltered inner owner on regex failure. Tests
  compare unaffected matching against C and put corrected behavior in inline unit
  tests and the public `ub_safe` suite. Invalid-name precedence is retained.
- The older `ConfigRef::reread_through` raw iterator seam now uses this owner/API.
  Ownership findings were submitted with Wavefront, not hand-edited.

## Build reuse

The source/header/build-input tree hash in this worktree and `/target` matched:
`6baf6d7311ed838ced029eaf1e6dd05eabae2100399ea42e59d5d2750116e977`.
`crustify/build.json` matched the reusable version 1; CMake caches matched the
prepared flags and Clang 18.1.3 (BSan uses its prepared toolchain/compiler).
No compiled C or shim changed. PCRE2 bindings select pkg-config's same shared
`libpcre2-8.so.0` (10.42), also linked by reusable libgit2. No PCRE2 source build
was needed. Reusable builds were not rebuilt or modified: C coverage data was
redirected with an agent-unique `GCOV_PREFIX` under `/tmp`.

## Validation

Passed:

- affected `libgit2-sys` regeneration/check and its layout tests;
- `cargo check --workspace`, `cargo clippy --workspace --all-targets`, and
  `cargo test --workspace`;
- complete workspace tests against ASan/UBSan and TSan C/Rust variants;
- final focused public iterator suites against ASan/UBSan and TSan;
- Miri's Rust-only iterator layout, projection, reborrow, and callback test
  (C-backed destruction/construction tests are explicitly ignored there);
- exact-name BSan tests only:
  `sys_config::iterators::iterator_foreign_refresh_error_and_retry`,
  `sys_config::iterators::iterator_lifecycle_reborrow_and_callbacks`, and
  `sys_config::iterators::iterator_regex_valueless_and_non_utf8`;
- static safety scan: no undocumented unsafe in the changed TU/tests, no new
  public raw-pointer smell or wrapped-body reference. The one new public unsafe
  setter is the unavailable backend-wrapper seam; the private adoption helper
  requires a constructor's ownership proof. Both have explicit contracts.

New evidence: 3 inline unit tests, 4 safe UB tests, 1 unsafe backend-setter test,
1 independent-input multi-step equivalence test, and 8 compile-fail doctests.
Scenarios cover nested/early release, release reentry, borrowed/exclusive views,
null/empty/non-UTF-8 values, regex/name errors, multiple/empty backends, lazy
foreign refresh failures and retries, and parent/entry lifetime exclusions.

## Coverage and remaining C limitations

Focused instrumented execution reached every authored iterator method and filter
function, all four fields, all ownership/handle forms, and the native filtered
allocation/destruction paths. C line coverage: normal and glob constructors and
free callbacks 100%; glob next 100%; multivar next/free 100%; config-list next/free
100%; all-iterator next 87.88%; multivar constructor 91.30%.

Uncovered: PCRE2 match-data OOM, construction OOM, C's nullable-free no-op (owners
are non-null), custom-backend-specific error branches, and C's unreachable final
all-iterator return. Failed C multivar regex cleanup is deliberately not invoked:
it leaks, and the wrapper compiles via PCRE2 with RAII instead. Safe configuration
construction currently exposes file backends, not arbitrary custom backends.

Previously reported upstream C allocation failures remain outside this wrap:
`misc/config-list-dup-corrupts-heap-on-allocation-failure` and
`misc/multivar-iterator-leaks-inner-on-allocation-failure` from config review wave 1.
They affect file-list duplication / the C name-selection constructor. This batch
neither copies those defective implementations into Rust nor deliberately triggers
known C heap corruption/leaks during its sanitizer/coverage runs.

Agent-unique detailed logs, scan, Rust coverage JSON, and C gcov reports are in
`/tmp/crustify-config-wave3-batch0/` in this execution environment.

## Landing integration

Rebased onto concurrent batch 1 (`8a2a2d30c`) using only this agent branch.
The only conflict was adjacent allowlist additions; both sets were retained.
Merged ownership findings remained valid and unique. All workspace gates and
focused instruments were repeated on the integrated tree before landing.
