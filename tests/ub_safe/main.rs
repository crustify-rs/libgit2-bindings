//! UB tests that target the safe public API. Each tested TU gets a module, homed at
//! its path below the C root, in `tests/ub_safe/<TU path>.rs` (`src/libgit2/commit.rs` for
//! `src/libgit2/commit.c`); the tests of a TU's `sys/` header are its `sys` module.
//!
//! `unsafe` is confined to [`fixture`], which stands in for safe constructors the
//! crate does not have yet; the tests themselves deny it. libgit2's state is
//! process-global, so tests needing it untouched rerun alone in a child process.
//!
//! Miri cannot call libgit2, so every test that allocates through it is ignored
//! there; the Rust-only paths (empty buffers, rejected sizes) still run.
#![deny(unsafe_code)]

mod fixture;
mod src;

/// Illegal safe paths that must fail to compile, one file each under `compile_fail/`.
#[test]
#[cfg_attr(miri, ignore = "trybuild runs cargo")]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ub_safe/compile_fail/*.rs");
}
