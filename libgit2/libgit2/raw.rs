//! Raw-lifetime strategies for `void` and string primitives.
//!
//! Each strategy is homed with the translation unit of the primitive it calls and
//! re-exported here.
//!
//! # `void`
//!
//! libgit2's byte allocator (`git__malloc` / `git__free` over the swappable
//! `git__allocator`) is internal to libgit2.so; `git2/sys/odb_backend.h` exports the
//! only public pair over it, `git_odb_backend_data_alloc` / `git_odb_backend_data_free`.
//!
//! | handle | policy | owner |
//! |---|---|---|
//! | one object | [`OdbDataFree`] | [`OdbDataBox<T>`] |
//! | a counted run | [`OdbDataArrayFree`] | [`OdbDataVec<T>`] |

pub use super::odb::sys::odb_backend::{OdbDataArrayFree, OdbDataBox, OdbDataFree, OdbDataVec};
