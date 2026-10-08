//! Generates the allowlisted libgit2 bindings, compiles the macro shims and links the
//! crustify prebuild selected by `CRUSTIFY_BUILD_VARIANT`.
//!
//! - `CRUSTIFY_BUILD_VARIANT`: a `variants` key of `crustify/build.json` (`plain`,
//!   `coverage`, `asan-ubsan`, `tsan`, `bsan`); default `plain`.
//! - `CRUSTIFY_LIBGIT2_BUILD_DIR`: an explicit libgit2 build directory, for a private
//!   build; overrides the variant's directory.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn allowlist(dir: &Path, kind: &str) -> Vec<String> {
    let path = dir.join(format!("{kind}.txt"));
    println!("cargo:rerun-if-changed={}", path.display());
    fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest.join("../../..").canonicalize().unwrap();
    let include = repo.join("include");
    let bindgen_dir = manifest.join("bindgen");

    println!("cargo:rerun-if-env-changed=CRUSTIFY_BUILD_VARIANT");
    println!("cargo:rerun-if-env-changed=CRUSTIFY_LIBGIT2_BUILD_DIR");
    let variant = env::var("CRUSTIFY_BUILD_VARIANT").unwrap_or_else(|_| "plain".into());
    let build_dir = match env::var_os("CRUSTIFY_LIBGIT2_BUILD_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => repo.join("crustify/builds").join(&variant),
    };
    let lib = build_dir.join("libgit2.so");
    assert!(
        lib.exists(),
        "{} is missing: build the `{variant}` variant from crustify/build.json first",
        lib.display()
    );
    println!("cargo:rerun-if-changed={}", lib.display());
    println!("cargo:rustc-link-search=native={}", build_dir.display());
    println!("cargo:rustc-link-lib=dylib=git2");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", build_dir.display());
    println!("cargo:build_dir={}", build_dir.display());

    for f in ["wrapper.h", "shims.h", "shims.c"] {
        println!("cargo:rerun-if-changed={}", bindgen_dir.join(f).display());
    }

    // Shims are compiled with the instrumentation of the linked variant.
    let mut cc = cc::Build::new();
    cc.compiler("clang")
        .file(bindgen_dir.join("shims.c"))
        .include(&include)
        .include(&bindgen_dir)
        .define("GIT_DEPRECATE_HARD", None)
        .warnings(true);
    match variant.as_str() {
        "asan-ubsan" => {
            cc.flag("-fsanitize=address,undefined").flag("-fno-omit-frame-pointer");
        }
        "tsan" => {
            cc.flag("-fsanitize=thread");
        }
        "coverage" => {
            cc.flag("--coverage");
        }
        _ => {}
    }
    cc.compile("crustify_libgit2_shims");

    let functions = allowlist(&bindgen_dir, "functions");
    let types = allowlist(&bindgen_dir, "types");
    let vars = allowlist(&bindgen_dir, "vars");

    let mut builder = bindgen::Builder::default()
        .header(bindgen_dir.join("wrapper.h").to_string_lossy())
        .clang_arg(format!("-I{}", include.display()))
        .clang_arg(format!("-I{}", bindgen_dir.display()))
        .clang_arg("-DGIT_DEPRECATE_HARD")
        .allowlist_recursively(true)
        .derive_default(true)
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false)
        .layout_tests(true)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
    // An empty allowlist must generate nothing rather than everything.
    if functions.is_empty() && types.is_empty() && vars.is_empty() {
        builder = builder.allowlist_item("^$");
    }
    for f in &functions {
        builder = builder.allowlist_function(f);
    }
    for t in &types {
        builder = builder.allowlist_type(t);
    }
    for v in &vars {
        builder = builder.allowlist_var(v);
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    builder
        .generate()
        .expect("bindgen failed")
        .write_to_file(out.join("bindings.rs"))
        .expect("cannot write bindings");
}
