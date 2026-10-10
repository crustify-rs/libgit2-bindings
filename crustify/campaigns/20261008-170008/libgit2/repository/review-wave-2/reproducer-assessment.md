# libgit2/repository review wave 2: reproducer assessment

Assessed by the orchestrator on 2026-10-10.

One runnable reproducer was added (a behavioural one, no sanitizer); it was rerun with
the `plain` variant on the pre-fix tree (`4e8b5c1a9`, the review's reports commit) and
on the reviewed tip. The other findings were checked against their evidence:

| finding | check | verdict |
|---|---|---|
| `batch-0/misc/worktree-add-asks-the-lent-references-repository` | rerun: pre-fix exits 1 ("a second worktree was added on `feature`", both `HEAD`s `ref: refs/heads/feature`), as in `run-affected.log`; reviewed tip exits 0 ("reference refs/heads/feature is already checked out"). Real: `git_branch_is_checked_out` walks `git_reference_owner(ref)`'s worktrees, not `repo`'s, so a lent reference of another repository lets one branch be checked out twice. Not UB; the fix is a deliberate, documented divergence from C (`equiv/worktree.rs::add_options::adds_with_another_repositorys_reference_match_c` records it), consistent with the wave 1 worktree wrappers that correct C defects | accepted |
| `batch-0/wavefront/worktree-add-checked-out-check-asks-the-references-owner` | same evidence; record resubmitted | accepted |
| `batch-0/misc/fetchhead-walk-refused-keeps-a-stale-stop` | the reset of `stopped` followed the wrapper's refusals; regression `unit_tests::init_ext_and_fetchhead::for_each_fetchhead_refuses_a_repository_without_paths` passes in the reviewed-wave gates | accepted |
| `batch-0/misc/fetchhead-entry-without-a-marker-undocumented` | `fetchhead_ref_parse` (`fetchhead.c:170`) leaves `is_merge` unset for an id + tab line; doc-only fix, equivalence test `fetchhead_entry_without_a_marker_keeps_the_previous_one` compares against C | accepted |
| `batch-0/conventions/safety-comment-split-in-add-options-name-of` | unseeded scans: undocumented unsafe 29 (wave 2) -> 28 (review wave 2); `tests/equiv/worktree.rs` 1 -> 0 | accepted |

Rejected: none.
