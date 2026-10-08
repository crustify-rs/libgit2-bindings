# Rejected review findings — libgit2/util

| review batch | report | category | verdict |
|---|---|---|---|
| review-wave-1 / batch-0 | `misc/asan-tsan-runner-sanitizes-host-proc-macros` | misc | rejected: did not reproduce |

## `misc/asan-tsan-runner-sanitizes-host-proc-macros`

The report claims `cargo-variant.sh asan-ubsan test -p libgit2 --test ub_safe` fails
deterministically at the wave-1 base `68ad24171` because the global `RUSTFLAGS` sanitizes
the `serde_derive` proc-macro. Rerun by the orchestrator at `68ad24171`, in the shared
target dir and from empty target dirs: `asan-ubsan` builds and passes 50/50 (the
`compile_fail` trybuild suite included), and `tsan` builds and passes 50/50 with
`compile_fail` running. `serde_derive` is in the graph and builds unsanitized. The fix
(`82d31e146`: per-target RUSTFLAGS, and skipping `compile_fail` under TSan) is reverted;
the skip would have dropped a suite that passes.
