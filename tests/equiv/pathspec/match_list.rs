//! Compare constructor status, copied strings, failed patterns, delta content,
//! bounds, repeated observations and release ordering on independent inputs.

use core::ptr;

use libgit2::ffi;
use libgit2::libgit2::core::types::FileMode;
use libgit2::libgit2::diff::diff::{Diff, DiffBox};
use libgit2::libgit2::index::index::{Index, IndexBox};
use libgit2::libgit2::index::pathspec::{MatchList, MatchListRef, PathspecFlags};
#[path = "../../support/pathspec_match_list.rs"]
mod support;
use std::ffi::{CStr, CString};

use libgit2::libgit2::object::blob::create_from_buffer;
use libgit2::libgit2::object::tree::{Tree, TreeBuilder};
use libgit2::libgit2::repository::repository::{Repository, RepositoryError};
use support::patterns;

fn index(paths: &[&CStr]) -> IndexBox {
    let index = Index::new().unwrap();
    for path in paths {
        let mut entry = ffi::git_index_entry {
            path: path.as_ptr(),
            mode: 0o100644,
            ..Default::default()
        };
        entry.id.type_ = ffi::GIT_OID_SHA1 as _;
        entry.id.id[0] = 1;
        // SAFETY: sole index and initialized input; C copies the entry and path.
        assert_eq!(unsafe { ffi::git_index_add(index.as_c_ptr(), &entry) }, 0);
    }
    index
}

struct RawList(*mut ffi::git_pathspec_match_list);
impl Drop for RawList {
    fn drop(&mut self) {
        // SAFETY: one fresh C result, or NULL on failure; freed once.
        unsafe { ffi::git_pathspec_match_list_free(self.0) }
    }
}

type Delta = (u32, Option<CString>, Option<CString>);
#[derive(Debug, PartialEq)]
struct Seen {
    count: usize,
    entries: Vec<Option<CString>>,
    failures: Vec<Option<CString>>,
    deltas: Vec<Option<Delta>>,
}

fn safe_seen(list: MatchListRef<'_, '_>) -> Seen {
    let probes: Vec<_> = (0..=list.len()).chain([usize::MAX]).collect();
    Seen {
        count: list.len(),
        entries: probes
            .iter()
            .map(|&i| list.entry(i).map(CStr::to_owned))
            .collect(),
        failures: (0..=list.failed_len())
            .chain([usize::MAX])
            .map(|i| list.failed_entry(i).map(CStr::to_owned))
            .collect(),
        deltas: probes
            .iter()
            .map(|&i| {
                list.diff_entry(i).map(|d| {
                    (
                        d.status(),
                        d.old_file().path().map(CStr::to_owned),
                        d.new_file().path().map(CStr::to_owned),
                    )
                })
            })
            .collect(),
    }
}

fn raw_seen(list: &RawList) -> Seen {
    // SAFETY: every pointer here is returned from this live list, which its source
    // diff (if any) outlives. C bounds-checks; all present strings are terminated.
    unsafe {
        let count = ffi::git_pathspec_match_list_entrycount(list.0);
        let probes: Vec<_> = (0..=count).chain([usize::MAX]).collect();
        let string = |p: *const core::ffi::c_char| {
            if p.is_null() {
                None
            } else {
                Some(CStr::from_ptr(p).to_owned())
            }
        };
        Seen {
            count,
            entries: probes
                .iter()
                .map(|&i| string(ffi::git_pathspec_match_list_entry(list.0, i)))
                .collect(),
            failures: (0..=ffi::git_pathspec_match_list_failed_entrycount(list.0))
                .chain([usize::MAX])
                .map(|i| string(ffi::git_pathspec_match_list_failed_entry(list.0, i)))
                .collect(),
            deltas: probes
                .iter()
                .map(|&i| {
                    let p = ffi::git_pathspec_match_list_diff_entry(list.0, i);
                    if p.is_null() {
                        None
                    } else {
                        Some((
                            (*p).status,
                            string((*p).old_file.path),
                            string((*p).new_file.path),
                        ))
                    }
                })
                .collect(),
        }
    }
}

fn compare_error(rc: i32, error: RepositoryError) {
    assert_eq!(rc, error.raw_code());
    // SAFETY: the preceding C failure set this thread's error; it is copied before
    // any other C operation. Successful safe allocations are not running here.
    unsafe {
        let e = ffi::git_error_last();
        assert!(!e.is_null());
        assert_eq!((*e).klass, error.raw_class());
        assert_eq!(CStr::from_ptr((*e).message), error.message());
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn index_flag_sequences_and_release_order() {
    let names = [c"a.txt", c"dir/b.rs", c"DIR/C.RS", c"!literal"];
    for pats in [
        vec![],
        vec![c"*.rs", c"missing"],
        vec![c"*", c"dir/*", c"*.rs"],
        vec![c"*", c"dir/*", c"*.rs", c"missing"],
        vec![c"!literal"],
        vec![c"!dir", c"*"],
        vec![c"absent"],
    ] {
        let mut safe_index = index(&names);
        let raw_index = index(&names);
        let safe_ps = patterns(&pats);
        let raw_ps = patterns(&pats);
        for bits in [0, 1, 2, 3, 4, 8, 16, 24, 32, 40, 48, 56, u32::MAX] {
            let safe = MatchList::from_index(
                safe_index.as_mut(),
                &safe_ps,
                PathspecFlags::from_bits_retain(bits),
            );
            let mut raw = RawList(ptr::null_mut());
            // SAFETY: independent initialized index/pathspec and a writable out
            // slot. The source owners remain live; RawList releases each result.
            let rc = unsafe {
                ffi::git_pathspec_match_index(
                    &mut raw.0,
                    raw_index.as_c_ptr(),
                    bits,
                    raw_ps.as_c_ptr(),
                )
            };
            match safe {
                Ok(mut list) => {
                    assert_eq!(rc, 0);
                    let expected = raw_seen(&raw);
                    assert_eq!(safe_seen(list.as_ref()), expected);
                    assert_eq!(safe_seen(list.as_mut().as_mut().as_ref()), raw_seen(&raw));
                    drop(list);
                    // Re-observe C's independent result after releasing Rust's.
                    assert_eq!(raw_seen(&raw), expected);
                }
                Err(error) => {
                    assert!(raw.0.is_null());
                    compare_error(rc, error);
                }
            }
        }
        let list =
            MatchList::from_index(safe_index.as_mut(), &safe_ps, PathspecFlags::FIND_FAILURES)
                .unwrap();
        let mut raw = RawList(ptr::null_mut());
        assert_eq!(
            // SAFETY: same independent initialized sources and writable out slot.
            unsafe {
                ffi::git_pathspec_match_index(
                    &mut raw.0,
                    raw_index.as_c_ptr(),
                    libgit2::ffi::GIT_PATHSPEC_FIND_FAILURES,
                    raw_ps.as_c_ptr(),
                )
            },
            0
        );
        drop(safe_ps);
        drop(raw_ps);
        drop(safe_index);
        drop(raw_index);
        assert_eq!(safe_seen(list.as_ref()), raw_seen(&raw));
    }
}

const RENAME: &[u8] = b"diff --git a/old.txt b/new.rs\nsimilarity index 100%\nrename from old.txt\nrename to new.rs\n";

const PATCH: &[u8] = b"diff --git a/a.txt b/a.txt\nnew file mode 100644\nindex 0000000..7898192\n--- /dev/null\n+++ b/a.txt\n@@ -0,0 +1 @@\n+a\n";

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn diff_variants_retain_only_borrowed_deltas() {
    for bytes in [b"".as_slice(), PATCH, RENAME] {
        let diff = Diff::from_buffer(bytes).unwrap();
        let raw_diff: DiffBox<'static> = Diff::from_buffer(bytes).unwrap();
        for pats in [
            vec![],
            vec![c"*.txt", c"missing"],
            vec![c"*.rs", c"missing"],
            vec![c"old.txt", c"new.rs"],
            vec![c"*", c"*.txt"],
            vec![c"!a.txt", c"*"],
            vec![c"absent"],
        ] {
            let ps = patterns(&pats);
            let raw_ps = patterns(&pats);
            for bits in [0, 4, 8, 16, 24, 32, 40, 48, 56] {
                let list =
                    MatchList::from_diff(diff.as_ref(), &ps, PathspecFlags::from_bits_retain(bits));
                let mut raw = RawList(ptr::null_mut());
                // SAFETY: independent live diff and pathspec. The diff remains live
                // and unchanged until after RawList releases its borrowed pointers.
                let rc = unsafe {
                    ffi::git_pathspec_match_diff(
                        &mut raw.0,
                        raw_diff.as_c_ptr(),
                        bits,
                        raw_ps.as_c_ptr(),
                    )
                };
                match list {
                    Ok(list) => {
                        assert_eq!(rc, 0);
                        for _ in 0..3 {
                            assert_eq!(safe_seen(list.as_ref()), raw_seen(&raw));
                        }
                    }
                    Err(error) => {
                        assert!(raw.0.is_null());
                        compare_error(rc, error);
                    }
                }
            }
        }
        assert_eq!(diff.as_ref().len(), raw_diff.as_ref().len());
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn tree_and_workdir_sources_and_bare_error() {
    for bare in [false, true] {
        let base =
            std::env::temp_dir().join(format!("crustify-list-equiv-{}-{bare}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let a = base.join("safe");
        let b = base.join("raw");
        let mut repo = Repository::init(&CString::new(a.to_str().unwrap()).unwrap(), bare).unwrap();
        let raw_repo = Repository::init(&CString::new(b.to_str().unwrap()).unwrap(), bare).unwrap();
        if !bare {
            for path in [&a, &b] {
                std::fs::write(path.join("a.txt"), b"text").unwrap();
            }
        }
        let ps = patterns(&[c"*.txt", c"missing"]);
        let raw_ps = patterns(&[c"*.txt", c"missing"]);
        let list = MatchList::from_workdir(repo.as_mut(), &ps, PathspecFlags::FIND_FAILURES);
        let mut raw = RawList(ptr::null_mut());
        // SAFETY: independent live repository/pathspec and writable output slot;
        // a bare repo is a supported error case, not an invalid pointer.
        let rc = unsafe {
            ffi::git_pathspec_match_workdir(
                &mut raw.0,
                raw_repo.as_c_ptr(),
                libgit2::ffi::GIT_PATHSPEC_FIND_FAILURES,
                raw_ps.as_c_ptr(),
            )
        };
        match list {
            Ok(list) => {
                assert_eq!(rc, 0);
                assert_eq!(safe_seen(list.as_ref()), raw_seen(&raw));
            }
            Err(error) => compare_error(rc, error),
        }
        let blob = create_from_buffer(repo.as_ref(), b"tree text").unwrap();
        let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
        builder
            .as_mut()
            .insert(c"a.txt", blob.as_ref(), FileMode::Blob)
            .unwrap();
        let id = builder.as_mut().write().unwrap();
        let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
        let raw_blob = create_from_buffer(raw_repo.as_ref(), b"tree text").unwrap();
        let mut raw_builder = TreeBuilder::new(raw_repo.as_ref()).unwrap();
        raw_builder
            .as_mut()
            .insert(c"a.txt", raw_blob.as_ref(), FileMode::Blob)
            .unwrap();
        let raw_id = raw_builder.as_mut().write().unwrap();
        let raw_tree = Tree::lookup(raw_repo.as_ref(), raw_id.as_ref()).unwrap();
        let list = MatchList::from_tree(tree.as_ref(), &ps, PathspecFlags::FIND_FAILURES).unwrap();
        let mut raw_tree_list = RawList(ptr::null_mut());
        assert_eq!(
            // SAFETY: independently owned tree and pathspec; C copies every result.
            unsafe {
                ffi::git_pathspec_match_tree(
                    &mut raw_tree_list.0,
                    raw_tree.as_c_ptr(),
                    libgit2::ffi::GIT_PATHSPEC_FIND_FAILURES,
                    raw_ps.as_c_ptr(),
                )
            },
            0
        );
        drop(tree);
        drop(raw_tree);
        drop(builder);
        drop(raw_builder);
        drop(repo);
        drop(raw_repo);
        drop(ps);
        drop(raw_ps);
        assert_eq!(safe_seen(list.as_ref()), raw_seen(&raw_tree_list));
        std::fs::remove_dir_all(base).unwrap();
    }
}
