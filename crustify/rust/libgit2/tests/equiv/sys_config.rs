//! Independent raw C initialization and public Rust memory-options operations.
use libgit2::ffi;
use libgit2::libgit2::config::sys::config::BackendMemoryOptions;
use std::ffi::{CStr, CString};
use std::mem::MaybeUninit;

type Observation = (u32, Option<Vec<u8>>, Option<Vec<u8>>);

fn observe_c(raw: &ffi::git_config_backend_memory_options) -> Observation {
    let string = |ptr: *const std::ffi::c_char| {
        if ptr.is_null() {
            None
        } else {
            // SAFETY: test only stores NULL or immutable NUL strings still live here.
            Some(unsafe { CStr::from_ptr(ptr) }.to_bytes().to_vec())
        }
    };
    (
        raw.version,
        string(raw.backend_type),
        string(raw.origin_path),
    )
}

fn observe_rust(options: &BackendMemoryOptions<'_>) -> Observation {
    let r = options.as_ref();
    (
        r.version(),
        r.backend_type().map(|s| s.to_bytes().to_vec()),
        r.origin_path().map(|s| s.to_bytes().to_vec()),
    )
}

#[test]
#[cfg_attr(miri, ignore = "calls C init")]
fn memory_options_init_and_mutation_match_c() {
    let mut raw = MaybeUninit::uninit();
    // SAFETY: writable storage for the supported version's full template.
    let rc = unsafe {
        ffi::git_config_backend_memory_options_init(
            raw.as_mut_ptr(),
            ffi::GIT_CONFIG_BACKEND_MEMORY_OPTIONS_VERSION,
        )
    };
    assert_eq!(rc, 0);
    // SAFETY: successful initialization filled all fields.
    let mut raw = unsafe { raw.assume_init() };
    let mut rust = BackendMemoryOptions::new();
    assert_eq!(observe_c(&raw), observe_rust(&rust));
    for (label, origin) in [
        (None, None),
        (Some(c"label"), None),
        (None, Some(c"path")),
        (Some(c""), Some(c"")),
    ] {
        raw.backend_type = label.map_or(std::ptr::null(), CStr::as_ptr);
        raw.origin_path = origin.map_or(std::ptr::null(), CStr::as_ptr);
        rust.as_mut().set_backend_type(label);
        rust.as_mut().set_origin_path(origin);
        assert_eq!(observe_c(&raw), observe_rust(&rust));
    }
    let c_bytes = CString::new(vec![0xff, b'x']).unwrap();
    let rust_bytes = CString::new(vec![0xff, b'x']).unwrap();
    raw.origin_path = c_bytes.as_ptr();
    rust.as_mut().set_origin_path(Some(&rust_bytes));
    assert_eq!(observe_c(&raw), observe_rust(&rust));
    for version in [0, 1, 2, u32::MAX] {
        raw.version = version;
        rust.as_mut().set_version(version);
        assert_eq!(observe_c(&raw), observe_rust(&rust));
    }
    let c_clone = raw;
    let rust_clone = rust.clone();
    raw.backend_type = std::ptr::null();
    rust.as_mut().set_backend_type(None);
    assert_eq!(observe_c(&raw), observe_rust(&rust));
    assert_eq!(observe_c(&c_clone), observe_rust(&rust_clone));
}
