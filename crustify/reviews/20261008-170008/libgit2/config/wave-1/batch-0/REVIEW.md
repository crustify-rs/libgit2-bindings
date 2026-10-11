# Review: libgit2 / config / wave 1 / batch 0 (the wave's types)

Reviewed commit `d79771f6f` (the review-wave base), rebased onto `1317ac5cb` (the wave's
symbol review) before landing. It covers the two `type` items that
wave 1 batch 0 (`512be0487`) landed:

| item | wrapper | home |
|---|---|---|
| `git_config_entry` | `Entry` / `EntryRef` / `EntryMut` (`define_ctype!`), `EntryArc = CArc<Entry, EntryFree>`, `EntryFree` (`git_config_entry_free`), `ConfigRef::entry` (`git_config_get_entry`) | `config/config.rs` |
| `git_configmap` | `ConfigMap<'s>` / `ConfigMapRef` / `ConfigMapMut` (handwritten), `MapMatch<'s>` | `config/config.rs` |

Config has waves 0–5 (`schedule.json`), so wave 1 is not the sub-campaign's last wave and
this review runs no BSan gate. BSan ran only on the one reproducer this review emits,
alone (`ub_safe/entry-lookup-forms-member-of-unwritten-backend-entry`): clean on the
fixed tree.

| step | outcome |
|---|---|
| lifecycle discovery | No gap. A grep for `git_config_entry **` over every `include/git2` header finds `git_config_get_entry` (an alloc constructor, `ConfigRef::entry`) and `git_config_next` (a borrowed entry valid until the iterator advances, `EntryRef`, wrapped in wave 4). `git_config_foreach_cb` lends `const git_config_entry *` (`EntryRef`, wave 2/3). The only releaser is `git_config_entry_free` (`EntryFree`), which calls the backend entry's `free`. For every built-in backend that is `git_config_list_entry_free`, an atomic down-ref of the immutable `git_config_list` that owns names, values and the interned backend-type/origin strings. No public cloner or up-ref exists (`git_config_list_dup_entry` is internal). `git_configmap` has no C lifecycle at all. `query symbols --lifetime-for` agrees for both. |
| wavefront findings | No defect. The type records match the C: list-owned, immutable borrowed strings; the casts to `git_config_backend_entry`; read-only `str_match` in maps. So do the lifecycle records (`git_config_entry_free` dropper, `git_config_get_entry` alloc constructor). Wavefront marks `name`/`backend_type` non-nullable, which is right as a published contract: C itself dereferences both (`git_config_foreach_match`'s regex, `git_config_list_dup_entry`). The Rust getters still return `Option` against custom backends, which is safe, not a misrepresentation. |
| safe boundary | No defect. The workset adds no public `unsafe fn` and no public raw pointer. `from_ptr` / `as_ptr` / `as_mut_ptr` are `pub(crate)` seams. `Entry`'s field is private, and no safe path reaches a `CBox<Entry, _>` or an `EntryMut` over C memory (`CArc` without `CRefClone` offers only `as_ref`). `crustify scan-unsafe`: no smell site in `config.rs` before or after. Counter changes: `unsafe_impls` +6 (the new `Send`/`Sync` impls); `unsafe_blocks` and `ffi_calls` +3 (the composed lookup's iterator calls, inside the wrapper impl); `ref_to_type_wrapper_sanctioned` +2 (the `&self` `Debug` impls on the `Entry`/`ConfigMap` layout types). Undocumented sites are unchanged, and the ones in `config.rs` and `tests/equiv/config.rs` belong to the symbol batches (`set_string`, `set_bool`, the repository-snapshot test). |
| UB | 1 report, fixed: `ub_safe/entry-lookup-forms-member-of-unwritten-backend-entry`. When a backend's `get` fails with anything but `GIT_ENOTFOUND` (a file rewritten into something libgit2 cannot parse, then reread by the lookup), C's `get_entry` evaluates `&be->entry` on the entry pointer the backend never wrote (`config.c:811`). UBSan aborts a `#![forbid(unsafe_code)]` program calling `ConfigRef::entry`, 3 runs out of 3. The wrap campaign does not patch C, so `entry()` is now composed: `name`'s multivar iterator rereads every backend, lowest priority first, and decides C's result (the highest-priority failure above every backend holding the key is returned). A snapshot, whose backends never reread, then hands out the entry or C's not-found error. Documented differences: lookups return distinct entries (no shared `ptr_eq`), and a failure in a backend *below* the key's is spent on a successful lookup (the thread's error is restored as C leaves it). Regression: `ub_safe … rewritten_unparsable_file_fails_lookup_cleanly`. Corrected behaviour: `unit_tests::entry_reports_failed_rereads`. The same C defect is reachable through the typed getters of the symbol batch; see below. Otherwise checked by hand, with no other candidate: entries outliving their config, repository, snapshot, view, refresh and writes (the list refcount keeps strings live, and an entry holds no repository pointer); `ConfigMap` variance (covariant record and `Ref`, `Mut` invariant in `'s`); setters keeping String's non-NULL invariant; getters lending `'a ≤ 's`; aliasing between `as_ref`/`as_mut` pointers. New ub_safe tests cover entries surviving snapshots, views, writes and multivars under ASan/LSan, and maps across threads under TSan and Miri. |
| equivalence | No defect. New `equiv` tests: Rust-built mapping tables (a `static` one, then four `ConfigMapMut` edits) dispatched by C's `git_config_lookup_map_value` match independently built C tables over every prefix and input. An entry-selection sequence matches C step by step: two levels, a multivar, a write, a snapshot, level and HIGHEST views, a lower-priority file broken while the key lives above it (C never rereads it), then repaired. Every acquired entry reads the same after both configs are freed. No step asks C to look up through a failing backend, since that is the UB above. |
| internal | No defect found in the private implementation. |
| conventions | 1 report, fixed: `conventions/entry-map-unit-tests-split-module-imports`. The batch's inline tests split `unit_tests`' imports and left rustfmt-rejected blank lines. |
| misc | 4 reports: 2 fixed, 2 documented C defects. `misc/configmap-not-send-sync-const-or-debug`: `ConfigMap` could not form a `static` table (not `Sync`, `new` not `const`) and had no `Debug`. It now has `const fn new`, `Send`/`Sync` for the record and handles, `Debug`, a `static` doc example and a `compile_fail,E0597` doctest. This mirrors wave 0's fix to `BackendMemoryOptions`. `misc/entry-handles-lack-debug`: `Debug` for `Entry`/`EntryRef`/`EntryMut`. Documented, found by the new allocation-failure sweep over the composed lookup: `misc/config-list-dup-corrupts-heap-on-allocation-failure` (every file-backend iterator and snapshot duplicates the entry list, and `git_config_list_dup_entry`/`_append` leave the map pointing at a freed entry when an allocation fails; `ConfigRef::snapshot` already reached it) and `misc/multivar-iterator-leaks-inner-on-allocation-failure`. The safe API cannot inject allocation failures (`set_allocator` is `unsafe`). Both are noted on `ConfigRef::entry`, and the sweep avoids them and says why. |
| coverage | Rust (`cargo llvm-cov`, coverage variant): in the entry/map block of `config.rs` (from `define_ctype!(Entry..)` to `unit_tests`) the only zero-count line is the `None => {}` arm of the thread-error restore, for an error class the header does not name. `config.rs` overall is 98.0 % of lines; the rest belong to other batches. The new `entry_allocation_failures_leave_config_usable` covers the snapshot-failure return. C (private `GCOV_PREFIX` gcov of the coverage build): fully covered are `git_config_entry_free`, `git_config_get_entry`, `git_config_snapshot`, `git_config_next`, `config_file_iterator`, `git_config_lookup_map_value`, `git_config_list_entry_free` and `config_list_free`. Remaining: in `get_entry` the `GET_NO_ERRORS` branch (only the internal `git_config__lookup_entry`) and the failing-backend branch (now reached only by C callers); in `git_config_multivar_iterator_new`/`multivar_iter_next` the regex branches (the lookup passes no regex) and the leaking allocation failure; in `all_iter_next` a backend iterator's `next` failing (built-in list iterators cannot); `config_snapshot_get`/`_open`'s mutex and allocation failures (environment-dependent, or the list-dup crash above). |

## Validation of the landed tree

Rebased onto `1317ac5cb` (the symbol review, which renamed `open_ondisk`/`add_file_ondisk`
to `open`/`add_file`; the tests and the reproducer use the new names), then validated
again. The shared prebuilds were reused: no C, header, CMake or `build.json` change since
setup `74e3b602a`, and the batch changes no compiled C (only the bindgen allowlist).
`cargo check --workspace` and `cargo clippy --workspace --all-targets` are clean (0
warnings). `cargo test --workspace` under plain, ASan/UBSan (LSan on) and TSan
(`-Zbuild-std`) gives `lib` 610 passed / 1 ignored, `equiv` 385, `ub_safe` 642
(`compile_fail` included), `ub_unsafe` 91, doctests 43 + 241, all clean. Miri: the config
unit module (9 Rust-only tests pass; 17 that call libgit2 are ignored) and the two map
suites in `ub_safe`. On the rebased commits the reproducer exits 1 under UBSan on the
affected commit (3/3) and 0 on the fixed one. BSan ran on the reproducer alone and is clean
on the fixed tree. `rustfmt +stable --check` is clean on `config.rs` and both touched test
files. `crustify scan-unsafe` against the wave tip: no smell site; `unsafe_impls` +6,
`unsafe_blocks`/`ffi_calls` +3, `ref_to_type_wrapper_sanctioned` +2; undocumented sites
unchanged (44).

## Noted, not changed

- **The typed getters share the UB.** `git_config_get_bool`, `_int32`, `_int64`,
  `_string`, `_string_buf` and `_path` call the same `get_entry`. A probe calling
  `ConfigRef::get_i32` on a rewritten unparsable file aborts the same way (trace in the
  UB report). Those wrappers belong to wave 1 batch 1, under the concurrent symbol review,
  and `git_config_get_mapped` comes in wave 2. They are left unchanged to avoid colliding
  with that review. Each is `get_entry` plus a parser over `entry->value`, so it can be
  rebuilt on `ConfigRef::entry` and the module's `parse_*` functions.
- **Iterating custom backends.** The composed lookup ends because built-in backends fail
  only while opening their iterator, after which libgit2 moves on. A custom backend whose
  iterator's `next` keeps failing would make it loop. No safe wrapper adds one yet. Wave 5
  (`git_config_add_backend`) should keep this in mind.

- **Custom backends' entries.** `EntryArc` is not tied to its config. That is correct for
  every built-in backend, whose entries pin their own list. A custom backend's entry
  `free` could reach the freed backend. No safe wrapper adds custom backends yet. Wave 5,
  which wraps `git_config_add_backend` and `git_config_backend`, should require entries
  that own their storage, or tie `EntryArc` to the config for such backends.
- **`ConfigMap` naming.** By the naming rule `git_configmap` would become `Map` in
  `config::config`, as `git_configmap_t` became `MapType`. `ConfigLevel` and `ConfigError`
  keep the prefix too, and wave 0's review left them. A rename is churn for later
  consumers, so it is left as is.
