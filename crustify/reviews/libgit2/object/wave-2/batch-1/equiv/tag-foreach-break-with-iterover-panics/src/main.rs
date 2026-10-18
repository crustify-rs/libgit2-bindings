//! `tag::foreach` panics where `git_tag_foreach` returns 0: a closure that breaks with
//! `GIT_ITEROVER` (-31).
//!
//! `git_tag_foreach` walks the tags through `git_reference_foreach_name`, which stops
//! at the callback's first non-zero answer and then maps `GIT_ITEROVER` to 0, the code
//! of a finished walk. `tag::foreach` records the code the closure broke with and
//! asserts (`debug_assert_eq!(rc, code.get())`) that libgit2 returned it, so in a build
//! with debug assertions (`cargo run`'s default profile) the safe call panics after C
//! stopped the walk.
//!
//! The reproducer runs the same walk over `testrepo.git` in C and in Rust, both
//! callbacks breaking with -31 at the first tag, and compares the tags each visited, the
//! thread's error after the walk, and that Rust reports the closure's break (C's 0 is
//! the iterator's end sentinel swallowing the callback's code, a C conflation the
//! wrapper documents rather than copies). Run from this directory:
//!
//! ```sh
//! cargo run
//! ```
//!
//! Exits 101 with "Rust panicked where C returned 0" on the affected commit; prints both
//! outcomes and exits 0 once `foreach` reports `Break(-31)`.

use std::ffi::{CStr, CString, c_int, c_void};
use std::num::NonZero;
use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};

use libgit2::ffi;
use libgit2::libgit2::object::tag::{ForeachCallback, foreach};
use libgit2::libgit2::oid::oid::OidRef;
use libgit2::libgit2::repository::repository::Repository;
use libgit2::libgit2::util::errors::last;

const TESTREPO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../../../../../../tests/resources/testrepo.git"
);

/// Every tag the C callback saw; it breaks with `GIT_ITEROVER` at the first.
unsafe extern "C" fn stop_at_first(
    name: *const std::ffi::c_char,
    _id: *mut ffi::git_oid,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: libgit2 passes the walk's `Vec` and a NUL-terminated name, for the call.
    let seen = unsafe { &mut *payload.cast::<Vec<String>>() };
    // SAFETY: as above.
    seen.push(unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned());
    ffi::GIT_ITEROVER
}

fn main() {
    let path = CString::new(std::fs::canonicalize(TESTREPO).unwrap().to_str().unwrap()).unwrap();

    // C: `git_tag_foreach` on a repository of its own.
    // SAFETY: plain initialisation, never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    let mut c_repo = std::ptr::null_mut();
    // SAFETY: a writable slot and a NUL-terminated path.
    assert_eq!(unsafe { ffi::git_repository_open(&mut c_repo, path.as_ptr()) }, 0);
    let mut c_seen: Vec<String> = Vec::new();
    // SAFETY: a live repository; the callback reads the live `Vec` it is given.
    let c_rc = unsafe {
        ffi::git_tag_foreach(c_repo, Some(stop_at_first), (&raw mut c_seen).cast())
    };
    let c_error = last().message().to_string_lossy().into_owned();
    // SAFETY: the repository opened above, released once.
    unsafe { ffi::git_repository_free(c_repo) };
    // SAFETY: a plain call.
    unsafe { ffi::git_error_clear() };
    println!("C:    rc {c_rc}, saw {c_seen:?}, error {c_error:?}");

    // Rust: the safe API on a repository of its own.
    let repo = Repository::open(&path).unwrap();
    let mut rust_seen: Vec<String> = Vec::new();
    let mut f = |name: &CStr, _: OidRef<'_>| {
        rust_seen.push(name.to_string_lossy().into_owned());
        ControlFlow::Break(NonZero::new(ffi::GIT_ITEROVER).unwrap())
    };
    let mut callback = ForeachCallback::new(&mut f);
    let walked = catch_unwind(AssertUnwindSafe(|| foreach(repo.as_ref(), &mut callback)));
    drop(callback);
    let Ok(walked) = walked else {
        eprintln!("Rust panicked where C returned {c_rc} after visiting {c_seen:?}");
        std::process::exit(101);
    };
    let rust_error = last().message().to_string_lossy().into_owned();
    println!("Rust: {walked:?}, saw {rust_seen:?}, error {rust_error:?}");

    assert_eq!(c_rc, 0, "C's walk returned the callback's code");
    assert_eq!(rust_seen, c_seen, "the walks visited different tags");
    assert_eq!(rust_error, c_error, "the walks left different errors");
    assert_eq!(
        walked.unwrap(),
        ControlFlow::Break(NonZero::new(ffi::GIT_ITEROVER).unwrap()),
        "the closure's break was lost"
    );
}
