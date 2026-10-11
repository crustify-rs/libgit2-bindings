# `SignatureCallback` docs omit the `CreateOptions` route to libgit2

- **item**: `git_commit_create_options` (the change that left them incomplete:
  `CreateOptionsMut::set_signer`); the docs are `git_commit_signature_cb`'s, in
  `crustify/rust/libgit2/libgit2/object/commit.rs`
- **affected commit**: `8d42785bd` (libgit2/object wave 2 batch 0), reviewed at `d6a6d1f95`
- **class**: documentation defect

## Finding

Batch 0 gave `CreateOptionsMut::set_signer` a `SignatureCallback` to lend to libgit2
through `git_commit_create_options`, and its docs send a caller to
`SignatureCallback::resume_panic` for a panic the closure raised in such a call — the
only way to resume it, since no wrapper of the routines reading these options exists
yet. The `SignatureCallback` docs were left as wave 1 wrote them for
`git_commit_create_ext_options` alone:

- `resume_panic`: "Resumes a panic the closure raised inside a libgit2 call it was lent
  to through [`CreateExtOptionsMut::set_signer`], if any";
- the type docs: "the routines writing a commit with options (`git_commit_create_ext`,
  `git_commit_create_from_stage`) call it once", leaving out
  `git_commit_create_from_tree`, `git_commit_amend_from_stage` and
  `git_commit_amend_from_tree`, which all sign through the same options
  (`init_ext_options` copies `sign` and `payload` into the extended options each of them
  builds).

## Fix

`resume_panic` names both `CreateExtOptionsMut::set_signer` and
`CreateOptionsMut::set_signer`; the type docs name `git_commit_create_ext` and the
`git_commit_create_from_*` and `git_commit_amend_from_*` routines. No behaviour changes.

## Evidence

`tests/equiv/commit.rs::create_options::commits_created_and_amended_with_the_options_match_c`
signs through all four `git_commit_create_options` routines on both sides (the callback's
call count is compared after every step: 9 calls over the script).
