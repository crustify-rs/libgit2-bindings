To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: wavefront's note on git_diff_from_buffer_ext's opts describes git_diff_options

Hi Marius,

This is an error in a recorded analysis note, not a bug in the Rust or in libgit2. It can be
handled in public.

The ownership store's finding for `git_diff_from_buffer_ext`'s `opts` parameter (submitted
in 8f238f339) has correct pointer properties, but its note says that the options hold
"pathspec/prefix contents", "callbacks/payload and borrowed pointers" whose "nested
referents" must outlive the result. It also says that both `version` and `oid_type` are read.
`git_diff_parse_options` is two scalars:

```c
typedef struct {
	unsigned int version;
	git_oid_t oid_type;
} git_diff_parse_options;
```

and `git_diff_from_buffer_ext` reads only `oid_type`, without validating it
(`diff_parse.c:90-91`). The note appears to be copied from `git_diff_options`. A binder
relying on it would add a keepalive the options don't need, and could miss that `oid_type`
must be limited to named types.

The corrected note, with the same properties, has been resubmitted through
`wavefront query symbols --update` on the review branch (crustify/waves/wave-229). The
report and the update file are under
`crustify/reviews/wave-229/batch-0/wavefront/ownership/`.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
