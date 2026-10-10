//! The `libgit2` link unit (`libgit2.so`).
//!
//! Subsystem modules are declared here as their first batches are scheduled.

pub mod config;
pub mod core;
pub mod index;
pub mod object;
pub mod odb;
pub mod oid;
pub mod raw;
pub mod refs;
pub mod repository;
pub mod util;
pub mod workdir;
