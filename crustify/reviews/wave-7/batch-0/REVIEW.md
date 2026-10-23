# Review: libgit2 / util / wave 1 / batch 0 (`git_error_last`)

Workset: the symbol `git_error_last`, wrapped as the free function `last() -> ErrorBuf`
in `crustify/rust/libgit2/libgit2/util/errors.rs` (anchor `/// Wraps: git_error_last`).
Objective `review`, route `symbol`. Reviewed commit `68ad24171` (wave-1 base); the
wrapper landed in `dd95a582a` and was first reviewed alongside the `git_error` type in
wave-0 batch-0's `REVIEW.md`. This pass re-verifies it against the full instrument
matrix.

| step | outcome |
|---|---|
| analysis / wavefront | verified against `src/util/errors.c` and `src/util/runtime.c`. `git_error_last` reads the calling thread's `last` error under a TLS key `git_libgit2_init` creates and the last `git_libgit2_shutdown` deletes; it returns a `const git_error *` that is either one of four file-static errors or the thread's own state, whose `message` aliases the thread's `git_str` buffer (static `git_str__initstr` when empty) and is rewritten/realloc'd by the next `git_error_set*`/`clear` on that thread. `git_runtime_init` returns `-1` only on a failed first init or a lock failure and `>= 1` otherwise, so `runtime_pinned`'s `count >= 0` success test is correct. No analysis defect. |
| safe boundary | `last()` is a safe `pub fn` with no raw pointer or `unsafe` in its signature; the two `unsafe` blocks (the FFI call and `ErrorRef::from_ptr`) carry correct SAFETY comments resting on the pin. `crustify scan-unsafe --name git_error_last`: 0 raw-pointer sites, 0 undocumented unsafe. No defect. |
| UB | no candidate. All instruments clean on the workset's tests: **Plain** (lib 27, ub_safe 50, equiv 20); **ASan/UBSan + LSan** (same suites — the thread-state allocations a reader makes are freed, no message aliases the realloc'd buffer, no leak); **TSan** (threaded `last` reads/copies across 8 threads race-free); **BSan** (same, no foreign-pointer conflict); **Miri** (Rust-only `from_ptr`/field-read and enum-niche paths). The pre-init TLS hazard is held off by the process-lifetime pin; `last()` synthesises C's "library has not been initialized" when libgit2 cannot be initialised rather than touching a dead key (documented, safe divergence). No patch, no new reproducer. |
| equivalence | no defect. The existing `tests/equiv/errors.rs` runs the same C step sequences on independent fresh threads and compares class, message and `errno` after every step through C's `git_error_last` and through `last()`, including message sizes 0..8192 across realloc, OS-errno formatting, unnamed classes, allocation-failure thread-state and buffer-OOM paths, and the documented pre-init divergence. Verified, no gap. |
| internal | no defect. `mod unit_tests` covers null/empty message, non-UTF-8 lossy display, unnamed-class retention, static-error reads, copy-outlives-next-error, cross-thread copy, thread-state failure, and the pin/no-runtime branches. |
| conventions | none. Naming (`last`, prefix dropped), anchor, free-function placement and `// SAFETY:` discipline conform; `cargo clippy -p libgit2 --all-targets` clean; the file is unchanged by this review. |
| misc | 1 defect, in campaign tooling, not the workset: `crustify/scripts/cargo-variant.sh` instrumented host proc-macros through global `RUSTFLAGS`, so the `asan-ubsan` and `tsan` runners failed to build (`serde_derive`, via the `trybuild` dev-dependency) and the UB instruments were unrunnable. Fixed with per-target `RUSTFLAGS` and a `tsan` `compile_fail` skip mirroring `bsan` — report `misc/asan-tsan-runner-sanitizes-host-proc-macros`. |
| coverage | `errors.rs` 99.76% lines (coverage variant, error suites). The single uncovered line (510) is the child-failure diagnostic inside the `isolated` test helper, reached only when a child test fails — none did. `last()`'s only unreachable arm is `.expect("git_error_last never returns NULL")` (C never returns NULL; llvm-cov attributes the cold panic elsewhere, so it is not even counted as a missed region). C `git_error_last`'s branches — pre-init (`uninitialized_error`), thread-state failure (`tlsdata_error`), no-error (`no_error`) and the live error — are each exercised through the equiv suite's direct C reads and the allocation-failure child. No reachable gap; no test added. |

## Verdict

No defect in the reviewed symbol. The `last()` wrapper is correct against C, UB-free
across Plain/ASan-UBSan/TSan/BSan/Miri, and effectively fully covered. The only fix is
to the shared sanitizer runner, without which the ASan/UBSan and TSan obligations could
not be met.
