//! Functional equivalence tests: the raw C implementation and the public Rust API run on
//! equivalent, independently owned inputs. Each tested TU gets a module, homed at
//! its path below the C root, in `tests/equiv/<TU path>.rs` (`src/libgit2/commit.rs` for
//! `src/libgit2/commit.c`); the tests of a TU's `sys/` header are its `sys` module.

mod src;
