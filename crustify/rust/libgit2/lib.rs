//! Safe Rust wrappers over libgit2's public API.
//!
//! The tree mirrors `crustify/subsystems.json`: one module per link unit, one
//! sub-module per subsystem, and one module per public header within it.

/// Raw bindings; see the `libgit2-sys` crate.
pub use libgit2_sys as ffi;

#[cfg(feature = "libgit2")]
pub mod libgit2;
