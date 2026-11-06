To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: git_odb_object's recorded lifecycle has no constructors

Hi Marius,

This is an analysis-record problem, not a soundness issue.

For `git_odb_object`, the campaign's ownership store recorded a dropper
(`git_odb_object_free`) and an up-ref cloner (`git_odb_object_dup`), but no constructor
(wave 176, commit b043039e2). `wavefront … query symbols --lifetime-for git_odb_object`
answered `constructed_by: []`.

Two routines construct one: `git_odb_read` and `git_odb_read_prefix`
(`src/libgit2/odb.c:1155`, `:1306`). Each stores a counted reference the caller owns through
its `git_odb_object **out`:

```c
*out = git_cache_get_raw(odb_cache(db), id);     /* hit: count raised */
...
error = odb_read_1(out, db, id, false);           /* miss: new object, cached */
```

Their own `out` notes already say "owned by the caller and released with
git_odb_object_free". The stream type's readers were recorded as `alloc` constructors in the
same wave. The Rust binding (`OdbObjectArc`, built from these results) is correct. Only the
record that the next wrappers and reviews read is incomplete.

Both roles are submitted through `query symbols --update` on the review branch
(crustify/waves/wave-177). The report is under
`crustify/reviews/legacy-wave-177/batch-0/wavefront/lifecycle/`.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
