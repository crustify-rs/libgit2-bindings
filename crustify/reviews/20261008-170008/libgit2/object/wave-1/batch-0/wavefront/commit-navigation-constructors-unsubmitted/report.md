# Wavefront: three constructors of the wave's types carried no finding

**Entities:** `git_commit_parent`, `git_commit_nth_gen_ancestor`, `git_commit_tree`
(symbols); `git_commit`, `git_tree` (the types they construct).

**Evidence.** Before this review, `query symbols --name <each>` showed
`"lifetime": null`, both `ptr` blocks `null` and `_analysis.submitted: false`;
`query symbols --lifetime-for git_commit` listed only `git_commit_lookup` and
`git_commit_lookup_prefix` under `constructed_by`, and `--lifetime-for git_tree` only
`git_tree_lookup` and `git_tree_lookup_prefix`. Yet the `_comment_agent` batch 1
submitted for `git_commit` reads "constructed by git_commit_lookup/_lookup_prefix …,
git_commit_parent, git_commit_nth_gen_ancestor": the analysis saw them, but no symbol
record carried the role, so the wrappers landed without them (see the two `lifecycle/`
reports for the C evidence).

**Resubmitted** with `--update` (`crustify/wavefront/ownership-store.json`), one record
each:

- `git_commit_parent`: `lifetime = {for: parent, is_constructor: {alloc: true}}`;
  `parent` a `by_ref` owned slot (one counted reference to the `n`th parent, looked up
  through `commit->object.repo`; untouched on failure, `GIT_ENOTFOUND` past the last
  parent); `commit` borrowed for the call, read only, its uncounted repository reached.
- `git_commit_nth_gen_ancestor`: `lifetime = {for: ancestor, is_constructor: {alloc:
  true}}`; `ancestor` a `by_ref` owned slot (the very commit up-ref'd for `n == 0`,
  otherwise the first-parent ancestor, intermediates freed); `commit` borrowed for the
  call (one transient up-ref through a `const` cast, settled before returning).
- `git_commit_tree`: `lifetime = {for: tree_out, is_constructor: {alloc: true}}`;
  `tree_out` a `by_ref` owned slot (one counted reference to the commit's tree, looked
  up through `commit->object.repo`); `commit` borrowed for the call, read only.

`--lifetime-for git_commit` now lists `git_commit_nth_gen_ancestor` and
`git_commit_parent` under `constructed_by`, and `--lifetime-for git_tree` lists
`git_commit_tree`. (`query types --name git_tree --lifecycle-ops` still omits
`git_commit_tree`, and lists it under `git_commit` instead, although the record's `for`
names `tree_out`, a `git_tree **`; `--lifetime-for`, which follows `for`, is right.)
