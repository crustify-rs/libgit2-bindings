//! Embeds the rpath of the libgit2 build that `libgit2-sys` links, and of the side
//! campaigns' installs it links, so this package's tests and examples load them: `rustc-link-arg`
//! from a dependency's build script does not reach dependents, and cargo puts only search paths
//! inside the target directory on the loader path.

fn main() {
    // `cargo bsan` compiles with `--cfg bsan`; tests use it to skip paths it cannot run.
    println!("cargo:rustc-check-cfg=cfg(bsan)");
    println!("cargo:rerun-if-env-changed=DEP_GIT2_BUILD_DIR");
    // A campaign build of libgit2-sys reports its build; a dependent's links an install.
    if let Ok(dir) = std::env::var("DEP_GIT2_BUILD_DIR") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
    // The side campaigns' installs libgit2 links, for binaries that link one directly.
    println!("cargo:rerun-if-env-changed=DEP_GIT2_DEP_LIB_DIRS");
    if let Ok(dirs) = std::env::var("DEP_GIT2_DEP_LIB_DIRS") {
        for dir in dirs.split(':').filter(|d| !d.is_empty()) {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
        }
    }
    // Under TSan, the instrumented libraries the executable reaches only through
    // libgit2.so (libcrypto's atomics, say) resolve their `__tsan_*` calls against the
    // runtime linked into the executable, which the linker otherwise exports only for
    // the libraries on its command line.
    println!("cargo:rerun-if-env-changed=DEP_GIT2_VARIANT");
    if std::env::var("DEP_GIT2_VARIANT").as_deref() == Ok("tsan") {
        println!("cargo:rustc-link-arg=-Wl,--export-dynamic-symbol=__tsan_*");
    }
}
