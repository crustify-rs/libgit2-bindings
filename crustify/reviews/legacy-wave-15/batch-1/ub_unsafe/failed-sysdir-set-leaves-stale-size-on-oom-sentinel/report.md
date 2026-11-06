# A failed system-directory copy leaves the directory reading past `git_str__oom`

- **item**: `git_libgit2_opts` (symbol), `crustify/rust/libgit2/libgit2/core/common.rs`
- **affected**: `set_search_path`, `set_template_path`, `set_homedir` (the
  `GIT_OPT_SET_SEARCH_PATH`, `GIT_OPT_SET_TEMPLATE_PATH` and `GIT_OPT_SET_HOMEDIR`
  wrappers), and through them the safe getters `search_path`, `template_path` and
  `homedir` (`GIT_OPT_GET_*`) called next
- **affected commit**: `7fc5171aa` (libgit2/core wave 0 batch 8; reviewed at `ab5de9104`)
- **class**: UB reachable through the unsafe API within its contract — a read past the
  end of a one-byte global (AddressSanitizer, global-buffer-overflow), performed by C
  under a *safe* getter
- **reproducer**: `src/main.rs` (ASan; run as described in its header; aborts with the
  trace below on the affected commit, exits 0 after the fix). Its two `unsafe` calls
  honour their contracts: the allocator is installed before libgit2 initialises and is a
  libc family, and no other thread runs.

## Finding

The three setters hand their path to `git_sysdir_set` (`src/libgit2/sysdir.c:488`),
which copies it over the directory's current value in place with `git_str_sets`. When
the copy has to grow the buffer and the allocation is refused, `git_str_try_grow`
(`src/util/str.c:36`) has already freed the old string; it then points the buffer at
libgit2's one-byte sentinel `git_str__oom` and returns, leaving the buffer's `size`
(and `asize`) as they were. `git_sysdir_set` sees `git_str_oom()` and returns -1, and
the wrappers reported that error correctly — but the directory is now
`{ ptr: git_str__oom, size: <old length>, asize: <old capacity> }`.

The next `GIT_OPT_GET_SEARCH_PATH` / `_TEMPLATE_PATH` / `_HOMEDIR` copies the
directory with `git_str_put(&str, tmp->ptr, tmp->size)` (`src/libgit2/settings.c:148`,
`:189`, `:435`): `size` bytes from a one-byte static. ASan, on the reproducer (a
64-byte global search path, then a 4 KiB copy refused):

```
==…==ERROR: AddressSanitizer: global-buffer-overflow on address 0x7c17f99033e1
READ of size 64 at 0x7c17f99033e1 thread T0
    #0 __asan_memmove
    #1 git_str_put /target/src/util/str.c:206:3
    #2 git_libgit2_opts /target/src/libgit2/settings.c:148:17
    #3 libgit2::libgit2::core::common::search_path::{closure#0} crustify/rust/libgit2/libgit2/core/common.rs:1025:18
    #4 libgit2::libgit2::core::common::buf_string::<…> crustify/rust/libgit2/libgit2/core/common.rs:884:14
    #5 libgit2::libgit2::core::common::search_path crustify/rust/libgit2/libgit2/core/common.rs:1020:5
    #6 failed_sysdir_set_leaves_stale_size_on_oom_sentinel::main src/main.rs:95:17
0x7c17f99033e1 is located 0 bytes after global variable 'git_str__oom' defined in '/target/src/util/str.c:17' (0x7c17f99033e0) of size 1
SUMMARY: AddressSanitizer: global-buffer-overflow in __asan_memmove
```

A `$PATH`-expanding path fails differently (the scratch `merge` buffer is what cannot
grow, and `git_str_swap` moves it into the directory with whatever `size` it had reached)
but ends in the same state. The batch's `allocation_failures_report_errors` suite only
refused allocations for *getters* and string options, never for a system-directory
setter, so the sanitizers never saw it. libgit2 itself has the same defect (its own
`git_sysdir_set` callers never read the directory back after a failure), and besides
the over-read it marks the directory out of memory for good: every later plain copy
into it is refused by `ENSURE_SIZE` before allocating, with no error set.

## Fix

The three setters now route their status through `sysdir_status`, which, when
`git_sysdir_set` failed, repairs the directory with two allocation-free sets through
the same key before returning the error: the empty path (`git_str_sets(buf, "")` →
`git_str_clear`, zeroing `size`), then `"$PATH"` alone (`git_sysdir_set` joins nothing
around an empty old value and swaps its fresh `GIT_STR_INIT` scratch buffer in; the
sentinel goes to the scratch buffer, which `git_str_dispose` releases without freeing).
The directory is then a live, empty string: readable, and accepting the next copy.
This is a documented divergence from C, whose directory would be unreadable and dead;
the setters' docs say the directory is empty after a failed copy, libgit2 having freed
its previous value before finding out.

Regressions: `tests/ub_unsafe/common.rs::failed_sysdir_copies_leave_empty_readable_directories`
(ASan; every level of the three keys, a plain and a `$PATH`-expanding refused copy
each, over the initial guess and over set values; reads empty while memory is short
and after, then takes plain and expanding copies again; fails with
`global-buffer-overflow` with the repair removed) and
`core/common.rs::unit_tests::failed_sysdir_copies_are_repaired` (the corrected
behaviour on `GIT_OPT_SET_TEMPLATE_PATH`, and `sysdir_status`'s pass-through of 0).
