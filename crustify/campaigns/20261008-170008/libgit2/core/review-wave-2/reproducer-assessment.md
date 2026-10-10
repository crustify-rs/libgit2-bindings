# libgit2/core review wave 2: reproducer assessment

Assessed by the orchestrator on 2026-10-10.

No runnable reproducers were added. Both findings change no behaviour and were
checked against their evidence:

| finding | check | verdict |
|---|---|---|
| `batch-0/unsafe/set-clear-contract-excludes-calls-not-presence` | `trace::set` / `trace::clear` `# Safety` now excludes threads already inside libgit2; new `ub_unsafe/trace.rs` regression passes in the reviewed-wave gates | accepted |
| `batch-0/conventions/equiv-mark-error-safety-note-split` | seeded scan: undocumented unsafe 23 (wave 2 batch 0) -> 22 (review wave 2 batch 0) | accepted |

Rejected: none.
