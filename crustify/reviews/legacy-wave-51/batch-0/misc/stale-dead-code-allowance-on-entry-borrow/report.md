# Stale `#[allow(dead_code)]` on `IndexEntryRef::from_ptr`

- **item**: `git_index_entry` (`IndexEntryRef::from_ptr`, `index/index.rs:2430` at the
  review base `3e694ec8b`; landed in `81e92a1ae`)
- **class**: misc (stale lint allowance and comment)

## Finding

```rust
#[allow(dead_code)] // Used by the index symbol waves, which lend entries.
pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_index_entry) -> Option<Self> {
```

The allowance was right when batch 0 landed: nothing called the borrow yet. Batch 2
(`05a56b569`) then made `IndexMut::find_prefix` call it (through `path_at_starts_with`),
and batch 1's review (`c5312ce66`) added two more callers.
The allowance now silences nothing, and its comment says the function is unused. Kept, it
would also hide the warning if the last caller ever went away.

## Fix

Removed the attribute and its comment. `cargo check --workspace --all-targets` and
`cargo clippy --workspace --all-targets` report no warning for the function.
