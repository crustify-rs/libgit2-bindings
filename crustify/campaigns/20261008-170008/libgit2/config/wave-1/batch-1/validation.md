# Config wave 1 batch 1 — symbol / wrap

## Surface and ownership

All 25 projected items are functions (no macro/type-route items). Existing
`Config::new`/`ConfigFree`, `ConfigMut::lock`, and
`BackendMemoryOptions::new` already anchor four lifecycle items; they are reused,
not duplicated. The other 21 symbols are wrapped in their projected home.

Read `config.c`, `config_mem.c`, the public declarations, config file/list/backend
and transaction implementations, repository consumers, and Wavefront records,
three-hop callees, callers and dependency closures. Submitted ownership findings
for every item through Wavefront `--update`.

* Config allocations and retained backend references stay C-owned and are released
  by existing ffibox policies; constructor outputs are adopted only after success.
* Paths, names, regexes and replacement values are borrowed for the call and copied
  when retained by C. No array inputs occur in this worklist.
* File backends retain the repository without an upref. `RepositoryRef<'r>` must
  outlive the config; explicit narrowing of a fresh owner's lifetime permits
  attaching a non-static repository. IncludeIf refresh and compile-fail tests
  exercise that relationship.
* Level/global views retain shared backends and their original parent pointers.
  Shared handles return a shared owner; writable views retain an exclusive parent
  borrow, preventing sibling recursive locks. A HIGHEST view has no enabled writer,
  exactly as C implements it. Tests cover that distinction.
* Snapshot strings are copied to `CString`; buffer/path outputs use the existing
  `BufVal` disposal policy, including failure paths. No string reference survives
  a backend refresh or release.
* Locked-writer mutations have separate `ConfigTransaction` methods without
  exposing its mutable config handle. Reads still see published values until C
  commits; dropping aborts and releases the retained backend.
* File registration rejects negative levels and INT_MAX before C's subtracting
  comparators. The accepted range includes INT_MAX-1, tested against a HIGHEST
  view's internal -1 level. No new Send/Sync assertion or public unsafe/raw API.

## Deliberate divergence

C's int64 parser returns -1 for a suffix tail such as `1kb` without setting an
error. The old safe scalar parser already corrects this defect. The new getter
clears stale errors and gives this otherwise silent failure a fresh Config-class
error (`failed to parse configuration value as an integer`). Its inline regression
seeds an unrelated previous error. Equivalence tests retain C behavior for all
unaffected values, codes, outputs, mutations, readonly failures, and lifecycle
sequences; they do not demand the stale-error defect.

## Validation

No compiled C or shim changes. The shared `/target/crustify/builds/{plain,
asan-ubsan,tsan,bsan,coverage}` builds were reused after checking the unchanged C
revision, identical build.json version 1, compiler paths and instrumentation flags.
Bindgen regenerated only the affected agent-owned function allowlist additions.
Cargo outputs/logs are worktree-private under `crustify/tmp/config-batch-1` and
`crustify/rust/target`; coverage uses GCOV_PREFIX and never writes shared gcda files.

* `cargo check --workspace`, `cargo clippy --workspace`, `cargo test --workspace`.
* Additional clippy gate for the changed equivalence and safe-UB binaries.
* Full workspace ASan/UBSan and TSan runs; selected config tests rerun on the final
  tree. Sanitizer flags are target-specific to avoid the existing shared runner's
  host proc-macro instrumentation problem. Compile-fail tests run in plain Cargo,
  not through the nested sanitized compiler.
* BSan runs only three new foreign-write/retained-pointer scenarios, separately by
  exact name with timeout: `config::file_access::file_api_sequences`,
  `global_views_and_repository_keepalive`, `default_search_and_path_buffers`.
* Miri safe-config integration filter after rebase: 3 passed, 12 ignored (foreign calls); new
  FFI tests are excluded under Miri. Rust-only handle/layout tests and existing
  lifetime doctests remain in the workspace. Two new trybuild cases reject a
  parent lock while a writable view lives and dropping a retained repository.
* Static `crustify scan-unsafe` with site enumeration: no new public unsafe/raw,
  private raw argument, or wrapper-reference smell in the authored surface.
  No `ub_unsafe` suite is added because no public unsafe API is introduced.

## Coverage

Coverage-instrumented config tests execute all 25 selected C symbols. Rust config
TU line coverage is 97.28% (824/847) and region coverage 98.23% (1501/1528); every added public
method/free-function variant is exercised. Full-TU metrics also include preexisting
layout, mapping and error helpers and per-binary unused instantiations.

C line coverage is 100% for free, all scalar getters/setters, string and path
getters/parsing, deletion/multivar mutations, all three find routines, lock, and
memory-options init. Construction/level/default-opening coverage is lower because
of allocation failures and environment-dependent discovery/error paths. Tests add
malformed-input cleanup, circular-symlink stat failure, absent/duplicate/replaced
backends, readonly snapshots, XDG/global presence and fallback, copied non-UTF-8
values, extreme integer/priority values, staged transactions, conditional includes
and independent threaded owners. Remaining allocation-failure branches require
process-global allocator failure injection; system-config success depends on the
host's system search path and is not forced by mutating it. Runtime-init failures
and impossible successful-NULL constructor outputs remain unexecuted.

## Landing validation

Rebased onto the landed entry/mapping batch (`512be0487`) and reran all gates.
Plain workspace totals: 600 unit tests (1 ignored), 378 equivalence tests,
634 safe-UB tests (including trybuild), 91 unsafe-UB tests, 42 sys doctests,
238 safe-crate doctests; no failures. ASan/UBSan and TSan repeat the workspace
with only the compile-fail driver filtered out (633 safe-UB cases). All three
exact-name BSan runs, Miri, coverage and the selected-test clippy gate pass.
Final site enumeration has no config TU public/private unsafe/raw smell sites.
`git diff --check` is clean. One translation changeset is landed by local
fast-forward push; no compiled C baseline is required for this Rust-only wrap.
