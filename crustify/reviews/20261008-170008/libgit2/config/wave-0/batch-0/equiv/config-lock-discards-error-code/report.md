# `ConfigMut::lock` drops the code `git_config_lock` returns

- **item**: `git_transaction` (type; its constructor `git_config_lock`),
  `crustify/rust/libgit2/libgit2/refs/transaction.rs` (`ConfigTransaction::lock`) and
  `crustify/rust/libgit2/libgit2/config/config.rs` (`ConfigMut::lock`)
- **affected**: `ConfigMut::lock`, every failure
- **affected commit**: `95f5f77cb` (libgit2/config wave 0 batch 1; reviewed at the
  review-wave base `b83f28c34`)
- **class**: equivalence — return values and errors: C returns `GIT_EREADONLY` (-40) or
  `GIT_ELOCKED` (-14) (or a custom backend's own code), the safe wrapper returned an
  `ErrorBuf`, which holds the class and message but no code
- **reproducer**: `src/main.rs` (`LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run`
  from this directory; exits 101 on the affected commit, 0 after the fix)

## Finding

`git_config_lock` (`src/libgit2/config.c:1306`) tells its failures apart by return
code:

```c
	if ((instance = get_writer_instance(config)) == NULL) {
		git_error_set(GIT_ERROR_CONFIG, "cannot lock: the configuration is read-only");
		return GIT_EREADONLY;
	}

	if ((error = instance->backend->lock(instance->backend)) < 0 ||
	    (error = git_transaction_config_new(out, config, instance)) < 0)
		return error;
```

The file backend's `lock` returns `GIT_ELOCKED` from `git_filebuf_open` when another
writer holds `<file>.lock`, the case callers retry on. A custom backend's `lock` can return
any code, with any error class. Batch 1 wrapped the constructor as

```rust
pub(crate) fn lock(mut config: ConfigMut<'c, 'r>) -> Result<Self, ErrorBuf> {
    ...
    if rc < 0 {
        return Err(last());
    }
```

so `rc` was dropped and a Rust caller could not see the code C reports. Other wrappers in
the crate that report codes keep them (`RepositoryError`, `WorktreeError`), and earlier
reviews compared failures "by code, class and message". The batch's equivalence test
(`tests/equiv/transaction.rs::config_construct_stage_abort_and_relock`) asserted C's code
but compared only class and message on the Rust side, so it missed this.

Comparison from the reproducer on the affected commit. Each side has its own config: empty,
then with a file backend whose lock another writer holds, then with that lock released.
The Rust code is read from the error's public `Debug` output:

```
empty config:
  C:    (Some("ReadOnly"), 7, "cannot lock: the configuration is read-only")
  Rust: (None, 7, "cannot lock: the configuration is read-only")
lock held by another writer:
  C:    (Some("Locked"), 2, "failed to lock file '<path>.lock' for writing: ")
  Rust: (None, 2, "failed to lock file '<path>.lock' for writing: ")
lock free:
  C:    (None, 0, "")
  Rust: (None, 0, "")
the safe wrapper's error lost the code git_config_lock returned
```

(exit status 101).

## Fix

`config.rs` gains `ConfigError`, built like `RepositoryError`: the returned code plus the
thread's last error (`code()`, `raw_code()`, `class()`, `raw_class()`, `message()`,
`error()`, `From<ConfigError> for ErrorBuf`, a `Debug` that names the code, and `Display`
showing the message). `ConfigMut::lock` and `ConfigTransaction::lock` now return
`Result<_, ConfigError>`, built from `rc`. The unpinned-runtime return and the unreachable
NULL-owner return report `GIT_ERROR`. Later config wrappers can reuse the type, for
example `GIT_ENOTFOUND` from the getters. After the fix the reproducer prints matching
records for all three steps and exits 0.

## Regression tests

- `tests/equiv/transaction.rs::config_lock_codes_match_c`: each side runs seven steps on
  its own config (empty, held lock, free, a second higher-priority `APP` file that becomes
  the writer, its lock held, free). Each step compares code, class, message and which
  backend files are locked while the transaction lives.
- `tests/equiv/transaction.rs::config_construct_stage_abort_and_relock` now compares
  `raw_code()` with C's code for its read-only and locked steps.
- `config.rs::unit_tests::config_errors_keep_codes` covers the accessors, `Debug`,
  `Display`, the conversion, non-negative codes and codes the header does not name.
