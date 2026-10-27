//! Raw bindings to libgit2's public API (`include/git2.h` and `include/git2/sys/*.h`,
//! built with `GIT_DEPRECATE_HARD`).
//!
//! Only the items `bindgen/allowlist.txt` names are generated, and shipped as
//! `src/bindings.rs`, which the `bindgen` feature regenerates against the headers found.
//! Callable macro shims from `bindgen/shims.c` are bound as `crustify_<NAME>`.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(dead_code, unused_imports, clippy::all)]

#[cfg(not(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu")))]
compile_error!(
    "libgit2-sys supports x86_64-unknown-linux-gnu only, the target its bindings are generated for"
);

/// The version of libgit2 the bindings were generated from, as its `pkg-config` file records it.
pub const C_VERSION: &str = env!("C_VERSION");

#[cfg(not(crustify_regenerated))]
include!("bindings.rs");
#[cfg(crustify_regenerated)]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
