//! Public safe memory-options surface: initialization, replacement, copies and
//! exclusive reborrows. The lifetime/aliasing failures are compile-fail doctests.
#![forbid(unsafe_code)]

use libgit2::libgit2::config::sys::config::BackendMemoryOptions;
use std::ffi::CString;

#[test]
#[cfg_attr(
    miri,
    ignore = "constructor calls libgit2; Rust-only projections tested inline"
)]
fn memory_options_reborrow_and_replace() {
    let label = CString::new("command line").unwrap();
    let origin = CString::new(vec![b'/', 0xff, b'x']).unwrap();
    let mut options = BackendMemoryOptions::new();
    assert_eq!(options.as_ref().version(), 1);
    assert_eq!(options.as_ref().backend_type(), None);
    assert_eq!(options.as_ref().origin_path(), None);
    {
        let mut exclusive = options.as_mut();
        exclusive.as_mut().set_backend_type(Some(&label));
        exclusive.as_mut().set_origin_path(Some(&origin));
        let shared = exclusive.as_ref();
        let copied = shared;
        assert_eq!(shared.backend_type(), Some(label.as_c_str()));
        assert_eq!(copied.origin_path(), Some(origin.as_c_str()));
    }
    let clone = options.clone();
    options.as_mut().set_backend_type(Some(c""));
    options.as_mut().set_origin_path(Some(c""));
    assert_eq!(options.as_ref().backend_type(), Some(c""));
    assert_eq!(options.as_ref().origin_path(), Some(c""));
    options.as_mut().set_backend_type(None);
    options.as_mut().set_origin_path(None);
    options.as_mut().set_version(u32::MAX);
    assert_eq!(options.as_ref().version(), u32::MAX);
    assert_eq!(options.as_ref().backend_type(), None);
    assert_eq!(options.as_ref().origin_path(), None);
    drop(options);
    // The clone keeps the same immutable borrow, not a freed owned pointer.
    assert_eq!(clone.as_ref().backend_type(), Some(label.as_c_str()));
    assert_eq!(clone.as_ref().origin_path(), Some(origin.as_c_str()));
    drop(clone);
    assert_eq!(label.as_bytes(), b"command line");
    assert_eq!(origin.as_bytes(), &[b'/', 0xff, b'x']);
    let defaults = BackendMemoryOptions::default();
    assert_eq!(defaults.as_ref().version(), 1);
    assert_eq!(defaults.as_ref().backend_type(), None);
}
