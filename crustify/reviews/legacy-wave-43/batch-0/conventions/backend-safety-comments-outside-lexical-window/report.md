# Three backend adoptions kept their SAFETY comment out of reach of the `unsafe` line

- **item**: `git_config_backend` (type): `Backend::from_string`, `Backend::from_values`,
  `BackendMut::snapshot`, `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `e797d4624` (landed by `beeda6a07`)
- **class**: conventions

## Finding

Each adoption put its `// SAFETY:` comment above a wrapping expression, with the
`unsafe` block one or two lines further down:

```rust
// SAFETY: success returned one complete memory allocation ...
Ok(UnopenedBackend(
    unsafe { BackendBox::from_c(out) }.expect("backend constructor returned NULL"),
))
```

Clippy accepts this. `crustify scan-unsafe`'s `undocumented_unsafe` counter does not:
it credits a comment only when nothing but comments, blank lines or attributes sits
between it and the `unsafe` line. It counted 5 undocumented sites in this file: these
three, the mock fixture's adoption in the tests, and the `compile_fail` doctest on
`IteratorMut::set_backend`.

## Fix

Each adoption now binds the `unsafe` result directly under its comment
(`let backend = unsafe { BackendBox::from_c(out) };`) and wraps it on the next line.
The test fixture does the same. The doctest's explanatory comment is now a `SAFETY`
comment. The new missing-slot code is laid out to stay within the window as well
(`rustfmt` would otherwise split `let x =\n unsafe {`). The file's undocumented count
went from 5 to 0, and every comment kept its wording.
