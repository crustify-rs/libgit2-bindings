# The matched-path trampoline dereferenced its payload outside the callback's `impl`

- **item**: `git_index_matched_path_cb` (callback), wrapped as
  `crustify/rust/libgit2/libgit2/index/index.rs::MatchedPathCallback` (libgit2/index
  wave 0 batch 8, `6c09dba0d`)
- **affected commit**: `ab3dabdcc` (the review wave's base)
- **class**: static-safety smell; no behaviour change

## Finding

`crustify scan-unsafe crustify/rust/libgit2 --sites raw_ptr_derefs_outside_impl` reported
one site in the workset's home, `libgit2/libgit2/index/index.rs:662`:

```rust
unsafe extern "C" fn matched_path_with(path, pattern, payload: *mut c_void) -> c_int {
    let callback = unsafe { &mut *payload.cast::<MatchedPathCallback<'_>>() };
```

The trampoline was a free module-level function, so its payload dereference counted as
a raw pointer dereference outside any wrapper `impl`. It also named the callback with an
elided `'_` rather than the lifetime of the handle `scan_with` lent.

## Fix

The trampoline is now the private associated function `MatchedPathCallback::trampoline`,
passed by `scan_with` as `Some(Self::trampoline)`. It casts the payload to `Self`: the
same `MatchedPathCallback<'f>` instance whose `&mut self` `scan_with` turned into the
payload. Its body, `# Safety` contract and `SAFETY` comments are unchanged.

`raw_ptr_derefs_outside_impl` goes 16 → 15 on the workspace and 1 → 0 in `index.rs`.
No other `*_smell` site or outside-`impl` counter points into the workset. The callback
adds no public `unsafe fn` and no public raw pointer. `MatchedPathCallback::new`,
`call` and `stopped` are safe, and `scan_with` is the documented `pub(crate)` seam for
the add/remove/update wrappers that a later wave will add.

Regression evidence: the existing inline and integration suites
(`index.rs::unit_tests::matched_path::*`, `tests/ub_safe/index.rs::matched_path_callback`,
`tests/equiv/index.rs::matched_path_callback`) pass unchanged. That includes the
Rust-only tests under Miri.
