# Review: libgit2 / index / wave 1 / batch 0 (the wave's types)

Reviewed on `3e694ec8b` (the review-wave base), then rebased onto `c5312ce66` (this wave's
symbol review, batch 1) and validated again before landing. It covers the three `type` items that
wave 1's translation batches 0 and 1 landed, with the lifecycle and lending routines
those type batches emitted alongside them (`git_index_options_init`, `git_index_new_ext`,
`git_index_open_ext`, `git_index_reuc_get_byindex`, `git_index_reuc_get_bypath`). Wave 2
schedules those routines as symbols, and no symbol review covers them before then.

| items | wrappers | home |
|---|---|---|
| `git_index_entry` | `IndexEntry<'p>` (hand-written, `Copy` value borrowing its path), `IndexEntryRef` / `IndexEntryMut`, all 12 fields | `index/index.rs` |
| `git_index_options` | `IndexOptions` value (`git_index_options_init`), `IndexOptionsRef` / `IndexOptionsMut`; consumed by `Index::with_oid_type` / `open_with_oid_type` | `index/index.rs` |
| `git_index_reuc_entry` | `ReucEntryRef` getters, `ReucSide`; lent by `IndexMut::reuc_entry` / `reuc_entry_by_path` | `index/sys/index.rs` |

Index has waves 0–2 (`schedule.json`), so wave 1 is not the sub-campaign's last wave and
this review owns no BSan gate. It emitted no reproducer (no UB or equivalence defect was
found), so BSan did not run.

| step | outcome |
|---|---|
| lifecycle discovery | No Rust gap. No public routine allocates, copies or frees a `git_index_entry` or `git_index_reuc_entry` for its caller (`grep` over `include/git2` for each type: only `const` lenders and by-pointer readers). Entries a caller builds are read for the call (`index_entry_dup`), and the index owns its own entries and records. `git_index_options` has its `init`, bound by `IndexOptions::new`. |
| wavefront findings | 1 report, fixed. `wavefront/entry-private-lifecycle-unrecorded`: `git_index_entry` had no dropper, cloner or constructor on record, unlike its REUC sibling. Submitted `index_entry_free`, `index_entry_create`, `index_entry_dup` and `index_entry_dup_pool`, spliced into HEAD's store layout (250 insertions, no deletions). The other records (both type records, the options `init`, `new_ext` / `open_ext`, the REUC getters, allocator and free) were checked against C and are right. |
| safe boundary | No defect. No public `unsafe fn` and no raw pointer on the three types' public surface (`into_raw` hands out the plain struct, as `Box::into_raw` would). The `from_ptr` / `as_ptr` seams are `pub(crate)`, and `scan-unsafe --sites all` reports no smell site in either home file. `IndexMut::reuc_entry(&self)` lending was rechecked against every routine an `IndexRef` reaches (`write` sorts the REUC vector, `write_tree` re-sorts it on an ignore-case index, diff and checkout only snapshot or iterate, `set_index` takes `&IndexArc`). None frees a record, so lending from `&self` holds. |
| UB | No defect. New `ub_safe` test `sys_index::reuc_records_outlive_writes_through_the_shared_reborrow` holds twelve records plus a by-path lookup from one `&IndexMut` across two writes through its `as_ref()`, case-sensitive and ignore-case, then rereads the file. Clean under ASan/UBSan (LSan on). |
| equivalence | No defect. The existing suites compare Rust- and C-built entries and options struct-for-struct and through `git_index_add` / written files, and REUC records from files and adds. The new test above also pins a C behaviour: an ignore-case index writes its REUC records case-folded, and the reader trusts that order. |
| internal | No defect. New unit test `reuc_record_allocation_failures_release_the_new_record` sweeps allocation failures through the REUC constructor (`reuc_entry_alloc`) and vector growth, for a new path and a replacement. Each failure reports out of memory, frees the new record and keeps the index's records. `git_vector_insert_sorted` grows a full vector before it looks for a duplicate, so a replacement can fail there too. |
| conventions | 1 report, fixed. `conventions/open-with-oid-type-split-unsafe-binding`: rustfmt had split `Index::open_with_oid_type`'s `let rc =` from its `unsafe` block, leaving the block out of its SAFETY comment's reach. It was the only undocumented site in `index/index.rs`. |
| misc | 3 reports, fixed. `misc/entry-dev-doc-names-the-wrong-stat-field`: libgit2 stores `st_rdev` (0 for a regular file) in `dev`, not the device holding the file. `misc/entry-mode-doc-omits-tree-modes`: `git_index_read_tree` keeps a tree's raw mode (`0100664`), which has no `FileMode`. Both are pinned by the new unit test `symbols::entry_getters_read_what_libgit2_stores`, and both were found with C probes. `misc/stale-dead-code-allowance-on-entry-borrow`: `IndexEntryRef::from_ptr` carried `#[allow(dead_code)]` "used by later waves" although `find_prefix` has called it since batch 2. |
| coverage | See below. |

## Coverage

`coverage` variant (`cargo llvm-cov`, filters `index options entry reuc` over `lib`,
`equiv`, `ub_safe`, `ub_unsafe`). C gcov goes through a private `GCOV_PREFIX`; figures
are review base → landed tree.

- **Rust**: `index/index.rs` 1931 lines, 13 unrun, all of them in test helpers (failure
  arms of `isolated`, never-called callbacks, `refuse_each_allocation`'s
  `unreachable!`). Every line of the three types' wrappers and their emitted
  constructors and lenders runs. `index/sys/index.rs` 279/279 lines; missed regions 5 → 4.
  The one REUC region left is `side`'s `?` on `CSlice::get`, which cannot miss for slots
  0–2. The others are symbol error arms that the symbol review owns.
- **C, the types' routines**: `reuc_entry_alloc` 92.86 → 100 % and `git_index_reuc_add`
  90.91 → 100 % (the allocation-failure sweep). Already at 100 %: `git_index_options_init`,
  `git_index_new_ext`, `git_index_reuc_get_byindex` / `_bypath`, `index_entry_create`,
  `index_entry_dup`, `index_entry_cpy`, `index_entry_reuc_init`, `index_reuc_insert`,
  `index_entry_reuc_free`, `git_index_entry__init_from_stat`, `git_index_entry_stage`.
- **Left uncovered**: `git_index_open_ext` 97.37 % (its `git_pool_init` failure, which
  allocates nothing and fails only when the system page size cannot be read;
  environment-dependent). `index_entry_free` 83.33 % (its NULL no-op, which no caller
  reaches with a live entry). `read_reuc` 90.70 % (vector init on an
  already-initialised vector, `git_oid_from_raw` failure, unreachable with a valid oid
  type, and vector-insert OOM, where C leaks the record it was filling, so LSan would
  fail an injected test). `read_tree_cb` 82.14 % belongs to `git_index_read_tree`
  (wave 2), which the new test drives only for its mode.

## Validation of the landed tree

This review changes no C, header, CMake, shim, bindgen allowlist or `build.json` (batch
1's one allowlisted function only regenerates bindings), so the shared prebuilds
(`build.json` v1) were reused and the C baseline was not rebuilt. Rust outputs
and logs are private (`crustify/rust/target/*`, `/tmp/crustify-index-rw1b0`).

- `cargo check --workspace --all-targets` and `cargo clippy --workspace --all-targets`
  (plain): no warning. rustfmt (stable, edition 2024) is clean on both touched files.
- `cargo test --workspace`, plain and ASan/UBSan (LSan on), whole crate, on the rebased
  tree: lib 702 (1 pre-existing ignore), equiv 450, ub_safe 737, ub_unsafe 95, doctests
  46 + 328, with no sanitizer report. The pre-rebase tree passed the same runs (lib 698,
  ub_safe 735). Before any fix, the review base ran green on plain over the whole workspace
  (lib 696, ub_safe 734) and under ASan/UBSan on the `index`-filtered suites.
- TSan (`-Zbuild-std`, `--test-threads=1` per the known `tzset` and filter-load races),
  filters `index entry options reuc`, rebased tree: lib 123, equiv 98, ub_safe 166, with
  no report. This review adds no `Send` / `Sync` impl to the types.
- Miri: the nine Rust-only unit tests of the workset (entry layout, borrow, flags,
  equality and debug; options accessors; timestamps; REUC layout and sides) pass. FFI
  tests are ignored under Miri.
- `crustify scan-unsafe`, review base → landed (rebased) tree: undocumented 46 → 45 (the
  conventions fix; batch 1's landing kept 46), no smell counter changed, and neither home
  file has a smell site or an undocumented one.
- BSan: not run. This is not the sub-campaign's last wave, and the review emitted no
  reproducer.

## Noted, not changed (for wave 2's entry lenders)

- `IndexEntryMut` must never be lent over an index-owned entry: `set_path` would store a
  borrowed path the index outlives, and `set_flags` could break the path-length and stage
  bits the index's search relies on. Lenders (`get_byindex`, `get_bypath`,
  `iterator_next`, `conflict_get/next`) should hand out `IndexEntryRef` only.
- C rewrites lent entries in place: `IndexRef::write` clears `UPTODATE` on every entry
  and zeroes racily clean sizes. So an `IndexEntryRef` lent from `&IndexMut` would alias
  writes that the same handle's `as_ref().write()` makes. Entry lenders should borrow
  the index exclusively for the entry's lifetime, or `write` should move to `IndexMut`.
  An `IndexRef` reached through an `IndexArc` should lend no entry at all, since another
  holder (the repository) may free it. The REUC records are unaffected: nothing an
  `IndexRef` reaches writes or frees them.
- `git_index_reuc_get_byindex` asserts that the vector is sorted. A file written by an
  ignore-case index and read case-sensitively is flagged sorted though it is not (C trusts
  the on-disk order), so binary-search lookups by path can miss, without UB. This is C
  behaviour; the new `ub_safe` test pins the order the file keeps.
