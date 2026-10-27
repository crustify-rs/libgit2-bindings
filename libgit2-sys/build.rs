//! Build script of the `-sys` crate, as crustify's `specs/bindgen.md` describes it; every
//! `-sys` crate of the workspace would hold a copy differing only in the constants below.
//!
//! - Shipped, by default: the committed `src/bindings.rs`, against the library found through
//!   `LIBGIT2_DIR` (an install prefix) or `pkg-config`, which must be the version the bindings were
//!   generated from.
//! - Regenerate, under the `bindgen` feature: bindings generated into `OUT_DIR` from the headers of
//!   the library found.
//! - Campaign, when `CRUSTIFY_BUILD_VARIANT` names a variant of `crustify/artifacts/build.json`, or
//!   `CRUSTIFY_BUILD_DIR` a private build: the bindings are regenerated from `c/` and checked
//!   against the committed ones, which `CRUSTIFY_BINDINGS_UPDATE=1` overwrites instead, and the
//!   build is linked with an rpath, with the libraries it links from its side campaigns' installs
//!   of the same variant.

/// The link unit: its library and its `pkg-config` module.
const LIB: &str = "libgit2";
/// The version of the library the committed bindings were generated from.
const C_VERSION: &str = "1.9.0";
/// The preprocessor definitions the bindings and the shims are built with.
const DEFINES: &[&str] = &["GIT_DEPRECATE_HARD"];
/// The out-of-tree libraries' wrappers whose installs a campaign variant links.
const SIDE_CAMPAIGNS: &[&str] = &["openssl-bindings", "pcre2-bindings", "zlib-bindings"];

use std::path::{Path, PathBuf};
use std::{env, fs};

/// The headers and version of the library a build compiles and links against.
struct Library {
    include: Vec<PathBuf>,
    version: String,
    /// The compiler and instrumentation flags of a campaign variant, for the C wrappers.
    cc: Option<(String, Vec<String>)>,
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let input = manifest.join("bindgen");
    for f in [
        "wrapper.h",
        "allowlist.txt",
        "external.txt",
        "shims.h",
        "shims.c",
    ] {
        println!("cargo:rerun-if-changed={}", input.join(f).display());
    }
    for f in ["src/bindings.rs", "src/static_fns.c"] {
        println!("cargo:rerun-if-changed={}", manifest.join(f).display());
    }
    for v in [
        "CRUSTIFY_BUILD_VARIANT",
        "CRUSTIFY_BUILD_DIR",
        "CRUSTIFY_BINDINGS_UPDATE",
    ] {
        println!("cargo:rerun-if-env-changed={v}");
    }
    println!("cargo::rustc-check-cfg=cfg(crustify_regenerated)");
    // Dependents' build scripts read it as `DEP_<LINKS>_BINDGEN_DIR`.
    println!("cargo:bindgen_dir={}", input.display());

    let set = |v: &str| env::var_os(v).is_some_and(|s| !s.is_empty());
    let campaign = set("CRUSTIFY_BUILD_VARIANT") || set("CRUSTIFY_BUILD_DIR");
    let lib = if campaign {
        campaign_library(&manifest)
    } else {
        installed_library()
    };
    let regenerate = cfg!(feature = "bindgen");
    if !regenerate && lib.version != C_VERSION {
        panic!(
            "{LIB} {} found, but the bindings were generated from {C_VERSION}: build {LIB} \
             {C_VERSION}, or enable the `bindgen` feature to regenerate them",
            lib.version
        );
    }

    let mut static_fns = manifest.join("src/static_fns.c");
    if regenerate {
        let out = PathBuf::from(env::var("OUT_DIR").unwrap());
        let (bindings, fns) = generate(&input, &lib.include, &out);
        if campaign {
            check_or_update(&manifest.join("src/bindings.rs"), &bindings, &out);
            check_or_update(&static_fns, fns.as_deref().unwrap_or(""), &out);
            println!("cargo:rustc-env=C_VERSION={C_VERSION}");
        } else {
            fs::write(out.join("bindings.rs"), bindings).unwrap();
            static_fns = out.join("static_fns.c");
            fs::write(&static_fns, fns.unwrap_or_default()).unwrap();
            println!("cargo:rustc-cfg=crustify_regenerated");
            println!("cargo:rustc-env=C_VERSION={}", lib.version);
        }
    } else {
        println!("cargo:rustc-env=C_VERSION={C_VERSION}");
    }
    compile_wrappers(&input, &lib, &static_fns);
}

/// The library of a campaign build: the variant's, or the private build's.
fn campaign_library(manifest: &Path) -> Library {
    let root = manifest.parent().unwrap().to_path_buf();
    let variant = env::var("CRUSTIFY_BUILD_VARIANT").unwrap_or_default();
    let build = match env::var_os("CRUSTIFY_BUILD_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let record = root.join("crustify/artifacts/build.json");
            println!("cargo:rerun-if-changed={}", record.display());
            let spec: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&record).expect("build.json"))
                    .expect("valid build.json");
            let dir = spec["variants"][&variant]["dir"]
                .as_str()
                .unwrap_or_else(|| panic!("no variant {variant:?} in {}", record.display()));
            root.join(dir)
        }
    };
    let so = build.join(format!("{LIB}.so"));
    assert!(so.exists(), "{} is missing: build it first", so.display());
    println!("cargo:rerun-if-changed={}", so.display());
    println!("cargo:rustc-link-search=native={}", build.display());
    println!("cargo:rustc-link-lib=dylib={}", &LIB[3..]);
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", build.display());
    // The libraries the build links from its side campaigns' installs of the same variant
    // go on the search path and the rpath, and to dependents (`DEP_GIT2_DEP_LIB_DIRS`) for
    // theirs, so that a test binary loads the very libraries the build was linked with.
    let mut dep_dirs = Vec::new();
    for wrapper in SIDE_CAMPAIGNS {
        let lib = root
            .join("..")
            .join(wrapper)
            .join("crustify/builds")
            .join(&variant)
            .join("install/lib");
        if lib.is_dir() {
            println!("cargo:rustc-link-search=native={}", lib.display());
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
            dep_dirs.push(lib.display().to_string());
        }
    }
    println!("cargo:dep_lib_dirs={}", dep_dirs.join(":"));
    println!("cargo:variant={variant}");
    println!("cargo:build_dir={}", build.display());
    Library {
        include: vec![root.join("c/include")],
        version: pc_version(&build.join(format!("{LIB}.pc"))),
        cc: Some(variant_cc(&build.join("CMakeCache.txt"))),
    }
}

/// The library as a dependent finds it: under `<LIB>_DIR`, or through `pkg-config`.
fn installed_library() -> Library {
    let dir_var = format!("{}_DIR", LIB.to_uppercase());
    println!("cargo:rerun-if-env-changed={dir_var}");
    if let Some(dir) = env::var_os(&dir_var).map(PathBuf::from) {
        let lib_dir = dir.join("lib");
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-link-lib=dylib={}", &LIB[3..]);
        return Library {
            include: vec![dir.join("include")],
            version: pc_version(&lib_dir.join(format!("pkgconfig/{LIB}.pc"))),
            cc: None,
        };
    }
    let found = pkg_config::Config::new()
        .probe(LIB)
        .unwrap_or_else(|e| panic!("{LIB} not found: set {dir_var} to its install prefix ({e})"));
    Library {
        include: found.include_paths,
        version: found.version,
        cc: None,
    }
}

/// The `Version` a `pkg-config` file records.
fn pc_version(pc: &Path) -> String {
    println!("cargo:rerun-if-changed={}", pc.display());
    fs::read_to_string(pc)
        .unwrap_or_else(|e| panic!("{}: {e}", pc.display()))
        .lines()
        .find_map(|l| l.strip_prefix("Version:"))
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| panic!("{} records no Version", pc.display()))
}

/// The compiler of a campaign variant, and the instrumentation flags of its C flags, as
/// its CMake cache records them.
fn variant_cc(cache: &Path) -> (String, Vec<String>) {
    let text = fs::read_to_string(cache).unwrap_or_else(|e| panic!("{}: {e}", cache.display()));
    let var = |name: &str| {
        text.lines()
            .find_map(|l| {
                l.strip_prefix(name)
                    .and_then(|r| r.split_once('='))
                    .map(|(_, v)| v)
            })
            .unwrap_or_default()
            .to_string()
    };
    let instrumentation = [
        "-fsanitize",
        "-fno-sanitize",
        "-fpass-plugin",
        "-fprofile-arcs",
        "-ftest-coverage",
        "--coverage",
    ];
    let flags = var("CMAKE_C_FLAGS:")
        .split_whitespace()
        .filter(|f| instrumentation.iter().any(|p| f.starts_with(p)))
        .map(String::from)
        .collect();
    (var("CMAKE_C_COMPILER:"), flags)
}

/// Compiles the macro shims and the wrappers of the `static inline` functions.
fn compile_wrappers(input: &Path, lib: &Library, static_fns: &Path) {
    let mut cc = cc::Build::new();
    if let Some((compiler, flags)) = &lib.cc {
        cc.compiler(compiler);
        for f in flags {
            cc.flag(f);
        }
    }
    for d in DEFINES {
        cc.define(d, None);
    }
    cc.file(input.join("shims.c"))
        .includes(&lib.include)
        .include(input)
        .warnings(true);
    if fs::metadata(static_fns).is_ok_and(|m| m.len() > 0) {
        cc.file(static_fns);
    }
    cc.compile(&format!("crustify_{LIB}_wrappers"));
}

/// Fails the build when `path` does not hold `content`, unless asked to write it; the
/// content generated is left in `out` for comparison.
fn check_or_update(path: &Path, content: &str, out: &Path) {
    if fs::read_to_string(path).unwrap_or_default() == content {
        return;
    }
    if env::var_os("CRUSTIFY_BINDINGS_UPDATE").is_some() {
        if content.is_empty() {
            let _ = fs::remove_file(path);
        } else {
            fs::write(path, content).unwrap();
        }
        return;
    }
    let generated = out.join(path.file_name().unwrap());
    fs::write(&generated, content).unwrap();
    panic!(
        "{} does not match its bindgen input, which generates {}: rebuild with \
         CRUSTIFY_BINDINGS_UPDATE=1 and commit it",
        path.display(),
        generated.display()
    );
}

#[cfg(not(feature = "bindgen"))]
fn generate(_: &Path, _: &[PathBuf], _: &Path) -> (String, Option<String>) {
    unreachable!("regeneration needs the `bindgen` feature")
}

/// Generates the bindings, and the C wrappers of the allowlisted `static inline` functions
/// when there are any, from the crate's `bindgen/` input.
#[cfg(feature = "bindgen")]
fn generate(input: &Path, include: &[PathBuf], out: &Path) -> (String, Option<String>) {
    let lines = |name: &str| -> Vec<String> {
        fs::read_to_string(input.join(name))
            .unwrap_or_default()
            .lines()
            .map(|l| l.split('#').next().unwrap().trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };
    let mut b = bindgen::Builder::default()
        .header(input.join("wrapper.h").to_str().unwrap())
        .clang_arg(format!("-I{}", input.display()))
        .clang_args(include.iter().map(|i| format!("-I{}", i.display())))
        .clang_args(DEFINES.iter().map(|d| format!("-D{d}")))
        .allowlist_item("crustify_.*")
        // A formatter of its own, not the toolchain's rustfmt, so that every toolchain
        // generates the same bytes.
        .formatter(bindgen::Formatter::Prettyplease)
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false)
        .derive_default(true)
        .layout_tests(true)
        .generate_comments(true)
        .wrap_static_fns(true)
        .wrap_static_fns_path(out.join("static_fns"))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
    for line in lines("allowlist.txt") {
        let (kind, pat) = line
            .split_once(char::is_whitespace)
            .expect("`<kind> <regex>`");
        let pat = pat.trim();
        b = match kind {
            "function" => b.allowlist_function(pat),
            "type" => b.allowlist_type(pat),
            "var" => b.allowlist_var(pat),
            k => panic!("allowlist.txt: unknown kind {k:?}"),
        };
    }
    for pat in lines("external.txt") {
        b = b.blocklist_type(pat);
    }
    let path = out.join("static_fns.c");
    // bindgen writes the wrappers only when the allowlist reaches a static inline function.
    let _ = fs::remove_file(&path);
    let bindings = b.generate().expect("bindgen").to_string();
    // The wrappers include the header by its absolute path; the build finds it in `bindgen/`.
    let fns = fs::read_to_string(&path).ok().map(|c| {
        c.replace(
            &format!("\"{}\"", input.join("wrapper.h").display()),
            "\"wrapper.h\"",
        )
    });
    (bindings, fns)
}
