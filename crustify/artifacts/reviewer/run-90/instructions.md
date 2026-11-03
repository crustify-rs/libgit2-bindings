# Annotation pass: `src/libgit2/src/libgit2/tree.rs`

This file's bindings predate the conventions' current Anchors section. Bring every
annotation in it to that section. Your `conventions` objective covers the Anchors section
only, this time: report any other deviation you notice in your final message, without
fixing it.

## Checklist

The C items whose bindings sit in this file, as landed `wrap` batches scheduled them:

| C item | scheduled as | defined in | fields scheduled |
|---|---|---|---|
| `git_filemode_t` | type | `include/git2/types.h` |  |
| `git_tree_entry` | type | `src/libgit2/tree.h` |  |
| `git_tree_update_t` | type | `include/git2/tree.h` |  |
| `git_treebuilder` | type | `src/libgit2/tree.h` |  |
| `git_treewalk_mode` | type | `include/git2/tree.h` |  |
| `git_tree` | type | `src/libgit2/tree.h` |  |
| `git_tree_update` | type | `include/git2/tree.h` | `action`, `id`, `path`, `filemode` |
| `git_tree_entry_cmp` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_dup` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_filemode` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_filemode_raw` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_free` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_id` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_name` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_to_object` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_type` | symbol | `src/libgit2/tree.c` |  |
| `git_treebuilder_clear` | symbol | `src/libgit2/tree.c` |  |
| `git_treebuilder_entrycount` | symbol | `src/libgit2/tree.c` |  |
| `git_treebuilder_filter_cb` | callback | `include/git2/tree.h` |  |
| `git_treebuilder_free` | symbol | `src/libgit2/tree.c` |  |
| `git_treebuilder_get` | symbol | `src/libgit2/tree.c` |  |
| `git_treebuilder_insert` | symbol | `src/libgit2/tree.c` |  |
| `git_treebuilder_remove` | symbol | `src/libgit2/tree.c` |  |
| `git_treebuilder_write` | symbol | `src/libgit2/tree.c` |  |
| `git_treewalk_cb` | callback | `include/git2/tree.h` |  |
| `git_tree_create_updated` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_byid` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_byindex` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_byname` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entry_bypath` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_entrycount` | symbol | `src/libgit2/tree.c` |  |
| `git_tree_walk` | symbol | `src/libgit2/tree.c` |  |
| `git_treebuilder_filter` | symbol | `src/libgit2/tree.c` |  |
| `git_treebuilder_new` | symbol | `src/libgit2/tree.c` |  |

A type-generating macro scheduled as a type is annotated as a `ty`, under the macro's
name.

The `/// Handwritten` marker is retired: remove each one in the file, and the empty doc
line it leaves at the head or end of a doc comment.

On this host, five compile-fail snapshots of `tests/ub_safe` fail before any change, because
ffibox is reached through a symlinked path the snapshots do not normalize: they are no
regression of yours.

## Departures for this run

- **No reports.** File nothing under `crustify/reviews/`, and land one commit, your change
  to `tree.rs`, rather than the reports commit and the fix commit.
- **No sanitizer, Miri or BSan gates, and no new tests**: the change is to doc comments and
  `#[doc(alias)]` attributes only. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p
  libgit2` to your gates: it rejects an alias where rustdoc admits none. Change no code
  beyond the annotations; if an annotation cannot be placed without one, leave it and say
  why in your final message.
- **No dashboard row and no ffimap scan**: this batch has no entry in `schedule.json`.
- **Commit nothing outside `tree.rs`**, `Cargo.lock` included.
