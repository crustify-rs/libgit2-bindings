//! UB tests for the resolve-undo (REUC) routines of `index/sys/index.rs`: records copy
//! their path and ids, so temporaries may go first; replacing, removing (in and out of
//! range), clearing, rereading and dropping the index free each record once.

use std::ffi::CString;

use libgit2::libgit2::core::types::FileMode;
use libgit2::libgit2::index::index::{Capabilities, CapabilitySetting, Index};
use libgit2::libgit2::index::sys::index::ReucSide;
use libgit2::libgit2::oid::oid::{Oid, OidType};

use crate::fixture::init;

fn id(byte: u8, kind: OidType) -> Oid {
    Oid::from_raw(&vec![byte; kind.size()], kind).unwrap()
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn records_copy_inputs_and_are_freed_once() {
    init();
    let mut index = Index::new().unwrap();
    for n in 0..20u8 {
        // Path and ids are temporaries, gone before the next record.
        let path = CString::new(format!("path-{}", n % 7)).unwrap();
        let ours = id(n, OidType::Sha1);
        let theirs = id(n, OidType::Sha256);
        index
            .as_mut()
            .add_reuc(
                &path,
                (n % 3 == 0).then_some(ReucSide::new(FileMode::Unreadable, ours.as_ref())),
                Some(ReucSide::new(FileMode::Blob, ours.as_ref())),
                Some(ReucSide::new(FileMode::Link, theirs.as_ref())),
            )
            .unwrap();
        drop((path, ours, theirs));
    }
    // Seven paths, each replaced (and its old record freed) as it came back.
    assert_eq!(index.as_ref().reuc_entry_count(), 7);
    let mut handle = index.as_mut();
    let at = handle.find_reuc(c"path-3").unwrap();
    handle.remove_reuc(at).unwrap();
    assert!(handle.remove_reuc(at + 100).is_err());
    assert!(handle.remove_reuc(usize::MAX).is_err());
    assert_eq!(handle.find_reuc(c"path-3"), None);
    handle
        .set_caps(CapabilitySetting::Explicit(Capabilities::IGNORE_CASE))
        .unwrap();
    assert!(handle.find_reuc(c"PATH-4").is_some());
    handle.add_reuc(c"", None, None, None).unwrap();
    while handle.as_ref().reuc_entry_count() > 3 {
        handle.remove_reuc(0).unwrap();
    }
    // Dropped with records left: the index frees them.
    drop(index);

    let mut index = Index::new().unwrap();
    let blob = id(1, OidType::Sha1);
    index
        .as_mut()
        .add_reuc(
            c"a",
            Some(ReucSide::new(FileMode::Blob, blob.as_ref())),
            None,
            None,
        )
        .unwrap();
    index.as_mut().clear_reuc().unwrap();
    index.as_mut().clear_reuc().unwrap();
    assert_eq!(index.as_ref().reuc_entry_count(), 0);
}

/// Records written to a file and read back, by a soft and a forced read, replace the
/// in-memory ones, which are freed.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn records_survive_a_write_and_reread() {
    init();
    let dir = std::env::temp_dir().join(format!("crustify-reuc-ub-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = CString::new(dir.join("index").to_str().unwrap()).unwrap();
    let mut index = Index::open(&file).unwrap();
    let ids = [id(1, OidType::Sha1), id(2, OidType::Sha1)];
    for path in [c"x", c"y/z", c"\xff"] {
        index
            .as_mut()
            .add_reuc(
                path,
                Some(ReucSide::new(FileMode::Blob, ids[0].as_ref())),
                Some(ReucSide::new(FileMode::BlobExecutable, ids[1].as_ref())),
                None,
            )
            .unwrap();
    }
    index.as_ref().write().unwrap();
    index.as_mut().clear_reuc().unwrap();
    index.as_mut().read(false).unwrap();
    index.as_mut().read(true).unwrap();
    assert_eq!(index.as_ref().reuc_entry_count(), 3);
    assert_eq!(index.as_mut().find_reuc(c"y/z"), Some(1));
    drop(index);
    let _ = std::fs::remove_dir_all(dir);
}
