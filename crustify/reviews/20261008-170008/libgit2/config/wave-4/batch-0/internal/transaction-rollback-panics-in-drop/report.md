# Dropping a transaction panicked when the rollback failed

- **item**: `git_config_backend` (type): `BackendTransaction`'s `Drop`
  (`unlock` slot), `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `e797d4624` (landed by `beeda6a07`)
- **class**: internal

## Finding

An unfinished `BackendTransaction` rolls back on drop, which is right. But it asserted the
callback's status:

```rust
let rc = unsafe { unlock(self.backend.as_mut_ptr(), 0) };
assert_eq!(rc, 0, "backend rollback failed");
```

A backend's `unlock(…, 0)` may fail (a file backend's lock file can vanish, and a custom
backend may refuse). Then the drop panicked. If the transaction was being dropped
*because* the caller was already unwinding (a panic inside the
transaction's scope), the second panic aborted the process. C never treats this as fatal:
`git_transaction_free` (`src/libgit2/transaction.c:381`) calls
`git_config_unlock(tx->cfg, tx->cfg_data, false)` and discards the result.
`BackendTransaction::finish(false)` already reports the status to a caller who wants it.

## Fix

The drop calls the rollback and discards its status, like `git_transaction_free`.
The `Drop` impl's doc comment says so and points to `finish` for callers who need the
status.

Regression: `unit_tests::backends::failed_rollback_on_drop_is_discarded`. The mock
backend's rollback fails. The test drops a transaction normally, then panics inside
`catch_unwind` with a transaction open: the rollback runs and the process survives. It
then checks that `finish(false)` still returns the error (skipped under Miri, where
error construction cannot call C) and that `finish(true)` still commits. On the base
commit the first drop panics with `backend rollback failed`.
