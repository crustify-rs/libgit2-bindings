# Unsafe surface: `FilterList::load_with_blob` / `load_ext_with_blob` took a raw `git_blob` pointer

**Item:** `git_filter_list` (wave 0 batch 1, `crustify/rust/libgit2/libgit2/filter/filter.rs`)

**Before.** Both blob-taking constructors were `pub unsafe fn`, taking
`NonNull<ffi::git_blob>` under a `# Safety` contract (a live `git_blob` for the call),
documented as raw seams "until the `object` subsystem's run wraps `git_blob`". Their
consumers (the `ub_unsafe` and `equiv` filter suites) created and looked blobs up raw and
freed them by hand; `crustify scan-unsafe` counted the two among the three
`unsafe_fns_pub_smell` sites of this workset.

**Why they need not be unsafe.** A blob object *is* a `git_blob` (its first member is
the `git_object`), and `git_object` was wrapped in the same wave: an `ObjectRef<'_, '_>`
keeps the blob alive for the call. `git_filter_list__load` reads nothing of the blob but
its id (`git_oid_cpy(&src.oid, git_blob_id(blob))`), copied into the list's source, so the
blob may be of any repository and may go as soon as the call returns. What the type
system cannot say is that the object is a blob; libgit2 does not check it either, and
would read a commit, tree or tag as a blob.

**Fix.** Both take `blob: ObjectRef<'_, '_>` and are safe; an object of another type is
refused with a `GIT_ERROR_FILTER` error "the object is not a blob" (`LiveBlob::of`), after
the pin and the absolute-path refusal and before the built-in filters are initialised.
The `tests/ub_unsafe/filter.rs` suite is gone; its six scenarios (the probe filter seeing
the blob's id and the list keeping no blob, blobs of another repository that goes first,
options only copied, the no-list / failing-load / absolute-path edge cases) run through
the safe API in `tests/ub_safe/filter.rs::with_blob`, with the probe filter moved into the
suite's fixture (`fixture::filter_probe`), joined by `only_blobs_are_read` (a commit, tree
and tag refused by both forms; ASan would flag the over-read). `tests/equiv/filter.rs`
compares the blob forms through object handles on the Rust side.

The workset adds no public `unsafe fn` any more; the `ub_unsafe` obligation for
`git_filter_list` is void.
