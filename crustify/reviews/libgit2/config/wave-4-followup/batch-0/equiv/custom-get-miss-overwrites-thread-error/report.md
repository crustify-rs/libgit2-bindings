# A `CustomBackend` miss overwrote the thread's last error

- **item**: `git_config_backend` (type): the `get` trampoline behind `CustomBackend::get`
  (`Custom::get`), and `BackendMut::get` (`Field: git_config_backend.get`),
  `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `df0deef13` (the review's base; landed by `a66691e90`, libgit2/config
  wave 4 follow-up)
- **class**: equivalence defect: the thread's error state after a call, and the error a
  safe getter reports, differ from a C backend serving the same store

## Finding

When a store's `get` answered `None`, the trampoline returned `GIT_ENOTFOUND` and also
made "config value '<key>' was not found" the thread's last libgit2 error
(`not_found(key)` → `ConfigError::raise`). libgit2's own backends report a miss without an
error: `git_config_list_get` (`src/libgit2/config_list.c:198`) just returns
`GIT_ENOTFOUND`. `get_entry` (`src/libgit2/config.c:790-830`) relies on that. It
describes a miss itself, and only when its caller wants an error (`GET_ALL_ERRORS`,
`:820`). For `GET_NO_MISSING` and `GET_NO_ERRORS` lookups
(`git_config__lookup_entry`, the `*_force` getters), and whenever a lower-priority backend
holds the key, it returns 0 and leaves the thread's error alone.

So a Rust backend at a higher level that lacked a key left a fabricated error behind a
successful lookup. The safe API depends on that error state. `ConfigRef::get_i64` clears
the error, calls `git_config_get_int64`, and if C fails without describing the failure
(its only silent failure is text after a `k`/`m`/`g` suffix, `config.c:1451`), it reports
"failed to parse configuration value as an integer". With the Rust backend above the
level that holds `1kb`, the stale miss was still there, so `get_i64` told the caller the
key **was not found**, though it exists with a malformed value.

## Reproducer

`src/main.rs` builds two configs over the same two stores. The higher level (`APP`)
lacks `core.bigfilethreshold`, and the lower level (`LOCAL`) holds `1kb`. In one config
each level is a C-ABI backend written as libgit2's are (a miss returns `GIT_ENOTFOUND`
and sets no error). In the other, each level is the same store as a `CustomBackend`. The
program clears the error, drives both configs through `git_config_get_entry` and
`git_config_get_int64`, and compares the return and `git_error_last()` after each call.
It then compares the safe `ConfigRef::get_i64` errors.

```sh
LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run --offline
```

On `df0deef13` (exit 101):

```
git_config_get_entry  c    (0, (0, "no error"))
git_config_get_entry  rust (0, (7, "config value 'core.bigfilethreshold' was not found"))
git_config_get_int64  c    (-1, (0, "no error"))
git_config_get_int64  rust (-1, (7, "config value 'core.bigfilethreshold' was not found"))
ConfigRef::get_i64    c    (-1, 7, "failed to parse configuration value as an integer")
ConfigRef::get_i64    rust (-1, 7, "config value 'core.bigfilethreshold' was not found")

thread 'main' panicked at src/main.rs:222:5:
assertion `left == right` failed: git_config_get_entry leaves a different error
  left: (0, (7, "config value 'core.bigfilethreshold' was not found"))
 right: (0, (0, "no error"))
```

On the fixed tree, every pair matches and the program exits 0.

## Fix

- `Custom::get` returns `GIT_ENOTFOUND` without setting an error for a miss, as
  libgit2's backends do. A NULL key gets the same treatment. A get refused for
  reentry still returns `GIT_ENOTFOUND`, but it raises the refusal ("config backend
  reentered from its own callback"), which has no C counterpart. That message used to
  read as an ordinary miss.
- `BackendMut::get`, which dispatches the slot directly and so has no `get_entry` to
  describe the miss, now sets "config value '<key>' was not found" itself when the slot
  returns `GIT_ENOTFOUND`. It copies the key byte for byte, as `config_error_notfound`
  does (the old message used `to_string_lossy`). For any backend, the miss no longer
  carries a stale thread error.
- The `CustomBackend` docs say that a miss leaves the thread's error alone.

Regressions:

- `tests/equiv/sys_config.rs`,
  `custom_backends::layered_misses_leave_errors_like_a_c_backend`: the reproducer's two
  levels on both sides. It compares the return, the thread's last error (after success as
  well as failure) and the stores' calls for `git_config_get_entry` on a key the lower
  level holds, one the higher holds, and a missing key. It does the same for the silent
  `git_config_get_int64` failure and for `git_config_get_string` on a snapshot (the
  read-only answer kept by the lower level). Finally it compares the safe
  `ConfigRef::get_i64` errors over both configs, then the releases. It fails on
  `df0deef13` at the first step.
- `unit_tests::custom::custom_default_slots_refuse_and_name_themselves` now seeds an
  unrelated error first. It checks that `BackendMut::get` describes a miss itself, with a
  non-UTF-8 key containing `%s` kept verbatim.
