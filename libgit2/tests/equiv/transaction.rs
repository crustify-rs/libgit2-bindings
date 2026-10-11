//! Independently owned C/Rust transaction inputs; compare lifecycle effects after
//! every lock/write/drop step. Raw manipulation is only for not-yet-wrapped symbols.
use core::ptr;
use libgit2::ffi;
use libgit2::libgit2::config::config::{Config, MapType};
use libgit2::libgit2::refs::transaction::Transaction;
use libgit2::libgit2::repository::repository::Repository;
use std::ffi::{CStr, CString};

fn path(tag: &str) -> (std::path::PathBuf, CString) {
    let p = std::env::temp_dir().join(format!("crustify-equiv-tx-{}-{tag}", std::process::id()));
    let c = CString::new(p.to_str().unwrap()).unwrap();
    (p, c)
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reference_construct_lock_abort_and_relock() {
    let (rp, rc) = path("refs-rust");
    let (cp, cc) = path("refs-c");
    let _ = std::fs::remove_dir_all(&rp);
    let _ = std::fs::remove_dir_all(&cp);
    let repo = Repository::init(&rc, true).unwrap();
    let c_repo = Repository::init(&cc, true).unwrap();
    let mut rust = Transaction::new(repo.as_ref()).unwrap();
    let mut raw = ptr::null_mut();
    // SAFETY: independently owned live C-side repository and writable output;
    // the resulting raw transaction is released before that repository below.
    let status = unsafe { ffi::git_transaction_new(&mut raw, c_repo.as_c_ptr()) };
    assert_eq!(status, 0);
    for tx in [rust.as_c_ptr(), raw] {
        // SAFETY: live reference transaction and NUL-terminated name; each
        // transaction operates on its independently allocated repository/refdb.
        let status = unsafe { ffi::git_transaction_lock_ref(tx, c"refs/heads/topic".as_ptr()) };
        assert_eq!(status, 0);
        // SAFETY: live transaction; an unlocked name returns NotFound.
        let status = unsafe { ffi::git_transaction_remove(tx, c"refs/heads/missing".as_ptr()) };
        assert_eq!(status, ffi::GIT_ENOTFOUND);
    }
    for p in [&rp, &cp] {
        assert!(p.join("refs/heads/topic.lock").exists());
    }
    let _ = rust.as_mut().as_ref();
    drop(rust);
    // SAFETY: consumes the sole raw transaction before its independent parent.
    unsafe { ffi::git_transaction_free(raw) };
    for p in [&rp, &cp] {
        assert!(!p.join("refs/heads/topic.lock").exists());
    }
    // Both can lock again; committing an unmodified lock releases it once.
    let rust = Transaction::new(repo.as_ref()).unwrap();
    // SAFETY: writable output and live C-side repository; prior raw owner is freed.
    let status = unsafe { ffi::git_transaction_new(&mut raw, c_repo.as_c_ptr()) };
    assert_eq!(status, 0);
    for tx in [rust.as_c_ptr(), raw] {
        // SAFETY: live reference transaction, exactly one lock and one commit;
        // the parent stays alive through release after commit.
        unsafe {
            assert_eq!(
                ffi::git_transaction_lock_ref(tx, c"refs/heads/topic".as_ptr()),
                0
            );
            assert_eq!(ffi::git_transaction_commit(tx), 0);
        }
    }
    for p in [&rp, &cp] {
        assert!(!p.join("refs/heads/topic.lock").exists());
    }
    drop(rust);
    // SAFETY: releases raw exactly once after commit, before its parent.
    unsafe { ffi::git_transaction_free(raw) };
    drop(repo);
    drop(c_repo);
    std::fs::remove_dir_all(rp).unwrap();
    std::fs::remove_dir_all(cp).unwrap();
}

fn add_and_write(cfg: *mut ffi::git_config, path: &CStr) {
    // SAFETY: callers give live configs created independently with no backend;
    // file path/name are NUL terminated. No parent repository is retained.
    unsafe {
        assert_eq!(
            ffi::git_config_add_file_ondisk(
                cfg,
                path.as_ptr(),
                ffi::GIT_CONFIG_LEVEL_LOCAL,
                ptr::null(),
                0
            ),
            0
        );
        assert_eq!(ffi::git_config_set_int32(cfg, c"a.b".as_ptr(), 4), 0);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn config_construct_stage_abort_and_relock() {
    let mut rust = Config::new().unwrap();
    let mut raw = ptr::null_mut();
    // SAFETY: runtime pinned by Config::new; raw receives an owned config.
    let status = unsafe { ffi::git_config_new(&mut raw) };
    assert_eq!(status, 0);
    let mut ctx = ptr::null_mut();
    assert!(rust.as_mut().lock().is_err());
    // SAFETY: initialized empty raw config and writable transaction output.
    let status = unsafe { ffi::git_config_lock(&mut ctx, raw) };
    assert_eq!(status, ffi::GIT_EREADONLY);
    let (rp, rc) = path("rust-config");
    let (cp, cc) = path("c-config");
    let _ = std::fs::remove_file(&rp);
    let _ = std::fs::remove_file(&cp);
    let rust_raw = rust.as_c_ptr();
    add_and_write(rust_raw, &rc);
    add_and_write(raw, &cc);
    assert_eq!(std::fs::read(&rp).unwrap(), std::fs::read(&cp).unwrap());
    drop(rust.as_mut().lock().unwrap()); // successful temporary owner aborts immediately
    for p in [&rp, &cp] {
        std::fs::write(p.with_extension("lock"), "held").unwrap();
    }
    let rust_error = rust.as_mut().lock().unwrap_err();
    // SAFETY: the raw config and output slot are live; its lock is held by the
    // independent filesystem fixture, so C returns an error without an owner.
    let status = unsafe { ffi::git_config_lock(&mut ctx, raw) };
    assert_eq!(status, ffi::GIT_ELOCKED);
    // SAFETY: the failed call leaves a thread-local error, read before another call.
    let raw_error = unsafe { ffi::git_error_last() };
    // SAFETY: the error struct and NUL-terminated message remain live here.
    unsafe {
        assert_eq!(rust_error.raw_class(), (*raw_error).klass);
        let rust_message = rust_error
            .message()
            .to_string_lossy()
            .replace(rc.to_str().unwrap(), "<path>");
        let raw_message = CStr::from_ptr((*raw_error).message)
            .to_string_lossy()
            .replace(cc.to_str().unwrap(), "<path>");
        assert_eq!(rust_message, raw_message);
    }
    for p in [&rp, &cp] {
        assert_eq!(
            std::fs::read_to_string(p.with_extension("lock")).unwrap(),
            "held"
        );
        std::fs::remove_file(p.with_extension("lock")).unwrap();
    }
    for value in [8, 19] {
        let tx = rust.as_mut().lock().unwrap();
        // SAFETY: live writable config and output, no outstanding raw lock.
        let status = unsafe { ffi::git_config_lock(&mut ctx, raw) };
        assert_eq!(status, 0);
        for cfg in [rust_raw, raw] {
            // SAFETY: each independently owned config is locked; setter stages a
            // copy, retaining no Rust pointer. Raw accesses form no Rust reborrow.
            let status = unsafe { ffi::git_config_set_int32(cfg, c"a.b".as_ptr(), value) };
            assert_eq!(status, 0);
        }
        assert_eq!(std::fs::read(&rp).unwrap(), std::fs::read(&cp).unwrap());
        drop(tx);
        // SAFETY: consumes raw transaction, aborting before config is released.
        unsafe { ffi::git_transaction_free(ctx) };
        assert_eq!(std::fs::read(&rp).unwrap(), std::fs::read(&cp).unwrap());
        assert!(!rp.with_extension("lock").exists());
        assert!(!cp.with_extension("lock").exists());
    }
    drop(rust);
    // SAFETY: releases raw config after its last transaction was freed.
    unsafe { ffi::git_config_free(raw) };
    std::fs::remove_file(rp).unwrap();
    std::fs::remove_file(cp).unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn mapping_mode_values_match_c_behavior() {
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
