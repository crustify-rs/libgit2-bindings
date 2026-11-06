# Lifecycle gap: `git_commit` has two constructors the type run did not emit

**Item:** `git_commit` (wave 1 batch 1, `crustify/rust/libgit2/libgit2/object/commit.rs`)
**Affected commit:** `e2fba68e0`, reviewed at `38e6d1b3e`

**Finding.** The batch emitted the owner `CommitArc<'r>` with its releaser
(`git_commit_free`), its up-ref cloner (`git_commit_dup`: `Clone` and
`CommitRef::to_commit`) and the constructors `git_commit_lookup` /
`git_commit_lookup_prefix`. `include/git2/commit.h` publishes two more routines that hand
the caller a counted reference of its own, both `alloc` constructors by out-parameter:

```c
int git_commit_parent(git_commit **out, const git_commit *commit, unsigned int n);
int git_commit_nth_gen_ancestor(git_commit **ancestor, const git_commit *commit, unsigned int n);
```

`src/libgit2/commit.c`: `git_commit_parent` reads `commit->parent_ids[n]` and returns
`git_commit_lookup(parent, commit->object.repo, parent_id)` — one more reference of the
repository's cache, or a fresh parse, released with `git_commit_free` — or
`GIT_ENOTFOUND` with `GIT_ERROR_INVALID` "parent %u does not exist" past the last parent.
`git_commit_nth_gen_ancestor` takes one reference on `commit` itself (`git_commit_dup`
through a cast of the `const` pointer), returns it for `n == 0`, and otherwise follows
first parents with `git_commit_parent`, freeing every intermediate commit. Either result
keeps the repository uncounted like every object, so it belongs to the same `'r` as the
commit it was derived from. The batch's own wavefront note on `git_commit` names both
routines as constructors; neither was emitted, and neither took anything unwrapped (a
shared commit handle and an index).

**Fix.** `CommitRef::parent(&self, n: u32)` (anchor `Wraps: git_commit_parent`) and
`CommitRef::nth_gen_ancestor(&self, n: u32)` (anchor `Wraps: git_commit_nth_gen_ancestor`),
both returning `Result<CommitArc<'r>, RepositoryError>`, as `Commit::lookup` does; `n`
keeps C's `unsigned int`. The `-sys` allowlist gained both routines (and
`git_commit_parentcount`, which the equivalence test reads raw). A `compile_fail,E0505`
doctest shows a parent cannot outlive the repository borrow.

**Evidence.**

- `tests/equiv/commit.rs::navigation::derived_commits_and_trees_match_c`: thirteen base
  commits in three repositories per side (`testrepo.git` merges, roots and its
  seven-generation first-parent chain; `shallow.git`, whose boundary commit is grafted
  parentless; a fresh repository holding hand-written commits whose parent is a blob or
  missing, or whose parent's tree is missing), every parent up to the third and ancestors
  of 0, 1, 2, 3, 6, 7 and `u32::MAX` generations: 143 steps compared with C by id, type,
  identity with the base and with earlier results, or by code, class and message (every
  failure kind is asserted reached); then the bases released and every kept result read
  again.
- `tests/ub_safe/commit.rs::commit::{derived_owners_outlive_what_they_came_from,
  derived_owners_survive_eviction_and_a_disabled_cache, refused_derivations_leave_nothing}`
  (ASan/LSan: chains derived from commits dropped first, from commits an array lends and
  outlive, through a one-byte and a disabled cache; refusals at every index width leave
  nothing).
- `commit.rs::unit_tests::parents_ancestors_and_trees_derive_owned_references`.

Wavefront findings submitted for both symbols (see the `wavefront/` report).
