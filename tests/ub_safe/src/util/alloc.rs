//! `git2/sys/alloc.h`: `Allocator` values, their handles, and the trampolines
//! `Allocator::new` builds, driven by libgit2 itself.
//!
//! Each libgit2 test installs a Rust allocator in a child process of its own (libgit2's
//! allocator is process-global), then runs safe workloads that allocate, resize and
//! free through it; ASan/LSan flag a trampoline that mishandles a block, `TSan` one that
//! races, and the counters show the calls reached Rust.
