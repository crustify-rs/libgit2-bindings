//! Functional equivalence tests: the raw C implementation and the public Rust API run on
//! equivalent, independently owned inputs. Each tested TU gets a module here, homed in
//! `tests/equiv/<tu>.rs`.

mod common;
mod errors;
mod oid;
mod oidarray;
mod strarray;
mod sys_alloc;
mod sys_errors;
mod sys_odb_backend;
mod trace;
mod types;
