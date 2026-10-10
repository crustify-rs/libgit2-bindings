# libgit2/repository review wave 0: reproducer assessment

Assessed by the orchestrator on 2026-10-10.

One runnable reproducer was added; it was rerun on the pre-fix tree (`80f527c2f`, the
review's reports commit) under the `asan-ubsan` variant. The other findings change no
runtime behaviour and were checked against their evidence:

| finding | check | verdict |
|---|---|---|
| `batch-0/unsafe/odb-sync-proof-omits-owner-repository-cache` | rerun: `heap-use-after-free` in `cache_get` from `git_odb_exists_ext` (`odb.c:791`), freed by `RepositoryFree::c_drop`, as in its `asan-trace.log`. The hazard is libgit2's: `odb_cache()` (`odb.c:46`) follows `rc.owner` without a reference. The reproducer stands in for the not-yet-wrapped `wrap_odb` / `OdbRef::exists` through raw FFI, so it shows a wrong `Sync` proof the next symbol batches would inherit, not UB in the landed safe API. The fix (`Odb<'r>` borrowing the owning repository) is pinned by `compile_fail/odb_owner_outlives_repository.rs`, which passes in the reviewed-wave gates | accepted |
| `batch-0/lifecycle/git-index` | `git_index_new_ext` / `git_index_open_ext` are the only SHA-256 index constructors; new `Index::with_oid_type` / `open_with_oid_type` and `tests/equiv/index.rs::oid_types` pass in the reviewed-wave gates | accepted |
| `batch-0/misc/init-flag-mkpath-env-config-is-windows-only` | `are_symlinks_supported` is the only reader of the flag's env-config path and is `GIT_WIN32`-only; doc-only fix | accepted |
| `batch-0/wavefront/init-flag-mkpath-env-config-is-windows-only` | same evidence; wavefront record resubmitted | accepted |

Rejected: none.
