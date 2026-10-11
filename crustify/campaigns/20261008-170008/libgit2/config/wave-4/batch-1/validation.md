# Configuration iterators — symbol / wrap

## Ownership and implementation

Verified the homogeneous symbol route and the five declarations/definitions,
callback implementations, callers, backend-list transitions and Rust consumers.
Wavefront dependency/caller queries and the source inspection establish:

- Normal and glob iterators uniquely own complete callback-specific allocations,
  but retain an **uncounted config borrow**. Config, backend-list and repository
  keepalives extend through advancement and release. Name and regex inputs are
  copied/compiled during construction, never retained as Rust borrows.
- Advancing exclusively mutates iterator state and can free the previous backend
  iterator. Success lends an immutable entry; neither its public nor backend form
  acquires a release reference. Error/end statuses do not initialize the output.
  A backend error permits retry, including progression to another backend.
- Release dispatches the allocation's own callback, freeing nested iterators,
  entry lists, copied names, compiled regexes and the enclosing allocation once.
  A null input is a no-op; no cloning primitive exists.

The earlier type batch already supplied ffibox's `IteratorBox` and
`IteratorFree` policy. The scheduled release symbol is therefore not duplicated.
Constructor methods and their existing unique anchors are homed in the supplied
`config/config.rs`, with no API/behavior change. The type module retains its
allocation-specific inline callback helper, `advance`, lending `BackendEntryRef`.
The public-symbol wrapper is now `IteratorMut::advance_entry`, calling
`ffi::git_config_next` and lending only `EntryRef`; its anchor replaces the one
that previously annotated the inline callback helper. Both methods freeze the
same exclusive handle while the entry lives. No new public unsafe/raw API or
Send/Sync assertion was added. Existing consumers remain source-compatible.

Submitted the missing `git_config_next` ownership findings using Wavefront's
`--update`; all five records now have no pending pointer slots. Existing
constructor/destructor findings were checked against the implementation.

The existing PCRE2 value-filter workaround is retained: filtered multivar
iteration skips valueless entries (C calls `strlen(NULL)`), distinguishes empty
values, and releases the name-selection owner on invalid-regex failure (C leaks
its copied name). No confirmed C defect was copied into Rust. Known upstream
allocation-failure defects described in config wave 3's validation remain
outside this wrap. No C, compiled shim, header or allowlist changed.

## Tests and validation

New evidence: three public `ub_safe` tests, an independent-allocation multi-step
`equiv` test, one inline fixture/lifecycle `unit` test, and two compile-fail
doctests. Coverage includes public/backend alternation, copied shared entries,
null/empty/non-UTF-8 values, all constructor variants, destruction before and
during iteration, short-lived key/regex inputs, refresh error/retry, repeated
exhaustion, and independently owned per-thread iterators. Equivalence compares
all six public entry fields and errors after every step. The inline fixture
confirms allocation-specific callbacks and exactly one release, without acquiring
the lent entry. No new public unsafe method means no new `ub_unsafe` suite.

Passed:

- `cargo check --workspace`, `cargo clippy --workspace --all-targets`, and
  `cargo test --workspace`;
- workspace ASan/UBSan and TSan regressions, and final focused iterator tests;
- BSan **only** these exact new safe tests, individually, under timeout:
  `config::iterator_entries::public_entries_reborrow_and_release` and
  `config::iterator_entries::public_entries_refresh_error_and_retry`;
- Miri's existing Rust-only iterator layout/projection/reborrow/callback test.
  The new method only wraps FFI, so C-dispatch tests are ignored under Miri;
  compile-fail doctests enforce its Rust lifetime boundary;
- static `crustify scan-unsafe` including relevant smell sites. Changed TUs/tests
  have no undocumented unsafe, new public pointer/unsafe smells or wrapped-body
  references. The previously documented backend setter seam is unchanged.

## Reusable builds and coverage

Reusable `/target/crustify/builds` libraries match this worktree's C/source,
header, dependency and CMake input tree hash:
`4e9e8033793f78e5e77808972656654b0b6db96b31bec28f52320aa1db6cdfcf`.
`crustify/build.json` matches version 1. CMake caches match each prepared
instrument, Clang 18.1.3 (the dedicated compiler/plugin for BSan), and PCRE2.
No shared build was rebuilt or modified; `GCOV_PREFIX` redirects writes into
agent-unique `/tmp` directories. No C change requires a full C baseline.

Final coverage runs include the new and existing focused iterator suites:

- Rust `advance_entry`: 20/20 executable regions, 93 calls; constructor methods
  and the existing release policy: all executable regions reached (combining
  library/test instantiations).
- C selected symbols: normal/glob/multivar constructors and `git_config_next`
  100% executable lines; `git_config_iterator_free` 80% (only the null no-op
  remains, unreachable from non-null ffibox owners).
- C glob/multivar next/free callbacks: 100%; `all_iter_free`: 100%;
  `all_iter_next`: 87.88%. Uncovered branches require unavailable custom-backend
  construction, or are its unreachable trailing return.
- Existing PCRE2 filter construction/next/free paths are reached. Match-data OOM
  remains uncovered because the system PCRE2 allocator has no campaign fault
  injection seam. Macro-generated private PCRE2 borrow helpers are unused.

Detailed logs, Wavefront query evidence, Rust coverage JSON and C gcov output:
`/tmp/crustify-config-wave4-batch1/`. The committed static scan accompanies this
summary; instrument runs use the matching sanitized C/Rust variants.
