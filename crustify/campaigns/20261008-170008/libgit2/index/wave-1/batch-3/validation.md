# libgit2/index wave 1 batch 3 — wrap validation

## Contract and scope

Symbol route, objective `wrap`. 18 `git_index_*` functions are new safe methods. The
7 pathspec items already carried filled `Wraps:` anchors from wave 0's type batches
(`a8fb7fc05`, `ab190ae80`), which emitted them with `Pathspec` / `MatchList`:
`git_pathspec_free` is `PathspecFree` (a lifecycle routine, skipped by the symbol
route), and `match_index` / `match_diff` / `match_list_{entry,entrycount,
failed_entry,diff_entry}` are `MatchList::from_index` / `from_diff` and
`MatchListRef::{entry,len,failed_entry,diff_entry}`. They were re-read against C and
left unchanged; their equiv/ub_safe suites ran again under ASan/UBSan and TSan.

Which handle each method takes depends on whether libgit2 frees storage an `IndexRef`
could lend:

- `IndexRef` (`index.rs`): `path` (the index's own copy, freed only with the index,
  so it borrows for the handle's lifetime), `version`, `write`, `write_tree`,
  `write_tree_to`; `reuc_entry_count` (`sys/index.rs`). The three writers sort
  vectors, replace the private tree cache and reset racily-clean entry sizes, but
  free no entry or record. A repository's index is shared (`IndexArc`), so it has
  only shared handles; the writers are what such an index needs.
- `IndexMut`: `read`, `set_version`, `set_caps`, `remove`, `remove_bypath`,
  `remove_directory`, `remove_all`, `update_all`; `add_reuc`, `find_reuc` (lazy
  sort, as `name_entry`), `remove_reuc`, `clear_reuc`.

Errors are `RepositoryError`, which keeps the code (`NotFound`, `Unmerged`,
`Locked`, callback codes); the REUC methods use `ErrorBuf`, as `sys/index.rs`'s NAME
methods do. Scans take `StrArrayRef` plus `Option<&mut MatchedPathCallback>` and
drive the callback through the existing crate-private `scan_with` seam. No public
`unsafe` API and no public raw pointer were added. `add_reuc` takes
`Option<ReucSide>` for each side, the struct batch 1 added for the sides a
`ReucEntryRef` lends, so those sides round-trip. Any non-zero `u32` mode is stored as
given, and `ReucSide::new(FileMode, OidRef)` builds a side from a mode libgit2 names.

## Refusals and C defects

- `write_tree_to` / `write_tree`: an entry id that libgit2 would look up
  (`check_entry` → `git_object__is_valid`) in a repository of another oid type
  overflows the loose backend's path buffer. Confirmed under ASan with a temporary
  raw probe that was not kept: `heap-buffer-overflow` in `git_oid_fmt_substr` ←
  `object_file_name` ← … ← `check_entry` ← `write_tree` ← `git_tree__write_index`.
  The wrappers refuse such ids with `id_fits`, and also path components over 65535
  bytes, which libgit2 would store truncated in a tree it reports as written. A
  conflicted index is left to C's `GIT_EUNMERGED`.
  `git_index_add` on a **repository-owned** index looks ids up the same way. A raw
  test fixture overflowed in `index_insert` → `git_object__is_valid`, so the
  `git_index_add` wrapper (a later batch) needs the same `id_fits` check.
- `remove_all` / `update_all`: mixed empty/non-empty pathspecs are refused
  (`patterns_are_parsable`). The callback cannot reach the scanned index, because
  the `IndexMut` stays exclusively borrowed for the scan (compile-fail doctest).
- `remove_directory(Stage::Any)` is refused: C compares the stage with -1
  literally, so it would remove nothing and report success.
- `remove_reuc` out of range: C returns `GIT_ENOTFOUND` without setting an error and
  leaves the previous one in place. The wrapper clears the error first and then
  describes the failure.
- `read` can return `1` on a filestamp race (the `updated = check < 0` precedence
  bug). The wrapper treats any non-zero return as a failure (`Generic`).

## Blocker (reported, not worked around)

`update_all`, `write_tree` and `set_caps(FromOwner)` need an index that a repository
owns. Every owned index is shared (`IndexArc`, from `git_repository_index` or after
`set_index`), so it never reaches `IndexMut`. `write_tree` takes `&self` and works on
a repository's index. `update_all` and `set_caps(FromOwner)` free or re-sort entries
and stay on `IndexMut`, where libgit2 always answers "no repository". Making them
reachable needs a sound way to mutate a shared index exclusively. That design spans
`repository.rs` and the existing `IndexArc` contract, so it is beyond this batch.

## Build reuse

Used the shared `crustify/builds/{plain,asan-ubsan,tsan,bsan,coverage}` prebuilds
through `cargo-variant.sh`. No C or shim was changed, only the bindgen function
allowlist (11 entries). C coverage was redirected with `GCOV_PREFIX` to a private
directory. No C baseline rebuild is needed.

## Tests and gates

- plain: `cargo check --workspace`, `cargo clippy --workspace --all-targets` (clean),
  `cargo test --workspace` (all pass, including doctests).
- ASan/UBSan (with LSan): every `index`, `sys_index` and `pathspec` test of the lib,
  `equiv` and `ub_safe` binaries.
- TSan: the same `index` / `pathspec` filters (no `Send`/`Sync` added).
- Miri: the Rust-only unit tests (`status_reports_positive_codes_as_generic`,
  `absent_and_unreadable_reuc_sides`, `layout_and_nullable_projection`); the others
  call libgit2 and are ignored there.
- BSan, by exact name only: `index::edits::path_borrows_outlive_writes_and_reads`
  and `index::edits::scan_callbacks_read_lent_paths_while_entries_go` (ub_safe):
  clean.
- `crustify scan-unsafe`: the smell counters equal the review-wave-0 baseline, and
  the new code has no smell site. After rebasing onto batches 0, 1 and 4, the
  undocumented count is 46: the extra site is batch 0's `open_with_oid_type`
  (`index.rs`, a split `let rc =`), not this changeset's.

Equivalence (`tests/equiv/index/edits.rs`, `tests/equiv/sys_index/reuc_edits.rs`) runs C and
Rust on independent indexes, files and repositories. After every step it compares
status, error, entries (path, flags, mode, id, file size), REUC count, version, caps
and the written index bytes. Covered: version and caps sequences (re-sorting);
removal by stage, path and directory over conflicts; `remove_all` scans with
apply/skip/abort codes, with no callback, and with a panic (against C's `GIT_EUSER`);
`update_all` without an owner; soft and forced reads of missing, unchanged, changed,
vanished, corrupt and short files; locked writes; `write_tree_to` with tree cache,
case-insensitive index, missing objects (top level and subtree), submodules and
conflicts; `write_tree` through each repository's own index; racily-clean writes in
a working directory; REUC add/replace/find (also case-insensitive)/remove/clear and
reread, including unnamed modes. UB tests (`tests/ub_safe/index/edits.rs`,
`tests/ub_safe/sys_index/reuc_edits.rs`)
cover borrowed paths across reborrows, writes and reads, and past a dropped
repository; every free route; nested scans, aborts and panics; the foreign-id
refusals; and temporary inputs and records freed once. These `ub_safe` fixture
helpers now call the safe API instead of raw C: `path`, `set_version` and `write`,
and batch 1's REUC stand-ins `add_reuc`, `remove_reuc`, `clear_reuc` and
`set_ignore_case`. All validation above was rerun on the rebased tree.

## Coverage

Rust (llvm-cov, focused tests): `index/index.rs` 99.3%, `index/sys/index.rs` 100%
of lines. The only uncovered lines in the new code are libgit2 OOM returns
(`add_reuc`, `clear_reuc`, `remove_reuc` with a C-set error). C (gcov): every line
of `git_index_{path,version,set_version,set_caps,write,write_tree,write_tree_to,
remove,remove_bypath,remove_directory,remove_all,update_all,reuc_*}` is covered.
Still uncovered:

- `git_index_read`'s forced-clear failure and filestamp race (`index.c:707`, `:749`);
- `compare_checksum`'s seek and short-read failures;
- `truncate_racily_clean`'s diff failure;
- `index_apply_to_all`'s OOM, NONE/UPDATE/default actions (unused by its only
  public caller);
- `write_tree`'s treebuilder and lookup I/O failures.

`update_all`'s scan beyond the owner check cannot be reached through the safe API
(see Blocker).
