To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: heap overflow from safe OdbRef::open_read_stream with a foreign id type

Hi Marius,

This is a soundness bug in the safe API of the libgit2 bindings: safe code can overflow a
heap buffer inside libgit2.

`OdbRef::open_read_stream` (`src/libgit2/src/libgit2/odb.rs`, commit 3f43c106d) passes its
id to `git_odb_open_rstream` without checking that the id's type matches the odb's. libgit2
does not check it either. The loose backend's `readstream` member formats the id into a path
it sized for the odb's type (`object_file_name`, `src/libgit2/odb_loose.c:82`), so a
SHA-256 id in a SHA-1 odb writes 65 hex digits into room for 42:

```rust
#![forbid(unsafe_code)]
let odb = Odb::open(c"/tmp/ors")?;                     // SHA-1, loose backend
let wide = Oid::from_raw(&[0xab; 32], OidType::Sha256)?;
odb.as_ref().open_read_stream(wide.as_ref());
```

```
ERROR: AddressSanitizer: heap-buffer-overflow
WRITE of size 1 thread T0
    #0 git_oid_fmt_substr src/libgit2/oid.h:133
    #1 git_oid_pathfmt src/libgit2/oid.c:153
    #2 object_file_name src/libgit2/odb_loose.c:94
    #4 loose_backend__readstream src/libgit2/odb_loose.c:1037
    #5 git_odb_open_rstream src/libgit2/odb.c:1587
    #6 OdbRef::open_read_stream src/libgit2/src/libgit2/odb.rs:2888
```

The converse, a SHA-1 id in a SHA-256 odb, leaves part of the path uninitialised before
`stat` reads it. The crate's other odb readers already refuse foreign ids
(`OdbRef::fits`). This entry point was bound in an earlier batch and missed the check.

The fix applies the same check to `open_read_stream`, and a regression test is added. Both
are on the review branch (crustify/waves/wave-177), and the full report is under
`crustify/reviews/legacy-wave-177/batch-0/ub_safe/`. The underlying libgit2 behaviour (no id-type
check before the loose backend formats an id) is the one reported earlier for
`git_treebuilder_insert`. The standalone reproducer is available on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
