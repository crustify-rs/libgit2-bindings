# The matched-path workdir tests raced libgit2's first filter load (TSan)

- **item**: `git_index_matched_path_cb` (callback). The two inline tests
  `index.rs::unit_tests::matched_path::foreign_scan_payload_and_panic_cleanup` and
  `::workdir_scan_skip_abort_and_removed_paths` came with its wrapper (libgit2/index wave
  0 batch 8, `6c09dba0d`).
- **affected commit**: `ab3dabdcc` (the review wave's base)
- **class**: test defect. It is a known libgit2 race, and the wrapper is not involved.

## Finding

Both tests drive `git_index_add_all` / `git_index_update_all` through
`MatchedPathCallback::scan_with`. For each accepted path, `apply_each_file`
(`src/libgit2/index.c:3764`) calls `git_index_add_bypath`, which hashes the file through
`git_blob__create_from_paths` → `git_filter_list_load`. On a process's first filter-list
loads, `git_filter_list__load` writes each built-in filter's `initialized` flag
(`src/libgit2/filter.c:344`) under only the registry's *read* lock, and other loads read
that flag (`:569`). The test harness ran the two tests on parallel threads in one process,
so each was the other's first load.

`cargo-variant.sh tsan test -p libgit2 --lib --test equiv --test ub_safe -- index::` on
`ab3dabdcc` plus this batch's other changes reported it and exited 66. `tsan.log` here holds
the report, with worktree and toolchain paths shortened:

```
WARNING: ThreadSanitizer: data race (pid=1733287)
  Read of size 4 at 0x721800009014 by thread T9 (mutexes: read M0):
    #0 git_filter_list__load src/libgit2/filter.c:569
    #1 git_filter_list_load src/libgit2/filter.c:640
    #2 git_blob__create_from_paths src/libgit2/blob.c:233
    #3 git_index_add_bypath src/libgit2/index.c:1043
    #4 apply_each_file src/libgit2/index.c:3764
    ...
    #8 git_index_add_all src/libgit2/index.c:3717
    #9 …::matched_path::foreign_scan_payload_and_panic_cleanup::{closure#3} index.rs:1902
  Previous write of size 4 at 0x721800009014 by thread T12 (mutexes: read M0):
    #0 git_filter_list__load src/libgit2/filter.c:344
    ...
    #9 …::matched_path::workdir_scan_skip_abort_and_removed_paths::{closure#1} index.rs:1976
```

The same race is handled the same way in the filter, blob and mailmap suites (an
isolated child process per list-loading test). These two tests were missed.

## Fix

Both test bodies now run through the module's existing `isolated` helper, which runs
the test alone (`--exact --test-threads=1`) in a child process of the same binary and
asserts that it passed. Neither the assertions nor the scenario changed. A doc line on
each test says why.

After the change, the same TSan run is clean: 34 + 26 + 38 tests, exit 0. Plain and ASan
runs of the workspace pass too.
