# Deprecated field variants and a deprecated callback were scheduled

## What the earlier plan decided

Planner run 2's `schedule.json` gave five `todo` type items field anchors that exist only
when `GIT_DEPRECATE_HARD` is undefined:

| wave | item | anchors only without `GIT_DEPRECATE_HARD` |
|---:|---|---|
| 188 | `git_blob_filter_options` (`include/git2/blob.h`) | `commit_id` |
| 260 | `git_remote_callbacks` (`include/git2/remote.h`) | `resolve_url`, `update_tips` |
| 292 | `git_attr_options` (`include/git2/attr.h`) | `commit_id` |
| 300 | `git_filter_options` (`include/git2/filter.h`) | `commit_id` |
| 422 | `git_rebase_options` (`include/git2/rebase.h`) | `signing_cb` |

For example, wave 260's item:

```json
{"name": "git_remote_callbacks", "defined_in": "include/git2/remote.h", "kind": "type",
 "field_anchors": [..., "reserved_update_tips", ..., "resolve_url", "update_tips"]}
```

The anchors list both sides of each `#ifdef GIT_DEPRECATE_HARD ... #else ... #endif`:
`reserved` and `commit_id`, `reserved_update_tips` and `update_tips`, `reserved` and
`resolve_url`.

## Evidence

The campaign's scope is "the whole public API of the target, excluding deprecated items",
and its target configuration builds with `-DDEPRECATE_HARD=ON`
(`crustify/artifacts/build.json`), which compiles the library with `GIT_DEPRECATE_HARD`
defined (`c/src/CMakeLists.txt:9`: `add_definitions(-DGIT_DEPRECATE_HARD)`). In that build
each struct has only the `reserved` side:

```text
c/include/git2/attr.h:164        #ifdef GIT_DEPRECATE_HARD / void *reserved; / #else / git_oid *commit_id;
c/include/git2/blob.h:155        #ifdef GIT_DEPRECATE_HARD / void *reserved; / #else / ... commit_id
c/include/git2/filter.h:85       #ifdef GIT_DEPRECATE_HARD / void *reserved; / #else / git_oid *commit_id;
c/include/git2/rebase.h:90       #ifdef GIT_DEPRECATE_HARD / void *reserved; / #else / ... (*signing_cb)(...)
c/include/git2/remote.h:634      #ifdef GIT_DEPRECATE_HARD / void *reserved_update_tips; / #else / ... GIT_CALLBACK(update_tips)
c/include/git2/remote.h:694      #ifdef GIT_DEPRECATE_HARD / void *reserved; / #else / git_url_resolve_cb resolve_url;
c/include/git2/remote.h:559      #ifndef GIT_DEPRECATE_HARD / typedef ... GIT_CALLBACK(git_url_resolve_cb)(...)
```

The CodeQL trace also compiles the tests and the CLI, which include the headers without
`GIT_DEPRECATE_HARD`, so the T1 tables hold both sides of each struct
(`crustify/wavefront/codeql/t1/fields.csv` lines 1914/1916, 2042/2044, 2411/2413,
2678/2686, and `git_remote_callbacks`' `reserved`, `reserved_update_tips`, `resolve_url`),
and an API schedule lists every one of them in `used_fields`:

```sh
cd /target/libgit2-bindings
wavefront /target/libgit2-bindings/c --config ../crustify/wavefront/configs/wavefront-config.json \
  schedule --api-headers-only --max-types 2 --min-fields 20 --max-syms 25 --max-loc 500 \
  --output /target/libgit2-bindings/crustify/tmp/api-schedule.json
```

Through `git_remote_callbacks.resolve_url`, the same schedule also holds the deprecated
callback `git_url_resolve_cb`, as a unit no module claims (wavefront warns: `1 scheduled
unit(s) belong to no module ... git_url_resolve_cb (include/git2/remote.h)`).
`modules.json` already leaves it out of scope, with the other deprecated declarations.

The landed binding of `git_blob_filter_options` (wave 28) anchors `reserved`, not
`commit_id` (`src/libgit2/src/libgit2/blob.rs:956`, `` /// Wraps `fld::git_blob_filter_options.reserved` ``),
since the `-sys` bindings are generated with `GIT_DEPRECATE_HARD` defined.

The fields were found by scanning the installed headers for declarations inside
`GIT_DEPRECATE_HARD`-only branches: `crustify/tmp/planner-run-3/deprecated_fields.py`
(planner run 3's working directory, not tracked).

## Edit and what it moves

Planner run 3 re-plans every wave, as its instructions ask. In its plan the deprecated
anchors are dropped from `git_attr_options`, `git_blob_filter_options`,
`git_filter_options`, `git_rebase_options` and `git_remote_callbacks`, and
`git_url_resolve_cb` is not scheduled; `git_remote_callbacks` keeps `reserved` and
`reserved_update_tips`. No other item, wave or batch moves for it: wavefront's schedule
places `git_url_resolve_cb` in a symbol batch of `src/libgit2/remote.c`'s second wave,
which run 3 keeps without it.

## Questions for the user

None.
