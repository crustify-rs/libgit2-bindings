//! Adopts the `ptr` of a `git_buf` libgit2 filled with nothing into an `OdbDataStr`,
//! the way the module docs on the affected commit invite ("the strings
//! `git_buf_dispose` release[s]"): `GIT_OPT_GET_TEMPLATE_PATH` after
//! `GIT_OPT_SET_TEMPLATE_PATH("")` returns `{ git_str__initstr, 0, 0 }`, a buffer that
//! owns no allocation, which `git_buf_dispose` leaves alone (`buf.c:64`).
//!
//! Run under ASan (`CRUSTIFY_BUILD_VARIANT=asan-ubsan RUSTFLAGS=-Zsanitizer=address
//! cargo +nightly run --target x86_64-unknown-linux-gnu`): the drop frees libgit2's
//! static empty string, an `attempting free on address which was not malloc()-ed`.

use std::ffi::{c_char, c_int};
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::raw::OdbDataStr;

fn main() {
    // SAFETY: no arguments; the reference dies with the process.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
    // SAFETY: `GIT_OPT_SET_TEMPLATE_PATH` takes a `const char *`, which it copies.
    let rc = unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_SET_TEMPLATE_PATH as c_int, c"".as_ptr()) };
    assert_eq!(rc, 0);

    let mut out = ffi::git_buf { ptr: ptr::null_mut(), reserved: 0, size: 0 };
    // SAFETY: `GIT_OPT_GET_TEMPLATE_PATH` takes a zeroed `git_buf *`, which it fills.
    let rc = unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_GET_TEMPLATE_PATH as c_int, &raw mut out) };
    assert_eq!(rc, 0);
    assert!(!out.ptr.is_null(), "libgit2 points an empty buffer at its static empty string");
    assert_eq!((out.reserved, out.size), (0, 0), "the buffer owns no allocation");

    // The docs say libgit2 releases `git_buf` contents with the free the policy calls,
    // so the pointer looks adoptable; `from_raw`'s generic contract is violated, but
    // nothing in the workset says how to tell.
    // SAFETY: NOT upheld — `out.ptr` is a static, not an allocation; this is the defect.
    let s = unsafe { OdbDataStr::from_raw(out.ptr.cast::<c_char>()) }.unwrap();
    assert!(s.is_empty());
    drop(s); // git_odb_backend_data_free(git_str__initstr): ASan bad-free
    println!("no sanitizer report: the static empty string was freed silently");
    std::process::exit(1);
}
