# Review: libgit2 / object / wave 3 / batch 0 (the wave's symbols)

Reviewed commit `26bf1e7a0` (the review wave's base). It covers the six `symbol` items
that wave 3 batch 0 (`11ea2e65f`) landed or skipped, all homed in
`crustify/rust/libgit2/libgit2/object/commit.rs`:

| item | Rust | anchor |
|---|---|---|
| `git_commit_create_from_tree` | `create_from_tree`, composed in Rust | `Replaces:` |
| `git_commit_create_from_stage` | `create_from_stage`, composed in Rust | `Replaces:` |
| `git_commit_amend_from_tree` | `amend_from_tree` | `Wraps:` |
| `git_commit_amend_from_stage` | `amend_from_stage` | `Wraps:` |
| `git_commit_create_options_init` | `CreateOptions::new` (wave 2 batch 0's type run; not wrapped again) | `Wraps:` |
| `git_commitarray_dispose` | `CommitArrayDispose` (repository wave 0 batch 2's type run; not wrapped again) | `Wraps:` |

The two composed routines were re-read line by line against `commit.c:1219-1371`. The
empty-commit check, the default-signature and parent order, the `-1` versus
`GIT_EUNCHANGED` returns, and the error each failure leaves all match C, except for the
documented `git_repository_commit_parents` divergences.

Wave 3 is object's last wave (`schedule.json`), so this review owns the sub-campaign's BSan
gate, which covers every wave object landed (see below).

| step | outcome |
|---|---|
| lifecycle discovery | Not applicable: a symbol-route batch. The two lifecycle routines scheduled as symbols are the policies their type runs emitted. `CreateOptions::new` copies the template whole into zeroed storage; `git_commit_create_options_init` fails only for another version. `CommitArrayDispose` frees each commit and the buffer once, skips the empty array, and is never handed NULL. Both were re-read against `commit.c:57-66, 1477-1490` and not wrapped again. |
| wavefront findings | 1 defect, resubmitted: `wavefront/amend-head-author-read-from-a-fresh-parse`. The two amend records said a NULL author is "HEAD's commit's author" without saying libgit2 reads it from a parse of its own, shared with a caller's only through the object cache, which keeps no commit of 4096 bytes or more. That gap is the root of the internal defect below. The diff to the store is those two notes, spliced in without the tool's re-sort. The two create records were re-read and hold. |
| safe boundary | Clean. The workset adds no public `unsafe fn` and no public raw pointer. `crustify scan-unsafe --sites all` finds no `*_smell` site in `commit.rs`. The only new `unsafe` block, the options read in `amend_on_head`, is documented. `diff.h` stays the documented raw seam inside `changes`, until a diff wrapper exists. No `ub_unsafe` suite to review. |
| UB | No true positive. Candidates checked by hand: the signer trampoline's `&mut` payload against the exclusively borrowed options (and the `with_ext_options` / amend copies that share it while the original is frozen); `git_index_write_tree` through a shared index owner (`IndexRef` lends nothing, `Repository` is neither `Send` nor `Sync`); the diff's `const`-cast tree; `held` and the default signature outliving the call; reentrant commits from a signer; foreign trees and unwritable signatures, refused before C. ASan/UBSan (with LSan) and TSan are clean on the whole workspace. Every BSan report is a false positive (gate below). The `(null)` author below is UB in C (a NULL `%s`), but no sanitizer reports it (glibc prints `(null)`), so it is filed as internal. |
| equivalence | No defect. The two composed routines match C on every compared path. 4 steps were added to `tests/equiv/commit.rs::create_options` that amend commits too large to be cached, with the author unset and with no options; history 21 → 24 commits. These run the new copy path against C after each step. The documented divergences (`HEAD` at a tag or a tree, foreign trees, unwritable signatures, the `git_repository_commit_parents` use after free) are unchanged. |
| internal | 1 defect, fixed: `internal/amend-author-relies-on-the-object-cache`. With no author set, the amend wrappers checked `HEAD`'s author in their own parse and let libgit2 write the author of its own peel, assuming the cache shared them. For a commit over 4096 bytes it does not, and a refusing allocator made both amends write `(null)` as the author. `amend_on_head` now passes libgit2 a copy of the options (C's defaults for `None`) with the checked author set. Regression: `commit.rs::unit_tests::uncached_amended_authors_are_the_ones_checked`, which fails on the base with `left: Some("(null)")`. |
| conventions | 1 report, fixed: `conventions/commitarray-dispose-safety-outside-the-scanned-window`. `CommitArrayDispose`'s `SAFETY` comment (seven lines) sat outside `scan-unsafe`'s six-line window; `undocumented_unsafe` goes 45 → 44. Otherwise clean: `Replaces:` / `Wraps:` anchors, names, handle shapes. The touched files were rustfmt-clean at the base and still are (`rustfmt +stable`). `cargo clippy --workspace --all-targets` is clean. |
| misc | 3 reports, all about the BSan gate (below): `misc/bsan-reports-variadic-parents-as-freed`, `misc/bsan-reports-strtoul-end-pointers` (false positives, each with a `#![forbid(unsafe_code)]` probe and its log) and `misc/bsan-gate-outlasted-by-slow-tests`. The 4 + 8 tests concerned now carry `#[cfg_attr(bsan, ignore = ...)]`; every other instrument still runs them. |
| coverage | See below. |

## Coverage

Rust (`cargo llvm-cov`, `coverage` variant, `commit` tests across all suites): the only
uncovered line in the workset's code (`options_writable`, `changes`, `commit_on_head`,
`create_from_*`, `head_commit`, `amend_on_head`, `amend_from_*`, `CreateOptions::new`,
`CommitArrayDispose`) is `commit_on_head`'s `unreachable!` arm.

C (gcov of the `coverage` build, private `GCOV_PREFIX`): at the base, `create_from_tree`'s
`git_repository_commit_parents` failure (`commit.c:1302`) was the one reachable line the
suites left unreached. C only gets there without its use after free when `HEAD` itself
cannot be read. `commit.rs::unit_tests::unreadable_heads_fail_collecting_parents_as_libgit2_does`
now compares both creating routines with C on a `HEAD` that is not a reference ("corrupted
loose reference file: HEAD", `-1`), with empty commits allowed. Still uncovered:
`git_commitarray_dispose`'s NULL return (`:1482`), which `CommitArrayDispose` never takes.

## BSan gate (the sub-campaign's)

One `--no-run` build, then one `cargo-variant.sh bsan test` per test binary, in parallel,
each under `timeout 2400`. Each ran from a snapshot of the tree (`git archive` plus
`git init`, `include/`, and the builds and `tests/` links), so work could go on in the
worktree. Logs: `bsan-gate-before/` and `bsan-gate-after/`.

- **Wave tip `26bf1e7a0`, before any fix.** `libgit2-sys` and `ub_unsafe` (91 passed) were
  clean. Results needing triage:
  - `lib` aborted 12 s in on `every_array_length_writes_the_slice_commit`. A rerun skipping
    that test passed 579 tests and was killed at 2400 s on
    `create_updated_refuses_overlong_components`.
  - `equiv` and `ub_safe` were killed at 2400 s with four and three tests still running.
  - Failed: `equiv` `commit::creating::{creation_scripts_match, unwritable_object_databases_fail_alike}`
    (the variadic false positive), `equiv` `signature::identity_and_dates_from_the_environment`
    (the `strtoul` false positive), and `ub_safe`
    `repository::head_and_lookups::first_discoveries_race_to_pin_the_runtime`. The last
    failed only because the snapshot was not yet a git repository, and passed alone once it
    was (`bsan-gate-before/alone/`).

  Each straggler was rerun alone under `timeout 3600`. Two passed (1628 s and 2600 s) and
  six were killed while on-CPU, with no report. No report survived triage: none
  reproduces as a real access through the Rust side, and ASan/UBSan is clean on all of
  them.
- **The tree landed here, with no `--skip`.** `libgit2-sys` and `ub_unsafe` (91 passed)
  clean; `lib` 581 passed and 4 ignored, in 52 s; `ub_safe` 611 passed and 5 ignored, in 457 s. The first `equiv` run had
  364 of 365 tests clean when it was killed at 2400 s, with only `content_checks_match_c`
  still running (`bsan-gate-after/equiv-before-content-checks-marker.log`). Once that test
  was marked too, `equiv` was rerun on the tree landed here, the only binary the marker
  changes: 357 passed and 8 ignored, in 1384 s. No BorrowSanitizer report. The 17 ignored
  tests are the 12 marked here, 4 BSan markers from earlier waves, and the 2 GiB blob scan,
  which every variant ignores.
- An intermediate run on the fixes, before the stall markers, skipped the eight by name
  and was clean too: `lib` 581 passed, `ub_safe` 611, `equiv` 357, `ub_unsafe` 91.

## Validation

On the landed tree:

- `cargo check --workspace`, `cargo clippy --workspace --all-targets`, and
  `cargo test --workspace` (plain, `compile_fail` and doctests included): clean.
- ASan/UBSan (LSan on) and TSan (`-Zbuild-std`) on every workspace test binary with
  `--skip compile_fail`: clean. These runs predate only the `cfg_attr(bsan, ignore)`
  markers, which do not compile in outside BSan. TSan reran the `equiv` commit tests after
  the last change to them.
- Miri on `commit.rs`'s Rust-only unit tests (17 passed, the rest `ignore`d as calling
  libgit2).
- BSan as above.
- `crustify scan-unsafe` on the tree: no smell site in the workset, `undocumented_unsafe`
  45 → 44.

No C changed, so no C baseline was rerun.
