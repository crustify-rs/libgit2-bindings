# libgit2/repository review wave 1: reproducer assessment

Assessed by the orchestrator on 2026-10-10.

One runnable reproducer was added; it was rerun on the pre-fix tree (`2b7cf31f6`, batch
1's reports commit) under the `asan-ubsan` variant. The other findings change no
runtime behaviour and were checked against their evidence:

| finding | check | verdict |
|---|---|---|
| `batch-1/ub_safe/repository-config-keeps-freed-repository-for-includeif` | rerun (`#![forbid(unsafe_code)]`, safe API only): `heap-use-after-free` in `git_repository_path` <- `git_repository__item_path` <- `do_match_gitdir` (`config_file.c:630`) <- `parse_conditional_include`, freed by `RepositoryFree::c_drop`, as in its `asan-trace.log`. Real: the config file backend keeps an uncounted `repo` (`config_file_open`, `b->repo = repo`) that `git_repository_free` never clears. Fixed by `Config<'r>` (`RepositoryRef::config -> ConfigArc<'a>`, `set_config(&ConfigArc<'static>)`), pinned by `compile_fail/config_arc_outlives_repository.rs` and `ub_safe/config.rs`, passing in the reviewed-wave gates | accepted |
| `batch-1/wavefront/repository-config-outlives-repository-claim` | same evidence; the `git_repository_config`, `git_repository_set_config` and `git_config` records resubmitted | accepted |
| `batch-1/misc/commit-parents-commits-keep-the-repository` | `git_commit_tree` / `git_commit_parent` read the uncounted `commit->object.repo`; doc-only fix (no safe path reaches a commit pointer yet: `commit_ptrs` is a raw seam) | accepted |
| `batch-1/conventions/safety-comments-split-from-their-unsafe-blocks` | unseeded scans: undocumented unsafe 35 (wave 1) -> 29 (review wave 1); `tests/equiv/repository.rs` 5 -> 0, `libgit2/repository/repository.rs` 1 -> 0 | accepted |
| `batch-0/misc/init-options-workdir-path-doc-misstates-the-default` | `repo_init_directories` (`repository.c:2749`) appends `.git` to a non-bare path without a workdir; the equivalence sequence's first step (no workdir) succeeds on both sides; doc-only fix | accepted |

Rejected: none.
