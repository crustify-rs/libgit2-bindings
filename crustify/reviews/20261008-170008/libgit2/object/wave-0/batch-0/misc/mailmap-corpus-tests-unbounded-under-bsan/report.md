# Misc: the mailmap corpus tests do not finish under the BSan gate

**Item:** `git_mailmap` (wave 0 batch 2): `mailmap.rs::unit_tests::classifier_agrees_with_libgit2`,
`tests/ub_safe/mailmap.rs::adversarial_buffers_parse_without_ub`,
`tests/equiv/mailmap.rs::buffers_match`.

**Finding.** The three tests were the only ones still running in the wave tip's BSan
gate after 60 minutes, with the rest of their binaries (440 + 433 + 287 tests) long
finished; `lib` and `ub_safe` hit the gate's 90-minute timeout on them. Batch 2's own
BSan log (`/tmp/b2_bsan1.log`) shows the `ub_safe` one killed with SIGTERM rather than
waited for. Timing probes under `cargo bsan` explain it:

- the classifier test's `corpus()` materialises 11⁴ × 3 ≈ 44 k small `Vec<u8>`s before
  parsing anything; BorrowSanitizer tracks every allocation, and building that corpus
  alone had not finished after 20 minutes (on plain it takes milliseconds);
- a `git_mailmap_from_buffer` of the ~600-byte reference `.mailmap` (about ten
  entries) takes 2.3 s under BSan against 15 ms for a one-entry buffer, so the
  `ub_safe` test's 600 prefixes, 64-fold repeat and two 64 KiB names, and the
  `equiv` test's 90 prefixes and 240 generated buffers, each add up to tens of minutes.

The other batches of this and earlier waves bound their corpus tests under BSan with a
`cfg!(bsan)` stride (`equiv/oid.rs`, `equiv/index.rs`, `equiv/repository.rs`,
`ub_safe/tree.rs`); these three did not.

**Fix.** The same discipline, applied where the cost is: `corpus()` builds only every
37th token combination under BSan (sampling at generation, not after), and the
classifier asks for proportionally fewer refusals; the `ub_safe` test parses every 37th
prefix, a 4-fold repeat and 1 KiB names under BSan; the `equiv` test every 77th prefix
and every 11th generated buffer. Plain, ASan/UBSan, TSan and Miri runs are unchanged
(`stride = 1`, full inputs). With that the gate on the landed tree completes (see
`REVIEW.md`).
