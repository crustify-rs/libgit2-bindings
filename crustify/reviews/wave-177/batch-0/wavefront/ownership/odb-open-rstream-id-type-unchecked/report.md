# `git_odb_open_rstream`'s `oid` finding omits that the backends take the id's type unchecked

- **item**: `git_odb_open_rstream`, `ptr_args.oid` (`src/libgit2/odb.c`)
- **submitted by**: wave 176 batch 1 (`3f43c106d`)
- **resubmitted**: `wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json
  <repo>/c query symbols --name git_odb_open_rstream --update <findings.json>` (the note
  only; every other facet of the block is unchanged)

## Previous finding

> Read for the call only.

The block's other facets (scalar by value, borrowed for the call, not nullable, not
mutable) are correct.

## Why it was incomplete

`git_odb_open_rstream` (`c/src/libgit2/odb.c:1563-1598`) hands `oid` to each backend's
`readstream` member without comparing its type with `db->options.oid_type`. The loose
backend's member, `loose_backend__readstream` (`c/src/libgit2/odb_loose.c:1037`), calls
`locate_object` → `object_file_name` (`odb_loose.c:82`). That function sizes the path for
the backend's id type, but `git_oid_pathfmt` writes the id's own hex size. The finding
recorded no obligation on the id's type, and the safe wrapper was written without one. The
other readers' findings (`git_odb_read`, `git_odb_read_prefix`, `git_odb_read_header`,
`git_odb_exists*`), submitted by batch 2, carry it. The result is the heap overflow filed as
`ub_safe/open-read-stream-formats-foreign-id-out-of-bounds`, with its ASan trace.

## Corrected finding

The note now says that the id is read for the call only and never retained, that it is
handed to each backend's `readstream` unchecked, and that its type must be the odb's: a
SHA-256 id in a SHA-1 odb overflows the loose backend's path buffer, and a SHA-1 or typeless
id in a SHA-256 odb leaves part of it unwritten. To check it:

```sh
wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json <repo>/c \
  query symbols --name git_odb_open_rstream
```
