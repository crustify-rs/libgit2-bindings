# The amend wrappers check `HEAD`'s author in one parse and let libgit2 write it from another

- **items**: `git_commit_amend_from_tree` (`amend_from_tree`), `git_commit_amend_from_stage`
  (`amend_from_stage`), through their shared `amend_on_head` / `head_commit` in
  `crustify/rust/libgit2/libgit2/object/commit.rs`
- **affected commit**: `11ea2e65f` (libgit2/object wave 3 batch 0)
- **class**: internal — a private keepalive whose justification does not hold

## Finding

When the options leave the author unset, `git_commit_amend_from_tree` writes the author of
the commit `HEAD` peels to (`git_commit_author(head_commit)`). libgit2's signature parser
leaves a NULL name or email when the copy runs out of memory and still reports success,
and the writer prints both with `%s` (undefined behaviour for NULL; glibc prints
`(null)`). The wrapper refuses such an author: `head_commit` peels `HEAD` itself and
checks the author with `writable`. Its doc then said:

> Held for the call, the commit stays in the object cache, from which libgit2's own
> lookup of the same id takes it.

and the `SAFETY` comment of the call relied on it ("`HEAD`'s commit, whose author is
written when the options set none, is writable").

That holds only for commits the cache keeps. `cache_should_store` (`src/libgit2/cache.c`)
keeps no commit of 4096 bytes or more (`git_cache__max_object_size[GIT_OBJECT_COMMIT]`), and
none at all once caching is turned off. Then libgit2's `git_reference_peel` parses `HEAD`'s
commit anew, and an allocation failure in that second parse reaches the writer with a NULL
name, after the wrapper checked a different parse.

## Evidence

`unit_tests::uncached_amended_authors_are_the_ones_checked` installs an allocator that
grants the first 201-byte block (the author name's copy, the parse's only block of that
length) and refuses later ones. It amends a commit carrying a 5000-byte message with no
author set. On `11ea2e65f` both wrappers succeed and write the author as `(null)`:

```
thread '…::uncached_amended_authors_are_the_ones_checked' panicked at libgit2/libgit2/object/commit.rs:7306:21:
assertion `left == right` failed
  left: Some("(null)")
 right: Some("NNNN…N")
```

The same run under the `asan-ubsan` variant fails the same assertion and gets no
sanitizer report, because glibc's `vsnprintf` prints `(null)` for a NULL `%s`. So this is
recorded as an internal defect, not as a UB reproducer. Reaching it needs an allocator
that fails (an `unsafe impl Alloc` within its contract, or real memory exhaustion).

## Fix

`amend_on_head` no longer relies on the cache. When `head_commit` holds a commit, the
wrapper passes libgit2 a copy of the options: the caller's, or C's defaults for `None`,
with `author` set to the held commit's author, which was just checked. With an author
set, `git_commit_amend_from_tree` writes that author and reads nothing from its own parse.
Everything else in the options is copied as it is, the signer's payload included. The
caller's options are untouched, and stay frozen behind the exclusive borrow while libgit2
uses the copy. The output is the same as before for every commit libgit2 parses the same
way.

One residual difference: if another process moves `HEAD` between the wrapper's peel and
libgit2's, the author comes from the commit the wrapper peeled, while the parents come
from the one libgit2 peeled.

Regressions:

- `uncached_amended_authors_are_the_ones_checked` in `commit.rs::unit_tests`, which fails
  on the base as above;
- four steps in `tests/equiv/commit.rs::create_options` amending commits over 4096 bytes
  (with options that leave the author unset, and with none), compared with C after each
  step.
