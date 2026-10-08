//! UB tests that target the safe public API. Each tested TU gets a module here,
//! homed in `tests/ub_safe/<tu>.rs`.
//!
//! `unsafe` is confined to [`fixture`], which stands in for safe constructors the
//! crate does not have yet; the tests themselves deny it. libgit2's state is
//! process-global, so tests needing it untouched rerun alone in a child process.
//!
//! Miri cannot call libgit2, so every test that allocates through it is ignored
//! there; the Rust-only paths (empty buffers, rejected sizes) still run.
#![deny(unsafe_code)]

mod fixture;
mod sys_errors;
mod sys_odb_backend;

/// Illegal safe paths that must fail to compile, one file each under `compile_fail/`.
#[test]
#[cfg_attr(miri, ignore = "trybuild runs cargo")]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ub_safe/compile_fail/*.rs");
}
