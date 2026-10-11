# `git_config_multivar_iterator_new` leaks its inner iterator on allocation failure (C, documented)

- **items**: `git_config_entry` (type), through `ConfigRef::entry` as composed by this
  review; later the wave-3/4 `git_config_get_multivar_foreach` /
  `git_config_multivar_iterator_new` wrappers
- **found at**: `d79771f6f` plus this review's fix, by
  `config.rs::unit_tests::entry_allocation_failures_leave_config_usable` under LSan
- **class**: misc. A libgit2 leak reachable only through allocation failure, which the
  safe API cannot inject. Documented, not fixed.

## Finding

```c
if ((error = git_config_iterator_new(&inner, config)) < 0)
    return error;

iter = git__calloc(1, sizeof(multivar_iter));
GIT_ERROR_CHECK_ALLOC(iter);          /* src/libgit2/config.c:1139: returns, leaking inner */
```

The later failures (name normalisation, regex compilation) go to `on_error`, which frees
`inner`. The allocation of `iter` returns directly. LeakSanitizer, refusing the lookup's
second allocation:

```
ERROR: LeakSanitizer: detected memory leaks
Direct leak of 64 byte(s) in 1 object(s) allocated from:
    #3 git__malloc /target/src/util/alloc.h:19:12
    #4 git__calloc /target/src/util/alloc.c:31:13
    #5 git_config_iterator_new /target/src/libgit2/config.c:538:9
    #6 git_config_multivar_iterator_new /target/src/libgit2/config.c:1135:15
    #7 <libgit2::libgit2::config::config::ConfigRef>::reread_through .. config.rs:1356:13
    #8 <libgit2::libgit2::config::config::ConfigRef>::entry .. config.rs:1318:37
```

## Action

Documented on `ConfigRef::entry` ("one failure leaks an iterator"). The sweep never
refuses that allocation and says why. It is the lookup's second allocation, and every
other one is refused in turn. The wave that wraps the multivar iterator inherits this
note.
