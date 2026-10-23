# Review: libgit2 / index / wave 2 / batch 0 (the wave's symbols)

Reviewed commit `b9d01cafd`, the review wave's base. It covers the 19 `symbol` items of
the worklist:

| batch | commit | items | Rust |
|---|---|---|---|
| 0 | `cb77d2e9b` | `git_index_{add,add_from_buffer,conflict_add,conflict_get,conflict_next,entry_is_conflict,entry_stage,get_byindex,get_bypath,iterator_next,read_tree}` | `IndexMut::{add,add_from_buffer,add_conflict,conflict,entry,entry_by_path,read_tree}`, `Iterator for ConflictIteratorMut` / `IndexIteratorMut`, `IndexEntryRef::{is_conflict,stage}`; new `Conflict`, `IndexEntryBuf` |
| 1 | `2b8ec7e2c` | `GIT_INDEX_ENTRY_STAGE`, `GIT_INDEX_ENTRY_STAGE_SET` | `IndexEntryRef::stage`, `IndexEntryMut::set_stage` (shims `crustify_GIT_INDEX_ENTRY_STAGE{,_SET}`), `AnyStageError` |
| wave 1 anchors | | `git_index_{new_ext,open_ext,options_init,reuc_get_byindex,reuc_get_bypath}`, `git_pathspec_match_tree` | `Index::{with_oid_type,open_with_oid_type}`, `IndexOptions::new`, `IndexMut::{reuc_entry,reuc_entry_by_path}`, `MatchList::from_tree` |

Every item has exactly one filled anchor. `IndexEntryRef::stage` carries two, for
`GIT_INDEX_ENTRY_STAGE` and `git_index_entry_stage`: the macro and the exported function
read the same two bits. Every wrapper was re-read against its C definition, and against
the C paths that free, sort, rewrite or retain what it lends. Index has waves 0-2
(`schedule.json`), so wave 2 is the sub-campaign's last, and this review ran its BSan gate
(below).

| step | outcome |
|---|---|
| lifecycle discovery | Not applicable: a symbol route. `git_index_new_ext` / `open_ext` / `options_init` are constructors already paired with `IndexFree` / plain values by wave 0-1's type runs, and were not wrapped again. |
| wavefront findings | 2 records incomplete, resubmitted and spliced into the base store layout (2 changed lines). `wavefront/add-from-buffer-stage-uaf-unrecorded`: the `source_entry` note left out the conflict-stage use after free the batch had found and refused. `wavefront/read-tree-failure-leaks-unrecorded`: the `index` note said only an out-of-memory map insert leaks. Every failure leaks the entries already read, and a refused name also leaks its joined path. The other 17 records were read against the C and hold (the two macros have no pointer slot to record). |
| safe boundary | No defect. `crustify scan-unsafe --sites all` shows no smell site in `index/index.rs`, `index/sys/index.rs` or `index/pathspec.rs`. The workset adds no public `unsafe fn`, no public raw pointer and no `ub_unsafe` suite. Re-derived and held: lending (`entry`, `entry_by_path`, `conflict`) from `&IndexMut` only, since libgit2's sort moves pointers, not entries, and every routine that frees or rewrites an entry takes `&mut`. Also held: the conflict cursor's `Conflict<'index>` outliving the cursor but not the exclusive borrow it moved in; `IndexIteratorMut` yielding `IndexEntryBuf` copies, since a shared index's holders rewrite entries in place; `IndexEntryBuf`'s `Send`/`Sync` (owned `CString`, no exclusive handle); `Conflict` and the entry handles being neither. |
| UB | No true positive. A temporary ASan/LSan probe read a tree with a name repeated three times (written raw to the odb), then ran lookups, a colliding add, removals, a conflict, a clear, rereads and a snapshot. Each entry was freed once. The entry map keeps one key per path, so a lookup can miss a duplicate, but every free removes the key first, so nothing dangles. Checked by hand: an entry lent by `entry()` across `IndexRef::write` (only `flags_extended` is rewritten, through no Rust reference) and `write_tree_to` (re-sorts only); `add` of an entry lent from the same index (rejected by the borrow checker); `add_conflict` with one side given twice; `add_from_buffer` with an empty slice; `read_tree` of a SHA-256 tree into a SHA-1 index; snapshot entries removed, replaced in place or cleared behind the iterator. ASan/UBSan (LSan on), TSan and Miri are clean on the final tree. The BSan gate found no report (below). |
| equivalence | 1 defect, fixed: `equiv/add-from-buffer-stage-refusal-replaces-c-errors`. `add_from_buffer` refused every conflict-stage entry before libgit2's own checks. Through the safe API the index never has an owner, so every refused call was one libgit2 fails safely ("could not initialize index entry…"), and the wrapper replaced that message with its own. The reproducer exits 101 on the base and 0 on the fix. The refusal now applies only where libgit2 would add, and then free, the entry: an owner, a file or link mode, and data that fits. Regression: `equiv::index::entries::conflict_stage_buffers_fail_as_c_before_adding`. The deliberate divergences (`Stage::Any` lookups, foreign ids refused before the path check) are documented and unchanged. |
| internal | Nothing beyond the misc items. `Conflict::from_sides`, `insertable`, `insertable_mode` and the `IndexEntryBuf` plumbing are covered by the inline tests and the suites. |
| conventions | 1 report, fixed: `conventions/conflict-safety-comment-outside-scan-window`. `IndexMut::conflict`'s call was split as `let rc =` / `unsafe {…}`. That was the one site `scan-unsafe` counted as undocumented in `index.rs` (46 → 45 in total). Names, homes, anchors, `addr_of!` projections and handle shapes conform. Touched files are `rustfmt +stable` clean, as at the base, and `cargo clippy --workspace --all-targets` is clean. |
| misc | 2 reports, fixed. (1) `misc/read-tree-doc-omits-entries-leaked-on-failure`. `read_tree`'s doc named only two of libgit2's leaks, but every failed read leaks the entries read so far. Both failing-read tests happened to fail before the first entry. The doc now says so; new test `ub_safe::index::entries::failed_tree_reads_after_entries_leave_the_index_as_it_was` (LSan off in its child). (2) `misc/long-path-tree-write-tests-outlast-the-bsan-gate`. Two wave 1 tests with >65535-byte path components ran past the gate's 40 minutes, which lost the `lib` and `ub_safe` verdicts. Their long-name halves are now separate tests with the campaign's `cfg_attr(bsan, ignore)` marker. |
| coverage | See below. New test `ub_safe::index::entries::snapshot_copies_cross_threads` exercises `IndexEntryBuf`'s asserted `Send`/`Sync` under TSan (shared reads and clones, moves, drops on other threads). |

## BSan gate (the sub-campaign's)

One `--no-run` build, then one `cargo-variant.sh bsan test` per test binary (`libgit2-sys`,
`lib`, `ub_safe`, `ub_unsafe`, `equiv`), all in parallel, each under `timeout 2400`, with no
`--skip` beyond the script's `compile_fail`. Each gate ran from a snapshot of its tree
(`git archive` of the base, or a `tar` of the working tree, plus `git init`, `include/`,
and the builds and `tests/` links), with its own target directory. Logs:
`bsan-gate-before/` and `bsan-gate-after/`.

- **Wave tip `b9d01cafd`, before any fix.** No BorrowSanitizer report anywhere.
  `libgit2-sys`: 0 tests. `ub_unsafe`: 95 passed (72 s). `equiv`: 455 passed, 8 ignored
  (2011 s). `lib` and `ub_safe` hit the timeout (exit 124) with every test done but one
  each: the long-path tree-write tests (`misc/long-path-tree-write-tests-outlast-the-bsan-gate`).
  Every ignored test carries an earlier wave's `cfg_attr(bsan, ignore)` marker.
- **The tree landed here.** Every binary exited 0, with no BorrowSanitizer report: `libgit2-sys` 0 tests,
  `lib` 707 passed and 5 ignored (61 s), `ub_unsafe` 95 (63 s), `ub_safe` 745 passed and 6
  ignored (907 s), `equiv` 456 passed and 8 ignored (1748 s). That covers every test this
  review added or changed, except the two `tree_writes_refuse_long_names` halves, which
  are the only new ignores. The snapshot was checked to match the landed `crustify/rust`
  and `include` byte for byte (`diff -r`). An earlier run of this gate, started before the
  test split, was stopped once the before-gate showed those two tests outlast it.

## Coverage

Rust (`cargo llvm-cov`, `coverage` variant, the `index` and `pathspec` tests of the lib,
`equiv` and `ub_safe` binaries, after the fixes):

- `index/index.rs`: 2506 of 2520 lines. Every line of the workset's wrappers and their
  helpers is covered. The uncovered lines are wave 1's `racy_paths_parsable` `None`
  branch (unreachable within the count) and test helpers' failure arms.
- `index/sys/index.rs`: 279 of 279. `index/pathspec.rs`: 323 of 326 (unit-test failure
  arms).

C (gcov of the `coverage` build, private `GCOV_PREFIX`). `git_index_get_byindex`,
`get_bypath`, `add`, `iterator_next`, `entry_stage`, `entry_is_conflict`,
`options_init`, `reuc_get_by{index,path}`, `index_conflict__get_byindex`,
`git_index__conflict_to_reuc` and `git_pathspec_match_tree` are fully covered. What
remains uncovered:

- allocation failures: `add_from_buffer`'s failed insert and REUC move after the blob is
  written (`index.c:1557`, `:1561`), `conflict_add`'s non-`ENOTFOUND` removal failure
  (`:1885`), `read_tree_cb`'s join and vector insert (`:3428`, `:3452-3453`),
  `read_tree`'s map resize, map insert and clear (`:3484`, `:3488-3489`, `:3498`), and
  `open_ext`'s pool init (`:451`);
- unreachable: `conflict_get` / `conflict_next`'s negative `index_conflict__get_byindex`
  return (`:1993`, `:2148`; it returns a count), and `index_insert`'s directory
  canonicalisation and collision failures (`:1427`, `:1442`), which these callers' trusted
  paths and replace mode never take.

## Validation

Shared prebuilds (`crustify/builds/*`, build.json version 1) through `cargo-variant.sh`,
private outputs under `/tmp/crustify-index-rw2-b0`. No C, shim or `-sys` change, so no C
baseline was needed.

| run | result |
|---|---|
| `cargo check --workspace`, `cargo clippy --workspace --all-targets` | clean |
| `cargo test --workspace --no-fail-fast` (plain, final tree) | lib 711 + equiv 464 + ub_safe 752 + ub_unsafe 95 + doctests 49 + 334 pass (1 ignored), `compile_fail` included |
| `cargo-variant.sh asan-ubsan test --workspace --no-fail-fast -- --skip compile_fail` (LSan on) | every test passes (before the test split); rerun after it on the `index` tests of lib, equiv and ub_safe (72 + 55 + 77) |
| `cargo-variant.sh tsan test --workspace --lib --tests -- --skip compile_fail` | lib, equiv, ub_unsafe pass. `ub_safe` reported glibc's `tzset` race once, in `config::iterator_entries::public_entries_interleaved_views_snapshots_and_reparse` (`set_head` → reflog signature → `mktime`; a known intermittent report, outside this workset), and passed in full (750) on the rerun. `index` / `pathspec` filtered: 71 + 61 + 87 pass |
| Miri (`cargo +nightly miri test -p libgit2 --lib -- index::index::unit_tests`) | 17 pass; 35 call libgit2 and are ignored |
| BSan | the gate above |
| reproducer `equiv/add-from-buffer-stage-refusal-replaces-c-errors` (plain) | base `b9d01cafd`: exit 101; fixed tree: exit 0 |
| `crustify scan-unsafe --json --sites all` | no smell counter changed; `undocumented_unsafe` 46 → 45 |

## Follow-ups outside this workset

- `IndexRef::tree_writable` (wave 1) still reads entries through raw
  `git_index_get_byindex` / `git_index_entry_stage` calls and field projections, which
  `IndexEntryRef::from_ptr` and its getters now cover. It is internal, so this is not a
  boundary defect.
- The `config` test above should run in a child process, as earlier tzset stragglers do.
