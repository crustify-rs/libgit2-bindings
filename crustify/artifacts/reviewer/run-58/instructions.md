# Annotation pass: `src/libgit2/src/libgit2/oid.rs`

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
| `git_oid` | type | `include/git2/oid.h` | `type`, `id` |
| `git_oid_shorten` | type | `src/libgit2/oid.c` |  |
| `git_oid_t` | type | `include/git2/oid.h` |  |
| `git_oid_cmp` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_cpy` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_equal` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_fmt` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_from_prefix` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_from_raw` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_from_string` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_is_zero` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_ncmp` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_nfmt` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_pathfmt` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_shorten_add` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_shorten_free` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_shorten_new` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_strcmp` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_streq` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_tostr` | symbol | `src/libgit2/oid.c` |  |
| `git_oid_tostr_s` | symbol | `src/libgit2/oid.c` |  |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name. ffibox's policy macros (`impl_cdrop!` and the others) now take outer attributes
ahead of their arguments and put them on the trait method they mint, as `define_ctype!`
does: annotate a lifecycle routine there, not on the private adapter a macro is given.
The `/// Handwritten` marker that ffibox's skill asks for is not an annotation: leave it
as it is.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`. A deviation you fix needs no
  report, and the batch lands one commit, its change to `oid.rs`, rather than the
  reports commit and the fix commit the Landing section of the review how-to names.
- **No sanitizer, Miri or BSan gates, and no new tests.** The change is to doc comments
  and `#[doc(alias)]` attributes only, which change no compiled behaviour. Run the
  Regressions gates' check, format, lint and test commands, and
  `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p libgit2`, which rejects an
  alias where rustdoc admits none. Change no code beyond the annotations; if an annotation
  cannot be placed without one, leave it and say why in your final message.
- **No dashboard row and no ffimap scan.** This batch has no entry in `schedule.json`.
- **Do not commit `Cargo.lock`** or any file outside `oid.rs`.
