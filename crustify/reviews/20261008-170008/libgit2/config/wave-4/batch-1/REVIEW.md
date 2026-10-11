# Review: libgit2 / config / wave 4 / batch 1 (the wave's symbols)

Reviewed commit `e797d4624` (the review-wave base). It covers the five `symbol` items that
wave 4 batch 1 (`8491945f1`) landed in `config/config.rs`:

| item | wrapper |
|---|---|
| `git_config_iterator_new` | `ConfigRef::iterator` |
| `git_config_iterator_glob_new` | `ConfigRef::iterator_matching(Option<&CStr>)` |
| `git_config_multivar_iterator_new` | `ConfigRef::multivar_iterator(name, Option<&CStr>)`: C's name selection, plus wave 3's PCRE2 value filter for `Some` |
| `git_config_next` | `IteratorMut::advance_entry`, which lends an `EntryRef` |
| `git_config_iterator_free` | no new wrapper. Wave 3's type run already emitted it as the `IteratorFree` release policy of `IteratorBox` (`config/sys/config.rs`), so the symbol route skips it |

Config has waves 0–5 (`schedule.json`), so wave 4 is not the sub-campaign's last wave and
this review runs no BSan gate. It found no UB or equivalence defect, so it emits no
reproducer and BSan had nothing to run.

| step | outcome |
|---|---|
| lifecycle discovery | Not applicable: no `type` or `raw-lifetime` items. |
| wavefront findings | No defect. The five records (`_analysis.pending` empty) match `config.c:427-575, 1087-1205` and `config_list.c:225-262`. Constructors write a fresh sole owner only on success. They keep an uncounted config borrow and copy or compile `name`/`regexp` during the call. `next` lends an immutable entry pinned until the next advance or the free, leaves the output untouched on end or error, and allows a retry. `free` dispatches the allocation's own callback, and NULL is a no-op. |
| safe boundary | No defect. The workset adds no public `unsafe fn` and no public raw pointer, so there is no `ub_unsafe` suite. `scan-unsafe --sites all`: the eight unsafe blocks in the workset (`config.rs:1349-1476`) are documented wrapper-impl blocks. No smell counter has a site in that range, and the counts are identical before and after this review. |
| UB | No true positive. Checked by hand: every iterator is an `IteratorBox<'a>` bounded by the `ConfigRef<'a, 'r>` it came from, so it cannot outlive the config, its owner or its repository (`'r: 'a`). `advance_entry`'s entry borrows the `&mut` reborrow, so it cannot survive another advance (doctest E0499) or the owner (E0505). Mutating the config needs `ConfigMut`, which the borrow freezes. A `ConfigArc` holder cannot add readers either, and C never removes them, so `all_iter`'s index stays valid. Writes and reparses through another handle or the repository replace a file backend's list, but each backend iterator walks its own copy (`config_file_iterator` → `git_config_list_dup`). Nothing is `Send`/`Sync`. A failed advance builds no `EntryRef`. The list-copy OOM crash and the multivar shell leak are C defects reported in wave 1 and reachable only through the `unsafe` allocator API. New `ub_safe` test `public_entries_interleaved_views_snapshots_and_reparse`: four iterators over one config, a level view and a snapshot advance interleaved and are released in mixed order. A snapshot's iterator outlives its source and a rewrite of the file. A repository config is reparsed under a moved HEAD (`includeIf onbranch`) while an iterator lends an entry of the old parse. |
| equivalence | No defect. New `equiv` tests compare every step (entry fields, codes, class and message) against independent pure-C configs. `public_entries_across_levels_interleaved_match_c`: SYSTEM/GLOBAL/LOCAL files with an include, four iterator pairs (all, glob, filtered multivar, subsection-case multivar) advanced in turn through exhaustion and beyond, then a level view of a repository's config. `public_constructor_failures_and_empty_configs_match_c`: five key regexes (four invalid) and seven multivar name/regex inputs (invalid names fail before any value regex), plus an empty config's iterators. |
| internal | No defect beyond the conventions fix. The moved `advance_entry` unit test now also covers what the old fixture could not: a failure status keeps its code and thread error, a positive status becomes `GIT_ERROR`, and the iterator is reused after a failure and after the end. |
| conventions | 1 report, fixed: `conventions/iterator-symbol-tests-homed-in-type-module`. The constructors' allocation-failure test and the batch's `advance_entry` test stayed in `sys/config.rs` when the batch moved the wrappers to `config.rs`. They now live in `config::config::unit_tests::iterators`, which reuses that module's refusing allocator instead of a second copy. |
| misc | 1 report, fixed: `misc/multivar-iterator-docs-misstate-leak-scope`. The docs said C's allocation-failure leak applies only without a regex. It applies to both forms, because the regex never reaches C. The report also covers the empty `///` line left on `IteratorMut::advance`. |
| coverage | See below. |

## Coverage

Measured with the `coverage` variant: `cargo llvm-cov` over the `config` tests of `lib`,
`equiv`, `ub_safe` and `ub_unsafe`, with C gcov from a private coverage build of this
worktree (`/tmp/crustify-cfg-rw4-b1/coverage`).

- **Rust**, `config.rs` from `iterator` to `advance_entry`: every line with counts is
  executed, before and after this review.
- **C**, `config.c`: `git_config_iterator_new`, `git_config_iterator_glob_new`,
  `git_config_multivar_iterator_new`, `git_config_next`, `git_config_iterator_free`,
  `all_iter_glob_next`, `all_iter_free`, `all_iter_glob_free`, `multivar_iter_next` and
  `multivar_iter_free` are all at 100 %. `all_iter_next` is at 87.88 %. Lines 453 and
  486 are a backend iterator's own `next` failing, and line 472 is a backend's `iterator`
  returning `GIT_ENOTFOUND`. No built-in backend does either, and a custom backend can
  join a config only through `git_config_add_backend` (wave 5). Line 490 is the
  unreachable trailing return.

## Validation of the landed tree

Wave 4 batch 0 changed `include/git2/sys/config.h` (two `GIT_EXTERN`s), so the shared
prebuilds lack exports the tree needs. The runs use batch 0's private builds
(`/tmp/crustify-config-w4-b0/<variant>`). Their C inputs equal this tree's: nothing outside
`crustify/` changed since `beeda6a07`. They were built with `build.json` version 1, Clang
18.1.3 and each variant's instrumentation. No C changed here, so no C baseline was run.

- `cargo check --workspace` and `cargo clippy --workspace --all-targets`: no warnings.
- `cargo test --workspace`, plain and ASan/UBSan (LSan on): lib 630 / 1 ignored (as at
  the base), equiv 405, ub_safe 667, ub_unsafe 92, sys 43, doctests 271. All passed.
- TSan (`-Zbuild-std`): the same counts, all passed, no report.
- Miri, `libgit2::config::` inline tests: 20 passed, 29 ignored (they call C).
- BSan: not run. This is not the sub-campaign's last wave, and the review emits no
  reproducer.
- `rustfmt +stable --edition 2024 --check` is clean on the four touched files, as at the base.

## Noted, not changed

- **`advance_entry` borrows its handle.** `let e = iter.as_mut().advance_entry()?;` does
  not compile, because the entry would outlive the temporary handle. Callers name the
  handle instead (`let mut m = iter.as_mut();`). A consuming `advance_entry(self) ->
  …EntryRef<'a>` would also be sound and would accept that one-liner. Switching is an API
  choice, not a defect.
- **A positive status from a custom iterator** comes back as `GIT_ERROR` (`-1`) where C's
  `git_config_next` returns it unchanged, following `ConfigError`'s documented
  always-negative code. Built-in iterators never return one. It becomes observable once
  wave 5 lets custom backends join a config.
