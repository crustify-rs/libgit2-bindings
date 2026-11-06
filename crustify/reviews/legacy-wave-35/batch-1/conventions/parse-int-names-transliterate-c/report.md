# `parse_int32` / `parse_int64` kept the C spelling of the integer types

- **items**: `git_config_parse_int32`, `git_config_parse_int64` (symbols), wrapped in
  `crustify/rust/libgit2/libgit2/config/config.rs` (libgit2/config wave 0 batch 2,
  `62c089598`)
- **affected commit**: `b83f28c34`
- **class**: naming convention

## Finding

The conventions say: "Drop a C name's library or owning-type prefix wherever the crate,
module or receiver already supplies it, and prefer the idiomatic Rust term over a
transliterated C one." The wrappers dropped the `git_config_` prefix correctly, but
`int32`/`int64` transliterate C's `int32_t`/`int64_t`. The functions return Rust's
`i32`/`i64`, and those are the idiomatic names (compare `str::parse::<i32>` and
`i32::from_str_radix`). No other function in the crate uses `int32`/`int64`.

This matters beyond these two functions. Config wave 1 wraps `git_config_get_int32`,
`git_config_get_int64`, `git_config_set_int32` and `git_config_set_int64` in the same
module, and those wrappers will follow whatever spelling the parsers use.

## Fix

Renamed `parse_int32` to `parse_i32` and `parse_int64` to `parse_i64`, and updated
the intra-doc link and the callers in `unit_tests`, `tests/equiv/config.rs` and
`tests/ub_safe/config.rs`. The `Wraps:` anchors still carry the C names. Behaviour is
unchanged apart from the separate `misc/parse-int64-suffix-tail-error-is-stale` fix.
