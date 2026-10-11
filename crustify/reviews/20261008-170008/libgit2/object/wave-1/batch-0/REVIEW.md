# Review: libgit2 / object / wave 1 / batch 0 (the wave's types)

Reviewed commit `38e6d1b3e` (review wave base; landed on `2c0063abc`, after batch 1),
covering the nine `type` items the wave's batches 0–4 landed:

| item | wrapper | home |
|---|---|---|
| `git_blob` | `Blob<'r>` / `BlobRef` / `BlobMut`, `BlobArc<'r>` (batch 0) | `object/blob.rs` |
| `git_blob_filter_options` | `FilterOptions` value and handles (batch 0) | `object/blob.rs` |
| `git_commit` | `Commit<'r>` / `CommitRef` / `CommitMut`, `CommitArc<'r>` (batch 1) | `object/commit.rs` |
| `git_commit_create_ext_options` | `CreateExtOptions<'o>` and handles (batch 1; signer by batch 5) | `object/commit.rs` |
| `git_message_trailer_array` | `MessageTrailerArray`, `MessageTrailerArrayVal` (batch 2) | `object/message.rs` |
| `git_object_id_options` | `ObjectIdOptions<'f>` and handles (batch 2) | `object/object.rs` |
| `git_tag` | `Tag<'r>` / `TagRef` / `TagMut`, `TagArc<'r>` (batch 3) | `object/tag.rs` |
| `git_tree` | `Tree<'r>` / `TreeRef` / `TreeMut`, `TreeArc<'r>` (batch 3) | `object/tree.rs` |
| `git_tree_update` | `TreeUpdate<'p>` and handles (batch 4) | `object/tree.rs` |

Object has waves 0–3 (`schedule.json`), so this is not the sub-campaign's last wave: no
BSan gate here. No reproducer was emitted (no UB or equivalence defect found), so no
BSan run either.

| step | outcome |
|---|---|
| lifecycle discovery | 2 gaps, filled: `lifecycle/git_commit` — `git_commit_parent` and `git_commit_nth_gen_ancestor`, `alloc` constructors by out-parameter that batch 1's own wavefront note names, are `CommitRef::parent(n)` and `CommitRef::nth_gen_ancestor(n)`, returning `CommitArc<'r>`. `lifecycle/git_tree` — `git_commit_tree` (declared in `commit.h`) is `CommitRef::tree()`, returning `TreeArc<'r>`. All three keep C's `unsigned int` and errors; the `-sys` allowlist gained them and `git_commit_parentcount` (read raw by the equivalence test). The other items' primitives are complete: `git_blob` and `git_tag` have only their lookups, free and dup (no other public routine yields one); `git_message_trailer_array` its `init` (`git_message_trailers`) and disposer; the three option structs their `*_init`; `git_tree_update` none. `git_tag_target` / `git_tag_peel` construct a `git_object` (a wave-0 type) and stay with their wave-2 symbol batch. |
| wavefront findings | 1 report, resubmitted: `wavefront/commit-navigation-constructors-unsubmitted` — the three routines carried no `lifetime` and no `ptr` blocks. The records the batches submitted for the nine types and their primitives were read against the C and hold (the four objects' refcount through `git_object`, the options' borrowed pointers, the trailer array's owned block and trailers; `payload`'s `mutable: null` is right for C, which never writes it). |
| safe boundary | no defect. The workset's one public `unsafe fn`, `MessageTrailerArray::from_raw`, is an adoption seam whose `# Safety` contract was re-read against `MessageTrailerArrayFree` and the `Send`/`Sync` claims, and holds; `ub_unsafe/message.rs` covers it. The only raw C type on the workset's surface is `FilterOptions`' by-value `From`/`Into` conversion with `git_blob_filter_options`, whose `reserved` pointer nothing dereferences. `crustify scan-unsafe`: no `*_pub_smell` site in `object/`; the only object sites left are `raw_ptr_derefs_outside_impl` in the callback trampolines (symbol items). |
| UB | no candidate survived. Hand-checked: the four typed casts (`from_object` checks the parsed type before reinterpreting; `as_object` / `into_object` keep the pointer), `to_*` up-refs, never-sole `CArc` owners and the covariance of `'r`; the options' covariance in `'o` / `'f` with invariant exclusive handles (outlives constraints carry the stored borrows); `CreateExtOptions::extra_headers` lending `'o` runs that outlive the handle; `ObjectIdOptions::filters` re-lending the exclusively borrowed list shared; `FilterOptions` and `TreeUpdate` `Send`/`Sync` (plain data, a never-read `reserved`, a `&'p CStr`); `MessageTrailerArray` `Send`/`Sync`, its empty-array dispose skip and `take`/`set` moves. Instruments on the whole workspace after the changes: plain, ASan/UBSan with LSan, TSan (`-Zbuild-std`) all clean; Miri on the six modules' Rust-only inline tests: 60 passed. |
| equivalence | no defect; gap filled for the new constructors: `tests/equiv/commit.rs::navigation::derived_commits_and_trees_match_c` (13 base commits in `testrepo.git`, `shallow.git` and a fresh repository of hand-written commits with a missing or blob tree and a missing or blob parent; 11 derivations each, 143 steps compared by id, type and identity or by code, class and message, every failure kind asserted reached; then the bases released first and the survivors read again). |
| internal | nothing to fix; the inline suites cover the private paths (`is_commit` / `is_tag` / `is_tree`, the dispose skips, `copy_id`, `EMPTY`). New inline test `commit.rs::unit_tests::parents_ancestors_and_trees_derive_owned_references`. |
| conventions | 2 reports, fixed: `conventions/commit-from-object-named-try-from-object` (`Commit::try_from_object` renamed `from_object`, as `Blob`, `Tag` and `Tree` name the same conversion); `conventions/message-trailers-parse-safety-comment-split` (the scan's undocumented site in `MessageTrailerArray::parse`). `cargo clippy --workspace --all-targets -D warnings` is clean; touched files are formatted with stable `rustfmt` (only this review's hunks in `tests/ub_safe/commit.rs`, which carries older unformatted hunks). |
| misc | 3 reports, fixed: `misc/commit-array-compile-fail-case-unstable-across-toolchains` (the ASan and TSan variants failed `ub_safe::compile_fail` on the wave tip; the toolchain-worded `E0521` check is now a doctest); `misc/typed-lookup-docs-redundant-link-targets` (seven rustdoc warnings on the workset's constructors); `misc/message-trailers-parse-hides-unchecked-buffer-growth` (a C NULL write on a failed trailer-buffer growth, now documented on `parse`; not reachable from safe Rust short of memory exhaustion). |
| coverage | Rust (`cargo llvm-cov`, `coverage` variant, the six modules' suites), lines: `object.rs` 99.7 %, `commit.rs` 99.6 %, `tree.rs` 99.5 %, `message.rs` 99.2 %, `blob.rs` 99.2 %, `tag.rs` 98.8 %; the new constructors are fully covered. Uncovered in the workset: the `unreachable!("git_*_dup cannot fail")` arms of `to_blob` / `to_commit` / `to_tag` / `to_tree`, `parse`'s `!runtime_pinned()` return (the pin cannot fail once libgit2 has initialised in the test process), and child-process failure formatting in test helpers. C (gcov of the `coverage` build, private `GCOV_PREFIX`): every workset routine — the 16 typed lookups / frees / dups, `git_commit_parent`, `git_commit_tree`, `git_message_trailers` with `extract_trailer_block`, `git_message_trailer_array_free` and the three `*_options_init` — is fully covered but for `git_commit_nth_gen_ancestor`'s `git_commit_dup` failure branch (an atomic increment, which cannot fail). |

## Validation of the landed tree

On the tree rebased onto batch 1's `2c0063abc`: `cargo check --workspace`, `cargo clippy
--workspace --all-targets -D warnings`, and `cargo test --workspace` under the plain,
ASan/UBSan (LSan on) and TSan (`-Zbuild-std`) variants: `lib` 540 passed / 1 ignored,
`equiv` 335, `ub_safe` 556 (`compile_fail` included), `ub_unsafe` 91, doctests 198, every
variant clean. Miri on the six modules' Rust-only inline tests: 60 passed.

## Noted, not changed

- **The signer and the future `git_commit_create_ext` wrapper.** `sign_with_closure`
  forms `&mut SignatureCallback` from the options' payload. No public path hands
  `CreateExtOptions` to C yet; the wave-2 wrapper should take the options exclusively
  (`CreateExtOptionsMut` or `&mut`), since a shared `&CreateExtOptions` could be captured
  by the signing closure (through a `Cell` set after the options are built) and the
  closure re-enter the same options, aliasing the callback.
- **Ids of another type.** `FilterOptionsMut::attr_commit_id_mut` / `set_attr_commit_id`
  and `TreeUpdateMut::id_mut` / `set_id` store any id, of any type byte, as the C structs
  allow. `git_blob_filter` and `git_tree_create_updated` (wave 2) read those ids from the
  object database; their wrappers should refuse ids of another type than the
  repository's, as `id_fits` does for every lookup.
- `cargo doc` still warns at `blob.rs:1245` (a symbol item's docs) and
  `filter/filter.rs:63`, outside this workset.
