# Review: libgit2 / object / wave 0 / batch 1 (symbols: `git_blob_data_is_binary`, `git_tag_name_is_valid`)

Reviewed commit `6583ab4ad` (wave base). Both items landed in `d7cf8bff1` (wave 0
batch 6): `blob::data_is_binary(&[u8]) -> bool` in
`crustify/rust/libgit2/libgit2/object/blob.rs` (anchored `Wraps: git_blob_data_is_binary`)
and `tag::name_is_valid(&CStr) -> Result<bool, ErrorBuf>` in
`crustify/rust/libgit2/libgit2/object/tag.rs` (anchored `Wraps: git_tag_name_is_valid`),
each a safe free function over one documented FFI call. The orchestrator's BSan gate on
the translated wave passed (no `bsan-gate.log`).

| step | outcome |
|---|---|
| lifecycle discovery | not applicable: a symbol-route batch with no type or raw-lifetime item, and neither symbol is a lifecycle routine (`git_blob_data_is_binary` attaches a not-owned `git_str` over the caller's bytes and frees nothing; `git_tag_name_is_valid` builds and disposes its own temporary `git_str`). |
| wavefront findings | both records were re-verified against `src/libgit2/blob.c:409-416`, `src/util/str.c:658-670` (`git_str_attach_notowned`: with `len == 0` the pointer is replaced by `git_str__initstr` and never read), `src/util/str.c:1253-1333`, `src/libgit2/tag.c:261-270, 571-592` and `src/libgit2/refs.c:1401-1425`: `str` is a by-value borrowed array of `len` bytes, nullable only when empty, read for the call and never retained; `name` is a borrowed NUL string, nullable (NULL yields `*valid = 0`, return 0), copied into the allocator; `valid` is a non-nullable by-value out-scalar, written 0 first. The notes' claims that no allocation or error state is touched by the blob check, that only OOM returns `< 0` from the tag check, and that an invalid reference name leaves `GIT_ERROR_REFERENCE` while returning 0, all hold (`GIT_ASSERT` is the returning form in this `RelWithDebInfo` build, `src/util/assert_safe.h`). Nothing to resubmit; no report. |
| safe boundary | both wrappers are safe `pub fn`s taking `&[u8]` and `&CStr` (the crate-wide shape for names) and returning `bool` / `Result<bool, ErrorBuf>`; no raw pointer or `unsafe` on the surface. `crustify scan-unsafe` on the tree finds in the two files only the two documented FFI-call blocks (`blob.rs:364`, `tag.rs:49`) plus the flag-set blocks of the type batch; no smell site and no undocumented block. No report. |
| UB | no candidate survived. `data_is_binary`: C reads at most `len` bytes (`git_str_detect_bom` checks `size` before each multi-byte look), never the pointer of an empty slice, touches no shared state, and the one UB in C, the `int` counters of `git_str_is_binary` overflowing past `INT_MAX` bytes of one class, is kept out of reach by `fits_c_counters` (re-confirmed on the base: a C probe with `INT_MAX + 1` printable bytes trips UBSan at `src/util/str.c:1271:13`, "signed integer overflow: 2147483647 + 1", and the `plain` build then answers *binary* for 2 GiB of text, while the wrapper's `usize` fallback answers text, `unit_tests::oversized_slices_are_counted_in_rust` run with `--ignored`). `name_is_valid`: the runtime is pinned before C allocates, `valid` is a live `int`, the `CStr` is NUL-terminated, and a failed pin returns without calling C. Whole workspace clean under plain, ASan/UBSan (with LSan) and TSan (`-Zbuild-std`), except one trybuild expectation outside this workset (see misc); Miri clean on every Rust-only test of the two modules; BSan gate below. No report. |
| equivalence | no defect. `tests/equiv/blob.rs` and `tests/equiv/tag.rs` already compare verdicts, return codes and the thread's last error after every step over byte-order marks and their truncations, every byte alone and among text, the 128:1 ratio, 10 000-byte inputs, a 2 000-input generated stream, a byte-at-a-time growing buffer; and over 41 named tag names, every name of up to three characters over `aH.-/@{*^ ~k`, and a 10-step alternating sequence without clearing the error. Added `tests/equiv/blob.rs::whitespace_classes` (runs of 127 and 128 of each whitespace and control byte the heuristic distinguishes, with one control byte after), which agrees, and which pinned the doc defect below. The one documented divergence, Rust counting slices longer than `INT_MAX` itself, stands (UB step). |
| internal | none: `wide::bom` / `wide::is_binary` mirror `git_str_detect_bom` / `git_str_is_binary` branch for branch (the 4-, 3- and 2-byte marks in C's order, NUL short-circuit, `git__isspace`'s six bytes), and `unit_tests::wide_heuristic_matches_libgit2` compares them with C on the corpus, now extended with the whitespace runs. |
| conventions | clean: both files and their tests are rustfmt-clean, `cargo clippy --workspace --all-targets` is clean, names drop the `git_blob_` / `git_tag_` prefixes the modules supply, both `unsafe` blocks carry specific `// SAFETY:` comments, and the anchors are the two `Wraps:` lines. No report. |
| misc | 2 reports, 1 fixed: `misc/data-is-binary-doc-misclassifies-space-and-form-feed` — the doc of `data_is_binary` said "whitespace counts as neither" printable nor non-printable, but `git_str_is_binary` counts space (above `0x1f`) and form feed (named with backspace and escape) as printable before it consults `git__isspace`; only tab, line feed, vertical tab and carriage return count as neither, so 128 spaces then `0x01` is text where the doc predicted binary. The verdicts were right (the wide fallback already matched C); the doc now states the classes. Regressions in `unit_tests::whitespace_classes` (Rust heuristic under Miri, and end to end), the C-comparison corpus, and `tests/equiv/blob.rs::whitespace_classes`. And `misc/bsan-gate-stalls-on-mailmap-tests`, not fixed here: the wave's BSan gate stalls on three `git_mailmap` tests (below), an item of the sibling type-route review batch. Outside this workset, noted only: under the nightly sanitizer toolchains `tests/ub_safe/compile_fail/tree_builder_not_send_or_clone.rs` (a `git_treebuilder` case of this wave's type-route review batch) mismatches its expected E0277 note chain while passing on stable; the campaign already records this toolchain difference for long E0277 chains. |
| coverage | C (gcov of the `coverage` build, private `GCOV_PREFIX`, the two modules' unit, `ub_safe` and `equiv` tests): `git_blob_data_is_binary` 4/4 lines; `git_str_is_binary` 13/13 lines, every branch of its byte classification taken; `git_str_detect_bom` 24/24 lines, every mark and truncation taken; `tag_name_is_valid` 2/2; `git_tag_name_is_valid` 13/13, with the refusal of the second `git_str_puts` (the reference name growing past the 16-byte block `refs/tags/` takes) now taken deterministically by the added `tag.rs::unit_tests::out_of_memory_growing_the_reference_name_is_an_error`, after the threaded tests' non-atomic counters had left it in doubt. Unreachable through the wrappers: `git_str_attach_notowned`'s dispose of an already allocated buffer (`str.c:661`; the wrapper's `git_str` is `GIT_STR_INIT`), the `!name` branch and the `GIT_ASSERT(valid)` refusal of `git_tag_name_is_valid` (a `&CStr` and a live `int`). Rust (`cargo llvm-cov`, `coverage` variant): `tag.rs` has no uncovered production line; `blob.rs`'s only one was `data_is_binary`'s `return wide::is_binary(data)` for slices longer than `INT_MAX`, reached only by the `#[ignore]`d 2 GiB scan — now covered by `unit_tests::oversized_slice_takes_the_rust_heuristic`, which hands over a lazily zeroed `INT_MAX + 1`-byte `vec!` (binary at its first byte, and under each 2- or 3-byte mark written over it) so the test costs one `calloc`, not a scan. The remaining uncovered line of `tag.rs` is the `stderr` formatting of a child-process failure message in a test helper. |

## BSan gate

Run as the Regressions step prescribes: one `--no-run` build, then one
`cargo-variant.sh bsan test` per test binary in parallel, each under `timeout 2400`,
started in the background and read only after it exited.

- **Wave tip `6583ab4ad`, before any fix**: `libgit2-sys` and `ub_unsafe` (99 passed,
  133 s) clean; `lib`, `ub_safe` and `equiv` each killed at 2400 s with exactly one test
  still running — `mailmap::unit_tests::classifier_agrees_with_libgit2`,
  `mailmap::adversarial_buffers_parse_without_ub` and `mailmap::buffers_match` — after
  440 / 433 / 287 passes and no BorrowSanitizer report, panic or failure in any log
  (`misc/bsan-gate-stalls-on-mailmap-tests`, with the log excerpt). No report survived
  triage: the three are CPU-bound many-iteration parses of this wave's `git_mailmap`
  tests, a stall of the kind the campaign has met before, owned by the type-route review
  batch that was gating the same tree concurrently; left to it to stride.
- **The tree landed here, after its last rebase**: the same gate with those three tests
  skipped by exact name, so every other test ran to completion: `lib` 443 passed, 2
  ignored (40 s; the four tests added here among them, the zeroed 2 GiB one included),
  `ub_safe` 433 passed, 2 ignored (332 s), `ub_unsafe` 99 passed (87 s), `equiv` 288
  passed, 1 ignored (1285 s), `libgit2-sys` clean; no BorrowSanitizer report. The new
  tests were also run alone under BSan by name beforehand (14 passed in 1.9 s, and
  `tests/equiv/blob.rs::whitespace_classes` in 0.15 s).


Validation of the final tree: `cargo check` / `clippy --workspace --all-targets` /
`test --workspace` (plain), ASan/UBSan and TSan on every workspace test binary (the
`ub_safe` binary with the one toolchain-dependent trybuild case noted above), Miri on
the Rust-only tests of both modules, BSan as above, `crustify scan-unsafe` on the tree.
No C change, so no C baseline was rerun.
