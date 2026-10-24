//! Safe Rust wrappers over libgit2's public API.
//!
//! The tree mirrors `crustify/artifacts/modules.json`: one module per link unit, and
//! within it one module per translation unit, at the TU's path below the C root
//! (`libgit2::src::libgit2::repository` for `src/libgit2/repository.c`), holding the
//! items of the TU and of the headers bundled with it.

/// Raw bindings; see the `libgit2-sys` crate.
pub use libgit2_sys as ffi;

#[cfg(feature = "libgit2")]
pub mod libgit2;
