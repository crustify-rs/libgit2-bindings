To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: OdbExpandId docs overstate what git_odb_expand_ids accepts

Hi Marius,

This is a documentation problem in the libgit2 bindings. No safe path is affected today, but
the next wrapper could become unsound if its author trusts the docs.

The docs of `OdbExpandId` (`src/libgit2/src/libgit2/odb.rs`, commit 603bf0085) said:
"Every value is valid for the type: libgit2 reads no more than the 32 id bytes whatever
`length` says". `git_odb_expand_ids` does not accept every value safely. It treats an entry
whose `length` reaches the odb's hex size as a full id, and reads its header through the
backends without checking the id's type against the odb's. The loose backend formats that id
into a path sized for the odb's type (`src/libgit2/odb_loose.c:82`):

```rust
let mut q = [OdbExpandId::new(sha256_id.as_ref(), 64, None)];        // SHA-1 odb
unsafe { ffi::git_odb_expand_ids(odb_ptr, q.as_mut_ptr().cast(), 1) };
```

```
ERROR: AddressSanitizer: heap-buffer-overflow
WRITE of size 1 thread T0
    #1 git_oid_pathfmt src/libgit2/oid.c:153
    #2 object_file_name src/libgit2/odb_loose.c:94
    #4 loose_backend__read_header src/libgit2/odb_loose.c:608
    #6 odb_otype_fast src/libgit2/odb.c:1196
```

The crate does not yet wrap `git_odb_expand_ids`, so this needs `unsafe`. The corrected docs
state the obligation for that wrapper: refuse a full-length query whose id is not of the
odb's type, as the crate's odb readers already do.

The corrected docs are on the review branch (crustify/waves/wave-177), and the report is
under `crustify/reviews/legacy-wave-177/batch-0/misc/`.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
