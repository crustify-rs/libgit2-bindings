# Lifecycle gap: `git_tree_entry` has a constructor the type run did not emit

**Item:** `git_tree_entry` (wave 0 batch 3, `crustify/rust/libgit2/libgit2/object/tree.rs`)

**Finding.** The batch emitted the owner `TreeEntryBox` with its releaser
(`git_tree_entry_free`, `TreeEntryFree`) and cloner (`git_tree_entry_dup`, `Clone` /
`try_clone` / `TreeEntryRef::to_tree_entry`), but no constructor. libgit2 publishes one:

```c
int git_tree_entry_bypath(git_tree_entry **entry_out, const git_tree *root, const char *path);
```

`src/libgit2/tree.c`: it walks `path` component by component through `root` (looking the
subtrees up in `root->object.repo`) and, for the last component, returns
`git_tree_entry_dup(entry_out, entry)`: a fresh block of libgit2's allocator holding the
struct and its inline name, which the caller owns and releases with
`git_tree_entry_free`. It keeps nothing of the tree or the repository. That is an `alloc`
constructor of the type (out-parameter `entry_out`), in the same sense as
`git_object_lookup` is one of `git_object`; the batch's `ub_safe` fixture
(`fixture::object_and_tree_entry::entry_bypath`) even adopted its result raw in every
test, "standing in for the `git_tree` wrapper's `git_tree_entry_bypath`".

The constructor does not need `git_tree` to be wrapped: a tree object is a `git_tree`
(its first member is the `git_object`), and `git_object` was wrapped by the same batch, so
a shared object handle whose type is `GIT_OBJECT_TREE` is the argument.

**Fix.** `TreeEntry::bypath(tree: ObjectRef<'_, '_>, path: &CStr) -> Result<TreeEntryBox, RepositoryError>`
(anchor `Wraps: git_tree_entry_bypath`). An object of another type is refused in Rust
with `GIT_EINVALIDSPEC` and a `GIT_ERROR_TREE` error "the object is not a tree" (the C
routine would read a commit, blob or tag as a tree). Failures carry libgit2's code
(`GIT_ENOTFOUND` for an invalid path, a missing component or a `/` after a non-tree; `-1`
for a subtree that cannot be read or a refused allocation). The fixture `entry_bypath` is
gone; the `ub_safe` tree-entry tests build their entries through the constructor.

**Evidence.** `tests/equiv/tree.rs::tree_entry::path_lookups_match_c` (17 probe paths,
successes and failures compared with `git_tree_entry_bypath` by entry view or by code,
class and message); `tests/ub_safe/tree.rs::tree_entry::{path_lookups_own_their_entry_alone,
only_trees_are_looked_into}` (ASan/LSan: every failure leaves nothing, every entry is
released once and outlives the tree, the repository and the subtrees read on the way);
`tree.rs::unit_tests::tree_entry::bypath_looks_only_into_trees` (the Rust-side refusal,
and a refused allocation reported as libgit2's out-of-memory error).

Wavefront finding submitted for `git_tree_entry_bypath` (see the `wavefront/` report).
