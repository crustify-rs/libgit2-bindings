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

mod backend_entries {
    use libgit2::ffi;
    use libgit2::libgit2::config::config::{Config, ConfigLevel};
    use libgit2::libgit2::config::sys::config::{BackendEntry, BackendEntryArc};
    use std::ffi::{CStr, CString};
    use std::ptr;

    #[derive(Debug, PartialEq, Eq)]
    struct Record {
        name: Option<Vec<u8>>,
        value: Option<Vec<u8>>,
        backend: Option<Vec<u8>>,
        origin_file: Option<String>,
        level: ffi::git_config_level_t,
        include_depth: u32,
        callback: bool,
    }

    fn raw_record(ptr: *mut ffi::git_config_entry) -> Record {
        let string = |ptr: *const std::ffi::c_char| {
            if ptr.is_null() {
                None
            } else {
                // SAFETY: acquired entry pins its immutable NUL-terminated strings.
                Some(unsafe { CStr::from_ptr(ptr) })
            }
        };
        // SAFETY: get_entry acquired this initialized entry, not yet released.
        let entry = unsafe { ptr.read() };
        // SAFETY: the public entry is the offset-zero field of a backend entry.
        let callback = unsafe {
            ptr.cast::<ffi::git_config_backend_entry>()
                .read()
                .free
                .is_some()
        };
        Record {
            name: string(entry.name).map(|s| s.to_bytes().to_vec()),
            value: string(entry.value).map(|s| s.to_bytes().to_vec()),
            backend: string(entry.backend_type).map(|s| s.to_bytes().to_vec()),
            origin_file: string(entry.origin_path).map(origin_file),
            level: entry.level,
            include_depth: entry.include_depth,
            callback,
        }
    }

    fn origin_file(s: &CStr) -> String {
        std::path::Path::new(s.to_str().unwrap())
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    fn rust_record(owner: &BackendEntryArc) -> Record {
        let entry = owner.as_ref().entry();
        let _policy = owner.as_ref().release_policy();
        Record {
            name: entry.name().map(|s| s.to_bytes().to_vec()),
            value: entry.value().map(|s| s.to_bytes().to_vec()),
            backend: entry.backend_type().map(|s| s.to_bytes().to_vec()),
            origin_file: entry.origin_path().map(origin_file),
            level: entry.level().raw(),
            include_depth: entry.include_depth(),
            callback: true,
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C reference and safe wrappers")]
    fn backend_entry_layout_and_lifecycle_match_c() {
        // Independently allocated configs and file buffers on each side.
        let base = std::env::temp_dir().join(format!(
            "crustify-backend-entry-equiv-{}",
            std::process::id()
        ));
        let mut paths = Vec::new();
        for side in ["c", "rust"] {
            let dir = base.join(side);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("config"),
                b"[test]\nvalue = old\nflag\nempty =\n[include]\npath = extra\n",
            )
            .unwrap();
            std::fs::write(dir.join("extra"), b"[test]\nnested = yes\n").unwrap();
            paths.push(CString::new(dir.join("config").to_str().unwrap()).unwrap());
        }
        let mut rust = Config::open(&paths[1]).unwrap();
        let mut c = ptr::null_mut();
        // SAFETY: initialized runtime, writable out slot and live C path string.
        let rc = unsafe { ffi::git_config_open_ondisk(&mut c, paths[0].as_ptr()) };
        assert_eq!(rc, 0);
        let mut retained = Vec::new();
        for key in [c"TEST.value", c"test.flag", c"test.empty", c"test.nested"] {
            let mut raw = ptr::null_mut();
            // SAFETY: initialized live config, writable slot, valid NUL key;
            // success returns one acquired reference, released below.
            let rc = unsafe { ffi::git_config_get_entry(&mut raw, c, key.as_ptr()) };
            assert_eq!(rc, 0);
            let owner = BackendEntry::from_entry(rust.as_ref().entry(key).unwrap());
            assert_eq!(raw_record(raw), rust_record(&owner));
            retained.push((raw, owner));
        }
        for value in [c"changed", c""] {
            let key = c"test.value";
            // SAFETY: live sole config and strings borrowed only during the call.
            let rc = unsafe { ffi::git_config_set_string(c, key.as_ptr(), value.as_ptr()) };
            assert_eq!(rc, 0);
            rust.as_mut().set_string(c"test.value", value).unwrap();
            let mut raw = ptr::null_mut();
            // SAFETY: as above, successful lookup acquires one reference.
            let rc = unsafe { ffi::git_config_get_entry(&mut raw, c, c"test.value".as_ptr()) };
            assert_eq!(rc, 0);
            let owner = BackendEntry::from_entry(rust.as_ref().entry(c"test.value").unwrap());
            assert_eq!(raw_record(raw), rust_record(&owner));
            retained.push((raw, owner));
            // Previously acquired entries are not affected by file refresh.
            for (raw, owner) in &retained {
                assert_eq!(raw_record(*raw), rust_record(owner));
            }
        }
        for key in [c"test.missing", c"invalid"] {
            let mut raw = ptr::null_mut();
            // SAFETY: on these errors C leaves NULL and transfers no reference.
            let rc = unsafe { ffi::git_config_get_entry(&mut raw, c, key.as_ptr()) };
            assert!(raw.is_null());
            assert_eq!(rc, rust.as_ref().entry(key).unwrap_err().raw_code());
        }
        // SAFETY: releases this config's sole reference; acquired entries retain lists.
        unsafe { ffi::git_config_free(c) };
        drop(rust);
        while let Some((raw, owner)) = retained.pop() {
            assert_eq!(raw_record(raw), rust_record(&owner));
            assert_eq!(owner.as_ref().entry().level(), ConfigLevel::LOCAL);
            let public = BackendEntry::into_entry(owner);
            let owner = BackendEntry::from_entry(public);
            assert_eq!(raw_record(raw), rust_record(&owner));
            // SAFETY: releases exactly the reference acquired above; no raw borrow remains.
            unsafe { ffi::git_config_entry_free(raw) };
            let policy = owner.as_ref().release_policy();
            policy.release(owner);
            for (raw, owner) in &retained {
                assert_eq!(raw_record(*raw), rust_record(owner));
            }
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    /// Entries from four priorities, two include levels and a shadowed key, with
    /// both configs freed before the entries are released in interleaved order.
    #[test]
    #[cfg_attr(miri, ignore = "calls C reference and safe wrappers")]
    fn backend_entries_across_levels_match_c() {
        const FILES: [(&str, &[u8]); 6] = [
            ("system", b"[a]\ntop = system\nsys = only\n"),
            ("global", b"[a]\ntop = global\n[include]\npath = inc1\n"),
            ("inc1", b"[include]\npath = inc2\n[a]\nshallow = 1\n"),
            ("inc2", b"[a]\ndeep = \"two levels\"\n"),
            ("local", b"[a]\ntop = local\nflag\n"),
            ("app", b"[a]\ntop = app\n"),
        ];
        let levels = [
            ("system", ConfigLevel::SYSTEM),
            ("global", ConfigLevel::GLOBAL),
            ("local", ConfigLevel::LOCAL),
            ("app", ConfigLevel::from_raw(ConfigLevel::APP.raw() + 3)),
        ];
        let base = std::env::temp_dir().join(format!(
            "crustify-backend-entry-levels-{}",
            std::process::id()
        ));
        // The safe constructor pins the runtime before the reference C calls.
        let mut rust = Config::new().unwrap();
        let mut c = ptr::null_mut();
        // SAFETY: initialized runtime and a writable out slot.
        assert_eq!(unsafe { ffi::git_config_new(&mut c) }, 0);
        for side in ["c", "rust"] {
            let dir = base.join(side);
            std::fs::create_dir_all(&dir).unwrap();
            for (name, contents) in FILES {
                std::fs::write(dir.join(name), contents).unwrap();
            }
            for (name, level) in levels {
                let path = CString::new(dir.join(name).to_str().unwrap()).unwrap();
                if side == "c" {
                    // SAFETY: live sole config; the path is copied; no repository.
                    let rc = unsafe {
                        ffi::git_config_add_file_ondisk(
                            c,
                            path.as_ptr(),
                            level.raw(),
                            ptr::null(),
                            0,
                        )
                    };
                    assert_eq!(rc, 0);
                } else {
                    rust.as_mut().add_file(&path, level, None, false).unwrap();
                }
            }
        }
        let mut retained = Vec::new();
        for key in [c"a.top", c"a.sys", c"a.shallow", c"A.deep", c"a.flag"] {
            let mut raw = ptr::null_mut();
            // SAFETY: live config, writable slot, NUL key; success acquires one
            // reference, released below.
            let rc = unsafe { ffi::git_config_get_entry(&mut raw, c, key.as_ptr()) };
            assert_eq!(rc, 0);
            let owner = BackendEntry::from_entry(rust.as_ref().entry(key).unwrap());
            assert_eq!(raw_record(raw), rust_record(&owner));
            retained.push((raw, owner));
        }
        let observed: Vec<_> = retained
            .iter()
            .map(|(_, owner)| {
                let entry = owner.as_ref().entry();
                (
                    entry.value().map(|v| v.to_bytes().to_vec()),
                    entry.level(),
                    entry.include_depth(),
                )
            })
            .collect();
        assert_eq!(
            observed,
            [
                (Some(b"app".to_vec()), levels[3].1, 0),
                (Some(b"only".to_vec()), ConfigLevel::SYSTEM, 0),
                (Some(b"1".to_vec()), ConfigLevel::GLOBAL, 1),
                (Some(b"two levels".to_vec()), ConfigLevel::GLOBAL, 2),
                (None, ConfigLevel::LOCAL, 0),
            ]
        );
        let mut raw = ptr::null_mut();
        // SAFETY: as above; a missing key transfers no reference.
        let rc = unsafe { ffi::git_config_get_entry(&mut raw, c, c"a.missing".as_ptr()) };
        assert!(raw.is_null());
        assert_eq!(
            rc,
            rust.as_ref().entry(c"a.missing").unwrap_err().raw_code()
        );
        // SAFETY: releases this config's sole reference; entries retain their lists.
        unsafe { ffi::git_config_free(c) };
        drop(rust);
        std::fs::remove_dir_all(&base).unwrap();
        // Release from the middle, then alternate ends, comparing the survivors.
        for index in [2, 0, 2, 1, 0] {
            let (raw, owner) = retained.remove(index);
            assert_eq!(raw_record(raw), rust_record(&owner));
            // SAFETY: releases exactly the reference acquired above.
            unsafe { ffi::git_config_entry_free(raw) };
            drop(owner);
            for (raw, owner) in &retained {
                assert_eq!(raw_record(*raw), rust_record(owner));
            }
        }
    }
}
