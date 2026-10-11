# git_index_matched_path_cb — wrap validation

## Contract and scope

Verified the symbol route (callback typedef), not the type route. Both C invokers
(`apply_each_file`, `index_apply_to_all` in `src/libgit2/index.c`) borrow a non-null
NUL path and a nullable NUL pattern for the invocation. `git_pathspec__match`
returns a null pattern for an empty pathspec; otherwise the parsed pathspec owns
it. The diff/index owns the path and may release it immediately after the callback.
The caller owns the opaque payload; the synchronous stack-local diff context
merely forwards it. No pointer is retained, freed, cloned, or transferred by the
callback machinery. Findings were submitted through Wavefront.

`MatchedPathCallback` exclusively borrows a higher-ranked Rust closure. Its inputs
are `&CStr` and `Option<&CStr>`; `MatchedPathResult` preserves every C integer code,
including application-specific negative aborts and positive skips. Panic payloads
are saved until C cleanup returns, then resumed in Rust. The handle is neither
Send nor Sync. There is no new public unsafe API or raw-pointer surface.

This is a handwritten callable adapter, not a C allocation owner; ffibox has no
callable primitive and its prescribed borrowed-string representation is `&CStr`.
The crate-private synchronous `scan_with` seam is for the separately scheduled
add/remove/update wrappers. No higher-layer function was independently wrapped.
Only the required callback and its three C test invokers were allowlisted.

## Build reuse

Used the immutable `/target/crustify/builds/{asan-ubsan,tsan,bsan,coverage}` builds
and the existing `cargo-variant.sh` runner. The worktree and `/target` have identical
C/header/dependency/build-system trees; `build.json` is identical (version 1).
CMake caches select the configured clang 18 compiler/instrumentation, or the
configured BSan clang/plugin. No C or shim was changed. Cargo targets, sanitizer
logs, Rust profiles and redirected `GCOV_PREFIX` outputs are worktree-private.
The shared C coverage data was not written. No full C baseline rebuild is needed.

## Tests and gates

- ASan/UBSan: workspace check, clippy, tests (including sys bindings and doctests).
- Clippy also checked all targets, including the new integration/unit tests.
- TSan: the seven focused matched-path tests.
- Miri: three safe public callback tests and the Rust-only internal stop/reset test;
  two actual-C unit tests are explicitly ignored because Miri cannot call libgit2.
- BSan: only the two exact foreign-boundary unit tests, each with a timeout:
  `libgit2::index::index::unit_tests::matched_path::foreign_scan_payload_and_panic_cleanup`
  and `libgit2::index::index::unit_tests::matched_path::workdir_scan_skip_abort_and_removed_paths`.
- Static `crustify scan-unsafe` plus unsafe/raw-smell site enumeration: no new smells
  in this workset. All explicit unsafe blocks pass the enforced safety-comment lint.
- Five compile-fail doctests reject escaping either string, dropping the captured
  closure, overlapping exclusive callback borrows, and sending the handle to a thread.

The public UB tests cover repeated calls, independent nested callbacks, temporary
non-UTF-8 strings, empty/null patterns, copied strings surviving their originals,
integer extremes, abort reuse, and panic payload recovery/reuse. The equivalence
suite compares independently owned C-index scans using a raw reference callback
versus the public Rust callable: return/error values, copied callback arguments,
and remaining index state after each step. Internal tests exercise the actual
production trampoline from C add, update and remove scans, including cleanup before
panic resumption and refusal to reinvoke a stopped callback.

## Coverage

Ran the focused tests against the coverage C build with private gcov output and
Rust LLVM coverage. All 64 executable lines of the authored callback/result region
are covered across unit and integration binaries. C `apply_each_file` reaches 100%
blocks, including unmatched paths, apply, skip, abort, add and workdir deletion.
The remove-scan callback decision branches also cover apply/skip/abort, and
`git_index_remove_all` reaches 100% blocks.

Uncovered surrounding C paths are not callable-handle paths: absent callbacks
(the handle always supplies one), allocation/I/O failures before invocation, and
`index_apply_to_all`'s unused NONE/UPDATE/default actions (the sole public caller
selects REMOVE). The callback's argument/payload construction and result/panic
paths have no uncovered executable lines. Raw logs/profiles are retained under
`crustify/tmp/index-batch-8/`, outside the committed changeset.
