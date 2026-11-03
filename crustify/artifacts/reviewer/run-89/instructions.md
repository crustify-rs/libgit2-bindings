# Annotation pass: `src/libgit2/src/libgit2/index.rs`

This file's bindings predate the conventions' current Anchors section. Bring every
annotation in it to that section. Your `conventions` objective covers the Anchors section
only, this time: report any other deviation you notice in your final message, without
fixing it.

## Checklist

The C items whose bindings sit in this file, as landed `wrap` batches scheduled them:

| C item | scheduled as | defined in | fields scheduled |
|---|---|---|---|
| `git_index` | type | `src/libgit2/index.h` |  |
| `git_index_add_option_t` | type | `include/git2/index.h` |  |
| `git_index_capability_t` | type | `include/git2/index.h` |  |
| `git_index_conflict_iterator` | type | `src/libgit2/index.h` |  |
| `git_index_entry_extended_flag_t` | type | `include/git2/index.h` |  |
| `git_index_entry_flag_t` | type | `include/git2/index.h` |  |
| `git_index_iterator` | type | `src/libgit2/index.h` |  |
| `git_index_name_entry` | type | `include/git2/sys/index.h` | `theirs`, `ours`, `ancestor` |
| `git_index_stage_t` | type | `include/git2/index.h` |  |
| `git_index_time` | type | `include/git2/index.h` | `nanoseconds`, `seconds` |
| `git_index_matched_path_cb` | callback | `include/git2/index.h` |  |
| `git_index_entry` | type | `include/git2/index.h` | `flags`, `id`, `path`, `file_size`, `mode`, `ctime`, `mtime`, `gid`, `uid`, `flags_extended`, `ino`, `dev` |
| `git_index_options` | type | `include/git2/index.h` | `version`, `oid_type` |
| `git_index_reuc_entry` | type | `include/git2/sys/index.h` | `oid`, `path`, `mode` |
| `git_index_add_all` | symbol | `src/libgit2/index.c` |  |
| `git_index_add_bypath` | symbol | `src/libgit2/index.c` |  |
| `git_index_caps` | symbol | `src/libgit2/index.c` |  |
| `git_index_clear` | symbol | `src/libgit2/index.c` |  |
| `git_index_conflict_cleanup` | symbol | `src/libgit2/index.c` |  |
| `git_index_conflict_iterator_free` | symbol | `src/libgit2/index.c` |  |
| `git_index_conflict_iterator_new` | symbol | `src/libgit2/index.c` |  |
| `git_index_conflict_remove` | symbol | `src/libgit2/index.c` |  |
| `git_index_entrycount` | symbol | `src/libgit2/index.c` |  |
| `git_index_extension_add` | symbol | `src/libgit2/index.c` |  |
| `git_index_extension_lookup` | symbol | `src/libgit2/index.c` |  |
| `git_index_extension_remove` | symbol | `src/libgit2/index.c` |  |
| `git_index_find` | symbol | `src/libgit2/index.c` |  |
| `git_index_find_prefix` | symbol | `src/libgit2/index.c` |  |
| `git_index_free` | symbol | `src/libgit2/index.c` |  |
| `git_index_has_conflicts` | symbol | `src/libgit2/index.c` |  |
| `git_index_iterator_free` | symbol | `src/libgit2/index.c` |  |
| `git_index_iterator_new` | symbol | `src/libgit2/index.c` |  |
| `git_index_name_add` | symbol | `src/libgit2/index.c` |  |
| `git_index_name_clear` | symbol | `src/libgit2/index.c` |  |
| `git_index_name_entrycount` | symbol | `src/libgit2/index.c` |  |
| `git_index_name_get_byindex` | symbol | `src/libgit2/index.c` |  |
| `git_index_new` | symbol | `src/libgit2/index.c` |  |
| `git_index_open` | symbol | `src/libgit2/index.c` |  |
| `git_index_owner` | symbol | `src/libgit2/index.c` |  |
| `git_index_path` | symbol | `src/libgit2/index.c` |  |
| `git_index_read` | symbol | `src/libgit2/index.c` |  |
| `git_index_remove` | symbol | `src/libgit2/index.c` |  |
| `git_index_remove_all` | symbol | `src/libgit2/index.c` |  |
| `git_index_remove_bypath` | symbol | `src/libgit2/index.c` |  |
| `git_index_remove_directory` | symbol | `src/libgit2/index.c` |  |
| `git_index_reuc_add` | symbol | `src/libgit2/index.c` |  |
| `git_index_reuc_clear` | symbol | `src/libgit2/index.c` |  |
| `git_index_reuc_entrycount` | symbol | `src/libgit2/index.c` |  |
| `git_index_reuc_find` | symbol | `src/libgit2/index.c` |  |
| `git_index_reuc_remove` | symbol | `src/libgit2/index.c` |  |
| `git_index_set_caps` | symbol | `src/libgit2/index.c` |  |
| `git_index_set_version` | symbol | `src/libgit2/index.c` |  |
| `git_index_update_all` | symbol | `src/libgit2/index.c` |  |
| `git_index_version` | symbol | `src/libgit2/index.c` |  |
| `git_index_write` | symbol | `src/libgit2/index.c` |  |
| `git_index_write_tree` | symbol | `src/libgit2/index.c` |  |
| `git_index_write_tree_to` | symbol | `src/libgit2/index.c` |  |
| `git_index_add` | symbol | `src/libgit2/index.c` |  |
| `git_index_add_from_buffer` | symbol | `src/libgit2/index.c` |  |
| `git_index_conflict_add` | symbol | `src/libgit2/index.c` |  |
| `git_index_conflict_get` | symbol | `src/libgit2/index.c` |  |
| `git_index_conflict_next` | symbol | `src/libgit2/index.c` |  |
| `git_index_entry_is_conflict` | symbol | `src/libgit2/index.c` |  |
| `git_index_entry_stage` | symbol | `src/libgit2/index.c` |  |
| `git_index_get_byindex` | symbol | `src/libgit2/index.c` |  |
| `git_index_get_bypath` | symbol | `src/libgit2/index.c` |  |
| `git_index_iterator_next` | symbol | `src/libgit2/index.c` |  |
| `git_index_new_ext` | symbol | `src/libgit2/index.c` |  |
| `git_index_open_ext` | symbol | `src/libgit2/index.c` |  |
| `git_index_options_init` | symbol | `src/libgit2/index.c` |  |
| `git_index_read_tree` | symbol | `src/libgit2/index.c` |  |
| `git_index_reuc_get_byindex` | symbol | `src/libgit2/index.c` |  |
| `git_index_reuc_get_bypath` | symbol | `src/libgit2/index.c` |  |
| `GIT_INDEX_ENTRY_STAGE` | symbol | `include/git2/index.h` |  |
| `GIT_INDEX_ENTRY_STAGE_SET` | symbol | `include/git2/index.h` |  |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name.

The `/// Handwritten` marker is retired: remove each one in the file, and the empty doc
line it leaves at the head or end of a doc comment.

On this host, five compile-fail snapshots of `tests/ub_safe` fail before any change, because
ffibox is reached through a symlinked path the snapshots do not normalize: they are no
regression of yours.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`, and land one commit, your change
  to `index.rs`, rather than the reports commit and the fix commit.
- **No sanitizer, Miri or BSan gates, and no new tests**: the change is to doc comments and
  `#[doc(alias)]` attributes only. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p
  libgit2` to your gates: it rejects an alias where rustdoc admits none. Change no code
  beyond the annotations; if an annotation cannot be placed without one, leave it and say
  why in your final message.
- **No dashboard row and no ffimap scan**: this batch has no entry in `schedule.json`.
- **Commit nothing outside `index.rs`**, `Cargo.lock` included.
