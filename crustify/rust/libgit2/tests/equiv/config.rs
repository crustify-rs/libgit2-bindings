//! Equivalence of `git2/config.h`'s [`Config`] owners with the `git_config` C code
//! builds with `git_config_new`, receives from `git_repository_config`, and releases
//! with `git_config_free`.
//!
//! Each config is produced twice, independently, one per side: an empty one (C:
//! `git_config_new`; Rust: [`Config::new`], and the shared form
//! [`ConfigArc::from`]), and a repository's (`git_repository_config` on its own open
//! `testrepo.git`, which the Rust side adopts as a [`ConfigArc`], as the wrapper
//! will). The config routines have no wrappers yet, so both sides drive their object
//! through the same raw calls, the Rust side reaching its object through the owner's
//! `as_c_ptr`: a lookup that misses, a write without a backend, adding a file backend
//! (each side its own file), writes and reads through it, then (C alone, see below)
//! a read after the repository is freed. After every step both record the return
//! code, the value read or the error left (class and message), and the records are
//! compared; the files each side wrote are compared at the end. C then frees its
//! config, and Rust drops its owner.
//!
//! One documented divergence: C lets the caller's reference to a repository's config
//! outlive the repository (a read that does not reparse the file goes through; one
//! that does reads the freed repository, see
//! `crustify/reviews/.../wave-1/batch-1/ub_safe/repository-config-keeps-freed-repository-for-includeif`),
//! where a [`ConfigArc<'r>`] borrows the repository and is released first.

use std::ffi::{CStr, CString, c_int};
use std::path::{Path, PathBuf};
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::config::config::{Config, ConfigArc};
use libgit2::libgit2::util::errors::last;

/// Pins the process runtime through the safe config constructor before reference C calls.
fn init() {
    let _ = Config::new().unwrap();
}

/// One step's observation: the return code, the value read (0 unless one was), and
/// for a failure the error libgit2 left, `(class, message)`.
type Record = (c_int, i32, Option<(c_int, String)>);

/// The calling thread's last error, read through C.
fn c_error() -> (c_int, String) {
    // SAFETY: no arguments; libgit2 is initialised, so it returns a live error.
    let error = unsafe { ffi::git_error_last().read() };
    // SAFETY: a libgit2 error's message is NUL-terminated, live until the next call.
    let message = unsafe { CStr::from_ptr(error.message) };
    (error.klass, message.to_string_lossy().into_owned())
}

/// The calling thread's last error, read through the safe API.
fn rust_error() -> (c_int, String) {
    let error = last();
    (
        error.raw_class(),
        error.message().to_string_lossy().into_owned(),
    )
}

fn record(rc: c_int, value: i32, error: fn() -> (c_int, String)) -> Record {
    (rc, value, (rc < 0).then(error))
}

/// A scratch config file path for `side` of `test`, removed first.
fn scratch(test: &str, side: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "crustify-equiv-config-{}-{test}-{side}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

/// The step sequence, run on one side's `config` with `error` reading its errors.
/// `file` names that side's own backend file.
fn steps(config: *mut ffi::git_config, file: &Path, error: fn() -> (c_int, String)) -> Vec<Record> {
    let file = CString::new(file.to_str().unwrap()).unwrap();
    let get = |name: &CStr| {
        let mut value = 0;
        // SAFETY: `config` is live for the call; `value` is writable, `name`
        // NUL-terminated.
        let rc = unsafe { ffi::git_config_get_int32(&mut value, config, name.as_ptr()) };
        record(rc, value, error)
    };
    let set = |name: &CStr, value: i32| {
        // SAFETY: `config` is live for the call; `name` is NUL-terminated.
        let rc = unsafe { ffi::git_config_set_int32(config, name.as_ptr(), value) };
        record(rc, 0, error)
    };
    let mut out = vec![get(c"crustify.value"), set(c"crustify.value", 1)];
    // SAFETY: `config` is live; `file` is NUL-terminated and copied; no repository.
    let rc = unsafe {
        ffi::git_config_add_file_ondisk(
            config,
            file.as_ptr(),
            ffi::GIT_CONFIG_LEVEL_LOCAL,
            ptr::null(),
            0,
        )
    };
    out.push(record(rc, 0, error));
    out.extend([
        set(c"crustify.value", 7),
        get(c"crustify.value"),
        set(c"crustify.value", -3),
        set(c"crustify.other", 11),
        get(c"crustify.value"),
        get(c"crustify.other"),
        get(c"crustify.missing"),
        set(c"invalid", 1),
    ]);
    out
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn new_configs_match() {
    init();
    let (c_file, rust_file) = (scratch("new", "c"), scratch("new", "rust"));
    let mut c_config = ptr::null_mut();
    // SAFETY: `c_config` is a writable slot.
    let c_rc = unsafe { ffi::git_config_new(&mut c_config) };
    let rust_config = Config::new();
    assert_eq!(c_rc == 0, rust_config.is_ok());
    let rust_config = rust_config.unwrap();

    let c_steps = steps(c_config, &c_file, c_error);
    let rust_steps = steps(rust_config.as_c_ptr(), &rust_file, rust_error);
    assert_eq!(c_steps, rust_steps);
    assert_eq!(c_steps[4], (0, 7, None), "the backend took the write");
    assert_eq!(
        std::fs::read(&c_file).unwrap(),
        std::fs::read(&rust_file).unwrap()
    );

    // SAFETY: the C side's only reference, released once.
    unsafe { ffi::git_config_free(c_config) };
    drop(rust_config);
    // Released configs leave their files as written.
    assert_eq!(
        std::fs::read(&c_file).unwrap(),
        std::fs::read(&rust_file).unwrap()
    );
    let _ = (
        std::fs::remove_file(c_file),
        std::fs::remove_file(rust_file),
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn shared_configs_match() {
    init();
    let (c_file, rust_file) = (scratch("shared", "c"), scratch("shared", "rust"));
    let mut c_config = ptr::null_mut();
    // SAFETY: `c_config` is a writable slot.
    assert_eq!(unsafe { ffi::git_config_new(&mut c_config) }, 0);
    let rust_config = ConfigArc::from(Config::new().unwrap());
    assert_eq!(
        steps(c_config, &c_file, c_error),
        steps(rust_config.as_c_ptr(), &rust_file, rust_error)
    );
    // SAFETY: the C side's only reference, released once.
    unsafe { ffi::git_config_free(c_config) };
    drop(rust_config);
    let _ = (
        std::fs::remove_file(c_file),
        std::fs::remove_file(rust_file),
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn repository_configs_match() {
    init();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git\0"
    );
    let open = || {
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` receives the repository; `path` is NUL-terminated.
        let rc = unsafe {
            ffi::git_repository_open_ext(
                &mut repo,
                path.as_ptr().cast(),
                ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                ptr::null(),
            )
        };
        assert_eq!(rc, 0);
        repo
    };
    let (c_repo, rust_repo) = (open(), open());
    let mut c_config = ptr::null_mut();
    // SAFETY: `c_config` is a writable slot, `c_repo` open.
    let c_rc = unsafe { ffi::git_repository_config(&mut c_config, c_repo) };
    let mut out = ptr::null_mut();
    // SAFETY: as above, for the Rust side's repository.
    let rust_rc = unsafe { ffi::git_repository_config(&mut out, rust_repo) };
    assert_eq!(c_rc, rust_rc);
    // SAFETY: `git_repository_config` hands out one up-ref'd reference to the
    // repository's config, released by `git_config_free`; the repository holds another,
    // and outlives the arc (freed below, after it): the arc's `'r`.
    let rust_config = unsafe { ConfigArc::from_c(out) }.unwrap();

    let reads = |config: *mut ffi::git_config, error: fn() -> (c_int, String)| {
        [
            c"core.repositoryformatversion",
            c"core.bare",
            c"core.missing",
        ]
        .map(|name| {
            let mut value = 0;
            // SAFETY: `config` is live; `value` is writable, `name` NUL-terminated.
            let rc = unsafe { ffi::git_config_get_int32(&mut value, config, name.as_ptr()) };
            record(rc, value, error)
        })
    };
    let before = reads(c_config, c_error);
    assert_eq!(before, reads(rust_config.as_c_ptr(), rust_error));
    // Documented divergence: C lets the caller's reference outlive the repository and
    // read on (the file backends still point at the freed repository, which only a
    // reparse reaches); a `ConfigArc<'r>` borrows the repository and goes first.
    drop(rust_config);
    for repo in [c_repo, rust_repo] {
        // SAFETY: each repository is open and freed once; the C config outlives its
        // repository, and the Rust arc was released before its own.
        unsafe { ffi::git_repository_free(repo) };
    }
    let after = reads(c_config, c_error);
    assert_eq!(after, before);
    // SAFETY: the C side's last reference, released once.
    unsafe { ffi::git_config_free(c_config) };
}

/// Each side owns its own input strings and replays the whole sequence without
/// clearing errors between calls: the boolean and 32-bit parsers' successes preserve
/// them. `parse_i64` clears the error before each call, and describes the one failure
/// libgit2 leaves silent (text after a size suffix) itself.
#[cfg(not(miri))]
mod parsers {
    use super::*;
    use libgit2::libgit2::config::config::{parse_bool, parse_i32, parse_i64};
    use libgit2::libgit2::util::errors::ErrorBuf;
    use libgit2::libgit2::util::errors::ErrorClass;
    use libgit2::libgit2::util::sys::errors::{clear, set_str};

    type Observation<T> = (c_int, Option<T>, Option<(c_int, Vec<u8>)>, (c_int, Vec<u8>));

    fn error_bytes(error: ErrorBuf) -> (c_int, Vec<u8>) {
        (error.raw_class(), error.message().to_bytes().to_vec())
    }

    fn rust_observation<T>(result: Result<T, ErrorBuf>) -> Observation<T> {
        let state = error_bytes(last());
        match result {
            Ok(out) => (0, Some(out), None, state),
            Err(error) => (-1, None, Some(error_bytes(error)), state),
        }
    }

    // C reads its last error directly, copying the bytes before the next call.
    fn raw_error() -> (c_int, Vec<u8>) {
        // SAFETY: the runtime is pinned before any step; the pointer is live on
        // this thread until its next C call. Read no error through a Rust reference.
        let error = unsafe { ffi::git_error_last().read() };
        // SAFETY: libgit2's message is NUL-terminated and still live, copied here.
        let bytes = unsafe { CStr::from_ptr(error.message) }.to_bytes().to_vec();
        (error.klass, bytes)
    }

    fn raw_observation<T>(rc: c_int, value: T) -> Observation<T> {
        let state = raw_error();
        (
            rc,
            (rc == 0).then_some(value),
            (rc < 0).then(|| state.clone()),
            state,
        )
    }

    fn seed(with_error: bool) {
        clear();
        if with_error {
            set_str(ErrorClass::Invalid, c"parser sequence sentinel").unwrap();
        }
    }

    fn inputs() -> Vec<Option<CString>> {
        let mut values = vec![Some(CString::new("1kb").unwrap()), None];
        // Suffix-tail failure comes before any new error, then again after one.
        for value in [
            "1kb",
            "true",
            "YES",
            "On",
            "FALSE",
            "no",
            "oFf",
            "",
            "0",
            "-0",
            "1",
            "-1",
            "+42",
            "  \t\r\n\x0b\x0c17",
            "010",
            "0x10",
            "-0X10",
            "0xFf",
            "1k",
            "-1K",
            "2m",
            "-2M",
            "1g",
            "-1G",
            "2147483647",
            "-2147483648",
            "2147483648",
            "-2147483649",
            "2097151K",
            "2097152k",
            "-2097152K",
            "2g",
            "-2G",
            "9223372036854775807",
            "-9223372036854775808",
            "9223372036854775808",
            "-9223372036854775809",
            "8589934591G",
            "8589934592G",
            "-8589934592G",
            "9007199254740992K",
            "9007199254740992G",
            "9007199254740992M",
            "8796093022208G",
            "8796093022208M",
            "-9007199254740992k",
            "-8796093022208m",
            " ",
            "+",
            "-",
            "0x",
            "0xg",
            "08",
            "1 ",
            "1t",
            "1kk",
            "1Mb",
            "1Gb",
            "true ",
            " true",
            "not-a-value",
            "1kb",
            "0",
            "off",
        ] {
            values.push(Some(CString::new(value).unwrap()));
        }
        values.push(Some(CString::new([0xff, b'1']).unwrap()));
        values.push(Some(CString::new("9".repeat(4096)).unwrap()));
        values
    }

    #[test]
    fn parse_sequences_match_c() {
        // Pins the runtime for the raw calls as well; each side then resets its
        // own thread error to the same starting state.
        clear();
        let c_inputs = inputs();
        let rust_inputs = inputs();
        macro_rules! compare {
            ($raw:path, $safe:path, $ty:ty, $convert:expr) => {{
                for with_error in [false, true] {
                    seed(with_error);
                    let c: Vec<_> = c_inputs
                        .iter()
                        .map(|value| {
                            let mut out: $ty = 123;
                            let input = value.as_ref().map_or(ptr::null(), |s| s.as_ptr());
                            // SAFETY: one aligned writable scalar and NULL or a live
                            // NUL-terminated read-only string; neither pointer is retained.
                            let rc = unsafe { $raw(&mut out, input) };
                            if rc < 0 {
                                assert_eq!(out, 123, "C must leave failed output untouched");
                            }
                            raw_observation(rc, ($convert)(out))
                        })
                        .collect();
                    seed(with_error);
                    for (index, value) in rust_inputs.iter().enumerate() {
                        let rust = rust_observation($safe(value.as_deref()));
                        assert_eq!(c[index], rust, "input {value:?}");
                    }
                }
            }};
        }
        compare!(ffi::git_config_parse_bool, parse_bool, c_int, |x| x != 0);
        compare!(ffi::git_config_parse_int32, parse_i32, i32, |x| x);
    }

    /// C is replayed with its error cleared before each call, as `parse_i64` clears it,
    /// and its silent failures are given the error C sets for every other one: the
    /// documented correction (`config.rs::unit_tests::parse_i64_suffix_tail_reports_its_own_error`).
    #[test]
    fn parse_i64_sequences_match_c() {
        clear();
        let c_inputs = inputs();
        let rust_inputs = inputs();
        // C's own silence: a suffix-tail failure leaves the earlier error in place.
        seed(true);
        let mut out = 123;
        // SAFETY: one aligned writable i64 and a live NUL-terminated read-only string;
        // neither pointer is retained.
        let rc = unsafe { ffi::git_config_parse_int64(&mut out, c"1kb".as_ptr()) };
        assert_eq!((rc, out), (-1, 123));
        let sentinel = (
            ErrorClass::Invalid.raw(),
            b"parser sequence sentinel".to_vec(),
        );
        assert_eq!(raw_error(), sentinel);
        for with_error in [false, true] {
            let mut silent = 0;
            let c: Vec<_> = c_inputs
                .iter()
                .map(|value| {
                    clear();
                    let mut out: i64 = 123;
                    let input = value.as_ref().map_or(ptr::null(), |s| s.as_ptr());
                    // SAFETY: one aligned writable i64 and NULL or a live NUL-terminated
                    // read-only string; neither pointer is retained.
                    let rc = unsafe { ffi::git_config_parse_int64(&mut out, input) };
                    if rc < 0 {
                        assert_eq!(out, 123, "C must leave failed output untouched");
                    }
                    let mut observed = raw_observation(rc, out);
                    if rc < 0 && observed.3.0 == ErrorClass::None.raw() {
                        let value = value.as_deref().map_or(&b"(null)"[..], CStr::to_bytes);
                        let message = [b"failed to parse '".as_slice(), value, b"' as an integer"];
                        let error = (ErrorClass::Config.raw(), message.concat());
                        observed.2 = Some(error.clone());
                        observed.3 = error;
                        silent += 1;
                    }
                    observed
                })
                .collect();
            assert!(silent >= 4, "the suffix-tail inputs fail silently in C");
            seed(with_error);
            for (index, value) in rust_inputs.iter().enumerate() {
                let rust = rust_observation(parse_i64(value.as_deref()));
                assert_eq!(c[index], rust, "input {value:?}");
            }
        }
    }
}

/// The signed sentinel and application-defined values survive a C-value round trip.
#[test]
fn config_level_values_match_c() {
    use libgit2::libgit2::config::config::ConfigLevel;
    for (rust, c) in [
        (ConfigLevel::SYSTEM, ffi::GIT_CONFIG_LEVEL_SYSTEM),
        (ConfigLevel::XDG, ffi::GIT_CONFIG_LEVEL_XDG),
        (ConfigLevel::GLOBAL, ffi::GIT_CONFIG_LEVEL_GLOBAL),
        (ConfigLevel::LOCAL, ffi::GIT_CONFIG_LEVEL_LOCAL),
        (ConfigLevel::WORKTREE, ffi::GIT_CONFIG_LEVEL_WORKTREE),
        (ConfigLevel::APP, ffi::GIT_CONFIG_LEVEL_APP),
        (ConfigLevel::HIGHEST, ffi::GIT_CONFIG_HIGHEST_LEVEL),
    ] {
        assert_eq!(rust.raw(), c);
        assert_eq!(ConfigLevel::from_raw(c), rust);
    }
    for custom in [7, 8, 42, i32::MAX] {
        assert_eq!(ConfigLevel::from_raw(custom).raw(), custom);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn mapping_mode_values_match_c_behavior() {
    use libgit2::libgit2::config::config::MapType;
    let _runtime = Config::new().unwrap();
    for (rust_mode, c_mode, value) in [
        (MapType::False, ffi::GIT_CONFIGMAP_FALSE, c"false"),
        (MapType::True, ffi::GIT_CONFIGMAP_TRUE, c"yes"),
        (MapType::Int32, ffi::GIT_CONFIGMAP_INT32, c"123"),
        (MapType::String, ffi::GIT_CONFIGMAP_STRING, c"MATCH"),
    ] {
        let mut observations = Vec::new();
        for mode in [rust_mode.raw(), c_mode] {
            let map = ffi::git_configmap {
                type_: mode,
                str_match: c"match".as_ptr(),
                map_value: 42,
            };
            for input in [value, c"unmapped"] {
                let mut out = -999;
                // SAFETY: initialized single-element map with a valid mode and
                // live matching string, writable output and NUL terminated input.
                let status = unsafe {
                    ffi::git_config_lookup_map_value(&mut out, &raw const map, 1, input.as_ptr())
                };
                observations.push((status, out));
            }
        }
        assert_eq!(observations[..2], observations[2..]);
        assert_eq!(
            observations[0],
            (0, if rust_mode == MapType::Int32 { 123 } else { 42 })
        );
        assert!(observations[1].0 < 0);
    }
}

mod entry_and_map_types {
    use super::*;
    use core::ptr::addr_of;
    use libgit2::libgit2::config::config::{ConfigLevel, ConfigMap, MapMatch};
    use libgit2::libgit2::repository::repository::Repository;

    fn bytes(p: *const core::ffi::c_char) -> Option<Vec<u8>> {
        if p.is_null() {
            None
        } else {
            // SAFETY: callers project live entry strings or their own CString
            // borrows, NUL-terminated and immutable until this copy completes.
            Some(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn entries_match_c_acquisition_fields_errors_and_release() {
        let dir =
            std::env::temp_dir().join(format!("crustify-equiv-entry-b0-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rust_path = CString::new(dir.join("rust").to_str().unwrap()).unwrap();
        let rust_repo = Repository::init(&rust_path, false).unwrap();
        let body = b"[core]\nrepositoryformatversion = 0\nbare = false\n[test]\nflag\nempty =\nvalue = old\n[include]\npath = extra\n";
        let rust_file = dir.join("rust/.git/config");
        let c_file = dir.join("config");
        std::fs::write(&rust_file, body).unwrap();
        std::fs::write(&c_file, body).unwrap();
        std::fs::write(dir.join("rust/.git/extra"), b"[test]\nnested = yes\n").unwrap();
        std::fs::write(dir.join("extra"), b"[test]\nnested = yes\n").unwrap();
        let rust_cfg = rust_repo.as_ref().config().unwrap();
        let mut c_cfg = ptr::null_mut();
        let c_path = CString::new(c_file.to_str().unwrap()).unwrap();
        // SAFETY: writable output and initialized runtime (Repository::init
        // pinned it); C owns this independently constructed config.
        assert_eq!(unsafe { ffi::git_config_new(&mut c_cfg) }, 0);
        assert_eq!(
            // SAFETY: live config; C copies path; no repository; force is false.
            unsafe {
                ffi::git_config_add_file_ondisk(
                    c_cfg,
                    c_path.as_ptr(),
                    ConfigLevel::LOCAL.raw(),
                    ptr::null(),
                    0,
                )
            },
            0
        );
        let mut c_kept = Vec::new();
        let mut rust_kept = Vec::new();
        for key in [
            c"TEST.value",
            c"test.flag",
            c"test.empty",
            c"test.nested",
            c"test.missing",
            c"invalid",
        ] {
            let mut c_entry = ptr::null_mut();
            // SAFETY: live C config, writable output slot, borrowed NUL key.
            let rc = unsafe { ffi::git_config_get_entry(&mut c_entry, c_cfg, key.as_ptr()) };
            let c_err = if rc < 0 { Some(last()) } else { None };
            match rust_cfg.as_ref().entry(key) {
                Err(e) => {
                    assert_eq!(e.raw_code(), rc);
                    assert_eq!(e.error(), c_err.as_ref().unwrap());
                }
                Ok(entry) => {
                    assert_eq!(rc, 0);
                    let r = entry.as_ref();
                    // SAFETY: acquisition succeeded; entry and its strings stay
                    // live until entry_free below; project without C references.
                    unsafe {
                        assert_eq!(
                            r.name().map(CStr::to_bytes).map(<[u8]>::to_vec),
                            bytes(addr_of!((*c_entry).name).read())
                        );
                        assert_eq!(
                            r.value().map(CStr::to_bytes).map(<[u8]>::to_vec),
                            bytes(addr_of!((*c_entry).value).read())
                        );
                        assert_eq!(
                            r.backend_type().map(CStr::to_bytes).map(<[u8]>::to_vec),
                            bytes(addr_of!((*c_entry).backend_type).read())
                        );
                        assert_eq!(r.level().raw(), addr_of!((*c_entry).level).read());
                        assert_eq!(r.include_depth(), addr_of!((*c_entry).include_depth).read());
                        let c_origin = bytes(addr_of!((*c_entry).origin_path).read()).unwrap();
                        let r_origin = r.origin_path().unwrap().to_bytes();
                        // Independent configs have different origin prefixes.
                        assert_eq!(
                            Path::new(std::str::from_utf8(&c_origin).unwrap()).file_name(),
                            Path::new(std::str::from_utf8(r_origin).unwrap()).file_name()
                        );
                    }
                    c_kept.push(c_entry);
                    rust_kept.push(entry);
                }
            }
        }
        // Refresh each independent config while old entries retain their storage.
        std::fs::write(&c_file, b"[test]\nvalue = replacement-longer\n").unwrap();
        std::fs::write(&rust_file, b"[core]\nrepositoryformatversion = 0\nbare = false\n[test]\nvalue = replacement-longer\n").unwrap();
        let mut fresh = ptr::null_mut();
        assert_eq!(
            // SAFETY: live config, output slot and NUL key; acquires one
            // independently releasable entry reference after refresh.
            unsafe { ffi::git_config_get_entry(&mut fresh, c_cfg, c"test.value".as_ptr()) },
            0
        );
        let fresh_rust = rust_cfg.as_ref().entry(c"test.value").unwrap();
        // SAFETY: fresh is a successfully acquired live entry.
        let fresh_value = unsafe { addr_of!((*fresh).value).read() };
        assert_eq!(
            fresh_rust.as_ref().value().unwrap().to_bytes(),
            bytes(fresh_value).unwrap()
        );
        // SAFETY: release each independently owned C config/reference once;
        // acquired entries own their storage and remain readable after this.
        unsafe { ffi::git_config_free(c_cfg) };
        drop(rust_cfg);
        drop(rust_repo);
        for (c_entry, entry) in c_kept.into_iter().zip(rust_kept) {
            // SAFETY: the acquired entry still retains its backing storage.
            let c_value = unsafe { addr_of!((*c_entry).value).read() };
            assert_eq!(
                entry
                    .as_ref()
                    .value()
                    .map(CStr::to_bytes)
                    .map(<[u8]>::to_vec),
                bytes(c_value)
            );
            // SAFETY: relinquishes exactly this acquired C entry reference.
            unsafe { ffi::git_config_entry_free(c_entry) };
            drop(entry);
        }
        // SAFETY: relinquishes exactly the fresh acquired C reference.
        unsafe { ffi::git_config_entry_free(fresh) };
        drop(fresh_rust);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mapping_fields_and_transitions_match_c_records() {
        init();
        let rust_text = CString::new("HeLLo").unwrap();
        let c_text = CString::new("HeLLo").unwrap();
        let mut map = ConfigMap::new(MapMatch::False, i32::MIN);
        let mut c_map = ffi::git_configmap {
            type_: ffi::GIT_CONFIGMAP_FALSE,
            str_match: ptr::null(),
            map_value: i32::MIN,
        };
        for (rule, input) in [
            (MapMatch::False, c"false"),
            (MapMatch::True, c"YES"),
            (MapMatch::Int32, c"-2k"),
            (MapMatch::String(&rust_text), c"hello"),
            (MapMatch::String(c""), c""),
        ] {
            map.as_mut().set_match(rule);
            map.as_mut().set_map_value(i32::MAX);
            c_map.type_ = rule.map_type().raw();
            c_map.map_value = i32::MAX;
            c_map.str_match = match rule {
                MapMatch::String(s) if s.is_empty() => c"".as_ptr(),
                MapMatch::String(_) => c_text.as_ptr(),
                _ => ptr::null(),
            };
            assert_eq!(map.as_ref().map_type().raw(), c_map.type_);
            assert_eq!(map.as_ref().map_value(), c_map.map_value);
            assert_eq!(
                map.as_ref()
                    .str_match()
                    .map(CStr::to_bytes)
                    .map(<[u8]>::to_vec),
                bytes(c_map.str_match)
            );
            let mut c_out = 0;
            assert_eq!(
                // SAFETY: one initialized mapping with a valid string for String
                // mode, independent immutable strings, exclusive scalar output;
                // C borrows everything only for this call.
                unsafe {
                    ffi::git_config_lookup_map_value(
                        &mut c_out,
                        &raw const c_map,
                        1,
                        input.as_ptr(),
                    )
                },
                0
            );
            let expected = if rule == MapMatch::Int32 {
                libgit2::libgit2::config::config::parse_i32(Some(input)).unwrap()
            } else {
                map.as_ref().map_value()
            };
            assert_eq!(c_out, expected);
            // Non-string modes may retain ignored borrowed strings as well.
            if rule.map_type() != libgit2::libgit2::config::config::MapType::String {
                map.as_mut().set_str_match(&rust_text);
                c_map.str_match = c_text.as_ptr();
                assert_eq!(
                    map.as_ref().str_match().unwrap().to_bytes(),
                    bytes(c_map.str_match).unwrap()
                );
                assert_eq!(
                    // SAFETY: unchanged valid mode, new immutable borrowed NUL
                    // string; C must ignore it while recognizing the same input.
                    unsafe {
                        ffi::git_config_lookup_map_value(
                            &mut c_out,
                            &raw const c_map,
                            1,
                            input.as_ptr(),
                        )
                    },
                    0
                );
                assert_eq!(c_out, expected);
            }
        }
    }
}

#[cfg(not(miri))]
mod file_access {
    use super::*;
    use libgit2::libgit2::config::config::{
        ConfigError, ConfigLevel, find_global, find_system, find_xdg, parse_path,
    };
    use libgit2::libgit2::core::buffer::BufVal;
    use libgit2::libgit2::util::sys::errors::clear;

    type Failure = (i32, i32, Vec<u8>);
    fn safe<T>(r: Result<T, ConfigError>) -> Result<T, Failure> {
        r.map_err(|e| (e.raw_code(), e.raw_class(), e.message().to_bytes().to_vec()))
    }
    fn raw<T>(rc: i32, out: T) -> Result<T, Failure> {
        if rc < 0 {
            let error = last();
            Err((rc, error.raw_class(), error.message().to_bytes().to_vec()))
        } else {
            Ok(out)
        }
    }
    struct Reference(*mut ffi::git_config);
    impl Reference {
        fn open(path: &CStr) -> Self {
            let mut out = ptr::null_mut();
            // SAFETY: writable output slot and copied NUL-terminated input. The
            // C reference is independently owned and released once by Drop.
            assert_eq!(
                // SAFETY: the independently owned C config and NUL-terminated
                // inputs are live; output storage is exclusively writable for this call.
                unsafe { ffi::git_config_open_ondisk(&mut out, path.as_ptr()) },
                0
            );
            Self(out)
        }
        fn level(&self, level: ConfigLevel, global: bool) -> Result<Self, Failure> {
            let mut out = ptr::null_mut();
            // SAFETY: live parent and writable slot. Success returns a fresh
            // counted config, backend retained; parent outlives this test's view.
            let rc = unsafe {
                if global {
                    ffi::git_config_open_global(&mut out, self.0)
                } else {
                    ffi::git_config_open_level(&mut out, self.0, level.raw())
                }
            };
            raw(rc, out).map(Self)
        }
    }
    impl Drop for Reference {
        fn drop(&mut self) {
            // SAFETY: releases precisely the constructor's one owned reference.
            unsafe { ffi::git_config_free(self.0) };
        }
    }
    fn bytes(r: Result<BufVal, ConfigError>) -> Result<Vec<u8>, Failure> {
        safe(r).map(|b| b.as_ref().as_bytes().to_vec())
    }
    fn buffer(f: impl FnOnce(*mut ffi::git_buf) -> i32) -> Result<Vec<u8>, Failure> {
        let mut out = ffi::git_buf {
            ptr: ptr::null_mut(),
            reserved: 0,
            size: 0,
        };
        let rc = f(&raw mut out);
        let result = if rc < 0 {
            raw(rc, Vec::new())
        } else {
            // SAFETY: successful C buffer routines fill a NUL-terminated buffer,
            // with static empty string for an empty result, still live until dispose.
            raw(rc, unsafe { CStr::from_ptr(out.ptr) }.to_bytes().to_vec())
        };
        // SAFETY: out is an initialized C buffer; releases any allocation once,
        // including partial results on failure. The runtime is already pinned.
        unsafe { ffi::git_buf_dispose(&raw mut out) };
        result
    }

    #[test]
    fn file_sequences_match_c() {
        clear();
        let (cfile, rfile) = (scratch("file-api", "c"), scratch("file-api", "rust"));
        for file in [&cfile, &rfile] {
            std::fs::write(file, b"[a]\n flag\n empty =\n path = relative/file\n number = 1\n multi = first\n multi = second\n").unwrap();
        }
        let cp = CString::new(cfile.to_str().unwrap()).unwrap();
        let rp = CString::new(rfile.to_str().unwrap()).unwrap();
        let c = Reference::open(&cp);
        let mut r = Config::open_ondisk(&rp).unwrap();
        macro_rules! compare {
            ($c:expr, $r:expr) => {{
                clear();
                let c_result = $c;
                clear();
                let r_result = $r;
                assert_eq!(c_result, r_result);
            }};
        }
        macro_rules! get {
            ($ffi:path, $method:ident, $key:expr, $initial:expr, $conv:expr) => {{
                let mut value = $initial;
                // SAFETY: independently owned live C config, writable scalar and
                // NUL-terminated name. C retains neither output nor name.
                compare!(
                    raw(
                        // SAFETY: the independently owned C config and NUL-terminated
                        // inputs are live; output storage is exclusively writable for this call.
                        unsafe { $ffi(&mut value, c.0, $key.as_ptr()) },
                        ($conv)(value)
                    ),
                    safe(r.as_ref().$method($key))
                );
            }};
        }
        get!(ffi::git_config_get_bool, get_bool, c"a.flag", 0, |v| v != 0);
        for key in [c"a.path", c"a.empty", c"a.flag", c"a.missing", c"invalid"] {
            // SAFETY: live C config, name and exclusively writable output buffer.
            compare!(
                buffer(|out| {
                    // SAFETY: out is initialized exclusive buffer storage; C config and
                    // input strings, when used, remain live for this call.
                    unsafe { ffi::git_config_get_string_buf(out, c.0, key.as_ptr()) }
                }),
                bytes(r.as_ref().get_string_buf(key))
            );
            // SAFETY: same as above; C checks valueless paths and returns an error.
            compare!(
                buffer(|out| {
                    // SAFETY: out is initialized exclusive buffer storage; C config and
                    // input strings, when used, remain live for this call.
                    unsafe { ffi::git_config_get_path(out, c.0, key.as_ptr()) }
                }),
                bytes(r.as_ref().get_path(key))
            );
            let mut out = ptr::null();
            // SAFETY: live config and name, writable output pointer; a live config
            // is rejected, with no borrowed result read here.
            compare!(
                raw(
                    // SAFETY: the independently owned C config and NUL-terminated
                    // inputs are live; output storage is exclusively writable for this call.
                    unsafe { ffi::git_config_get_string(&mut out, c.0, key.as_ptr()) },
                    ()
                ),
                safe(r.as_ref().get_string(key)).map(|_| ())
            );
        }
        for value in [i32::MIN, -1, 0, i32::MAX] {
            // SAFETY: live C config and name; value is formatted and copied by C.
            compare!(
                raw(
                    // SAFETY: the independently owned C config and NUL-terminated
                    // inputs are live; output storage is exclusively writable for this call.
                    unsafe { ffi::git_config_set_int32(c.0, c"a.number".as_ptr(), value) },
                    ()
                ),
                safe(r.as_mut().set_i32(c"a.number", value))
            );
            get!(ffi::git_config_get_int32, get_i32, c"a.number", 0, |v| v);
            get!(ffi::git_config_get_int64, get_i64, c"a.number", 0, |v| v);
        }
        for value in [i64::MIN, i64::MAX, 0] {
            // SAFETY: as the i32 setter, with C's full signed 64-bit scalar range.
            compare!(
                raw(
                    // SAFETY: the independently owned C config and NUL-terminated
                    // inputs are live; output storage is exclusively writable for this call.
                    unsafe { ffi::git_config_set_int64(c.0, c"a.number".as_ptr(), value) },
                    ()
                ),
                safe(r.as_mut().set_i64(c"a.number", value))
            );
            get!(ffi::git_config_get_int64, get_i64, c"a.number", 0, |v| v);
            get!(ffi::git_config_get_int32, get_i32, c"a.number", 0, |v| v);
        }
        for value in [true, false] {
            // SAFETY: live config, immutable NUL-terminated name; C copies scalar.
            compare!(
                raw(
                    // SAFETY: the independently owned C config and NUL-terminated
                    // inputs are live; output storage is exclusively writable for this call.
                    unsafe { ffi::git_config_set_bool(c.0, c"a.flag".as_ptr(), value.into()) },
                    ()
                ),
                safe(r.as_mut().set_bool(c"a.flag", value))
            );
            get!(ffi::git_config_get_bool, get_bool, c"a.flag", 0, |v| v != 0);
        }
        for pattern in [c"first", c"none", c"["] {
            // SAFETY: all strings are NUL-terminated and only borrowed for the call;
            // C compiles and disposes its regex even on invalid-pattern failures.
            compare!(
                raw(
                    // SAFETY: the independently owned C config and NUL-terminated
                    // inputs are live; output storage is exclusively writable for this call.
                    unsafe {
                        ffi::git_config_set_multivar(
                            c.0,
                            c"a.multi".as_ptr(),
                            pattern.as_ptr(),
                            c"replacement".as_ptr(),
                        )
                    },
                    ()
                ),
                safe(r.as_mut().set_multivar(c"a.multi", pattern, c"replacement"))
            );
            // SAFETY: same config/name/regex borrowing contract as set_multivar.
            compare!(
                raw(
                    // SAFETY: the independently owned C config and NUL-terminated
                    // inputs are live; output storage is exclusively writable for this call.
                    unsafe {
                        ffi::git_config_delete_multivar(c.0, c"a.multi".as_ptr(), pattern.as_ptr())
                    },
                    ()
                ),
                safe(r.as_mut().delete_multivar(c"a.multi", pattern))
            );
            assert_eq!(
                std::fs::read(&cfile).unwrap(),
                std::fs::read(&rfile).unwrap()
            );
        }
        for key in [c"a.flag", c"a.flag", c"invalid", c"a.multi"] {
            // SAFETY: live writer and NUL-terminated name, no pointer retained.
            compare!(
                raw(
                    // SAFETY: the independently owned C config and NUL-terminated
                    // inputs are live; output storage is exclusively writable for this call.
                    unsafe { ffi::git_config_delete_entry(c.0, key.as_ptr()) },
                    ()
                ),
                safe(r.as_mut().delete_entry(key))
            );
        }
        for force in [false, true] {
            // SAFETY: each side uses its own file path, copied; safe local priority;
            // repository is NULL and C handles backend transfer or disposal.
            compare!(
                raw(
                    // SAFETY: the independently owned C config and NUL-terminated
                    // inputs are live; output storage is exclusively writable for this call.
                    unsafe {
                        ffi::git_config_add_file_ondisk(
                            c.0,
                            cp.as_ptr(),
                            ConfigLevel::LOCAL.raw(),
                            ptr::null(),
                            force.into(),
                        )
                    },
                    ()
                ),
                safe(
                    r.as_mut()
                        .add_file_ondisk(&rp, ConfigLevel::LOCAL, None, force)
                )
            );
        }
        let mut transaction = ptr::null_mut();
        // SAFETY: live writer config and writable output slot; C transfers one
        // transaction on success, freed once below to abort its staged writes.
        let rc = unsafe { ffi::git_config_lock(&mut transaction, c.0) };
        assert_eq!(rc, 0);
        let mut tx = r.as_mut().lock().unwrap();
        clear();
        // SAFETY: live locked config and NUL-terminated inputs borrowed for call;
        // scalar or replacement contents are copied by the C writer.
        let rc = unsafe { ffi::git_config_set_int32(c.0, c"a.number".as_ptr(), 1) };
        let c_result = raw(rc, ());
        clear();
        assert_eq!(c_result, safe(tx.set_i32(c"a.number", 1)));
        clear();
        // SAFETY: live locked config and NUL-terminated inputs borrowed for call;
        // scalar or replacement contents are copied by the C writer.
        let rc = unsafe { ffi::git_config_set_int64(c.0, c"a.number".as_ptr(), i64::MAX) };
        let c_result = raw(rc, ());
        clear();
        assert_eq!(c_result, safe(tx.set_i64(c"a.number", i64::MAX)));
        clear();
        // SAFETY: live locked config and NUL-terminated inputs borrowed for call;
        // scalar or replacement contents are copied by the C writer.
        let rc = unsafe { ffi::git_config_set_bool(c.0, c"a.tx".as_ptr(), 1) };
        let c_result = raw(rc, ());
        clear();
        assert_eq!(c_result, safe(tx.set_bool(c"a.tx", true)));
        clear();
        // SAFETY: live locked config and NUL-terminated inputs borrowed for call;
        // scalar or replacement contents are copied by the C writer.
        let rc = unsafe {
            ffi::git_config_set_multivar(
                c.0,
                c"a.txmulti".as_ptr(),
                c".*".as_ptr(),
                c"first".as_ptr(),
            )
        };
        let c_result = raw(rc, ());
        clear();
        assert_eq!(
            c_result,
            safe(tx.set_multivar(c"a.txmulti", c".*", c"first"))
        );
        clear();
        // SAFETY: live locked config and NUL-terminated inputs borrowed for call;
        // scalar or replacement contents are copied by the C writer.
        let rc = unsafe { ffi::git_config_delete_entry(c.0, c"a.tx".as_ptr()) };
        let c_result = raw(rc, ());
        clear();
        assert_eq!(c_result, safe(tx.delete_entry(c"a.tx")));
        clear();
        // SAFETY: live locked config and NUL-terminated inputs borrowed for call;
        // scalar or replacement contents are copied by the C writer.
        let rc = unsafe {
            ffi::git_config_delete_multivar(c.0, c"a.txmulti".as_ptr(), c"first".as_ptr())
        };
        let c_result = raw(rc, ());
        clear();
        assert_eq!(c_result, safe(tx.delete_multivar(c"a.txmulti", c"first")));
        // SAFETY: sole successfully constructed transaction, released once,
        // aborting staged writes and releasing its retained backend reference.
        unsafe { ffi::git_transaction_free(transaction) };
        drop(tx);
        assert_eq!(
            std::fs::read(&cfile).unwrap(),
            std::fs::read(&rfile).unwrap()
        );
        for level in [
            ConfigLevel::LOCAL,
            ConfigLevel::HIGHEST,
            ConfigLevel::GLOBAL,
        ] {
            let cv = c.level(level, false);
            let mut parent = r.as_mut();
            let rv = parent.open_level(level);
            assert_eq!(
                cv.as_ref().err(),
                rv.as_ref()
                    .err()
                    .map(|e| (e.raw_code(), e.raw_class(), e.message().to_bytes().to_vec()))
                    .as_ref()
            );
            if let (Ok(cv), Ok(mut rv)) = (cv, rv) {
                // SAFETY: both independent views retain their backend and parent.
                compare!(
                    raw(
                        // SAFETY: the independently owned C config and NUL-terminated
                        // inputs are live; output storage is exclusively writable for this call.
                        unsafe { ffi::git_config_set_int32(cv.0, c"a.view".as_ptr(), 7) },
                        ()
                    ),
                    safe(rv.as_mut().set_i32(c"a.view", 7))
                );
            }
        }
        assert_eq!(
            std::fs::read(&cfile).unwrap(),
            std::fs::read(&rfile).unwrap()
        );
        drop(c);
        drop(r);
        for file in [cfile, rfile] {
            std::fs::remove_file(file).unwrap();
        }
    }

    #[test]
    fn snapshot_strings_and_readonly_mutations_match_c() {
        use libgit2::libgit2::repository::repository::Repository;
        clear();
        let dirs = [
            scratch("snapshot-file", "c"),
            scratch("snapshot-file", "rust"),
        ];
        let mut repositories = Vec::new();
        for dir in &dirs {
            let _ = std::fs::remove_dir_all(dir);
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let repo = Repository::init(&path, true).unwrap();
            std::fs::write(
                dir.join("config"),
                b"[core]\n bare = true\n[a]\n flag\n empty =\n path = relative/file\n number = 7\n",
            )
            .unwrap();
            repositories.push(repo);
        }
        let mut out = ptr::null_mut();
        // SAFETY: live independently owned repository and writable output slot.
        // A successful snapshot is a fresh config released by Reference's Drop.
        let rc =
            unsafe { ffi::git_repository_config_snapshot(&mut out, repositories[0].as_c_ptr()) };
        assert_eq!(rc, 0);
        let c = Reference(out);
        let mut r = repositories[1].as_ref().config_snapshot().unwrap();
        for name in [
            c"a.flag",
            c"a.empty",
            c"a.path",
            c"a.number",
            c"a.missing",
            c"invalid",
        ] {
            clear();
            let mut out = ptr::null();
            // SAFETY: snapshot keeps the borrowed result alive until its release;
            // live name and writable output slot; failure output is never read.
            let rc = unsafe { ffi::git_config_get_string(&mut out, c.0, name.as_ptr()) };
            let c_result = if rc < 0 {
                raw(rc, CString::default())
            } else {
                // SAFETY: successful snapshot getter returns a live NUL string,
                // copied before any other C call can occur.
                raw(rc, unsafe { CStr::from_ptr(out) }.to_owned())
            };
            clear();
            assert_eq!(c_result, safe(r.as_ref().get_string(name)));
        }
        macro_rules! readonly {
            ($raw:expr, $safe:expr) => {{
                clear();
                let c_result = raw($raw, ());
                clear();
                assert_eq!(c_result, safe($safe));
            }};
        }
        readonly!(
            // SAFETY: live readonly config and borrowed NUL name; scalar copied.
            unsafe { ffi::git_config_set_bool(c.0, c"a.flag".as_ptr(), 1) },
            r.as_mut().set_bool(c"a.flag", true)
        );
        readonly!(
            // SAFETY: live readonly config and borrowed NUL name; scalar copied.
            unsafe { ffi::git_config_set_int32(c.0, c"a.number".as_ptr(), 1) },
            r.as_mut().set_i32(c"a.number", 1)
        );
        readonly!(
            // SAFETY: live readonly config and borrowed NUL name; scalar copied.
            unsafe { ffi::git_config_set_int64(c.0, c"a.number".as_ptr(), 1) },
            r.as_mut().set_i64(c"a.number", 1)
        );
        readonly!(
            // SAFETY: live readonly config and NUL strings borrowed for this call.
            unsafe {
                ffi::git_config_set_multivar(
                    c.0,
                    c"a.flag".as_ptr(),
                    c".*".as_ptr(),
                    c"true".as_ptr(),
                )
            },
            r.as_mut().set_multivar(c"a.flag", c".*", c"true")
        );
        readonly!(
            // SAFETY: live readonly config and borrowed NUL name, nothing retained.
            unsafe { ffi::git_config_delete_entry(c.0, c"a.flag".as_ptr()) },
            r.as_mut().delete_entry(c"a.flag")
        );
        readonly!(
            // SAFETY: live readonly config and NUL strings borrowed for this call.
            unsafe { ffi::git_config_delete_multivar(c.0, c"a.flag".as_ptr(), c".*".as_ptr()) },
            r.as_mut().delete_multivar(c"a.flag", c".*")
        );
        drop(c);
        drop(r);
        drop(repositories);
        for dir in dirs {
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn search_and_path_results_match_c() {
        clear();
        for (raw_find, safe_find) in [
            (
                ffi::git_config_find_global as unsafe extern "C" fn(*mut ffi::git_buf) -> i32,
                find_global as fn() -> Result<BufVal, ConfigError>,
            ),
            (ffi::git_config_find_xdg, find_xdg),
            (ffi::git_config_find_system, find_system),
        ] {
            clear();
            // SAFETY: initialized exclusive buffer and pinned library runtime.
            let c = buffer(|out| {
                // SAFETY: out is initialized exclusive buffer storage; C config and
                // input strings, when used, remain live for this call.
                unsafe { raw_find(out) }
            });
            clear();
            assert_eq!(c, bytes(safe_find()));
        }
        for value in [c"", c"relative/\xff", c"~", c"~/child", c"~user/child"] {
            clear();
            // SAFETY: initialized exclusive buffer; NUL-terminated immutable value
            // remains live throughout C's expansion and copy; no input retained.
            let c = buffer(|out| {
                // SAFETY: out is initialized exclusive buffer storage; C config and
                // input strings, when used, remain live for this call.
                unsafe { ffi::git_config_parse_path(out, value.as_ptr()) }
            });
            clear();
            assert_eq!(c, bytes(parse_path(value)));
        }
        let mut out = ptr::null_mut();
        // SAFETY: writable output slot; C builds an owned config loaded for no repo.
        let rc = unsafe { ffi::git_config_open_default(&mut out) };
        let c = raw(rc, out).map(Reference);
        let r = safe(Config::open_default());
        assert_eq!(c.is_ok(), r.is_ok());
        if let (Ok(c), Ok(r)) = (c, r) {
            for global in [false, true] {
                clear();
                let cv = c.level(ConfigLevel::HIGHEST, global);
                clear();
                let rv = if global {
                    r.as_ref().open_global()
                } else {
                    r.as_ref().open_level(ConfigLevel::HIGHEST)
                };
                assert_eq!(cv.is_ok(), rv.is_ok());
                if let (Err(c), Err(r)) = (cv, rv) {
                    assert_eq!(c, safe::<()>(Err(r)).unwrap_err());
                }
            }
        }
    }
}

mod writes_and_snapshots {
    use super::*;
    use libgit2::libgit2::config::config::{ConfigBox, ConfigError, ConfigLevel};
    use libgit2::libgit2::util::errors::ErrorCode;

    fn raw_config() -> ConfigBox<'static> {
        let _pin = Config::new().unwrap();
        let mut out = ptr::null_mut();
        // SAFETY: runtime pinned, one writable result slot; C creates a sole ref.
        assert_eq!(unsafe { ffi::git_config_new(&mut out) }, 0);
        // SAFETY: fresh sole reference, not repository-bound, paired with config_free.
        unsafe { ConfigBox::from_c(out) }.unwrap()
    }

    fn attach(config: &mut ConfigBox<'_>, file: &Path, level: ConfigLevel) {
        let file = CString::new(file.to_str().unwrap()).unwrap();
        config
            .as_mut()
            .add_file_ondisk(&file, level, None, false)
            .unwrap();
    }

    fn observation(status: Result<(), ConfigError>) -> Record {
        match status {
            Ok(()) => (0, 0, None),
            Err(e) => (
                e.raw_code(),
                0,
                Some((e.raw_class(), e.message().to_string_lossy().into_owned())),
            ),
        }
    }

    fn set(config: &mut ConfigBox<'_>, raw: bool, name: &CStr, value: &CStr) -> Record {
        if raw {
            // SAFETY: live independent config; strings read/copied, not retained.
            let rc = unsafe {
                ffi::git_config_set_string(config.as_c_ptr(), name.as_ptr(), value.as_ptr())
            };
            record(rc, 0, c_error)
        } else {
            observation(config.as_mut().set_string(name, value))
        }
    }

    fn order(config: &mut ConfigBox<'_>, raw: bool, levels: &[ConfigLevel]) -> Record {
        if raw {
            let mut levels: Vec<_> = levels.iter().map(|level| level.raw()).collect();
            let p = if levels.is_empty() {
                ptr::null_mut()
            } else {
                levels.as_mut_ptr()
            };
            // SAFETY: exclusive independent config, bounded array read only during call.
            let rc = unsafe { ffi::git_config_set_writeorder(config.as_c_ptr(), p, levels.len()) };
            record(rc, 0, c_error)
        } else {
            observation(config.as_mut().set_write_order(levels))
        }
    }

    fn snapshot(config: &ConfigBox<'_>, raw: bool) -> Result<ConfigBox<'static>, Record> {
        if raw {
            let mut out = ptr::null_mut();
            // SAFETY: live source; writable slot; C owns partial construction on failure.
            let rc = unsafe { ffi::git_config_snapshot(&mut out, config.as_c_ptr()) };
            if rc < 0 {
                assert!(out.is_null());
                return Err(record(rc, 0, c_error));
            }
            // SAFETY: success hands out one fresh sole snapshot, independent of source.
            Ok(unsafe { ConfigBox::from_c(out) }.unwrap())
        } else {
            config.as_ref().snapshot().map_err(|e| observation(Err(e)))
        }
    }

    fn get(config: &ConfigBox<'_>, name: &CStr, raw: bool) -> Record {
        if !raw {
            return match config.as_ref().get_i32(name) {
                Ok(value) => (0, value, None),
                Err(e) => (
                    e.raw_code(),
                    0,
                    Some((e.raw_class(), e.message().to_string_lossy().into_owned())),
                ),
            };
        }
        let mut value = 0;
        // SAFETY: live config, output slot, NUL-terminated key; no retention.
        let rc = unsafe { ffi::git_config_get_int32(&mut value, config.as_c_ptr(), name.as_ptr()) };
        record(rc, value, c_error)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn multi_call_writes_order_and_snapshot_match_c() {
        let mut configs = [raw_config(), Config::new().unwrap()];
        let files: Vec<_> = ["c", "rust"]
            .iter()
            .map(|side| {
                [
                    scratch("string-local", side),
                    scratch("string-global", side),
                ]
            })
            .collect();
        // Empty config, invalid names, empty values; errors copied before next call.
        let mut observations = [Vec::new(), Vec::new()];
        for side in 0..2 {
            let raw = side == 0;
            observations[side].push(set(&mut configs[side], raw, c"a.b", c"1"));
            let empty = snapshot(&configs[side], raw).unwrap();
            assert_eq!(get(&empty, c"a.b", raw).0, ErrorCode::NotFound.raw());
            attach(&mut configs[side], &files[side][0], ConfigLevel::LOCAL);
            attach(&mut configs[side], &files[side][1], ConfigLevel::GLOBAL);
        }
        assert_eq!(observations[0], observations[1]);
        for levels in [
            vec![ConfigLevel::GLOBAL],
            vec![ConfigLevel::LOCAL],
            vec![ConfigLevel::LOCAL, ConfigLevel::GLOBAL],
            vec![ConfigLevel::GLOBAL, ConfigLevel::LOCAL],
            vec![ConfigLevel::LOCAL, ConfigLevel::LOCAL],
            vec![
                ConfigLevel::from_raw(i32::MIN),
                ConfigLevel::from_raw(i32::MAX),
            ],
            vec![],
            vec![ConfigLevel::GLOBAL],
        ] {
            let mut step = [Vec::new(), Vec::new()];
            for side in 0..2 {
                let raw = side == 0;
                step[side].push(order(&mut configs[side], raw, &levels));
                for (name, value) in [
                    (c"a.b", c"123"),
                    (c"a.b", c"456"),
                    (c"a.b", c"456"),
                    (c"a.text", c""),
                    (c"a.text", c" white # ; \" \\ \n\xff"),
                    (c"invalid", c"1"),
                    (c".broken", c"2"),
                    (c"A.Subsection.KEY", c"3"),
                ] {
                    step[side].push(set(&mut configs[side], raw, name, value));
                    step[side].push(get(&configs[side], c"a.b", raw));
                }
                let mut snap = snapshot(&configs[side], raw).unwrap();
                step[side].push(get(&snap, c"a.b", raw));
                step[side].push(set(&mut snap, raw, c"a.b", c"777"));
                step[side].push(order(
                    &mut snap,
                    raw,
                    &[ConfigLevel::LOCAL, ConfigLevel::GLOBAL],
                ));
                step[side].push(set(&mut snap, raw, c"a.b", c"888"));
                step[side].push(set(&mut configs[side], raw, c"a.b", c"999"));
                step[side].push(get(&snap, c"a.b", raw));
                let nested = snapshot(&snap, raw).unwrap();
                drop(snap);
                step[side].push(get(&nested, c"a.b", raw));
            }
            assert_eq!(step[0], step[1], "levels {levels:?}");
            for (c_file, rust_file) in files[0].iter().zip(&files[1]) {
                assert_eq!(
                    std::fs::read(c_file).unwrap_or_default(),
                    std::fs::read(rust_file).unwrap_or_default()
                );
            }
        }
        drop(configs);
        for file in files.iter().flatten() {
            let _ = std::fs::remove_file(file);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn multivar_and_file_lock_error_sequences_match_c() {
        let mut observations = Vec::new();
        for (side, raw) in [("c", true), ("rust", false)] {
            let mut writer = if raw {
                raw_config()
            } else {
                Config::new().unwrap()
            };
            let file = scratch("write-errors", side);
            std::fs::write(&file, "[a]\n b = 1\n b = 2\n").unwrap();
            attach(&mut writer, &file, ConfigLevel::LOCAL);
            let duplicate = set(&mut writer, raw, c"a.b", c"3");
            let mut blocker = Config::new().unwrap();
            attach(&mut blocker, &file, ConfigLevel::LOCAL);
            let lock = blocker.as_mut().lock().unwrap();
            let locked = set(&mut writer, raw, c"a.unique", c"3");
            assert_eq!(locked.0, ErrorCode::Locked.raw());
            drop(lock);
            drop(blocker);
            let success = set(&mut writer, raw, c"a.unique", c"3");
            let sequence: Vec<_> = [duplicate, locked, success]
                .into_iter()
                .map(|(rc, value, error)| {
                    (
                        rc,
                        value,
                        error.map(|(class, message)| {
                            (class, message.replace(file.to_str().unwrap(), "<config>"))
                        }),
                    )
                })
                .collect();
            observations.push((sequence, std::fs::read(&file).unwrap()));
            drop(writer);
            std::fs::remove_file(file).unwrap();
        }
        assert_eq!(observations[0], observations[1]);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn partial_snapshot_failure_matches_c_and_releases_partial_owner() {
        let mut results = Vec::new();
        for (side, raw) in [("c", true), ("rust", false)] {
            let mut config = if raw {
                raw_config()
            } else {
                Config::new().unwrap()
            };
            let good = scratch("partial-good", side);
            let bad = scratch("partial-bad", side);
            std::fs::write(&good, "[a]\n b = 12\n").unwrap();
            std::fs::write(&bad, "[a]\n b = 34\n").unwrap();
            attach(&mut config, &good, ConfigLevel::LOCAL);
            attach(&mut config, &bad, ConfigLevel::GLOBAL);
            // LOCAL is copied before GLOBAL's iterator discovers the malformed file.
            std::fs::write(&bad, "[invalid\n").unwrap();
            let error = snapshot(&config, raw).unwrap_err();
            let (rc, _, failure) = error;
            let (class, message) = failure.unwrap();
            results.push((rc, class, message.replace(bad.to_str().unwrap(), "<bad>")));
            std::fs::write(&bad, "[a]\n b = 56\n").unwrap();
            let snap = snapshot(&config, raw).unwrap();
            drop(config);
            assert_eq!(get(&snap, c"a.b", raw).1, 12);
            drop(snap);
            std::fs::remove_file(good).unwrap();
            std::fs::remove_file(bad).unwrap();
        }
        assert_eq!(results[0], results[1]);
    }
}
