# `generate_with_options` lets libgit2 read past an empty pathspec pattern within its contract

- **item**: `git_diff` (type), `Diff::generate_with_options` (the workset's only public
  `unsafe fn`), `crustify/rust/libgit2/libgit2/diff/diff.rs`
- **affected commit**: `5be7c5199` (libgit2/index wave 0 batch 0); reproduced on the
  review wave tip `b3716baeb`
- **class**: UB through the unsafe public API, called within its stated `# Safety`
  contract (AddressSanitizer)

## Finding

Every generator copies its options into the diff and compiles `options.pathspec`
(`diff_generated_apply_options` → `git_pathspec__vinit`, `src/libgit2/diff_generate.c:498`).
When at least one pattern is non-empty, `git_pathspec__vinit` parses every pattern, and
`parse_optimized_patterns` (`src/libgit2/attr_file.c:842`) tests `pattern[1]` before
`pattern[0]`. For an empty pattern, that is the byte after its terminator: an
out-of-bounds read. The libgit2/index review wave 0 batch 1 found the same parse behind
`Pathspec::new` and fixed that safe constructor
(`batch-1/misc/pathspec-empty-entry-overread`). Its report lists the `git_diff_*`
options among the other paths to the parse, but considered none of them wrapped.
`Diff::generate_with_options` is wrapped, as an `unsafe fn`, and its contract asked only
that "pathspec is a readable array of its stated count, every string is NUL-terminated".
`["a", ""]` satisfies that and still reaches the read.

## Reproducer

`src/main.rs` in this directory is a standalone Cargo package. It builds default options
with the supported version, a two-entry pathspec of heap strings (`"a"`, and `""` as a
one-byte allocation, so the next byte is a redzone), no callbacks and no input trees.
That honours every obligation of the contract as stated on the affected commit. The doc
comment gives the command (the `asan-ubsan` variant, `cargo +nightly run`). On
`b3716baeb` it aborts with exit status 134:

```
==1806307==ERROR: AddressSanitizer: heap-buffer-overflow on address 0x75c829c4fd71 at pc 0x79a82b219b0a bp 0x7ffc9372e540 sp 0x7ffc9372e538
READ of size 1 at 0x75c829c4fd71 thread T0
    #0 parse_optimized_patterns /target/src/libgit2/attr_file.c:842:7
    #1 git_attr_fnmatch__parse /target/src/libgit2/attr_file.c:730:6
    #2 git_pathspec__vinit /target/src/libgit2/pathspec.c:89:9
    #3 diff_generated_apply_options /target/src/libgit2/diff_generate.c:498:7
    #4 git_diff__from_iterators /target/src/libgit2/diff_generate.c:1287:15
    #5 git_diff_tree_to_tree /target/src/libgit2/diff_generate.c:1397:15
    #6 <libgit2::libgit2::diff::diff::Diff>::generate_with_options crustify/rust/libgit2/libgit2/diff/diff.rs:555:51
    #7 diff_options_empty_pattern_overread::main .../ub_unsafe/diff-options-empty-pattern-overread/src/main.rs:58:13
    ...
0x75c829c4fd71 is located 0 bytes after 1-byte region [0x75c829c4fd70,0x75c829c4fd71)
allocated by thread T0 here:
    #0 malloc
    ...
SUMMARY: AddressSanitizer: heap-buffer-overflow (/target/crustify/builds/asan-ubsan/libgit2.so.1.9+0x1b1b09)
```

On the fixed tree it prints
`refused: diff error -1: invalid pathspec: an empty pattern beside a non-empty one` and
exits 0. It also runs alone under BSan (see `REVIEW.md`).

## Fix

`generate_with_options` views `options.pathspec` through `StrArrayRef`. The contract
already makes that array readable. The function refuses a mix of empty and non-empty
patterns with batch 1's crate-private `pathspec::patterns_are_parsable`, before any
generator runs: code `-1`, `GIT_ERROR_INVALID`, "invalid pathspec: an empty pattern
beside a non-empty one". The check lives in the wrapper, so it is not a caller
obligation, and the `# Safety` section says it is checked. Empty and all-empty
pathspecs, which libgit2 never parses, still generate. As batch 1 argued, dropping the
empty patterns instead would change the compiled pathspec's prefix.

Regression: `tests/ub_unsafe/diff.rs::generate_with_options_refuses_empty_pattern_beside_others`
drives both orders through all six generators with one-byte heap strings, and checks that
`[]`, `[""]` and `["", ""]` still generate. With the check disabled, the test aborts
under ASan with the heap-buffer-overflow above. With it, the test passes under every
instrument.
