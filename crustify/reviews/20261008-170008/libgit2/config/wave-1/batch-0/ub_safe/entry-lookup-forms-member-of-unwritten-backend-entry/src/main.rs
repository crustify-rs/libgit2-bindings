//! `ConfigRef::entry` (`git_config_get_entry`) on a config whose file changed on disk
//! into something libgit2 cannot parse. The lookup reparses the file
//! (`config_file_get` -> `config_file_refresh`), the parse fails, and `get_entry`
//! (`src/libgit2/config.c:811`) evaluates `*out = &be->entry` for the backend entry
//! that the failed `get` never wrote: a member access through an indeterminate pointer.
//! Under UndefinedBehaviorSanitizer the stale slot reads NULL ("member access within
//! null pointer of type 'git_config_backend_entry'") and the process aborts.
//!
//! Safe API only. Run it with the ASan/UBSan prebuild (see `report.md`).
#![forbid(unsafe_code)]

use std::ffi::CString;

use libgit2::libgit2::config::config::Config;

fn main() {
    let path = std::env::temp_dir().join(format!(
        "crustify-entry-unwritten-backend-entry-{}",
        std::process::id()
    ));
    std::fs::write(&path, b"[test]\n key = value\n").unwrap();
    let file = CString::new(path.to_str().unwrap()).unwrap();
    let config = Config::open(&file).unwrap();
    let entry = config.as_ref().entry(c"test.key").unwrap();
    assert_eq!(entry.as_ref().value(), Some(c"value"));

    // A rewrite of another size makes the next lookup reparse the file, which fails.
    std::fs::write(&path, b"[test\n key = no closing bracket\n").unwrap();
    let result = config.as_ref().entry(c"test.key");
    std::fs::remove_file(&path).unwrap();

    let error = result.expect_err("the reparse fails");
    println!("lookup failed, as in libgit2: {error}");
    assert_eq!(entry.as_ref().value(), Some(c"value"));
}
