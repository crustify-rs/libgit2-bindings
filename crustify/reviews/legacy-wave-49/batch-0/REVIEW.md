# Review: libgit2 / index / wave 0 / batch 0 (the wave's types)

Reviewed on `ab3dabdcc` (the review-wave base), then rebased onto `b3716baeb` (this wave's
symbol review, batch 1) before landing. It covers the fifteen `type` items that wave 0's
translation batches 0–7 landed:

| items | wrappers | home |
|---|---|---|
| `git_diff`, `git_diff_delta` | `Diff<'r>` / `DiffRef` / `DiffMut`, `DiffBox` / `DiffArc` over `DiffFree`; eight constructors (`from_buffer`, `from_buffer_with_oid_type`, six generators, `generate_with_options`); `DiffDelta` / `DiffDeltaRef` and the dependent `DiffFileRef` projections | `diff/diff.rs` |
| `git_index_add_option_t`, `git_index_capability_t`, `git_index_entry_flag_t`, `git_index_entry_extended_flag_t`, `git_index_stage_t` | `AddFlags`, `Capabilities` + `CapabilitySetting`, `EntryFlags`, `EntryExtendedFlags`, `Stage` | `index/index.rs` |
| `git_index_iterator`, `git_index_conflict_iterator` | `IndexIterator` (`IndexIteratorBox`, `IndexIterator::new(&IndexArc)`), `ConflictIterator<'index>` (`ConflictIteratorBox`, `new(IndexMut)`) | `index/index.rs` |
| `git_index_time` | `IndexTime` value with `IndexTimeRef` / `IndexTimeMut` accessors | `index/index.rs` |
| `git_index_name_entry` | `NameEntryRef` getters; `IndexMut::{add_name, name_entry, clear_names}`, `IndexRef::name_entry_count` | `index/sys/index.rs` |
| `git_iterator` | `NoteIterator<'r>` (`NoteIteratorBox`, `new`, `from_commit`) | `index/notes.rs` |
| `git_pathspec`, `git_pathspec_flag_t`, `git_pathspec_match_list` | `PathspecBox` / `PathspecArc`, `PathspecFlags`, `MatchList<'diff>` (`MatchListBox`, four matchers, getters) | `index/pathspec.rs` |

Index has waves 0–2 (`schedule.json`), so wave 0 is not the sub-campaign's last wave.
BSan ran only on this review's one reproducer, alone.

| step | outcome |
|---|---|
| lifecycle discovery | No gap. Every out-parameter constructor in `include/git2` (`grep "git_<type> \*\*"`) and every destructor has a wrapper and a lifetime record: 8 diff constructors + `git_diff_free`; both note-iterator constructors + free; `git_pathspec_new` / `_free` (private `__init` / `__clear` intentionally unexposed); the four matchers + `git_pathspec_match_list_free`; both index iterators' new/free. NAME entries are parent-owned (`add_name` / `clear_names`); deltas, files, flags, stages and timestamps have no lifecycle. `query symbols --lifetime-for` agrees for each type. |
| wavefront findings | No defect. All fifteen type records (and `git_diff_file`) are submitted with no pending pointer slot. Spot-checked against C: counted snapshot index (`git_index_iterator.index`), uncounted cursor index, NAME strings (owned, nullable, empty on disk → NULL), match-list delta borrows and counted pathspec, the diff's uncounted repository. |
| safe boundary | 2 reports, fixed. `unsafe/generate-with-options-states-checked-obligations`: the one public `unsafe fn` required an object-id compatibility that the six safe generators already violate harmlessly, and a version that C checks itself; the contract now lists only real obligations and names what is checked. `unsafe/raw-adoption-helpers`: the private `from_raw_result` / `finish` `unsafe fn`s taking raw out-pointers became safe `constructed(rc, Option<Box>)` helpers (C leaves every failed output NULL, verified per constructor). The remaining workset smell is the documented `generate_with_options` cut until `git_diff_options` (diff wave 3) is wrapped. |
| UB | 1 report, fixed. `ub_unsafe/diff-options-empty-pattern-overread`: `generate_with_options`, within its contract, let libgit2 read past an empty pathspec pattern beside a non-empty one (ASan heap-buffer-overflow, reproducer included). The batch 1 fix for `Pathspec::new` did not reach this path. The wrapper now refuses such a pathspec with batch 1's `patterns_are_parsable`. Probed and clean: inputs of the other object-id type for all generators, both ways round (new `ub_safe` test). |
| equivalence | No defect. New `equiv` test `case_insensitive_populated_and_bare_generators_match_c` compares a populated working directory, a case-insensitive repository index (C's delta re-sort, passed and loaded), an unparsable on-disk index (C clears the reload error) and the bare-repository failures (`GIT_EBAREREPO`, error text) against C. |
| internal | Nothing beyond the fixes above. |
| conventions | 2 reports, fixed. `conventions/undocumented-unsafe-blocks`: four unsafe blocks had their SAFETY comment out of reach; one failed `clippy --all-targets`, reported but left by five batches. `conventions/unformatted-wave-code`: `commit.rs`'s rewired diff consumer and the conflict-cursor fixture were not rustfmt-clean. |
| misc | 2 reports, fixed. `misc/positive-code-for-refused-diff-options-version`: `git_diff_tree_to_workdir` / `git_diff_index_to_index` return a refused options version as `1` (C precedence bug); the wrapper refuses it as `-1` like the other generators. `misc/diff-not-sync-trybuild-depends-on-toolchain`: the `diff_not_sync` trybuild case failed the plain (stable) suite; it is now two `compile_fail,E0277` doctests. |
| coverage | See below. |

## Coverage

`coverage` variant (`cargo llvm-cov`, filters `diff pathspec index notes` over `lib`,
`equiv`, `ub_safe`, `ub_unsafe`), C gcov through a private `GCOV_PREFIX`.

- **Rust**: `diff/diff.rs` 415/417 lines, 737/740 regions; `index/index.rs` 896/898 lines;
  `index/notes.rs` 179/179; `index/pathspec.rs` 318/321; `index/sys/index.rs` 77/77. Both
  `!runtime_pinned()` exits of the diff parsers, uncovered since batch 0, are now covered
  (`unit_tests::parsers_refuse_without_a_runtime`, a child whose runtime fails). Remaining:
  defensive success-with-NULL closures (`constructed` in `pathspec.rs`, `MatchList`),
  failure arms of test helpers, a test allocator's never-called `free`, the unreachable
  `clear_names` error arm and `default_ref`'s config error arm (batch 5).
- **C, generators** (batch 0's figures → now): `git_diff_tree_to_index` 90 → 96.67 %,
  `git_diff_index_to_workdir` 86.21 → 89.66 %, `git_diff_tree_to_workdir` 95.83 → 100 %,
  `git_diff_tree_to_workdir_with_index` 80 → 95 %, `git_diff_index_to_index` 92.31 → 100 %;
  `diff_set_ignore_case`, `diff_load_index` and `diff_prepare_iterator_opts` 100 %.
  `git_diff_tree_to_tree` 96.15 %, `diff_generated_alloc` 88.46 %, `diff_parsed_alloc`
  79.17 %: the rest are allocation failures and `GIT_DIFF_UPDATE_INDEX` index writes (a
  nondefault option of the unsafe API), not forced here.
- **C, other lifecycles**: parsers, `git_diff_free` / `num_deltas` / `get_delta`, both
  index iterators' constructors, snapshot new/release, conflict cursor new/free, NAME
  add/clear/count/get, `git_pathspec_new`, the four matchers' entry points and all list
  getters 100 %. NULL no-ops (`git_pathspec_free`, `git_note_iterator_free`,
  `git_index_iterator_free`, `index_name_entry_free`) are unreachable from non-NULL owners.
  `git_note_commit_iterator_new` 85.71 %: the missed line is the C defect batch 5 refuses.
  `pathspec_match_from_iterator` 89.55 % and `git_pathspec_match_diff` 92.59 %:
  allocation-failure cleanup.

## Validation of the landed tree

Rebased onto `b3716baeb` and validated again. No C, header, CMake, bindgen allowlist or
`build.json` changed, so the shared prebuilds (`build.json` v1) were reused and the C
baseline was not rebuilt. Rust outputs and logs are private (`crustify/rust/target/*`,
`/tmp/crustify-index-rw0b0`).

- `cargo check --workspace --all-targets` and `cargo clippy --workspace --all-targets`:
  clean (the inherited `tests/equiv/index.rs` lint failure is fixed). rustfmt clean on
  every touched file.
- `cargo test --workspace`, plain, ASan/UBSan (LSan on) and TSan (`-Zbuild-std`,
  `--test-threads=1` per the known glibc `tzset` and filter-initialisation races): lib 675
  (1 pre-existing ignore), equiv 429, ub_safe 705 (with `compile_fail`, now green on plain
  too), ub_unsafe 95, doctests 44 + 314. No sanitizer report.
- Miri: the workset's Rust-only unit tests (layouts, flags, stages, projections,
  timestamps, trampoline state) pass; FFI tests are ignored under Miri.
- `crustify scan-unsafe --sites all`, wave tip → landed: `unsafe_fns_priv_smell` 19 → 17,
  `raw_ptr_priv_smell` 21 → 19, undocumented 49 → 45, no counter increased.
- BSan: only the reproducer `ub_unsafe/diff-options-empty-pattern-overread`, alone, on
  the fixed tree (`cargo bsan run`): clean, prints the refusal.

## Noted, not changed

- The diff generators take `IndexRef` although libgit2 sorts the index's entry vector
  when snapshotting it. `IndexIterator::new` (`&IndexArc`) shares that view, while
  `MatchList::from_index` takes `IndexMut`. Nothing behind `IndexRef` lends entries or
  positions today, so the sort is unobservable. Wave 1's entry accessors (`get_byindex`
  also sorts lazily) should keep ordered access on `IndexMut`.
- `DiffDeltaRef::status` / `flags` and `DiffFileRef::flags` / `mode` return raw integers.
  `git_delta_t` and `git_diff_flag_t` are scheduled in diff wave 0, and `git_diff_file` in
  diff wave 1 (cut seams).
- Deltas of foreign-type ids carry the repository's `id_abbrev` (40 for SHA-1, 64 for
  SHA-256), not the id's own hex size. This is a harmless C quirk, now pinned by the
  foreign-type test.
