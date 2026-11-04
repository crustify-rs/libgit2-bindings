## Replace the fixture's stand-ins with the safe constructors (departure)

This batch is not in `schedule.json`: at the user's request it reviews the safe UB suite,
`tests/ub_safe/`, for one thing only. `tests/ub_safe/fixture.rs` opts out of the suite's
`unsafe_code` lint to stand in for safe constructors the crate did not have, and the
testing spec replaces each such fixture once its safe constructor exists. Many now do: of
the 51 FFI functions the fixture calls, these 23 have safe wrappers:
`git_blob_create_from_buffer`, `git_blob_create_from_stream`,
`git_blob_create_from_stream_commit`, `git_buf_dispose`, `git_error_set_str`,
`git_filter_list_free`, `git_filter_list_new`, `git_index_add`, `git_index_read_tree`,
`git_libgit2_init`, `git_libgit2_opts`, `git_object_type`, `git_odb_free`,
`git_odb_object_data`, `git_odb_object_free`, `git_odb_object_size`, `git_odb_read`,
`git_odb_write`, `git_oid_from_string`, `git_oid_tostr`, `git_reference_free`,
`git_repository_index`, `git_repository_odb`.

Replace every fixture use whose raw calls the safe API now covers with that API, in the
fixture and in the tests that call it, and delete what nothing uses any more. Leave the
fixtures whose raw calls have no safe counterpart yet; keep what still needs the raw form
for a reason the wrappers do not cover, and say why in the fixture's docs. Do not change
the crate under `src/`, and do not change what any test checks. File one `misc` report
listing what you replaced and what remains, run the suite's gates as a landing does, and
land.

## Wavefront configuration

The committed configurations under `crustify/wavefront/configs/` predate the installed
wavefront's schema, which rejects them (`out_of_scope` is no longer read). Query wavefront
through the campaign-wide configuration converted to the current schema instead:

```bash
wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json c query ...
```

Its `state_dir` resolves to your tree's `crustify/wavefront/`, so findings you submit with
`--update` land in your tree's ownership store as usual. Leave the committed
configurations as they are: a planner run brings them to the current schema.
