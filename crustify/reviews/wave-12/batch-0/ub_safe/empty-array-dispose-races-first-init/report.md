# Dropping an empty `OidArrayVal` races the process's first `git_libgit2_init`

- **item**: `git_oidarray` (type), `crustify/rust/libgit2/libgit2/oid/oidarray.rs`
- **affected**: `OidArrayDispose` (the `CDispose` policy of `OidArrayVal`), hence the
  drop of every empty array: `OidArray::new()`, `OidArray::from_slice(&[])`, an array
  after `take_ids` or `set_ids(OdbDataVec::empty())`
- **affected commit**: `b70e52aae` (libgit2/oid wave 1 batch 0; reviewed at `8cb33ad6b`)
- **class**: UB reachable from the safe API — a data race (ThreadSanitizer), between a
  C read in `git__free` and the C write of `git_stdalloc_init_allocator`
- **reproducer**: `src/main.rs` (`#![forbid(unsafe_code)]`; TSan, run as described in
  its header; exit 66 on the affected commit, 0 after the fix)

## Finding

`OidArrayDispose` was `impl_cdispose!(OidArrayDispose, OidArray, ffi::git_oidarray_dispose)`,
so every `OidArrayVal` drop called `git_oidarray_dispose`, which is `git__free(arr->ids)`,
which is `git__allocator.gfree(ptr)` — a plain read of a global function pointer
(`src/util/alloc.h:39`). The policy's comment held that the empty array is fine to
dispose "even before `git_libgit2_init`, when the allocator is `failalloc`", which is
true on one thread: `git__allocator` is statically `failalloc`, whose free is a no-op.

It is not true concurrently. The first `git_libgit2_init` of the process writes
`git__allocator` (`git_allocator_global_init` → `git_stdalloc_init_allocator`,
`stdalloc.c:35`), under libgit2's init mutex, which `git__free` never takes; a
`GIT_OPT_SET_ALLOCATOR` before initialisation `memcpy`s it the same way. Every
non-empty array is ordered after that write: its buffer came from libgit2's allocator,
and the allocation pinned the runtime (`runtime_pinned()`, a `OnceLock`) first. An
empty array allocated nothing, so nothing orders its drop after another thread's
initialisation — and the safe API lets one thread drop empty arrays while another
thread's first `from_slice` initialises the library. Two threads, no `unsafe`:

```
WARNING: ThreadSanitizer: data race (pid=1166241)
  Write of size 8 at 0x7ffff7fb85b8 by thread T2 (mutexes: write M0):
    #0 git_stdalloc_init_allocator /target/src/util/allocators/stdalloc.c:35
    #1 setup_default_allocator /target/src/util/alloc.c:95
    #2 git_allocator_global_init /target/src/util/alloc.c:108 (libgit2.so.1.9+0x1af68)
    #3 init_common /target/src/util/runtime.c:23
    #4 git_runtime_init /target/src/util/runtime.c:120 (libgit2.so.1.9+0x2d1aa)
    #5 git_libgit2_init /target/src/libgit2/libgit2.c:61
    #15 libgit2::libgit2::util::sys::errors::runtime_pinned libgit2/libgit2/util/sys/errors.rs:56
    #22 <libgit2::libgit2::oid::oidarray::OidArray>::from_slice libgit2/libgit2/oid/oidarray.rs:116
    #23 empty_array_dispose_races_first_init::main::{closure#1} src/main.rs

  Previous read of size 8 at 0x7ffff7fb85b8 by thread T1:
    #0 git__free /target/src/util/alloc.h:39
    #1 git_oidarray_dispose /target/src/libgit2/oidarray.c:15 (libgit2.so.1.9+0xd590d)
    #2 <libgit2::libgit2::oid::oidarray::OidArrayDispose as ffibox::traits::CDispose<libgit2::libgit2::oid::oidarray::OidArray>>::c_dispose ffibox/mac
    #3 <ffibox::refs::CVal<libgit2::libgit2::oid::oidarray::OidArray, libgit2::libgit2::oid::oidarray::OidArrayDispose> as core::ops::drop::Drop>::dro
    #4 core::ptr::drop_glue::<ffibox::refs::CVal<libgit2::libgit2::oid::oidarray::OidArray, libgit2::libgit2::oid::oidarray::OidArrayDispose>> <std>
    #5 core::mem::drop::<ffibox::refs::CVal<libgit2::libgit2::oid::oidarray::OidArray, libgit2::libgit2::oid::oidarray::OidArrayDispose>> <std>
    #6 empty_array_dispose_races_first_init::main::{closure#0} src/main.rs

  ...
SUMMARY: ThreadSanitizer: data race /target/src/util/allocators/stdalloc.c:35 in git_stdalloc_init_allocator
```

The wrapper's own `tests/ub_safe/oidarray.rs::views_stay_in_bounds` hit it under TSan
in the reviewed tree (its `n = 0` iteration drops an empty array while another test's
first allocation initialises libgit2), which is how the candidate surfaced.

## Fix

`OidArrayDispose` is now a hand-written `CDispose` (`/// Handwritten`) that reads
`ids` and returns without calling C when it is NULL: an empty array owns nothing, and
`git_oidarray_dispose` would only have passed NULL to `git__free`. A non-NULL `ids`
is, per the handles' invariant, a buffer from libgit2's allocator, whose allocation
initialised the library, so that path is ordered after the write and still goes
through `git_oidarray_dispose`. This is the rule ffibox's `CVec` already follows for a
NULL buffer, and the equivalence harness already documented the same divergence for
`set_ids` (`free(NULL)` is not recorded; the only observer would be a custom
`gfree` counting NULL calls). The module docs state the rule.

Regression tests:

- `tests/ub_safe/oidarray.rs::empty_array_drops_do_not_race_the_first_init` (TSan): in
  a child process where nothing has initialised libgit2, one thread drops empty arrays
  of every safe origin while another performs the process's first allocation. With the
  fix reverted (the early return removed) the child reports the race and the test
  fails; with it the whole oidarray suite is TSan-clean.
- `unit_tests::null_buffer_is_empty_and_disposes_without_c` (Miri): drives the policy
  on an empty array; Miri aborts on any foreign call, so passing proves the drop makes
  none.
