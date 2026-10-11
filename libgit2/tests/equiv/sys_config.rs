//! Independent raw C initialization and public Rust memory-options operations, and
//! Rust-implemented backends against a C-ABI backend serving the same store.
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

mod iterators {
    use super::*;
    use libgit2::libgit2::config::config::{Config, ConfigLevel, EntryRef};
    use libgit2::libgit2::config::sys::config::IteratorBox;
    use libgit2::libgit2::util::errors::last;
    use libgit2::libgit2::util::sys::errors::clear;
    use std::ptr;

    type Row = (
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        u32,
        i32,
    );

    fn observe(entry: EntryRef<'_>) -> Row {
        let bytes = |s: Option<&CStr>| s.map(|s| s.to_bytes().to_vec());
        (
            bytes(entry.name()),
            bytes(entry.value()),
            bytes(entry.backend_type()),
            bytes(entry.origin_path()),
            entry.include_depth(),
            entry.level().raw(),
        )
    }

    unsafe fn observe_raw(entry: *mut ffi::git_config_entry) -> Row {
        // SAFETY: callback success lends a live entry; copy the scalars/pointers
        // without forming a reference, before the next raw advance.
        let e = unsafe { entry.read() };
        let bytes = |p: *const std::ffi::c_char| {
            if p.is_null() {
                None
            } else {
                // SAFETY: entry strings are immutable and NUL-terminated until
                // the raw iterator advances; this copies them immediately.
                Some(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
            }
        };
        (
            bytes(e.name),
            bytes(e.value),
            bytes(e.backend_type),
            bytes(e.origin_path),
            e.include_depth,
            e.level,
        )
    }

    fn compare(mut rust: IteratorBox<'_>, raw: *mut ffi::git_config_iterator) -> usize {
        let mut count = 0;
        // SAFETY: independent complete raw iterator from successful C constructor;
        // reads only its initialized public base fields.
        unsafe {
            assert_eq!((*raw).flags, rust.as_ref().flags());
            assert!((*raw).backend.is_null());
        }
        let callback = rust.as_ref().next_callback();
        loop {
            let mut out = ptr::null_mut();
            // SAFETY: exclusive raw iterator and writable out slot; no previous
            // raw entry is retained. Rust runs on an independent C allocation.
            let rc = unsafe { ffi::git_config_next(&mut out, raw) };
            let mut m = rust.as_mut();
            match callback.call(&mut m).unwrap() {
                Some(entry) => {
                    assert_eq!(rc, 0);
                    // SAFETY: rc == 0 initialized out; copy while still borrowed.
                    assert_eq!(observe(entry.entry()), unsafe { observe_raw(out) });
                    count += 1;
                }
                None => {
                    assert_eq!(rc, ffi::GIT_ITEROVER);
                    break;
                }
            }
        }
        let mut out = ptr::null_mut();
        // SAFETY: raw iterator remains live after exhaustion, as C allows.
        let rc = unsafe { ffi::git_config_next(&mut out, raw) };
        assert_eq!(rc, ffi::GIT_ITEROVER);
        assert!(rust.as_mut().advance().unwrap().is_none());
        // SAFETY: releases the uniquely owned complete raw allocation exactly once.
        unsafe { ffi::git_config_iterator_free(raw) };
        rust.as_ref().release_policy().release(rust);
        count
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn iterator_sequences_match_c() {
        let dir =
            std::env::temp_dir().join(format!("crustify-iterator-equiv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config");
        std::fs::write(
            &file,
            b"[a]\nx = one\nx = two\nbin = \xff\nflag\nempty =\n[include]\npath = extra\n",
        )
        .unwrap();
        std::fs::write(dir.join("extra"), b"[a]\nnested = three\n").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let rust_config = Config::open(&path).unwrap();
        let raw_config = Config::open(&path).unwrap();
        let mut raw = ptr::null_mut();
        // SAFETY: live independent config, writable out; success uniquely owns raw.
        let rc = unsafe { ffi::git_config_iterator_new(&mut raw, raw_config.as_c_ptr()) };
        assert_eq!(rc, 0);
        assert_eq!(compare(rust_config.as_ref().iterator().unwrap(), raw), 7);
        for regexp in [None, Some(c"^a\\.x$"), Some(c"nomatch"), Some(c"")] {
            let mut raw = ptr::null_mut();
            // SAFETY: config remains live, regex borrowed only during construction.
            let rc = unsafe {
                ffi::git_config_iterator_glob_new(
                    &mut raw,
                    raw_config.as_c_ptr(),
                    regexp.map_or(ptr::null(), CStr::as_ptr),
                )
            };
            assert_eq!(rc, 0);
            compare(rust_config.as_ref().iterator_matching(regexp).unwrap(), raw);
        }
        for (name, regexp, expected) in [
            (c"A.x", None, 2),
            (c"a.x", Some(c"^two$"), 1),
            (c"a.empty", Some(c"^$"), 1),
            (c"a.missing", None, 0),
            (c"a.flag", None, 1),
            (c"a.bin", Some(c"^\xff$"), 1),
        ] {
            let mut raw = ptr::null_mut();
            // SAFETY: config lives through iteration and destruction; valid name
            // and optional regex are copied; success initializes a complete owner.
            let rc = unsafe {
                ffi::git_config_multivar_iterator_new(
                    &mut raw,
                    raw_config.as_c_ptr(),
                    name.as_ptr(),
                    regexp.map_or(ptr::null(), CStr::as_ptr),
                )
            };
            assert_eq!(rc, 0);
            assert_eq!(
                compare(
                    rust_config
                        .as_ref()
                        .multivar_iterator(name, regexp)
                        .unwrap(),
                    raw
                ),
                expected
            );
        }
        // Invalid names clean up C's inner allocation. Do not reproduce upstream
        // C's name leak with an invalid multivar regex; glob's path is clean.
        let mut raw = ptr::null_mut();
        // SAFETY: invalid regex is NUL-terminated; C cleans failed glob storage.
        let rc = unsafe {
            ffi::git_config_iterator_glob_new(&mut raw, raw_config.as_c_ptr(), c"[".as_ptr())
        };
        let error = rust_config
            .as_ref()
            .iterator_matching(Some(c"["))
            .unwrap_err();
        assert_eq!(error.raw_code(), rc);
        assert!(raw.is_null());
        // SAFETY: invalid name takes the constructor's cleanup path; writable out.
        let rc = unsafe {
            ffi::git_config_multivar_iterator_new(
                &mut raw,
                raw_config.as_c_ptr(),
                c"invalid".as_ptr(),
                ptr::null(),
            )
        };
        let error = rust_config
            .as_ref()
            .multivar_iterator(c"invalid", None)
            .unwrap_err();
        assert_eq!(error.raw_code(), rc);
        assert!(raw.is_null());
        drop((rust_config, raw_config));
        std::fs::remove_dir_all(dir).unwrap();
    }

    type Step = Result<Option<Row>, (i32, i32, Vec<u8>)>;

    fn rust_step(iter: &mut IteratorBox<'_>) -> Step {
        match iter.as_mut().advance() {
            Ok(entry) => Ok(entry.map(|e| observe(e.entry()))),
            Err(e) => Err((e.raw_code(), e.raw_class(), e.message().to_bytes().to_vec())),
        }
    }

    fn raw_step(raw: *mut ffi::git_config_iterator) -> Step {
        let mut out = ptr::null_mut();
        // SAFETY: exclusive live raw iterator and a writable slot; no earlier
        // raw entry is used after this advance.
        match unsafe { ffi::git_config_next(&mut out, raw) } {
            // SAFETY: success lent a live entry; it is copied before the next step.
            0 => Ok(Some(unsafe { observe_raw(out) })),
            ffi::GIT_ITEROVER => Ok(None),
            rc => {
                let e = last();
                Err((rc, e.raw_class(), e.message().to_bytes().to_vec()))
            }
        }
    }

    /// The regex-filtered multivar iterator (Rust-side PCRE2 filter over C's
    /// name selection) against C's own regex multivar iterator, step by step:
    /// entries from the lower backend, a higher backend whose file became
    /// unparsable before its lazy refresh, retries after the failure and after
    /// exhaustion, and the error each side leaves. No valueless entry carries
    /// the key, so the C reference never reaches its strlen(NULL).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2 and PCRE2")]
    fn filtered_multivar_steps_match_c() {
        let dir =
            std::env::temp_dir().join(format!("crustify-filtered-equiv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let global = dir.join("global");
        let local = dir.join("local");
        std::fs::write(&global, b"[a]\nx = one\nx = two\nx = three\ny\n").unwrap();
        std::fs::write(&local, b"[a]\nx = tw\xff\n").unwrap();
        let open = || {
            let path = CString::new(local.to_str().unwrap()).unwrap();
            let mut config = Config::open(&path).unwrap();
            let path = CString::new(global.to_str().unwrap()).unwrap();
            config
                .as_mut()
                .add_file(&path, ConfigLevel::GLOBAL, None, false)
                .unwrap();
            config
        };
        let (rust_config, raw_config) = (open(), open());
        let regex = c"^t";
        let mut rust = rust_config
            .as_ref()
            .multivar_iterator(c"A.X", Some(regex))
            .unwrap();
        let mut raw = ptr::null_mut();
        // SAFETY: live independent config; name and regex are copied; success
        // writes a sole owner, released below.
        let rc = unsafe {
            ffi::git_config_multivar_iterator_new(
                &mut raw,
                raw_config.as_c_ptr(),
                c"A.X".as_ptr(),
                regex.as_ptr(),
            )
        };
        assert_eq!(rc, 0);
        let mut steps = vec![];
        for step in 0..6 {
            if step == 2 {
                // The local backend is opened (and refreshed) only once the
                // global one is exhausted.
                std::fs::write(&local, b"[unterminated\n").unwrap();
            }
            clear();
            let r = rust_step(&mut rust);
            let rust_error = last();
            clear();
            let c = raw_step(raw);
            let raw_error = last();
            assert_eq!(r, c, "step {step}");
            assert_eq!(
                (rust_error.raw_class(), rust_error.message()),
                (raw_error.raw_class(), raw_error.message()),
                "step {step}"
            );
            steps.push(r);
        }
        let values: Vec<_> = steps[..2]
            .iter()
            .map(|s| s.as_ref().unwrap().as_ref().unwrap().1.clone().unwrap())
            .collect();
        assert_eq!(values, [b"two".to_vec(), b"three".to_vec()]);
        assert!(matches!(&steps[2], Err((-1, class, _)) if *class == ffi::GIT_ERROR_CONFIG as i32));
        assert!(steps[3..].iter().all(|s| *s == Ok(None)));
        // SAFETY: releases the uniquely owned raw iterator once.
        unsafe { ffi::git_config_iterator_free(raw) };
        drop(rust);
        // A fresh pair sees the repaired file, through both backends.
        std::fs::write(&local, b"[a]\nx = tw\xff\n[b]\nx = top\n").unwrap();
        let mut rust = rust_config
            .as_ref()
            .multivar_iterator(c"a.x", Some(regex))
            .unwrap();
        let mut raw = ptr::null_mut();
        // SAFETY: as above.
        let rc = unsafe {
            ffi::git_config_multivar_iterator_new(
                &mut raw,
                raw_config.as_c_ptr(),
                c"a.x".as_ptr(),
                regex.as_ptr(),
            )
        };
        assert_eq!(rc, 0);
        let mut count = 0;
        loop {
            let (r, c) = (rust_step(&mut rust), raw_step(raw));
            assert_eq!(r, c);
            match r {
                Ok(Some(_)) => count += 1,
                _ => break,
            }
        }
        assert_eq!(count, 3);
        // SAFETY: as above.
        unsafe { ffi::git_config_iterator_free(raw) };
        drop(rust);
        drop((rust_config, raw_config));
        std::fs::remove_dir_all(dir).unwrap();
    }

    const REGEX_CHILD: &str = "CRUSTIFY_EQUIV_FILTER_REGEX_CHILD";

    /// Rust compiles a multivar regex with PCRE2 itself; its failure must read as
    /// C's: code, class and PCRE2's message, after C's name check. C leaks the
    /// normalized name on this path, so the comparison runs in a child process
    /// with LeakSanitizer off; the Rust side's cleanliness is checked by the
    /// `ub_safe` suite.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2 and PCRE2")]
    fn multivar_regex_errors_match_c() {
        const NAME: &str = "sys_config::iterators::multivar_regex_errors_match_c";
        if std::env::var_os(REGEX_CHILD).is_none() {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([NAME, "--exact", "--test-threads=1", "--nocapture"])
                .env(REGEX_CHILD, "1")
                .env("ASAN_OPTIONS", "detect_leaks=0:abort_on_error=1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            return;
        }
        let config = Config::new().unwrap();
        let seed = |class: i32| {
            // SAFETY: runtime pinned by the config; the message is copied.
            unsafe { ffi::git_error_set_str(class, c"seed".as_ptr()) };
        };
        for (name, regex) in [
            (c"a.b", c"["),
            (c"a.b", c"("),
            (c"a.b", c"a{2,1}"),
            (c"a.b", c"\\"),
            (c"a.b", c"*"),
            (c"a.b", c"(?<n>a)(?<n>b)"),
            (c"a.b", c"\xff("),
            (c"invalid", c"["),
            (c"a.", c"("),
        ] {
            for class in [ffi::GIT_ERROR_OS as i32, 4242] {
                seed(class);
                let r = config
                    .as_ref()
                    .multivar_iterator(name, Some(regex))
                    .unwrap_err();
                let r = (r.raw_code(), r.raw_class(), r.message().to_owned());
                let rust_error = last();
                seed(class);
                let mut raw = ptr::null_mut();
                // SAFETY: live config, NUL-terminated name and regex, writable
                // slot that C leaves untouched on failure.
                let rc = unsafe {
                    ffi::git_config_multivar_iterator_new(
                        &mut raw,
                        config.as_c_ptr(),
                        name.as_ptr(),
                        regex.as_ptr(),
                    )
                };
                assert!(raw.is_null());
                let e = last();
                assert_eq!(
                    r,
                    (rc, e.raw_class(), e.message().to_owned()),
                    "{name:?} {regex:?}"
                );
                assert_eq!(rust_error, e);
            }
        }
    }
}

mod backends {
    use super::*;
    use libgit2::libgit2::config::sys::config::Backend;

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn backend_template_init_matches_c() {
        let mut raw = MaybeUninit::uninit();
        // SAFETY: writable slot for the supported version's complete template.
        assert_eq!(
            // SAFETY: complete writable slot for the supported template.
            unsafe {
                ffi::git_config_init_backend(raw.as_mut_ptr(), ffi::GIT_CONFIG_BACKEND_VERSION)
            },
            0
        );
        // SAFETY: successful init initialized every field.
        let raw = unsafe { raw.assume_init() };
        let rust = Backend::new();
        assert_eq!(raw.version, rust.as_ref().version());
        assert_eq!(raw.readonly, rust.as_ref().readonly_raw());
        assert_eq!(raw.cfg.is_null(), rust.as_ref().config_identity().is_none());
        assert!(raw.open.is_none() && raw.free.is_none());
    }
}

mod custom_backends {
    //! One store behind two backends: a C-ABI backend written against
    //! `git_config_backend` as a C author writes one, and the same store as a
    //! [`CustomBackend`]. Both are driven through libgit2's config routines in the
    //! same sequences, comparing returns, errors, outputs, the store's calls and
    //! its releases after each step.
    use libgit2::ffi;
    use libgit2::libgit2::config::config::{Config, ConfigBox, ConfigError, ConfigLevel};
    use libgit2::libgit2::config::sys::config::{
        BackendEntry, BackendEntryArc, CustomBackend, EntryOrigin, UnopenedBackend,
    };
    use libgit2::libgit2::repository::repository::RepositoryRef;
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};
    use std::cell::RefCell;
    use std::ffi::{CStr, CString, c_char, c_int};
    use std::ptr::{self, addr_of_mut};
    use std::rc::Rc;

    type Pairs = Vec<(CString, CString)>;
    /// A store's failure: code, class, message.
    type Failure = (c_int, c_int, CString);

    /// The store both backends serve, recording its calls.
    #[derive(Clone)]
    struct Store {
        values: Pairs,
        staged: Option<Pairs>,
        log: Rc<RefCell<Vec<String>>>,
    }

    fn failure(code: c_int, message: String) -> Failure {
        (
            code,
            ffi::GIT_ERROR_CONFIG as c_int,
            CString::new(message).unwrap(),
        )
    }

    impl Store {
        fn new(values: &[(&CStr, &CStr)]) -> Self {
            Self {
                values: values
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                staged: None,
                log: Rc::default(),
            }
        }
        fn record(&self, call: String) {
            self.log.borrow_mut().push(call);
        }
        fn writing(&mut self) -> &mut Pairs {
            self.staged.as_mut().unwrap_or(&mut self.values)
        }
        fn open(&mut self, level: c_int, repo: bool) -> Result<(), Failure> {
            self.record(format!("open {level} {repo}"));
            if self
                .values
                .iter()
                .any(|(k, _)| k.as_c_str() == c"open.fail")
            {
                return Err((-7, ffi::GIT_ERROR_OS as c_int, c"store refused".to_owned()));
            }
            Ok(())
        }
        fn get(&mut self, key: &CStr) -> Option<(CString, CString)> {
            self.record(format!("get {key:?}"));
            self.values
                .iter()
                .rev()
                .find(|(k, _)| k.as_c_str() == key)
                .cloned()
        }
        fn entries(&mut self) -> Result<Pairs, Failure> {
            self.record("iterator".to_owned());
            if self
                .values
                .iter()
                .any(|(k, _)| k.as_c_str() == c"iterator.fail")
            {
                return Err(failure(ffi::GIT_ERROR, "cannot walk".to_owned()));
            }
            Ok(self.values.clone())
        }
        fn snapshot(&mut self) -> Store {
            self.record("snapshot".to_owned());
            Store {
                values: self.values.clone(),
                staged: None,
                log: self.log.clone(),
            }
        }
        fn set(&mut self, key: &CStr, value: &CStr) -> Result<(), Failure> {
            self.record(format!("set {key:?} {value:?}"));
            let values = self.writing();
            match values.iter().filter(|(k, _)| k.as_c_str() == key).count() {
                0 => values.push((key.to_owned(), value.to_owned())),
                1 => {
                    values
                        .iter_mut()
                        .find(|(k, _)| k.as_c_str() == key)
                        .unwrap()
                        .1 = value.to_owned()
                }
                _ => return Err(failure(ffi::GIT_EEXISTS, format!("{key:?} is a multivar"))),
            }
            Ok(())
        }
        fn set_multivar(&mut self, key: &CStr, regexp: &CStr, value: &CStr) -> Result<(), Failure> {
            self.record(format!("set_multivar {key:?} {regexp:?} {value:?}"));
            let pattern = regexp.to_bytes().to_owned();
            let values = self.writing();
            let mut hits = 0;
            for (k, v) in values.iter_mut() {
                if k.as_c_str() == key && v.to_bytes().starts_with(&pattern) {
                    *v = value.to_owned();
                    hits += 1;
                }
            }
            if hits == 0 {
                values.push((key.to_owned(), value.to_owned()));
            }
            Ok(())
        }
        fn delete(&mut self, key: &CStr) -> Result<(), Failure> {
            self.record(format!("del {key:?}"));
            let values = self.writing();
            let before = values.len();
            values.retain(|(k, _)| k.as_c_str() != key);
            match before - values.len() {
                0 => Err(failure(ffi::GIT_ENOTFOUND, format!("no {key:?} to delete"))),
                _ => Ok(()),
            }
        }
        fn delete_multivar(&mut self, key: &CStr, regexp: &CStr) -> Result<(), Failure> {
            self.record(format!("del_multivar {key:?} {regexp:?}"));
            let pattern = regexp.to_bytes().to_owned();
            let values = self.writing();
            let before = values.len();
            values.retain(|(k, v)| k.as_c_str() != key || !v.to_bytes().starts_with(&pattern));
            match before - values.len() {
                0 => Err(failure(ffi::GIT_ENOTFOUND, format!("no {key:?} matches"))),
                _ => Ok(()),
            }
        }
        fn lock(&mut self) -> Result<(), Failure> {
            self.record("lock".to_owned());
            if self.staged.is_some() {
                return Err(failure(ffi::GIT_ELOCKED, "already locked".to_owned()));
            }
            self.staged = Some(self.values.clone());
            Ok(())
        }
        fn unlock(&mut self, commit: bool) -> Result<(), Failure> {
            self.record(format!("unlock {commit}"));
            let staged = self.staged.take().unwrap();
            if commit {
                self.values = staged;
            }
            Ok(())
        }
    }

    impl Drop for Store {
        fn drop(&mut self) {
            self.record("free".to_owned());
        }
    }

    // --- The store as a CustomBackend ---------------------------------------------

    struct Safe(Store);

    fn safe_error((code, class, message): Failure) -> ConfigError {
        let class = ErrorClass::from_raw(class).unwrap();
        ConfigError::new(class, &message)
            .with_code(ErrorCode::from_raw(code).unwrap_or(ErrorCode::Generic))
    }

    fn safe_entry(name: &CStr, value: &CStr) -> BackendEntryArc {
        let origin = EntryOrigin {
            backend_type: Some(c"store"),
            origin_path: None,
            include_depth: 0,
        };
        BackendEntry::new(name, Some(value), ConfigLevel::APP, origin)
    }

    impl CustomBackend for Safe {
        type Entries = std::vec::IntoIter<Result<BackendEntryArc, ConfigError>>;
        fn open(
            &mut self,
            level: ConfigLevel,
            repository: Option<RepositoryRef<'_>>,
        ) -> Result<(), ConfigError> {
            self.0
                .open(level.raw(), repository.is_some())
                .map_err(safe_error)
        }
        fn get(&mut self, key: &CStr) -> Option<BackendEntryArc> {
            self.0.get(key).map(|(k, v)| safe_entry(&k, &v))
        }
        fn iterator(&mut self) -> Result<Self::Entries, ConfigError> {
            let pairs = self.0.entries().map_err(safe_error)?;
            Ok(pairs
                .iter()
                .map(|(k, v)| Ok(safe_entry(k, v)))
                .collect::<Vec<_>>()
                .into_iter())
        }
        fn snapshot(&mut self) -> Result<UnopenedBackend, ConfigError> {
            Ok(UnopenedBackend::new_readonly(Safe(self.0.snapshot())))
        }
        fn set(&mut self, key: &CStr, value: &CStr) -> Result<(), ConfigError> {
            self.0.set(key, value).map_err(safe_error)
        }
        fn set_multivar(
            &mut self,
            key: &CStr,
            regexp: &CStr,
            value: &CStr,
        ) -> Result<(), ConfigError> {
            self.0.set_multivar(key, regexp, value).map_err(safe_error)
        }
        fn delete(&mut self, key: &CStr) -> Result<(), ConfigError> {
            self.0.delete(key).map_err(safe_error)
        }
        fn delete_multivar(&mut self, key: &CStr, regexp: &CStr) -> Result<(), ConfigError> {
            self.0.delete_multivar(key, regexp).map_err(safe_error)
        }
        fn lock(&mut self) -> Result<(), ConfigError> {
            self.0.lock().map_err(safe_error)
        }
        fn unlock(&mut self, commit: bool) -> Result<(), ConfigError> {
            self.0.unlock(commit).map_err(safe_error)
        }
    }

    // --- The store as a C author's backend ---------------------------------------

    /// Entry strings, shared by every entry handed out for them and kept by the
    /// backend until it is freed, as C's own backends keep their entry lists.
    #[repr(C)]
    struct RawEntry {
        base: ffi::git_config_backend_entry,
        strings: Rc<(CString, CString)>,
    }

    #[repr(C)]
    struct Raw {
        base: ffi::git_config_backend,
        store: Store,
        kept: Vec<Rc<(CString, CString)>>,
    }

    #[repr(C)]
    struct RawIter {
        base: ffi::git_config_iterator,
        pairs: std::vec::IntoIter<(CString, CString)>,
        current: *mut ffi::git_config_backend_entry,
    }

    unsafe extern "C" fn raw_entry_free(entry: *mut ffi::git_config_backend_entry) {
        // SAFETY: installed only on `RawEntry` boxes, each released once.
        drop(unsafe { Box::from_raw(entry.cast::<RawEntry>()) });
    }

    fn raw_entry(
        kept: &mut Vec<Rc<(CString, CString)>>,
        (k, v): (CString, CString),
    ) -> *mut ffi::git_config_backend_entry {
        let strings = Rc::new((k, v));
        kept.push(strings.clone());
        let base = ffi::git_config_backend_entry {
            entry: ffi::git_config_entry {
                name: strings.0.as_ptr(),
                value: strings.1.as_ptr(),
                backend_type: c"store".as_ptr(),
                origin_path: ptr::null(),
                include_depth: 0,
                level: ffi::GIT_CONFIG_LEVEL_APP,
            },
            free: Some(raw_entry_free),
        };
        Box::into_raw(Box::new(RawEntry { base, strings })).cast()
    }

    /// The store of the backend `p` heads.
    ///
    /// # Safety
    ///
    /// `p` heads a live `Raw` box that nothing else reaches for `'a`.
    unsafe fn raw<'a>(p: *mut ffi::git_config_backend) -> &'a mut Raw {
        // SAFETY: per this function's contract.
        unsafe { &mut *p.cast::<Raw>() }
    }

    fn raw_status(result: Result<(), Failure>) -> c_int {
        match result {
            Ok(()) => 0,
            Err((code, class, message)) => {
                // SAFETY: a NUL-terminated message, copied by libgit2.
                unsafe { ffi::git_error_set_str(class, message.as_ptr()) };
                code
            }
        }
    }

    /// # Safety
    ///
    /// `s` is a NUL-terminated string live for `'a`.
    unsafe fn arg<'a>(s: *const c_char) -> &'a CStr {
        // SAFETY: per this function's contract.
        unsafe { CStr::from_ptr(s) }
    }

    unsafe extern "C" fn raw_open(
        p: *mut ffi::git_config_backend,
        level: ffi::git_config_level_t,
        repo: *const ffi::git_repository,
    ) -> c_int {
        // SAFETY: libgit2 calls each slot with the live backend, one at a time.
        raw_status(unsafe { raw(p) }.store.open(level, !repo.is_null()))
    }
    unsafe extern "C" fn raw_get(
        p: *mut ffi::git_config_backend,
        key: *const c_char,
        out: *mut *mut ffi::git_config_backend_entry,
    ) -> c_int {
        // SAFETY: as above; libgit2 passes a live key and a writable slot.
        let (this, key) = unsafe { (raw(p), arg(key)) };
        match this.store.get(key) {
            // SAFETY: as above.
            Some(pair) => unsafe {
                out.write(raw_entry(&mut this.kept, pair));
                0
            },
            None => {
                // SAFETY: as above.
                unsafe { out.write(ptr::null_mut()) };
                ffi::GIT_ENOTFOUND
            }
        }
    }
    unsafe extern "C" fn raw_set(
        p: *mut ffi::git_config_backend,
        key: *const c_char,
        value: *const c_char,
    ) -> c_int {
        // SAFETY: as above, with live strings.
        raw_status(unsafe { raw(p).store.set(arg(key), arg(value)) })
    }
    unsafe extern "C" fn raw_set_multivar(
        p: *mut ffi::git_config_backend,
        key: *const c_char,
        regexp: *const c_char,
        value: *const c_char,
    ) -> c_int {
        // SAFETY: as above, with live strings.
        raw_status(unsafe { raw(p).store.set_multivar(arg(key), arg(regexp), arg(value)) })
    }
    unsafe extern "C" fn raw_del(p: *mut ffi::git_config_backend, key: *const c_char) -> c_int {
        // SAFETY: as above, with a live key.
        raw_status(unsafe { raw(p).store.delete(arg(key)) })
    }
    unsafe extern "C" fn raw_del_multivar(
        p: *mut ffi::git_config_backend,
        key: *const c_char,
        regexp: *const c_char,
    ) -> c_int {
        // SAFETY: as above, with live strings.
        raw_status(unsafe { raw(p).store.delete_multivar(arg(key), arg(regexp)) })
    }
    unsafe extern "C" fn raw_iter_next(
        out: *mut *mut ffi::git_config_backend_entry,
        iter: *mut ffi::git_config_iterator,
    ) -> c_int {
        // SAFETY: installed only on live `RawIter` boxes, advanced one call at a time.
        let iter = unsafe { &mut *iter.cast::<RawIter>() };
        if !iter.current.is_null() {
            // SAFETY: the entry lent by the previous advance, released once.
            unsafe { raw_entry_free(iter.current) };
            iter.current = ptr::null_mut();
        }
        match iter.pairs.next() {
            Some(pair) => {
                iter.current = raw_entry(&mut Vec::new(), pair);
                // SAFETY: libgit2 passes a writable slot.
                unsafe { out.write(iter.current) };
                0
            }
            None => ffi::GIT_ITEROVER,
        }
    }
    unsafe extern "C" fn raw_iter_free(iter: *mut ffi::git_config_iterator) {
        // SAFETY: installed only on live `RawIter` boxes, released once.
        let iter = unsafe { Box::from_raw(iter.cast::<RawIter>()) };
        if !iter.current.is_null() {
            // SAFETY: the entry still lent, released once.
            unsafe { raw_entry_free(iter.current) };
        }
    }
    unsafe extern "C" fn raw_iterator(
        out: *mut *mut ffi::git_config_iterator,
        p: *mut ffi::git_config_backend,
    ) -> c_int {
        // SAFETY: as for the other slots.
        let this = unsafe { raw(p) };
        match this.store.entries() {
            Ok(pairs) => {
                let iter = RawIter {
                    base: ffi::git_config_iterator {
                        backend: ptr::null_mut(),
                        flags: 0,
                        next: Some(raw_iter_next),
                        free: Some(raw_iter_free),
                    },
                    pairs: pairs.into_iter(),
                    current: ptr::null_mut(),
                };
                // SAFETY: libgit2 passes a writable slot.
                unsafe { out.write(Box::into_raw(Box::new(iter)).cast()) };
                0
            }
            Err(failure) => raw_status(Err(failure)),
        }
    }
    unsafe extern "C" fn raw_snapshot(
        out: *mut *mut ffi::git_config_backend,
        p: *mut ffi::git_config_backend,
    ) -> c_int {
        // SAFETY: as for the other slots.
        let store = unsafe { raw(p) }.store.snapshot();
        // SAFETY: libgit2 passes a writable slot.
        unsafe { out.write(raw_backend(store, true)) };
        0
    }
    unsafe extern "C" fn raw_lock(p: *mut ffi::git_config_backend) -> c_int {
        // SAFETY: as for the other slots.
        raw_status(unsafe { raw(p) }.store.lock())
    }
    unsafe extern "C" fn raw_unlock(p: *mut ffi::git_config_backend, commit: c_int) -> c_int {
        // SAFETY: as for the other slots.
        raw_status(unsafe { raw(p) }.store.unlock(commit != 0))
    }
    unsafe extern "C" fn raw_free(p: *mut ffi::git_config_backend) {
        // SAFETY: libgit2 releases the backend once.
        drop(unsafe { Box::from_raw(p.cast::<Raw>()) });
    }

    fn raw_backend(store: Store, readonly: bool) -> *mut ffi::git_config_backend {
        let mut base = std::mem::MaybeUninit::uninit();
        // SAFETY: a writable slot for the supported version's template.
        let rc = unsafe {
            ffi::git_config_init_backend(base.as_mut_ptr(), ffi::GIT_CONFIG_BACKEND_VERSION)
        };
        assert_eq!(rc, 0);
        // SAFETY: initialised by the call above.
        let mut base = unsafe { base.assume_init() };
        base.readonly = c_int::from(readonly);
        base.open = Some(raw_open);
        base.get = Some(raw_get);
        base.set = Some(raw_set);
        base.set_multivar = Some(raw_set_multivar);
        base.del = Some(raw_del);
        base.del_multivar = Some(raw_del_multivar);
        base.iterator = Some(raw_iterator);
        base.snapshot = Some(raw_snapshot);
        base.lock = Some(raw_lock);
        base.unlock = Some(raw_unlock);
        base.free = Some(raw_free);
        Box::into_raw(Box::new(Raw {
            base,
            store,
            kept: Vec::new(),
        }))
        .cast()
    }

    // --- Driving both ------------------------------------------------------------

    /// A routine's return and the error it left: (rc, class, message); class and
    /// message only for a failure.
    type Outcome = (c_int, Option<(c_int, CString)>);
    /// A looked-up entry: name, value, backend type, level.
    type Fields = (CString, CString, Option<CString>, c_int);

    fn outcome(rc: c_int) -> Outcome {
        if rc >= 0 {
            return (rc, None);
        }
        // SAFETY: the calling thread's last error, read right after the failure.
        let error = unsafe { ffi::git_error_last() };
        // SAFETY: libgit2 returns a live error with a NUL-terminated message.
        let error = unsafe { &*error };
        // SAFETY: as above.
        let message = unsafe { CStr::from_ptr(error.message) }.to_owned();
        let class = error.klass;
        (rc, Some((class, message)))
    }

    struct Side {
        config: *mut ffi::git_config,
        log: Rc<RefCell<Vec<String>>>,
        /// Keeps the safe side's config owned by Rust, as a consumer holds it.
        _owner: Option<ConfigBox<'static>>,
    }

    impl Side {
        fn calls(&self) -> Vec<String> {
            self.log.take()
        }
        fn entry(&self, name: &CStr) -> (Outcome, Option<Fields>) {
            let mut out = ptr::null_mut();
            // SAFETY: a live config and key, a writable slot.
            let rc = unsafe { ffi::git_config_get_entry(&mut out, self.config, name.as_ptr()) };
            let result = outcome(rc);
            if rc < 0 {
                return (result, None);
            }
            // SAFETY: success returned an entry with live strings until freed.
            let e = unsafe { &*out };
            // SAFETY: as above; name and value are non-NULL in this store.
            let fields = unsafe {
                (
                    CStr::from_ptr(e.name).to_owned(),
                    CStr::from_ptr(e.value).to_owned(),
                    (!e.backend_type.is_null()).then(|| CStr::from_ptr(e.backend_type).to_owned()),
                    e.level,
                )
            };
            // SAFETY: the reference the lookup acquired, released once.
            unsafe { ffi::git_config_entry_free(out) };
            (result, Some(fields))
        }
        fn walk(&self) -> (Outcome, Pairs) {
            let mut iter = ptr::null_mut();
            // SAFETY: a live config and a writable slot.
            let rc = unsafe { ffi::git_config_iterator_new(&mut iter, self.config) };
            assert_eq!(rc, 0);
            let mut pairs = Vec::new();
            let rc = loop {
                let mut entry = ptr::null_mut();
                // SAFETY: the live iterator; a lent entry is read before the next call.
                let rc = unsafe { ffi::git_config_next(&mut entry, iter) };
                if rc != 0 {
                    break rc;
                }
                // SAFETY: as above.
                let e = unsafe { &*entry };
                // SAFETY: as above.
                pairs.push(unsafe {
                    (
                        CStr::from_ptr(e.name).to_owned(),
                        CStr::from_ptr(e.value).to_owned(),
                    )
                });
            };
            let result = outcome(rc);
            // SAFETY: the iterator from above, released once.
            unsafe { ffi::git_config_iterator_free(iter) };
            (result, pairs)
        }
        fn set(&self, key: &CStr, value: &CStr) -> Outcome {
            // SAFETY: a live config and strings.
            outcome(unsafe {
                ffi::git_config_set_string(self.config, key.as_ptr(), value.as_ptr())
            })
        }
        fn set_multivar(&self, key: &CStr, regexp: &CStr, value: &CStr) -> Outcome {
            // SAFETY: a live config and strings.
            outcome(unsafe {
                ffi::git_config_set_multivar(
                    self.config,
                    key.as_ptr(),
                    regexp.as_ptr(),
                    value.as_ptr(),
                )
            })
        }
        fn delete(&self, key: &CStr) -> Outcome {
            // SAFETY: a live config and key.
            outcome(unsafe { ffi::git_config_delete_entry(self.config, key.as_ptr()) })
        }
        fn delete_multivar(&self, key: &CStr, regexp: &CStr) -> Outcome {
            // SAFETY: a live config and strings.
            outcome(unsafe {
                ffi::git_config_delete_multivar(self.config, key.as_ptr(), regexp.as_ptr())
            })
        }
        /// Locks, writes `key` inside the transaction, and commits or rolls back.
        fn transaction(&self, key: &CStr, commit: bool) -> (Outcome, Outcome, Outcome) {
            let mut tx = ptr::null_mut();
            // SAFETY: a live config and a writable slot.
            let locked = outcome(unsafe { ffi::git_config_lock(&mut tx, self.config) });
            let set = self.set(key, c"staged");
            let mut relocked = ptr::null_mut();
            // SAFETY: as above; the store refuses a nested lock.
            let nested = outcome(unsafe { ffi::git_config_lock(&mut relocked, self.config) });
            assert!(relocked.is_null());
            if commit {
                // SAFETY: the live transaction, committed once.
                assert_eq!(unsafe { ffi::git_transaction_commit(tx) }, 0);
            }
            // SAFETY: the transaction, released once; uncommitted, it rolls back.
            unsafe { ffi::git_transaction_free(tx) };
            (locked, set, nested)
        }
        fn snapshot_string(&self, key: &CStr) -> (Outcome, Option<CString>) {
            let mut snapshot = ptr::null_mut();
            // SAFETY: a live config and a writable slot.
            let rc = unsafe { ffi::git_config_snapshot(&mut snapshot, self.config) };
            assert_eq!(rc, 0);
            let mut value = ptr::null();
            // SAFETY: the live snapshot, read-only; the value lives with it.
            let rc = unsafe { ffi::git_config_get_string(&mut value, snapshot, key.as_ptr()) };
            let result = outcome(rc);
            // SAFETY: as above, read before the snapshot is released.
            let value = (rc == 0).then(|| unsafe { CStr::from_ptr(value) }.to_owned());
            // SAFETY: the snapshot, released once.
            unsafe { ffi::git_config_free(snapshot) };
            (result, value)
        }
    }

    fn raw_side(store: Store, level: ConfigLevel) -> (Side, Outcome) {
        let log = store.log.clone();
        // The safe constructor initialises libgit2 for good before any raw call.
        drop(Config::new().unwrap());
        let mut config = ptr::null_mut();
        // SAFETY: a writable slot; libgit2 is initialised.
        assert_eq!(unsafe { ffi::git_config_new(&mut config) }, 0);
        let backend = raw_backend(store, false);
        let level = level.raw();
        // SAFETY: a live config and the complete backend; on failure it stays ours.
        let rc = unsafe { ffi::git_config_add_backend(config, backend, level, ptr::null(), 0) };
        let result = outcome(rc);
        if rc < 0 {
            // SAFETY: the refused backend, released through its own slot.
            unsafe { raw_free(backend) };
        }
        (
            Side {
                config,
                log,
                _owner: None,
            },
            result,
        )
    }

    fn safe_side(store: Store, level: ConfigLevel) -> (Side, Outcome) {
        let log = store.log.clone();
        let mut owner = Config::new().unwrap();
        let result =
            match owner
                .as_mut()
                .add_backend(UnopenedBackend::new(Safe(store)), level, None, false)
            {
                Ok(()) => (0, None),
                Err(error) => (
                    error.raw_code(),
                    Some((error.raw_class(), error.message().to_owned())),
                ),
            };
        let config = owner.as_c_ptr();
        (
            Side {
                config,
                log,
                _owner: Some(owner),
            },
            result,
        )
    }

    fn free_raw(side: Side) -> Vec<String> {
        // SAFETY: the raw side's config, released once.
        unsafe { ffi::git_config_free(side.config) };
        side.calls()
    }

    fn free_safe(mut side: Side) -> Vec<String> {
        drop(side._owner.take());
        side.calls()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn custom_backend_sequences_match_a_c_backend() {
        let values = [
            (c"core.editor", c"vi"),
            (c"multi.v", c"a1"),
            (c"multi.v", c"b2"),
        ];
        let (c, c_added) = raw_side(Store::new(&values), ConfigLevel::LOCAL);
        let (r, r_added) = safe_side(Store::new(&values), ConfigLevel::LOCAL);
        assert_eq!(c_added, r_added);
        assert_eq!(c.calls(), r.calls());
        for key in [c"core.editor", c"multi.v", c"core.missing"] {
            assert_eq!(c.entry(key), r.entry(key), "{key:?}");
            assert_eq!(c.calls(), r.calls());
        }
        let steps: [&dyn Fn(&Side) -> Outcome; 9] = [
            &|s| s.set(c"core.editor", c"nano"),
            &|s| s.set(c"core.new", c"value"),
            &|s| s.set(c"multi.v", c"x"),
            &|s| s.set_multivar(c"multi.v", c"b", c"b3"),
            &|s| s.set_multivar(c"multi.v", c"z", c"z1"),
            &|s| s.delete_multivar(c"multi.v", c"a"),
            &|s| s.delete_multivar(c"multi.v", c"q"),
            &|s| s.delete(c"core.new"),
            &|s| s.delete(c"core.new"),
        ];
        for (i, step) in steps.iter().enumerate() {
            assert_eq!(step(&c), step(&r), "step {i}");
            assert_eq!(c.calls(), r.calls(), "step {i}");
            assert_eq!(c.walk(), r.walk(), "step {i}");
            assert_eq!(c.calls(), r.calls(), "step {i}");
        }
        for commit in [false, true] {
            assert_eq!(
                c.transaction(c"tx.key", commit),
                r.transaction(c"tx.key", commit)
            );
            assert_eq!(c.calls(), r.calls());
            assert_eq!(c.entry(c"tx.key"), r.entry(c"tx.key"));
            assert_eq!(c.calls(), r.calls());
        }
        for key in [c"core.editor", c"tx.key", c"core.missing"] {
            assert_eq!(c.snapshot_string(key), r.snapshot_string(key), "{key:?}");
            assert_eq!(c.calls(), r.calls());
        }
        assert_eq!(free_raw(c), free_safe(r));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn custom_backend_failures_match_a_c_backend() {
        let failing = [(c"open.fail", c"1")];
        let (c, c_added) = raw_side(Store::new(&failing), ConfigLevel::GLOBAL);
        let (r, r_added) = safe_side(Store::new(&failing), ConfigLevel::GLOBAL);
        assert_eq!(c_added, r_added);
        assert_eq!(c_added.0, -7);
        assert_eq!(c.calls(), r.calls());
        assert_eq!(free_raw(c), free_safe(r));
        let walk = [(c"iterator.fail", c"1"), (c"a.b", c"c")];
        let (c, _) = raw_side(Store::new(&walk), ConfigLevel::APP);
        let (r, _) = safe_side(Store::new(&walk), ConfigLevel::APP);
        let walked = c.walk();
        assert_eq!(walked, r.walk());
        assert_eq!(walked.0.0, ffi::GIT_ERROR);
        assert_eq!(c.calls(), r.calls());
        let raw_store = |s: &Side| {
            let mut iter = ptr::null_mut();
            let mut entry = ptr::null_mut();
            // SAFETY: a live config, writable slots; the first advance fails.
            unsafe {
                assert_eq!(ffi::git_config_iterator_new(&mut iter, s.config), 0);
                let first = outcome(ffi::git_config_next(&mut entry, iter));
                let second = outcome(ffi::git_config_next(&mut entry, iter));
                ffi::git_config_iterator_free(iter);
                (first, second)
            }
        };
        assert_eq!(raw_store(&c), raw_store(&r));
        assert_eq!(c.calls(), r.calls());
        assert_eq!(free_raw(c), free_safe(r));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn custom_backend_entry_layout_matches_c() {
        let mut owner = Config::new().unwrap();
        let store = Store::new(&[(c"a.b", c"v")]);
        owner
            .as_mut()
            .add_backend(
                UnopenedBackend::new(Safe(store)),
                ConfigLevel::APP,
                None,
                false,
            )
            .unwrap();
        let mut out = ptr::null_mut();
        let config = owner.as_c_ptr();
        // SAFETY: a live config and key, a writable slot.
        let rc = unsafe { ffi::git_config_get_entry(&mut out, config, c"a.b".as_ptr()) };
        assert_eq!(rc, 0);
        // SAFETY: the acquired entry heads its backend entry, whose callback is set.
        let backend = unsafe { &mut *out.cast::<ffi::git_config_backend_entry>() };
        assert!(backend.free.is_some());
        assert_eq!(
            addr_of_mut!(backend.entry).cast::<ffi::git_config_entry>(),
            out
        );
        // SAFETY: as above.
        let backend_type = unsafe { CStr::from_ptr(backend.entry.backend_type) };
        assert_eq!(backend_type, c"store");
        assert!(backend.entry.origin_path.is_null());
        // SAFETY: the one acquired reference, released once.
        unsafe { ffi::git_config_entry_free(out) };
        drop(owner);
    }
}
