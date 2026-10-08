//! Raw bindings to libgit2's public API (`include/git2.h` and `include/git2/sys/*.h`,
//! built with `GIT_DEPRECATE_HARD`).
//!
//! Only the items named by the allowlists in `bindgen/` are generated. Callable macro
//! shims from `bindgen/shims.c` are bound as `crustify_<NAME>`.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(dead_code, unused_imports, clippy::all)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
