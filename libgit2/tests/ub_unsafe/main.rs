//! UB tests that target the unsafe public API. Each tested TU gets a module here,
//! homed in `tests/ub_unsafe/<tu>.rs`.
//!
//! Miri cannot call libgit2, so every test that allocates through it is ignored
//! there; the Rust-only paths still run.

mod buffer;
mod common;
mod sys_alloc;
mod sys_odb_backend;
