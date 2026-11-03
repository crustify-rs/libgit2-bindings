# Annotation pass: `src/libgit2/src/libgit2/commit.rs`

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
| `git_commitarray` | type | `include/git2/commit.h` |  |
| `git_commit_header` | type | `include/git2/commit.h` | `value`, `field` |
| `git_commitbuilder` | type | `src/libgit2/commit.c` |  |
| `git_commit` | type | `src/libgit2/commit.h` |  |
| `git_commit_create_ext_options` | type | `include/git2/commit.h` | `payload`, `version`, `sign`, `extra_headers_len`, `extra_headers`, `message_encoding`, `update_ref` |
| `git_commit_create_with_signature` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_extract_signature` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_parent_callback` | callback | `include/git2/sys/commit.h` |  |
| `git_commit_signature_cb` | callback | `include/git2/commit.h` |  |
| `git_commitbuilder_add_header` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create_options` | type | `include/git2/commit.h` | `payload`, `version`, `sign`, `extra_headers_len`, `extra_headers`, `message_encoding`, `committer`, `author`, `allow_empty_commit` |
| `git_commit_amend` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_author` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_author_with_mailmap` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_body` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_committer` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_committer_with_mailmap` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create_buffer` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create_cb` | callback | `include/git2/commit.h` |  |
| `git_commit_create_ext` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create_ext_options_init` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create_from_callback` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create_from_ids` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create_v` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_header_field` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_message` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_message_encoding` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_message_raw` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_nth_gen_ancestor` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_parent` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_parent_id` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_parentcount` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_raw_header` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_summary` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_time` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_time_offset` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_tree` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_tree_id` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_amend_from_stage` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_amend_from_tree` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create_from_stage` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create_from_tree` | symbol | `src/libgit2/commit.c` |  |
| `git_commit_create_options_init` | symbol | `src/libgit2/commit.c` |  |
| `git_commitarray_dispose` | symbol | `src/libgit2/commit.c` |  |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name. ffibox's policy macros (`impl_cdrop!` and the others) now take outer attributes
ahead of their arguments and put them on the trait method they mint, as `define_ctype!`
does: annotate a lifecycle routine there, not on the private adapter a macro is given.
The `/// Handwritten` marker that ffibox's skill asks for is not an annotation: leave it
as it is.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`. A deviation you fix needs no
  report, and the batch lands one commit, its change to `commit.rs`, rather than the
  reports commit and the fix commit the Landing section of the review how-to names.
- **No sanitizer, Miri or BSan gates, and no new tests.** The change is to doc comments
  and `#[doc(alias)]` attributes only, which change no compiled behaviour. Run the
  Regressions gates' check, format, lint and test commands, and
  `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p libgit2`, which rejects an
  alias where rustdoc admits none. Change no code beyond the annotations; if an annotation
  cannot be placed without one, leave it and say why in your final message.
- **No dashboard row and no ffimap scan.** This batch has no entry in `schedule.json`.
- **Do not commit `Cargo.lock`** or any file outside `commit.rs`.
