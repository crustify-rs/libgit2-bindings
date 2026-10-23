//! Public transaction lifecycle and keepalive tests. No unsafe transaction adoption.
//! Illegal parent drops, aliasing, and thread transfer are compile-fail cases.
use libgit2::libgit2::config::config::{Config, ConfigLevel};
use libgit2::libgit2::refs::transaction::Transaction;
use libgit2::libgit2::repository::repository::Repository;

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reference_owners_and_reborrows_release_once() {
    let path = std::env::temp_dir().join(format!("crustify-ub-tx-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&path));
    let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    let repo = Repository::init(&cpath, true).unwrap();
    for _ in 0..8 {
        let mut tx = Transaction::new(repo.as_ref()).unwrap();
        let shared = tx.as_ref();
        let copy = shared;
        assert_eq!(format!("{shared:?}"), format!("{copy:?}"));
        let mut exclusive = tx.as_mut();
        assert_eq!(
            format!("{:?}", exclusive.as_mut().as_ref()),
            "Transaction { .. }"
        );
    }
    // The retained refdb references were released; the repository remains usable.
    assert!(repo.as_ref().is_bare());
    drop(repo);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn config_lock_borrows_and_abort_releases_backend() {
    let path = std::env::temp_dir().join(format!("crustify-ub-tx-config-{}", std::process::id()));
    drop(std::fs::remove_file(&path));
    let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    let mut config = Config::new().unwrap();
    assert!(
        config.as_mut().lock().is_err(),
        "empty config has no writer"
    );
    config
        .as_mut()
        .add_file(&cpath, ConfigLevel::LOCAL, None, false)
        .unwrap();
    config.as_mut().set_i32(c"a.b", 7).unwrap();
    let before = std::fs::read(&path).unwrap();
    let lock_path = path.with_extension("lock");
    std::fs::write(&lock_path, "held by another writer").unwrap();
    assert!(config.as_mut().lock().is_err());
    assert_eq!(
        std::fs::read_to_string(&lock_path).unwrap(),
        "held by another writer"
    );
    std::fs::remove_file(&lock_path).unwrap();
    for _ in 0..3 {
        let tx = config.as_mut().lock().unwrap();
        let _ = tx.config();
        assert_eq!(format!("{tx:?}"), "ConfigTransaction { .. }");
        drop(tx);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("lock").exists());
    }
    drop(config);
    std::fs::remove_file(path).unwrap();
}
