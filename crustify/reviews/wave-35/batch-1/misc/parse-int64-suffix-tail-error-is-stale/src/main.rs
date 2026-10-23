//! `parse_int64` returns an error that does not describe its failure when the value
//! has text after a size suffix (`1kb`).
//!
//! `git_config_parse_int64` returns -1 for characters after a `k`/`m`/`g` suffix
//! without calling `git_error_set` (`src/libgit2/config.c`, the bare `return -1`
//! after `num_end[1] != '\0'`), and the wrapper copies whatever the thread's last
//! error happens to be: an earlier, unrelated failure, or "no error".
//!
//! Build and run against the `plain` variant, from this directory:
//!
//! ```sh
//! LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run
//! ```
//!
//! Panics (exit 101) on the affected commit with the stale error. The fix renames
//! the wrapper `parse_i64`, so this package no longer builds on the fixed tree; its
//! regression is `config.rs::unit_tests::parse_i64_suffix_tail_reports_its_own_error`.
#![forbid(unsafe_code)]

use libgit2::libgit2::config::config::parse_int64;
use libgit2::libgit2::util::errors::ErrorClass;
use libgit2::libgit2::util::sys::errors::{clear, set_str};

fn main() {
    // An unrelated earlier failure on this thread.
    set_str(ErrorClass::Odb, c"object not found - an earlier, unrelated lookup").unwrap();
    let error = parse_int64(Some(c"1kb")).unwrap_err();
    println!("after an unrelated error: {:?} {error}", error.class());
    let stale = error.class() != Some(ErrorClass::Config)
        || !error.message().to_bytes().windows(3).any(|w| w == b"1kb");

    // No earlier failure.
    clear();
    let error = parse_int64(Some(c"1kb")).unwrap_err();
    println!("after no error:           {:?} {error}", error.class());
    let silent = error.class() != Some(ErrorClass::Config)
        || !error.message().to_bytes().windows(3).any(|w| w == b"1kb");

    assert!(
        !stale && !silent,
        "parse_int64(\"1kb\") failed with an error that does not describe it"
    );
}
