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
//!
//! # string
//!
//! libgit2 releases the strings it hands out (`git_strarray` elements, `git_buf`
//! contents, `git_signature` names) with `git__free` and makes them with
//! `git__strdup`, neither of which is exported; the same pair settles them.
//!
//! | handle | policy | owner |
//! |---|---|---|
//! | one string | [`OdbDataStrFree`] | [`OdbDataStr`] |
//! | a counted run of strings | [`OdbDataStrFree`] in [`OdbDataArrayFree`] storage | [`OdbDataStrVec`] |

pub use super::odb::sys::odb_backend::{
    OdbDataArrayFree, OdbDataBox, OdbDataFree, OdbDataStr, OdbDataStrFree, OdbDataStrVec,
    OdbDataVec,
};
