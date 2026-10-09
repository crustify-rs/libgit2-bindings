# libgit2/core review wave 0: reproducer assessment

Rerun by the orchestrator on 2026-10-09 against the asan-ubsan build.

| reproducer | pre-fix (`3abe8f912`) | reviewed tip (`77b4eba62`) | verdict |
|---|---|---|---|
| `batch-1/ub_unsafe/failed-sysdir-set-leaves-stale-size-on-oom-sentinel` | aborts: `global-buffer-overflow` on `git_str__oom` in `git_str_put` | exits 0, the level reads `""` | accepted: real over-read, fixed in `a5bf88975` |
| `batch-1/misc/extensions-partial-copy-leaks-in-c` | — | LSan: 82 bytes leaked in 3 allocations | accepted: C defect in `git_repository__extensions`, documented only, nothing to revert |
| `batch-0/misc/bsan-gate-filemode-probe-stall` | wave 0 BSan gate timed out (2400 s) | reviewed-wave BSan gate passes (535 tests) | accepted: unstrided 65k-step probe, fixed in `516af668c` |

Report-only findings (Time docs, `Buf::from_raw` contract, two wavefront resubmissions,
global.rs shutdown docs) change documentation and the ownership store only and were
accepted as filed. Rejected: none.
