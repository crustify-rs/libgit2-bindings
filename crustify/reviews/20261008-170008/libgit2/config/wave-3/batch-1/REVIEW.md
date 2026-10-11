# Review: libgit2 / config / wave 3 / batch 1 (the wave's symbols)

Reviewed commit `0cf65b483` (the review-wave base), covering the three `symbol` items
of the wave. Batch 1 (`8a2a2d30c`) wrapped them all in `config/config.rs` on
`ConfigRef`, over wave 2's `ForeachCallback` trampoline:

| item | wrapper |
|---|---|
| `git_config_foreach` | `ConfigRef::foreach` |
| `git_config_foreach_match` | `ConfigRef::foreach_match(Option<&CStr>, …)` |
| `git_config_get_multivar_foreach` | `ConfigRef::foreach_multivar` |

Config has waves 0–5 (`schedule.json`), so wave 3 is not the sub-campaign's last and
there is no BSan gate here. This review emits no reproducer (its two reports are a
misc report and a Wavefront report), so BSan had nothing to run.

| step | outcome |
|---|---|
| lifecycle discovery | Not applicable: no `type` or `raw-lifetime` items. The walks own no lifecycle: C creates and frees the iterator inside the call. |
| wavefront findings | 1 report, resubmitted: `wavefront/enumeration-regexp-notes-conflate-filters`. `foreach_match`'s name-regex note was a copy of the multivar value-regex note. It said the safe wrapper omits the parameter, which is false, and left out the `-1`/`GIT_ERROR_REGEX` compile failure. The multivar `name` note said the copy is always freed; it leaks when the value pattern fails to compile. The other records (configs, callbacks, payloads, `lifetime: null`) were checked against the C and hold. |
| safe boundary | No defect. The workset adds no public `unsafe fn` and no public raw pointer, so there is no `ub_unsafe` suite. `scan-unsafe --sites all`: the only config site is still `raw_ptr_derefs_outside_impl` at `foreach_with`'s payload reconstruction (wave 2's documented trampoline seam). After the fixes, the scan counts `unsafe_blocks` 1223 → 1225 (all in `config.rs`). The source adds exactly one block, the composed walk's documented trampoline call. The error reporting uses the safe `set` helper, not variadic `git_error_set`. Undocumented sites stay at 44, and there is no new smell. |
| UB | No sanitizer-provable defect. Checked by hand: `walk_with` lends the payload as a raw child of `&mut self`, and the closure is borrowed exclusively by the callback. A panic is caught in the trampoline and resumed only after C has freed the iterator. Entries come from per-backend list copies (`config_file_iterator`, `config_memory_iterator` → `git_config_list_dup`), so a nested walk's refresh, or a write to the same file, cannot free an entry being lent. `all_iter` reads `readers` by index, and only `ConfigMut` can change it. A `(*UTF)` pattern meets invalid UTF-8 names through `pcre2_match` without `NO_UTF_CHECK`, so it is a reported non-match. The compile-fail doctests were checked against positive controls: each fails only for its stated borrow. The list-copy OOM crashes are reachable only through the `unsafe` allocator API. New ub_safe tests cover the repository-loaded `ConfigArc<'r>`, a level view and snapshots, independent walks on threads (TSan), and the new filtered path. |
| equivalence | No defect in the existing paths. The equivalence suite's `compare` now also runs every walk without an earlier error, which compares the "… callback returned N" message C leaves. |
| internal | Nothing further to fix. |
| conventions | No deviation. One filled `Wraps:` anchor per item, safety comments on every block, inline tests after the `unit_tests` imports, suites homed under `config.rs`. |
| misc | 1 report, fixed: `misc/multivar-foreach-drops-value-regexp`. `foreach_multivar` had dropped C's value regex because C's own filter runs `strlen(NULL)` on valueless settings. Batch 0 of the same wave added a safe PCRE2 value filter (`ConfigRef::multivar_iterator(name, Some(re))`). `foreach_multivar(name, regexp, callback)` now builds the filtered walk the way C does (`config.c:1057-1086`): construction errors, stop codes, `git_error_set_after_callback`'s message, iterator release, then `NotFound` with the caller's spelling. The unfiltered path still calls C. |
| coverage | See below. |

## Coverage

Measured with the `coverage` variant (`cargo llvm-cov` over the workset's `lib`, `equiv`
and `ub_safe` enumeration tests; C gcov through a private `GCOV_PREFIX`).

- **Rust**, `config/config.rs` from `foreach` to `ForeachFn`: no uncovered region. That
  includes both branches of `foreach_multivar`, every arm of the composed loop
  (entry, stop, exhaustion, backend error, construction error, not-found),
  `after_callback` with and without an existing error, and `foreach_result`.
- **C**, `config.c`: `git_config_foreach` 2/2, `git_config_foreach_match` 12/12,
  `git_config_get_multivar_foreach` 16/16, `git_config_iterator_glob_new` 15/15,
  `all_iter_glob_next` 8/8, `git_config_multivar_iterator_new` 23/23 and
  `multivar_iter_next` 12/12 lines. The equivalence references drive C's own value-regex
  branches, on names without valueless values only.

## Validation of the landed tree

See the second commit's message for the final run on the tree as landed. Before landing:

- `cargo check --workspace` and `cargo clippy --workspace --all-targets` produce no
  warnings.
- Base, plain: `lib` 623 / 1 ignored, `equiv` 395, `ub_safe` 657, `ub_unsafe` 92,
  doctests 43 + 262.
- Fixed tree, plain, ASan/UBSan (LSan on) and TSan (`-Zbuild-std`): `lib` 623 / 1
  ignored, `equiv` 397, `ub_safe` 660, `ub_unsafe` 92, doctests 43 + 262, all clean. The
  first ASan run caught C's reference leaking its normalized name on an invalid value
  pattern (40 × 4 bytes). That comparison now runs in a child process without leak
  detection; the wrapper's own leak freedom stays under LSan in ub_safe.
- Miri, config inline tests: 11 passed, 21 ignored (they call libgit2).
- `rustfmt +stable --edition 2024 --check` passes on the changed files, and
  `git diff --check` is clean.

## Noted, not changed

- **A filtered walk is composed, not C's.** Only `Some(pattern)` goes through the
  composition, and it differs from C in exactly two ways: valueless settings are
  skipped, and nothing leaks on an invalid pattern.
- **C's multivar iterator leaks `inner` if its own `calloc` fails**
  (`config.c:1139`, documented in wave 1). Both multivar paths reach it, but only
  through the `unsafe` allocator API.
- **The "error exists" test in `after_callback` reads the error class.** libgit2's
  `git_error_exists` is not exported. A thread error deliberately set with class
  `GIT_ERROR_NONE` is taken as no error, as `restore_last` already does.
- **Naming.** `foreach_multivar` drops `get_`, as getters do. The crate mixes `foreach`
  (`tag::foreach`) and `for_each_*` (`for_each_fetchhead`), so neither spelling is a
  convention deviation.
