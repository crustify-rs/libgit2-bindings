# Annotation pass: `src/libgit2/src/libgit2/blob.rs`

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
| `git_blob_filter_flag_t` | type | `include/git2/blob.h` |  |
| `git_blob_data_is_binary` | symbol | `src/libgit2/blob.c` |  |
| `git_blob` | type | `src/libgit2/blob.h` |  |
| `git_blob_filter_options` | type | `include/git2/blob.h` | `flags`, `version`, `reserved`, `attr_commit_id` |
| `git_blob_create_from_buffer` | symbol | `src/libgit2/blob.c` |  |
| `git_blob_create_from_disk` | symbol | `src/libgit2/blob.c` |  |
| `git_blob_create_from_stream` | symbol | `src/libgit2/blob.c` |  |
| `git_blob_create_from_stream_commit` | symbol | `src/libgit2/blob.c` |  |
| `git_blob_create_from_workdir` | symbol | `src/libgit2/blob.c` |  |
| `git_blob_filter` | symbol | `src/libgit2/blob.c` |  |
| `git_blob_filter_options_init` | symbol | `src/libgit2/blob.c` |  |
| `git_blob_is_binary` | symbol | `src/libgit2/blob.c` |  |
| `git_blob_rawcontent` | symbol | `src/libgit2/blob.c` |  |
| `git_blob_rawsize` | symbol | `src/libgit2/blob.c` |  |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name. ffibox's policy macros (`impl_cdrop!` and the others) now take outer attributes
ahead of their arguments and put them on the trait method they mint, as `define_ctype!`
does: annotate a lifecycle routine there, not on the private adapter a macro is given.
The `/// Handwritten` marker that ffibox's skill asks for is not an annotation: leave it
as it is.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`. A deviation you fix needs no
  report, and the batch lands one commit, its change to `blob.rs`, rather than the
  reports commit and the fix commit the Landing section of the review how-to names.
- **No sanitizer, Miri or BSan gates, and no new tests.** The change is to doc comments
  and `#[doc(alias)]` attributes only, which change no compiled behaviour. Run the
  Regressions gates' check, format, lint and test commands, and
  `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p libgit2`, which rejects an
  alias where rustdoc admits none. Change no code beyond the annotations; if an annotation
  cannot be placed without one, leave it and say why in your final message.
- **No dashboard row and no ffimap scan.** This batch has no entry in `schedule.json`.
- **Do not commit `Cargo.lock`** or any file outside `blob.rs`.
