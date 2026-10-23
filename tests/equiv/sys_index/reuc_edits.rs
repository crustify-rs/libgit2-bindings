//! Independent resolve-undo (REUC) records compared after every add, replacement,
//! lookup, removal and clear: `IndexMut::{add_reuc, find_reuc, remove_reuc,
//! clear_reuc}` and `IndexRef::reuc_entry_count` against their `git_index_reuc_*`
//! routines, on two file-backed indexes whose written files are compared too.

use std::ffi::c_int;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use libgit2::libgit2::core::types::FileMode;
use libgit2::libgit2::index::index::{Capabilities, CapabilitySetting};
use libgit2::libgit2::index::sys::index::ReucSide;
use libgit2::libgit2::oid::oid::{Oid, OidType};

use super::*;

/// Two index files in a scratch directory, removed on drop.
struct Files(PathBuf);

impl Files {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("crustify-reuc-equiv-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> CString {
        CString::new(self.0.join(name).as_os_str().as_bytes()).unwrap()
    }

    fn bytes(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.0.join(name)).unwrap()
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn id(byte: u8) -> Oid {
    Oid::from_raw(&[byte; 20], OidType::Sha1).unwrap()
}

/// One side's mode as the C routine takes it: 0 for an absent side.
fn raw_mode(side: Option<ReucSide<'_>>) -> c_int {
    side.map_or(0, |side| side.mode as c_int)
}

/// Both indexes' counts, then their files once written.
fn compare(files: &Files, c: &IndexBox, rust: &IndexBox) {
    // SAFETY: a live index, only read.
    let count = unsafe { ffi::git_index_reuc_entrycount(c.as_c_ptr()) };
    assert_eq!(count, rust.as_ref().reuc_entry_count());
    // SAFETY: a live file-backed index.
    assert_eq!(unsafe { ffi::git_index_write(c.as_c_ptr()) }, 0);
    rust.as_ref().write().unwrap();
    assert_eq!(files.bytes("c"), files.bytes("r"));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reuc_sequences_match() {
    let files = Files::new("sequence");
    let c = Index::open(&files.path("c")).unwrap();
    let mut rust = Index::open(&files.path("r")).unwrap();
    let ids = [id(1), id(2), id(3)];
    let side = |mode, n: usize| Some(ReucSide::new(mode, ids[n].as_ref()));
    let blob = |n: usize| side(FileMode::Blob, n);
    let sides: Vec<(&CStr, [Option<ReucSide<'_>>; 3])> = vec![
        (c"b", [blob(0), blob(1), blob(2)]),
        (c"a", [None, blob(1), side(FileMode::BlobExecutable, 2)]),
        (c"c/d", [blob(0), None, None]),
        (c"", [None, None, side(FileMode::Link, 0)]),
        (c"\xff", [side(FileMode::Commit, 1), None, blob(0)]),
        // The same path again: replaced.
        (c"b", [None, blob(2), blob(1)]),
        // A side with mode 0 keeps no id.
        (c"z", [side(FileMode::Unreadable, 0), blob(0), None]),
        (c"B", [None, None, None]),
        // Modes libgit2 does not name are stored as given.
        (
            c"odd",
            [
                Some(ReucSide {
                    mode: u32::MAX,
                    id: ids[0].as_ref(),
                }),
                Some(ReucSide {
                    mode: 0o644,
                    id: ids[1].as_ref(),
                }),
                None,
            ],
        ),
    ];
    for (path, [ancestor, ours, theirs]) in sides {
        let modes = [ancestor, ours, theirs].map(raw_mode);
        let ids = [ancestor, ours, theirs].map(|s| s.map(|s| s.id.to_oid()));
        let ptrs = ids.each_ref().map(|i| {
            i.as_ref().map_or(ptr::null(), |i| {
                std::ptr::from_ref(i).cast::<ffi::git_oid>()
            })
        });
        // SAFETY: a live index; the path and each id (or NULL with mode 0) are read and
        // copied for the call.
        let rc = unsafe {
            ffi::git_index_reuc_add(
                c.as_c_ptr(),
                path.as_ptr(),
                modes[0],
                ptrs[0],
                modes[1],
                ptrs[1],
                modes[2],
                ptrs[2],
            )
        };
        assert_eq!(rc, 0);
        rust.as_mut()
            .add_reuc(path, ancestor, ours, theirs)
            .unwrap();
        compare(&files, &c, &rust);
    }
    let find = |c: &IndexBox, rust: &mut IndexBox, path: &CStr| {
        let mut at = usize::MAX;
        // SAFETY: a live index and writable slot; the path is read for the call.
        let rc = unsafe { ffi::git_index_reuc_find(&mut at, c.as_c_ptr(), path.as_ptr()) };
        assert_eq!(
            (rc == 0).then_some(at),
            rust.as_mut().find_reuc(path),
            "{path:?}"
        );
    };
    for path in [c"a", c"b", c"B", c"", c"\xff", c"c", c"c/d", c"missing"] {
        find(&c, &mut rust, path);
    }
    // Case-insensitive lookups once the index ignores case.
    // SAFETY: a live index.
    assert_eq!(unsafe { ffi::git_index_set_caps(c.as_c_ptr(), 1) }, 0);
    rust.as_mut()
        .set_caps(CapabilitySetting::Explicit(Capabilities::IGNORE_CASE))
        .unwrap();
    for path in [c"A", c"b", c"Z", c"C/D", c"missing"] {
        find(&c, &mut rust, path);
    }
    for position in [1, 0, 100, usize::MAX, 3] {
        // SAFETY: a live index; C bounds-checks the position.
        let rc = unsafe { ffi::git_index_reuc_remove(c.as_c_ptr(), position) };
        let result = rust.as_mut().remove_reuc(position);
        assert_eq!(rc == 0, result.is_ok(), "{position}");
        if rc != 0 {
            assert_eq!(rc, ffi::GIT_ENOTFOUND);
        }
        compare(&files, &c, &rust);
    }
    // SAFETY: a live index.
    assert_eq!(unsafe { ffi::git_index_reuc_clear(c.as_c_ptr()) }, 0);
    rust.as_mut().clear_reuc().unwrap();
    compare(&files, &c, &rust);
    // Reading the files back yields equal records.
    // SAFETY: a live file-backed index.
    assert_eq!(unsafe { ffi::git_index_read(c.as_c_ptr(), 1) }, 0);
    rust.as_mut().read(true).unwrap();
    compare(&files, &c, &rust);
}

/// Records written by one side read back alike, positions included.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reuc_records_read_from_a_file_match() {
    let files = Files::new("read");
    let mut writer = Index::open(&files.path("w")).unwrap();
    for (n, path) in [c"q", c"p", c"r/s"].into_iter().enumerate() {
        let id = id(n as u8 + 1);
        writer
            .as_mut()
            .add_reuc(
                path,
                Some(ReucSide::new(FileMode::Blob, id.as_ref())),
                None,
                Some(ReucSide::new(FileMode::Blob, id.as_ref())),
            )
            .unwrap();
    }
    writer.as_ref().write().unwrap();
    std::fs::copy(files.0.join("w"), files.0.join("c")).unwrap();
    std::fs::copy(files.0.join("w"), files.0.join("r")).unwrap();
    let c = Index::open(&files.path("c")).unwrap();
    let mut rust = Index::open(&files.path("r")).unwrap();
    for path in [c"p", c"q", c"r/s", c"r"] {
        let mut at = usize::MAX;
        // SAFETY: a live index and writable slot; the path is read for the call.
        let rc = unsafe { ffi::git_index_reuc_find(&mut at, c.as_c_ptr(), path.as_ptr()) };
        assert_eq!((rc == 0).then_some(at), rust.as_mut().find_reuc(path));
    }
    compare(&files, &c, &rust);
}
