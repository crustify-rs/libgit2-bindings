# `Index::open_with_oid_type`'s unsafe block sat out of its SAFETY comment's reach

- **item**: `git_index_options` (its consumer `git_index_open_ext`, emitted with the type
  as `Index::open_with_oid_type`, `index/index.rs:488` at the review base `3e694ec8b`;
  landed in `81e92a1ae`)
- **class**: conventions (every unsafe block carries a SAFETY comment)

## Finding

rustfmt had split the binding, so the `// SAFETY:` comment sat above `let rc =` rather
than above the `unsafe` line:

```rust
// SAFETY: as in `open`; `opts` is a live options struct of this version naming a
// valid type, which the routine reads for the call and does not retain.
let rc =
    unsafe { ffi::git_index_open_ext(&mut out, path.as_ptr(), opts.as_ref().as_ptr()) };
```

`crustify scan-unsafe` counts the site as undocumented. It was the only undocumented site
in `index/index.rs` (`undocumented_unsafe.per_file`: 1, in the wave baseline scan and at
`3e694ec8b`; batch 1's review attributes it to this site too). Clippy's `undocumented_unsafe_blocks` accepts the shape, which is why the batch's
lint run did not catch it.

## Fix

The options handle is borrowed on its own line (`let opts = opts.as_ref();`), so the call
fits on the `unsafe` line under the comment. The comment now says that `opts` *borrows*
the live options, and the borrow keeps them alive by construction. Behaviour is
unchanged. The scan now counts no undocumented site in `index/index.rs` (total 46 → 45).
