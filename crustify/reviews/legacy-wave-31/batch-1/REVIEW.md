# Review: libgit2 / object / wave 2 / batch 1 (the wave's symbols and callbacks)

Reviewed commit `d6a6d1f95` (review-wave base), covering the 89 `symbol` / `callback`
items the wave's symbol batches landed or re-scheduled: batch 1 (`e54be84f8`: blob
readers and filtering, commit signatures and body, the commit creation family and
`git_commit_create_cb`), batch 2 (`5e8ea7fbf`: commit getters, mailmap signature
resolution, object ids from buffers and files), batch 3 (`06cf4d560`: tag accessors and
writers, `git_tag_foreach`, signature defaults, tree updates) and batch 4 (`8c39ae046`:
tree reads, `git_tree_walk`, `git_treebuilder_filter`), in
`object/{blob,commit,mailmap,message,object,signature,tag,tree}.rs`,
`object/sys/commit.rs` and `core/types.rs` (the signature constructors). Every item
carries its `Wraps:` anchor. The lifecycle routines among them (`git_{blob,commit,tag,tree}_{dup,free}`,
`git_signature_{dup,free}`, `git_message_trailer_array_free`) are the policies the type
runs emitted (`BlobFree`, `CommitFree`, `TagFree`, `TreeFree`, `SignatureFree`,
`MessageTrailerArrayFree`), and the lookups, `git_commit_{parent,nth_gen_ancestor,tree}`,
`git_tree_entry_bypath`, `git_treebuilder_new`, the `*_options_init` routines and the
signature constructors are the constructors those runs anchored; they were re-read
against the C, not re-emitted.

Object's last wave is wave 3 (`schedule.json`), so this review ran BSan only on its own
reproducer, alone.

| step | outcome |
|---|---|
| lifecycle discovery | not applicable: a symbol-route batch. The lifecycle routines scheduled as symbols are bound to the policies above; their `CDrop` / `CRefClone` / `CDupClone` / `CDispose` contracts hold against the C (the object frees skip NULL and only count, `git_signature_dup` is refused for a signature missing a string, `git_message_trailer_array_free` is skipped for an empty array). |
| wavefront findings | 1 report, resubmitted: `wavefront/tag-foreach-iterover-return-unrecorded`. `git_tag_foreach`'s callback note said every non-zero answer is returned, but `git_reference_foreach_name` turns `GIT_ITEROVER` into 0 (the root of the equivalence defect below). The store diff is that one note: the tool's re-sort of the symbol list, which a batch had left unsorted, was not committed. Re-read and accurate: `git_tree_walk`, `git_treebuilder_filter`, `git_commit_create_from_callback`, `git_commit_create_ext`, `git_tag_annotation_create`, `git_commit_summary`, `git_commit_body`, `git_tree_entry_byname`. |
| safe boundary | clean. The workset adds no public `unsafe fn` and no public raw pointer. `crustify scan-unsafe --sites all` finds no `*_smell` site in the object files. The public `from_raw` seams there (`MessageTrailer`, `MessageTrailerArray`, `CommitArray`) are type-run adoption constructors outside this workset. The only `raw_ptr_derefs_outside_impl` sites are the documented `extern "C"` trampolines' payload derefs. There is no `ub_unsafe` suite to review. |
| UB | no true positive. An ASan/UBSan probe (LSan on, `#![forbid(unsafe_code)]`) ran clean through the public API: `Tree::create_updated` over empty, `/`-led, `//`, trailing-`/`, `..`, `.git` and 70 000-byte components, upserts and removals, with and without a baseline; `get_by_name` with a 70 000-byte name; `header_field` over empty, spaced and prefix fields; tree walks stopped at every position in both orders, with skips; `foreach` breaking with -31, -30, -7, ±1, 31, `i32::MAX` and `i32::MIN`; `BlobRef::filter` over `""`, `.`, `..`, `a/../b`, `x/`; `id_from_file` on `/dev/null`, `/proc/self/status`, `/` and a missing path; `create` with empty strings, an empty encoding, newline and bracket strings, and the extreme times and offsets `writable` lets through. The C quirks the campaign recorded (`header_field` newline, extract-signature hang, tag zero target, tree-builder foreign ids and long names, foreign parents, `INT_MIN` offsets) are still refused before C. |
| equivalence | 1 defect, fixed: `equiv/tag-foreach-break-with-iterover-panics`. `tag::foreach` asserted that libgit2 returns the code the closure broke with; for `GIT_ITEROVER` libgit2 returns 0, so the safe call panicked (`debug_assert_eq!`, every debug build) where C returns 0. It now reports `Break(-31)`, the docs state the divergence, and the error clause is corrected (libgit2 keeps any error already set). The reproducer exits 101 on the base and 0 on the fix, and BSan ran it alone on the fix with no report. Regressions: `tests/equiv/tag.rs::repository_tags::iterover_breaks_match_c_but_for_the_code` and `GIT_ITEROVER` in `tag.rs::unit_tests::foreach_tells_breaks_from_failures`, both failing with the old assertion. |
| internal | none found. Re-read: the trampolines' panic and NULL guards and their `stopped` bookkeeping (`WalkCallback`, `TreeBuilderFilter`, `ForeachCallback`, `CreateCallback`, `ParentCallback`), the parent collection before libgit2 runs, `resume_signer_panic`, `reads_attr_commit_id` against `git_blob_filter`'s order, `default_from_env`'s committer-first sequence, and `create_from_array`'s per-arity dispatch. |
| conventions | 1 report, fixed: `conventions/safety-comments-outside-the-scanned-window`. The `SAFETY` comments of `TreeRef::owner` (seven lines) and `TreeBuilder::new` (split from `unsafe` by rustfmt) sat outside `scan-unsafe`'s six-line window; both now sit in it. `rustfmt +stable --edition 2024` was run on the five touched files, all of which were rustfmt-clean at the base. `cargo clippy --workspace --all-targets` is clean. |
| misc | none beyond the `foreach` error clause folded into the equivalence fix. |
| coverage | see below. |

## Coverage

Rust (`cargo llvm-cov`, `coverage` variant, whole crate, base): the nine workset files
have no uncovered production line except unreachable ones. Those are the
`unreachable!("git_*_dup cannot fail")` arms, the `!runtime_pinned()` early returns
(`MessageTrailerArray::parse`, `id_from_buffer`, `id_from_file`, `TreeBuilder::new` /
`from_tree`), the `ParentCallback` trampoline's guard after an unresumed panic, and the
child-failure message formatting of the isolated test helpers.

C (gcov of the `coverage` build, private `GCOV_PREFIX`): the base left
`git_commit__create_internal`'s object-write failure (commit.c:243) unreached by every
creation routine. Two equivalence tests now reach it, one per translation unit, on
repositories whose missing loose fan-out directories are regular files:
`tests/equiv/commit.rs::creating::unwritable_object_databases_fail_alike` (`create`,
`create_from_array`, `create_ext`, `amend`, with `create_buffer` unaffected) and
`tests/equiv/sys_commit.rs::unwritable_object_databases_fail_alike`
(`create_from_ids`, `create_from_callback`). Each compares the code, class, masked
message and references after every step.

Still uncovered, and why:

- **OOM arms**: `git_commit__create_buffer_internal`, `tree_walk`'s path growth, `write_tag_annotation`, `git_commit__header_field`.
- **Unreachable from the safe API**:
  - NULL-argument arms (`git_signature_dup`, `git_mailmap_resolve_signature`), which the wrappers cannot pass;
  - `normalize_options`' invalid object and id types, which the setters cannot store;
  - `git_tree_create_updated`'s `git_tree_dup` failure, which cannot fail;
  - `create_popped_tree`'s D/F branch (descent checks the same tree first);
  - `commit_parent_from_array`'s foreign-parent stop, which the wrappers refuse first.
- **Not reached by `git_object_id_from_file`**: `git_object_id_from_symlink`, and `id_from_fd`'s read-size and hash-context failures.
- **Environment-dependent**: `user_from_env`'s non-not-found `user.email` read failure.

## Validation

On the landed tree:

- `cargo check --workspace`, `cargo clippy --workspace --all-targets` and `cargo test --workspace` (plain, `compile_fail` and doctests included);
- ASan/UBSan (LSan on) and TSan (`-Zbuild-std`) on every workspace test binary with `--skip compile_fail`;
- Miri on the Rust-only unit tests of `object/tag.rs` and `object/tree.rs`;
- the reproducer on plain (fails on the base, passes on the fix) and alone under BSan (clean);
- `crustify scan-unsafe` on the tree.

No C changed, so no C baseline was rerun.
