# Annotation pass: `src/libgit2/src/libgit2/config.rs`

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
name. ffibox's policy macros (`impl_cdrop!` and the others) now take outer attributes
ahead of their arguments and put them on the trait method they mint, as `define_ctype!`
does: annotate a lifecycle routine there, not on the private adapter a macro is given.
The `/// Handwritten` marker that ffibox's skill asks for is not an annotation: leave it
as it is.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`. A deviation you fix needs no
  report, and the batch lands one commit, its change to `config.rs`, rather than the
  reports commit and the fix commit the Landing section of the review how-to names.
- **No sanitizer, Miri or BSan gates, and no new tests.** The change is to doc comments
  and `#[doc(alias)]` attributes only, which change no compiled behaviour. Run the
  Regressions gates' check, format, lint and test commands, and
  `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p libgit2`, which rejects an
  alias where rustdoc admits none. Change no code beyond the annotations; if an annotation
  cannot be placed without one, leave it and say why in your final message.
- **No dashboard row and no ffimap scan.** This batch has no entry in `schedule.json`.
- **Do not commit `Cargo.lock`** or any file outside `config.rs`.
