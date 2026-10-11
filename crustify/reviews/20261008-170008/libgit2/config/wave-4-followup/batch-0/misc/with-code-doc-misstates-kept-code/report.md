# `ConfigError::with_code` documented the wrong code for `ErrorCode::Ok`

- **item**: `ConfigError::with_code` (added for `CustomBackend` stores by `a66691e90`),
  `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `df0deef13`
- **class**: misc (documentation; the method had no test)

## Finding

The doc said that `with_code(ErrorCode::Ok)` would read as success, "so the code then
stays `GIT_ERROR`". The method ignores `Ok` and keeps the error's current code, which is
`GIT_ERROR` only for a fresh `ConfigError::new`. For a store that re-codes an error, or
forwards one libgit2 returned, `with_code(Ok)` keeps that code: for example
`new(..).with_code(NotFound).with_code(Ok)` is still `GIT_ENOTFOUND`. No test covered
`ConfigError::new`, `with_code` or `raise`.

## Fix

The doc now says that the error keeps its code. The new inline test
`config::unit_tests::store_errors_take_codes_and_raise_verbatim` covers `new`'s
`GIT_ERROR`, re-coding, `Ok` keeping both a re-coded and a fresh code, and `raise`. It
checks that `raise` returns the code and restores the OS-class message verbatim (no
`errno` suffix, `%s` literal).
