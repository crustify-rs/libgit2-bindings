# `foreach_multivar` had no value filter, so C's filtered walk could not be expressed

- **item**: `git_config_get_multivar_foreach` (`ConfigRef::foreach_multivar`),
  libgit2/config wave 3 batch 1 (`8a2a2d30c`),
  `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `0cf65b483` (the review wave's base)
- **class**: incomplete safe surface. Part of the C routine's behaviour had no safe
  equivalent, although a safe building block for it now exists in the tree. There is no
  reproducer: the affected API cannot even express the call that would be compared.

## Finding

`git_config_get_multivar_foreach(cfg, name, regexp, cb, payload)` takes an optional
value regex. With one, the walk calls back only for values of `name` that match. If
nothing matches, it fails with `GIT_ENOTFOUND`. libgit2 uses this to visit selected
values of a multivar, such as one `remote.<name>.fetch` refspec.

The batch dropped the parameter and always passed NULL. The batch's own docs gave the
reason: C's `multivar_iter_next` (`config.c:1110`) passes a valueless entry's NULL value
to `git_regexp_match`, which calls `strlen(NULL)`. `git_config_multivar_iterator_new`
(`config.c:1145`) also leaks the normalized name when the pattern fails to compile.
Neither defect can be reached safely through C's own filter. The docs told callers to
filter `entry.value()` in the closure. That is not equivalent: libgit2's matching
engine (PCRE2, with libgit2's compile options) is not available to the closure. The
`NotFound` result for "no value matched" was lost too, because the unfiltered walk
succeeds as soon as `name` has any value. As a result, no safe call reproduced
`git_config_get_multivar_foreach(cfg, "a.m", "^t", …)`.

The same wave's batch 0 (`0de2b59b4`) added `ConfigRef::multivar_iterator(name,
Some(regexp))` in `config/sys/config.rs`. It keeps C's name-selection iterator and adds
a Rust-owned value filter. The filter links the same system PCRE2 with libgit2's
options, skips valueless entries and frees everything when a pattern fails. So a safe
dependency for the missing half was in the tree. The two batches ran concurrently,
which is why the translation could not use it.

## Fix

`foreach_multivar(name, regexp: Option<&CStr>, callback)`:

- `None` calls `git_config_get_multivar_foreach` with NULL, as before.
- `Some(pattern)` builds the walk the way C does (`config.c:1057-1086`), over
  `multivar_iterator(name, Some(pattern))` and the same `ForeachCallback` trampoline.
  It runs inside `walk_with`, so a construction failure resets the stop state too.
  The behaviour copies C's: construction errors leave the thread's error as it is (an
  invalid name gives `InvalidSpec` before the pattern is compiled, then an invalid
  pattern gives `InvalidSpec`/`GIT_ERROR_REGEX`). A non-zero callback return stops the
  walk. It leaves "git_config_get_multivar_foreach callback returned N" only when no
  error is set (a private `after_callback`, which is `git_error_set_after_callback`). The
  iterator is released before returning. An empty filtered walk sets C's "config value
  '%s' was not found" with the caller's spelling and returns `GIT_ENOTFOUND`. Stops,
  `GIT_ITEROVER` stops and panics are reported as on the unfiltered path.
- The one deliberate divergence: a valueless setting is skipped, not passed to
  `strlen(NULL)`.

All callers in the crate and its suites now pass `None`.

## Regression evidence

- `tests/equiv/config.rs::enumeration::enumeration_value_filters_match_c` runs 11
  name/pattern pairs × 5 stop codes × 2 error seeds against C's filtered walk on
  independently owned files. That is 110 walks per phase, over four phases: one file, an
  `APP` backend added, `set_multivar` replacing a value, and the low file corrupted. It
  covers matches across backends, `^$` on an empty value, a raw `\xff` pattern byte,
  subsection case, `(*UTF)`, no match, an invalid name before an invalid pattern, and
  refresh errors. Each walk compares return code, stop state, callback fields and order,
  and the last error. The shared `compare` now also runs every walk once with no earlier
  error, so the "callback returned" message C leaves is compared too. With that message
  misspelled in `after_callback`, or the not-found format changed, the test fails.
- `enumeration_invalid_value_filters_match_c` compares invalid patterns for valid and
  missing names (code, class, PCRE2 message). It runs in a child process with
  `detect_leaks=0`, because C's reference leaks the normalized name there. The first
  ASan run of this review caught that leak: 40 × 4 bytes, one `"a.m"` per C walk.
- `tests/ub_safe/config.rs::enumeration::enumeration_value_filter_skips_valueless_and_cleans_up`
  covers valueless settings (`NotFound`, no `strlen(NULL)`), invalid patterns repeated
  under LSan, stop codes including extremes and `GIT_ITEROVER`, stop state reset after a
  failed construction, panic cleanup and reuse, and nested filtered walks whose outer
  entry stays pinned.
- `unit_tests::enumeration_initial_allocation_failure_is_reported` also runs the
  filtered walk under a refusing allocator.
