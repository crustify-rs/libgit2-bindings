# `ub_safe/sys_config.rs` module doc describes only the memory options

- **item**: `git_config_iterator` (type), `crustify/rust/libgit2/tests/ub_safe/sys_config.rs`
- **affected commit**: `0de2b59b4` (libgit2/config wave 3 batch 0)
- **class**: misc (documentation)

## Finding

The suite's `//!` header still read "Public safe memory-options surface: ...". It dated
from wave 0. Wave 2 added the `backend_entries` module and this batch added `iterators`,
so the header described a third of the file.

## Fix

The header now lists the three surfaces the suite covers (memory options, backend
entries, iterators) and keeps the note that lifetime and aliasing failures are
compile-fail doctests.
