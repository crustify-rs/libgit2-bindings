To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: git_odb_backend's recorded lifecycle has no constructors or droppers

Hi Marius,

This is an analysis-record problem, not a soundness issue.

Wave 180 batch 0 (commit 9fe83bce6) wrapped `git_odb_backend` and recorded one lifecycle
role in the campaign's ownership store: `git_odb_init_backend` as the `init` constructor.
`wavefront … query symbols --lifetime-for git_odb_backend` therefore answered
`dropped_by: []` and only that one constructor.

The public API has four `alloc` constructors, each storing a new backend through a
`git_odb_backend **`. Three static `free` members release one:

```c
int git_odb_backend_pack(git_odb_backend **backend_out, ...);   /* odb_pack.c:943 */
int git_odb_backend_one_pack(git_odb_backend **backend_out, ...);/* odb_pack.c:920 */
int git_odb_backend_loose(git_odb_backend **backend_out, ...);  /* odb_loose.c:1234 */
int git_mempack_new(git_odb_backend **out);                     /* odb_mempack.c:193 */
backend->parent.free = &pack_backend__free;  /* also loose_backend__free, impl__free */
```

The batch's own docs name these constructors, and its `OdbBackendFree` policy releases
through the `free` member. The Rust is correct. Only the record that the later constructor
waves (442, 464, 470) will read is incomplete.

All seven roles, with the out-parameter and argument ownership blocks, are submitted
through `query symbols --update` on the review branch (crustify/waves/wave-181). The report
is under `crustify/reviews/wave-181/batch-0/wavefront/lifecycle/`.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
