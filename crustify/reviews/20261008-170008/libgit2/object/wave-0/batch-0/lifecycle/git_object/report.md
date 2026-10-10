# Lifecycle gap: `git_object` has two constructors the type run did not emit

**Item:** `git_object` (wave 0 batch 3, `crustify/rust/libgit2/libgit2/object/object.rs`)

**Finding.** The batch emitted the owner `ObjectArc<'r>` with its releaser
(`git_object_free`), its up-ref cloner (`git_object_dup`, `Clone` and
`ObjectRef::to_object`) and the constructors `git_object_lookup` / `git_object_lookup_prefix`.
`include/git2/object.h` publishes two more routines that hand the caller one counted
reference of its own, both `alloc` constructors by out-parameter:

```c
int git_object_peel(git_object **peeled, const git_object *object, git_object_t target_type);
int git_object_lookup_bypath(git_object **out, const git_object *treeish, const char *path, git_object_t type);
```

`src/libgit2/object.c`: `git_object_peel` returns the very object with one more
reference (`git_object_dup`) when it is already of the target type, otherwise follows
tags (`git_tag_target`) and commits (`git_commit_tree`), each a lookup through the
object's repository, freeing every intermediate object; `git_object_lookup_bypath` peels
the object to a tree, takes the entry at `path` (`git_tree_entry_bypath`) and looks its
object up (`git_tree_entry_to_object` → `git_object_lookup` in
`git_object_owner(treeish)`). Either result is released with `git_object_free`, keeps the
repository uncounted like every object, and belongs to the same `'r`. Neither takes
anything the batch had not wrapped: a shared handle, a `&CStr` and the `ObjectType`
filter `lookup` already takes.

**Fix.** `ObjectRef::peel(&self, kind: Option<ObjectType>)` (anchor
`Wraps: git_object_peel`) and `ObjectRef::lookup_bypath(&self, path: &CStr, kind: Option<ObjectType>)`
(anchor `Wraps: git_object_lookup_bypath`), both returning
`Result<ObjectArc<'r>, RepositoryError>` like `lookup`. `None` is `GIT_OBJECT_ANY`, so the
C routines' argument check on the target type always passes.

**Evidence.** `tests/equiv/object.rs::peel_and_bypath::peels_and_path_lookups_match_c`
(six base objects — a commit, a tree, a blob, a tag of a tag of a commit, a tag of a blob
and the commit whose tree nests three subtrees — peeled to every filter, the results
peeled again, and 16 probe paths looked up under every filter: 59 derived objects and
every failure compared with C by id and type or by code, class and message, including
whether a peel returned the very object); `tests/ub_safe/object.rs::{derived_objects_count_their_own_reference,
refused_derivations_leave_nothing_behind, derived_objects_outlive_cache_eviction}`
(ASan/LSan: chains of derived objects released in every order after the objects they
came from, every refusal leaving nothing, derived objects surviving a one-byte cache);
`object.rs::unit_tests::peels_and_path_lookups_derive_owned_references`.

Wavefront findings submitted for both symbols (see the `wavefront/` report).
