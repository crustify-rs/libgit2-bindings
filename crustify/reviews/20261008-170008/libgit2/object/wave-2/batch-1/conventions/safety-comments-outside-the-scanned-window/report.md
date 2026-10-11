# Two SAFETY comments of the workset sit where the campaign's scan cannot see them

- **items**: `git_tree_owner` (`TreeRef::owner`), `git_treebuilder_new`
  (`TreeBuilder::new`), `crustify/rust/libgit2/libgit2/object/tree.rs`
- **affected commits**: `8c39ae046` (wave 2 batch 4, `TreeRef::owner`); `8ca54e899`
  (wave 0 batch 4, the `git_treebuilder` type run that emitted `TreeBuilder::new`, which
  this wave schedules as a symbol)
- **class**: conventions — every `unsafe` block carries its `// SAFETY:` comment where the
  campaign's audit finds it

## Finding

`crustify scan-unsafe` counts an `unsafe {` line as documented only when a `SAFETY`
comment is on it or within the six lines above, with only comments, blank lines or
attributes in between. Clippy's `undocumented_unsafe_blocks` accepts both sites, but the
scan counts them as undocumented: two of the 47 `undocumented_unsafe` sites it reports for
the crate at the review base.

- `TreeRef::owner` (tree.rs:1394 at the base): a seven-line comment above
  `unsafe { RepositoryRef::from_ptr(repo) }`.
- `TreeBuilder::new` (tree.rs:1894 at the base): rustfmt split
  `let rc =\n unsafe { ffi::git_treebuilder_new(&mut out, repo.as_ptr().cast_mut(), …) }`,
  which leaves the `let rc =` code line between the comment and `unsafe`.

## Fix

The `owner` comment says the same in five lines. `new` takes the raw repository pointer
into a local beside `out`, so the call fits one line under its comment. Neither changes
behaviour. Re-running the scan's rule over `tree.rs` leaves only `insert_entry`'s site
(`git_treebuilder_insert`, wave 1, outside this workset).
