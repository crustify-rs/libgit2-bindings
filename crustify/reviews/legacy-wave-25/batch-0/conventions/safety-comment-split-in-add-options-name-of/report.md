# A `// SAFETY:` comment separated from its `unsafe` block by a statement

- **item**: the `add_options` equivalence harness `Worktree::add`'s tests use,
  `crustify/rust/libgit2/tests/equiv/worktree.rs`: `name_of`
- **affected commit**: `7a9191d6d` (libgit2/repository wave 1 batch 1, the `AddOptions`
  wrapper); the batch under review (`9468d5210`) extended the harness; reviewed at
  `16d2fcaae`
- **class**: coding convention (every unsafe block carries its `// SAFETY:` comment
  immediately above), `crustify scan-unsafe` counted the site as undocumented

## Finding

```rust
        // SAFETY: NULL or a live reference; its name lives as long as it and is copied.
        (!reference.is_null())
            .then(|| unsafe { CStr::from_ptr(ffi::git_reference_name(reference)) }.to_owned())
```

The comment sits above the `(!reference.is_null())` line, not the `unsafe {` one:
`crustify scan-unsafe` (whose undocumented count is lexical) lists it as the one
undocumented site of `tests/equiv/worktree.rs`, and `clippy::undocumented_unsafe_blocks`
only passes because it credits the enclosing statement.

## Fix

The NULL case returns early and the `unsafe` block follows its comment directly. The
other undocumented site the scan lists in a file of this batch
(`tests/ub_safe/fixture.rs:363`, `Upper::write`) is a core batch's (`2906ca7d6`) and
is left alone.
