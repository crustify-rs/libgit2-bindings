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

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

/// One step's observation: the return code, the value read (0 unless one was), and
/// for a failure the error libgit2 left, `(class, message)`.
type Record = (c_int, i32, Option<(c_int, String)>);

/// The calling thread's last error, read through C.
fn c_error() -> (c_int, String) {
    // SAFETY: no arguments; libgit2 is initialised, so it returns a live error.
    let error = unsafe { &*ffi::git_error_last() };
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
