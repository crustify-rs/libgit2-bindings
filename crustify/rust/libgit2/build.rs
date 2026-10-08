//! Embeds the rpath of the libgit2 build that `libgit2-sys` links, so this package's
//! tests and examples load it: `rustc-link-arg` from a dependency's build script does
//! not reach dependents, and cargo puts only search paths inside the target directory
//! on the loader path.

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_GIT2_BUILD_DIR");
    let dir = std::env::var("DEP_GIT2_BUILD_DIR").expect("libgit2-sys exports build_dir");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
}
