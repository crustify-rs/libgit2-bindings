# A config transaction could stage every write except a string

- **items**: `git_config_set_string` (symbol, wave 1 batch 2, `760d8dc91`) and
  `git_config_lock` (symbol; `ConfigMut::lock` → `ConfigTransaction`)
- **affected commit**: `d79771f6f` (the review wave's base)
- **class**: missing safe variant (API gap); no UB and no equivalence failure

## Finding

`ConfigMut::lock` consumes the exclusive config handle and returns a
`ConfigTransaction`. On purpose, the transaction never hands the `ConfigMut` back: that
would allow a second `git_config_lock` of the same backend. Writes staged under the lock
therefore go through methods on the transaction itself. Batch 1 added one for each
mutation it wrapped: `set_bool`, `set_i32`, `set_i64`, `set_multivar`, `delete_entry`
and `delete_multivar`.

Batch 2 wrapped `git_config_set_string` as `ConfigMut::set_string` and landed in parallel.
It added no transaction variant, and the rebase did not add one either. A caller holding
a transaction could stage a boolean or an integer, but not a plain string value (a path,
a URL, a name). The only workaround, `set_multivar(name, "^$"-style regexp, value)`, has
different semantics and fails on a key that is already a multivar. Dropping the
transaction to write the string would release the lock and defeat its purpose.

## Fix

Added `ConfigTransaction::set_string` (`Wraps: git_config_set_string`, duplicating the
anchor as the sibling transaction methods do). It delegates to
`ConfigMut::set_string` through the transaction's retained exclusive borrow, exactly
as the other staged writes do.

Tests:

- `tests/ub_safe/config.rs::file_access::file_api_sequences` stages a non-UTF-8 string
  through the transaction from a temporary `CString` dropped right after the call, so C
  must not have retained it. It checks that the value is not published before commit,
  and that dropping the transaction aborts it (the file is unchanged).
- `tests/equiv/config.rs::file_access::file_sequences_match_c` adds two steps to the
  locked sequence: a staged string and an invalid name. Each compares code, class and
  message with C's `git_config_set_string` on the locked C config. The files are compared
  after both transactions are aborted.
