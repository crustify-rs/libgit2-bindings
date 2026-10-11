# Safe mapping of a valueless setting reached C's `printf("%s", NULL)`

- **items**: `git_config_lookup_map_value` (`lookup_map_value`) and
  `git_config_get_mapped` (`ConfigRef::get_mapped`), libgit2/config wave 2 batch 1
  (`d6a13729c`), `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `3a5814a4e` (the review wave's base)
- **class**: C defect reachable from the safe API. It is undefined behaviour in ISO C,
  but glibc defines it and no prepared sanitizer reports it, so it is filed here and not
  under `ub_safe`. There is no reproducer: on the campaign's platform the program's
  observable behaviour does not change.

## Finding

`git_config_lookup_map_value` (`src/libgit2/config.c:1347`) reports a value that no rule
maps as follows:

```c
	git_error_set(GIT_ERROR_CONFIG, "failed to map '%s'", value);
	return -1;
```

`value` may be NULL, and the safe API passes NULL for a valueless setting: `None` for
`lookup_map_value`, and `[a] flag` (no `=`) for `get_mapped`. Only a
`GIT_CONFIGMAP_TRUE` rule matches NULL. `git_config_parse_bool(NULL)` is true,
`git_config_parse_int32(NULL)` fails, and string rules skip NULL. So
`lookup_map_value(&[], None)`, or a table with no true rule, hands NULL to `%s`. ISO C
(7.21.6.1) requires a pointer to a string there, so this is undefined. glibc prints
`(null)`. Other libcs may crash. libgit2's own parsers avoid the same thing:
`git_config_parse_int64` / `_int32` format `value ? value : "(null)"`. The existing
equivalence tests ran this path on both sides (empty prefixes in
`map_lookup_matches_c_after_each_table_edit`, `a.flag` in
`mapped_config_matches_c_across_writes_snapshots_and_errors`). They pass under
ASan/UBSan: the printf interceptor skips NULL `%s` arguments, and UBSan has no check for
it.

## Fix

- `lookup_map_value`: when `value` is `None` and the table has no `MapMatch::True`
  rule, the wrapper sets `GIT_ERROR_CONFIG` "failed to map '(null)'" itself and returns
  `GIT_ERROR`, without calling C. That is C's result and last error as glibc renders
  them. C's speculative integer-parse errors in that case are always overwritten by the
  final one, so nothing observable is lost. Tables with a true rule still go to C, which
  never reaches the formatting then, so the stale parse error it leaves on success is
  preserved.
- `get_mapped` is now composed as C composes it: `entry(name)` (the existing safe
  reread + snapshot acquisition) followed by `lookup_map_value(maps, entry.value())`. It
  no longer calls `git_config_get_mapped`, whose mapper would format the entry's NULL
  value internally. This also removes the duplicated reread/snapshot/restore code and
  one unsafe block.

Equivalence with C is unchanged on glibc: the two existing tests above still compare the
None/valueless cases with C and pass. The corrected behaviour is pinned in
`config.rs::unit_tests::valueless_misses_never_reach_c_formatting`, which covers the
empty table, an Int32-only table, a False/Int32/String table, and tables with a true
rule (C's stale parse error is kept), through both `lookup_map_value` and `get_mapped`.
