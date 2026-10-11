# Lifecycle gap: `git_tree` has a constructor the type run did not emit

**Item:** `git_tree` (wave 1 batch 3, `crustify/rust/libgit2/libgit2/object/tree.rs`)
**Affected commit:** `47a8e7a36`, reviewed at `38e6d1b3e`

**Finding.** The batch emitted `TreeArc<'r>` with `git_tree_free`, `git_tree_dup` and the
constructors `git_tree_lookup` / `git_tree_lookup_prefix`, and the earlier object review
added `TreeEntry::bypath`. A third public routine hands the caller a counted tree of its
own, an `alloc` constructor by out-parameter declared in `include/git2/commit.h`:

```c
int git_commit_tree(git_tree **tree_out, const git_commit *commit);
```

`src/libgit2/commit.c`: it is `git_tree_lookup(tree_out, commit->object.repo,
&commit->tree_id)` — one more reference of the repository's cache, or a fresh parse,
released with `git_tree_free`, untouched on failure (a missing tree, or an object of
another type). The tree keeps the commit's repository uncounted, so it carries the same
`'r`. Both of its types were wrapped by the same wave (`git_commit` in batch 1), so
nothing kept it from being emitted with either type run.

**Fix.** `CommitRef::tree(&self) -> Result<TreeArc<'r>, RepositoryError>` (anchor
`Wraps: git_commit_tree`), homed with its header in `object/commit.rs`; a
`compile_fail,E0505` doctest shows the tree cannot outlive the repository borrow. The
`-sys` allowlist gained the routine.

**Evidence.** `tests/equiv/commit.rs::navigation::derived_commits_and_trees_match_c`
derives the tree of each of the thirteen base commits on both sides, including a commit
whose tree is missing and one whose tree is a blob, and compares id, type and identity
with earlier results, or code, class and message, before and after the bases are
released; `tests/ub_safe/commit.rs::commit::{derived_owners_outlive_what_they_came_from,
derived_owners_survive_eviction_and_a_disabled_cache}` (trees outliving their commit, its
array, and cache eviction); `commit.rs::unit_tests::parents_ancestors_and_trees_derive_owned_references`.
