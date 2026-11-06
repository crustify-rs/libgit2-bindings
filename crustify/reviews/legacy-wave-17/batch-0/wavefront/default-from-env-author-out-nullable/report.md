# `git_signature_default_from_env.author_out` was recorded as non-nullable

- **entity**: `git_signature_default_from_env` (symbol record, `ptr_args[0]`), submitted
  by core wave 1 batch 0 (`f62505e43`)
- **class**: inconsistent finding — the flag contradicts the note and the C

## Previous finding

`author_out`: `nullable: false`, with the note "Optional out slot (author_out ||
committer_out is asserted): written only on success with a fresh signature owned by the
caller. On error both temporaries are released with git__free, leaking their strings."
`committer_out` (position 1) was `nullable: true`.

## Evidence it is wrong

`src/libgit2/signature.c:281-314`:

```c
	GIT_ASSERT_ARG(author_out || committer_out);
	...
	if (author_out &&
	    (error = user_from_env(&author, repo, "GIT_AUTHOR_NAME", ...)) < 0)
		goto on_error;
	...
	if (author_out)
		*author_out = author;
```

Either slot may be NULL; only both NULL is refused. The two slots are symmetric, and a
wrapper that offered `author_out` as mandatory would deny a caller the "committer only"
call, `git_signature_default_from_env(NULL, &committer, repo)`, which libgit2's own CLI
makes (`src/cli/cmd_commit.c:481`); the "author only" form, with `committer_out` NULL,
is what `examples/stash.c:111` and `examples/tag.c:229` make.

## Resubmitted

`author_out` is now `nullable: true`, with a note that spells out the assertion and that
a NULL slot is left untouched; the rest of the record (the owned out-slot contract, the
`is_constructor` role on `author_out`, the error-path leak of the strings, `repo` only
read) was checked against the C and left as it was. The routine itself stays with the
object / signature symbol batch, which has the repository wrapper it needs.
