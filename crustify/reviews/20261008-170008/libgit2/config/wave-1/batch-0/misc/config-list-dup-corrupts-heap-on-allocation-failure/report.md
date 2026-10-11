# Duplicating a config entry list corrupts the heap on allocation failure (C, documented)

- **items**: `git_config_entry` (type), through `ConfigRef::entry` once this review
  composes it from iterators and a snapshot (`../../ub_safe/entry-lookup-forms-member-of-unwritten-backend-entry`);
  also `ConfigRef::snapshot` (`git_config_snapshot`, wave 1 batch 2), and later the
  wave 3/4 iterator and foreach wrappers
- **found at**: `d79771f6f` plus this review's fix, by
  `config.rs::unit_tests::entry_allocation_failures_leave_config_usable`
- **class**: misc. A libgit2 defect reachable only through allocation failure. The safe
  API cannot inject one (`core::common::set_allocator` is `unsafe`), so this is
  documented, not fixed.

## Finding

Every file-backend iterator duplicates the backend's entry list
(`config_file_iterator`, `config_file.c:279`: `git_config_list_dup`), and every snapshot
copies entries one by one (`config_snapshot_open` -> `git_config_list_dup_entry`). Both
mishandle a failed allocation in `src/libgit2/config_list.c`:

- `git_config_list_append` puts the new key's `map_head` into the map, and points an
  existing multivar head at the new entry, *before* allocating the list node. When that
  allocation fails, `git_config_list_dup_entry`'s `out:` frees `duplicated` (and its
  name) while the map still points at it. For a multivar the name it frees is the
  *first* entry's, which the append had substituted. `config_list_free` then frees
  `head->entry->base.entry.name` through the freed entry (`config_list.c:135`).
- `map_head = git__calloc(..)` in `git_config_list_append` is not checked before
  `map_head->entry = entry`.
- The `GIT_ERROR_CHECK_ALLOC` returns in `git_config_list_dup_entry` leak `duplicated`
  and its name.
- `git_config_list_dup` ignores the error of `git_config_list_dup_entry` (`error` is still
  0 from `git_config_list_new`). It returns success without setting `*out`, and
  `config_file_iterator` then builds an iterator over a NULL list.

With a counting allocator that refuses every allocation after the Nth, an entry lookup
on a config with one file backend crashed in the child:

```
#0  __GI___libc_free (mem=0x7ff80ff7d1fc) at ./malloc/malloc.c:3375
#3  git__free (ptr=0x7ff80ff7d1fc) at /target/src/util/alloc.h:39
#4  config_list_free (config_list=0x7ffff00c50f0) at /target/src/libgit2/config_list.c:135
#5  git_config_list_free (config_list=..) at /target/src/libgit2/config_list.c:155
#6  git_config_list_dup (out=.., config_list=..) at /target/src/libgit2/config_list.c:112
#7  config_file_iterator (iter=.., backend=..) at /target/src/libgit2/config_file.c:279
#8  all_iter_next (..) at /target/src/libgit2/config.c:469
#9  multivar_iter_next (..) at /target/src/libgit2/config.c:1103
#10 git_config_next (..) at /target/src/libgit2/config.c:1193
#11 libgit2::libgit2::config::config::ConfigRef::reread_through .. config.rs:1373
#12 libgit2::libgit2::config::config::ConfigRef::entry .. config.rs:1318
```

C's own `git_config_get_entry` reads a file backend's list in place and never duplicates
it, so on this path the composed lookup trades an allocation-failure crash for the
reread UB it removes. `ConfigRef::snapshot` already had the exposure, as will every
config iterator. The wave-0 review documented the same class of defect for
`git_refdb_open` (`../../../wave-0/batch-0/misc/refdb-open-frees-uninitialised-pointer-on-oom`).

## Action

Documented on `ConfigRef::entry`. The new allocation-failure sweep
(`entry_allocation_failures_leave_config_usable`) runs over a config without file
backends, whose lookup duplicates no list. It points here. A fix belongs in libgit2:
allocate the list node before touching the map, free on the early returns, and
propagate the error in `git_config_list_dup`.
