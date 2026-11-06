# Mapping records missed the NULL `%s` miss report and misdescribed the output slot

- **items**: `git_config_lookup_map_value` (arg 0 `out`, arg 3 `value`) and
  `git_config_get_mapped` (arg 1 `config`), submitted by libgit2/config wave 2 batch 1
  (`d6a13729c`)
- **affected commit**: `3a5814a4e`
- **class**: incomplete/incorrect ownership notes (Wavefront store). The flags
  (`nullable`, `borrowed`, `mutable`) were right.

## Finding

1. `git_config_lookup_map_value`, arg 3 `value`: "NULL denotes valueless boolean true;
   empty denotes false." That is true only for a `GIT_CONFIGMAP_TRUE` rule. With no such
   rule, C's miss path runs `git_error_set(GIT_ERROR_CONFIG, "failed to map '%s'", value)`
   (`config.c:1385`) with NULL, which is undefined in ISO C. A consumer reading the record
   would pass NULL freely. See `../../misc/valueless-map-miss-formats-null`.
2. `git_config_lookup_map_value`, arg 0 `out`: "Failed parse may touch output." It never
   does. `*out = m->map_value` runs only on a match, and `git_config_parse_int32` stores
   `*out = truncate` only after `git_config_parse_int64` succeeded and the value fits
   (`config.c:1469-1487`). On a failed parse it jumps to `fail_parse` before the store.
3. `git_config_get_mapped`, arg 1 `config`: said to "wrap through reread + snapshot
   before calling mapper". That plan still reaches finding 1 for a valueless entry, since
   C's `get_mapped` calls the mapper with `entry->value`.

## Fix

Resubmitted both records through `wavefront query symbols --name <sym> --update`. The
`value` note gives the TRUE-only match and the NULL `%s` miss report. The `out` note says
it is written only on success. The `config` note records the composition the wrapper now
uses (safe entry acquisition + safe `lookup_map_value`). The store diff is the three note
lines. `query symbols --name` reads them back.
