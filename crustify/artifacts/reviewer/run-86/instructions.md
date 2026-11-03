# Annotation pass: `src/libgit2/src/libgit2/commit.rs`

This file's bindings predate the conventions' current Anchors section. Bring every
annotation in it to that section. Your `conventions` objective covers the Anchors section
only, this time: report any other deviation you notice in your final message, without
fixing it.

## Checklist

The C items whose bindings sit in this file, as landed `wrap` batches scheduled them:

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
name.

The `/// Handwritten` marker is retired: remove each one in the file, and the empty doc
line it leaves at the head or end of a doc comment.

On this host, five compile-fail snapshots of `tests/ub_safe` fail before any change, because
ffibox is reached through a symlinked path the snapshots do not normalize: they are no
regression of yours.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`, and land one commit, your change
  to `commit.rs`, rather than the reports commit and the fix commit.
- **No sanitizer, Miri or BSan gates, and no new tests**: the change is to doc comments and
  `#[doc(alias)]` attributes only. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p
  libgit2` to your gates: it rejects an alias where rustdoc admits none. Change no code
  beyond the annotations; if an annotation cannot be placed without one, leave it and say
  why in your final message.
- **No dashboard row and no ffimap scan**: this batch has no entry in `schedule.json`.
- **Commit nothing outside `commit.rs`**, `Cargo.lock` included.
