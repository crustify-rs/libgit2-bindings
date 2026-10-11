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
    let clone = {
        // The original goes out of scope before the clone is read.
        let mut options = options;
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
        clone
    };
    // The clone keeps the same immutable borrow, not a freed owned pointer.
    assert_eq!(clone.as_ref().backend_type(), Some(label.as_c_str()));
    assert_eq!(clone.as_ref().origin_path(), Some(origin.as_c_str()));
    assert_eq!(label.as_bytes(), b"command line");
    assert_eq!(origin.as_bytes(), &[b'/', 0xff, b'x']);
    let defaults = BackendMemoryOptions::default();
    assert_eq!(defaults.as_ref().version(), 1);
    assert_eq!(defaults.as_ref().backend_type(), None);
}

#[test]
#[cfg_attr(miri, ignore = "constructor calls libgit2")]
fn memory_options_cross_threads() {
    static LABEL: &std::ffi::CStr = c"shared";
    let origin = CString::new("scoped origin").unwrap();
    let origin = origin.as_c_str();
    let mut options = BackendMemoryOptions::new();
    options.as_mut().set_backend_type(Some(LABEL));
    options.as_mut().set_origin_path(Some(origin));
    // Concurrent readers through the owner and through copied shared handles.
    std::thread::scope(|s| {
        let shared = options.as_ref();
        for _ in 0..4 {
            s.spawn(move || {
                assert_eq!(shared.backend_type(), Some(LABEL));
                assert_eq!(shared.origin_path(), Some(origin));
            });
        }
        let owner = &options;
        s.spawn(move || assert_eq!(owner.as_ref().version(), 1));
    });
    // An exclusive handle writes on another thread; the owner reads it afterwards.
    std::thread::scope(|s| {
        let mut exclusive = options.as_mut();
        s.spawn(move || exclusive.set_origin_path(None));
    });
    assert_eq!(options.as_ref().origin_path(), None);
    // Options holding only 'static borrows move to a detached thread.
    let mut detached = BackendMemoryOptions::new();
    detached.as_mut().set_backend_type(Some(LABEL));
    let observed = std::thread::spawn(move || {
        let r = detached.as_ref();
        (
            r.version(),
            r.backend_type().map(ToOwned::to_owned),
            r.origin_path().is_none(),
        )
    })
    .join()
    .unwrap();
    assert_eq!(observed, (1, Some(LABEL.to_owned()), true));
}
