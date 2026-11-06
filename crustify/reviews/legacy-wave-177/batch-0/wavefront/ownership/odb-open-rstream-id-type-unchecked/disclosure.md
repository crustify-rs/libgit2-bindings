To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: git_odb_open_rstream's ownership-store note misses the id-type obligation

Hi Marius,

This is an analysis-record problem. It led to a soundness bug, which is reported separately.

The campaign's ownership store described `git_odb_open_rstream`'s `oid` parameter only as
"Read for the call only." (submitted with commit 3f43c106d). libgit2 hands the id to each
backend's `readstream` member without checking its type against the odb's. The loose
backend then formats it into a path sized for the odb's type
(`src/libgit2/odb_loose.c:82`, reached from `loose_backend__readstream`, line 1037):

```c
size_t path_size = be->oid_hexsize + 1;          /* the odb's type */
git_str_grow_by(name, path_size + 1);
git_oid_pathfmt(name->ptr + name->size, id);      /* the id's type */
```

With no obligation recorded, the safe wrapper `OdbRef::open_read_stream` was bound without
the id-type check that the crate's other odb readers apply. A SHA-256 id in a SHA-1 odb
then overflows the heap (AddressSanitizer: heap-buffer-overflow in `git_oid_pathfmt`).

The note is corrected through `query symbols --update` on the review branch
(crustify/waves/wave-177). The report is under
`crustify/reviews/legacy-wave-177/batch-0/wavefront/ownership/`. The patch is available on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
