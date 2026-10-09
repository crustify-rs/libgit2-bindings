# `OidShortenerMut::add`'s `# Aborts` section understates when it aborts

- **item**: `git_oid_shorten` (type), `crustify/rust/libgit2/libgit2/oid/oid.rs`
- **affected**: the documentation of `OidShortenerMut::add` (`git_oid_shorten_add`)
- **affected commit**: `ed8e139b2` (libgit2/oid wave 1 batch 1, which added `add`;
  reviewed at `fc216e522`)
- **class**: misc (documentation); no UB or equivalence defect

## Finding

`add` must tell a full trie (C keeps the trie, every later addition fails cleanly)
from a lost one (C's `resize_trie` assigns a failed `git__reallocarray` result to
`os->nodes`, so the next addition dereferences NULL) and aborts on the latter, because
a borrowed handle cannot stop the next addition. Both come back as `-1`; the wrapper
tells them apart by the thread's last error, treating only an exact
`GIT_ERROR_INVALID` "unable to shorten OID - OID set full" as the full case.

That is the right discriminator (`src/util/errors.c`: a lost trie reaches
`git_error_set_oom`, which installs the static out-of-memory error without
allocating; the full case goes through `git_error_set`, which formats into the
thread's `git_str`), but it has one more aborting input than the doc stated: once a
thread's error buffer has failed to grow it is marked `git_str__oom` for the thread's
lifetime (`git_str_try_grow` refuses it, `git_str_clear` keeps it), after which
`git_error_set` records nothing. A full trie on such a thread leaves the previous
error in place, the wrapper cannot tell it from a lost trie, and it aborts a process
whose shortener is intact. The doc said only "when libgit2 cannot grow the trie".

Confirmed with a scratch probe (a child process with a refusing allocator installed:
one oversized `git_error_set` while allocations fail marks the buffer, allocations are
then allowed again, and 40,000 ids are added to a fresh shortener):

```
after sticky: Some(NoMemory) "Out of memory"
after set on sticky thread: Some(NoMemory) "Out of memory"
git_oid_shorten_add lost its trie to an allocation failure; aborting
```

The second line shows `git_error_set` no longer recording anything on the thread; the
third is the abort at the point where, on a healthy thread, `add` returns the
"OID set full" error (`tests/equiv/oid.rs::shortener_fills_up_like_c`, same ids).

The behaviour is deliberate and safe (the alternative is a NULL dereference on the
next addition; Rust also aborts on allocation failure, and the state is reachable only
after libgit2 has already failed to allocate a few bytes on that thread), so the code
is unchanged.

## Fix

The `# Aborts` section now states the second input: a full trie whose error libgit2
could not record aborts too, and why. No code change, so no regression test; the
existing `unit_tests::add_aborts_when_the_trie_cannot_grow` covers the abort path.
