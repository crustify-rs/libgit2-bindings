# Annotation pass: `src/libgit2/src/libgit2/oid.rs`

This file's bindings predate the conventions' current Anchors section. Bring every
annotation in it to that section. Your `conventions` objective covers the Anchors section
only, this time: report any other deviation you notice in your final message, without
fixing it.

## Checklist

The C items whose bindings sit in this file, as landed `wrap` batches scheduled them:

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
name.

The `/// Handwritten` marker is retired: remove each one in the file, and the empty doc
line it leaves at the head or end of a doc comment.

On this host, five compile-fail snapshots of `tests/ub_safe` fail before any change, because
ffibox is reached through a symlinked path the snapshots do not normalize: they are no
regression of yours.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`, and land one commit, your change
  to `oid.rs`, rather than the reports commit and the fix commit.
- **No sanitizer, Miri or BSan gates, and no new tests**: the change is to doc comments and
  `#[doc(alias)]` attributes only. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p
  libgit2` to your gates: it rejects an alias where rustdoc admits none. Change no code
  beyond the annotations; if an annotation cannot be placed without one, leave it and say
  why in your final message.
- **No dashboard row and no ffimap scan**: this batch has no entry in `schedule.json`.
- **Commit nothing outside `oid.rs`**, `Cargo.lock` included.
