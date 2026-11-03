# Annotation pass: `src/libgit2/src/libgit2/config.rs`

This file's bindings predate the conventions' current Anchors section. Bring every
annotation in it to that section. Your `conventions` objective covers the Anchors section
only, this time: report any other deviation you notice in your final message, without
fixing it.

## Checklist

The C items whose bindings sit in this file, as landed `wrap` batches scheduled them:

| C item | scheduled as | defined in | fields scheduled |
|---|---|---|---|
| `git_config` | type | `src/libgit2/config.h` |  |
| `git_config_level_t` | type | `include/git2/config.h` |  |
| `git_configmap_t` | type | `include/git2/config.h` |  |
| `git_config_parse_bool` | symbol | `src/libgit2/config.c` |  |
| `git_config_parse_int32` | symbol | `src/libgit2/config.c` |  |
| `git_config_parse_int64` | symbol | `src/libgit2/config.c` |  |
| `git_config_entry` | type | `include/git2/config.h` | `name`, `level`, `value`, `backend_type`, `origin_path`, `include_depth` |
| `git_configmap` | type | `include/git2/config.h` | `type`, `map_value`, `str_match` |
| `git_config_add_file_ondisk` | symbol | `src/libgit2/config.c` |  |
| `git_config_delete_entry` | symbol | `src/libgit2/config.c` |  |
| `git_config_delete_multivar` | symbol | `src/libgit2/config.c` |  |
| `git_config_find_global` | symbol | `src/libgit2/config.c` |  |
| `git_config_find_system` | symbol | `src/libgit2/config.c` |  |
| `git_config_find_xdg` | symbol | `src/libgit2/config.c` |  |
| `git_config_free` | symbol | `src/libgit2/config.c` |  |
| `git_config_get_bool` | symbol | `src/libgit2/config.c` |  |
| `git_config_get_int32` | symbol | `src/libgit2/config.c` |  |
| `git_config_get_int64` | symbol | `src/libgit2/config.c` |  |
| `git_config_get_path` | symbol | `src/libgit2/config.c` |  |
| `git_config_get_string` | symbol | `src/libgit2/config.c` |  |
| `git_config_get_string_buf` | symbol | `src/libgit2/config.c` |  |
| `git_config_lock` | symbol | `src/libgit2/config.c` |  |
| `git_config_new` | symbol | `src/libgit2/config.c` |  |
| `git_config_open_default` | symbol | `src/libgit2/config.c` |  |
| `git_config_open_global` | symbol | `src/libgit2/config.c` |  |
| `git_config_open_level` | symbol | `src/libgit2/config.c` |  |
| `git_config_open_ondisk` | symbol | `src/libgit2/config.c` |  |
| `git_config_parse_path` | symbol | `src/libgit2/config.c` |  |
| `git_config_set_bool` | symbol | `src/libgit2/config.c` |  |
| `git_config_set_int32` | symbol | `src/libgit2/config.c` |  |
| `git_config_set_int64` | symbol | `src/libgit2/config.c` |  |
| `git_config_set_multivar` | symbol | `src/libgit2/config.c` |  |
| `git_config_set_string` | symbol | `src/libgit2/config.c` |  |
| `git_config_set_writeorder` | symbol | `src/libgit2/config.c` |  |
| `git_config_snapshot` | symbol | `src/libgit2/config.c` |  |
| `git_config_backend_entry` | type | `include/git2/sys/config.h` | `free`, `entry` |
| `git_config_entry_free` | symbol | `src/libgit2/config.c` |  |
| `git_config_foreach_cb` | callback | `include/git2/config.h` |  |
| `git_config_get_entry` | symbol | `src/libgit2/config.c` |  |
| `git_config_get_mapped` | symbol | `src/libgit2/config.c` |  |
| `git_config_lookup_map_value` | symbol | `src/libgit2/config.c` |  |
| `git_config_iterator` | type | `include/git2/sys/config.h` | `free`, `flags`, `next`, `backend` |
| `git_config_foreach` | symbol | `src/libgit2/config.c` |  |
| `git_config_foreach_match` | symbol | `src/libgit2/config.c` |  |
| `git_config_get_multivar_foreach` | symbol | `src/libgit2/config.c` |  |
| `git_config_backend` | type | `include/git2/sys/config.h` | `free`, `version`, `iterator`, `cfg`, `unlock`, `open`, `lock`, `snapshot`, `del_multivar`, `del`, `set_multivar`, `set`, `get`, `readonly` |
| `git_config_iterator_free` | symbol | `src/libgit2/config.c` |  |
| `git_config_iterator_glob_new` | symbol | `src/libgit2/config.c` |  |
| `git_config_iterator_new` | symbol | `src/libgit2/config.c` |  |
| `git_config_multivar_iterator_new` | symbol | `src/libgit2/config.c` |  |
| `git_config_next` | symbol | `src/libgit2/config.c` |  |
| `git_config_add_backend` | symbol | `src/libgit2/config.c` |  |
| `git_config_backend_foreach_match` | symbol | `src/libgit2/config.c` |  |
| `git_config_init_backend` | symbol | `src/libgit2/config.c` |  |
| `git_config_backend` | type | `include/git2/sys/config.h` | `free`, `version`, `iterator`, `cfg`, `unlock`, `open`, `lock`, `snapshot`, `del_multivar`, `del`, `set_multivar`, `set`, `get`, `readonly` |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name.

The `/// Handwritten` marker is retired: remove each one in the file, and the empty doc
line it leaves at the head or end of a doc comment.

On this host, five compile-fail snapshots of `tests/ub_safe` fail before any change, because
ffibox is reached through a symlinked path the snapshots do not normalize: they are no
regression of yours.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`, and land one commit, your change
  to `config.rs`, rather than the reports commit and the fix commit.
- **No sanitizer, Miri or BSan gates, and no new tests**: the change is to doc comments and
  `#[doc(alias)]` attributes only. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p
  libgit2` to your gates: it rejects an alias where rustdoc admits none. Change no code
  beyond the annotations; if an annotation cannot be placed without one, leave it and say
  why in your final message.
- **No dashboard row and no ffimap scan**: this batch has no entry in `schedule.json`.
- **Commit nothing outside `config.rs`**, `Cargo.lock` included.
