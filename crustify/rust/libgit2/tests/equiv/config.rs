//! Equivalence of `git2/config.h`'s [`Config`] owners with the `git_config` C code
//! builds with `git_config_new`, receives from `git_repository_config`, and releases
//! with `git_config_free`.
//!
//! Each config is produced twice, independently, one per side: an empty one (C:
//! `git_config_new`; Rust: [`Config::new`], and the shared form
//! [`ConfigArc::from`]), and a repository's (`git_repository_config` on its own open
//! `testrepo.git`; Rust: the [`ConfigArc`] `Repository::config` returns). C drives its
//! object through the raw calls, Rust through the safe wrappers: a lookup that misses,
//! a write without a backend, adding a file backend (each side its own file), writes
//! and reads through it, then (C alone, see below) a read after the repository is
//! freed. A shared reference cannot write, so the Rust side of the shared case writes
//! through the box before sharing it, and both sides then read. After every step both
//! record the return code, the value read or the error (class and message), and the
//! records are compared; the files each side wrote are compared at the end. C then
//! frees its config, and Rust drops its owner.
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
use libgit2::libgit2::config::config::{
    Config, ConfigArc, ConfigBox, ConfigError, ConfigLevel, ConfigRef,
};
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

fn record(rc: c_int, value: i32, error: fn() -> (c_int, String)) -> Record {
    (rc, value, (rc < 0).then(error))
}

/// A safe wrapper's result as the record C's would make: the value read (0 for a
/// write), or the code and the error the `ConfigError` carries.
fn rust_record(result: Result<i32, ConfigError>) -> Record {
    match result {
        Ok(value) => (0, value, None),
        Err(e) => (
            e.raw_code(),
            0,
            Some((e.raw_class(), e.message().to_string_lossy().into_owned())),
        ),
    }
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

/// The names the read phase looks up, after the writes.
const READS: [&CStr; 3] = [c"crustify.value", c"crustify.other", c"crustify.missing"];

/// `git_config_get_int32` of `name` on C's `config`.
fn c_get(config: *mut ffi::git_config, name: &CStr) -> Record {
    let mut value = 0;
    // SAFETY: `config` is live for the call; `value` is writable, `name` NUL-terminated.
    let rc = unsafe { ffi::git_config_get_int32(&mut value, config, name.as_ptr()) };
    record(rc, value, c_error)
}

/// The write sequence through C on `config`; `file` names C's own backend file.
fn c_steps(config: *mut ffi::git_config, file: &Path) -> Vec<Record> {
    let file = CString::new(file.to_str().unwrap()).unwrap();
    let get = |name: &CStr| c_get(config, name);
    let set = |name: &CStr, value: i32| {
        // SAFETY: `config` is live for the call; `name` is NUL-terminated.
        let rc = unsafe { ffi::git_config_set_int32(config, name.as_ptr(), value) };
        record(rc, 0, c_error)
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
    out.push(record(rc, 0, c_error));
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

/// The same write sequence through the safe wrappers on the sole `config`.
fn rust_steps(config: &mut ConfigBox<'_>, file: &Path) -> Vec<Record> {
    let file = CString::new(file.to_str().unwrap()).unwrap();
    let get = |config: &ConfigBox<'_>, name: &CStr| rust_record(config.as_ref().get_i32(name));
    let set = |config: &mut ConfigBox<'_>, name: &CStr, value: i32| {
        rust_record(config.as_mut().set_i32(name, value).map(|()| 0))
    };
    let mut out = vec![
        get(config, c"crustify.value"),
        set(config, c"crustify.value", 1),
    ];
    let added = config
        .as_mut()
        .add_file(&file, ConfigLevel::LOCAL, None, false);
    out.push(rust_record(added.map(|()| 0)));
    out.extend([
        set(config, c"crustify.value", 7),
        get(config, c"crustify.value"),
        set(config, c"crustify.value", -3),
        set(config, c"crustify.other", 11),
        get(config, c"crustify.value"),
        get(config, c"crustify.other"),
        get(config, c"crustify.missing"),
        set(config, c"invalid", 1),
    ]);
    out
}

/// The read phase through the safe wrappers on any handle.
fn rust_reads(config: ConfigRef<'_, '_>, names: &[&CStr]) -> Vec<Record> {
    names
        .iter()
        .map(|name| rust_record(config.get_i32(name)))
        .collect()
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
    let mut rust_config = rust_config.unwrap();

    let c_records = c_steps(c_config, &c_file);
    let rust_records = rust_steps(&mut rust_config, &rust_file);
    assert_eq!(c_records, rust_records);
    assert_eq!(c_records[4], (0, 7, None), "the backend took the write");
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
    let mut rust_box = Config::new().unwrap();
    assert_eq!(
        c_steps(c_config, &c_file),
        rust_steps(&mut rust_box, &rust_file)
    );
    let rust_config = ConfigArc::from(rust_box);
    let c_reads: Vec<_> = READS.iter().map(|name| c_get(c_config, name)).collect();
    assert_eq!(c_reads, rust_reads(rust_config.as_ref(), &READS));
    assert_eq!(
        c_reads[1],
        (0, 11, None),
        "shared, the config reads its backend"
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
    use libgit2::libgit2::repository::repository::Repository;
    init();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git\0"
    );
    let mut c_repo = ptr::null_mut();
    // SAFETY: `c_repo` receives the repository; `path` is NUL-terminated.
    let rc = unsafe {
        ffi::git_repository_open_ext(
            &mut c_repo,
            path.as_ptr().cast(),
            ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
            ptr::null(),
        )
    };
    assert_eq!(rc, 0);
    let rust_repo = Repository::open(CStr::from_bytes_with_nul(path.as_bytes()).unwrap());
    let rust_repo = rust_repo.unwrap();
    let mut c_config = ptr::null_mut();
    // SAFETY: `c_config` is a writable slot, `c_repo` open.
    let c_rc = unsafe { ffi::git_repository_config(&mut c_config, c_repo) };
    let rust_config = rust_repo.as_ref().config();
    assert_eq!(c_rc == 0, rust_config.is_ok());
    let rust_config = rust_config.unwrap();

    let names = [
        c"core.repositoryformatversion",
        c"core.bare",
        c"core.missing",
    ];
    let reads = |config| names.map(|name| c_get(config, name)).to_vec();
    let before = reads(c_config);
    assert_eq!(before, rust_reads(rust_config.as_ref(), &names));
    // Documented divergence: C lets the caller's reference outlive the repository and
    // read on (the file backends still point at the freed repository, which only a
    // reparse reaches); a `ConfigArc<'r>` borrows the repository and goes first.
    drop(rust_config);
    drop(rust_repo);
    // SAFETY: the repository is open and freed once; the C config outlives it.
    unsafe { ffi::git_repository_free(c_repo) };
    let after = reads(c_config);
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
    use libgit2::libgit2::config::config::{ConfigLevel, ConfigMap, ConfigRef, MapMatch};
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

    /// One `git_config_lookup_map_value` observation: code, output, error.
    fn lookup(maps: *const ffi::git_configmap, n: usize, input: &CStr) -> Record {
        let mut out = -999;
        // SAFETY: `maps` addresses `n` initialized records whose String rules
        // have live strings; the input is NUL-terminated, the output writable.
        let rc = unsafe { ffi::git_config_lookup_map_value(&mut out, maps, n, input.as_ptr()) };
        record(rc, out, c_error)
    }

    /// Tables of records built and edited through `ConfigMap` and of C records
    /// built by hand, with independent strings, map every input alike through
    /// C's dispatcher, over every prefix, after each edit.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn rust_map_tables_dispatch_like_c_tables() {
        init();
        static RUST_TABLE: [ConfigMap<'static>; 4] = [
            ConfigMap::new(MapMatch::False, 10),
            ConfigMap::new(MapMatch::True, 11),
            ConfigMap::new(MapMatch::String(c"input"), 12),
            ConfigMap::new(MapMatch::Int32, 13),
        ];
        let (c_input, c_later) = (
            CString::new("input").unwrap(),
            CString::new("Later").unwrap(),
        );
        let rust_later = CString::new("Later").unwrap();
        let mut c_table = [
            (ffi::GIT_CONFIGMAP_FALSE, ptr::null(), 10),
            (ffi::GIT_CONFIGMAP_TRUE, ptr::null(), 11),
            (ffi::GIT_CONFIGMAP_STRING, c_input.as_ptr(), 12),
            (ffi::GIT_CONFIGMAP_INT32, ptr::null(), 13),
        ]
        .map(|(type_, str_match, map_value)| ffi::git_configmap {
            type_,
            str_match,
            map_value,
        });
        let mut rust_table = RUST_TABLE.to_vec();
        let inputs = [
            c"no", c"on", c"INPUT", c"later", c"2k", c"-0x10", c"", c"neither",
        ];
        let compare = |step: &str, rust: &[ConfigMap<'_>], c: &[ffi::git_configmap]| {
            for (r, c) in rust.iter().zip(c) {
                assert_eq!(r.as_ref().map_type().raw(), c.type_, "{step}");
                assert_eq!(r.as_ref().map_value(), c.map_value, "{step}");
                assert_eq!(
                    r.as_ref()
                        .str_match()
                        .map(CStr::to_bytes)
                        .map(<[u8]>::to_vec),
                    bytes(c.str_match),
                    "{step}"
                );
            }
            for n in 0..=c.len() {
                for input in inputs {
                    // `ConfigMap` is transparent over `git_configmap`.
                    let rust_maps = rust.as_ptr().cast::<ffi::git_configmap>();
                    assert_eq!(
                        lookup(rust_maps, n, input),
                        lookup(c.as_ptr(), n, input),
                        "{step}: {input:?} over {n} records"
                    );
                }
            }
        };
        compare("constant table", &rust_table, &c_table);
        rust_table[2].as_mut().set_str_match(&rust_later);
        c_table[2].str_match = c_later.as_ptr();
        compare("retargeted string rule", &rust_table, &c_table);
        let mut first = rust_table[0].as_mut();
        first.set_match(MapMatch::String(c""));
        first.set_map_value(-1);
        c_table[0] = ffi::git_configmap {
            type_: ffi::GIT_CONFIGMAP_STRING,
            str_match: c"".as_ptr(),
            map_value: -1,
        };
        compare("empty string rule first", &rust_table, &c_table);
        rust_table[3].as_mut().set_match(MapMatch::True);
        c_table[3].type_ = ffi::GIT_CONFIGMAP_TRUE;
        compare("integer rule turned boolean", &rust_table, &c_table);
        rust_table[1].as_mut().set_str_match(&rust_later);
        c_table[1].str_match = c_later.as_ptr();
        compare("ignored string on a boolean rule", &rust_table, &c_table);
    }

    /// An entry's fields as both sides see them: name, value, backend type,
    /// origin file name (each side reads its own file), level, include depth.
    type Fields = (
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<std::ffi::OsString>,
        i32,
        u32,
    );

    fn file_name(origin: Option<Vec<u8>>) -> Option<std::ffi::OsString> {
        use std::os::unix::ffi::OsStringExt;
        origin.map(|o| {
            Path::new(&std::ffi::OsString::from_vec(o))
                .file_name()
                .unwrap()
                .into()
        })
    }

    fn c_fields(entry: *const ffi::git_config_entry) -> Fields {
        // SAFETY: `entry` is a live acquired entry, whose strings stay live and
        // unchanged until it is released; fields are read by projection.
        unsafe {
            (
                bytes(addr_of!((*entry).name).read()),
                bytes(addr_of!((*entry).value).read()),
                bytes(addr_of!((*entry).backend_type).read()),
                file_name(bytes(addr_of!((*entry).origin_path).read())),
                addr_of!((*entry).level).read(),
                addr_of!((*entry).include_depth).read(),
            )
        }
    }

    fn rust_fields(entry: libgit2::libgit2::config::config::EntryRef<'_>) -> Fields {
        let owned = |s: Option<&CStr>| s.map(|s| s.to_bytes().to_vec());
        (
            owned(entry.name()),
            owned(entry.value()),
            owned(entry.backend_type()),
            file_name(owned(entry.origin_path())),
            entry.level().raw(),
            entry.include_depth(),
        )
    }

    type Lookup = Result<Fields, (c_int, (c_int, String))>;

    /// Entry selection across two levels, multivars, a write that replaces the
    /// file's list, a snapshot, level and HIGHEST views, and a lower file broken
    /// then repaired matches C step by step, and every acquired entry still
    /// reads the same after both configs are gone.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn entry_selection_sequences_match_c() {
        init();
        let dir = std::env::temp_dir().join(format!(
            "crustify-equiv-entry-levels-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let files = |side: &str| {
            let side_dir = dir.join(side);
            std::fs::create_dir_all(&side_dir).unwrap();
            let low = side_dir.join("global");
            let high = side_dir.join("local");
            std::fs::write(&low, b"[test]\n key = low\n shared = g\n").unwrap();
            std::fs::write(&high, b"[test]\n key = high\n[multi]\n v = a\n v = b\n").unwrap();
            [low, high].map(|p| CString::new(p.to_str().unwrap()).unwrap())
        };
        let [c_low, c_high] = files("c");
        let [rust_low, rust_high] = files("rust");
        let levels = [ConfigLevel::GLOBAL, ConfigLevel::LOCAL];

        let mut c_cfg = ptr::null_mut();
        // SAFETY: writable slot; the runtime is pinned by `init`.
        assert_eq!(unsafe { ffi::git_config_new(&mut c_cfg) }, 0);
        for (path, level) in [&c_low, &c_high].into_iter().zip(levels) {
            // SAFETY: live config; C copies the path; no repository, no force.
            let rc = unsafe {
                ffi::git_config_add_file_ondisk(c_cfg, path.as_ptr(), level.raw(), ptr::null(), 0)
            };
            assert_eq!(rc, 0);
        }
        let mut rust_cfg = Config::new().unwrap();
        for (path, level) in [&rust_low, &rust_high].into_iter().zip(levels) {
            rust_cfg
                .as_mut()
                .add_file(path, level, None, false)
                .unwrap();
        }

        let mut c_kept = Vec::new();
        let mut rust_kept = Vec::new();
        let mut c_get = |cfg: *mut ffi::git_config, key: &CStr| -> Lookup {
            let mut entry = ptr::null_mut();
            // SAFETY: live config, writable slot, NUL-terminated key.
            let rc = unsafe { ffi::git_config_get_entry(&mut entry, cfg, key.as_ptr()) };
            if rc < 0 {
                return Err((rc, c_error()));
            }
            c_kept.push(entry);
            Ok(c_fields(entry))
        };
        let mut rust_get = |cfg: ConfigRef<'_, '_>, key: &CStr| -> Lookup {
            let entry = cfg.entry(key).map_err(|e| {
                let message = e.message().to_string_lossy().into_owned();
                (e.raw_code(), (e.raw_class(), message))
            })?;
            let fields = rust_fields(entry.as_ref());
            rust_kept.push(entry);
            Ok(fields)
        };
        let step = |what: &str, c: Lookup, rust: Lookup| assert_eq!(c, rust, "{what}");

        for key in [
            c"test.key",
            c"TEST.shared",
            c"multi.v",
            c"test.missing",
            c"bad",
        ] {
            step(
                "initial lookups",
                c_get(c_cfg, key),
                rust_get(rust_cfg.as_ref(), key),
            );
        }
        let (key, value) = (c"test.key", c"new");
        // SAFETY: live config; both strings are borrowed for the call.
        let rc = unsafe { ffi::git_config_set_string(c_cfg, key.as_ptr(), value.as_ptr()) };
        let status = rust_cfg
            .as_mut()
            .set_string(c"test.key", c"new")
            .map_err(|e| e.raw_code());
        assert_eq!(status, if rc < 0 { Err(rc) } else { Ok(()) });
        let written = c_get(c_cfg, c"test.key");
        step(
            "after the write",
            written.clone(),
            rust_get(rust_cfg.as_ref(), c"test.key"),
        );
        assert_eq!(written.unwrap().4, ConfigLevel::LOCAL.raw());

        let mut c_snapshot = ptr::null_mut();
        // SAFETY: live source config and writable slot.
        let rc = unsafe { ffi::git_config_snapshot(&mut c_snapshot, c_cfg) };
        assert_eq!(rc, 0);
        let rust_snapshot = rust_cfg.as_ref().snapshot().unwrap();
        for key in [c"test.key", c"test.shared", c"multi.v"] {
            step(
                "snapshot",
                c_get(c_snapshot, key),
                rust_get(rust_snapshot.as_ref(), key),
            );
        }

        let mut c_view = ptr::null_mut();
        let global = ConfigLevel::GLOBAL.raw();
        // SAFETY: live parent config and writable slot.
        let rc = unsafe { ffi::git_config_open_level(&mut c_view, c_cfg, global) };
        assert_eq!(rc, 0);
        {
            let mut parent = rust_cfg.as_mut();
            let rust_view = parent.open_level(ConfigLevel::GLOBAL).unwrap();
            for key in [c"test.key", c"multi.v"] {
                step(
                    "global view",
                    c_get(c_view, key),
                    rust_get(rust_view.as_ref(), key),
                );
            }
        }

        // A HIGHEST view holds the top backend under the selector level.
        let mut c_top = ptr::null_mut();
        let highest = ConfigLevel::HIGHEST.raw();
        // SAFETY: live parent config and writable slot.
        let rc = unsafe { ffi::git_config_open_level(&mut c_top, c_cfg, highest) };
        assert_eq!(rc, 0);
        {
            let mut parent = rust_cfg.as_mut();
            let rust_top = parent.open_level(ConfigLevel::HIGHEST).unwrap();
            for key in [c"test.key", c"test.shared"] {
                step(
                    "highest view",
                    c_get(c_top, key),
                    rust_get(rust_top.as_ref(), key),
                );
            }
        }

        // The global file is rewritten into something libgit2 cannot parse. C's
        // lookup of a key the local file holds never rereads it, so it succeeds,
        // and so does Rust's, which rereads it. C reaches a backend that fails to
        // reread only through an unwritten entry pointer (config.c:811), so no
        // step here asks C to; see `unit_tests::entry_reports_failed_rereads`.
        // Once repaired, both read the new global file.
        let global = |side: &str| dir.join(side).join("global");
        std::fs::write(global("c"), b"[test\n shared = broken\n").unwrap();
        std::fs::write(global("rust"), b"[test\n shared = broken\n").unwrap();
        let key = c"test.key";
        step(
            "broken lower file",
            c_get(c_cfg, key),
            rust_get(rust_cfg.as_ref(), key),
        );
        std::fs::write(global("c"), b"[test]\n shared = repaired-value\n").unwrap();
        std::fs::write(global("rust"), b"[test]\n shared = repaired-value\n").unwrap();
        for key in [c"test.shared", c"test.key"] {
            step(
                "repaired lower file",
                c_get(c_cfg, key),
                rust_get(rust_cfg.as_ref(), key),
            );
        }

        // SAFETY: each config reference is released once; entries keep their lists.
        unsafe {
            ffi::git_config_free(c_top);
            ffi::git_config_free(c_view);
            ffi::git_config_free(c_snapshot);
            ffi::git_config_free(c_cfg);
        }
        drop(rust_snapshot);
        drop(rust_cfg);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(c_kept.len(), rust_kept.len());
        for (c_entry, entry) in c_kept.into_iter().zip(rust_kept) {
            assert_eq!(c_fields(c_entry), rust_fields(entry.as_ref()));
            // SAFETY: releases exactly this acquired C entry reference.
            unsafe { ffi::git_config_entry_free(c_entry) };
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
        let mut r = Config::open(&rp).unwrap();
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
                safe(r.as_mut().add_file(&rp, ConfigLevel::LOCAL, None, force))
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
        let (name, value) = (c"a.txstring", c"staged");
        // SAFETY: live locked config and NUL-terminated inputs borrowed for call;
        // the value is copied by the C writer.
        let rc = unsafe { ffi::git_config_set_string(c.0, name.as_ptr(), value.as_ptr()) };
        let c_result = raw(rc, ());
        clear();
        assert_eq!(c_result, safe(tx.set_string(name, value)));
        clear();
        // SAFETY: as above; an invalid name fails before anything is staged.
        let rc = unsafe { ffi::git_config_set_string(c.0, c"invalid".as_ptr(), c"x".as_ptr()) };
        let c_result = raw(rc, ());
        clear();
        assert_eq!(c_result, safe(tx.set_string(c"invalid", c"x")));
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
        let (mut out, repo) = (ptr::null_mut(), repositories[0].as_c_ptr());
        // SAFETY: live independently owned repository and writable output slot.
        // A successful snapshot is a fresh config released by Reference's Drop.
        let rc = unsafe { ffi::git_repository_config_snapshot(&mut out, repo) };
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
        // C returns the code of a delete without a writer but sets no error, so the
        // thread keeps whatever it had (here, after the clear, "no error"). The
        // wrappers set their own error there; only the code is C's.
        macro_rules! readonly_delete {
            ($raw:expr, $safe:expr) => {{
                clear();
                let c_result = raw($raw, ());
                clear();
                let r_result = safe($safe);
                assert_eq!(
                    c_result,
                    Err((
                        ffi::GIT_EREADONLY,
                        ffi::GIT_ERROR_NONE as i32,
                        b"no error".to_vec()
                    ))
                );
                assert_eq!(
                    r_result,
                    Err((
                        ffi::GIT_EREADONLY,
                        ffi::GIT_ERROR_CONFIG as i32,
                        b"cannot delete 'a.flag': the configuration is read-only".to_vec()
                    ))
                );
            }};
        }
        readonly_delete!(
            // SAFETY: live readonly config and borrowed NUL name, nothing retained.
            unsafe { ffi::git_config_delete_entry(c.0, c"a.flag".as_ptr()) },
            r.as_mut().delete_entry(c"a.flag")
        );
        readonly_delete!(
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

    /// Views of levels a config lacks fail as C's do, through the shared and the
    /// exclusive handle: `open_level` with C's not-found error, and `open_global` with
    /// the error of its `GLOBAL` fallback once `XDG` is missing too.
    #[test]
    fn missing_level_views_match_c() {
        let (cfile, rfile) = (
            scratch("missing-level", "c"),
            scratch("missing-level", "rust"),
        );
        let cp = CString::new(cfile.to_str().unwrap()).unwrap();
        let rp = CString::new(rfile.to_str().unwrap()).unwrap();
        let c = Reference::open(&cp);
        let mut r = Config::open(&rp).unwrap();
        let failure = |e: ConfigError| safe::<()>(Err(e)).unwrap_err();
        for level in [
            ConfigLevel::GLOBAL,
            ConfigLevel::APP,
            ConfigLevel::from_raw(-7),
        ] {
            clear();
            let cv = c.level(level, false).map(drop);
            clear();
            let rv = r.as_ref().open_level(level).map(drop).map_err(failure);
            assert_eq!(cv, rv);
            assert_eq!(cv.unwrap_err().0, ffi::GIT_ENOTFOUND);
            clear();
            let rv = r.as_mut().open_level(level).map(drop).map_err(failure);
            assert_eq!(c.level(level, false).map(drop), rv);
        }
        clear();
        let cv = c.level(ConfigLevel::GLOBAL, true).map(drop);
        clear();
        assert_eq!(cv, r.as_ref().open_global().map(drop).map_err(failure));
        clear();
        assert_eq!(cv, r.as_mut().open_global().map(drop).map_err(failure));
        assert_eq!(cv.unwrap_err().0, ffi::GIT_ENOTFOUND);
        drop((c, r));
        let _ = (std::fs::remove_file(cfile), std::fs::remove_file(rfile));
    }

    const SEARCH_CHILD: &str = "CRUSTIFY_EQUIV_CONFIG_SEARCH_CHILD";

    /// In a process whose `$HOME` holds a malformed `.gitconfig` and whose system search
    /// path is an empty directory, the search routines and `open_default` fail and
    /// succeed as C's do: `open_default` with the parse error of the global file (C
    /// frees the config it built), `find_system` and `find_xdg` with not found, and
    /// `find_global` with the malformed file's path. libgit2 reads `$HOME` when it
    /// initialises, so the scenario runs in a child.
    #[test]
    fn isolated_search_and_default_failures_match_c() {
        let dir = std::env::temp_dir().join(format!(
            "crustify-equiv-config-search-{}",
            std::process::id()
        ));
        if std::env::var_os(SEARCH_CHILD).is_some() {
            init();
            let system = CString::new(dir.join("system").to_str().unwrap()).unwrap();
            // SAFETY: `GIT_OPT_SET_SEARCH_PATH` reads an `int` level and a
            // NUL-terminated path, which libgit2 copies; this child runs one thread.
            let rc = unsafe {
                ffi::git_libgit2_opts(
                    ffi::GIT_OPT_SET_SEARCH_PATH as c_int,
                    ffi::GIT_CONFIG_LEVEL_SYSTEM,
                    system.as_ptr(),
                )
            };
            assert_eq!(rc, 0);
            for (raw_find, safe_find) in [
                (
                    ffi::git_config_find_global as unsafe extern "C" fn(*mut ffi::git_buf) -> i32,
                    find_global as fn() -> Result<BufVal, ConfigError>,
                ),
                (ffi::git_config_find_xdg, find_xdg),
                (ffi::git_config_find_system, find_system),
            ] {
                clear();
                // SAFETY: out is initialized exclusive buffer storage.
                let c = buffer(|out| unsafe { raw_find(out) });
                clear();
                assert_eq!(c, bytes(safe_find()));
            }
            clear();
            let mut out = ptr::null_mut();
            // SAFETY: writable output slot; on failure C frees what it built.
            let rc = unsafe { ffi::git_config_open_default(&mut out) };
            let c = raw(rc, out).map(Reference);
            clear();
            let r = safe(Config::open_default());
            let (c, r) = (c.map(drop), r.map(drop));
            assert_eq!(c, r);
            let (code, class, _) = c.unwrap_err();
            assert_eq!((code, class), (-1, ffi::GIT_ERROR_CONFIG as c_int));
            return;
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("system")).unwrap();
        std::fs::write(dir.join(".gitconfig"), b"[unfinished\n").unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "config::file_access::isolated_search_and_default_failures_match_c",
                "--exact",
                "--nocapture",
            ])
            .env("HOME", &dir)
            .env("XDG_CONFIG_HOME", dir.join("xdg"))
            .env(SEARCH_CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        std::fs::remove_dir_all(dir).unwrap();
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
        config.as_mut().add_file(&file, level, None, false).unwrap();
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
