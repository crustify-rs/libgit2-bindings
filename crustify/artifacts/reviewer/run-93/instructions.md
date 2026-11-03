# Annotation pass: `src/libgit2/src/libgit2/tag.rs`

This file's bindings predate the conventions' current Anchors section. Bring every
annotation in it to that section. Your `conventions` objective covers the Anchors section
only, this time: report any other deviation you notice in your final message, without
fixing it.

## Checklist

The C items whose bindings sit in this file, as landed `wrap` batches scheduled them:

| C item | scheduled as | defined in | fields scheduled |
|---|---|---|---|
| `git_tag_name_is_valid` | symbol | `src/libgit2/tag.c` |  |
| `git_tag` | type | `src/libgit2/tag.h` |  |
| `git_tag_create_from_buffer` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_create_lightweight` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_delete` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_foreach_cb` | callback | `include/git2/tag.h` |  |
| `git_tag_list` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_list_match` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_annotation_create` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_create` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_foreach` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_message` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_name` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_peel` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_tagger` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_target` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_target_id` | symbol | `src/libgit2/tag.c` |  |
| `git_tag_target_type` | symbol | `src/libgit2/tag.c` |  |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name.

The `/// Handwritten` marker is retired: remove each one in the file, and the empty doc
line it leaves at the head or end of a doc comment.

On this host, five compile-fail snapshots of `tests/ub_safe` fail before any change, because
ffibox is reached through a symlinked path the snapshots do not normalize: they are no
regression of yours.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`, and land one commit, your change
  to `tag.rs`, rather than the reports commit and the fix commit.
- **No sanitizer, Miri or BSan gates, and no new tests**: the change is to doc comments and
  `#[doc(alias)]` attributes only. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p
  libgit2` to your gates: it rejects an alias where rustdoc admits none. Change no code
  beyond the annotations; if an annotation cannot be placed without one, leave it and say
  why in your final message.
- **No dashboard row and no ffimap scan**: this batch has no entry in `schedule.json`.
- **Commit nothing outside `tag.rs`**, `Cargo.lock` included.
