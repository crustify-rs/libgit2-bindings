# `generate_with_options` states obligations that libgit2 checks or does not need

- **item**: `git_diff` (type), `Diff::generate_with_options`,
  `crustify/rust/libgit2/libgit2/diff/diff.rs`
- **affected commit**: `5be7c5199` (libgit2/index wave 0 batch 0)
- **class**: unsafe (safety contract)

## Finding

The `# Safety` section of the one public `unsafe fn` in the workset imposed two
conditions that are not safety obligations:

1. "`options.version` must equal `GIT_DIFF_OPTIONS_VERSION`." Every generator calls
   `diff_prepare_iterator_opts` first, and its `GIT_ERROR_CHECK_VERSION` refuses another
   version before any other field is read. A wrong version is an error, not UB. (Two
   generators misreport that error as `1`; see
   `misc/positive-code-for-refused-diff-options-version`.)
2. "Input trees and indices must be compatible with the repository's object-id type."
   The six **safe** constructors accept the same `TreeRef` / `IndexRef` inputs with no
   such check. If the condition were needed, they would be unsound. It is not needed.
   libgit2 copies every id as a whole `git_oid` (type byte included) and compares ids by
   type first (`git_oid_equal`, `git_oid_cpy` in `diff_generate.c`). A nonzero
   `options.oid_type` other than the repository's is refused at
   `diff_generate.c:502-506`.

Evidence for (2): every generator was driven with SHA-256 trees
(`tests/resources/testrepo_256.git`) and a SHA-256 index (`git-sha256.index`, 4344
entries) against a SHA-1 repository, and with SHA-1 inputs against a SHA-256
repository with a working directory. Under ASan/UBSan (LSan on) there was no report.
The deltas carry each input id's own type; only `id_abbrev` follows the repository's
hex size.

A contract that forbids what the safe API permits leaves callers unsure which inputs
are actually dangerous. The real obligations (nested pointers, referent lifetime,
callback discipline) are kept. The pathspec obligation turned out to admit UB (an
empty pattern beside a non-empty one). The wrapper now checks for that, so it is listed
with the checked conditions; see `ub_unsafe/diff-options-empty-pattern-overread`.

## Fix

The `# Safety` section lists only the real obligations. A second paragraph says what is
checked: the version, a mismatched `oid_type` and a mixed empty pathspec fail with `-1`
/ `GIT_ERROR_INVALID`, and foreign-type inputs are accepted with their own id types.

Regressions:

- `ub_safe::diff::foreign_object_id_type_inputs_stay_in_bounds`: ten mixed-type
  combinations over all six safe generators, asserting delta counts, statuses, id types
  and abbreviations, under the sanitizers.
- `ub_unsafe::diff::generate_with_options_checked_fields`: within the revised contract,
  the checked fields fail for every generator, and a SHA-256 tree diffed through the
  unsafe API in a SHA-1 repository succeeds with SHA-256 ids.
