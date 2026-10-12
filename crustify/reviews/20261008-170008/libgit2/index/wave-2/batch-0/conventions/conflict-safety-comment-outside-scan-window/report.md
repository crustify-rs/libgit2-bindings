# `IndexMut::conflict`'s `unsafe` block sat outside `scan-unsafe`'s SAFETY window

- **item**: `git_index_conflict_get`, `crustify/rust/libgit2/libgit2/index/index.rs`
  (`IndexMut::conflict`)
- **affected commit**: `cb77d2e9b` (libgit2/index wave 2 batch 0)
- **class**: conventions ("every unsafe block carries a specific, falsifiable
  `// SAFETY:` comment", as measured by `crustify scan-unsafe`)

## Finding

rustfmt split the call across two lines:

```rust
        // SAFETY: a live index no other holder reaches, ...
        // ... for the call.
        let rc =
            unsafe { ffi::git_index_conflict_get(ancestor, ours, theirs, index, path.as_ptr()) };
```

Clippy's `undocumented_unsafe_blocks` accepts this. `crustify scan-unsafe`'s
`undocumented_unsafe` counter does not: it looks for a SAFETY comment on the `unsafe`
line or directly above it, past comments and attributes only. The `let rc =` line breaks
that. This was the only undocumented site the scan reported in `index/index.rs` at the
review base. The batch noted that count in its summary without tracing it to this block.

## Fix

The key pointer is bound beside the index pointer. The call then fits on the
`let rc = unsafe { … };` line under its comment, with no other change. The scan's
lexical rule now finds no undocumented site in `index/index.rs`, nor in the two
suites this review touched.
