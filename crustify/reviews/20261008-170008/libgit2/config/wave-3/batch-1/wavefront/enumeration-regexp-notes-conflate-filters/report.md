# Enumeration records shared one regex note between a name filter and a value filter

- **items**: `git_config_foreach_match` (arg 1 `regexp`) and
  `git_config_get_multivar_foreach` (arg 1 `name`, arg 2 `regexp`), submitted by
  libgit2/config wave 3 batch 1 (`8a2a2d30c`)
- **affected commit**: `0cf65b483`
- **class**: incorrect/incomplete ownership notes (Wavefront store). The flags
  (`string`, `borrowed`, `nullable`, `mutable`) and `lifetime: null` were right.

## Finding

1. `git_config_foreach_match`, arg 1 `regexp`, carried the same note as the multivar
   value regex: "foreach_match filters names; multivar filters values, dereferencing
   NULL valueless entries. Invalid multivar regex leaks normalized name; safe wrapper
   omits that parameter." The last clause is false for this routine:
   `ConfigRef::foreach_match` exposes the pattern as `Option<&CStr>`. The note also
   omits what the routine actually does with it. `git_config_iterator_glob_new`
   (`config.c:552-576`) walks everything for NULL, compiles anything else, and on a
   compile failure returns `-1`, not the `GIT_EINVALIDSPEC` that `git_regexp_compile`
   returned, with a `GIT_ERROR_REGEX` error.
2. `git_config_get_multivar_foreach`, arg 1 `name`: "copy freed on ordinary
   completion/errors." Its own arg 2 note contradicts this. If the pattern fails to
   compile after the name was normalized, `on_error` (`config.c:1159-1163`) frees the
   iterator but not `iter->name`. The note also omitted the `GIT_EINVALIDSPEC` for an
   invalid name, and that `GIT_ENOTFOUND` quotes the caller's spelling, not the
   normalized one (`config_error_notfound(name)`, `config.c:1083`).
3. `git_config_get_multivar_foreach`, arg 2 `regexp`, said the safe wrapper omits the
   parameter. After `../../misc/multivar-foreach-drops-value-regexp` it no longer
   does. The wrapper still passes NULL to C, and builds a filtered walk over the safe
   value filter.

## Fix

Resubmitted both records through `wavefront query symbols --name <sym> --update`,
changing only the three notes. Each routine now has its own regex note, and the name
note records the leak and both error paths. The store diff is three note lines.
`query symbols --name` reads them back.
