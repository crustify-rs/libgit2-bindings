# The `git_buf.ptr` note implied every public producer reuses a handed-in allocation

- **entity**: `git_buf` (type record, `ptr` field note and `_comment_agent`),
  submitted by the raw-lifetime-string review and libgit2/core wave 0 batch 0
  (`cfcd11d5c`)
- **class**: incomplete finding — one public filler departs from the described
  protocol

## Previous finding

> libgit2's producers (git_buf_fromstr) leave reserved = git_str asize, so reserved > 0
> means ptr is a git__malloc buffer of >= reserved bytes the struct owns, writable and
> reusable (git_buf_sanitize writes ptr[0], git_buf_tostr adopts it as git_str asize
> and may git__realloc it).

## Evidence it is incomplete

Of the 43 public routines that fill a `git_buf *out` (30 through `GIT_BUF_WRAP_PRIVATE`,
the rest by hand in `settings.c`, `commit.c`, `diff_print.c`, `email.c`, `midx.c`,
`worktree.c`), every one first moves the caller's struct through `git_buf_tostr` —
which reuses or reallocates a `reserved > 0` allocation and drops a `reserved == 0`
`ptr` — except `git_repository_discover` (`src/libgit2/repository.c:1308-1323`):

```c
	if ((error = find_repo(&paths, start_path, ceiling_dirs, flags)) == 0)
		error = git_buf_fromstr(out, &paths.gitdir);
	repo_paths_dispose(&paths);
```

It overwrites `out` with the found path's `git_str` without reading what `out` held, so
an owned allocation handed in is leaked (not freed twice, and not dangling:
`git_buf_fromstr` moves the string out of `paths.gitdir` before `repo_paths_dispose`
runs). The wrapper convention — fill a fresh `Buf::new()` through
`as_mut().as_mut_ptr()` — already avoids the leak, but the record should name the
exception so the repository batch does not offer a reusable-buffer variant for this
routine.

## Resubmitted

The `ptr` note and `_comment_agent` of `git_buf` now name `git_repository_discover` as
the one public filler that skips `git_buf_tostr`, with the consequence for a reused
buffer; the field's ownership record (`owned`, nullable, mutable, runtime-conditional
on `reserved`) was verified correct and left as it was. The comment also records the
strengthened `Buf::from_raw` contract
(`../../unsafe/buf-from-raw-contract-omits-initialised-contents`).
