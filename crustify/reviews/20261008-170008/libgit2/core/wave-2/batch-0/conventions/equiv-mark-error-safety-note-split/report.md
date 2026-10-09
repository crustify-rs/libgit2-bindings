# `tests/equiv/trace.rs::config::mark_error`: `// SAFETY:` separated from its `unsafe` block by a rustfmt line break

- item: `git_trace_set` (equivalence suite, `crustify/rust/libgit2/tests/equiv/trace.rs`)
- kind: conventions — the campaign's lexical unsafe scan counted the block as
  undocumented
- affected commit: `943038c1d`, reviewed at `6ccd53a97`
- fixed in: the Rust-tree commit of this review

## Finding

`crustify scan-unsafe` reported one undocumented unsafe site in
`libgit2/tests/equiv/trace.rs` for the batch (the orchestrator's
`campaigns/.../wave-2/batch-0/scan-unsafe.json` shows it too). The site is
`mark_error`'s call to `git_error_set_str`: the `// SAFETY:` comment is present and
`clippy::undocumented_unsafe_blocks` (denied workspace-wide) accepts it, but rustfmt
broke `let rc = unsafe { ... };` over two lines, so the comment sits two lines above
the `unsafe` keyword with `let rc =` in between, which the scan's lexical rule does not
look past. The other site the scan lists, `tests/ub_safe/fixture.rs:368`, predates this
batch (`2906ca7d6`) and is outside it.

## Fix

The class is bound first (`let class = ffi::GIT_ERROR_CALLBACK as c_int;`), so the
call fits one line directly under its comment. No behaviour change; the scan's count
for the file drops to 0.
