# Config synchronous enumeration — wave 3 batch 1

## Analysis and safe surface

Verified the homogeneous `symbol` route and requested `config/config.rs` home.
Read public declarations, the three bodies, iterator construction/advancement/
cleanup, file/memory/snapshot backend iterators, entry-list retention and release,
regex implementation, callback-error handling, and the repository/remote/
submodule/diff-driver callers. Queried Wavefront records, callees and callers;
submitted complete pointer findings for exactly the three scheduled symbols.
A semantic ownership-store comparison confirms no other records changed.

* `ConfigRef::foreach` visits all entries in increasing backend priority, retaining
  duplicates. `foreach_match` takes `Option<&CStr>` and filters normalized names
  through C's case-sensitive regex implementation. NULL means unfiltered; an
  empty pattern matches every name; no matches is success.
* `ConfigRef::foreach_multivar` normalizes the NUL key and visits all its values
  across backends. No values is `NotFound`, not an empty successful walk. A
  valueless setting remains `None`, distinct from an empty string or non-UTF-8.
* All methods reuse `ForeachCallback`, `EntryRef` and the existing ffibox-backed
  config handles. Config/repository, callback and input strings stay borrowed
  during the synchronous call; C retains no caller pointer. C owns the iterator
  and entry lists, releasing them before return. Higher-ranked callback entry
  lifetimes prohibit retaining either entries or their strings. Shared handles
  allow independent nested reads but exclude mutation/dropping the active config.
* Results are `Result<ControlFlow<NonZero<c_int>>, ConfigError>`. Callback stop
  state is checked before interpreting C's return value, so negative stops are
  not backend failures and a callback's `GIT_ITEROVER` is not lost when C folds
  it into success. Positive/negative/extreme stop codes are preserved. Panics
  resume on the Rust side only after foreign iterator cleanup; callbacks reset
  their stop state and remain reusable after errors, stops and caught panics.
* Multivar **value-regex filtering is intentionally not exposed**. C's
  `multivar_iter_next` passes a valueless entry's NULL value to
  `git_regexp_match`, which calls `strlen(NULL)`. Its constructor also leaks its
  normalized key when regex compilation fails. The safe method always supplies
  NULL for that parameter; callers can filter the nullable value inside the
  callback. No unsafe regex variant or new raw public seam was added.
* File and snapshot enumeration inherit the already documented C list-copy
  allocation-failure defects. These methods document that limitation rather
  than silently adding a snapshot preflight, which would itself traverse those
  defects and change refresh semantics. The allocation regression safely tests
  only the first iterator allocation failing, before any list is copied.

Exactly one filled anchor was added per scheduled symbol. No new public unsafe
API, representation, lifecycle policy, Send/Sync assertion or shim was needed.
The previous internal foreign-panic fixture now calls the safe enumeration
method instead of reconstructing the raw callback seam. The sys allowlist gains
only the two missing enumeration functions; regeneration/check/tests are included
in workspace validation. C implementations and ABI are unchanged.

## Build reuse and regressions

Reusable C source/header/dependency/CMake inputs match campaign revision
`0551dfd4ad989b6a3d5683c0d4cf326c6efef929` (`git diff` is empty for those inputs).
Build specification version is 1. CMake caches match clang 18.1.3 and each
prepared instrumentation, including the supplied BSan compiler/plugin. Reused
immutable `/target/crustify/builds` and `cargo-variant.sh`; Rust targets are
private to this worktree. Logs/coverage outputs live in
`/tmp/crustify-config-w3b1`; C gcov writes use private `GCOV_PREFIX` trees, with
notes copied there for inspection. No C/shim rebuild or full C baseline is
required for this wrap-only changeset.

Final validations all exit zero:

* Workspace `cargo check`, `cargo clippy` (also all targets), and full plain
  `cargo test`. Clippy emits no warnings.
* Full matching ASan/UBSan and TSan workspace tests: like plain, 620 unit tests
  pass (one pre-existing ignored), 394 equivalence, 653 safe-UB, 91 existing
  unsafe-UB, and 297 doctests (43 merged and 254 others). The sys crate passes.
* BSan runs only these four new safe-API tests, individually by exact name and
  under 420-second timeouts: `enumeration_nested_shared_and_mutable_reborrows`,
  `enumeration_stops_reuse_null_empty_and_non_utf8`,
  `enumeration_panics_cleanup_then_reuse_all_variants`, and
  `enumeration_errors_and_empty_walks_keep_callback_usable`. All pass. No BSan
  whole-binary, suite or workspace gate was run.
* Miri passes the new Rust-only result interpretation test and existing callback
  trampoline test (borrowed entry fixture, stop suppression, NULL rejection,
  panic capture/resumption and reuse). C-dependent tests are ignored under Miri.
* Static safety scan with unsafe/raw smell site enumeration: no workset smell or
  undocumented unsafe. The necessary private trampoline payload reconstruction
  is documented. `git diff --check` passes.

New evidence consists of four public safe-UB tests, three independent-input
C/Rust equivalence tests, two inline unit tests, and three compile-fail doctests.
It covers sole/shared config forms, shared/exclusive-reborrow access, nested
callbacks, duplicate entries and backend order, normalized/subsection names,
empty/valueless/non-UTF-8 values, stop extremes, panic cleanup/reuse, missing and
invalid names, invalid name regexes, empty configs, backend refresh errors and
recovery, initial allocation failure, and multi-call add/write/refresh sequences.
Equivalence compares callback fields/order, returns, stop state, and last-error
class/message after each step; independently owned fixture paths are normalized
only in error messages. No new public unsafe API means no new unsafe-UB suite.

## Coverage

Ran the final workset tests against prepared C coverage and Rust llvm-cov,
inspecting private gcov and Rust JSON regions. C scheduled-function executable
line coverage is `git_config_foreach` 2/2, `git_config_foreach_match` 12/12, and
`git_config_get_multivar_foreach` 16/16; gcov reports 100% executed blocks for
all three. Refresh-error coverage was expanded after inspecting the initial
run to exercise the multivar's no-entry/backend-error branch independently.

Each new Rust method and the result interpreter has 8/8 regions covered;
foreign-call closure regions and the integration callback walk instantiations
are fully covered too. Unrelated functions in the shared config TU are outside
this workset-only coverage workload. Transitive multivar value-regex branches
are intentionally absent from the safe API for the C defects above. Later C
list-copy/constructor allocation failures with known C defects are not
intentionally triggered; the first-allocation failure path is covered safely.
