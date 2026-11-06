# `git_tag_foreach`'s callback record says every non-zero answer is returned; -31 comes back as 0

- **record**: `git_tag_foreach` (symbol, `src/libgit2/tag.c`), `ptr_args[1]` (`cb`) note
- **submitted by**: libgit2/object wave 2 batch 3 (`06cf4d560`)
- **resubmitted**: yes, through `wavefront … query symbols --name git_tag_foreach --update`
  (one note changed in `crustify/wavefront/ownership-store.json`)

## Previous finding

> git_tag_foreach_cb, called synchronously on the calling thread with the full refname
> (borrowed from the iterator) and a stack-local resolved id, both valid for the call
> only; a non-zero return stops the walk and is returned (git_error_set_after_callback_function
> sets GIT_ERROR_CALLBACK if no error).

## Evidence it is incomplete

`git_tag_foreach` returns `git_reference_foreach_name(repo, &tags_cb, &data)`, and
`tags_cb` returns the callback's value. `git_reference_foreach_name`
(`src/libgit2/refs.c:724-732`) breaks on it and then rewrites the iterator's end sentinel:

```c
	while (!(error = git_reference_next_name(&refname, iter))) {
		if ((error = callback(refname, payload)) != 0) {
			git_error_set_after_callback(error);
			break;
		}
	}

	if (error == GIT_ITEROVER)
		error = 0;
```

So a callback that stops the walk with `GIT_ITEROVER` (-31) gets 0 back, as a finished
walk does. A C run on `testrepo.git` whose callback returns -31 at the first tag visits
one tag and returns 0 (`equiv/tag-foreach-break-with-iterover-panics`). The wrapper built
on the old record asserted `rc == code` and panicked there.

The parenthesis was also loose: `git_error_set_after_callback_function` sets its error
only when the thread has no error at all, stale ones included. Nothing need be set by
the callback itself.

## Corrected finding

> … a non-zero return stops the walk (git_error_set_after_callback_function sets
> GIT_ERROR_CALLBACK if no error is set yet) and is returned, EXCEPT GIT_ITEROVER (-31):
> git_reference_foreach_name maps it to 0 after the break, so a callback stopping with
> -31 is indistinguishable from a finished walk by the return code. A wrapper recording
> the callback's stop code must not assert rc == code for -31.

Ownership, borrow, nullability and mutability facets are unchanged. The other
callback-taking records of the workset were re-read against the C and hold:
`git_tree_walk` (negative returned as is, positive skips in pre-order),
`git_treebuilder_filter`, `git_commit_create_from_callback`, `git_commit_create_ext` (sign
callback: every negative fails, `GIT_PASSTHROUGH` included).
