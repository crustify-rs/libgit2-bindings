# Review: libgit2 / config / wave 4 follow-up / batch 0 (`git_config_backend`)

Reviewed commit `df0deef13`, the review's base. It covers the one `type` item the
follow-up `a66691e90` landed again after the revert `c5440d3ee`:

| item | Rust | home | anchors |
|---|---|---|---|
| `git_config_backend` | `CustomBackend` + `UnopenedBackend::new`/`new_readonly` (`Custom<B>` trampolines, `Entered` reentry guard, `SharedEntry` kept answers, `Entries` iterator), on top of wave 4's `Backend`/`BackendBox`/`BackendMut` surface; `BackendEntry::new`/`EntryOrigin`, `ConfigError::new`/`with_code` | `config/sys/config.rs` (and `config/config.rs`) | `Wraps:` + 14 `Field:` (unchanged) |

Every trampoline was re-read against what `config.c` calls in each slot and what it does
with the result: `get_entry` (`:790-830`), `all_iter_next` (`:436-494`),
`git_config_snapshot` (`:182-212`), `git_config_add_backend` (`:290-391`), the writers
(`:660-730`, `:1167-1185`), `git_config_lock`/`_unlock` (`:1307-1340`), and the 25
callers of `git_config_entry_free` outside the backends. The last were checked for
use-after-free of a writable backend's entry: every one reads the value before it frees
the entry, and only `git_config_get_string` keeps a value, which the follow-up handles.

Config's waves are 0–5 (`schedule.json`). This follow-up repeats wave 4, so it is not the
sub-campaign's last wave, and BSan ran only on reproducers. This review emitted no UB
reproducer, so BSan did not run (see "Not covered" below).

| step | outcome |
|---|---|
| lifecycle discovery | Complete; no report. Construction: `Backend::new` (inert `git_config_init_backend` template) and `UnopenedBackend::new`/`new_readonly` (the author path). Opening: `BackendOpen`/`UnopenedBackend::open`, or `ConfigMut::add_backend`. Copy: the `snapshot` slot (`BackendMut::snapshot`, opened at once). Release: `BackendFree` (the allocation's own `free`, deferred while a callback runs). `nm -D` of the shared plain build exports only `git_config_init_backend`, `_add_backend`, `_backend_foreach_match` and `_backend_memory_options_init`. Every `git_config_backend **` producer in the headers (`from_string`/`from_values`, and `from_file`/`snapshot` in `config_backend.h`) is unexported. |
| wavefront findings | 1 report, resubmitted: `wavefront/backend-slot-implementer-contracts-incomplete`. The `get` note left out that a miss must not set an error. The `iterator`, `snapshot` and `lock` notes left out the C contracts that the trampolines depend on: iterator not-found skip, a failing `next` re-called, unopened read-only snapshots freed on open failure, and `lock` left held when the transaction allocation fails. The other notes and every ownership flag are correct. |
| safe boundary | Clean; no report. The follow-up adds no public `unsafe fn`, no public raw pointer and no `Send`/`Sync`. `CustomBackend: 'static`, `type Entries: 'static`, and the per-call `&CStr`/`RepositoryRef<'_>` arguments stop a store from keeping borrows (compile-fail doctests E0597 and the lifetime error). `UnopenedBackend`, `BackendBox`, `Config` and `Repository` are not `Send`, so a non-`Send` store never changes threads. `crustify scan-unsafe --sites all` finds no smell in the follow-up's lines: the only one in these files is wave 4's `IteratorMut::set_backend`. Undocumented count 45, none in config. |
| UB | No true positive. Checked by hand: the trampolines' provenance (the Miri unit tests dispatch through `BackendMut`), the reentry guard against every slot, release during a callback, field-disjoint borrows in `Entered`, `kept` answers against C's `get_string` contract, lent iterator entries, drop order (`kept` before the store), panics (they abort at the `extern "C"` boundary), and the C readers of custom entries (names are never NULL; NULL values take the existing value-filter path). A store whose `Drop` reaches its own backend could only do so through a raw pointer. That is unreachable from safe code and is not reported. ASan+UBSan (LSan on) and TSan pass on the whole workspace, and Miri passes on the unit tests. |
| equivalence | 1 report, fixed: `equiv/custom-get-miss-overwrites-thread-error`. A store's miss set "config value '<key>' was not found" as the thread's error. libgit2's backends set none, and `get_entry` depends on that. A Rust backend above the level holding `1kb` made `ConfigRef::get_i64` report the existing key as **not found** instead of unparsable. Regression: `equiv::sys_config::custom_backends::layered_misses_leave_errors_like_a_c_backend`. The existing equivalence tests used one backend and compared the thread's error only after failures, so they could not see this. |
| internal | None. |
| conventions | No defect. Names, homes, anchors (`UnopenedBackend::new` duplicates `Wraps: git_config_init_backend` on purpose: it is the same item's author path), `/// Handwritten` on the hand-written representations, `SAFETY` windows, rustfmt (`+stable`, edition 2024), and `cargo clippy --workspace --all-targets` all pass. |
| misc | 2 reports, fixed: `misc/unopened-open-doc-names-reverted-constructors` (`UnopenedBackend::open` described the removed memory constructors, and not what a custom store's `open` receives) and `misc/with-code-doc-misstates-kept-code` (`with_code(Ok)` keeps the current code, not `GIT_ERROR`). The second also adds the first test of `ConfigError::new`/`with_code`/`raise`. |
| coverage | See below. |

## Coverage

Rust (`cargo llvm-cov`, `coverage` variant; the workset's `lib`, `equiv` and `ub_safe`
tests): every line of the follow-up's code has counts, both before and after this review:
`CustomBackend`, `Custom`, `Entered`, `SharedEntry`, `Entries`, `OwnedEntry`,
`BackendEntry::new`, the `UnopenedBackend` constructors, and `ConfigError::new`/`with_code`/`raise`
(the last only now, through the new unit test). The review's own lines (`not_found`, the
`BackendMut::get` miss branch) are covered. `BackendRef::open_callback`
(`Field: git_config_backend.open`) had no test anywhere. The Miri unit test now opens
through it. The lines still uncovered in `sys/config.rs` are wave 4's `set_backend`
(covered by `ub_unsafe`, outside this filter) and the PCRE2 match-data OOM branch.

C (gcov of the shared `coverage` build with a private `GCOV_PREFIX`; same tests):
`git_config_add_backend` 15/15, `git_config_get_string` 9/9, `git_config_lock` 11/11,
`git_config_unlock` 8/8, `git_config_set_multivar` 6/6, `git_config_delete_multivar` 5/5,
`all_iter_next` 32/33 (31/33 before a new `ub_safe` case, a walk that fails on its first
entry, covered its `return error` after a fresh backend's first `next`, `:486`),
`git_config_snapshot` 17/18, `get_entry` 25/28. Still uncovered: `:490`,
which is dead after the loop; `git_config_new`'s OOM return (`:192`); and `get_entry`'s
`GET_NO_ERRORS` clear of a non-not-found failure (`:822-823`), which a `CustomBackend`
cannot reach by design, since its `get` can only succeed or miss.

## Not covered by this review

- **BSan.** The follow-up landed after wave 5's sub-campaign BSan gate (`5fc58a580`), so
  its trampolines have never run under BSan. Miri covers the dispatch through `BackendMut`
  and the release paths, but not the `get` answer path, `Entries::next`, or the
  `iterator`/`snapshot` trampolines: those tests release entries through C and are
  `cfg_attr(miri, ignore)`. This wave's rules allow BSan only on emitted reproducers, and
  there were none. Running the `sys_config` suites under BSan is worth doing once
  (`ub_safe`, `equiv` and the `lib` unit tests filtered to `sys_config`/`config::sys`).
- C defect noted, not reported (internal, outside the workset): `git_config__update_entry`
  (`config.c:733-761`) returns early, without freeing the entry it found, when it must not
  overwrite it, when the value is unchanged, and when deleting a valueless entry. Each
  such call leaks that entry (for file backends, a reference to the whole list).
