//! `delete_entry` and `delete_multivar` fail on a config without a writable backend
//! with an error that does not describe the failure.
//!
//! `git_config_delete_entry` and `git_config_delete_multivar` (`src/libgit2/config.c`)
//! return `GIT_EREADONLY` when `get_writer` finds no writer, without calling
//! `git_error_set`. `git_config_set_string`, `git_config_set_multivar` and
//! `git_config_lock` set "the configuration is read-only" for the same condition. The
//! wrappers copy whatever the thread's last error happens to be into the
//! `ConfigError`: an earlier, unrelated failure, or "no error".
//!
//! Build and run against the `plain` variant, from this directory:
//!
//! ```sh
//! LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run
//! ```
//!
//! Panics (exit 101) on the affected commit; exits 0 once the wrappers report the
//! failure themselves. Regression:
//! `config.rs::unit_tests::readonly_deletes_report_their_own_error`.
#![forbid(unsafe_code)]

use libgit2::libgit2::config::config::{Config, ConfigError};
use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};
use libgit2::libgit2::util::sys::errors::{clear, set_str};

/// Whether `error` is a read-only failure that says so.
fn describes_itself(what: &str, error: &ConfigError) -> bool {
    println!("{what:<44} {:?} {:?} {error}", error.code(), error.class());
    error.code() == Some(ErrorCode::ReadOnly)
        && error.class() == Some(ErrorClass::Config)
        && error.to_string().contains("read-only")
}

fn main() {
    // An empty config has no backend, so no writer; a snapshot has only read-only ones.
    let mut empty = Config::new().unwrap();
    let mut snapshot = empty.as_ref().snapshot().unwrap();
    let mut ok = true;
    for (name, config) in [("empty", &mut empty), ("snapshot", &mut snapshot)] {
        set_str(ErrorClass::Odb, c"object not found - an earlier, unrelated lookup").unwrap();
        let error = config.as_mut().delete_entry(c"a.b").unwrap_err();
        ok &= describes_itself(&format!("{name}: delete_entry after an earlier error:"), &error);

        clear();
        let error = config.as_mut().delete_entry(c"a.b").unwrap_err();
        ok &= describes_itself(&format!("{name}: delete_entry after no error:"), &error);

        set_str(ErrorClass::Odb, c"object not found - an earlier, unrelated lookup").unwrap();
        let error = config.as_mut().delete_multivar(c"a.b", c".*").unwrap_err();
        ok &= describes_itself(&format!("{name}: delete_multivar after an earlier error:"), &error);

        clear();
        let error = config.as_mut().delete_multivar(c"a.b", c".*").unwrap_err();
        ok &= describes_itself(&format!("{name}: delete_multivar after no error:"), &error);
    }
    assert!(ok, "a read-only delete failed with an error that does not describe it");
}
