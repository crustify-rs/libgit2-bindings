//! `ConfigMut::lock` drops the code `git_config_lock` returns: C tells a read-only
//! config (`GIT_EREADONLY`) from a lock another writer holds (`GIT_ELOCKED`) by its
//! return value, the safe wrapper reports both as a bare `ErrorBuf`.
//!
//! The reproducer runs the same three steps on two independently owned configs, one
//! through raw C and one through the safe API: lock an empty config, lock a file
//! config whose `.lock` file another writer holds, then lock it once that writer is
//! gone. After each step it compares the return code, the error class and the error
//! message (paths replaced). The Rust code is read from the error's public `Debug`
//! output (`code: <ErrorCode>`), the only way a caller can observe one, so the
//! reproducer builds both before and after the fix. Run from this directory:
//!
//! ```sh
//! LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run
//! ```
//!
//! On the affected commit it prints every step and exits 101: the Rust error carries
//! no code for the first two. It exits 0 once `lock` reports libgit2's code.

use std::ffi::{CStr, CString, c_int};
use std::fmt::Debug;
use std::path::PathBuf;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::config::config::Config;
use libgit2::libgit2::util::errors::{ErrorCode, last};

/// One step's outcome: code (`None` on success), class and message.
type Observation = (Option<String>, c_int, String);

fn scratch(side: &str) -> (PathBuf, CString) {
    let path = std::env::temp_dir().join(format!(
        "crustify-repro-config-lock-code-{}-{side}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("lock"));
    let c = CString::new(path.to_str().unwrap()).unwrap();
    (path, c)
}

/// The code a Rust error reports through its public `Debug` output.
fn rust_code(error: &impl Debug) -> Option<String> {
    let debug = format!("{error:?}");
    let start = debug.find("code: ")? + "code: ".len();
    let rest = &debug[start..];
    Some(rest[..rest.find([',', ' ', '}']).unwrap_or(rest.len())].to_owned())
}

fn c_code(rc: c_int) -> Option<String> {
    (rc < 0).then(|| match ErrorCode::from_raw(rc) {
        Some(code) => format!("{code:?}"),
        None => rc.to_string(),
    })
}

fn add_file(config: *mut ffi::git_config, path: &CStr) {
    // SAFETY: a live config with no other user, a NUL-terminated path; no repository.
    let rc = unsafe {
        ffi::git_config_add_file_ondisk(
            config,
            path.as_ptr(),
            ffi::GIT_CONFIG_LEVEL_LOCAL,
            ptr::null(),
            0,
        )
    };
    assert_eq!(rc, 0);
    // SAFETY: as above; the setter copies the name.
    assert_eq!(
        unsafe { ffi::git_config_set_int32(config, c"a.b".as_ptr(), 1) },
        0
    );
}

fn main() {
    let (c_path, c_cpath) = scratch("c");
    let (r_path, r_cpath) = scratch("rust");

    let mut rust = Config::new().unwrap();
    let mut raw = ptr::null_mut();
    // SAFETY: the runtime is pinned by `Config::new`; a writable slot.
    assert_eq!(unsafe { ffi::git_config_new(&mut raw) }, 0);

    let c_step = |raw: *mut ffi::git_config| -> Observation {
        let mut tx = ptr::null_mut();
        // SAFETY: a live config and a writable slot; a transaction is freed at once.
        let rc = unsafe { ffi::git_config_lock(&mut tx, raw) };
        if rc == 0 {
            // SAFETY: the one transaction just handed out, released once.
            unsafe { ffi::git_transaction_free(tx) };
            return (None, 0, String::new());
        }
        let error = last();
        let message = error
            .message()
            .to_string_lossy()
            .replace(c_cpath.to_str().unwrap(), "<path>");
        (c_code(rc), error.raw_class(), message)
    };
    let rust_step =
        |rust: &mut libgit2::libgit2::config::config::ConfigBox<'static>| -> Observation {
            match rust.as_mut().lock() {
                Ok(tx) => {
                    drop(tx);
                    (None, 0, String::new())
                }
                Err(error) => {
                    let message = error
                        .message()
                        .to_string_lossy()
                        .replace(r_cpath.to_str().unwrap(), "<path>");
                    (rust_code(&error), error.raw_class(), message)
                }
            }
        };

    let mut failed = false;
    let mut compare = |step: &str, c: Observation, r: Observation| {
        println!("{step}:\n  C:    {c:?}\n  Rust: {r:?}");
        if c != r {
            failed = true;
        }
    };

    compare("empty config", c_step(raw), rust_step(&mut rust));

    add_file(raw, &c_cpath);
    add_file(rust.as_c_ptr(), &r_cpath);
    std::fs::write(c_path.with_extension("lock"), "held").unwrap();
    std::fs::write(r_path.with_extension("lock"), "held").unwrap();
    compare(
        "lock held by another writer",
        c_step(raw),
        rust_step(&mut rust),
    );

    std::fs::remove_file(c_path.with_extension("lock")).unwrap();
    std::fs::remove_file(r_path.with_extension("lock")).unwrap();
    compare("lock free", c_step(raw), rust_step(&mut rust));

    drop(rust);
    // SAFETY: the raw config, released once after its last transaction.
    unsafe { ffi::git_config_free(raw) };
    let _ = std::fs::remove_file(c_path);
    let _ = std::fs::remove_file(r_path);

    if failed {
        eprintln!("the safe wrapper's error lost the code git_config_lock returned");
        std::process::exit(101);
    }
}
