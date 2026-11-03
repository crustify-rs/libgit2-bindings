# Annotation pass: `src/libgit2/src/libgit2/signature.rs`

This file's bindings predate the conventions' current Anchors section. Bring every
annotation in it to that section. Your `conventions` objective covers the Anchors section
only, this time: report any other deviation you notice in your final message, without
fixing it.

## Checklist

The C items whose bindings sit in this file, as landed `wrap` batches scheduled them:

| C item | scheduled as | defined in | fields scheduled |
|---|---|---|---|
| `git_time` | type | `include/git2/types.h` | `offset`, `sign`, `time` |
| `git_signature` | type | `include/git2/types.h` | `name`, `when`, `email` |
| `git_signature_default` | symbol | `src/libgit2/signature.c` |  |
| `git_signature_default_from_env` | symbol | `src/libgit2/signature.c` |  |
| `git_signature_dup` | symbol | `src/libgit2/signature.c` |  |
| `git_signature_free` | symbol | `src/libgit2/signature.c` |  |
| `git_signature_from_buffer` | symbol | `src/libgit2/signature.c` |  |
| `git_signature_new` | symbol | `src/libgit2/signature.c` |  |
| `git_signature_now` | symbol | `src/libgit2/signature.c` |  |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name.

The `/// Handwritten` marker is retired: remove each one in the file, and the empty doc
line it leaves at the head or end of a doc comment.

On this host, five compile-fail snapshots of `tests/ub_safe` fail before any change, because
ffibox is reached through a symlinked path the snapshots do not normalize: they are no
regression of yours.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`, and land one commit, your change
  to `signature.rs`, rather than the reports commit and the fix commit.
- **No sanitizer, Miri or BSan gates, and no new tests**: the change is to doc comments and
  `#[doc(alias)]` attributes only. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p
  libgit2` to your gates: it rejects an alias where rustdoc admits none. Change no code
  beyond the annotations; if an annotation cannot be placed without one, leave it and say
  why in your final message.
- **No dashboard row and no ffimap scan**: this batch has no entry in `schedule.json`.
- **Commit nothing outside `signature.rs`**, `Cargo.lock` included.
