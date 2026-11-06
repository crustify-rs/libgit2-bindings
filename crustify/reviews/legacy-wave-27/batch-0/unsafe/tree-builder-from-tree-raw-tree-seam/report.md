# Unsafe surface: `TreeBuilder::from_tree` took a raw `git_tree` pointer

**Item:** `git_treebuilder` (wave 0 batch 4, `crustify/rust/libgit2/libgit2/object/tree.rs`)

**Before.** `pub unsafe fn from_tree(repo: RepositoryRef<'r>, source: NonNull<ffi::git_tree>)`,
documented as a raw seam "until `git_tree` is wrapped", with the caller vouching that
the pointer addresses a live tree for the call. Every consumer (the `ub_unsafe` and
`equiv` suites) had to look the tree up raw (`git_tree_lookup`), hold it as a bare
pointer and free it by hand; `crustify scan-unsafe` counted it among the three
`unsafe_fns_pub_smell` sites of this workset.

**Why it need not be unsafe.** A tree object *is* a `git_tree` (`struct git_tree { git_object object; ... }`),
and `git_object` was wrapped in the same wave (batch 3): a shared object handle,
`ObjectRef<'_, '_>`, keeps its object alive for the handle's lifetime and carries the
borrow of the repository the object reads. `git_treebuilder_new` only reads the
source's entries and keeps nothing of it. The one thing the type system cannot say is
that the object *is* a tree, and that is a one-line runtime check on the type libgit2
caches in the object (`git_object_type`), which `git_treebuilder_new` itself never makes:
given a commit, blob or tag it reads the smaller object as a tree.

**Fix.** `pub fn from_tree(repo: RepositoryRef<'r>, source: ObjectRef<'_, '_>) -> Result<TreeBuilderBox<'r>, ErrorBuf>`:
an object of another type is refused with a `GIT_ERROR_TREE` error "the object is not
a tree" (shared helper `as_tree`, also used by the new `TreeEntry::bypath`). The seam,
its `# Safety` contract, the `compile_fail/tree_builder_from_tree_requires_unsafe` case and
the `tests/ub_unsafe/tree.rs` suite are gone; its four scenarios (flat and nested trees
of another repository, trees of the builder's own repository, a SHA256 tree seeding a
SHA1 builder) now run through the safe API in `tests/ub_safe/tree.rs::builder`, joined by
`only_trees_seed_builders` (a commit, blob and tag refused before libgit2 reads them;
ASan would flag the over-read). `tests/equiv/tree.rs::builder` compares the seeded
builders and the allocation-failure script through the safe form.

The workset adds no public `unsafe fn` any more; the `ub_unsafe` obligation for
`git_treebuilder` is void.
