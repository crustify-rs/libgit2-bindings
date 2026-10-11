# Read-only `delete_entry` / `delete_multivar` failed with an error that did not describe it

- **items**: `git_config_delete_entry`, `git_config_delete_multivar` (symbols), wrapped as
  `ConfigMut::delete_entry` / `ConfigMut::delete_multivar` and the
  `ConfigTransaction` methods that delegate to them
  (`crustify/rust/libgit2/libgit2/config/config.rs`, libgit2/config wave 1 batch 1,
  `5a0f495b6`)
- **affected commit**: `d79771f6f` (the review wave's base)
- **class**: copied C defect. The wrappers matched C exactly, and C is what fails
  silently. The fix is a deliberate, documented divergence, so its regression lives in
  `unit_tests`. `tests/equiv` keeps C's code and pins C's silence.

## Finding

`get_writer` returns NULL when a config has no writable backend. That covers an empty
config, a snapshot (every backend `readonly`), and a config whose write order excludes
every level. The two delete routines then return `GIT_EREADONLY` without calling
`git_error_set` (`src/libgit2/config.c`):

```c
int git_config_delete_entry(git_config *config, const char *name)
{
	git_config_backend *backend;

	if ((backend = get_writer(config)) == NULL)
		return GIT_EREADONLY;
```

`git_config_delete_multivar` has the same early return. Every other routine that checks
the same condition sets an error first: `git_config_set_string` and
`git_config_set_multivar` set `cannot set '%s': the configuration is read-only`, and
`git_config_lock` sets `cannot lock: the configuration is read-only`.

The wrappers built their `ConfigError` from `last()`, so the error they returned was
whatever the thread's last error happened to be:

- after an unrelated failure, that failure (here an ODB "object not found"), with its
  class; or
- on a clean thread, `GIT_ERROR_NONE` / "no error", an `Err` whose error says nothing
  failed.

The code (`GIT_EREADONLY`) was right, but `ConfigError::class()`, `message()`, `Display`
and the `ErrorBuf` conversion all described a different failure. The crate already treats
this pattern as a defect to correct: `parse_i64` and `get_i64` (wave 0 and wave 1),
`MailmapBox::from_buffer` and the worktree wrappers each replace libgit2's silence with
an error of their own.

The existing equivalence test (`snapshot_strings_and_readonly_mutations_match_c`) cleared
the error before each side, so both sides read "no error" and matched. That is how the
defect passed.

Reproducer: `src/main.rs` in this directory, which forbids `unsafe_code`. It deletes
through an empty config and a snapshot, once after an unrelated error and once on a clean
thread. On the affected commit:

```
$ LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run
empty: delete_entry after an earlier error:  Some(ReadOnly) Some(Odb) object not found - an earlier, unrelated lookup
empty: delete_entry after no error:          Some(ReadOnly) Some(None) no error
empty: delete_multivar after an earlier error: Some(ReadOnly) Some(Odb) object not found - an earlier, unrelated lookup
empty: delete_multivar after no error:       Some(ReadOnly) Some(None) no error
snapshot: delete_entry after an earlier error: Some(ReadOnly) Some(Odb) object not found - an earlier, unrelated lookup
snapshot: delete_entry after no error:       Some(ReadOnly) Some(None) no error
snapshot: delete_multivar after an earlier error: Some(ReadOnly) Some(Odb) object not found - an earlier, unrelated lookup
snapshot: delete_multivar after no error:    Some(ReadOnly) Some(None) no error

thread 'main' panicked at src/main.rs:56:5:
a read-only delete failed with an error that does not describe it
```

(exit 101). On the fixed tree every line reads `Some(ReadOnly) Some(Config) cannot
delete 'a.b': the configuration is read-only`, and it exits 0.

## Fix

Both wrappers now clear the thread's error before calling C. A new private
`delete_status` then checks the result: if C returned `GIT_EREADONLY` and the error is
still `GIT_ERROR_NONE`, it sets `cannot delete '<name>': the configuration is read-only`
in `GIT_ERROR_CONFIG`. That follows `git_config_set_string`'s wording. The name's bytes
are quoted unchanged through `set`'s `"%s"` format, so a `%` in the name stays literal.
The error goes through `git_error_set`, so the thread's last error describes the failure
too. Backend errors (`could not find key .. to delete`, a multivar refused by
`delete_entry`, a bad regex) pass through unchanged.

One side effect, the same one `parse_i64` / `get_i64` have: a success now leaves no
earlier error behind, where C leaves the earlier one in place. Both wrapper docs record
the divergence.

Regressions:

- `config.rs::unit_tests::readonly_deletes_report_their_own_error` covers both routines
  on an empty config and on a snapshot, each seeded and clean, with a `%` in the name. It
  checks code, class, message and that `last()` matches. It also checks that the file
  backend's own errors pass through and that a success clears the earlier error.
- `config.rs::unit_tests::readonly_delete_error_quotes_the_name` checks the message
  builder with empty, non-UTF-8 and `%` names. It runs under Miri.
- `tests/ub_safe/config.rs::writes_and_snapshots::readonly_delete_errors_are_owned`
  checks that the corrected errors outlive the config, the name and a later error on the
  thread.
- `tests/equiv/config.rs::file_access::snapshot_strings_and_readonly_mutations_match_c`
  now compares the code with C. It pins C's silence (`GIT_ERROR_NONE`, "no error" after
  the clear) and the wrapper's corrected error, rather than requiring the two to match.
