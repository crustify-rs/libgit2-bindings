# `multivar_iterator`'s docs limit C's allocation-failure leak to the regex-less form

- **item**: `git_config_multivar_iterator_new` (`ConfigRef::multivar_iterator`),
  `crustify/rust/libgit2/libgit2/config/config.rs`; also `IteratorMut::advance`'s doc
  comment in `config/sys/config.rs`
- **affected commit**: `8491945f1` (libgit2/config wave 4 batch 1), reviewed at
  `e797d4624`
- **class**: misc (documentation)

## Finding

`multivar_iterator`'s docs (`config.rs:1392` at the reviewed commit) ended:

> C's existing outer-allocation-failure leak without a regex remains.

The leak is real. `git_config_multivar_iterator_new` builds the inner all-iterator first,
then `GIT_ERROR_CHECK_ALLOC(iter)` (`src/libgit2/config.c:1138-1139`) returns without
freeing it when the `multivar_iter` allocation fails (reported in
`../../../wave-1/batch-0/misc/multivar-iterator-leaks-inner-on-allocation-failure`).
"Without a regex" is wrong, though. The wrapper never passes the regex to C: it always
calls `git_config_multivar_iterator_new(&out, cfg, name, NULL)` (`config.rs:1405-1411`)
and applies the Rust PCRE2 filter to the result. So `multivar_iterator(name,
Some(regexp))` goes through the same C allocation sequence and leaks the same way. A
caller could read the sentence as saying that the filtered form avoids the leak. The
inline OOM test already skips that allocation for both forms
(`(2, …None)` and `(2, …Some(c"^a"))` grant the shell's allocation, never refuse it).

When the batch moved `IteratorMut::advance`'s `Wraps: git_config_next` anchor to
`advance_entry`, it also left an empty `///` line at the end of `advance`'s doc comment
(`sys/config.rs:725`).

## Fix

The sentence now says that C's name-selection constructor runs either way, so its leak
of the inner iterator when its own allocation fails (`config.c:1139`) remains, with or
without a regex. The empty doc line is gone. No code changed.
