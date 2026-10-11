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
