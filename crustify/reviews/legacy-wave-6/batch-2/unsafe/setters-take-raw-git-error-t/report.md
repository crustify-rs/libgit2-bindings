# Setters took the raw `git_error_t` where the safe `ErrorClass` now exists

Affected: `set`, `set_without_message`, `set_str` in
`crustify/rust/libgit2/libgit2/util/sys/errors.rs` (util wave 0 batch 2, commit
`ad5e40d35`), wrapping `git_error_set` and `git_error_set_str`.

## Finding

The three setters took the error class as `ffi::git_error_t`, bindgen's `u32` alias
for the C enum, and cast it to `int` at the call. The module documented it as a raw
seam: "the safe `git_error_t` wrapper (`util/errors.rs`) replaces it". That wrapper,
`ErrorClass`, landed in the same wave (batch 1, commit `cb97c64a8`) and was never
joined up: consumers wrote `set_str(ErrorClass::Odb.into(), ..)`, going through
`From<ErrorClass> for git_error_t` to feed a typed value back into an untyped
parameter, and any `u32` was accepted, `u32::MAX` landing in libgit2 as class `-1`.

The C header documents `error_class` as "one of the `git_error_t` enum", and no
libgit2 source passes anything else. `ErrorRef::class()` already returns
`Option<ErrorClass>` with `raw_class()` for ints C code may store; the setters are
the only safe way to produce such an int, which the type system should not offer.

## Fix

The setters take `ErrorClass` and pass `class.raw()`; the raw-seam note is replaced
by the contract. Passing a raw constant now fails to compile (new `compile_fail`
doctest on `set`). The `ub_safe` and `equiv` suites of the module are ported: the
"any int is stored verbatim" case is replaced by every named class round-tripping
through all four setters, and the unnamed-int case stays covered on the C side only
(`equiv/errors.rs`, `git_error_set_str(1000, ..)`). Consumers in
`tests/ub_safe/errors.rs` and `tests/equiv/errors.rs` drop their `.into()`.

Not a UB defect: no reproducer. Verified with the plain, ASan/UBSan, TSan, BSan and
Miri variants of the workspace after the change.
