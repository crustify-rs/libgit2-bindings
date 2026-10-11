//! Independently owned C/Rust transaction inputs; compare lifecycle effects after
//! every lock/write/drop step. Raw manipulation is only for not-yet-wrapped symbols.
use core::ptr;
use libgit2::ffi;
use libgit2::libgit2::config::config::Config;
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
    let rust_error = rust.as_mut().lock().unwrap_err();
    // SAFETY: initialized empty raw config and writable transaction output.
    let status = unsafe { ffi::git_config_lock(&mut ctx, raw) };
    assert_eq!(status, ffi::GIT_EREADONLY);
    assert_eq!(rust_error.raw_code(), status);
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
    assert_eq!(rust_error.raw_code(), status);
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

/// One lock attempt's outcome: the code (0 on success), and for a failure the error's
/// class and message with the side's scratch prefix replaced; on success, which of
/// the backends' files are locked while the transaction lives.
type LockRecord = (i32, Option<(i32, String)>, Vec<bool>);

fn locked(files: &[std::path::PathBuf]) -> Vec<bool> {
    files
        .iter()
        .map(|f| f.with_extension("lock").exists())
        .collect()
}

fn add_file(cfg: *mut ffi::git_config, path: &CStr, level: ffi::git_config_level_t) {
    // SAFETY: a live config of the caller's alone, a NUL-terminated path; no
    // repository is retained by the backend.
    let rc = unsafe { ffi::git_config_add_file_ondisk(cfg, path.as_ptr(), level, ptr::null(), 0) };
    assert_eq!(rc, 0);
}

/// Regression for `crustify/reviews/.../config/wave-0/batch-0/equiv/config-lock-discards-error-code`:
/// the safe lock reports libgit2's code. Each side locks its own config through an
/// empty config, a held lock file, a free one, then two file backends where the
/// higher-priority file is the writer, its lock held, then free; every step compares
/// code, error and which lock files exist.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn config_lock_codes_match_c() {
    let mut rust = Config::new().unwrap();
    let mut raw = ptr::null_mut();
    // SAFETY: runtime pinned by Config::new; raw receives an owned config.
    assert_eq!(unsafe { ffi::git_config_new(&mut raw) }, 0);
    let rust_raw = rust.as_c_ptr();
    let side = |tag: &str| {
        let files: Vec<_> = ["local", "app"]
            .iter()
            .map(|level| path(&format!("codes-{tag}-{level}")))
            .collect();
        for (p, _) in &files {
            let _ = std::fs::remove_file(p);
            let _ = std::fs::remove_file(p.with_extension("lock"));
        }
        files
    };
    let (rust_files, c_files) = (side("rust"), side("c"));
    let prefix = |files: &[(std::path::PathBuf, CString)]| {
        files[0]
            .1
            .to_str()
            .unwrap()
            .trim_end_matches("-local")
            .to_owned()
    };
    let (rust_prefix, c_prefix) = (prefix(&rust_files), prefix(&c_files));
    let rust_paths: Vec<_> = rust_files.iter().map(|(p, _)| p.clone()).collect();
    let c_paths: Vec<_> = c_files.iter().map(|(p, _)| p.clone()).collect();

    let mut steps = 0;
    let mut compare = |rust: &mut libgit2::libgit2::config::config::ConfigBox<'static>| {
        let rust_record: LockRecord = match rust.as_mut().lock() {
            Ok(tx) => {
                let held = locked(&rust_paths);
                drop(tx);
                (0, None, held)
            }
            Err(e) => {
                let message = e.message().to_string_lossy().replace(&rust_prefix, "<p>");
                (e.raw_code(), Some((e.raw_class(), message)), vec![])
            }
        };
        let mut tx = ptr::null_mut();
        // SAFETY: a live config and a writable slot.
        let rc = unsafe { ffi::git_config_lock(&mut tx, raw) };
        let c_record: LockRecord = if rc == 0 {
            let held = locked(&c_paths);
            // SAFETY: the transaction just handed out, released once (aborting).
            unsafe { ffi::git_transaction_free(tx) };
            (0, None, held)
        } else {
            let e = libgit2::libgit2::util::errors::last();
            let message = e.message().to_string_lossy().replace(&c_prefix, "<p>");
            (rc, Some((e.raw_class(), message)), vec![])
        };
        assert_eq!(rust_record, c_record, "step {steps}");
        steps += 1;
        if rust_record.0 == 0 {
            // Every abort releases the lock it took, on both sides.
            assert_eq!(locked(&rust_paths), [false, false]);
            assert_eq!(locked(&c_paths), [false, false]);
        }
        rust_record
    };

    assert_eq!(compare(&mut rust).0, ffi::GIT_EREADONLY);
    add_file(rust_raw, &rust_files[0].1, ffi::GIT_CONFIG_LEVEL_LOCAL);
    add_file(raw, &c_files[0].1, ffi::GIT_CONFIG_LEVEL_LOCAL);
    let hold = |files: &[std::path::PathBuf], i: usize, on: bool| {
        let lock = files[i].with_extension("lock");
        if on {
            std::fs::write(lock, "held").unwrap();
        } else {
            std::fs::remove_file(lock).unwrap();
        }
    };
    hold(&rust_paths, 0, true);
    hold(&c_paths, 0, true);
    assert_eq!(compare(&mut rust).0, ffi::GIT_ELOCKED);
    hold(&rust_paths, 0, false);
    hold(&c_paths, 0, false);
    assert_eq!(compare(&mut rust), (0, None, vec![true, false]));
    add_file(rust_raw, &rust_files[1].1, ffi::GIT_CONFIG_LEVEL_APP);
    add_file(raw, &c_files[1].1, ffi::GIT_CONFIG_LEVEL_APP);
    // The higher-priority application file is now the writer: only it is locked.
    assert_eq!(compare(&mut rust), (0, None, vec![false, true]));
    hold(&rust_paths, 1, true);
    hold(&c_paths, 1, true);
    assert_eq!(compare(&mut rust).0, ffi::GIT_ELOCKED);
    hold(&rust_paths, 1, false);
    hold(&c_paths, 1, false);
    assert_eq!(compare(&mut rust), (0, None, vec![false, true]));
    drop(rust);
    // SAFETY: the raw config, released once after its last transaction.
    unsafe { ffi::git_config_free(raw) };
    for p in rust_paths.iter().chain(&c_paths) {
        let _ = std::fs::remove_file(p);
    }
}
