//! UB tests for `git2/config.h`: the `Config` object owned by a `ConfigBox` (the sole
//! reference) or a `ConfigArc` (one of several), their handles, the `git_config_new`
//! constructor and the `git_config_free` policy.
//!
//! Every reference must be released exactly once, whichever owner holds it and in
//! whatever order owners and repositories go (ASan: double free, use after free;
//! LSan: leak). The tests drive the object through the safe wrappers only. Nothing
//! here runs without libgit2, so Miri runs none of it. Illegal uses are compile-fail
//! doctests on the crate and `compile_fail/` cases: a
//! `ConfigArc` clone, an exclusive handle from a shared owner, sending either owner to
//! another thread, a handle outliving its owner, a repository's config outliving the
//! repository its file backends read.

use std::ffi::{CStr, c_int};
use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::config::config::{Config, ConfigArc, ConfigBox, ConfigLevel, ConfigRef};
use libgit2::libgit2::repository::repository::Repository;
use libgit2::libgit2::util::errors::ErrorCode;

use crate::fixture::commit_and_config::{OpenRepo, TESTREPO, config};
use crate::fixture::isolated;

const NOT_FOUND: i32 = ErrorCode::NotFound.raw();

/// `get_i32` of `name`: the value, or the code it failed with.
fn number(config: ConfigRef<'_, '_>, name: &CStr) -> Result<i32, c_int> {
    config.get_i32(name).map_err(|e| e.raw_code())
}

/// Adds `file` as the local backend of `config`, then writes `value` to `name` in it.
fn write_through_file(config: &mut ConfigBox<'_>, file: &CStr, name: &CStr, value: i32) {
    config
        .as_mut()
        .add_file(file, ConfigLevel::LOCAL, None, false)
        .unwrap();
    config.as_mut().set_i32(name, value).unwrap();
}

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
    assert_eq!(number(sole.as_ref(), c"a.b"), Err(NOT_FOUND));
    let file = scratch("sole");
    write_through_file(&mut sole, &file, c"a.b", 5);
    assert_eq!(number(sole.as_ref(), c"a.b"), Ok(5));
    drop(sole);

    let boxed = ConfigBox::new().unwrap();
    let shared = ConfigArc::new().unwrap();
    assert_eq!(number(boxed.as_ref(), c"a.b"), Err(NOT_FOUND));
    assert_eq!(number(shared.as_ref(), c"a.b"), Err(NOT_FOUND));
    let _ = std::fs::remove_file(file.to_str().unwrap());
}

/// A box shared as an arc, and a repository's config adopted twice, are each released
/// once per reference, in either order, the repository first or last.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn shared_configs_release_each_reference_once() {
    let file = scratch("shared");
    let mut sole = Config::new().unwrap();
    write_through_file(&mut sole, &file, c"a.b", 9);
    let shared = ConfigArc::from(sole);
    let _ = shared.as_ref();
    assert_eq!(number(shared.as_ref(), c"a.b"), Ok(9));
    drop(shared);

    let repo = OpenRepo::open(TESTREPO);
    let (first, second) = (config(&repo), config(&repo));
    assert!(ConfigArc::ptr_eq(&first, &second));
    drop(second);
    assert_eq!(
        number(first.as_ref(), c"core.repositoryformatversion"),
        Ok(0)
    );
    drop(first);
    // The repository still holds its own reference, and reloads nothing.
    let third = config(&repo);
    assert_eq!(
        number(third.as_ref(), c"core.repositoryformatversion"),
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
/// `compile_fail/config_arc_outlives_repository.rs`). A shared reference cannot write,
/// so the reparse here is the refresh a lookup makes after the file changed on disk,
/// which evaluates the condition against the live repository.
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
        number(shared.as_ref(), c"core.repositoryformatversion"),
        Ok(0)
    );
    // The changed size makes the lookup reparse the file, which evaluates the
    // condition against the repository: live, as the borrow guarantees.
    std::fs::write(
        dir.join(".git/config"),
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n[includeIf \"gitdir:/nowhere/\"]\n\tpath = extra\n[a]\n\tb = 7\n",
    )
    .unwrap();
    assert_eq!(number(shared.as_ref(), c"a.b"), Ok(7));
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
                        assert_eq!(number(config.as_ref(), c"a.b"), Err(NOT_FOUND));
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
        // Each lookup reads its own snapshot of the config: distinct entries.
        assert!(!EntryArc::ptr_eq(&first, &second));
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

    /// A `static` table is read from many threads at once, shared handles and
    /// scoped borrows cross threads, an exclusive handle writes on another
    /// thread, and records holding only `'static` borrows move to a detached one
    /// (TSan: data race; Miri: aliasing of the handles' pointers).
    #[test]
    fn maps_cross_threads() {
        static TABLE: [ConfigMap<'static>; 3] = [
            ConfigMap::new(MapMatch::False, 0),
            ConfigMap::new(MapMatch::Int32, 1),
            ConfigMap::new(MapMatch::String(c"input"), 2),
        ];
        let text = CString::new("scoped").unwrap();
        let text = text.as_c_str();
        let mut map = ConfigMap::new(MapMatch::String(text), 7);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    let rules: Vec<_> = TABLE.iter().map(|m| m.as_ref().rule()).collect();
                    assert_eq!(
                        rules,
                        [MapMatch::False, MapMatch::Int32, MapMatch::String(c"input")]
                    );
                });
            }
            let shared = map.as_ref();
            for _ in 0..2 {
                s.spawn(move || assert_eq!(shared.rule(), MapMatch::String(text)));
            }
            let owner = &map;
            s.spawn(move || assert_eq!(owner.as_ref().map_value(), 7));
        });
        std::thread::scope(|s| {
            let mut exclusive = map.as_mut();
            s.spawn(move || {
                exclusive.set_match(MapMatch::True);
                exclusive.set_map_value(8);
            });
        });
        assert_eq!(map.as_ref().rule(), MapMatch::True);
        assert_eq!(map.as_ref().str_match(), None);
        let detached = TABLE.to_vec();
        let observed = std::thread::spawn(move || {
            detached
                .iter()
                .map(|m| (m.as_ref().map_value(), m.as_ref().str_match().is_some()))
                .collect::<Vec<_>>()
        })
        .join()
        .unwrap();
        assert_eq!(observed, [(0, false), (1, false), (2, true)]);
    }

    /// Regression for `crustify/reviews/.../config/wave-1/batch-0/ub_safe/entry-lookup-forms-member-of-unwritten-backend-entry`:
    /// a lookup that rereads a file rewritten into something libgit2 cannot parse
    /// fails with the parse error instead of reaching `get_entry`'s member
    /// access through the entry pointer the failed backend never wrote (UBSan:
    /// member access within null pointer); entries acquired before stay valid.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn rewritten_unparsable_file_fails_lookup_cleanly() {
        let path = std::env::temp_dir().join(format!(
            "crustify-ub-entry-unparsable-{}",
            std::process::id()
        ));
        std::fs::write(&path, b"[test]\n key = value\n").unwrap();
        let file = CString::new(path.to_str().unwrap()).unwrap();
        let config = Config::open(&file).unwrap();
        let entry = config.as_ref().entry(c"test.key").unwrap();
        std::fs::write(&path, b"[test\n key = no closing bracket\n").unwrap();
        let error = config.as_ref().entry(c"test.key").unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Generic));
        let stale = config.as_ref().entry(c"test.key").unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(entry.as_ref().value(), Some(c"value"));
        assert_eq!(stale.as_ref().value(), Some(c"value"));
    }

    /// Entries stay readable after everything that produced them is gone: a
    /// snapshot and its source, a level view and its parent, the list a write
    /// replaces, and the file's own config; any release order is fine (ASan:
    /// use after free, double free; LSan: leaked entry lists).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn entries_outlive_snapshots_views_and_writes() {
        let path =
            std::env::temp_dir().join(format!("crustify-ub-entry-sources-{}", std::process::id()));
        std::fs::write(&path, b"[multi]\n v = one\n[test]\n key = before\n").unwrap();
        let file = CString::new(path.to_str().unwrap()).unwrap();
        let mut source = Config::new().unwrap();
        source
            .as_mut()
            .add_file(&file, ConfigLevel::GLOBAL, None, false)
            .unwrap();

        let before = source.as_ref().entry(c"test.key").unwrap();
        source
            .as_mut()
            .set_multivar(c"multi.v", c"^$", c"two")
            .unwrap();
        let multi = source.as_ref().entry(c"multi.v").unwrap();
        assert_eq!(multi.as_ref().value(), Some(c"two"));
        source.as_mut().set_string(c"test.key", c"after").unwrap();
        let after = source.as_ref().entry(c"test.key").unwrap();
        assert!(!EntryArc::ptr_eq(&before, &after));
        assert_eq!(before.as_ref().value(), Some(c"before"));

        let snapshot = source.as_ref().snapshot().unwrap();
        let snapped = snapshot.as_ref().entry(c"test.key").unwrap();
        let viewed = {
            let mut m = source.as_mut();
            let view = m.open_level(ConfigLevel::GLOBAL).unwrap();
            let viewed = view.as_ref().entry(c"multi.v").unwrap();
            drop(view);
            viewed
        };
        drop(snapshot);
        drop(source);
        std::fs::remove_file(&path).unwrap();

        assert_eq!(snapped.as_ref().value(), Some(c"after"));
        assert_eq!(snapped.as_ref().level(), ConfigLevel::GLOBAL);
        assert_eq!(snapped.as_ref().origin_path(), Some(file.as_c_str()));
        assert_eq!(viewed.as_ref().value(), Some(c"two"));
        assert_eq!(viewed.as_ref().name(), Some(c"multi.v"));
        assert_eq!(after.as_ref().value(), Some(c"after"));
        drop(after);
        assert_eq!(before.as_ref().backend_type(), Some(c"file"));
        drop((multi, viewed));
        assert_eq!(before.as_ref().name(), Some(c"test.key"));
        drop((snapped, before));
    }
}

#[cfg(not(miri))]
mod file_access {
    use super::*;
    use libgit2::libgit2::config::config::{
        ConfigLevel, find_global, find_system, find_xdg, parse_path,
    };
    use std::ffi::CString;

    #[test]
    fn file_api_sequences() {
        let file = scratch("file-api");
        std::fs::write(
            file.to_str().unwrap(),
            b"[a]\n flag\n empty =\n path = relative/path\n multi = first\n multi = second\n",
        )
        .unwrap();
        let mut cfg = Config::open(&file).unwrap();
        assert!(cfg.as_ref().get_bool(c"a.flag").unwrap());
        assert_eq!(
            cfg.as_ref()
                .get_string_buf(c"a.flag")
                .unwrap()
                .as_ref()
                .as_bytes(),
            b""
        );
        assert!(cfg.as_ref().get_path(c"a.flag").is_err());
        assert_eq!(
            cfg.as_ref()
                .get_path(c"a.path")
                .unwrap()
                .as_ref()
                .as_bytes(),
            b"relative/path"
        );
        assert!(cfg.as_ref().get_string(c"a.path").is_err());
        let retained = cfg.as_ref().get_string_buf(c"a.path").unwrap();
        for value in [i32::MIN, -1, 0, 1, i32::MAX] {
            cfg.as_mut().set_i32(c"a.number", value).unwrap();
            assert_eq!(cfg.as_ref().get_i32(c"a.number").unwrap(), value);
        }
        for value in [i64::MIN, i64::MAX, 0, -1] {
            cfg.as_mut().set_i64(c"a.number", value).unwrap();
            assert_eq!(cfg.as_ref().get_i64(c"a.number").unwrap(), value);
        }
        for value in [true, false] {
            cfg.as_mut().set_bool(c"a.flag", value).unwrap();
            assert_eq!(cfg.as_ref().get_bool(c"a.flag").unwrap(), value);
        }
        let input = CString::new("copied").unwrap();
        cfg.as_mut()
            .set_multivar(c"a.multi", c"first", &input)
            .unwrap();
        drop(input);
        cfg.as_mut().delete_multivar(c"a.multi", c"second").unwrap();
        assert_eq!(
            cfg.as_ref()
                .get_string_buf(c"a.multi")
                .unwrap()
                .as_ref()
                .as_bytes(),
            b"copied"
        );
        assert!(cfg.as_mut().set_multivar(c"a.multi", c"[", c"bad").is_err());
        assert!(cfg.as_mut().delete_multivar(c"a.multi", c"[").is_err());
        cfg.as_mut().delete_entry(c"a.multi").unwrap();
        assert!(cfg.as_ref().get_i32(c"a.multi").is_err());
        assert!(cfg.as_mut().delete_entry(c"a.missing").is_err());
        assert!(cfg.as_ref().get_i64(c"invalid").is_err());
        assert!(cfg.as_mut().set_i32(c"invalid", 0).is_err());
        let path = CString::new(file.to_bytes()).unwrap();
        assert!(
            cfg.as_mut()
                .add_file(&path, ConfigLevel::LOCAL, None, false)
                .is_err()
        );
        cfg.as_mut()
            .add_file(&path, ConfigLevel::LOCAL, None, true)
            .unwrap();
        drop(path);
        let mut parent = cfg.as_mut();
        let mut highest = parent.open_level(ConfigLevel::HIGHEST).unwrap();
        assert!(highest.as_mut().set_i32(c"a.view", 99).is_err());
        let extra = scratch("highest-max-priority");
        highest
            .as_mut()
            .add_file(&extra, ConfigLevel::from_raw(i32::MAX - 1), None, false)
            .unwrap();
        highest.as_mut().set_i32(c"a.extra", 3).unwrap();
        drop(highest);
        std::fs::remove_file(extra.to_str().unwrap()).unwrap();
        let mut view = parent.open_level(ConfigLevel::LOCAL).unwrap();
        view.as_mut().set_i32(c"a.view", 99).unwrap();
        view.as_mut().set_bool(c"a.tx", false).unwrap();
        view.as_mut()
            .set_multivar(c"a.txmulti", c".*", c"first")
            .unwrap();
        {
            let mut tx = view.as_mut().lock().unwrap();
            tx.set_i32(c"a.view", 100).unwrap();
            tx.set_i64(c"a.large", i64::MAX).unwrap();
            tx.set_bool(c"a.tx", true).unwrap();
            let staged = CString::new(b"staged \xff".to_vec()).unwrap();
            tx.set_string(c"a.txstring", &staged).unwrap();
            drop(staged);
            assert!(tx.config().get_string_buf(c"a.txstring").is_err());
            tx.set_multivar(c"a.txmulti", c".*", c"first").unwrap();
            tx.delete_multivar(c"a.txmulti", c"first").unwrap();
            tx.delete_entry(c"a.tx").unwrap();
            // C stages locked writes without updating the published values.
            assert_eq!(tx.config().get_i32(c"a.view").unwrap(), 99);
        } // abort staged changes and release the retained backend
        let written = std::fs::read(file.to_str().unwrap()).unwrap();
        assert!(!written.windows(3).any(|w| w == b"100"));
        assert!(!written.windows(6).any(|w| w == b"staged"));
        drop(view);
        // Aborted staged writes leave both disk and published values untouched.
        let expected = cfg.as_ref().get_i32(c"a.view").unwrap();
        let shared = cfg.as_ref().open_level(ConfigLevel::LOCAL).unwrap();
        assert_eq!(shared.as_ref().get_i32(c"a.view").unwrap(), expected);
        drop(shared);
        drop(cfg);
        assert_eq!(retained.as_ref().as_bytes(), b"relative/path");
        std::fs::remove_file(file.to_str().unwrap()).unwrap();
    }

    #[test]
    fn default_search_and_path_buffers() {
        let _cfg = Config::open_default().unwrap();
        // Environment-dependent find failures are valid results, but all returned
        // allocations must survive other calls and be dropped exactly once.
        let outputs = [find_global(), find_xdg(), find_system()];
        let copied = parse_path(c"relative/\xff").unwrap();
        for input in [c"", c"~", c"~/child", c"~named/child"] {
            let _ = parse_path(input);
        }
        for output in outputs.into_iter().flatten() {
            assert!(!output.as_ref().as_bytes().is_empty());
        }
        assert_eq!(copied.as_ref().as_bytes(), b"relative/\xff");
    }

    #[test]
    fn file_constructor_error_cleanup() {
        let malformed = scratch("malformed-input");
        std::fs::write(malformed.to_str().unwrap(), b"[unfinished\n").unwrap();
        assert!(Config::open(&malformed).is_err());
        let mut cfg = Config::new().unwrap();
        #[cfg(unix)]
        {
            let circular = scratch("circular-symlink");
            let path = circular.to_str().unwrap();
            std::os::unix::fs::symlink(path, path).unwrap();
            assert!(Config::open(&circular).is_err());
            assert!(
                cfg.as_mut()
                    .add_file(&circular, ConfigLevel::LOCAL, None, false)
                    .is_err()
            );
            std::fs::remove_file(path).unwrap();
        }

        assert!(
            cfg.as_mut()
                .add_file(&malformed, ConfigLevel::LOCAL, None, false)
                .is_err()
        );
        assert!(cfg.as_ref().get_bool(c"a.missing").is_err());
        assert!(cfg.as_ref().get_i32(c"a.missing").is_err());
        assert!(cfg.as_ref().get_i64(c"a.missing").is_err());
        drop(cfg);
        std::fs::remove_file(malformed.to_str().unwrap()).unwrap();
    }

    #[test]
    fn independent_file_owners_on_threads() {
        let threads: Vec<_> = (0..8)
            .map(|index| {
                std::thread::spawn(move || {
                    let file = scratch(&format!("file-thread-{index}"));
                    let mut cfg = Config::open(&file).unwrap();
                    for value in [i64::MIN, 0, i64::MAX] {
                        cfg.as_mut().set_i64(c"a.value", value).unwrap();
                        assert_eq!(cfg.as_ref().get_i64(c"a.value").unwrap(), value);
                    }
                    let out = cfg.as_ref().get_string_buf(c"a.value").unwrap();
                    drop(cfg);
                    std::fs::remove_file(file.to_str().unwrap()).unwrap();
                    out // buffer (not the config) may be released on the joining thread
                })
            })
            .collect();
        for thread in threads {
            assert!(!thread.join().unwrap().as_ref().as_bytes().is_empty());
        }
    }

    #[test]
    fn search_locations_in_isolated_environment() {
        let dir =
            std::env::temp_dir().join(format!("crustify-config-search-{}", std::process::id()));
        if std::env::var_os("CRUSTIFY_CONFIG_SEARCH_CHILD").is_some() {
            let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
            assert_eq!(
                find_global().unwrap().as_ref().as_bytes(),
                home.join(".gitconfig").to_str().unwrap().as_bytes()
            );
            let xdg = std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap());
            assert_eq!(
                find_xdg().unwrap().as_ref().as_bytes(),
                xdg.join("git/config").to_str().unwrap().as_bytes()
            );
            let cfg = Config::open_default().unwrap();
            assert_eq!(cfg.as_ref().get_i32(c"a.value").unwrap(), 11);
            assert_eq!(
                cfg.as_ref()
                    .open_global()
                    .unwrap()
                    .as_ref()
                    .get_i32(c"a.value")
                    .unwrap(),
                22
            );
            return;
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("xdg/git")).unwrap();
        std::fs::write(dir.join(".gitconfig"), b"[a]\n value = 11\n").unwrap();
        std::fs::write(dir.join("xdg/git/config"), b"[a]\n value = 22\n").unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "config::file_access::search_locations_in_isolated_environment",
                "--exact",
                "--nocapture",
            ])
            .env("HOME", &dir)
            .env("XDG_CONFIG_HOME", dir.join("xdg"))
            .env("CRUSTIFY_CONFIG_SEARCH_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn global_views_and_repository_keepalive() {
        let global = scratch("global-view");
        let xdg = scratch("xdg-view");
        let mut cfg = Config::new().unwrap();
        cfg.as_mut()
            .add_file(&global, ConfigLevel::GLOBAL, None, false)
            .unwrap();
        let mut parent = cfg.as_mut();
        let mut view = parent.open_global().unwrap();
        view.as_mut().set_i32(c"a.value", 1).unwrap();
        drop(view);
        parent
            .add_file(&xdg, ConfigLevel::XDG, None, false)
            .unwrap();
        let mut view = parent.open_global().unwrap();
        view.as_mut().set_i32(c"a.value", 2).unwrap();
        drop(view);
        let shared = cfg.as_ref().open_global().unwrap();
        assert_eq!(shared.as_ref().get_i32(c"a.value").unwrap(), 2);
        drop(shared);
        drop(cfg);
        for file in [global, xdg] {
            std::fs::remove_file(file.to_str().unwrap()).unwrap();
        }

        let dir =
            std::env::temp_dir().join(format!("crustify-safe-config-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = CString::new(dir.to_str().unwrap()).unwrap();
        let repo = Repository::init(&path, true).unwrap();
        let file = CString::new(dir.join("config").to_str().unwrap()).unwrap();
        let included = dir.join("included.cfg");
        std::fs::write(&included, b"[a]\n conditional = 41\n").unwrap();
        let mut contents = std::fs::read(dir.join("config")).unwrap();
        contents.extend_from_slice(
            format!(
                "\n[includeIf \"gitdir:{}\"]\n path = included.cfg\n",
                dir.display()
            )
            .as_bytes(),
        );
        std::fs::write(dir.join("config"), contents).unwrap();

        // The explicit annotation narrows Config::new's covariant static owner.
        fn bound<'r>(
            repo: libgit2::libgit2::repository::repository::RepositoryRef<'r>,
            file: &std::ffi::CStr,
        ) -> ConfigBox<'r> {
            let mut cfg: ConfigBox<'r> = Config::new().unwrap();
            cfg.as_mut()
                .add_file(file, ConfigLevel::LOCAL, Some(repo), false)
                .unwrap();
            cfg
        }
        let mut cfg = bound(repo.as_ref(), &file);
        assert_eq!(cfg.as_ref().get_i32(c"a.conditional").unwrap(), 41);
        std::fs::write(&included, b"[a]\n conditional = 12345\n").unwrap();
        assert_eq!(cfg.as_ref().get_i32(c"a.conditional").unwrap(), 12345);

        cfg.as_mut().set_bool(c"a.flag", true).unwrap();
        let snapshot = repo.as_ref().config_snapshot().unwrap();
        let text = snapshot.as_ref().get_string(c"a.flag").unwrap();
        drop(snapshot);
        assert_eq!(text, c"true");
        drop(cfg);
        drop(repo);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

// Constructs through the safe config and backend APIs now available at this wave tip.
fn string_file_config(tag: &str) -> (ConfigBox<'static>, std::ffi::CString) {
    let mut config = Config::new().unwrap();
    let file = scratch(tag);
    config
        .as_mut()
        .add_file(
            &file,
            libgit2::libgit2::config::config::ConfigLevel::LOCAL,
            None,
            false,
        )
        .unwrap();
    (config, file)
}

#[forbid(unsafe_code)]
mod writes_and_snapshots {
    use super::*;
    use std::ffi::CString;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn string_inputs_and_order_slices_are_not_retained() {
        let (mut config, file) = string_file_config("strings-not-retained");
        let mut name = CString::new("a.b").unwrap();
        let mut value = CString::new("123").unwrap();
        let mut levels = vec![
            ConfigLevel::LOCAL,
            ConfigLevel::LOCAL,
            ConfigLevel::from_raw(i32::MIN),
        ];
        let mut m = config.as_mut();
        m.set_write_order(&levels).unwrap();
        levels.clear();
        levels.shrink_to_fit();
        m.as_mut().set_string(&name, &value).unwrap();
        name = CString::new("other.name").unwrap();
        value = CString::new("999").unwrap();
        assert_eq!(number(config.as_ref(), c"a.b"), Ok(123));
        drop((name, value, levels));
        for value in [c"", c"  spaces # ; quotes \" slash \\ \n", c"\xff"] {
            config.as_mut().set_string(c"a.text", value).unwrap();
            config.as_mut().set_string(c"a.text", value).unwrap();
        }
        config.as_mut().set_write_order(&[]).unwrap();
        let error = config.as_mut().set_string(c"a.b", c"7").unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::ReadOnly));
        let copy = error.clone();
        config
            .as_mut()
            .set_write_order(&[ConfigLevel::LOCAL])
            .unwrap();
        config.as_mut().set_string(c"a.b", c"456").unwrap();
        assert_eq!(number(config.as_ref(), c"a.b"), Ok(456));
        drop(config);
        assert_eq!(error, copy);
        std::fs::remove_file(file.to_str().unwrap()).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn snapshot_copies_survive_source_and_repository_drop() {
        let empty = Config::new().unwrap();
        let mut snapshot = empty.as_ref().snapshot().unwrap();
        drop(empty);
        assert_eq!(
            snapshot
                .as_mut()
                .set_string(c"a.b", c"1")
                .unwrap_err()
                .code(),
            Some(ErrorCode::ReadOnly)
        );
        let (mut source, file) = string_file_config("snapshot-independent");
        source.as_mut().set_string(c"a.b", c"12").unwrap();
        let mut first = source.as_mut().as_ref().snapshot().unwrap();
        source.as_mut().set_string(c"a.b", c"34").unwrap();
        let shared = ConfigArc::from(source);
        let second = shared.as_ref().snapshot().unwrap();
        drop(shared);
        assert_eq!(number(first.as_ref(), c"a.b"), Ok(12));
        assert_eq!(number(second.as_ref(), c"a.b"), Ok(34));
        first
            .as_mut()
            .set_write_order(&[ConfigLevel::LOCAL])
            .unwrap();
        assert_eq!(
            first.as_mut().set_string(c"a.b", c"56").unwrap_err().code(),
            Some(ErrorCode::ReadOnly)
        );
        let third = first.as_ref().snapshot().unwrap();
        drop(first);
        assert_eq!(number(third.as_ref(), c"a.b"), Ok(12));
        std::fs::remove_file(file.to_str().unwrap()).unwrap();
        let path = CString::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/resources/testrepo.git"
        ))
        .unwrap();
        let repo = Repository::open(&path).unwrap();
        let config = repo.as_ref().config().unwrap();
        let snapshot = config.as_ref().snapshot().unwrap();
        drop(config);
        drop(repo);
        assert_eq!(
            number(snapshot.as_ref(), c"core.repositoryformatversion"),
            Ok(0)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn multivar_and_locked_file_failures_leave_owned_errors() {
        let (mut writer, file) = string_file_config("write-failures");
        std::fs::write(file.to_str().unwrap(), "[a]\n b = 1\n b = 2\n").unwrap();
        // A lookup refreshes the file backend cache after an external edit.
        assert!(number(writer.as_ref(), c"a.b").is_ok());
        let duplicate = writer.as_mut().set_string(c"a.b", c"3").unwrap_err();
        assert_eq!(
            duplicate.message(),
            c"entry is not unique due to being a multivar"
        );
        let mut blocker = Config::new().unwrap();
        blocker
            .as_mut()
            .add_file(&file, ConfigLevel::LOCAL, None, false)
            .unwrap();
        let lock = blocker.as_mut().lock().unwrap();
        let locked = writer.as_mut().set_string(c"a.unique", c"3").unwrap_err();
        assert_eq!(locked.code(), Some(ErrorCode::Locked));
        drop(lock);
        drop(blocker);
        writer.as_mut().set_string(c"a.unique", c"3").unwrap();
        drop(writer);
        assert_eq!(
            duplicate.message(),
            c"entry is not unique due to being a multivar"
        );
        assert_eq!(locked.code(), Some(ErrorCode::Locked));
        std::fs::remove_file(file.to_str().unwrap()).unwrap();
    }

    /// A delete without a writer reports an error the wrapper set itself, which the
    /// `ConfigError` owns: it outlives the config and later errors on the thread.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn readonly_delete_errors_are_owned() {
        use libgit2::libgit2::util::errors::ErrorClass;
        use libgit2::libgit2::util::sys::errors::set_str;
        let mut empty = Config::new().unwrap();
        let mut snapshot = empty.as_ref().snapshot().unwrap();
        let name = CString::new("a.temporary").unwrap();
        let errors = [
            empty.as_mut().delete_entry(&name).unwrap_err(),
            snapshot.as_mut().delete_multivar(&name, c".*").unwrap_err(),
        ];
        drop((empty, snapshot, name));
        set_str(ErrorClass::Odb, c"a later, unrelated failure").unwrap();
        for error in errors {
            assert_eq!(error.code(), Some(ErrorCode::ReadOnly));
            assert_eq!(error.class(), Some(ErrorClass::Config));
            assert_eq!(
                error.message(),
                c"cannot delete 'a.temporary': the configuration is read-only"
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn independent_writes_and_snapshots_on_threads() {
        thread::scope(|s| {
            for worker in 0..4 {
                s.spawn(move || {
                    let (mut source, file) = string_file_config(&format!("thread-{worker}"));
                    source.as_mut().set_string(c"a.b", c"123").unwrap();
                    let snapshot = source.as_ref().snapshot().unwrap();
                    drop(source);
                    assert_eq!(number(snapshot.as_ref(), c"a.b"), Ok(123));
                    drop(snapshot);
                    std::fs::remove_file(file.to_str().unwrap()).unwrap();
                });
            }
        });
    }
}

/// Public-only mapping and callback lifetime/foreign-write regressions.
mod mapping_callbacks {
    use super::*;
    use libgit2::libgit2::config::config::{
        ConfigMap, EntryRef, ForeachCallback, MapMatch, lookup_map_value,
    };
    use std::ffi::CString;
    use std::num::NonZero;
    use std::ops::ControlFlow;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mapping_borrows_tables_and_returns_owned_results() {
        let text = CString::new("input").unwrap();
        let mut maps = [
            ConfigMap::new(MapMatch::String(&text), i32::MIN),
            ConfigMap::new(MapMatch::Int32, i32::MAX),
        ];
        assert_eq!(lookup_map_value(&maps, Some(c"INPUT")).unwrap(), i32::MIN);
        assert_eq!(lookup_map_value(&maps, Some(c"-2k")).unwrap(), -2048);
        let held = lookup_map_value(&maps, Some(c"input")).unwrap();
        maps[0].as_mut().set_map_value(17);
        maps[1].as_mut().set_match(MapMatch::True);
        assert_eq!(held, i32::MIN);
        assert_eq!(lookup_map_value(&maps, None).unwrap(), i32::MAX);
        assert!(lookup_map_value(&[], None).is_err());
        assert!(lookup_map_value(&maps, Some(c"\xff")).is_err());
        drop(text);
        assert_eq!(held, i32::MIN);
        let falsy = [ConfigMap::new(MapMatch::False, 7)];
        assert_eq!(lookup_map_value(&falsy, Some(c"")).unwrap(), 7);
        assert!(lookup_map_value(&falsy, None).is_err());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mapped_refresh_errors_do_not_reach_uninitialized_entry() {
        let path = scratch("mapped-refresh");
        std::fs::write(path.to_str().unwrap(), b"[a]\n b = input\n").unwrap();
        let config = Config::open(&path).unwrap();
        let maps = [ConfigMap::new(MapMatch::String(c"input"), 9)];
        assert_eq!(config.as_ref().get_mapped(c"a.b", &maps).unwrap(), 9);
        std::fs::write(
            path.to_str().unwrap(),
            b"[invalid\nthis is a longer malformed config\n",
        )
        .unwrap();
        assert!(config.as_ref().get_mapped(c"a.b", &maps).is_err());
        std::fs::write(path.to_str().unwrap(), b"[a]\n b = input\n c = recovered\n").unwrap();
        assert_eq!(config.as_ref().get_mapped(c"a.b", &maps).unwrap(), 9);
        let shared = ConfigArc::from(config);
        assert_eq!(shared.as_ref().get_mapped(c"a.b", &maps).unwrap(), 9);
        let snapshot = shared.as_ref().snapshot().unwrap();
        drop(shared);
        std::fs::remove_file(path.to_str().unwrap()).unwrap();
        assert_eq!(snapshot.as_ref().get_mapped(c"a.b", &maps).unwrap(), 9);
        assert!(snapshot.as_ref().get_mapped(c"a.b", &[]).is_err());
        assert!(snapshot.as_ref().get_mapped(c"a.missing", &maps).is_err());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn callback_parent_drop_nested_calls_and_panic_reuse() {
        let path = scratch("foreach-callback");
        std::fs::write(path.to_str().unwrap(), b"[a]\n b = input\n flag\n").unwrap();
        let mut parent = Some(Config::open(&path).unwrap());
        let entry = parent.as_ref().unwrap().as_ref().entry(c"a.b").unwrap();
        let flag = parent.as_ref().unwrap().as_ref().entry(c"a.flag").unwrap();
        let mut seen = Vec::new();
        let mut inner = |e: EntryRef<'_>| {
            seen.push(e.name().unwrap().to_owned());
            ControlFlow::Continue(())
        };
        let mut nested = ForeachCallback::new(&mut inner);
        let mut outer = |e: EntryRef<'_>| {
            drop(parent.take());
            assert_eq!(e.value(), Some(c"input"));
            assert_eq!(nested.call(flag.as_ref()), ControlFlow::Continue(()));
            ControlFlow::Break(NonZero::new(37).unwrap())
        };
        let mut callback = ForeachCallback::new(&mut outer);
        assert_eq!(
            callback.call(entry.as_ref()),
            ControlFlow::Break(NonZero::new(37).unwrap())
        );
        assert_eq!(callback.stopped().unwrap().get(), 37);
        drop(callback);
        drop(outer);
        drop(nested);
        assert_eq!(seen, [c"a.flag".to_owned()]);
        let mut first = true;
        let mut panicking = |e: EntryRef<'_>| {
            if std::mem::take(&mut first) {
                panic!("callback panic");
            }
            assert_eq!(e.value(), Some(c"input"));
            ControlFlow::Continue(())
        };
        let mut callback = ForeachCallback::new(&mut panicking);
        assert!(catch_unwind(AssertUnwindSafe(|| callback.call(entry.as_ref()))).is_err());
        assert_eq!(callback.stopped().unwrap().get(), ErrorCode::User.raw());
        assert_eq!(callback.call(entry.as_ref()), ControlFlow::Continue(()));
        assert_eq!(callback.stopped(), None);
        std::fs::remove_file(path.to_str().unwrap()).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mapping_tables_can_be_read_on_scoped_threads() {
        let input = CString::new("input").unwrap();
        let maps = [
            ConfigMap::new(MapMatch::String(&input), 11),
            ConfigMap::new(MapMatch::Int32, 0),
        ];
        thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..32 {
                        assert_eq!(lookup_map_value(&maps, Some(c"INPUT")).unwrap(), 11);
                        assert_eq!(lookup_map_value(&maps, Some(c"2k")).unwrap(), 2048);
                    }
                });
            }
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mapped_lower_backend_failure_preserves_the_previous_error() {
        use libgit2::libgit2::util::errors::{ErrorClass, last};
        use libgit2::libgit2::util::sys::errors::{clear, set};
        let lower = scratch("mapped-lower");
        let upper = scratch("mapped-upper");
        for seeded in [false, true] {
            std::fs::write(lower.to_str().unwrap(), b"[a]\n other = old\n").unwrap();
            std::fs::write(upper.to_str().unwrap(), b"[a]\n b = input\n").unwrap();
            let mut config = Config::open(&lower).unwrap();
            config
                .as_mut()
                .add_file(&upper, ConfigLevel::APP, None, false)
                .unwrap();
            std::fs::write(
                lower.to_str().unwrap(),
                b"[broken\nlonger malformed backend\n",
            )
            .unwrap();
            clear();
            if seeded {
                set(ErrorClass::Odb, c"previous error");
            }
            let before = last();
            let maps = [ConfigMap::new(MapMatch::String(c"input"), 42)];
            assert_eq!(config.as_ref().get_mapped(c"a.b", &maps).unwrap(), 42);
            assert_eq!(last(), before);
        }
        for path in [&lower, &upper] {
            std::fs::remove_file(path.to_str().unwrap()).unwrap();
        }
    }
}

mod enumeration {
    use super::*;
    use libgit2::libgit2::config::config::{EntryRef, ForeachCallback};
    use std::num::NonZero;
    use std::ops::ControlFlow;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn input(tag: &str) -> (std::ffi::CString, ConfigBox<'static>) {
        let path = scratch(tag);
        std::fs::write(
            path.to_str().unwrap(),
            b"[A]\n m = one\n m = two\n flag\n empty =\n bytes = \xff\n",
        )
        .unwrap();
        let config = Config::open(&path).unwrap();
        (path, config)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_nested_shared_and_mutable_reborrows() {
        let (path, mut config) = input("enumeration-nested");
        let m = config.as_mut();
        let view = m.as_ref();
        let mut seen = Vec::new();
        let mut inner = |e: EntryRef<'_>| {
            seen.push(e.value().map(CStr::to_owned));
            ControlFlow::Continue(())
        };
        let mut nested = ForeachCallback::new(&mut inner);
        let mut calls = 0;
        let mut outer = |e: EntryRef<'_>| {
            calls += 1;
            // Reentry is into a distinct callback/iterator, never the active closure.
            assert_eq!(
                view.foreach_multivar(c"A.M", None, &mut nested).unwrap(),
                ControlFlow::Continue(())
            );
            assert!(e.name().is_some());
            ControlFlow::Continue(())
        };
        let mut cb = ForeachCallback::new(&mut outer);
        assert_eq!(view.foreach(&mut cb).unwrap(), ControlFlow::Continue(()));
        drop(cb);
        drop(outer);
        drop(nested);
        assert_eq!(calls, 5);
        assert_eq!(seen.len(), 10);
        assert_eq!(seen[0], Some(c"one".to_owned()));
        assert_eq!(seen[1], Some(c"two".to_owned()));
        drop(config);
        std::fs::remove_file(path.to_str().unwrap()).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_stops_reuse_null_empty_and_non_utf8() {
        let (path, config) = input("enumeration-stops");
        let shared = ConfigArc::from(config);
        for code in [
            1,
            -1,
            ErrorCode::User.raw(),
            ffi_iterover(),
            i32::MIN,
            i32::MAX,
        ] {
            let code = NonZero::new(code).unwrap();
            let mut stop = |_: EntryRef<'_>| ControlFlow::Break(code);
            let mut cb = ForeachCallback::new(&mut stop);
            assert_eq!(
                shared.as_ref().foreach(&mut cb).unwrap(),
                ControlFlow::Break(code)
            );
            assert_eq!(
                shared
                    .as_ref()
                    .foreach_match(Some(c"^a\\.m$"), &mut cb)
                    .unwrap(),
                ControlFlow::Break(code)
            );
            assert_eq!(
                shared
                    .as_ref()
                    .foreach_multivar(c"a.m", None, &mut cb)
                    .unwrap(),
                ControlFlow::Break(code)
            );
            assert_eq!(
                shared
                    .as_ref()
                    .foreach_match(Some(c"no-matches"), &mut cb)
                    .unwrap(),
                ControlFlow::Continue(())
            );
            assert_eq!(cb.stopped(), None);
        }
        let mut values = Vec::new();
        let mut visit = |e: EntryRef<'_>| {
            values.push((e.name().unwrap().to_owned(), e.value().map(CStr::to_owned)));
            ControlFlow::Continue(())
        };
        assert_eq!(
            shared
                .as_ref()
                .foreach_match(None, &mut ForeachCallback::new(&mut visit))
                .unwrap(),
            ControlFlow::Continue(())
        );
        assert!(values.contains(&(c"a.flag".to_owned(), None)));
        assert!(values.contains(&(c"a.empty".to_owned(), Some(c"".to_owned()))));
        assert!(values.contains(&(c"a.bytes".to_owned(), Some(c"\xff".to_owned()))));
        let mut values = Vec::new();
        let mut visit = |e: EntryRef<'_>| {
            values.push(e.value().map(CStr::to_owned));
            ControlFlow::Continue(())
        };
        // No C regex ever sees the NULL value, even when filtering in the closure.
        assert_eq!(
            shared
                .as_ref()
                .foreach_multivar(c"a.flag", None, &mut ForeachCallback::new(&mut visit))
                .unwrap(),
            ControlFlow::Continue(())
        );
        assert_eq!(values, [None]);
        drop(shared);
        std::fs::remove_file(path.to_str().unwrap()).unwrap();
    }

    fn ffi_iterover() -> i32 {
        ErrorCode::IterOver.raw()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_panics_cleanup_then_reuse_all_variants() {
        let (path, config) = input("enumeration-panic");
        for mode in 0..3 {
            let mut first = true;
            let mut calls = 0;
            let mut f = |_: EntryRef<'_>| {
                calls += 1;
                if std::mem::take(&mut first) {
                    panic!("enumeration panic");
                }
                ControlFlow::Continue(())
            };
            let mut cb = ForeachCallback::new(&mut f);
            let walk = |cb: &mut ForeachCallback<'_>| match mode {
                0 => config.as_ref().foreach(cb),
                1 => config.as_ref().foreach_match(Some(c".*"), cb),
                _ => config.as_ref().foreach_multivar(c"a.m", None, cb),
            };
            assert!(catch_unwind(AssertUnwindSafe(|| walk(&mut cb))).is_err());
            assert_eq!(cb.stopped().unwrap().get(), ErrorCode::User.raw());
            assert_eq!(walk(&mut cb).unwrap(), ControlFlow::Continue(()));
            assert_eq!(cb.stopped(), None);
            drop(cb);
            assert_eq!(calls, if mode == 2 { 3 } else { 6 });
        }
        drop(config);
        std::fs::remove_file(path.to_str().unwrap()).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_errors_and_empty_walks_keep_callback_usable() {
        let empty = Config::new().unwrap();
        let mut calls = 0;
        let mut f = |_: EntryRef<'_>| {
            calls += 1;
            ControlFlow::Continue(())
        };
        let mut cb = ForeachCallback::new(&mut f);
        assert_eq!(
            empty.as_ref().foreach(&mut cb).unwrap(),
            ControlFlow::Continue(())
        );
        assert_eq!(
            empty.as_ref().foreach_match(Some(c""), &mut cb).unwrap(),
            ControlFlow::Continue(())
        );
        assert!(empty.as_ref().foreach_match(Some(c"["), &mut cb).is_err());
        assert_eq!(
            empty
                .as_ref()
                .foreach_multivar(c"a.m", None, &mut cb)
                .unwrap_err()
                .code(),
            Some(ErrorCode::NotFound)
        );
        assert!(
            empty
                .as_ref()
                .foreach_multivar(c"invalid", None, &mut cb)
                .is_err()
        );
        drop(cb);
        assert_eq!(calls, 0);
        for mode in 0..3 {
            let (path, config) = input(&format!("enumeration-refresh-{mode}"));
            std::fs::write(
                path.to_str().unwrap(),
                b"[broken\na much longer malformed file\n",
            )
            .unwrap();
            let mut f = |_: EntryRef<'_>| ControlFlow::Continue(());
            let mut cb = ForeachCallback::new(&mut f);
            let walk = |cb: &mut ForeachCallback<'_>| match mode {
                0 => config.as_ref().foreach(cb),
                1 => config.as_ref().foreach_match(Some(c".*"), cb),
                _ => config.as_ref().foreach_multivar(c"a.m", None, cb),
            };
            assert!(walk(&mut cb).is_err());
            assert_eq!(cb.stopped(), None);
            std::fs::write(
                path.to_str().unwrap(),
                b"[a]\n m = repaired and longer than the malformed file contents\n",
            )
            .unwrap();
            assert_eq!(walk(&mut cb).unwrap(), ControlFlow::Continue(()));
            drop(config);
            std::fs::remove_file(path.to_str().unwrap()).unwrap();
        }
    }

    fn filter_input(tag: &str) -> (std::ffi::CString, ConfigBox<'static>) {
        let path = scratch(tag);
        std::fs::write(
            path.to_str().unwrap(),
            b"[A]\n m = one\n m\n m = two\n m =\n flag\n",
        )
        .unwrap();
        let config = Config::open(&path).unwrap();
        (path, config)
    }

    /// C's value filter calls strlen(NULL) on a valueless setting; the wrapper's
    /// skips it. Invalid patterns release everything (LSan), and stops, panics and
    /// nested filtered walks leave the callback reusable.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_value_filter_skips_valueless_and_cleans_up() {
        let (path, config) = filter_input("enumeration-filter");
        let walk = |name: &CStr, regexp: &CStr| {
            let mut values = Vec::new();
            let mut f = |e: EntryRef<'_>| {
                values.push(e.value().map(CStr::to_owned));
                ControlFlow::Continue(())
            };
            let result = config
                .as_ref()
                .foreach_multivar(name, Some(regexp), &mut ForeachCallback::new(&mut f))
                .map_err(|e| e.code());
            (result, values)
        };
        let some = |v: &CStr| Some(v.to_owned());
        for regexp in [c"", c".", c"^$"] {
            assert_eq!(
                walk(c"a.flag", regexp),
                (Err(Some(ErrorCode::NotFound)), vec![])
            );
        }
        assert_eq!(
            walk(c"a.m", c""),
            (
                Ok(ControlFlow::Continue(())),
                vec![some(c"one"), some(c"two"), some(c"")]
            )
        );
        assert_eq!(
            walk(c"A.M", c"^$"),
            (Ok(ControlFlow::Continue(())), vec![some(c"")])
        );
        assert_eq!(
            walk(c"a.m", c"^t"),
            (Ok(ControlFlow::Continue(())), vec![some(c"two")])
        );
        for name in [c"a.m", c"a.flag", c"missing.key"] {
            let (result, values) = walk(name, c"[");
            assert_eq!(
                (result, values),
                (Err(Some(ErrorCode::InvalidSpec)), vec![])
            );
        }
        assert_eq!(
            walk(c"invalid", c"["),
            (Err(Some(ErrorCode::InvalidSpec)), vec![])
        );

        let shared = ConfigArc::from(config);
        for code in [
            1,
            -1,
            ErrorCode::User.raw(),
            ffi_iterover(),
            i32::MIN,
            i32::MAX,
        ] {
            let code = NonZero::new(code).unwrap();
            let mut stop = |_: EntryRef<'_>| ControlFlow::Break(code);
            let mut cb = ForeachCallback::new(&mut stop);
            assert_eq!(
                shared
                    .as_ref()
                    .foreach_multivar(c"a.m", Some(c"o"), &mut cb)
                    .unwrap(),
                ControlFlow::Break(code)
            );
            assert_eq!(
                shared
                    .as_ref()
                    .foreach_multivar(c"a.m", Some(c"zzz"), &mut cb)
                    .unwrap_err()
                    .code(),
                Some(ErrorCode::NotFound)
            );
            assert_eq!(cb.stopped(), None);
            // A failed construction also forgets the previous walk's stop.
            let stopped = shared
                .as_ref()
                .foreach_multivar(c"a.m", Some(c"o"), &mut cb);
            assert_eq!(stopped.unwrap(), ControlFlow::Break(code));
            let invalid = shared
                .as_ref()
                .foreach_multivar(c"a.m", Some(c"["), &mut cb);
            assert_eq!(invalid.unwrap_err().code(), Some(ErrorCode::InvalidSpec));
            assert_eq!(cb.stopped(), None);
        }

        let mut first = true;
        let mut calls = 0;
        let mut f = |_: EntryRef<'_>| {
            calls += 1;
            if std::mem::take(&mut first) {
                panic!("filtered enumeration panic");
            }
            ControlFlow::Continue(())
        };
        let mut cb = ForeachCallback::new(&mut f);
        let view = shared.as_ref();
        let filtered = |cb: &mut ForeachCallback<'_>| view.foreach_multivar(c"a.m", Some(c""), cb);
        assert!(catch_unwind(AssertUnwindSafe(|| filtered(&mut cb))).is_err());
        assert_eq!(cb.stopped().unwrap().get(), ErrorCode::User.raw());
        assert_eq!(filtered(&mut cb).unwrap(), ControlFlow::Continue(()));
        drop(cb);
        assert_eq!(calls, 4);

        let mut inner_values = Vec::new();
        let mut inner = |e: EntryRef<'_>| {
            inner_values.push(e.value().map(CStr::to_owned));
            ControlFlow::Continue(())
        };
        let mut nested = ForeachCallback::new(&mut inner);
        let mut outer = |e: EntryRef<'_>| {
            let before = e.value().map(CStr::to_owned);
            assert_eq!(
                view.foreach_multivar(c"a.m", Some(c"^o"), &mut nested)
                    .unwrap(),
                ControlFlow::Continue(())
            );
            // The outer entry stays pinned across the nested walk's refresh.
            assert_eq!(e.value().map(CStr::to_owned), before);
            ControlFlow::Continue(())
        };
        assert_eq!(
            view.foreach_multivar(c"a.m", Some(c"."), &mut ForeachCallback::new(&mut outer))
                .unwrap(),
            ControlFlow::Continue(())
        );
        drop(nested);
        assert_eq!(inner_values, [some(c"one"), some(c"one")]);
        drop(shared);
        std::fs::remove_file(path.to_str().unwrap()).unwrap();
    }

    /// A repository's config (whose file backends evaluate `includeIf` through the
    /// repository on every reparse), its level view and snapshots: the callback moves
    /// HEAD and rewrites the file, so a nested walk reparses under the outer entry.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_repository_level_and_snapshot_configs() {
        use std::ffi::CString;
        let dir = std::env::temp_dir().join(format!(
            "crustify-ub-config-enumeration-repo-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = CString::new(dir.to_str().unwrap()).unwrap();
        let repo = Repository::init(&path, false).unwrap();
        let config_path = dir.join(".git/config");
        let base = std::fs::read(&config_path).unwrap();
        let mut text = base.clone();
        text.extend_from_slice(
            b"[includeIf \"onbranch:main\"]\n path = on-main\n\
              [includeIf \"onbranch:other\"]\n path = on-other\n[a]\n m = local\n",
        );
        std::fs::write(&config_path, &text).unwrap();
        std::fs::write(dir.join(".git/on-main"), b"[a]\n m = main\n").unwrap();
        std::fs::write(dir.join(".git/on-other"), b"[a]\n m = other-branch\n").unwrap();
        repo.as_ref().set_head(c"refs/heads/main").unwrap();
        // Moving HEAD may have parsed the file already; reparse it on main.
        text.extend_from_slice(b"# on main\n");
        std::fs::write(&config_path, &text).unwrap();

        let config = repo.as_ref().config().unwrap();
        let mut inner_values = Vec::new();
        let mut inner = |e: EntryRef<'_>| {
            inner_values.push((e.value().map(CStr::to_owned), e.include_depth()));
            ControlFlow::Continue(())
        };
        let mut nested = ForeachCallback::new(&mut inner);
        let mut outer_values = Vec::new();
        let mut outer = |e: EntryRef<'_>| {
            let value = e.value().map(CStr::to_owned);
            repo.as_ref().set_head(c"refs/heads/other").unwrap();
            text.extend_from_slice(b"# grow the file so its stamp changes\n");
            std::fs::write(&config_path, &text).unwrap();
            assert!(
                config
                    .as_ref()
                    .foreach_multivar(c"a.m", None, &mut nested)
                    .unwrap()
                    .is_continue()
            );
            assert_eq!(e.value().map(CStr::to_owned), value);
            outer_values.push((value, e.include_depth()));
            ControlFlow::Continue(())
        };
        assert!(
            config
                .as_ref()
                .foreach_match(Some(c"^a\\.m$"), &mut ForeachCallback::new(&mut outer))
                .unwrap()
                .is_continue()
        );
        drop(outer);
        drop(nested);
        let some = |v: &CStr| Some(v.to_owned());
        assert_eq!(outer_values, [(some(c"main"), 1), (some(c"local"), 0)]);
        // Each nested walk reparsed under the moved HEAD.
        assert_eq!(
            &inner_values[..2],
            [(some(c"other-branch"), 1), (some(c"local"), 0)]
        );

        let level = config.as_ref().open_level(ConfigLevel::LOCAL).unwrap();
        let snapshot = level.as_ref().snapshot().unwrap();
        let repo_snapshot = repo.as_ref().config_snapshot().unwrap();
        for view in [level.as_ref(), snapshot.as_ref(), repo_snapshot.as_ref()] {
            let mut values = Vec::new();
            let mut f = |e: EntryRef<'_>| {
                values.push(e.value().map(CStr::to_owned));
                ControlFlow::Continue(())
            };
            let mut cb = ForeachCallback::new(&mut f);
            assert!(view.foreach(&mut cb).unwrap().is_continue());
            assert!(
                view.foreach_multivar(c"a.m", Some(c"^o"), &mut cb)
                    .unwrap()
                    .is_continue()
            );
            drop(cb);
            assert!(values.contains(&some(c"other-branch")));
        }
        drop(level);
        drop(config);
        drop(snapshot);
        drop(repo_snapshot);
        drop(repo);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Independent configs walk concurrently (TSan): every walk's iterator, regex and
    /// error state is the calling thread's own.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_independent_configs_walk_on_threads() {
        let barrier = Barrier::new(4);
        thread::scope(|s| {
            for worker in 0..4 {
                let barrier = &barrier;
                s.spawn(move || {
                    let (path, config) = filter_input(&format!("enumeration-thread-{worker}"));
                    let mut calls = 0;
                    let mut f = |_: EntryRef<'_>| {
                        calls += 1;
                        ControlFlow::Continue(())
                    };
                    let mut cb = ForeachCallback::new(&mut f);
                    barrier.wait();
                    for _ in 0..8 {
                        let view = config.as_ref();
                        assert!(view.foreach(&mut cb).unwrap().is_continue());
                        assert!(
                            view.foreach_match(Some(c"^a\\."), &mut cb)
                                .unwrap()
                                .is_continue()
                        );
                        assert!(
                            view.foreach_multivar(c"a.m", None, &mut cb)
                                .unwrap()
                                .is_continue()
                        );
                        assert!(
                            view.foreach_multivar(c"a.m", Some(c"o"), &mut cb)
                                .unwrap()
                                .is_continue()
                        );
                        assert!(view.foreach_match(Some(c"["), &mut cb).is_err());
                    }
                    drop(cb);
                    assert_eq!(calls, 8 * (5 + 5 + 4 + 2));
                    drop(config);
                    std::fs::remove_file(path.to_str().unwrap()).unwrap();
                });
            }
        });
    }
}
