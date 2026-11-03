# Annotation pass: `src/libgit2/src/libgit2/worktree.rs`

This file's bindings predate the conventions' current Anchors section. Bring every
annotation in it to that section. Your `conventions` objective covers the Anchors section
only, this time: report any other deviation you notice in your final message, without
fixing it.

## Checklist

The C items whose bindings sit in this file, as landed `wrap` batches scheduled them:

| C item | scheduled as | defined in | fields scheduled |
|---|---|---|---|
| `git_worktree` | type | `src/libgit2/worktree.h` |  |
| `git_worktree_prune_options` | type | `include/git2/worktree.h` | `flags`, `version` |
| `git_worktree_prune_t` | type | `include/git2/worktree.h` |  |
| `git_worktree_add_options` | type | `include/git2/worktree.h` | `version`, `ref`, `lock`, `checkout_options`, `checkout_existing` |
| `git_worktree_free` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_is_locked` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_is_prunable` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_list` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_lock` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_lookup` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_name` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_open_from_repository` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_path` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_prune` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_prune_options_init` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_unlock` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_validate` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_add` | symbol | `src/libgit2/worktree.c` |  |
| `git_worktree_add_options_init` | symbol | `src/libgit2/worktree.c` |  |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name.

The `/// Handwritten` marker is retired: remove each one in the file, and the empty doc
line it leaves at the head or end of a doc comment.

On this host, five compile-fail snapshots of `tests/ub_safe` fail before any change, because
ffibox is reached through a symlinked path the snapshots do not normalize: they are no
regression of yours.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`, and land one commit, your change
  to `worktree.rs`, rather than the reports commit and the fix commit.
- **No sanitizer, Miri or BSan gates, and no new tests**: the change is to doc comments and
  `#[doc(alias)]` attributes only. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p
  libgit2` to your gates: it rejects an alias where rustdoc admits none. Change no code
  beyond the annotations; if an annotation cannot be placed without one, leave it and say
  why in your final message.
- **No dashboard row and no ffimap scan**: this batch has no entry in `schedule.json`.
- **Commit nothing outside `worktree.rs`**, `Cargo.lock` included.
