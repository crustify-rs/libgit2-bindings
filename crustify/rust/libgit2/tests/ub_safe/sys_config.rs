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

mod backend_entries {
    use libgit2::libgit2::config::config::Config;
    use libgit2::libgit2::config::sys::config::BackendEntry;
    use std::ffi::CString;

    #[test]
    #[cfg_attr(miri, ignore = "acquisition and release call libgit2")]
    fn backend_entry_retained_storage_and_release() {
        let dir = std::env::temp_dir().join(format!(
            "crustify-backend-entry-safe-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config");
        std::fs::write(
            &file,
            b"[test]\nvalue = first\nflag\nempty =\n[include]\npath = extra\n",
        )
        .unwrap();
        std::fs::write(dir.join("extra"), b"[test]\nnested = included\n").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let mut config = Config::open(&path).unwrap();
        let first = BackendEntry::from_entry(config.as_ref().entry(c"TEST.value").unwrap());
        let second = BackendEntry::from_entry(config.as_ref().entry(c"test.value").unwrap());
        let flag = BackendEntry::from_entry(config.as_ref().entry(c"test.flag").unwrap());
        let empty = BackendEntry::from_entry(config.as_ref().entry(c"test.empty").unwrap());
        let nested = BackendEntry::from_entry(config.as_ref().entry(c"test.nested").unwrap());
        {
            let borrowed = first.as_ref();
            let copy = borrowed;
            assert_eq!(copy.entry().name(), Some(c"test.value"));
            assert_eq!(borrowed.entry().value(), Some(c"first"));
            assert_eq!(borrowed.entry().origin_path(), Some(path.as_c_str()));
        }
        assert_eq!(flag.as_ref().entry().value(), None);
        assert_eq!(empty.as_ref().entry().value(), Some(c""));
        assert_eq!(nested.as_ref().entry().include_depth(), 1);
        // An entry owner does not borrow its config: refreshing cannot invalidate it.
        let bytes = CString::new([b'x', 0xff]).unwrap();
        config.as_mut().set_string(c"test.value", &bytes).unwrap();
        let changed = BackendEntry::from_entry(config.as_ref().entry(c"test.value").unwrap());
        assert_eq!(first.as_ref().entry().value(), Some(c"first"));
        assert_eq!(changed.as_ref().entry().value(), Some(bytes.as_c_str()));
        let policy = first.as_ref().release_policy();
        let public = BackendEntry::into_entry(first);
        assert_eq!(public.as_ref().value(), Some(c"first"));
        let first = BackendEntry::from_entry(public);
        drop(config);
        drop(bytes);
        // No parent or input string remains: backing storage owns the copies.
        assert_eq!(
            changed.as_ref().entry().value().unwrap().to_bytes(),
            &[b'x', 0xff]
        );
        policy.release(first);
        assert_eq!(second.as_ref().entry().value(), Some(c"first"));
        assert_eq!(nested.as_ref().entry().value(), Some(c"included"));
        // A strategy from a now-released entry recovers the second owner's callback.
        policy.release(second);
        drop((flag, empty, nested, changed));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
