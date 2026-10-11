# The amend records omit that `HEAD`'s author comes from libgit2's own parse

- **records**: `git_commit_amend_from_tree` (`ptr_args[4]`, `opts`) and
  `git_commit_amend_from_stage` (`ptr_args[3]`, `opts`), symbols in `src/libgit2/commit.c`
- **submitted by**: libgit2/object wave 3 batch 0 (`11ea2e65f`)
- **resubmitted**: yes, through `wavefront … query symbols --name <record> --update`. Only
  the two notes changed in `crustify/wavefront/ownership-store.json`; they were spliced into
  the store's existing layout, without the re-sort the tool makes.

## Previous finding

> … author/committer (NULL = git_signature_default; for amend a NULL author is HEAD's
> commit's author), … Signatures are printed with %s and their offset negated as int:
> NULL strings / INT_MIN are UB. …

## Evidence it is incomplete

The note says where the author comes from but not which parse is read, and that
distinction decides whether a caller can check the author first.
`git_commit_amend_from_tree` (`src/libgit2/commit.c:1402-1464`) peels `HEAD` itself
(`git_reference_peel((git_object **)&head_commit, head_ref, GIT_OBJECT_COMMIT)`) and
writes `git_commit_author(head_commit)`. That object is shared with a commit the caller
looked up only through the object cache, and `cache_should_store` (`src/libgit2/cache.c`)
keeps no commit of 4096 bytes or more, and nothing while caching is off. A parse that runs
out of memory copying the name or email leaves it NULL and still succeeds
(`git_signature__parse`, `extract_trimmed`), so the writer can get a NULL `%s` from the
one parse a caller cannot check.

The wrappers built on the old record checked a parse of their own and assumed libgit2
would read the same object (`internal/amend-author-relies-on-the-object-cache`, with a
reproducing unit test).

## Corrected finding

> … for amend a NULL author is HEAD's commit's author, read from libgit2's own
> git_reference_peel of HEAD: a fresh parse unless the object cache holds the commit,
> and the cache keeps no commit over 4096 bytes nor any with caching off, so it need not
> be the parse a caller looked up and checked, and a parse that ran out of memory leaves
> its name/email NULL while succeeding …

The ownership, borrow, nullability and mutability facets are unchanged. Re-read against
the C and found accurate: the other slots of both records, and the
`git_commit_create_from_tree` / `git_commit_create_from_stage` records. Their
`git_repository_commit_parents` hazard and the `-1` versus `GIT_EUNCHANGED` returns are
right.
