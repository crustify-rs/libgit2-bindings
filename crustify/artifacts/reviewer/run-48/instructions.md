# Annotation pass: `src/libgit2/src/libgit2/repository.rs`

The conventions' Anchors section has changed since this file was bound. Its bindings
still carry the earlier form, `/// Wraps: <name>`, `/// Replaces: <name>` and
`/// Field: <name>.<field>`, and no `#[doc(alias)]`. Bring every annotation in the file
to the current section: the kind-qualified anchor (`fn`, `ty`, `fld`, `var`) in backticks
and the alias beside it, on the item the What is annotated section names. Your
`conventions` objective covers the Anchors section only, this time: report any other
deviation you notice in your final message, without fixing it.

## Checklist

These are the C items whose bindings sit in this file, all bound by landed `wrap` batches.
Each must end up with its annotations, on every stand-in the Several stand-ins section
names; any annotation for a C item not listed (a routine of another module that a
lifecycle strategy calls, for example) follows the same rules. Name a field by its C
name, as the C header spells it, even where bindgen renamed it (`str` for bindgen's
`str_`).

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
name. ffibox's policy macros (`impl_cdrop!` and the others) now take outer attributes
ahead of their arguments and put them on the trait method they mint, as `define_ctype!`
does: annotate a lifecycle routine there, not on the private adapter a macro is given.
The `/// Handwritten` marker that ffibox's skill asks for is not an annotation: leave it
as it is.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`. A deviation you fix needs no
  report, and the batch lands one commit, its change to `repository.rs`, rather than the
  reports commit and the fix commit the Landing section of the review how-to names.
- **No sanitizer, Miri or BSan gates, and no new tests.** The change is to doc comments
  and `#[doc(alias)]` attributes only, which change no compiled behaviour. Run the
  Regressions gates' check, format, lint and test commands, and
  `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p libgit2`, which rejects an
  alias where rustdoc admits none. Change no code beyond the annotations; if an annotation
  cannot be placed without one, leave it and say why in your final message.
- **No dashboard row and no ffimap scan.** This batch has no entry in `schedule.json`.
- **Do not commit `Cargo.lock`** or any file outside `repository.rs`.
