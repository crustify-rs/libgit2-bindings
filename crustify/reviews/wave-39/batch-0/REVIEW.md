# Review: libgit2 / config / wave 2 / batch 0 (the wave's types)

Reviewed commit `3a5814a4e` (the review-wave base), rebased onto `fb61c8337` (the wave's
symbol review) before landing. It covers the one `type` item that wave 2 batch 0
(`a6e60a4d5`) landed:

| item | wrapper | home |
|---|---|---|
| `git_config_backend_entry` | `BackendEntry` / `BackendEntryRef` / `BackendEntryMut` (`define_ctype!`), `BackendEntryArc = CArc<BackendEntry, BackendEntryFree>`, `BackendEntryFree` (`git_config_entry_free` through the enclosing layout), `BackendEntry::from_entry` / `into_entry`, field accessors `entry` / `entry_mut` / `release_policy` | `config/sys/config.rs` |

Config has waves 0–5 (`schedule.json`), so wave 2 is not the sub-campaign's last wave and
this review runs no BSan gate. This review found no UB or equivalence defect, so it emits
no reproducer, and there was nothing to run BSan on.

| step | outcome |
|---|---|
| lifecycle discovery | No gap. Public producers of `git_config_backend_entry`: the `git_config_backend.get` vtable slot (acquired reference) and `git_config_iterator.next` (borrowed until the iterator advances). Both are callback fields of types scheduled in waves 3–4. `git_config_get_entry` hands out `&be->entry`, which reaches the wrapper through `EntryArc` → `BackendEntry::from_entry`. The only public releaser is `git_config_entry_free`, which casts back and calls the entry's `free`. For every built-in backend that is `git_config_list_entry_free`, an atomic down-ref of the immutable list owning the strings. No public constructor, up-ref or cloner exists (`git_config_list_dup_entry` is internal and appends to a list). `query symbols --lifetime-for git_config_backend_entry` agrees: only the internal `git_config_list_entry_free` matches by argument type, and the public dropper acts through the `git_config_entry` cast that the type record lists. The owner (`CArc` without `CRefClone`) matches the C contract: references alias, cannot be copied and are released once. |
| wavefront findings | No defect. The type record (`free` is a non-null, allocator-chosen callback, not a data owner; `entry` is inline) and the lifecycle records (`git_config_list_entry_free` and `git_config_entry_free` droppers, `git_config_get_entry` alloc constructor) match the C. |
| safe boundary | No defect. The workset adds no public `unsafe fn` and no public raw pointer, so there is no `ub_unsafe` suite. `from_ptr` / `as_ptr` / `as_mut_ptr` are `pub(crate)`. The layout type's field is private, and no safe path reaches a `BackendEntryMut` over C memory. `crustify scan-unsafe --sites all`: no smell, `*_outside_impl` or undocumented site in the workset's three files, before or after. Counter deltas: `ffi_calls` +1 (the policy now calls `git_config_entry_free` directly) and `ref_to_type_wrapper_sanctioned` +1 (`&self` `Debug` on the layout type). |
| UB | No true positive. Checked by hand: `from_entry` only adopts an `EntryArc`, whose own policy already requires the enclosing allocation (C casts unconditionally). The conversions move one reference and never duplicate it. `entry()` ties `EntryRef` to the backend-entry borrow. Retained entries outlive their config, files and snapshot (the list refcount). A reentrant release from inside a callback consumes each reference once. No `Send`/`Sync` is asserted, since the callback may be thread-bound. New ub_safe test `backend_entry_debug_reads_retained_storage` formats entries after their config, files and a sibling are gone, clean under ASan/UBSan (LSan on) and TSan. |
| equivalence | No defect. New `equiv` test `backend_entries_across_levels_match_c`: four priorities (system, global, local, a custom app priority), a key shadowed across all of them, includes at depths 1 and 2, a valueless key and a miss. C and Rust records (name, value, backend type, origin file, level, include depth, callback) match after each lookup, after both configs and their files are freed, and after each step of an interleaved release order. |
| internal | Nothing beyond what the conventions fix touches. `release_policy`'s missing-callback check now has a unit test. |
| conventions | 2 reports, fixed. `conventions/backend-entry-free-lacks-lifecycle-anchors`: `BackendEntryFree` hand-implements `CDrop` around `git_config_entry_free` but carried neither `Wraps: git_config_entry_free` nor `Handwritten`. It also reached C through `EntryFree.c_drop` + `new_unchecked`; it now calls C directly. ffibox's `impl_cdrop!` + adapter form was tried and rejected because the scanner counts a free adapter as four smell sites. `conventions/backend-entry-items-after-unit-tests`: the wrapper had been appended after `mod unit_tests`, the only such file in the crate. It now precedes it. |
| misc | 2 reports, fixed. `misc/backend-entry-mutation-doctest-is-vacuous`: the "acquired owner cannot be mutated" compile-fail doctest called `CArc::as_mut`, which does not exist under any policy. With a `CRefClone` impl added, `get_mut` compiled and the doctest still passed. It now tests `get_mut` and `try_into_box`, and every compile-fail doctest pins its error code. Re-running the probe, the new doctests fail as intended. `misc/backend-entry-handles-lack-debug`: `Debug` for `BackendEntry` / `Ref` / `Mut`, printing the entry and eliding the callback, matching the module's other wrappers. |
| coverage | See below. |

## Coverage

Measured with the `coverage` variant (`cargo llvm-cov` over the `backend_entr`-filtered
`lib`, `equiv` and `ub_safe` tests) and C gcov through a private `GCOV_PREFIX`.

- **Rust**, the backend-entry block of `config/sys/config.rs` (`define_ctype!` to
  `unit_tests`): all 40 lines with counts are executed. Before this review,
  `release_policy`'s missing-callback branch and the `Debug` impls (new) had no test.
  Now `release_policy_rejects_missing_callback` and `debug_prints_the_entry_only` cover
  them, and both also run under Miri.
- **C**: `git_config_list_entry_free` and `git_config_get_entry` are 100 %.
  `git_config_entry_free` covers all but its NULL no-op, which no owner can reach
  because owners are non-NULL. `git_config_list_dup_entry` covers all but its
  allocation-failure exits, which are the heap-corrupting paths reported in wave 1
  (`misc/config-list-dup-corrupts-heap-on-allocation-failure`) and are not forced here.
  `config_snapshot_get` covers all but the snapshot mutex failure, which depends on the
  environment. `config_file_get`'s error exits are not reached, because the safe lookup
  goes through a snapshot (wave 1's UB fix). `get_entry`'s `GET_NO_ERRORS` branch serves
  only the internal `git_config__lookup_entry`. The memory backend's `config_memory_get`
  waits for the safe backend wrapper (wave 5).

## Validation of the landed tree

The work was rebased onto `fb61c8337` (the symbol review, which touches only
`config/config.rs` and `tests/equiv/config.rs`, not this workset's files) and validated
again. The shared prebuilds were reused: this review changes no C, header, CMake,
bindgen allowlist or `build.json`, so the C baseline was not rebuilt. Rust outputs and
logs are private to this worktree's `crustify/rust/target/`, and C gcov data went
through `GCOV_PREFIX` into `/tmp/crustify-w2b0-gcov`.

- `cargo check --workspace --all-targets` and `cargo clippy --workspace --all-targets`
  are clean (0 warnings). rustfmt (`+stable --edition 2024 --check`) is clean on the
  three touched files.
- `cargo test --workspace` under plain, ASan/UBSan (LSan on) and TSan (`-Zbuild-std`):
  `lib` 618 passed / 1 ignored (pre-existing), `equiv` 391, `ub_safe` 649 (with
  `compile_fail`), `ub_unsafe` 91, doctests 43 + 251. No sanitizer report.
- Miri, `unit_tests::backend_entries`: 3 Rust-only tests pass, and the 1 that calls C is
  ignored.
- The compile-fail regression probe (a temporary `CRefClone` impl on
  `BackendEntryFree`) makes the `get_mut`, `try_into_box` and `clone` doctests fail as
  intended. It was reverted before committing.
- `crustify scan-unsafe --sites all`: no smell, `*_outside_impl` or undocumented site in
  `config/sys/config.rs`, `tests/equiv/sys_config.rs` or `tests/ub_safe/sys_config.rs`.
  The crate-wide smell counters are unchanged.
- BSan: not run. This is not the sub-campaign's last wave, and the review emitted no
  reproducer.

## Noted, not changed

- `EntryArc`'s doc (`config/config.rs`, type `git_config_entry`, wave 1) carries the same
  vacuous `entry.as_mut()` compile-fail doctest. It is outside this workset. See the misc
  report.
- `release_policy` returns a stateless token whose `release` is equivalent to dropping
  the owner, and anyone can also obtain the token through `Default`. It is harmless and
  keeps the `free` field's anchor on an accessor. Removing it would be an API change
  without a defect behind it.
