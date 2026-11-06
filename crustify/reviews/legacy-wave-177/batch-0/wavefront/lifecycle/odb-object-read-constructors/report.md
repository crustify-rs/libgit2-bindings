# `git_odb_object`'s lifecycle misses its constructors, `git_odb_read` and `git_odb_read_prefix`

- **type**: `git_odb_object` (`include/git2/odb_types.h`; private layout in
  `src/libgit2/odb.h`)
- **submitted by**: wave 176 batch 2 (`b043039e2`), which recorded `git_odb_object_free` as
  the dropper and `git_odb_object_dup` as the up-ref cloner, and bound both readers
- **resubmitted**: `query symbols --name git_odb_read --update -` and
  `query symbols --name git_odb_read_prefix --update -`, each with
  `{"lifetime": {"for": "out", "is_dropper": false, "is_disposer": false, "is_cloner": null,
  "is_constructor": {"alloc": true, "init": false}}}`

## Previous lifecycle

```sh
wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json <repo>/c \
  query symbols --lifetime-for git_odb_object
```

At the wave tip this answered `dropped_by: [git_odb_object_free]`,
`cloned_by: [git_odb_object_dup (upref)]` and `constructed_by: []`. Both readers' records
had `lifetime: null`.

## Why it was incomplete

`docs/type-lifecycle.md` records as an `alloc` constructor any routine that hands the caller
a new object it owns through a `T **` out-parameter. Both readers do so for
`git_odb_object`:

- `git_odb_read` (`c/src/libgit2/odb.c:1155`) stores in `*out` either a cache hit, which
  `git_cache_get_raw` returns with its count raised, or a new object that `odb_read_1` allocates
  (`odb_object__alloc`, `odb.c:108`) and stores in the cache. In both cases the caller owns one counted
  reference and releases it with `git_odb_object_free`.
- `git_odb_read_prefix` (`odb.c:1306`) does the same through `git_cache_get_raw` and
  `read_prefix_1`.

These are the only routines of the public API that produce a `git_odb_object`. The batch's
own `out` notes say so ("on success receives one counted reference to the object …, owned by
the caller and released with git_odb_object_free"). Batch 1 recorded the analogous readers of
`git_odb_stream` (`git_odb_open_rstream`, `git_odb_open_wstream`) as `alloc` constructors.
The Rust adopts both results as `OdbObjectArc` (`read_out`), so the binding is unaffected.
Only the record is incomplete.

## Corrected lifecycle

Both readers now carry `is_constructor: {alloc: true}` for `out`, whose block is already
owned at depth 2 and mutable, as the rule for `alloc` constructors requires. The query above
now answers `constructed_by: [git_odb_read, git_odb_read_prefix]`. No Rust representation is
missing: `OdbObjectArc` is built only from these two routines' results.
