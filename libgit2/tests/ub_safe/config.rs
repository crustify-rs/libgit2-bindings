//! UB tests for `git2/config.h`: the `Config` object owned by a `ConfigBox` (the sole
//! reference) or a `ConfigArc` (one of several), their handles, the `git_config_new`
//! constructor and the `git_config_free` policy.
//!
//! Every reference must be released exactly once, whichever owner holds it and in
//! whatever order owners and repositories go (ASan: double free, use after free;
//! LSan: leak). The config routines have no wrappers yet: the tests drive the object
//! through fixtures over the owners' `as_c_ptr`. Nothing here runs without libgit2, so
//! Miri runs none of it. Illegal uses are compile-fail doctests on the crate: a
//! `ConfigArc` clone, an exclusive handle from a shared owner, sending either owner to
//! another thread, a handle outliving its owner, a repository's config outliving the
//! repository its file backends read.

use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::config::config::{Config, ConfigArc, ConfigBox};
use libgit2::libgit2::repository::repository::Repository;
use libgit2::libgit2::util::errors::ErrorCode;

use crate::fixture::commit_and_config::{
    OpenRepo, TESTREPO, config, get_int32, set_int32, write_through_file,
};
use crate::fixture::isolated;

const NOT_FOUND: i32 = ErrorCode::NotFound.raw();

/// A scratch config file for `tag`, removed first.
fn scratch(tag: &str) -> std::ffi::CString {
    let path =
        std::env::temp_dir().join(format!("crustify-ub-config-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    std::ffi::CString::new(path.to_str().unwrap()).unwrap()
}

/// Built through `Config::new` and the policy's constructors, each config is used and
/// released once, its backends with it.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn configs_build_use_and_release() {
    let mut sole = Config::new().unwrap();
    let _ = sole.as_ref();
    let _ = sole.as_mut().as_ref();
    assert_eq!(get_int32(sole.as_c_ptr(), c"a.b"), Err(NOT_FOUND));
    let file = scratch("sole");
    assert_eq!(
        write_through_file(sole.as_c_ptr(), &file, c"a.b", 5),
        (0, 0)
    );
    assert_eq!(get_int32(sole.as_c_ptr(), c"a.b"), Ok(5));
    drop(sole);

    let boxed = ConfigBox::new().unwrap();
    let shared = ConfigArc::new().unwrap();
    assert_eq!(get_int32(boxed.as_c_ptr(), c"a.b"), Err(NOT_FOUND));
    assert_eq!(get_int32(shared.as_c_ptr(), c"a.b"), Err(NOT_FOUND));
    let _ = std::fs::remove_file(file.to_str().unwrap());
}

/// A box shared as an arc, and a repository's config adopted twice, are each released
/// once per reference, in either order, the repository first or last.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn shared_configs_release_each_reference_once() {
    let file = scratch("shared");
    let sole = Config::new().unwrap();
    assert_eq!(
        write_through_file(sole.as_c_ptr(), &file, c"a.b", 9),
        (0, 0)
    );
    let shared = ConfigArc::from(sole);
    let _ = shared.as_ref();
    assert_eq!(get_int32(shared.as_c_ptr(), c"a.b"), Ok(9));
    drop(shared);

    let repo = OpenRepo::open(TESTREPO);
    let (first, second) = (config(&repo), config(&repo));
    assert!(ConfigArc::ptr_eq(&first, &second));
    drop(second);
    assert_eq!(
        get_int32(first.as_c_ptr(), c"core.repositoryformatversion"),
        Ok(0)
    );
    drop(first);
    // The repository still holds its own reference, and reloads nothing.
    let third = config(&repo);
    assert_eq!(
        get_int32(third.as_c_ptr(), c"core.repositoryformatversion"),
        Ok(0)
    );
    drop(third);
    drop(repo);
    let _ = std::fs::remove_file(file.to_str().unwrap());
}

/// A repository's config is used only while the repository lives: its file backends
/// keep the repository for their `includeIf` conditions, which every reparse of a
/// file evaluates (`conditional_match_gitdir` reads `git_repository_path`). A write
/// through a config a repository lent, after that repository was freed, was a use
/// after free (review wave 1 batch 1); now the arc borrows the repository (a
/// compile-fail doctest on `ConfigArc` and
/// `compile_fail/config_arc_outlives_repository.rs`), and the write reparses the file
/// against the live repository.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn repository_config_reparses_against_its_live_repository() {
    let dir = std::env::temp_dir().join(format!(
        "crustify-ub-config-includeif-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
    let repo = Repository::init(&path, false).unwrap();
    std::fs::write(
        dir.join(".git/config"),
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n[includeIf \"gitdir:/nowhere/\"]\n\tpath = extra\n",
    )
    .unwrap();
    let shared = repo.as_ref().config().unwrap();
    assert_eq!(
        get_int32(shared.as_c_ptr(), c"core.repositoryformatversion"),
        Ok(0)
    );
    // The write reparses the file, which evaluates the condition against the
    // repository: live, as the borrow guarantees.
    assert_eq!(set_int32(shared.as_c_ptr(), c"a.b", 7), 0);
    assert_eq!(get_int32(shared.as_c_ptr(), c"a.b"), Ok(7));
    drop(shared);
    drop(repo);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The process's first configs, built on several threads at once: each `Config::new`
/// pins the runtime before `git_config_new` reaches the allocator, so they race
/// neither the first `git_libgit2_init` nor each other (TSan); each config stays on its
/// own thread.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn first_configs_race_to_init() {
    isolated(
        concat!(module_path!(), "::first_configs_race_to_init"),
        || {
            let barrier = Barrier::new(4);
            thread::scope(|s| {
                for _ in 0..4 {
                    s.spawn(|| {
                        barrier.wait();
                        let config = Config::new().unwrap();
                        assert_eq!(get_int32(config.as_c_ptr(), c"a.b"), Err(NOT_FOUND));
                    });
                }
            });
        },
    );
}

/// Parsers only borrow input during the call. These tests use exclusively the
/// safe public surface: stack bounds/alignment, integer overflow, discarded
/// inputs, retained error copies, repeated calls and concurrent error reporting.
#[forbid(unsafe_code)]
mod parsers {
    use super::*;
    use libgit2::libgit2::config::config::{parse_bool, parse_i32, parse_i64};
    use libgit2::libgit2::util::errors::ErrorClass;
    use libgit2::libgit2::util::sys::errors::{clear, set_str};
    use std::ffi::{CStr, CString};

    #[test]
    #[cfg_attr(
        miri,
        ignore = "calls libgit2; no Rust-only pointer or layout operations"
    )]
    fn nullable_and_boundary_inputs() {
        assert!(parse_bool(None).unwrap());
        assert!(!parse_bool(Some(c"")).unwrap());
        assert!(parse_i32(None).is_err());
        assert!(parse_i64(None).is_err());
        assert_eq!(parse_i32(Some(c"-2147483648")).unwrap(), i32::MIN);
        assert_eq!(parse_i32(Some(c"2147483647")).unwrap(), i32::MAX);
        assert_eq!(parse_i64(Some(c"-9223372036854775808")).unwrap(), i64::MIN);
        assert_eq!(parse_i64(Some(c"9223372036854775807")).unwrap(), i64::MAX);
        for value in [c"2147483648", c"-2147483649", c"2G"] {
            assert!(parse_i32(Some(value)).is_err());
            assert!(parse_bool(Some(value)).is_err());
        }
        for value in [
            c"9223372036854775808",
            c"-9223372036854775809",
            c"8589934592G",
            c"8796093022208m",
            c"9007199254740992g",
            c"9007199254740992m",
            c"8796093022208g",
            c"9007199254740992k",
            c"1kb",
            c"",
            c"+",
            c"-",
            c"0x",
        ] {
            assert!(parse_i64(Some(value)).is_err());
        }
        // The NUL sits at the very end of stack/heap storage; C's suffix lookahead
        // may read it but must not go beyond it. Invalid bytes need no UTF-8 claim.
        for bytes in [b"1k\0".as_slice(), b"1kb\0", b"\xff\0"] {
            let value = CStr::from_bytes_with_nul(bytes).unwrap();
            let _ = parse_bool(Some(value));
            let _ = parse_i32(Some(value));
            let _ = parse_i64(Some(value));
        }
        let long = CString::new("9".repeat(4096)).unwrap();
        assert!(parse_i64(Some(&long)).is_err());
        for byte in 1..=255 {
            let input = CString::new([byte]).unwrap();
            let _ = parse_bool(Some(&input));
            let _ = parse_i32(Some(&input));
            let _ = parse_i64(Some(&input));
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn results_outlive_inputs_and_later_calls() {
        let (boolean, small, large, error) = {
            let input = CString::new("-2m").unwrap();
            let invalid = CString::new([0xff, b'%']).unwrap();
            (
                parse_bool(Some(&input)).unwrap(),
                parse_i32(Some(&input)).unwrap(),
                parse_i64(Some(&input)).unwrap(),
                parse_i64(Some(&invalid)).unwrap_err(),
            )
        };
        let copy = error.clone();
        clear();
        set_str(ErrorClass::Invalid, c"overwrite the foreign error buffer").unwrap();
        assert!(boolean);
        assert_eq!(small, -2 * 1024 * 1024);
        assert_eq!(large, i64::from(small));
        assert_eq!(error, copy);
        assert_eq!(error.class(), Some(ErrorClass::Config));
        assert!(
            error
                .message()
                .to_bytes()
                .windows(2)
                .any(|w| w == [0xff, b'%'])
        );
        // A new mutable use of input storage is safe after each completed call:
        // neither C nor the returned scalar/error retained its earlier borrow.
        let mut bytes = b"1k\0".to_vec();
        for _ in 0..64 {
            assert_eq!(
                parse_i32(Some(CStr::from_bytes_with_nul(&bytes).unwrap())).unwrap(),
                1024
            );
            bytes[0] = b'2';
            assert_eq!(
                parse_i64(Some(CStr::from_bytes_with_nul(&bytes).unwrap())).unwrap(),
                2048
            );
            bytes[0] = b'1';
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn first_parser_errors_race_to_init() {
        isolated(
            concat!(module_path!(), "::first_parser_errors_race_to_init"),
            || {
                let barrier = Barrier::new(6);
                thread::scope(|scope| {
                    for index in 0..6 {
                        let barrier = &barrier;
                        scope.spawn(move || {
                            let invalid = CString::new(format!("invalid-thread-{index}")).unwrap();
                            // Text after a suffix: the error the wrapper sets itself.
                            let tail = CString::new(format!("1k-invalid-thread-{index}")).unwrap();
                            barrier.wait();
                            let mut kept = Vec::new();
                            for _ in 0..32 {
                                kept.push(parse_i64(Some(&invalid)).unwrap_err());
                                kept.push(parse_i64(Some(&tail)).unwrap_err());
                                kept.push(parse_i32(Some(&invalid)).unwrap_err());
                                kept.push(parse_bool(Some(&invalid)).unwrap_err());
                                assert!(parse_bool(None).unwrap());
                                assert_eq!(parse_i32(Some(c"1K")).unwrap(), 1024);
                                assert_eq!(parse_i64(Some(c"1G")).unwrap(), 1 << 30);
                            }
                            drop(invalid);
                            drop(tail);
                            for error in kept {
                                assert_eq!(error.class(), Some(ErrorClass::Config));
                                assert!(
                                    error
                                        .to_string()
                                        .contains(&format!("invalid-thread-{index}"))
                                );
                            }
                        });
                    }
                });
            },
        );
    }
}

/// Scalar priorities are copied between threads without accessing foreign memory;
/// extreme/unknown values are never interpreted as Rust enum discriminants.
#[test]
fn config_level_values_and_threads() {
    use libgit2::libgit2::config::config::ConfigLevel;
    let levels = [
        ConfigLevel::SYSTEM,
        ConfigLevel::XDG,
        ConfigLevel::GLOBAL,
        ConfigLevel::LOCAL,
        ConfigLevel::WORKTREE,
        ConfigLevel::APP,
        ConfigLevel::HIGHEST,
        ConfigLevel::from_raw(i32::MIN),
        ConfigLevel::from_raw(i32::MAX),
        ConfigLevel::from_raw(42),
    ];
    thread::scope(|s| {
        for level in &levels {
            s.spawn(move || assert_eq!(ConfigLevel::from_raw(level.raw()), *level));
        }
    });
}

/// Unknown mapping-mode values are rejected, never transmuted into the enum.
#[test]
fn mapping_modes_are_checked() {
    use libgit2::libgit2::config::config::MapType;
    for mode in MapType::ALL {
        assert_eq!(MapType::from_raw(mode.raw()), Some(mode));
    }
    assert_eq!(MapType::from_raw(4), None);
    assert_eq!(MapType::from_raw(u32::MAX), None);
}


#[forbid(unsafe_code)]
mod entry_and_map_types {
    use libgit2::libgit2::config::config::{Config, ConfigLevel, ConfigMap, EntryArc, MapMatch};
    use libgit2::libgit2::repository::repository::Repository;
    use libgit2::libgit2::util::errors::ErrorCode;
    use std::ffi::CString;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn entries_retain_storage_across_parent_drop_and_refresh() {
        let dir = std::env::temp_dir().join(format!("crustify-entry-b0-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = CString::new(dir.to_str().unwrap()).unwrap();
        let repo = Repository::init(&path, false).unwrap();
        let config_path = dir.join(".git/config");
        std::fs::write(&config_path,
            b"[core]\nrepositoryformatversion = 0\nbare = false\n[test]\nflag\nempty =\nvalue = first\n[include]\npath = extra\n").unwrap();
        std::fs::write(dir.join(".git/extra"), b"[test]\nincluded = nested\n").unwrap();
        let cfg = repo.as_ref().config().unwrap();
        let first = cfg.as_ref().entry(c"TEST.value").unwrap();
        let second = cfg.as_ref().entry(c"test.value").unwrap();
        assert!(EntryArc::ptr_eq(&first, &second));
        assert_eq!(first.as_ref().name(), Some(c"test.value"));
        assert_eq!(first.as_ref().value(), Some(c"first"));
        assert_eq!(first.as_ref().backend_type(), Some(c"file"));
        assert_eq!(first.as_ref().level(), ConfigLevel::LOCAL);
        assert_eq!(first.as_ref().include_depth(), 0);
        let origin = CString::new(config_path.to_str().unwrap()).unwrap();
        assert_eq!(first.as_ref().origin_path(), Some(origin.as_c_str()));
        let flag = cfg.as_ref().entry(c"test.flag").unwrap();
        let empty = cfg.as_ref().entry(c"test.empty").unwrap();
        let included = cfg.as_ref().entry(c"test.included").unwrap();
        assert_eq!(flag.as_ref().value(), None);
        assert_eq!(empty.as_ref().value(), Some(c""));
        assert_eq!(included.as_ref().include_depth(), 1);
        // Changed size guarantees refresh notices this, independent of timestamp precision.
        std::fs::write(&config_path, b"[core]\nrepositoryformatversion = 0\nbare = false\n[test]\nvalue = changed-and-longer\n").unwrap();
        let changed = cfg.as_ref().entry(c"test.value").unwrap();
        assert_eq!(changed.as_ref().value(), Some(c"changed-and-longer"));
        assert_eq!(first.as_ref().value(), Some(c"first"));
        drop(first);
        drop(cfg);
        drop(repo);
        assert_eq!(second.as_ref().value(), Some(c"first"));
        assert_eq!(included.as_ref().value(), Some(c"nested"));
        assert_eq!(changed.as_ref().value(), Some(c"changed-and-longer"));
        drop((second, flag, empty, included, changed));
        std::fs::remove_dir_all(dir).unwrap();
        let cfg = Config::new().unwrap();
        assert_eq!(
            cfg.as_ref().entry(c"test.missing").unwrap_err().code(),
            Some(ErrorCode::NotFound)
        );
        assert!(cfg.as_ref().entry(c"invalid").is_err());
    }

    #[test]
    fn maps_preserve_borrows_across_clone_and_mutable_reborrows() {
        let text = CString::new([b'x', 0xff]).unwrap();
        let cloned = {
            let mut map = ConfigMap::new(MapMatch::String(&text), i32::MIN);
            let cloned = map.clone();
            let mut m = map.as_mut();
            for rule in [
                MapMatch::False,
                MapMatch::True,
                MapMatch::Int32,
                MapMatch::String(c""),
                MapMatch::String(&text),
            ] {
                m.as_mut().set_match(rule);
                assert_eq!(m.as_ref().rule(), rule);
                m.set_map_value(i32::MAX);
                assert_eq!(m.as_ref().map_value(), i32::MAX);
                m.set_str_match(&text);
                assert_eq!(m.as_ref().str_match(), Some(text.as_c_str()));
            }
            cloned
        };
        assert_eq!(cloned.as_ref().str_match(), Some(text.as_c_str()));
        assert_eq!(cloned.as_ref().map_value(), i32::MIN);
    }
}
