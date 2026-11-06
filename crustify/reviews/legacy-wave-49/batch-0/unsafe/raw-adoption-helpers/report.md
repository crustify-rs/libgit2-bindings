# Private `unsafe fn`s adopt raw constructor results where a safe helper suffices

- **items**: `git_diff` (`diff/diff.rs`, `from_raw_result`, commit `5be7c5199`) and
  `git_pathspec_match_list` (`index/pathspec.rs`, `finish`, commit `a8fb7fc05`)
- **class**: unsafe (`crustify scan-unsafe` smells)

## Finding

Both modules funnelled every constructor through a private `unsafe fn` taking the raw
output pointer:

```rust
unsafe fn from_raw_result<'r>(out: *mut ffi::git_diff, rc: i32) -> Result<DiffBox<'r>, DiffError>
unsafe fn finish<'diff>(rc: i32, out: *mut ffi::git_pathspec_match_list) -> Result<MatchListBox<'diff>, RepositoryError>
```

`crustify scan-unsafe --sites all` counts them as two `unsafe_fns_priv_smell` and two
`raw_ptr_priv_smell` sites, the only smells in the workset apart from the documented
`generate_with_options` cut. Each call site then needed an `unsafe` block to call a
helper whose only unsafe step was `from_c`. The sibling `index.rs` already uses the
smell-free shape: the call site adopts with `IndexBox::from_c(out)` and passes the
`Option` to the safe `constructed(rc, index)`.

That shape is valid only if every constructor leaves its output NULL on failure.
Verified in C:

- all eight diff constructors set `*out = NULL` first and assign it only on success.
  A progress callback's positive code also leaves it NULL (`diff_generate.c`, the
  `diff = NULL` path), as does `git_diff_tree_to_workdir_with_index`'s
  `if (error) ... d1 = NULL`;
- the four matchers either return before touching `*out` (Rust passes NULL) or reset it
  on failure (`pathspec.c:419-420`, `:501`, `:607-608`, `:672`).

## Fix

`diff.rs` and `pathspec.rs` each get a safe `constructed(rc, Option<Box>)` helper with
the same results: the exact nonzero code for diffs, negative codes for match lists,
and the NULL-on-success fallbacks unchanged. Each constructor adopts its output in the
SAFETY-commented block that already justified the result. The scan now reports no
`unsafe_fns_priv_smell` or `raw_ptr_priv_smell` site in either file. The full diff,
pathspec and index suites pass under ASan/UBSan (LSan on); the constructor-failure
paths include parse errors, cancelled progress callbacks, `GIT_ENOTFOUND` from
`NO_MATCH_ERROR`, and the injected allocation failures.
