# Review: libgit2 / config / wave 3 / batch 0 (the wave's types)

Reviewed commit `0cf65b483` (the review-wave base), rebased onto `64681cd5c` (the wave's
symbol review) before landing. It covers the one `type` item that
wave 3 batch 0 (`0de2b59b4`) landed:

| item | wrapper | home |
|---|---|---|
| `git_config_iterator` | `Iterator<'c>` / `IteratorRef` / `IteratorMut` (hand-written `CCell`), `IteratorBox<'c> = CBox<Iterator<'c>, IteratorFree>`, `IteratorFree` (`git_config_iterator_free`), `IteratorNext` (`next` slot), constructors `ConfigRef::iterator` / `iterator_matching` / `multivar_iterator`, `IteratorMut::advance` (`git_config_next`), field accessors `flags` / `set_flags`, `backend_raw` / `unsafe set_backend_raw`, `next_callback`, `release_policy`, and the Rust-owned PCRE2 value filter (`value_filter`) | `config/sys/config.rs` |

Config has waves 0–5 (`schedule.json`), so wave 3 is not the sub-campaign's last wave and
this review runs no BSan gate. It found no UB or equivalence defect, so it emits no
reproducer and had nothing to run BSan on.

| step | outcome |
|---|---|
| lifecycle discovery | No gap. Public producers of a `git_config_iterator`: `git_config_iterator_new`, `git_config_iterator_glob_new` and `git_config_multivar_iterator_new` (all wrapped, sole owners) and the `git_config_backend.iterator` vtable slot (the backend type is wave 4). Every `git_config_iterator **` in `include/git2` was checked. The only releaser is `git_config_iterator_free`, which dispatches the allocation's own `free`. No clone, up-ref or init exists. `query symbols --lifetime-for git_config_iterator` agrees: one dropper, three `alloc` constructors. Owned (`IteratorBox`) and borrowed (`IteratorRef` / `IteratorMut`) forms match the C contract: unique ownership, entries lent until the next advance. |
| wavefront findings | No defect. Field notes (`next` / `free` fixed per allocation, `next` lends and never acquires, `backend` a nullable borrow that built-ins leave NULL and never read) and the constructor/dropper records match the C. Checked against `config.c:427-575, 1087-1205` and `config_list.c:225-262`. |
| safe boundary | 2 reports, fixed (`unsafe/iterator-adoption-helper-is-a-raw-unsafe-fn`, `unsafe/value-filter-callbacks-project-outside-impl`). Kept as is: `backend_raw()` / `unsafe set_backend_raw()`, the documented raw seam for `git_config_backend` (wave 4 batch 0, a cut SCC). The setter's `# Safety` contract was checked against C: built-in `next`/`free` never read the slot, and a custom iterator may depend on it, so keepalive for `'c` plus subclass compatibility is the right obligation. `ub_unsafe/sys_config.rs` exercises it within contract. `from_ptr` / `as_ptr` / `as_mut_ptr` are `pub(crate)`. Scan deltas: `unsafe_fns` −1, `unsafe_fns_priv_smell` −1, `raw_ptr_priv_smell` −1, `field_proj_outside_impl` −1, `raw_ptr_derefs_outside_impl` −2. Undocumented sites stay at 44 crate-wide, none in the workset. |
| UB | No true positive. Checked by hand: entries borrow the `&mut IteratorMut` reborrow, which is bounded by `'c`, so they cannot survive an advance, the iterator or the config (C frees the previous backend iterator's list on advance). The config borrow freezes `ConfigMut` (doctest E0502), and C never removes readers, so `all_iter`'s stored index stays valid. No `Send`/`Sync` is asserted. ffibox's `CBox` builds handles from its `NonNull` without forming references, so the `Filtered` payload keeps full-allocation provenance from `Box::into_raw` to `Box::from_raw`. A panic inside the `extern "C"` filter callback would abort, but none is reachable for built-in backends. The `IteratorMut` invariance doctest is not vacuous: the same signature over the covariant `IteratorRef` compiles. ASan/UBSan (LSan on), TSan and Miri (the Rust-only iterator test) are clean. |
| equivalence | No defect. Probes compared C's and Rust's multivar regex compile failures (code, class, PCRE2 message, name check first) for nine patterns: identical. Both link the same system `libpcre2-8.so.0` (pkg-config `pcre2-sys` and libgit2's `USE_REGEX=pcre2`; checked with `ldd`). New `equiv` tests: `filtered_multivar_steps_match_c` (Rust's filter against C's own regex multivar iterator, step by step: entries of the lower backend, the higher backend's refresh failure, retries after failure and exhaustion, and the thread error after each step, then a fresh pair over the repaired file) and `multivar_regex_errors_match_c` (OS and undefined-class error seeds, run in a child with LeakSanitizer off because C leaks the normalized name on this path; confirmed: 56 bytes in 14 objects with LSan on). The documented divergence stays: C's `strlen(NULL)` on a valueless entry. |
| internal | No defect beyond the conventions fixes. New inline test `unit_tests::iterators::allocation_failures::constructors_report_allocation_failure`: a counting libgit2 allocator in a child process refuses the all/glob iterator's only allocation and the multivar name copy. Each constructor returns `NoMemory`, and the next call succeeds. C's leak when the multivar shell itself is refused (`config.c:1139`, reported in wave 1) is skipped, as is file-list duplication. |
| conventions | 2 reports, fixed: `conventions/iterator-unit-tests-outside-unit-tests` (a second top-level test module) and `conventions/value-filter-policies-break-declaration-pattern` (derive above docs, no `Debug`). |
| misc | 1 report, fixed: `misc/ub-safe-suite-doc-covers-options-only`. New `ub_safe` test `filtered_iterator_release_points`: the filter released before, during and after iteration, over a backend-less config, and after failed compiles. |
| coverage | See below. |

Noted, not changed: `IteratorMut::advance` borrows the handle, so
`let e = iter.as_mut().advance()?;` does not compile. Callers must name the handle
(`let mut m = iter.as_mut();`). Returning `'a` from `&mut self` would allow two live
entries, so this is the sound shape. A consuming `advance(self)` would be the ergonomic
alternative, if the wave-4 symbol review wants one.

## Coverage

Measured with the `coverage` variant (`cargo llvm-cov` over the `iterator`-filtered `lib`,
`equiv`, `ub_safe` and `ub_unsafe` tests) and C gcov through a private `GCOV_PREFIX`.

- **Rust**, the iterator block of `config/sys/config.rs` (the `Iterator` docs up to
  `mod unit_tests`): 172 of 174 lines with counts are executed. The batch left
  `iterator()`'s failure return uncovered; the allocation-failure test now reaches it.
  Still uncovered: the PCRE2 match-data allocation failure in `matches` (`set_oom`).
  PCRE2 allocates through libc, not libgit2's allocator, so no test reaches it without
  interposing `malloc`.
- **C**: `git_config_iterator_new`, `git_config_iterator_glob_new`, `all_iter_glob_next`,
  `all_iter_glob_free`, `all_iter_free`, `git_config_next`, `multivar_iter_next`,
  `multivar_iter_free`, `git_config_list_iterator_new`, `config_iterator_next` /
  `_free` and `git_regexp_match` are at 100 %. `git_config_multivar_iterator_new` rose
  from 91.30 % to 100 % (name-copy failure, regex-compile failure in the equivalence
  child). `all_iter_next` is at 87.88 %. Its uncovered lines are a custom backend
  iterator's own `next` errors (`config.c:453, 486`), a backend `iterator` returning
  `GIT_ENOTFOUND` (`:472`, no built-in does), and the unreachable final return (`:490`).
  Custom backends need the wave-4/5 backend wrappers. `git_config_iterator_free`'s NULL
  no-op is unreachable (owners are non-NULL). `git_regexp_compile`'s `ICASE` flag is
  never passed by config.

## Validation of the landed tree

The work was rebased onto `64681cd5c`. That symbol review now composes
`git_config_get_multivar_foreach` over this workset's `multivar_iterator` value filter;
its own files did not conflict. Everything below was run again on the rebased tree.

No C, header, CMake, bindgen allowlist or `build.json` changed, so the shared prebuilds
were reused and the C baseline was not rebuilt. Rust outputs are private to this
worktree's `crustify/rust/target/`, and gcov data went to `/tmp/crustify-cfg-rw3-b0/gcov`.

- `cargo check --workspace`, `cargo clippy --workspace --all-targets`: clean (0 warnings).
  `rustfmt +stable --edition 2024 --check` is clean on the three touched files, which
  were clean at the base.
- `cargo test --workspace`, plain: lib 624, equiv 399, ub_safe 661, ub_unsafe 92,
  doctests 43 + 262, all passing.
- Same counts, all passing, under ASan/UBSan (LSan on) and TSan (`-Zbuild-std`), no
  sanitizer report.
- Miri: `config::sys::config` unit tests, 7 passing, 4 ignored (they call C).
- BSan: not run. This is not the sub-campaign's last wave, and the review emits no
  reproducer.
