# `Pathspec::new` let libgit2 read one byte past an empty pattern placed next to a non-empty one

- **item**: `git_pathspec_new`, the constructor of `git_pathspec` (type), wrapped as
  `crustify/rust/libgit2/libgit2/index/pathspec.rs::Pathspec::new` (libgit2/index wave 0,
  `ab190ae80`). It belongs to this review wave's type batch, not to this batch's callback.
  It is filed here because this batch's matched-path regression test found it (below).
- **affected commit**: `ab3dabdcc` (the review wave's base)
- **class**: undefined behaviour in libgit2, reachable from the safe public API
  (`#![forbid(unsafe_code)]` reproducer, AddressSanitizer)

## Finding

`git_pathspec__vinit` (`src/libgit2/pathspec.c:67-103`) returns early only when *every*
entry is empty (`git_pathspec_is_empty`). If any entry is non-empty, it passes each
entry to `git_attr_fnmatch__parse`, which first calls `parse_optimized_patterns`
(`src/libgit2/attr_file.c:837-851`):

```c
if (!pattern[1] && (pattern[0] == '*' || pattern[0] == '.')) {
```

For an empty pattern, `pattern[1]` is the byte after the terminator. That read is out of
bounds, and so undefined behaviour. The entry is then dropped as `GIT_ENOTFOUND`, so the
compiled pathspec looks fine, and only a sanitizer notices. `Pathspec::new` passed any
`StrArrayRef` straight to `git_pathspec_new`. `StrArrayView::new(&[c"a", c""])` or an
owned `StrArray` holding the same two strings reaches the read without `unsafe`.

This batch hit it with `git_index_remove_all`: a first draft of
`unit_tests::matched_path::unusable_patterns_match_every_path_without_a_pattern` passed
`["#a", "\nb", "\r\nb", ""]`, and the `asan-ubsan` run of the lib tests aborted with a
`global-buffer-overflow` in `parse_optimized_patterns` under `git_pathspec__init` ←
`index_apply_to_all`. The same parse sits behind `git_index_add_all` / `_update_all`
(`index.c:3707`, `:3801`), `git_diff_*` options (`diff_generate.c:498`) and checkout
paths (`checkout.c:1330`). None of those has a safe wrapper yet, so `Pathspec::new` was
the only safe path to it.

## Reproducer

`src/main.rs` in this directory (`#![forbid(unsafe_code)]`) compiles `["a", ""]` from two
heap `CString`s, so the byte after `""` is in a redzone. Its doc comment gives the command
(the `asan-ubsan` variant, `cargo +nightly run`). On `ab3dabdcc` it aborts (exit 134).
The trace below has the libgit2 and reproducer frames symbolized with
`llvm-symbolizer-18`:

```
==1735294==ERROR: AddressSanitizer: heap-buffer-overflow on address 0x7ad4465e0031 at pc 0x7eb447eafb0a bp 0x7fffc5698560 sp 0x7fffc5698558
READ of size 1 at 0x7ad4465e0031 thread T0
    #0 parse_optimized_patterns  src/libgit2/attr_file.c:842:7       (libgit2.so.1.9+0x1b1b09)
    #1 git_attr_fnmatch__parse   src/libgit2/attr_file.c:730:6       (libgit2.so.1.9+0x2fbc72)
    #2 git_pathspec__vinit       src/libgit2/pathspec.c:89:9         (libgit2.so.1.9+0x2fc711)
    #3 git_pathspec__init        src/libgit2/pathspec.c:242:15       (libgit2.so.1.9+0x2fc8a3)
       git_pathspec_new          src/libgit2/pathspec.c:262:15
    #4 <Pathspec>::new           crustify/rust/libgit2/libgit2/index/pathspec.rs:122
    #5 pathspec_empty_entry_overread::main  src/main.rs
    ...
0x7ad4465e0031 is located 0 bytes after 1-byte region [0x7ad4465e0030,0x7ad4465e0031)
allocated by thread T0 here:
    ...
    #2 <alloc::ffi::c_str::CString>::new::<&str>
SUMMARY: AddressSanitizer: heap-buffer-overflow (/target/crustify/builds/asan-ubsan/libgit2.so.1.9+0x1b1b09)
```

## Fix

`Pathspec::new` now checks its patterns with the new crate-private
`pathspec::patterns_are_parsable` before libgit2 sees them. That function refuses an
array holding both an empty and a non-empty pattern, setting a `GIT_ERROR_INVALID` error
("invalid pathspec: an empty pattern beside a non-empty one"). Empty and all-empty
arrays, which libgit2 never parses, still compile as before. The constructor's doc states
the refusal.

Dropping the empty entries was not chosen: libgit2 also derives the pathspec's prefix
(`git_pathspec_prefix`, which bounds match-list iteration) from the raw strings, empty
ones included, so a rewritten array does not necessarily compile to the same pathspec.
Refusing it means the wrapper never changes its input silently. The check is
`pub(crate)` so the add/remove/update, diff and checkout wrappers can reuse it.

With the fix, the reproducer prints `refused: …` and exits 0.

Regressions:

- `tests/ub_safe/pathspec.rs::empty_patterns_beside_others_are_refused`: both orders,
  borrowed and owned arrays of one-byte heap strings, under every instrument; an all-empty
  array still compiles.
- `pathspec.rs::unit_tests::mixed_empty_patterns_are_refused_before_libgit2`: the error
  class and message for three mixes, and acceptance of `[]`, `[""]`, `["", ""]` and
  `["a", "#x"]`.
