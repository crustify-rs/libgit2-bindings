//! UB tests that target the unsafe public API. Each tested TU gets a module, homed at
//! its path below the C root, in `tests/ub_unsafe/<TU path>.rs` (`src/libgit2/commit.rs` for
//! `src/libgit2/commit.c`); the tests of a TU's `sys/` header are its `sys` module.
//!
//! Miri cannot call libgit2, so every test that allocates through it is ignored
//! there; the Rust-only paths still run.

mod src;
