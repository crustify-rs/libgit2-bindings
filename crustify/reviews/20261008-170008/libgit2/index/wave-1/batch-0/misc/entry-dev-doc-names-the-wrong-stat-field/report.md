# `IndexEntryRef::dev` was documented as the device holding the file

- **item**: `git_index_entry.dev` (`IndexEntryRef::dev`, `index/index.rs:2469` at the
  review base `3e694ec8b`; landed in `81e92a1ae`)
- **class**: misc (documentation contradicting C's behaviour)

## Finding

The getter said:

> The device holding the file, truncated to 32 bits.

That is git's meaning (`st_dev`). libgit2 fills the field differently when it stats a file
itself (`git_index_add_bypath`, `git_index_update_all`, checkout, submodule adds):
`git_index_entry__init_from_stat` (`src/libgit2/index.c:938`) stores
`entry->dev = st->st_rdev;`. That is the device a *special* file represents, so it is 0 for
every regular file. Its workdir iterator, by contrast, uses `st_dev`
(`src/libgit2/iterator.c:1536`). A C probe adding a regular file by path to a repository
index on this host printed:

```
entry dev=0 ino=34643291 | st_dev=89 st_rdev=0 st_ino=34643291
```

A caller comparing `dev()` with `std::fs::Metadata::dev()`, as the doc invites, would
find every libgit2-added entry "on another device". Entries read from an index file that
git wrote do carry git's `st_dev`.

## Fix

The doc now says that the field holds the device number as stored: git writes `st_dev`,
and libgit2 stores `st_rdev` (0 for a regular file) when it stats a file. The new unit test
`index::index::unit_tests::symbols::entry_getters_read_what_libgit2_stores` pins it. It
adds a file by path to a repository's index, borrows the entry through
`IndexEntryRef::from_ptr`, and checks that `dev()` equals the file's `rdev()` (0) while
`st_dev` differs, and that `ino()` equals `st_ino`.
