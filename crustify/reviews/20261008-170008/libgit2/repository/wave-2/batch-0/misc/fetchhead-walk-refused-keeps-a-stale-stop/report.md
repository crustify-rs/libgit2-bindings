# `for_each_fetchhead` reset the callback's stop record only after its refusals

- **item**: `git_repository_fetchhead_foreach` (symbol),
  `crustify/rust/libgit2/libgit2/repository/repository.rs`:
  `RepositoryRef::for_each_fetchhead`, with `FetchheadForeachCallback::stopped`
- **affected commit**: `9468d5210` (libgit2/repository wave 2 batch 0); reviewed at
  `16d2fcaae`
- **class**: inconsistent state after a refused call; no reproducer (a unit test
  shows it)

## Finding

The wrapper documents that each walk resets the callback's record of a stop
(`stopped()`: "whether the last call stopped the iteration"), so that a walk that makes
no call (an empty `FETCH_HEAD`) or fails before any (no `FETCH_HEAD`) reads as "not
stopped", and `GIT_EUSER` is told from libgit2's own errors by it. The reset
(`callback.stopped = false`) came after the two refusals the wrapper makes before
calling libgit2 (no pinned runtime; a repository without paths), so a callback that
had stopped an earlier walk still answered `stopped() == true` after a walk refused for
a pathless repository, where the same callback after a walk that failed for a missing
`FETCH_HEAD` answered `false` (the unit test
`for_each_fetchhead_tells_a_stop_from_a_later_failure` asserts the latter).

## Fix

The reset now comes first: any walk, refused or not, that made no call reads as not
stopped. Regression: the second half of
`unit_tests::init_ext_and_fetchhead::for_each_fetchhead_refuses_a_repository_without_paths`
(a callback that stopped a `call`, then a refused walk, then `stopped()` is `false`).
