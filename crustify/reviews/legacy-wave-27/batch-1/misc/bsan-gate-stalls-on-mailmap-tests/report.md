# The wave's BSan gate times out on three `git_mailmap` tests, with no BorrowSanitizer report

- **item**: `git_mailmap` (type; libgit2/object wave 0 batch 2, `5c83b2e99`), tests
  `crustify/rust/libgit2/libgit2/object/mailmap.rs::unit_tests::classifier_agrees_with_libgit2`,
  `crustify/rust/libgit2/tests/ub_safe/mailmap.rs::adversarial_buffers_parse_without_ub` and
  `crustify/rust/libgit2/tests/equiv/mailmap.rs::buffers_match` — outside this batch's
  workset (`git_blob_data_is_binary`, `git_tag_name_is_valid`), which is why no fix is
  landed here (see below)
- **affected commit**: `6583ab4ad` (the review wave's base)
- **class**: gate failure; no UB and no wrong answer — BorrowSanitizer never finished
  the three tests inside the gate's `timeout`

## Finding

This batch's BSan gate on the wave base (`cargo-variant.sh bsan test`, one invocation
per test binary in parallel, each under `timeout 2400`, after one `--no-run` build)
ended with `rc=124` for the `lib`, `ub_safe` and `equiv` binaries (`bsan-gate.log`);
`ub_unsafe` (99 passed, 133 s) and the `libgit2-sys` binary passed. In each killed
binary exactly one test was still running, and every other test had reported `ok` or
its documented `ignored`:

| binary | passed before the kill | still running after 2400 s |
|---|---|---|
| `lib` | 440 of 443 (2 ignored) | `libgit2::object::mailmap::unit_tests::classifier_agrees_with_libgit2` |
| `ub_safe` | 433 of 436 (3 ignored) | `mailmap::adversarial_buffers_parse_without_ub` |
| `equiv` | 287 of 289 (1 ignored) | `mailmap::buffers_match` |

None of the three logs holds a BorrowSanitizer report, a panic or a `FAILED` line. The
`lib` straggler runs in an isolated child process (`CRUSTIFY_UNIT_CHILD`), which was
on-CPU the whole time (31 min of CPU after 31 min of wall time when inspected, state
`R`), not blocked: it feeds every line of a generated corpus to
`git_mailmap_from_buffer`, and the other two parse 64 copies of the reference mailmap,
a 64 KiB name, and every prefix of the reference file, which is the many-iteration,
large-buffer shape the campaign already records as stalling BorrowSanitizer's GC
safepoints (the libgit2/core wave 0 review's
`misc/bsan-gate-filemode-probe-stall`, and the `cfg!(bsan)` strides in
`tests/ub_safe/{refdb,tree,index}.rs`).

Because the three tests never complete, the gate cannot show the mailmap wrappers
clean under BorrowSanitizer, and any other binary-wide run of the suite inherits the
same 40 min ceiling.

## Why no fix is landed here

`git_mailmap` is an item of this wave's type-route review batch (`review-wave-0/batch-0`),
which was running its own BSan gate on the same tree while this one ran, and whose
landing touches the same three files. A concurrent edit to its tests from this batch
would collide on the wave branch. The fix is the one the core wave 0 review applied
to its probe: take a `cfg!(bsan)` stride over the corpus / the prefix loop, or run the
generated-corpus comparison on a sample under BorrowSanitizer, keeping the full sweep
for the other variants.

For this batch's own gate on the tree it lands, the three tests were skipped by exact
name (`--skip` of the three paths above) so every other test of every binary ran to
completion under BorrowSanitizer; the result is in `REVIEW.md`.
