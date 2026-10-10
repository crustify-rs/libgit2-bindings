# Wavefront: three constructors of the wave's types carried no finding

**Entities:** `git_tree_entry_bypath`, `git_object_peel`, `git_object_lookup_bypath`
(symbols); `git_tree_entry`, `git_object` (the types they construct).

**Evidence.** Before this review, `query symbols --name <each>` showed
`"lifetime": null`, every `ptr` block `null` and `_analysis.submitted: false`; and
`query symbols --lifetime-for git_tree_entry` listed no constructor at all
(`"constructed_by": []`), `--lifetime-for git_object` only `git_object_lookup` and
`git_object_lookup_prefix`. The three routines are `alloc` constructors by
out-parameter (see the two `lifecycle/` reports for the C evidence), so the type runs'
lifecycle discovery, which reads `--lifecycle-ops`, could not see them, and the
`TreeEntry` / `Object` wrappers landed without them.

**Resubmitted** with `--update` (`crustify/wavefront/ownership-store.json`), one record
each:

- `git_tree_entry_bypath`: `lifetime = {for: entry_out, is_constructor: {alloc: true}}`;
  `entry_out` a `by_ref` owned slot (a fresh `git_tree_entry_dup` block the caller frees
  with `git_tree_entry_free`, untouched on failure); `root` borrowed for the call, read
  only, with the note that a path of several components looks the subtrees up through
  `root->object.repo` and that the routine reads any object pointer as a tree; `path` a
  borrowed string read for the call.
- `git_object_peel`: `lifetime = {for: peeled, is_constructor: {alloc: true}}`; `peeled`
  a `by_ref` owned slot (the same object up-ref'd when already of the target type,
  otherwise the tag target / commit tree looked up through the object's repository);
  `object` borrowed for the call, read only, its uncounted repository reached.
- `git_object_lookup_bypath`: `lifetime = {for: out, is_constructor: {alloc: true}}`;
  `out` a `by_ref` owned slot (one counted reference to the entry's object, looked up in
  `git_object_owner(treeish)`); `treeish` borrowed for the call, peeled to a tree and its
  repository used for every lookup; `path` a borrowed string.

`--lifetime-for git_tree_entry` now lists `git_tree_entry_bypath` under
`constructed_by`, and `--lifetime-for git_object` lists all four constructors.
