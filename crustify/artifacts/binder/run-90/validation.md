# index wave 1 / batch 4 — pathspec symbols (wrap)

## Surface and ownership

- Verified the homogeneous **symbol** route. Four of the five items already carried
  filled anchors from the wave-0 type runs and are completed work, not re-emitted:
  `git_pathspec_new` (`Pathspec::new`, the type's constructor), `git_pathspec_match_list_free`
  (`MatchListFree`, the list's release policy), `git_pathspec_match_list_failed_entrycount`
  (`MatchListRef::failed_len`) and `git_pathspec_match_workdir` (`MatchList::from_workdir`).
  Their wavefront findings were already submitted; existing ub_safe/equiv suites cover them.
- New: `PathspecRef::matches_path(&self, PathspecFlags, &CStr) -> bool`
  (`Wraps: git_pathspec_matches_path`). The pathspec is only read (const, no refcount
  change); the path is read during the call and never retained, so a shared handle and a
  borrowed `&CStr` express the whole contract. It is reachable from `PathspecBox`,
  `PathspecMut` and `PathspecArc` (also while a match list retains a reference). C's only
  failure path is `GIT_ASSERT_ARG` on NULL, unreachable here, so the result is `bool`.
- Doc records observed behaviour: only `NO_GLOB` / `IGNORE_CASE` are consulted (no
  platform case fallback); patterns are tried in order and the first deciding one wins,
  so `!x` excludes only ahead of an including pattern; literal patterns match as directory
  prefixes. `"/"` and `"!/"` compile to nothing (leading slash stripped, length 0 →
  `GIT_ENOTFOUND`) and so match every path, like an empty pathspec.
- Checked `pathspec_match_one`'s `path[len]` / `path[len+1]` probes: both follow a
  `strncmp`/`git__strncasecmp` equality over `len` bytes of a NUL-free pattern, so they
  stay within the path. Zero-length patterns never reach the comparators. No C defect found.
- Wavefront findings for `git_pathspec_matches_path` submitted with `--update` (additive
  store diff, no re-sort).

## Build reuse and validation

No C, shim, header or bindgen change. Worktree `src/`, `include/`, `deps/`, `cmake/`,
`CMakeLists.txt` and `crustify/build.json` match `/target`; reused the immutable shared
builds (build.json version 1) through `crustify/scripts/cargo-variant.sh`. Private logs
under `/tmp/crustify-index-w1-b4`.

- `asan-ubsan check --workspace --all-targets`, `clippy --workspace --all-targets`: clean
  (only pre-existing manifest warnings).
- `asan-ubsan test --workspace`: passed (lib 675, equiv 431, ub_safe 708, ub_unsafe 95,
  doctests), no sanitizer report.
- `tsan test -p libgit2 --test equiv --test ub_safe -- pathspec`: passed (7 + 12).
- BSan, exact name only, under a 1800 s timeout:
  `ub_safe pathspec::matches_path_across_owners_reborrows_and_retaining_list`: passed
  (flag sweep strided under `cfg!(bsan)`).
- Miri: not run; every new test calls libgit2 (`cfg_attr(miri, ignore)`), and the batch adds
  no Rust-only logic.
- `crustify scan-unsafe --json --sites all`: no smell counter changed versus the
  review-wave-0 baseline; undocumented unsafe stays 45 (6 new sites, all documented).

## Tests

- equiv: `construction_copy_and_release_sequences` now compares the safe `matches_path`
  (shared and reborrowed handles, two passes, 64 flag combinations plus an unknown high bit,
  25 paths, 15 pattern sets) against `git_pathspec_matches_path` on an independent raw
  pathspec, asserts the result ignores unconsulted bits, and re-checks the Rust owner after
  the raw one is freed. `matching_across_retaining_list_sequence` compares after each step of
  create → list retains → list released → inputs released. `slash_only_patterns_compile_to_nothing`
  pins the slash-pattern behaviour on both sides.
- ub_safe: every owner/handle form and reborrow depth, a retaining match list, exactly sized
  heap paths (redzone after each terminator, including negative-pattern `!` probes), freed
  paths between calls, and concurrent thread-local pathspecs.
- No public unsafe API added: no ub_unsafe suite. No compile-fail case added: the handle's
  borrow already rejects use after the owner (existing doctests).

## Coverage

Coverage build with `GCOV_PREFIX` (private .gcda). Rust `matches_path`: every region hit.
C `git_pathspec_matches_path`, `pathspec_match_context_init`, `pathspec_match_one` and
`git_pathspec__match_at`: every line hit, including the directory-prefix and negative `!`
branches. Uncovered and unreachable from this entry point: `git_pathspec__match`'s
`matched_pathspec` / `matched_at` writes (lines 211, 221–226; this caller passes NULL) and
the NULL-argument `GIT_ASSERT_ARG` failures (the safe API cannot pass NULL).
