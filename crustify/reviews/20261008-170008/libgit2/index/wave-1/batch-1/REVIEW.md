# Review: libgit2 / index / wave 1 / batch 1 (the wave's symbols)

Reviewed commit `3e694ec8b`, the review wave's base. It covers the 55 `symbol` items the
wave's translation batches landed:

| batch | commit | items | Rust |
|---|---|---|---|
| 2 | `05a56b569` | `git_index_{entrycount,has_conflicts,caps,owner,clear,add_bypath,add_all,find,find_prefix,conflict_remove,conflict_cleanup,extension_lookup,extension_add,extension_remove}` | `IndexRef::{entry_count,has_conflicts,capabilities,has_owner,is_owned_by}`, `IndexMut::{clear,add_path,add_all,find,find_prefix,remove_conflict,clear_conflicts,extension,set_extension,remove_extension}` |
| 3 | `7f4d85bd7` | `git_index_{path,version,write,write_tree,write_tree_to,read,set_version,set_caps,remove,remove_bypath,remove_directory,remove_all,update_all,reuc_entrycount,reuc_add,reuc_find,reuc_remove,reuc_clear}` | `IndexRef::{path,version,write,write_tree,write_tree_to,reuc_entry_count}`, `IndexMut::{read,set_version,set_caps,remove,remove_bypath,remove_directory,remove_all,update_all,add_reuc,find_reuc,remove_reuc,clear_reuc}` |
| 3, 4 | wave 0 anchors | `git_pathspec_{new,free,match_index,match_diff,match_workdir,matches_path,match_list_{entry,entrycount,failed_entry,failed_entrycount,diff_entry,free}}` | `Pathspec::new`, `PathspecFree`, `MatchList::{from_index,from_diff,from_workdir}`, `PathspecRef::matches_path` (batch 4, `6cf89dc1c`), `MatchListRef::{entry,len,failed_entry,failed_len,diff_entry}`, `MatchListFree` |
| wave 0 type runs | anchors | `git_index_{new,open,free,iterator_new,iterator_free,conflict_iterator_new,conflict_iterator_free,name_add,name_clear,name_entrycount,name_get_byindex}` | `Index::{new,open}`, `IndexFree`, `IndexIterator::new`, `IndexIteratorFree`, `ConflictIterator::new`, `ConflictIteratorFree`, `IndexMut::{add_name,clear_names,name_entry}`, `IndexRef::name_entry_count` |

Each item has exactly one filled anchor, two for `git_index_new` (constructor and policy)
and two for `git_index_owner` (two contracts). Every wrapper was re-read against its C
definition, and against the C paths that free, sort, retain or lend what it touches.
Batch 2 landed without a `validation.md`, so its routines got the closest reading. Wave 1
is not index's last wave (`schedule.json`: waves 0-2), so BSan ran only on the reproducer
emitted here.

| step | outcome |
|---|---|
| lifecycle discovery | Not applicable: a symbol route. The lifecycle items in the worklist (`IndexFree`, `IndexIteratorFree`, `ConflictIteratorFree`, `PathspecFree`, `MatchListFree`, constructors) were emitted by wave 0's type runs and still pair each allocator with its releaser. |
| wavefront findings | 3 notes were incomplete. Each was resubmitted and spliced into the base store layout: 3 changed lines. `wavefront/index-write-racy-scan-pathspec`: `git_index_write`'s record omitted the racily clean scan's pathspec, which is the UB below. `wavefront/index-remove-wildcard-lookup-key`: `git_index_remove` with `-1` drops the stage 3 lookup key and removes an arbitrary stage. `wavefront/find-prefix-case-folded-miss`: `git_index_find_prefix` misses case variants in a case-insensitive index. |
| safe boundary | No defect. `crustify scan-unsafe --sites all` shows no smell site in `index/index.rs`, `index/sys/index.rs` or `index/pathspec.rs`. Their only `unsafe fn`s are the lifecycle policies' `CDrop` seams. The workset adds no public `unsafe fn` and no public raw pointer, so there is no `ub_unsafe` suite to review. Re-read and held: why each routine takes `IndexRef` or `IndexMut`. Shared writers free no entry. Lookups that sort take `&mut`. `reuc_entry` lends from a shared reborrow of the exclusive handle. |
| UB | 1 true positive, fixed: `ub_safe/index-write-empty-path-overread`. A repository's index holding the racily clean entries `""` and `"a"` (an index file can) makes `IndexRef::write` reach libgit2's pathspec parser, which reads one byte past the empty path. There is a `#![forbid(unsafe_code)]` ASan reproducer: it aborts on the base and exits 0 on the fix, and exits 0 alone under BSan. `write` now refuses such an index before libgit2 scans it. Checked by hand, with no defect: extension data of 0 bytes (a non-NULL `malloc(0)`, adopted and freed); `set_extension` with an empty slice; REUC ids of the other oid type (copied whole, written in bounds); scans whose callbacks hold other indexes; the conflict cursor's covariant index lifetime; match lists over diffs. |
| equivalence | No equivalence defect: every wrapper the suites drive matches C where it does not deliberately diverge. Two wrappers reproduced libgit2 defects their docs promised to avoid. They are filed under misc and diverge on purpose now, and their equivalence steps compare against C's observable entries instead. |
| internal | Nothing beyond the misc items. |
| conventions | No report. One nit fixed in place: a 156-column line in `add_all`'s doc was reflowed. rustfmt (stable, edition 2024) is clean on every touched file. The new unsafe blocks carry `SAFETY` comments, and `undocumented` stays at 46. |
| misc | 2 reports, fixed. (1) `misc/remove-any-stage-drops-a-lookup-key`: `remove(path, Stage::Any)` passed libgit2's wildcard on, which removes stage 2 of a three-way conflict and hides the kept stage 3 from `git_index_get_bypath`. The wrapper now removes the first entry `find` finds. (2) `misc/find-prefix-misses-case-variants`: in a case-insensitive index, `find_prefix(c"a/")` returned `None` beside `"A/x"`, `"a/y"`. It now scans for the first byte-wise match, as documented. |
| coverage | See below. |

## Follow-ups outside this workset

- The racily clean scan behind `git_index_write` also runs inside libgit2 routines that
  write the repository's index: `git_checkout_*`, `git_reset_default`,
  `git_stash_apply`, `git_submodule_add_to_index`, and `git_diff_index_to_workdir` with
  `GIT_DIFF_UPDATE_INDEX` once it updates an entry. `diff/diff.rs` wraps the last one.
  This review did not build a reproducer for it. The others have no wrapper yet. Their
  wrappers need the same check, which is `IndexRef::racy_paths_parsable` (private to
  `index.rs` today).
- `ffi::git_index_get_bypath` is now in the `-sys` allowlist, ahead of its wave 2 wrapper.

## Coverage

Rust (`cargo llvm-cov`, `coverage` variant, the `index` and `pathspec` tests of the lib,
`equiv` and `ub_safe` binaries, after the fixes):

- `index/index.rs`: 2017 of 2032 lines. The two uncovered lines in workset code are
  `None` branches of `IndexEntryRef::from_ptr`, in `racy_paths_parsable` and
  `first_stage_at`. Both are unreachable, since each position is within the count. The
  other 13 are test helpers' failure arms.
- `index/sys/index.rs`: 279 of 279.
- `index/pathspec.rs`: 323 of 326; the 3 uncovered lines are unit-test failure arms.

C (gcov of the `coverage` build, private `GCOV_PREFIX`): every line of the workset's
routines in `src/libgit2/index.c` and `src/libgit2/pathspec.c` is reached except:

- allocation failures: `git_index_clear`'s removal error, `index_conflict_remove`'s
  break, `git_index_reuc_add`'s free on a failed insert, the pathspec-init failures in
  `git_index_add_all` / `index_apply_to_wd_diff` / `index_apply_to_all`, and the
  `pathspec.c` allocation and iterator-construction failures;
- `git_index_read`'s forced-clear failure and `truncate_racily_clean`'s diff failure;
- `add_repo_as_submodule`'s workdir-path, `stat`, entry and open failures, then
  `git_index_add_bypath`'s failed insert after it and failed `REUC` move. These are races,
  OOM, or a repository found but not openable, on which libgit2 leaks;
- `index_apply_to_wd_diff`'s failed diff: a configuration corrupted after the
  repository was opened did not make it fail;
- `git_index_extension_add`'s remove and search failures, which are unreachable
  (`git_vector_bsearch2` returns only 0 or `GIT_ENOTFOUND`);
- `index_apply_to_all`'s `NONE` / `UPDATE` / default actions, which no public routine
  passes, and `git_index_iterator_free(NULL)`;
- `git_pathspec__match`'s `matched_at` writes, which these callers never request.

New coverage test: `index.rs::unit_tests::symbols::add_path_reports_a_failed_submodule_lookup`
covers `git_index_add_bypath`'s submodule-lookup failure (`index.c:1647`), using a
`.gitmodules` libgit2 cannot parse.

## Validation

Shared prebuilds (`crustify/builds/*`, build.json version 1) through `cargo-variant.sh`.
No C or shim changed. The only `-sys` change is one allowlisted function
(`git_index_get_bypath`), so no C baseline was needed.

| run | result |
|---|---|
| `cargo check --workspace`, `cargo clippy --workspace`, `cargo clippy --workspace --all-targets` | clean |
| `cargo test --workspace --no-fail-fast` (plain) | 700 + 450 + 736 + 95 + 46 + 328 pass (1 ignored), `compile_fail` included |
| `cargo-variant.sh asan-ubsan test --workspace --no-fail-fast` (LSan on) | every test passes; rerun on the final tree for `index` / `pathspec` (61 + 47 + 72) |
| `cargo-variant.sh tsan test -p libgit2 --lib --test equiv --test ub_safe -- index pathspec` | 61 + 47 + 72 pass |
| Miri (`cargo +nightly miri test -p libgit2 --lib`, index unit tests) | 18 pass; 30 call libgit2 and are ignored |
| BSan | only the `ub_safe/index-write-empty-path-overread` reproducer, alone: exit 0 |
| reproducer, `asan-ubsan` | base `3e694ec8b`: exit 134 (heap-buffer-overflow); fixed tree: exit 0 |
| `crustify scan-unsafe --json --sites all` | no smell counter changed; `undocumented` stays 46 (the one site in `index.rs` is batch 0's `open_with_oid_type` split, outside this workset) |
