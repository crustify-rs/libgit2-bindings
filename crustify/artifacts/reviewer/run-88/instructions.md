# Annotation pass: `src/libgit2/src/libgit2/repository.rs`

This file's bindings predate the conventions' current Anchors section. Bring every
annotation in it to that section. Your `conventions` objective covers the Anchors section
only, this time: report any other deviation you notice in your final message, without
fixing it.

## Checklist

The C items whose bindings sit in this file, as landed `wrap` batches scheduled them:

| C item | scheduled as | defined in | fields scheduled |
|---|---|---|---|
| `git_repository` | type | `src/libgit2/repository.h` |  |
| `git_repository_init_flag_t` | type | `include/git2/repository.h` |  |
| `git_repository_init_mode_t` | type | `include/git2/repository.h` |  |
| `git_repository_item_t` | type | `include/git2/repository.h` |  |
| `git_repository_open_flag_t` | type | `include/git2/repository.h` |  |
| `git_repository_state_t` | type | `include/git2/repository.h` |  |
| `git_repository_init_options` | type | `include/git2/repository.h` | `flags`, `version`, `description`, `mode`, `oid_type`, `refdb_type`, `initial_head`, `origin_url`, `template_path`, `workdir_path` |
| `git_repository_new_options` | type | `include/git2/sys/repository.h` | `version`, `oid_type` |
| `git_repository__cleanup` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_commit_parents` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_commondir` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_config` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_config_snapshot` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_detach_head` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_discover` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_fetchhead_foreach_cb` | callback | `include/git2/repository.h` |  |
| `git_repository_free` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_get_namespace` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_hashfile` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_head` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_head_detached` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_head_detached_for_worktree` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_head_for_worktree` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_head_unborn` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_ident` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_index` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_init` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_is_bare` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_is_empty` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_is_shallow` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_is_worktree` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_item_path` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_mergehead_foreach_cb` | callback | `include/git2/repository.h` |  |
| `git_repository_message` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_message_remove` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_new` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_odb` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_oid_type` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_open` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_open_bare` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_open_ext` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_open_from_worktree` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_path` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_refdb` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_reinit_filesystem` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_bare` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_config` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_head` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_head_detached` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_head_detached_from_annotated` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_ident` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_index` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_namespace` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_odb` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_refdb` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_set_workdir` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_state` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_state_cleanup` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_submodule_cache_all` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_submodule_cache_clear` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_workdir` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_wrap_odb` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_init_ext` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_init_options_init` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_new_ext` | symbol | `src/libgit2/repository.c` |  |
| `git_repository_new_options_init` | symbol | `src/libgit2/repository.c` |  |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name.

The `/// Handwritten` marker is retired: remove each one in the file, and the empty doc
line it leaves at the head or end of a doc comment.

On this host, five compile-fail snapshots of `tests/ub_safe` fail before any change, because
ffibox is reached through a symlinked path the snapshots do not normalize: they are no
regression of yours.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`, and land one commit, your change
  to `repository.rs`, rather than the reports commit and the fix commit.
- **No sanitizer, Miri or BSan gates, and no new tests**: the change is to doc comments and
  `#[doc(alias)]` attributes only. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p
  libgit2` to your gates: it rejects an alias where rustdoc admits none. Change no code
  beyond the annotations; if an annotation cannot be placed without one, leave it and say
  why in your final message.
- **No dashboard row and no ffimap scan**: this batch has no entry in `schedule.json`.
- **Commit nothing outside `repository.rs`**, `Cargo.lock` included.
