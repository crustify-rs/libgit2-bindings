## Landing-only run (departure)

Your batch was reviewed by reviewer run 130, whose two commits are kept at
`refs/crustify/attempts/wave-333-batch-0-run-130`:

- `024af4038`, its reviews dir (two reports under `crustify/reviews/wave-333/batch-0/`);
- `0d973d4fa`, its fixes to the Rust tree, the ownership store and the dashboard.

They could not be promoted: run 130 ran beside review wave 331 from the same base, on
its own integration branch, so it never rebased over 331's landing, and `0d973d4fa`
now conflicts with it in `tests/equiv/src/libgit2/checkout.rs` (both appended a
test) and `crustify/artifacts/dashboard.md` (both added rows).

Do not review again. Replay the two commits onto your branch with `git cherry-pick`,
in order. Resolve the conflicts by keeping both sides: both tests, and both sets of
dashboard rows in their tables' order, the `Σ` rows recounted. Then run the gates
your prompt gives a landing whose rebase conflicted, on the resolved tree, fix what
fails, and land the two commits as a review batch lands them. Keep the commits'
messages, adding a line to the second that names the conflict you resolved.
