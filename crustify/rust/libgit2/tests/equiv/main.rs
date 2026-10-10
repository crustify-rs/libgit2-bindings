//! Functional equivalence tests: the raw C implementation and the public Rust API run on
//! equivalent, independently owned inputs. Each tested TU gets a module here, homed in
//! `tests/equiv/<tu>.rs`.

mod buffer;
mod commit;
mod common;
mod config;
mod errors;
mod global;
mod odb;
mod oid;
mod oidarray;
mod refdb;
mod refs;
mod repository;
mod strarray;
mod sys_alloc;
mod sys_errors;
mod sys_odb_backend;
mod sys_path;
mod trace;
mod types;
mod worktree;
