//! `ConfigRef::entry` and `ConfigRef::get_mapped` change the thread's previous error
//! when it is of class `GIT_ERROR_OS`; `git_config_get_entry` and
//! `git_config_get_mapped` leave it alone.
//!
//! Both wrappers first reread every backend through a multivar iterator, which spends
//! the failure of a backend below the one holding the key. C never reads that backend,
//! so they put the thread's previous error back with `set(class, message)`. `set`
//! calls `git_error_set(class, "%s", message)`, and for `GIT_ERROR_OS` libgit2 appends
//! `": "` and the description of the thread's `errno`: the restored message is not the
//! one that was there.
//!
//! Build and run against the `plain` variant, from this directory:
//!
//! ```sh
//! LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run
//! ```
//!
//! Panics (exit 101) on the affected commit; exits 0 once the wrappers restore the
//! message verbatim. Regression: `tests/equiv/config.rs`,
//! `mapping_callbacks::spent_backend_failures_keep_any_previous_error`.

use std::ffi::{CStr, CString, c_int};
use std::path::PathBuf;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::config::config::{Config, ConfigLevel, ConfigMap, MapMatch};
use libgit2::libgit2::util::errors::{ErrorClass, last};
use libgit2::libgit2::util::sys::errors::set_str;

const PREVIOUS: &CStr = c"failed to stat '/nowhere': No such file or directory";

fn scratch(name: &str) -> CString {
    let path: PathBuf = std::env::temp_dir().join(format!(
        "crustify-repro-os-restore-{}-{name}",
        std::process::id()
    ));
    CString::new(path.to_str().unwrap()).unwrap()
}

fn write(path: &CStr, content: &[u8]) {
    std::fs::write(path.to_str().unwrap(), content).unwrap();
}

/// Breaks a lower backend's file after it was loaded; each call changes its size,
/// so the next reread parses it again.
fn break_file(path: &CStr, round: usize) {
    write(
        path,
        format!("[broken\n{}\n", "x".repeat(round + 8)).as_bytes(),
    );
}

/// The call's result, then the thread's last error.
type Observed = (Result<String, c_int>, c_int, String);

fn observe(result: Result<String, c_int>) -> Observed {
    let error = last();
    (
        result,
        error.raw_class(),
        error.message().to_string_lossy().into_owned(),
    )
}

fn main() {
    let (rust_lower, rust_upper) = (scratch("rust-lower"), scratch("rust-upper"));
    let (c_lower, c_upper) = (scratch("c-lower"), scratch("c-upper"));
    for (lower, upper) in [(&rust_lower, &rust_upper), (&c_lower, &c_upper)] {
        write(lower, b"[a]\n other = old\n");
        write(upper, b"[a]\n b = input\n");
    }
    let mut config = Config::open(&rust_lower).unwrap();
    config
        .as_mut()
        .add_file(&rust_upper, ConfigLevel::APP, None, false)
        .unwrap();
    let mut raw = ptr::null_mut();
    // SAFETY: writable slot and live NUL path; success hands out one owned config.
    assert_eq!(
        unsafe { ffi::git_config_open_ondisk(&mut raw, c_lower.as_ptr()) },
        0
    );
    // SAFETY: live exclusive config, live NUL path, a free priority, no repository.
    let rc = unsafe {
        ffi::git_config_add_file_ondisk(
            raw,
            c_upper.as_ptr(),
            ffi::GIT_CONFIG_LEVEL_APP,
            ptr::null(),
            0,
        )
    };
    assert_eq!(rc, 0);
    let maps = [ConfigMap::new(MapMatch::String(c"input"), 42)];
    let raw_maps = [ffi::git_configmap {
        type_: ffi::GIT_CONFIGMAP_STRING,
        str_match: c"input".as_ptr(),
        map_value: 42,
    }];

    let mut diverged = false;
    for (round, class) in [ErrorClass::Odb, ErrorClass::Os].into_iter().enumerate() {
        break_file(&rust_lower, 2 * round);
        break_file(&c_lower, 2 * round);
        set_str(class, PREVIOUS).unwrap();
        let rust = observe(
            config
                .as_ref()
                .entry(c"a.b")
                .map(|e| e.as_ref().value().unwrap().to_string_lossy().into_owned())
                .map_err(|e| e.raw_code()),
        );
        set_str(class, PREVIOUS).unwrap();
        let mut entry = ptr::null_mut();
        // SAFETY: live config, NUL key, writable slot. The key's backend is the
        // highest, so C never reaches the broken one below it.
        let rc = unsafe { ffi::git_config_get_entry(&mut entry, raw, c"a.b".as_ptr()) };
        let c_value = if rc < 0 {
            Err(rc)
        } else {
            // SAFETY: success acquired a live entry whose value is a NUL string.
            Ok(unsafe { CStr::from_ptr((*entry).value) }
                .to_string_lossy()
                .into_owned())
        };
        let c = observe(c_value);
        // SAFETY: releases the one acquired reference (NULL is a no-op).
        unsafe { ffi::git_config_entry_free(entry) };
        println!("entry, previous {class:?}:\n  rust {rust:?}\n  c    {c:?}");
        diverged |= rust != c;

        break_file(&rust_lower, 2 * round + 1);
        break_file(&c_lower, 2 * round + 1);
        set_str(class, PREVIOUS).unwrap();
        let rust = observe(
            config
                .as_ref()
                .get_mapped(c"a.b", &maps)
                .map(|v| v.to_string())
                .map_err(|e| e.raw_code()),
        );
        set_str(class, PREVIOUS).unwrap();
        let mut out = 0;
        // SAFETY: live config, NUL key, writable scalar; one record with a live string.
        let rc = unsafe {
            ffi::git_config_get_mapped(&mut out, raw, c"a.b".as_ptr(), raw_maps.as_ptr(), 1)
        };
        let c = observe(if rc < 0 { Err(rc) } else { Ok(out.to_string()) });
        println!("get_mapped, previous {class:?}:\n  rust {rust:?}\n  c    {c:?}");
        diverged |= rust != c;
    }
    drop(config);
    // SAFETY: releases the one owned C config.
    unsafe { ffi::git_config_free(raw) };
    for path in [&rust_lower, &rust_upper, &c_lower, &c_upper] {
        let _ = std::fs::remove_file(path.to_str().unwrap());
    }
    assert!(
        !diverged,
        "the wrappers left a different previous error than C"
    );
}
