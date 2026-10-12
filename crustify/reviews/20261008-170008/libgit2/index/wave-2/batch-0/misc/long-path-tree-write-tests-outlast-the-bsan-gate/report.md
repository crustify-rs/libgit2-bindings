# Two long-path tree-write tests outlasted the sub-campaign's BSan gate

- **items**: `git_index_write_tree` / `git_index_write_tree_to` (libgit2/index wave 1
  batch 3, `7f4d85bd7`) and their tests
  `index.rs::unit_tests::edits::tree_writes_refuse_foreign_ids_and_long_names` and
  `tests/ub_safe/index/edits.rs::tree_writes_refuse_foreign_ids_without_overflow`
- **class**: misc (test suite shape; no BorrowSanitizer report, no defect in the code under
  test)

## Finding

This wave is the index sub-campaign's last (`schedule.json`: waves 0-2), so this review ran
its BSan gate: one `cargo-variant.sh bsan test` per test binary, each under
`timeout 2400`. On the wave tip `b9d01cafd` (`bsan-gate-before/`), `equiv`, `ub_unsafe` and
`libgit2-sys` passed. `lib` and `ub_safe` were killed by the timeout (exit 124) with every
other test finished and no BorrowSanitizer report. The one test left running in each was
one of these two (`gate.log`; `… has been running for over 60 seconds` with no result line
in `libgit2_lib.log` / `ub_safe.log`).

Each test adds an entry whose path has a component longer than a tree entry's
65535-byte name (65536 and 70000 bytes), then asks for a tree. A timing probe of the `lib`
test under `cargo bsan`, run alone on the review's tree, printed the elapsed time after
each statement. Every step up to the long entry took about 95 ms in total. Adding the
65540-byte path `dir/n…n` (`git_index_add`, instrumented C) took 8.4 s. `write_tree_to`, whose
refusal is the Rust scan of the entry's path components (`IndexRef::tree_writable`), did
not return in the more than 20 minutes left before the probe's `timeout 1500`. Plain, ASan and TSan run each test in
milliseconds. So these are allocation- and byte-heavy inputs that BorrowSanitizer cannot
finish in a gate, as object wave 0's review found for its mailmap corpus. It is not a hang. While they run, the
whole binary's verdict is lost: `lib` and `ub_safe` hold about 1460 other tests.

## Fix

Each test was split in two, keeping its foreign-id half under BSan:

- `tree_writes_refuse_foreign_ids` (lib) and `tree_writes_refuse_foreign_ids_without_overflow`
  (ub_safe) keep the foreign-id, zero-id, submodule and `GIT_EUNMERGED` checks and still
  run under every instrument.
- `tree_writes_refuse_long_names` (lib, and the same name in ub_safe) holds the long-name
  checks unchanged, with the campaign's existing marker
  `#[cfg_attr(bsan, ignore = "its large inputs outlast the BorrowSanitizer gate, over an hour alone; the other variants run it")]`.

The final gate (`bsan-gate-after/`) runs every binary to completion.

