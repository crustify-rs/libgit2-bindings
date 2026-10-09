# libgit2/core review wave 1: reproducer assessment

Rerun by the orchestrator on 2026-10-09 against the plain build.

| reproducer | pre-fix (`c783be0a6`) | reviewed tip (`4ebccd107`) | verdict |
|---|---|---|---|
| `batch-0/internal/parse-lost-string-reports-parsers-later-error` | panics: refusal carries `(Invalid, "failed to convert string to long: not a number")` | exits 0: `(NoMemory, "Out of memory")` | accepted: wrong error reported for a lost name/email, fixed in `94d67a100` |

Report-only findings (`TraceCallback::from_fn`'s `# Safety` contract, fixed in
`009343ba0` as documentation; two wavefront resubmissions for
`git_signature_default_from_env` and `git_trace_cb`) were accepted as filed.
Rejected: none.
