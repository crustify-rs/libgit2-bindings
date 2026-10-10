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
//! another thread, a handle outliving its owner.

use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::config::config::{Config, ConfigArc, ConfigBox};
use libgit2::libgit2::util::errors::ErrorCode;

use crate::fixture::commit_and_config::{
    OpenRepo, TESTREPO, config, get_int32, write_through_file,
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
    drop(repo);
    assert_eq!(
        get_int32(third.as_c_ptr(), c"core.repositoryformatversion"),
        Ok(0)
    );
    drop(third);
    let _ = std::fs::remove_file(file.to_str().unwrap());
}

/// A repository's config used and dropped after the repository (the parent first).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn repository_config_outlives_its_repository() {
    let repo = OpenRepo::open(TESTREPO);
    let shared = config(&repo);
    drop(repo);
    assert_eq!(
        get_int32(shared.as_c_ptr(), c"core.repositoryformatversion"),
        Ok(0)
    );
    assert_eq!(
        get_int32(shared.as_c_ptr(), c"core.missing"),
        Err(NOT_FOUND)
    );
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
