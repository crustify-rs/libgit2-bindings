# Two diff generators report a refused options version as `1`

- **item**: `git_diff` (type), `Diff::generate_with_options`,
  `crustify/rust/libgit2/libgit2/diff/diff.rs`
- **affected commit**: `5be7c5199` (libgit2/index wave 0 batch 0)
- **class**: misc (a confirmed C defect passed through to Rust)

## Finding

`git_diff_tree_to_workdir` and `git_diff_index_to_index` parenthesise their first
check wrongly (`src/libgit2/diff_generate.c:1532` and `:1604`):

```c
if ((error = diff_prepare_iterator_opts(&prefix, &a_opts, 0,
                                        &b_opts, GIT_ITERATOR_DONT_AUTOEXPAND, opts) < 0) ||
```

`error` receives the comparison, not the result. When `diff_prepare_iterator_opts`
refuses the options, both functions return `1` with a NULL output, together with the
`GIT_ERROR_INVALID` "invalid version N on git_diff_options" error. The other four
generators return `-1` for the same options. In practice the version check is the only
failure: the function's allocation check tests `prefix` (the `char **` argument), which
is never NULL.

The safe default-options constructors pass NULL options, which skips the check. The
unsafe `Diff::generate_with_options` passed the options through, and `DiffError`
preserves positive codes because a progress callback's cancellation code is positive.
So a caller got `Err` with `raw_code() == 1` and `code() == None`. It could not tell
this from a callback that returned `1`, and the four other generators reported the same
input as `-1`.

Observed on the affected commit with `options.version = 2` (ASan/UBSan build, one line
per `DiffInputs` variant, in the order of the new `ub_unsafe` test):

```
v=2 t=0 -> -1 Some(Invalid) "invalid version 2 on git_diff_options"   Trees
v=2 t=0 -> -1 Some(Invalid) "invalid version 2 on git_diff_options"   TreeToIndex
v=2 t=0 -> -1 Some(Invalid) "invalid version 2 on git_diff_options"   IndexToWorkdir
v=2 t=0 -> 1 Some(Invalid) "invalid version 2 on git_diff_options"    TreeToWorkdir
thread 'diff::generate_with_options_checked_fields' panicked at libgit2/tests/ub_unsafe/diff.rs:171:13:
assertion `left == right` failed
  left: 1
 right: -1
```

## Fix

`generate_with_options` checks the version first, accepting what libgit2 accepts
(`1..=GIT_DIFF_OPTIONS_VERSION`, `git_error__check_version`). It refuses anything else as
the correct generators do: code `-1` and the same `GIT_ERROR_INVALID` message, with the
unsigned version formatted as `%d` formats it. The C code is unchanged (wrap campaign),
and the divergence is documented on the function.

Regressions:

- `unit_tests::refused_options_version_is_negative_for_every_generator` (corrected
  behaviour). For versions 0, 2 and `u32::MAX`, `TreeToWorkdir` and `Indices` return
  exactly the code and error that C's `git_diff_tree_to_tree` returns for the same
  options.
- `ub_unsafe::diff::generate_with_options_checked_fields`. All six generators return `-1`
  / `GIT_ERROR_INVALID` for versions 2 and 0, and for an `oid_type` that does not match
  the repository.
