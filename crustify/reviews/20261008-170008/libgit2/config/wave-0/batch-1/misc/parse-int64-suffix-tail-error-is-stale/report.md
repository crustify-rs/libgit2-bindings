# `parse_int64` failed on text after a size suffix with an error that did not describe it

- **item**: `git_config_parse_int64` (symbol), wrapped as
  `crustify/rust/libgit2/libgit2/config/config.rs::parse_int64` (libgit2/config wave 0
  batch 2, `62c089598`)
- **affected commit**: `b83f28c34` (the review wave's base)
- **class**: copied C defect. The wrapper matched C exactly; C is what fails silently.
  The fix is a deliberate, documented divergence, so its regression lives in
  `unit_tests` and not in `tests/equiv`

## Finding

`git_config_parse_int64` (`src/libgit2/config.c`) reports every failure through
`fail_parse`, which calls `git_error_set(GIT_ERROR_CONFIG, "failed to parse '%s' as an
integer", ..)`. The one exception is text after a `k`/`m`/`g` suffix:

```c
		/* check that that there are no more characters after the
		 * given modifier suffix */
		if (num_end[1] != '\0')
			return -1;
```

It returns -1 and sets no error at all. The wrapper returned `Err(last())`, so the
`ErrorBuf` it handed back was whatever the thread's last error happened to be:

- after an unrelated failure, that failure (for example an ODB "object not found"); or
- on a clean thread, `GIT_ERROR_NONE`, "no error", which is an `Err` saying nothing failed.

The wrapper's doc said this was so, but documenting it doesn't make it a usable error.
A `Result::Err` should describe its own failure, and the crate already treats this as a
defect to correct elsewhere. `MailmapBox::from_buffer` and the worktree wrappers clear
the error, call C, and replace libgit2's silence with a descriptive error. The task
rules also forbid copying a confirmed C defect into Rust.

Only `parse_int64` is affected. `git_config_parse_int32` and `git_config_parse_bool`
call it, but each sets its own error when it fails ("as a 32-bit integer", "as a
boolean").

Reproducer: `src/main.rs` in this directory, which forbids `unsafe_code`. On the affected
commit:

```
$ LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run
after an unrelated error: Some(Odb) object not found - an earlier, unrelated lookup
after no error:           Some(None) no error

thread 'main' panicked at src/main.rs:39:5:
parse_int64("1kb") failed with an error that does not describe it
```

(exit 101). The fix also renames the wrapper to `parse_i64` (see
`../../conventions/parse-int-names-transliterate-c`), so the reproducer no longer builds
on the fixed tree. Its scenario continues as the unit regression below.

## Fix

`parse_i64` clears the thread's error before the call. If C fails and the error is
still `GIT_ERROR_NONE`, the wrapper sets the error C sets for every other failure:
`failed to parse '<value>' as an integer`, class `GIT_ERROR_CONFIG`, with the value's
bytes quoted unchanged. The error is set through `git_error_set`, so the thread's last
error describes the failure as well. One side effect: a success now leaves no earlier
error behind, where C leaves the earlier one in place. The doc records both differences.

Regressions:

- `config.rs::unit_tests::parse_i64_suffix_tail_reports_its_own_error` checks six
  suffix-tail inputs, each on a clean thread and after an unrelated error. It also
  checks that C's own errors pass through, that success clears the state, and that the
  32-bit and boolean parsers keep their own messages.
- `config.rs::unit_tests::integer_parse_error_quotes_the_value` checks the message
  builder. It runs under Miri.
- `tests/equiv/config.rs::parsers::parse_i64_sequences_match_c` replays C with its error
  cleared before each call, as the wrapper does, and maps C's silent failures to the
  corrected error. It requires at least four such failures. It also pins C's own
  behaviour: on `1kb`, C leaves a seeded sentinel in place.
- `tests/ub_safe/config.rs::parsers::first_parser_errors_race_to_init` now also runs
  the new `set` path on six threads at once, under ASan and TSan.
