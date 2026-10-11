//! Functional equivalence tests: the raw C implementation and the public Rust API run on
//! equivalent, independently owned inputs. Each tested TU gets a module here, homed in
//! `tests/equiv/<tu>.rs`.

mod annotated_commit;
mod blob;
mod buffer;
mod checkout;
mod commit;
mod common;
mod config;
mod errors;
mod filter;
mod global;
mod index;
mod mailmap;
mod message;
mod object;
mod odb;
mod oid;
mod oidarray;
mod refdb;
mod refs;
mod repository;
mod signature;
mod strarray;
mod sys_alloc;
mod sys_commit;
mod sys_errors;
mod sys_odb_backend;
mod sys_path;
mod sys_repository;
mod tag;
mod trace;
mod tree;
mod types;
mod worktree;
