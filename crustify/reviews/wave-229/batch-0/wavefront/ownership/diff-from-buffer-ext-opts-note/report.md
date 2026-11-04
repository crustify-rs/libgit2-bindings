# `git_diff_from_buffer_ext`'s `opts` note describes another options type

- **entity**: `git_diff_from_buffer_ext` (`src/libgit2/diff_parse.c`), parameter `opts`
- **submitted by**: `8f238f339` ("wrap diff ownership, constructors and borrowed deltas",
  2026-10-11)
- **relied on by**: wave 228 batch 0 (`0fcf482ab`), whose `Diff::from_buffer_with_options`
  passes `DiffParseOptions` through this parameter
- **resubmitted**: `wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json c
  query symbols --name git_diff_from_buffer_ext --update updates/git_diff_from_buffer_ext.json`,
  from the repository root, with this directory's `updates/` file

## Previous finding

```sh
wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json c \
  query symbols --name git_diff_from_buffer_ext
```

At the wave tip (`9872835e0`), `findings.ptr_args.opts.ptr` was:

```json
{"scalar": {"by_val": true}, "array": null, "string": false, "owned": false,
 "borrowed": {"lifetime": "arg:opts"}, "nullable": true, "mutable": false,
 "note": "Nullable default options. Generator copies struct; pathspec/prefix contents are copied into pool but copied raw options include callbacks/payload and borrowed pointers. Wrapper conservatively requires nested referents outlive result. Parse options only read scalar version/oid_type during call."}
```

## Why it was wrong

The properties are right. The note is not, and the note is the reasoning a binder reads:

- `git_diff_parse_options` (`c/include/git2/diff.h:1325-1330`) is `{ unsigned int version;
  git_oid_t oid_type; }`. It has no pathspec, prefix, callback, payload or any other
  pointer, so "nested referents" that must "outlive result" do not exist. Those sentences
  describe `git_diff_options`, as consumed by the tree/index diff generators, and were
  apparently copied from that parameter's note.
- "only read scalar version/oid_type": `git_diff_from_buffer_ext`
  (`c/src/libgit2/diff_parse.c:76-125`) reads only `opts->oid_type`
  (`:90-91`). It never reads or checks `version`. Wave 228 batch 0 depends on exactly this
  when it lets `DiffParseOptions` store any version.
- The note says nothing about the hazard the binding has to handle: `oid_type` is not
  validated. A value `git_oid_t` does not name gives `git_oid_hexsize` 0
  (`c/src/libgit2/oid.h:39-49`).

A binder trusting the old note would add a lifetime parameter or keepalive the options do not
need. It could also miss that the oid type must be restricted. The wave-228 binding got
this right from the C, not from the note.

## Corrected finding

The same properties, with a note that states what the C does: NULL or 0 selects SHA-1,
only `oid_type` is read, during the call, nothing is retained, no pointer is held, and
`oid_type` is unchecked. See `updates/git_diff_from_buffer_ext.json`. The Rust needs no
change: `DiffParseOptions` already holds only 0 or a named `OidType`, and the call borrows
a local copy for its duration.
