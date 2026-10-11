# `entry` / `get_mapped` rewrote a previous `GIT_ERROR_OS` error when they spent a lower backend's failure

- **items**: `git_config_get_entry` (`ConfigRef::entry`, wave 1) and
  `git_config_get_mapped` (`ConfigRef::get_mapped`, libgit2/config wave 2 batch 1,
  `d6a13729c`), `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `3a5814a4e` (the review wave's base)
- **class**: equivalence defect: the thread's last error after a successful lookup
  differs from C's

## Finding

Neither wrapper calls C's lookup on the live config, because `get_entry`
(`src/libgit2/config.c:811`) forms a pointer from an entry the backend never wrote when a
backend fails to reread its file. Both first reread every backend through a multivar
iterator (`reread_through`), lowest priority first, then look the key up on a snapshot.
The iterator also rereads backends *below* the one holding the key, which C never reads.
When one of those fails, its error replaces the thread's last error. The wrappers
noticed (`last() != before`) and put the previous error back:

```rust
match before.class() {
    Some(ErrorClass::None) => clear(),
    Some(class) => set(class, before.message()),
    None => {}
}
```

`set` is `git_error_set(class, "%s", message)`. For `GIT_ERROR_OS`, `git_error_vset`
(`src/util/errors.c`) appends `": "` after the formatted message, and then the
description of the thread's `errno` if `errno` is non-zero, and it resets `errno`. So a
restored OS-class error gained a suffix every time a lookup spent a failure. C's lookup
stops at the backend holding the key and leaves the error alone. An error of a class
`ErrorClass` does not name (`None`) was not restored at all, so the spent backend's
failure stayed in its place.

## Reproducer

`src/main.rs` builds the same two-backend config on each side: a lower `LOCAL` file and
an `APP` file holding `a.b`. It then breaks the lower file on disk before each lookup,
changing its size so the next reread parses it again. Before each call it seeds the same
previous error with `set_str`, which stores it verbatim, and it compares C's
`git_config_get_entry` / `git_config_get_mapped` with the wrappers: the value, the class
and the message left afterwards. C never reaches the broken file, so the reference side
stays defined.

```sh
LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run
```

On `3a5814a4e` (exit 101):

```
entry, previous Odb:
  rust (Ok("input"), 9, "failed to stat '/nowhere': No such file or directory")
  c    (Ok("input"), 9, "failed to stat '/nowhere': No such file or directory")
get_mapped, previous Odb:
  rust (Ok("42"), 9, "failed to stat '/nowhere': No such file or directory")
  c    (Ok("42"), 9, "failed to stat '/nowhere': No such file or directory")
entry, previous Os:
  rust (Ok("input"), 2, "failed to stat '/nowhere': No such file or directory: ")
  c    (Ok("input"), 2, "failed to stat '/nowhere': No such file or directory")
get_mapped, previous Os:
  rust (Ok("42"), 2, "failed to stat '/nowhere': No such file or directory: ")
  c    (Ok("42"), 2, "failed to stat '/nowhere': No such file or directory")

thread 'main' panicked at src/main.rs:159:5:
the wrappers left a different previous error than C
```

On the fixed tree it prints matching pairs and exits 0.

## Fix

A private `restore_last` puts the previous error back with
`git_error_set_str(before.raw_class(), message)`, which copies the message verbatim and
stores the raw class without interpreting it, or with `clear()` when there was none.
`get_mapped` is now composed from `entry` (see
`../../misc/valueless-map-miss-formats-null`), so `entry` is the only restore site.

Regression: `tests/equiv/config.rs`,
`mapping_callbacks::spent_backend_failures_keep_any_previous_error`. It compares both
wrappers with C after no previous error, an ODB-class one, an OS-class one and an
undefined class (4242), for a valued and a valueless key. It fails on the old restore
(checked by reverting `restore_last` alone) and passes on the fixed tree.
