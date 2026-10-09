# The `git_oidarray` note called disposing an empty array before init "harmless"

- **entity**: `git_oidarray` (type record, `_comment_agent`), submitted by libgit2/oid
  wave 1 batch 0 (`b70e52aae`)
- **class**: incomplete finding — a lifecycle property stated for one thread that does
  not hold across threads, and which the wrapper relied on

## Previous finding

> All-zero is the empty array and dispose accepts it (git__free(NULL)); before
> git_libgit2_init git__free is failalloc's no-op, so disposing an empty array pre-init
> is harmless.

## Evidence it is incomplete

`git__free` is `git__allocator.gfree(ptr)` (`src/util/alloc.h:39`), an unsynchronised
read of a global the first `git_libgit2_init` writes (`git_allocator_global_init` →
`git_stdalloc_init_allocator`, `src/util/allocators/stdalloc.c:35`, under the init
mutex that `git__free` does not take) and that `GIT_OPT_SET_ALLOCATOR` `memcpy`s. On
one thread the note is right; with the dispose on one thread and the first
initialisation on another it is a data race, which ThreadSanitizer reports — see
`../../ub_safe/empty-array-dispose-races-first-init/report.md` for the reproducer and
trace. The property matters precisely for the empty array: every other `git_oidarray`
holds a buffer from libgit2's allocator, whose allocation initialised the library and
so orders its later free; the empty array is the only one whose dispose can precede
initialisation, which is what made the wrapper pass it to C without a pin.

## Resubmitted

The `_comment_agent` of `git_oidarray` now records the race and the resulting rule
(an empty array is never handed to `git_oidarray_dispose`); the field records and
`git_oidarray_dispose`'s lifetime record were verified correct and left as they were.
