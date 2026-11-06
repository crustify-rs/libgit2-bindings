# `global.rs` overstated what an unbalanced C shutdown does, and under-documented its errors

- **items**: `git_libgit2_init`, `git_libgit2_shutdown` (symbols),
  `crustify/rust/libgit2/libgit2/core/global.rs`
- **affected**: the module documentation, `init`'s and `Runtime::shutdown`'s
  failure documentation, `InitError::Failed` and `ShutdownError`
- **affected commit**: `7fc5171aa` (libgit2/core wave 0 batch 8; reviewed at `ab5de9104`)
- **class**: documentation defect (no behaviour change)

## Finding

The module doc said that a C caller calling `git_libgit2_shutdown` unbalanced drives
the count negative, "after which libgit2 never sets itself up again". That is not what
`git_runtime_init` does (`src/util/runtime.c:111`): it runs `init_common` whenever
`git_atomic32_inc(&init_count)` returns 1, from whatever value. From -1, the next
`git_libgit2_init` returns 0 and sets nothing up, and the one after returns 1 and sets
libgit2 up again. The new unit test
`an_unbalanced_c_shutdown_fails_the_next_init` shows it: after a raw shutdown to -1
and `init()`'s refusal, a raw `git_libgit2_init` answers 1 and `GIT_OPT_GET_USER_AGENT`
works.

Two consequences of that scenario on the wrappers were undocumented:

- `init()` treats the 0 that `git_libgit2_init` returns on the way back (a reference
  taken, nothing set up) as `InitError::Failed`, and poisons every later `init()`; the
  doc of `init` and of `InitError::Failed` named only a mutex failure or a subsystem
  that could not be set up.
- `Runtime::shutdown` claimed to fail "only when libgit2 cannot take or release its
  initialisation mutex", with the reference possibly staying counted. It also fails
  when an unbalanced C shutdown had already released the reference's count, so that
  `git_libgit2_shutdown` reports -1; the reference is released then. The new unit
  test `an_unbalanced_c_shutdown_fails_the_runtimes_release` shows it.

Both behaviours are the right conservative answers (a count driven negative by C is
C's misuse, and `init` refusing after a 0 keeps every later wrapper off
half-initialised state), so only the documentation changed.

## Fix

The module doc, `init`'s doc, `InitError::Failed`, `Runtime::shutdown`'s doc and
`ShutdownError` now describe the unbalanced-shutdown case as libgit2 implements it,
with the two unit tests above as evidence.
