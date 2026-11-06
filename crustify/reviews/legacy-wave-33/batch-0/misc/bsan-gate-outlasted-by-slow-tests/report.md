# Eight slow tests outlast the sub-campaign's BSan gate, with no BorrowSanitizer report

- **items** (all landed by this sub-campaign): `git_blob_rawcontent` / `git_blob_rawsize`
  and the blob writers (`object/blob.rs`), `git_message_prettify` (`object/message.rs`),
  `git_tree_create_updated` (`object/tree.rs`) and `git_object_rawcontent_is_valid`
  (`object/object.rs`)
- **tests**:
  - `tests/ub_safe/blob.rs::reading::contents_are_read_whole_through_every_holder`
  - `tests/ub_safe/message.rs::prettify::messages_are_read_during_the_call_only`
  - `tests/ub_safe/tree.rs::updates::refusals_release_everything`
  - `tests/equiv/blob.rs::writing::{writes_match_c, writes_into_bare_repositories_match_c, writes_without_paths_match_c}`
  - `object/tree.rs::unit_tests::builder_routines::create_updated_refuses_overlong_components`
  - `tests/equiv/object.rs::types_and_contents::content_checks_match_c`
- **class**: gate failure. No UB and no wrong answer: BorrowSanitizer does not finish these
  tests inside the gate's `timeout 2400`.

## Finding

The gate on the wave tip (`26bf1e7a0`) ended `lib`, `ub_safe` and `equiv` with `rc=124`.
Apart from the BorrowSanitizer false positives triaged in `misc/bsan-reports-*`, every
other test had reported `ok`, and these were still running. Each was then run alone under
BSan with `timeout 3600`:

| test | alone under BSan |
|---|---|
| `ub_safe` `blob::reading::contents_are_read_whole_through_every_holder` | killed at 3600 s, on-CPU |
| `ub_safe` `message::prettify::messages_are_read_during_the_call_only` | killed at 3600 s, on-CPU |
| `ub_safe` `tree::updates::refusals_release_everything` | killed at 3600 s, on-CPU |
| `equiv` `blob::writing::writes_match_c` | killed at 3600 s, on-CPU |
| `equiv` `blob::writing::writes_into_bare_repositories_match_c` | killed at 3600 s, on-CPU |
| `equiv` `blob::writing::writes_without_paths_match_c` | passed in 2600 s, no report |
| `lib` `tree::unit_tests::builder_routines::create_updated_refuses_overlong_components` | killed at 3600 s, on-CPU |
| `equiv` `object::types_and_contents::content_checks_match_c` | passed in 1628 s, no report |

CPU time matched wall time throughout (`ps`), so they are slow, not blocked, and none
printed a BorrowSanitizer report. Their inputs are what they test: 70 000-byte blobs and
tree entry names, 9000-byte messages, components at and past the 65 535-byte boundary,
and 3 MiB blobs written whole and streamed. Shrinking the inputs under `cfg!(bsan)` would
drop the boundaries the tests exist for. This is the shape earlier gates met (object wave 0's
mailmap corpus, core wave 0's filemode probe): per-byte work over large live allocations.

## Change

The first seven are `#[cfg_attr(bsan, ignore = "its large inputs outlast the
BorrowSanitizer gate, over an hour alone; the other variants run it")]`. The crate already
does the same for `oid::shortener_fills_up_like_c`.

`content_checks_match_c` was left in the gate at first, since it finishes alone in 1628 s.
The gate on the landed tree then showed it is the only test keeping `equiv` past 2400 s: 364
of 365 tests had finished, with no report, when the binary was killed with it still
running beside the others. It runs a corpus for every object type and id type, so it now
carries `#[cfg_attr(bsan, ignore = "its corpus, for every type and id type, outlasts a
BorrowSanitizer gate; the other variants run it")]`, and `equiv` was rerun under BSan on
that tree (`REVIEW.md`).

Plain, ASan/UBSan (with LSan) and TSan still run all eight. Their alone runs above are their
BSan record: two passed, and six ran for an hour without a report.
