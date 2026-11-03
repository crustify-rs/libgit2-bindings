# Annotation pass: `src/libgit2/src/libgit2/object.rs`

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
| `git_object_t` | type | `include/git2/types.h` |  |
| `git_object` | type | `src/libgit2/object.h` |  |
| `git_object_id_options` | type | `include/git2/object.h` | `version`, `oid_type`, `object_type`, `filters` |
| `git_object_dup` | symbol | `src/libgit2/object.c` |  |
| `git_object_free` | symbol | `src/libgit2/object.c` |  |
| `git_object_id` | symbol | `src/libgit2/object.c` |  |
| `git_object_lookup` | symbol | `src/libgit2/object.c` |  |
| `git_object_lookup_bypath` | symbol | `src/libgit2/object.c` |  |
| `git_object_lookup_prefix` | symbol | `src/libgit2/object.c` |  |
| `git_object_owner` | symbol | `src/libgit2/object.c` |  |
| `git_object_peel` | symbol | `src/libgit2/object.c` |  |
| `git_object_rawcontent_is_valid` | symbol | `src/libgit2/object.c` |  |
| `git_object_short_id` | symbol | `src/libgit2/object.c` |  |
| `git_object_string2type` | symbol | `src/libgit2/object.c` |  |
| `git_object_type` | symbol | `src/libgit2/object.c` |  |
| `git_object_type2string` | symbol | `src/libgit2/object.c` |  |
| `git_object_type_is_valid` | symbol | `src/libgit2/object.c` |  |
| `git_object_id_from_buffer` | symbol | `src/libgit2/object.c` |  |
| `git_object_id_from_file` | symbol | `src/libgit2/object.c` |  |
| `git_object_id_options_init` | symbol | `src/libgit2/object.c` |  |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name. ffibox's policy macros (`impl_cdrop!` and the others) now take outer attributes
ahead of their arguments and put them on the trait method they mint, as `define_ctype!`
does: annotate a lifecycle routine there, not on the private adapter a macro is given.
The `/// Handwritten` marker that ffibox's skill asks for is not an annotation: leave it
as it is.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`. A deviation you fix needs no
  report, and the batch lands one commit, its change to `object.rs`, rather than the
  reports commit and the fix commit the Landing section of the review how-to names.
- **No sanitizer, Miri or BSan gates, and no new tests.** The change is to doc comments
  and `#[doc(alias)]` attributes only, which change no compiled behaviour. Run the
  Regressions gates' check, format, lint and test commands, and
  `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p libgit2`, which rejects an
  alias where rustdoc admits none. Change no code beyond the annotations; if an annotation
  cannot be placed without one, leave it and say why in your final message.
- **No dashboard row and no ffimap scan.** This batch has no entry in `schedule.json`.
- **Do not commit `Cargo.lock`** or any file outside `object.rs`.
