# `IndexRef::write` let libgit2 read past an empty entry path while scanning racily clean entries

- **item**: `git_index_write`, wrapped as
  `crustify/rust/libgit2/libgit2/index/index.rs::IndexRef::write` (libgit2/index wave 1
  batch 3, `7f4d85bd7`)
- **affected commit**: `3e694ec8b` (the review wave's base)
- **class**: undefined behaviour in libgit2, reachable from the safe public API
  (`#![forbid(unsafe_code)]` reproducer, AddressSanitizer)

## Finding

Before writing, `git_index_write` calls `truncate_racily_clean`
(`src/libgit2/index.c:770-822`). When a repository with a working directory owns the
index, that function collects the path of each entry that is not `UPTODATE`, not a
gitlink, and not older than the stamp the index file was last read with. It then passes
those paths to `git_diff_index_to_workdir` as `diff_opts.pathspec`. The diff compiles them
with `git_pathspec__vinit` (`diff_generate.c:498`), which parses every pattern once one is
non-empty. `parse_optimized_patterns` (`attr_file.c:842`) reads `pattern[1]` first. For
an entry whose path is `""`, that is the byte after its terminator. `index_entry_create`
allocates the path as the last byte of the entry's block, so the byte lies past the
allocation.

libgit2 accepts an empty path in an index file: `read_entry` → `index_entry_dup` →
`index_entry_create` validates it with `git_path_is_valid`, which lets `""` through.
Entries read from a file are never `UPTODATE`, and a modification time at or after the
file's makes them racily clean. So a file holding the entries `""` and `"a"` is enough. It
can be a repository's own `.git/index`, read by `RepositoryRef::index`, or any file opened
with `Index::open` and installed with `RepositoryMut::set_index`. `IndexRef::write` then
reaches the read. Batch 3's wrapper took the shared handle and checked nothing there.

This is the same parser defect as wave 0's `misc/pathspec-empty-entry-overread`. The
earlier fix guarded the pathspec arrays a caller passes in. Here libgit2 builds the array
itself from index entries.

## Reproducer

`src/main.rs` in this directory is `#![forbid(unsafe_code)]`. It initialises a
non-bare repository and writes a version 2 index file of two entries, `""` and `"a"`,
modified in 2038, with a zero checksum (`index.skipHash`) as `.git/index`. It then calls
`repo.as_ref().index()?.as_ref().write()`. Its doc comment gives the command (`asan-ubsan`
variant, `cargo +nightly run`).

On `3e694ec8b` it aborts with exit 134. The frames are symbolized with
`llvm-symbolizer-18`; frame #0 is `parse_optimized_patterns`, inlined into
`git_attr_fnmatch__parse`:

```
==2235114==ERROR: AddressSanitizer: heap-buffer-overflow on address 0x70be9be0caf1 at pc 0x740e9d6b7b0a bp 0x7fff14a2f0e0 sp 0x7fff14a2f0d8
READ of size 1 at 0x70be9be0caf1 thread T0
    #0 git_attr_fnmatch__parse        src/libgit2/attr_file.c:842:7     (libgit2.so.1.9+0x1b1b09)
    #1 git_pathspec__vinit            src/libgit2/pathspec.c:89:9       (libgit2.so.1.9+0x2fbc72)
    #2 git_diff__from_iterators       src/libgit2/diff_generate.c:498:7 (libgit2.so.1.9+0x2236b2)
    #3 git_diff_index_to_workdir      src/libgit2/diff_generate.c:1495:15 (libgit2.so.1.9+0x227ad7)
    #4 git_index_write                src/libgit2/index.c:801:15        (libgit2.so.1.9+0x2596ec)
    #5 <IndexRef>::write              crustify/rust/libgit2/libgit2/index/index.rs:1428:25
    #6 index_write_empty_path_overread::main  src/main.rs:65:26
    ...
0x70be9be0caf1 is located 0 bytes after 97-byte region [0x70be9be0ca90,0x70be9be0caf1)
allocated by thread T0 here:
    #0 __interceptor_malloc
    #1 git__calloc                    src/util/alloc.h:19:12
    #2 index_entry_create             src/libgit2/index.c:993:10
    #3 index_entry_dup                src/libgit2/index.c:1126:6
    #4 parse_index                    src/libgit2/index.c:2768:6
    #5 git_index_read                 src/libgit2/index.c:735:11
    #6 git_index_open_ext             src/libgit2/index.c:476:37
    #7 git_repository_index__weakptr  src/libgit2/repository.c:1719:11
    #8 git_repository_index           src/libgit2/repository.c:1742:6
    #9 <RepositoryRef>::index         crustify/rust/libgit2/libgit2/repository/repository.rs:3015:27
    #10 index_write_empty_path_overread::main  src/main.rs:63:31
SUMMARY: AddressSanitizer: heap-buffer-overflow (libgit2.so.1.9+0x1b1b09)
```

With the fix it prints `refused: RepositoryError { code: Generic, error: ErrorBuf {
class: Index, message: "cannot write an index holding an entry with an empty path beside
others" } }` and exits 0. Run alone under BSan (`cargo bsan run`, `bsan` build), it
exits 0 on the fixed tree.

## Fix

`IndexRef::write` now calls the new private `IndexRef::racy_paths_parsable` first. It
refuses with a `GIT_ERROR_INDEX` error, code `-1`, when three things hold. A repository
with a working directory owns the index (`git_index_owner` and `RepositoryRef::workdir`).
The index holds an entry with an empty path. It holds another entry. Both entries must be
neither up to date (`EntryExtendedFlags::UP_TO_DATE`) nor submodules. This mirrors
`truncate_racily_clean`'s filter, except for its timestamp test: libgit2 keeps the read
stamp private, so an entry that *may* be racy counts. Indexes with no owner, or whose owner
is bare, are still written. So is an index whose only candidate is the empty path, since
an all-empty pathspec is never parsed. The doc of `write` states the refusal.

Removing the empty-path entry, or skipping the scan, was not chosen. Either would change
the index or its file behind the caller's back. A refused write leaves both as they were.

Regressions:

- `tests/ub_safe/index/edits.rs::writes_refuse_empty_paths_libgit2_would_scan`: this
  reproducer's scenario through the repository's own index and through `set_index`. The
  file is left unchanged. The same index is written once no repository owns it.
- `tests/ub_safe/index/edits.rs::writes_scan_a_lone_empty_path`: a lone racily clean `""`
  still goes through libgit2's scan and write, in a child process.
- `index.rs::unit_tests::edits::writes_refuse_empty_paths_beside_scannable_entries`: which
  entries count (up to date and submodule entries do not), and the bare, ownerless and
  dropped-owner cases left to libgit2.
