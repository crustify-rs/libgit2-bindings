To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: git_odb_refresh frees the commit-graph file a revision walk reads unlocked

Hi Marius,

This is a contract gap in the libgit2 bindings' thread-safety reasoning. It rests on a race
inside libgit2. No safe path of the crate reaches it today, but the wrappers of the
revision walks would.

Wave 176 (commit b043039e2) made `OdbRef::clear_commit_graph` unsafe because
`git_odb_set_commit_graph` frees the commit-graph under the odb's lock, while a revision walk
reads the graph's file after releasing it. `Odb`'s `Sync` justification treats that as the
only path to the free. libgit2 frees the same file on a second path:

```c
/* git_odb_refresh, src/libgit2/odb.c:1708, under db->lock */
if (db->cgraph) git_commit_graph_refresh(db->cgraph);
/* git_commit_graph_refresh, src/libgit2/commit_graph.c:520 */
if (cgraph->file && git_commit_graph_file_needs_refresh(...)) {
        git_commit_graph_file_free(cgraph->file); cgraph->file = NULL; }
```

`git_odb_refresh` runs after every lookup miss (`git_odb_read`, `_read_prefix`,
`_read_header`, `_exists*`, `git_odb_write` of a new object). The reader,
`git_commit_list_parse` (`src/libgit2/commit_list.c:180`), uses the pointer that
`git_odb__get_commit_graph_file` returned after the lock was dropped. So a walk on one
thread races an odb lookup miss on another whenever `objects/info/commit-graph` is
rewritten.

The crate stays sound only because nothing in it reaches `git_commit_list_parse` yet. The
corrected docs record the obligation for the wrappers of `git_revwalk_*`,
`git_merge_base*` and `git_graph_*`. This is found by reading the code and has not been
reproduced; the race in libgit2 itself may be worth raising upstream once it is.

The corrected docs are on the review branch (crustify/waves/wave-177), and the report is
under `crustify/reviews/wave-177/batch-0/contracts/`.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
