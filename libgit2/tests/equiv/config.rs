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
    use libgit2::libgit2::config::config::{
        ConfigLevel, ConfigMap, ConfigRef, MapMatch, lookup_map_value,
    };
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
        record(rc, if rc < 0 { 0 } else { out }, c_error)
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
                    assert_eq!(
                        rust_record(lookup_map_value(&rust[..n], Some(input))),
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

mod mapping_callbacks {
    use super::*;
    use libgit2::libgit2::config::config::{
        ConfigMap, EntryRef, ForeachCallback, MapMatch, lookup_map_value,
    };
    use libgit2::libgit2::util::errors::ErrorClass;
    use libgit2::libgit2::util::sys::errors::{clear, set};
    use std::ffi::c_void;
    use std::num::NonZero;
    use std::ops::ControlFlow;

    /// Includes last-error state even on successful speculative mapping.
    fn observe(result: Result<i32, ConfigError>) -> (Record, (c_int, Vec<u8>)) {
        let record = rust_record(result);
        let error = last();
        (
            record,
            (error.raw_class(), error.message().to_bytes().to_vec()),
        )
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn map_lookup_matches_c_after_each_table_edit() {
        init();
        let rust_text = CString::new("input").unwrap();
        let c_text = CString::new("input").unwrap();
        let mut maps = [
            ConfigMap::new(MapMatch::False, i32::MIN),
            ConfigMap::new(MapMatch::True, i32::MAX),
            ConfigMap::new(MapMatch::String(&rust_text), 17),
            ConfigMap::new(MapMatch::Int32, -999),
        ];
        let mut raw = [
            ffi::git_configmap {
                type_: ffi::GIT_CONFIGMAP_FALSE,
                str_match: ptr::null(),
                map_value: i32::MIN,
            },
            ffi::git_configmap {
                type_: ffi::GIT_CONFIGMAP_TRUE,
                str_match: ptr::null(),
                map_value: i32::MAX,
            },
            ffi::git_configmap {
                type_: ffi::GIT_CONFIGMAP_STRING,
                str_match: c_text.as_ptr(),
                map_value: 17,
            },
            ffi::git_configmap {
                type_: ffi::GIT_CONFIGMAP_INT32,
                str_match: ptr::null(),
                map_value: -999,
            },
        ];
        for step in 0..3 {
            for n in 0..=maps.len() {
                for value in [
                    None,
                    Some(c""),
                    Some(c"no"),
                    Some(c"on"),
                    Some(c"INPUT"),
                    Some(c"2k"),
                    Some(c"2147483648"),
                    Some(c"garbage"),
                    Some(c"\xff"),
                    Some(c"1kb"),
                ] {
                    set(ErrorClass::Odb, c"previous");
                    let safe = observe(lookup_map_value(&maps[..n], value));
                    set(ErrorClass::Odb, c"previous");
                    let mut out = 0;
                    // SAFETY: independently owned initialized C table and live
                    // immutable strings, exact prefix count, writable output.
                    let rc = unsafe {
                        ffi::git_config_lookup_map_value(
                            &mut out,
                            raw.as_ptr(),
                            n,
                            value.map_or(ptr::null(), CStr::as_ptr),
                        )
                    };
                    let record = record(rc, if rc < 0 { 0 } else { out }, c_error);
                    let error = last();
                    assert_eq!(
                        safe,
                        (
                            record,
                            (error.raw_class(), error.message().to_bytes().to_vec())
                        ),
                        "step {step}, prefix {n}, value {value:?}"
                    );
                }
            }
            match step {
                0 => {
                    maps[0].as_mut().set_match(MapMatch::String(c""));
                    raw[0].type_ = ffi::GIT_CONFIGMAP_STRING;
                    raw[0].str_match = c"".as_ptr();
                }
                _ => {
                    maps[3].as_mut().set_map_value(42);
                    raw[3].map_value = 42;
                }
            }
        }
        clear();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mapped_config_matches_c_across_writes_snapshots_and_errors() {
        init();
        let rust_path = scratch("mapped", "rust");
        let c_path = scratch("mapped", "c");
        for path in [&rust_path, &c_path] {
            std::fs::write(path, b"[a]\n b = input\n flag\n").unwrap();
        }
        let rust_config =
            Config::open(&CString::new(rust_path.to_str().unwrap()).unwrap()).unwrap();
        let mut raw_config = ptr::null_mut();
        let c_name = CString::new(c_path.to_str().unwrap()).unwrap();
        // SAFETY: exclusive output slot and live NUL path; C creates one owner.
        let rc = unsafe { ffi::git_config_open_ondisk(&mut raw_config, c_name.as_ptr()) };
        assert_eq!(rc, 0);
        let maps = [
            ConfigMap::new(MapMatch::String(c"input"), 7),
            ConfigMap::new(MapMatch::True, 8),
            ConfigMap::new(MapMatch::Int32, 9),
        ];
        let raw_maps = [
            ffi::git_configmap {
                type_: ffi::GIT_CONFIGMAP_STRING,
                str_match: c"input".as_ptr(),
                map_value: 7,
            },
            ffi::git_configmap {
                type_: ffi::GIT_CONFIGMAP_TRUE,
                str_match: ptr::null(),
                map_value: 8,
            },
            ffi::git_configmap {
                type_: ffi::GIT_CONFIGMAP_INT32,
                str_match: ptr::null(),
                map_value: 9,
            },
        ];
        for content in [
            b"[a]\n b = input\n flag\n".as_slice(),
            b"[a]\n b = 2k\n flag\n extra = 1\n",
            b"[a]\n b = invalid\n flag\n extra = longer\n",
        ] {
            for path in [&rust_path, &c_path] {
                std::fs::write(path, content).unwrap();
            }
            for name in [c"A.B", c"a.flag", c"a.missing", c"invalid", c""] {
                for n in 0..=maps.len() {
                    clear();
                    let safe = observe(rust_config.as_ref().get_mapped(name, &maps[..n]));
                    clear();
                    let mut out = 0;
                    // SAFETY: live independent C config with a valid file, name
                    // and mapping strings are NUL-terminated; exact table count.
                    let rc = unsafe {
                        ffi::git_config_get_mapped(
                            &mut out,
                            raw_config,
                            name.as_ptr(),
                            raw_maps.as_ptr(),
                            n,
                        )
                    };
                    let record = record(rc, if rc < 0 { 0 } else { out }, c_error);
                    let error = last();
                    assert_eq!(
                        safe,
                        (
                            record,
                            (error.raw_class(), error.message().to_bytes().to_vec())
                        ),
                        "{name:?}, {n}"
                    );
                }
            }
        }
        let snapshot = rust_config.as_ref().snapshot().unwrap();
        let mut c_snapshot = ptr::null_mut();
        // SAFETY: live config and writable output; successful snapshot owns one reference.
        let rc = unsafe { ffi::git_config_snapshot(&mut c_snapshot, raw_config) };
        assert_eq!(rc, 0);
        drop(rust_config);
        // SAFETY: release the owned C config once; its snapshot is independent.
        unsafe { ffi::git_config_free(raw_config) };
        assert_eq!(snapshot.as_ref().get_mapped(c"a.flag", &maps).unwrap(), 8);
        let mut out = 0;
        // SAFETY: independent live C snapshot, writable scalar, valid table/key.
        let rc = unsafe {
            ffi::git_config_get_mapped(
                &mut out,
                c_snapshot,
                c"a.flag".as_ptr(),
                raw_maps.as_ptr(),
                raw_maps.len(),
            )
        };
        assert_eq!(rc, 0);
        assert_eq!(out, 8);
        // SAFETY: releases this owned snapshot once, with no remaining borrows.
        unsafe { ffi::git_config_free(c_snapshot) };
        for path in [&rust_path, &c_path] {
            std::fs::remove_file(path).unwrap();
        }
    }

    struct Calls {
        code: c_int,
        values: Vec<Option<Vec<u8>>>,
    }
    /// # Safety
    /// entry is a live immutable C entry for this call; payload addresses a
    /// live exclusively borrowed Calls on this thread. Neither is retained.
    unsafe extern "C" fn c_callback(
        entry: *const ffi::git_config_entry,
        payload: *mut c_void,
    ) -> c_int {
        // SAFETY: test caller lends an initialized immutable entry for this call.
        let value = unsafe { core::ptr::addr_of!((*entry).value).read() };
        let value = if value.is_null() {
            None
        } else {
            // SAFETY: non-NULL entry values are immutable NUL strings live for the call.
            Some(unsafe { CStr::from_ptr(value) }.to_bytes().to_vec())
        };
        // SAFETY: payload is the exclusively borrowed Calls state of this invocation.
        let calls = unsafe { &mut *payload.cast::<Calls>() };
        calls.values.push(value);
        calls.code
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn borrowed_callback_values_and_nonzero_codes_match_c() {
        let rpath = scratch("callback", "rust");
        let cpath = scratch("callback", "c");
        for path in [&rpath, &cpath] {
            std::fs::write(path, b"[a]\n value = input\n flag\n empty =\n").unwrap();
        }
        let config = Config::open(&CString::new(rpath.to_str().unwrap()).unwrap()).unwrap();
        let mut raw_config = ptr::null_mut();
        let path = CString::new(cpath.to_str().unwrap()).unwrap();
        // SAFETY: live path and writable output; creates independently owned config.
        let rc = unsafe { ffi::git_config_open_ondisk(&mut raw_config, path.as_ptr()) };
        assert_eq!(rc, 0);
        for code in [0, 1, -7, i32::MIN, i32::MAX, ffi::GIT_ITEROVER] {
            let mut safe_values = Vec::new();
            let mut f = |entry: EntryRef<'_>| {
                safe_values.push(entry.value().map(|s| s.to_bytes().to_vec()));
                NonZero::new(code).map_or(ControlFlow::Continue(()), ControlFlow::Break)
            };
            let mut callback = ForeachCallback::new(&mut f);
            let mut calls = Calls {
                code,
                values: Vec::new(),
            };
            for key in [c"a.value", c"a.flag", c"a.empty"] {
                let entry = config.as_ref().entry(key).unwrap();
                let mut raw_entry = ptr::null_mut();
                // SAFETY: live config, key, and writable output slot; success acquires an entry.
                let rc = unsafe {
                    // Acquire only the C side's independently owned reference.
                    ffi::git_config_get_entry(&mut raw_entry, raw_config, key.as_ptr())
                };
                assert_eq!(rc, 0);
                // SAFETY: this C callback receives a live acquired entry and
                // exclusive typed payload, retaining or freeing neither.
                let c_rc = unsafe { c_callback(raw_entry, core::ptr::from_mut(&mut calls).cast()) };
                let rc = match callback.call(entry.as_ref()) {
                    ControlFlow::Continue(()) => 0,
                    ControlFlow::Break(code) => code.get(),
                };
                assert_eq!(rc, c_rc);
                assert_eq!(
                    callback.stopped().map(NonZero::get),
                    NonZero::new(code).map(NonZero::get)
                );
                // SAFETY: one acquired reference, not the callback borrow, released once.
                unsafe { ffi::git_config_entry_free(raw_entry) };
            }
            drop(callback);
            assert_eq!(safe_values, calls.values);
        }
        // SAFETY: releases independently owned raw config once.
        unsafe { ffi::git_config_free(raw_config) };
        for path in [&rpath, &cpath] {
            std::fs::remove_file(path).unwrap();
        }
    }

    /// A lookup whose key sits above a backend that can no longer reread its file
    /// spends that backend's failure on the Rust side only, then leaves the thread's
    /// previous error as C does, verbatim, whatever its class: none, an ordinary one,
    /// `GIT_ERROR_OS` (which `git_error_set` would extend with `errno`'s description)
    /// and a class libgit2 does not define. Regression for
    /// `crustify/reviews/.../config/wave-2/batch-1/equiv/spent-backend-failure-reformats-os-error`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn spent_backend_failures_keep_any_previous_error() {
        init();
        let paths = ["rust-lower", "rust-upper", "c-lower", "c-upper"]
            .map(|side| CString::new(scratch("spent", side).to_str().unwrap()).unwrap());
        for (lower, upper) in [(&paths[0], &paths[1]), (&paths[2], &paths[3])] {
            std::fs::write(lower.to_str().unwrap(), b"[a]\n other = old\n").unwrap();
            std::fs::write(upper.to_str().unwrap(), b"[a]\n b = input\n flag\n").unwrap();
        }
        let mut config = Config::open(&paths[0]).unwrap();
        config
            .as_mut()
            .add_file(&paths[1], ConfigLevel::APP, None, false)
            .unwrap();
        let mut raw = ptr::null_mut();
        // SAFETY: writable slot and live NUL path; success hands out one owned config.
        let rc = unsafe { ffi::git_config_open_ondisk(&mut raw, paths[2].as_ptr()) };
        assert_eq!(rc, 0);
        // SAFETY: live exclusive config, live NUL path, a free priority, no repository.
        let rc = unsafe {
            ffi::git_config_add_file_ondisk(
                raw,
                paths[3].as_ptr(),
                ffi::GIT_CONFIG_LEVEL_APP,
                ptr::null(),
                0,
            )
        };
        assert_eq!(rc, 0);
        let maps = [
            ConfigMap::new(MapMatch::String(c"input"), 42),
            ConfigMap::new(MapMatch::True, 7),
        ];
        let raw_maps = [
            ffi::git_configmap {
                type_: ffi::GIT_CONFIGMAP_STRING,
                str_match: c"input".as_ptr(),
                map_value: 42,
            },
            ffi::git_configmap {
                type_: ffi::GIT_CONFIGMAP_TRUE,
                str_match: ptr::null(),
                map_value: 7,
            },
        ];
        let seed = |previous: Option<(c_int, &CStr)>| match previous {
            None => clear(),
            // SAFETY: a NUL-terminated message, copied; the class is only stored.
            Some((class, message)) => unsafe {
                ffi::git_error_set_str(class, message.as_ptr());
            },
        };
        let error = || {
            let error = last();
            (error.raw_class(), error.message().to_bytes().to_vec())
        };
        // Each break changes the size, so the next reread parses the file again.
        // C never reads the lower files: both keys are found above them.
        let mut round = 0;
        let mut break_lower = || {
            round += 1;
            for lower in [&paths[0], &paths[2]] {
                let content = format!("[broken\n{}\n", "x".repeat(round));
                std::fs::write(lower.to_str().unwrap(), content).unwrap();
            }
        };
        for previous in [
            None,
            Some((ffi::GIT_ERROR_ODB as c_int, c"previous error")),
            Some((
                ffi::GIT_ERROR_OS as c_int,
                c"failed to stat '/nowhere': No such file or directory",
            )),
            Some((4242, c"a class libgit2 does not define")),
        ] {
            for name in [c"a.b", c"a.flag"] {
                break_lower();
                seed(previous);
                let entry = config.as_ref().entry(name).unwrap();
                let safe = (
                    entry.as_ref().value().map(|v| v.to_bytes().to_vec()),
                    error(),
                );
                drop(entry);
                seed(previous);
                let mut raw_entry = ptr::null_mut();
                // SAFETY: live config, NUL key, writable slot; success acquires
                // one entry reference.
                let rc = unsafe { ffi::git_config_get_entry(&mut raw_entry, raw, name.as_ptr()) };
                assert_eq!(rc, 0);
                // SAFETY: the acquired entry is live; its value is NULL or a NUL string.
                let value = unsafe { core::ptr::addr_of!((*raw_entry).value).read() };
                let value = (!value.is_null())
                    // SAFETY: non-NULL values are NUL strings live with the entry.
                    .then(|| unsafe { CStr::from_ptr(value) }.to_bytes().to_vec());
                let c = (value, error());
                // SAFETY: releases the one acquired reference.
                unsafe { ffi::git_config_entry_free(raw_entry) };
                assert_eq!(safe, c, "entry {name:?} after {previous:?}");

                break_lower();
                seed(previous);
                let safe = observe(config.as_ref().get_mapped(name, &maps));
                seed(previous);
                let mut out = 0;
                // SAFETY: live config, NUL key, writable scalar; exact table count
                // with a live string for the String rule.
                let rc = unsafe {
                    ffi::git_config_get_mapped(
                        &mut out,
                        raw,
                        name.as_ptr(),
                        raw_maps.as_ptr(),
                        raw_maps.len(),
                    )
                };
                let c = (record(rc, if rc < 0 { 0 } else { out }, c_error), error());
                assert_eq!(safe, c, "get_mapped {name:?} after {previous:?}");
            }
        }
        clear();
        drop(config);
        // SAFETY: releases the one owned C config.
        unsafe { ffi::git_config_free(raw) };
        for path in &paths {
            std::fs::remove_file(path.to_str().unwrap()).unwrap();
        }
    }
}

mod enumeration {
    use super::*;
    use libgit2::libgit2::config::config::{EntryRef, ForeachCallback};
    use libgit2::libgit2::util::errors::ErrorClass;
    use libgit2::libgit2::util::sys::errors::{clear, set};
    use std::ffi::c_void;
    use std::num::NonZero;
    use std::ops::ControlFlow;

    #[derive(Debug, PartialEq)]
    struct Fields {
        name: Option<CString>,
        value: Option<CString>,
        level: i32,
        depth: u32,
        backend: Option<CString>,
        origin: bool,
    }
    #[derive(Default)]
    struct Payload {
        entries: Vec<Fields>,
        stop: i32,
        stopped: Option<i32>,
    }
    /// # Safety
    /// C must supply a live immutable entry and an exclusive live Payload for
    /// this synchronous call, with immutable NUL-terminated non-NULL strings.
    unsafe extern "C" fn collect(entry: *const ffi::git_config_entry, payload: *mut c_void) -> i32 {
        // SAFETY: C lends an immutable initialized entry for this synchronous call.
        let entry = unsafe { entry.read() };
        let string = |p: *const std::ffi::c_char| {
            if p.is_null() {
                None
            } else {
                // SAFETY: every non-NULL entry string is immutable and NUL-terminated
                // for the callback; only owned copies survive the invocation.
                Some(unsafe { CStr::from_ptr(p) }.to_owned())
            }
        };
        // SAFETY: the caller passes an exclusive live Payload, never retained by C.
        let p = unsafe { &mut *payload.cast::<Payload>() };
        p.entries.push(Fields {
            name: string(entry.name),
            value: string(entry.value),
            level: entry.level,
            depth: entry.include_depth,
            backend: string(entry.backend_type),
            origin: !entry.origin_path.is_null(),
        });
        if p.stop != 0 {
            p.stopped = Some(p.stop);
        }
        p.stop
    }
    fn fields(e: EntryRef<'_>) -> Fields {
        Fields {
            name: e.name().map(CStr::to_owned),
            value: e.value().map(CStr::to_owned),
            level: e.level().raw(),
            depth: e.include_depth(),
            backend: e.backend_type().map(CStr::to_owned),
            origin: e.origin_path().is_some(),
        }
    }
    fn c_open(path: &CStr) -> ConfigBox<'static> {
        let mut out = ptr::null_mut();
        // SAFETY: writable result slot, immutable NUL path; success constructs one
        // sole config reference, without any borrowed repository.
        let rc = unsafe { ffi::git_config_open_ondisk(&mut out, path.as_ptr()) };
        assert_eq!(rc, 0);
        // SAFETY: successful C constructor transfers a sole initialized reference,
        // paired with ConfigFree, and no repository borrow is involved.
        unsafe { ConfigBox::from_c(out) }.unwrap()
    }
    fn error_record(e: &libgit2::libgit2::util::errors::ErrorBuf) -> (c_int, String) {
        // Each side owns its own backend files; only those fixture paths differ.
        (
            e.raw_class(),
            e.message()
                .to_string_lossy()
                .replace("-enumeration-c-", "-enumeration-side-")
                .replace("-enumeration-r-", "-enumeration-side-"),
        )
    }

    // Compare return/error state after every walk. Capture callback stops separately
    // because C conflates GIT_ITEROVER returned by a callback with normal exhaustion.
    // `regexp` is mode 2's value filter.
    fn compare(
        c: &ConfigBox<'_>,
        r: &ConfigBox<'_>,
        mode: u8,
        arg: &CStr,
        regexp: Option<&CStr>,
        stop: i32,
    ) {
        // With an earlier error, and without one: a stop then reports itself.
        for seeded in [true, false] {
            compare_seeded(c, r, mode, arg, regexp, stop, seeded);
        }
    }

    fn seed(seeded: bool) {
        clear();
        if seeded {
            set(ErrorClass::Odb, c"earlier error");
        }
    }

    fn compare_seeded(
        c: &ConfigBox<'_>,
        r: &ConfigBox<'_>,
        mode: u8,
        arg: &CStr,
        regexp: Option<&CStr>,
        stop: i32,
        seeded: bool,
    ) {
        seed(seeded);
        let mut p = Payload {
            stop,
            ..Payload::default()
        };
        // SAFETY: independent live reference config; immutable NUL arguments; the
        // payload is exclusive throughout synchronous C callbacks; C retains nothing.
        let rc = unsafe {
            match mode {
                0 => ffi::git_config_foreach(
                    c.as_c_ptr(),
                    Some(collect),
                    ptr::from_mut(&mut p).cast(),
                ),
                1 => ffi::git_config_foreach_match(
                    c.as_c_ptr(),
                    arg.as_ptr(),
                    Some(collect),
                    ptr::from_mut(&mut p).cast(),
                ),
                _ => ffi::git_config_get_multivar_foreach(
                    c.as_c_ptr(),
                    arg.as_ptr(),
                    regexp.map_or(ptr::null(), CStr::as_ptr),
                    Some(collect),
                    ptr::from_mut(&mut p).cast(),
                ),
            }
        };
        let c_last = last();
        seed(seeded);
        let mut observed = Vec::new();
        let mut visit = |e: EntryRef<'_>| {
            observed.push(fields(e));
            NonZero::new(stop).map_or(ControlFlow::Continue(()), ControlFlow::Break)
        };
        let mut cb = ForeachCallback::new(&mut visit);
        let result = match mode {
            0 => r.as_ref().foreach(&mut cb),
            1 => r.as_ref().foreach_match(Some(arg), &mut cb),
            _ => r.as_ref().foreach_multivar(arg, regexp, &mut cb),
        };
        let r_last = last();
        let r_stop = cb.stopped().map(NonZero::get);
        let r_rc = match result {
            Ok(ControlFlow::Continue(())) => 0,
            Ok(ControlFlow::Break(code)) if code.get() == ffi::GIT_ITEROVER => 0,
            Ok(ControlFlow::Break(code)) => code.get(),
            Err(e) => {
                assert_eq!(error_record(e.error()), error_record(&c_last));
                e.raw_code()
            }
        };
        drop(cb);
        assert_eq!(
            (r_rc, r_stop, observed, error_record(&r_last)),
            (rc, p.stopped, p.entries, error_record(&c_last)),
            "mode {mode}, arg {arg:?}, regexp {regexp:?}, stop {stop}, seeded {seeded}"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_sequences_match_c_callbacks_errors_and_priority() {
        init();
        let paths: Vec<_> = ["c-low", "r-low", "c-high", "r-high"]
            .map(|side| scratch("enumeration", side))
            .into();
        let text = b"[A]\n m = one\n m = two\n flag\n empty =\n bytes = \xff\n[A \"SubCase\"]\n Key = upper\n";
        for path in &paths {
            std::fs::write(path, text).unwrap();
        }
        let names: Vec<_> = paths
            .iter()
            .map(|p| CString::new(p.to_str().unwrap()).unwrap())
            .collect();
        let c = c_open(&names[0]);
        let mut r = Config::open(&names[1]).unwrap();
        for phase in 0..3 {
            for (mode, arg) in [
                (0, c""),
                (1, c""),
                (1, c"^a\\.m$"),
                (1, c"SubCase"),
                (1, c"subcase"),
                (1, c"["),
                (2, c"A.M"),
                (2, c"a.flag"),
                (2, c"missing.key"),
                (2, c"invalid"),
            ] {
                for stop in [0, 23, -47, ffi::GIT_EUSER, ffi::GIT_ITEROVER] {
                    compare(&c, &r, mode, arg, None, stop);
                }
            }
            if phase == 0 {
                // SAFETY: live C config and immutable NUL file path; C copies it,
                // without a repository borrow. No callback or entry is active.
                let rc = unsafe {
                    ffi::git_config_add_file_ondisk(
                        c.as_c_ptr(),
                        names[2].as_ptr(),
                        ffi::GIT_CONFIG_LEVEL_APP,
                        ptr::null(),
                        0,
                    )
                };
                assert_eq!(rc, 0);
                r.as_mut()
                    .add_file(&names[3], ConfigLevel::APP, None, false)
                    .unwrap();
            } else if phase == 1 {
                // Independent mutable state transitions, followed by the same walks.
                // SAFETY: sole live C config, immutable strings, no active entry borrow.
                let rc = unsafe {
                    ffi::git_config_set_string(c.as_c_ptr(), c"a.new".as_ptr(), c"new".as_ptr())
                };
                assert_eq!(rc, 0);
                r.as_mut().set_string(c"a.new", c"new").unwrap();
                // Exercise refresh-error propagation without C's defective entry getter.
                for path in &paths[..2] {
                    std::fs::write(path, b"[broken\nlonger malformed configuration contents\n")
                        .unwrap();
                }
            }
        }
        // Both configs/backends are released by the proven common policy after use.
        drop(c);
        drop(r);
        for path in paths {
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_refresh_errors_and_recovery_match_c() {
        init();
        for mode in 0..3 {
            let paths = [
                scratch("enumeration", &format!("c-refresh-{mode}")),
                scratch("enumeration", &format!("r-refresh-{mode}")),
            ];
            for path in &paths {
                std::fs::write(path, b"[a]\n m = old\n").unwrap();
            }
            let names = paths
                .each_ref()
                .map(|p| CString::new(p.to_str().unwrap()).unwrap());
            let c = c_open(&names[0]);
            let r = Config::open(&names[1]).unwrap();
            for text in [
                b"[broken\na longer invalid backend file\n".as_slice(),
                b"[a]\n m = repaired value longer than the invalid backend contents\n".as_slice(),
            ] {
                for path in &paths {
                    std::fs::write(path, text).unwrap();
                }
                compare(&c, &r, mode, c"a.m", None, 0);
            }
            drop(c);
            drop(r);
            for path in paths {
                std::fs::remove_file(path).unwrap();
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_value_filters_match_c() {
        init();
        // C's own value filter calls strlen(NULL) on a valueless value of the walked
        // name, so the C reference never filters a.flag; see ub_safe for that case.
        let low = b"[A]\n m = one\n m = two\n m =\n m = \xff\n flag\n[A \"Sub\"]\n m = sub\n";
        let high = b"[a]\n m = three\n other = x\n";
        let paths: Vec<_> = [
            "c-filter-low",
            "r-filter-low",
            "c-filter-high",
            "r-filter-high",
        ]
        .map(|side| scratch("enumeration", side))
        .into();
        for (path, text) in paths.iter().zip([low.as_slice(), low, high, high]) {
            std::fs::write(path, text).unwrap();
        }
        let names: Vec<_> = paths
            .iter()
            .map(|p| CString::new(p.to_str().unwrap()).unwrap())
            .collect();
        let c = c_open(&names[0]);
        let mut r = Config::open(&names[1]).unwrap();
        // An invalid pattern for a valid name leaks in C: see the test below.
        let walks: [(&CStr, &CStr); 11] = [
            (c"a.m", c"^t"),
            (c"A.M", c"o"),
            (c"a.m", c"^$"),
            (c"a.m", c""),
            (c"a.m", c"\xff"),
            (c"a.m", c"nomatch"),
            (c"invalid", c"["),
            (c"missing.key", c"."),
            (c"a.Sub.m", c"sub"),
            (c"a.sub.m", c"sub"),
            (c"a.other", c"(*UTF)x"),
        ];
        for phase in 0..4 {
            for (name, regexp) in walks {
                for stop in [0, 23, -47, ffi::GIT_EUSER, ffi::GIT_ITEROVER] {
                    compare(&c, &r, 2, name, Some(regexp), stop);
                }
            }
            match phase {
                0 => {
                    // SAFETY: live C config and immutable NUL file path; C copies it,
                    // without a repository borrow. No callback or entry is active.
                    let rc = unsafe {
                        ffi::git_config_add_file_ondisk(
                            c.as_c_ptr(),
                            names[2].as_ptr(),
                            ffi::GIT_CONFIG_LEVEL_APP,
                            ptr::null(),
                            0,
                        )
                    };
                    assert_eq!(rc, 0);
                    r.as_mut()
                        .add_file(&names[3], ConfigLevel::APP, None, false)
                        .unwrap();
                }
                1 => {
                    // SAFETY: sole live C config, immutable strings, no active entry.
                    let rc = unsafe {
                        ffi::git_config_set_multivar(
                            c.as_c_ptr(),
                            c"a.m".as_ptr(),
                            c"^three$".as_ptr(),
                            c"tri".as_ptr(),
                        )
                    };
                    assert_eq!(rc, 0);
                    r.as_mut().set_multivar(c"a.m", c"^three$", c"tri").unwrap();
                }
                2 => {
                    for path in &paths[..2] {
                        std::fs::write(path, b"[broken\nlonger malformed configuration contents\n")
                            .unwrap();
                    }
                }
                _ => {}
            }
        }
        drop(c);
        drop(r);
        for path in paths {
            std::fs::remove_file(path).unwrap();
        }
    }

    /// C's reference leaks the normalized name when a valid name's value pattern
    /// fails to compile (`config.c:1159`), so this comparison runs in a child without
    /// leak detection. The wrapper's own leak freedom is ub_safe's (LSan) to check.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_invalid_value_filters_match_c() {
        const CHILD: &str = "CRUSTIFY_EQUIV_CONFIG_INVALID_FILTER";
        let name = "config::enumeration::enumeration_invalid_value_filters_match_c";
        if std::env::var_os(CHILD).is_none() {
            let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
            let separator = if inherited.is_empty() { "" } else { ":" };
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, "1")
                .env(
                    "ASAN_OPTIONS",
                    format!("{inherited}{separator}detect_leaks=0"),
                )
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "child `{name}` failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            return;
        }
        init();
        let paths = [
            scratch("enumeration", "c-invalid-filter"),
            scratch("enumeration", "r-invalid-filter"),
        ];
        for path in &paths {
            std::fs::write(path, b"[a]\n m = one\n").unwrap();
        }
        let names = paths
            .each_ref()
            .map(|p| CString::new(p.to_str().unwrap()).unwrap());
        let c = c_open(&names[0]);
        let r = Config::open(&names[1]).unwrap();
        for (name, regexp) in [(c"a.m", c"["), (c"A.M", c"(?<"), (c"missing.key", c"*")] {
            for stop in [0, 23] {
                compare(&c, &r, 2, name, Some(regexp), stop);
            }
        }
        drop(c);
        drop(r);
        for path in paths {
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_empty_configs_match_c() {
        init();
        let mut out = ptr::null_mut();
        // SAFETY: writable result slot receives one fresh empty config.
        assert_eq!(unsafe { ffi::git_config_new(&mut out) }, 0);
        // SAFETY: the successful constructor transfers a sole reference, without
        // any repository borrow, paired with ConfigFree on drop.
        let c = unsafe { ConfigBox::from_c(out) }.unwrap();
        let r = Config::new().unwrap();
        for (mode, arg) in [(0, c""), (1, c""), (1, c"["), (2, c"a.m"), (2, c"invalid")] {
            compare(&c, &r, mode, arg, None, 0);
        }
    }
}

mod iterator_entries {
    use super::*;
    use libgit2::libgit2::config::config::EntryRef;
    use libgit2::libgit2::config::sys::config::IteratorBox;
    use libgit2::libgit2::repository::repository::Repository;

    type Row = (
        Option<CString>,
        Option<CString>,
        Option<CString>,
        Option<CString>,
        u32,
        i32,
    );
    fn observe(entry: EntryRef<'_>) -> Row {
        (
            entry.name().map(CStr::to_owned),
            entry.value().map(CStr::to_owned),
            entry.backend_type().map(CStr::to_owned),
            entry.origin_path().map(CStr::to_owned),
            entry.include_depth(),
            entry.level().raw(),
        )
    }

    fn compare(mut rust: IteratorBox<'_>, raw: *mut ffi::git_config_iterator) -> usize {
        let mut count = 0;
        loop {
            let mut out = ptr::null_mut();
            // SAFETY: complete independent live raw iterator; exclusive output
            // slot, with no raw entry retained across this next call.
            let rc = unsafe { ffi::git_config_next(&mut out, raw) };
            let raw_error = (rc != 0 && rc != ffi::GIT_ITEROVER).then(last);
            let mut m = rust.as_mut();
            match m.advance_entry() {
                Ok(Some(entry)) => {
                    assert_eq!(rc, 0);
                    // SAFETY: success initializes out to a live borrowed entry;
                    // read pointers/scalars by value, with no reference to C layout.
                    let e = unsafe { out.read() };
                    let string = |p: *const std::ffi::c_char| {
                        if p.is_null() {
                            None
                        } else {
                            // SAFETY: entry strings are immutable NUL strings
                            // pinned until the next raw advance; copy immediately.
                            Some(unsafe { CStr::from_ptr(p) }.to_owned())
                        }
                    };
                    assert_eq!(
                        observe(entry),
                        (
                            string(e.name),
                            string(e.value),
                            string(e.backend_type),
                            string(e.origin_path),
                            e.include_depth,
                            e.level
                        )
                    );
                    count += 1;
                }
                Ok(None) => {
                    assert_eq!(rc, ffi::GIT_ITEROVER);
                    assert!(out.is_null(), "C leaves output untouched on ITEROVER");
                    // SAFETY: C permits repeated advancement of an exhausted iterator.
                    let repeated = unsafe { ffi::git_config_next(&mut out, raw) };
                    assert_eq!(repeated, rc);
                    assert!(m.advance_entry().unwrap().is_none());
                    break;
                }
                Err(error) => {
                    assert_eq!(error.raw_code(), rc);
                    assert_eq!(error.error(), &raw_error.unwrap());
                    assert!(out.is_null(), "C leaves output untouched on error");
                }
            }
        }
        // SAFETY: consumes the sole raw owner exactly once, before its config dies.
        unsafe { ffi::git_config_iterator_free(raw) };
        drop(rust);
        count
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn public_entries_sequences_match_c() {
        let file = scratch("public-iterator", "both");
        std::fs::write(&file, b"[a]\nx = one\nx = two\nflag\nempty =\nbin = \xff\n").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let rust_cfg = Config::open(&path).unwrap();
        let raw_cfg = Config::open(&path).unwrap(); // Independently allocated backend/list.
        let mut raw = ptr::null_mut();
        // SAFETY: live reference-side config; writable output; success gives sole owner.
        let rc = unsafe { ffi::git_config_iterator_new(&mut raw, raw_cfg.as_c_ptr()) };
        assert_eq!(rc, 0);
        assert_eq!(compare(rust_cfg.as_ref().iterator().unwrap(), raw), 5);
        for regexp in [None, Some(c"^a\\.x$"), Some(c"nomatch")] {
            // SAFETY: config remains live; regex borrowed only during construction.
            let rc = unsafe {
                ffi::git_config_iterator_glob_new(
                    &mut raw,
                    raw_cfg.as_c_ptr(),
                    regexp.map_or(ptr::null(), CStr::as_ptr),
                )
            };
            assert_eq!(rc, 0);
            compare(rust_cfg.as_ref().iterator_matching(regexp).unwrap(), raw);
        }
        // A regex on a valueless entry is deliberately not sent to defective C.
        for (key, regex, expected) in [
            (c"A.X", None, 2),
            (c"a.x", Some(c"two"), 1),
            (c"a.flag", None, 1),
            (c"a.empty", Some(c"^$"), 1),
            (c"a.bin", Some(c"^\xff$"), 1),
            (c"a.missing", None, 0),
        ] {
            // SAFETY: key/regex are NUL strings; inputs cover only defined C behavior;
            // config remains live until compare releases the complete returned owner.
            let rc = unsafe {
                ffi::git_config_multivar_iterator_new(
                    &mut raw,
                    raw_cfg.as_c_ptr(),
                    key.as_ptr(),
                    regex.map_or(ptr::null(), CStr::as_ptr),
                )
            };
            assert_eq!(rc, 0);
            assert_eq!(
                compare(
                    rust_cfg.as_ref().multivar_iterator(key, regex).unwrap(),
                    raw
                ),
                expected
            );
        }
        let rust = rust_cfg.as_ref().iterator().unwrap();
        // SAFETY: independent live config, writable output; backend opens lazily.
        let rc = unsafe { ffi::git_config_iterator_new(&mut raw, raw_cfg.as_c_ptr()) };
        assert_eq!(rc, 0);
        std::fs::write(&file, b"[unterminated\n").unwrap();
        assert_eq!(compare(rust, raw), 0); // Refresh failure, then exhaustion.
        drop((rust_cfg, raw_cfg));
        std::fs::remove_file(file).unwrap();
    }

    /// One step's observation: the entry lent, the end, or the failure's code,
    /// class and message.
    type Step = Result<Option<Row>, (c_int, c_int, String)>;

    fn rust_step(iter: &mut IteratorBox<'_>) -> Step {
        let mut m = iter.as_mut();
        match m.advance_entry() {
            Ok(entry) => Ok(entry.map(observe)),
            Err(e) => Err((
                e.raw_code(),
                e.raw_class(),
                e.message().to_string_lossy().into_owned(),
            )),
        }
    }

    fn raw_step(raw: *mut ffi::git_config_iterator) -> Step {
        let mut out = ptr::null_mut();
        // SAFETY: exclusive live raw iterator and a writable slot; no entry of
        // an earlier step is read after this advance.
        let rc = unsafe { ffi::git_config_next(&mut out, raw) };
        let string = |p: *const std::ffi::c_char| {
            // SAFETY: a lent entry's non-NULL strings are immutable and
            // NUL-terminated until the next raw advance; copied immediately.
            (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_owned())
        };
        match rc {
            0 => {
                // SAFETY: success lent a live entry; read by value, no reference.
                let e = unsafe { out.read() };
                Ok(Some((
                    string(e.name),
                    string(e.value),
                    string(e.backend_type),
                    string(e.origin_path),
                    e.include_depth,
                    e.level,
                )))
            }
            ffi::GIT_ITEROVER => Ok(None),
            rc => {
                let (class, message) = c_error();
                Err((rc, class, message))
            }
        }
    }

    /// `git_config_new` plus `files` added in order, built through C alone.
    fn raw_config(files: &[(&CStr, ConfigLevel)]) -> *mut ffi::git_config {
        let mut config = ptr::null_mut();
        // SAFETY: writable slot; success returns the only reference.
        assert_eq!(unsafe { ffi::git_config_new(&mut config) }, 0);
        for (path, level) in files {
            // SAFETY: live config; NUL-terminated path copied; no repository.
            let rc = unsafe {
                ffi::git_config_add_file_ondisk(config, path.as_ptr(), level.raw(), ptr::null(), 0)
            };
            assert_eq!(rc, 0);
        }
        config
    }

    /// Three priorities, one with an include, iterated by several iterators of
    /// each side at once: every pair advances in turn, so one config serves
    /// interleaved advancement, and each step is compared, through exhaustion
    /// and one more advance. Then a level view of a repository's config, whose
    /// view shares the repository's backend.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn public_entries_across_levels_interleaved_match_c() {
        init();
        let dir = std::env::temp_dir().join(format!(
            "crustify-equiv-config-{}-public-levels",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, body: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, body).unwrap();
            CString::new(path.to_str().unwrap()).unwrap()
        };
        let system = write("system", b"[a]\nx = system\n[b]\nonly = sys\n");
        let global = write("global", b"[a]\nx = global\nflag\n");
        let local = write(
            "local",
            b"[a]\nx = local\n[include]\npath = included\n[b \"Sub\"]\nKey = mixed\n",
        );
        write("included", b"[a]\nx = included\n[b]\nnested = deep\n");
        let files = [
            (system.as_c_str(), ConfigLevel::SYSTEM),
            (global.as_c_str(), ConfigLevel::GLOBAL),
            (local.as_c_str(), ConfigLevel::LOCAL),
        ];
        let mut rust_cfg = Config::new().unwrap();
        for (path, level) in files {
            rust_cfg
                .as_mut()
                .add_file(path, level, None, false)
                .unwrap();
        }
        let raw_cfg = raw_config(&files);

        let mut raws = [ptr::null_mut(); 4];
        // SAFETY: live independent config, writable slots; the regexes are
        // borrowed for construction. No a.x value is missing, so C's own
        // value filter never reaches its strlen(NULL).
        unsafe {
            assert_eq!(ffi::git_config_iterator_new(&mut raws[0], raw_cfg), 0);
            let glob = c"^b\\.".as_ptr();
            assert_eq!(
                ffi::git_config_iterator_glob_new(&mut raws[1], raw_cfg, glob),
                0
            );
            let (name, value) = (c"A.X".as_ptr(), c"l".as_ptr());
            assert_eq!(
                ffi::git_config_multivar_iterator_new(&mut raws[2], raw_cfg, name, value),
                0
            );
            let name = c"b.Sub.key".as_ptr();
            assert_eq!(
                ffi::git_config_multivar_iterator_new(&mut raws[3], raw_cfg, name, ptr::null()),
                0
            );
        }
        let mut rusts = [
            rust_cfg.as_ref().iterator().unwrap(),
            rust_cfg.as_ref().iterator_matching(Some(c"^b\\.")).unwrap(),
            rust_cfg
                .as_ref()
                .multivar_iterator(c"A.X", Some(c"l"))
                .unwrap(),
            rust_cfg
                .as_ref()
                .multivar_iterator(c"b.Sub.key", None)
                .unwrap(),
        ];
        let mut steps = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        let mut done = [false; 4];
        while !done.iter().all(|&d| d) {
            for i in 0..4 {
                let (raw, rust) = (raw_step(raws[i]), rust_step(&mut rusts[i]));
                assert_eq!(rust, raw, "iterator {i}, step {}", steps[i].len());
                if raw == Ok(None) {
                    if done[i] {
                        continue;
                    }
                    done[i] = true;
                }
                steps[i].push(raw);
            }
        }
        let entries: Vec<_> = steps
            .iter()
            .map(|s| s.iter().filter(|s| matches!(s, Ok(Some(_)))).count())
            .collect();
        // All: nine entries, include.path and the included pair (depth 1)
        // among them; b.*: three; a.x matching "l": global, local and
        // included; the normalized name keeps its subsection's case.
        assert_eq!(entries, [9, 3, 3, 1]);
        let rows: Vec<_> = steps[0].iter().flatten().flatten().collect();
        assert!(
            rows.iter()
                .any(|r| r.1.as_deref() == Some(c"included") && r.4 == 1)
        );
        assert!(rows.iter().any(|r| r.0.as_deref() == Some(c"b.Sub.key")));
        for (rust, raw) in rusts.into_iter().zip(raws) {
            // SAFETY: each raw iterator is released once, before its config.
            unsafe { ffi::git_config_iterator_free(raw) };
            drop(rust);
        }
        // SAFETY: the C side's only reference, released once.
        unsafe { ffi::git_config_free(raw_cfg) };
        drop(rust_cfg);

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/resources/testrepo.git\0"
        );
        let path = CStr::from_bytes_with_nul(path.as_bytes()).unwrap();
        let rust_repo = Repository::open(path).unwrap();
        let mut c_repo = ptr::null_mut();
        // SAFETY: writable slot; NUL-terminated path read during the call.
        let rc = unsafe {
            ffi::git_repository_open_ext(
                &mut c_repo,
                path.as_ptr(),
                ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                ptr::null(),
            )
        };
        assert_eq!(rc, 0);
        let (mut c_cfg, mut c_view, mut raw) = (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
        // SAFETY: live repository and writable slots; the view upgrades the
        // repository config's local backend into a fresh config.
        unsafe {
            assert_eq!(ffi::git_repository_config(&mut c_cfg, c_repo), 0);
            let level = ffi::GIT_CONFIG_LEVEL_LOCAL;
            assert_eq!(ffi::git_config_open_level(&mut c_view, c_cfg, level), 0);
            assert_eq!(ffi::git_config_iterator_new(&mut raw, c_view), 0);
        }
        let rust_cfg = rust_repo.as_ref().config().unwrap();
        let rust_view = rust_cfg.as_ref().open_level(ConfigLevel::LOCAL).unwrap();
        assert!(compare(rust_view.as_ref().iterator().unwrap(), raw) > 0);
        drop(rust_view);
        drop(rust_cfg);
        drop(rust_repo);
        // SAFETY: each C reference is released once, the repository last.
        unsafe {
            ffi::git_config_free(c_view);
            ffi::git_config_free(c_cfg);
            ffi::git_repository_free(c_repo);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Constructor failures leave the same code, class and message on both
    /// sides: invalid key-regexes and invalid multivar names (normalized first,
    /// before any value regex). An empty config's iterators end at once. A
    /// valid name with an invalid value regex is compared in
    /// `sys_config::iterators::multivar_regex_errors_match_c` (C leaks there).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn public_constructor_failures_and_empty_configs_match_c() {
        init();
        let rust_cfg = Config::new().unwrap();
        let raw_cfg = raw_config(&[]);
        let record = |rc: c_int, raw: *mut ffi::git_config_iterator| {
            let failure = (rc < 0).then(c_error);
            (rc, failure, raw.is_null())
        };
        let rust_record = |result: Result<IteratorBox<'_>, ConfigError>| match result {
            Ok(mut iter) => {
                assert_eq!(rust_step(&mut iter), Ok(None));
                (0, None, false)
            }
            Err(e) => (
                e.raw_code(),
                Some((e.raw_class(), e.message().to_string_lossy().into_owned())),
                true,
            ),
        };
        for regexp in [c"[", c"(", c"a{2,1}", c"\\", c""] {
            let mut raw = ptr::null_mut();
            // SAFETY: live config, writable slot; regex read during the call.
            let rc =
                unsafe { ffi::git_config_iterator_glob_new(&mut raw, raw_cfg, regexp.as_ptr()) };
            let expected = record(rc, raw);
            if rc == 0 {
                assert_eq!(raw_step(raw), Ok(None));
                // SAFETY: the only owner, released once.
                unsafe { ffi::git_config_iterator_free(raw) };
            }
            let actual = rust_record(rust_cfg.as_ref().iterator_matching(Some(regexp)));
            assert_eq!(actual, expected, "{regexp:?}");
        }
        for (name, regexp) in [
            (c"invalid", None),
            (c"a.", None),
            (c".b", None),
            (c"", None),
            (c"a b.c", None),
            (c"invalid", Some(c"[")),
            (c"a.b", None),
        ] {
            let mut raw = ptr::null_mut();
            // SAFETY: live config, writable slot; inputs read during the call;
            // every failing name fails before C compiles a value regex.
            let rc = unsafe {
                ffi::git_config_multivar_iterator_new(
                    &mut raw,
                    raw_cfg,
                    name.as_ptr(),
                    regexp.map_or(ptr::null(), CStr::as_ptr),
                )
            };
            let expected = record(rc, raw);
            if rc == 0 {
                assert_eq!(raw_step(raw), Ok(None));
                // SAFETY: the only owner, released once.
                unsafe { ffi::git_config_iterator_free(raw) };
            }
            let actual = rust_record(rust_cfg.as_ref().multivar_iterator(name, regexp));
            assert_eq!(actual, expected, "{name:?} {regexp:?}");
        }
        let mut raw = ptr::null_mut();
        // SAFETY: live config and writable slot.
        let rc = unsafe { ffi::git_config_iterator_new(&mut raw, raw_cfg) };
        assert_eq!(rc, 0);
        let mut rust = rust_cfg.as_ref().iterator().unwrap();
        for _ in 0..2 {
            assert_eq!(rust_step(&mut rust), raw_step(raw));
        }
        // SAFETY: each C object released once, the iterator first.
        unsafe {
            ffi::git_config_iterator_free(raw);
            ffi::git_config_free(raw_cfg);
        }
    }
}
