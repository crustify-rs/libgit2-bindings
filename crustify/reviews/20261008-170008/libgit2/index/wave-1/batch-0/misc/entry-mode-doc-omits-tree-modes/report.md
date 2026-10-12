# `IndexEntryRef::mode` claimed every entry libgit2 adds has a `FileMode` value

- **item**: `git_index_entry.mode` (`IndexEntryRef::mode`, `index/index.rs:2486` at the
  review base `3e694ec8b`; landed in `81e92a1ae`)
- **class**: misc (documentation contradicting C's behaviour)

## Finding

The getter said:

> The entry's mode as stored: a [`FileMode`] value for every entry libgit2 adds, but
> whatever an index file spelled for one it read.

Only `git_index_add` (`index.c:1729`) and `git_index_conflict_add` (`:1870`) check the mode
(`valid_filemode`), and the working-directory paths build it with
`git_index__create_mode`. `git_index_read_tree` adds entries without a check:
`read_tree_cb` copies the tree entry's raw attribute (`entry->mode = tentry->attr;`,
`index.c:3433`), and the tree parser keeps any octal mode. A C probe that wrote a tree
holding git's legacy `100664 legacy` entry and read it into a fresh index printed
`read_tree rc=0 mode=100664`. That value has no `FileMode` variant, so `filemode()` is
`None` for an entry libgit2 itself added.

## Fix

The doc now lists where a `FileMode` value is guaranteed (`git_index_add`,
`git_index_conflict_add`, entries statted in the working directory). It also lists where
the raw value can be anything: an index file's spelling, and a tree's raw mode through
`git_index_read_tree` (`0100664`, say). The unit test
`index::index::unit_tests::symbols::entry_getters_read_what_libgit2_stores` writes such a
tree, reads it into an index and checks `(mode(), filemode()) == (0o100664, None)`.
`git_index_read_tree` itself is scheduled for wave 2; the test drives it through `ffi`.
