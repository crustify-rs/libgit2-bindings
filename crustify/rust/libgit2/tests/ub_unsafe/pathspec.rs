//! Lists own copied names and failures, but borrow diff deltas. Repeated reborrows
//! and C-retained pathspec pointers exercise the foreign boundary under BSan.

use libgit2::libgit2::core::types::FileMode;
use libgit2::libgit2::diff::diff::Diff;
use libgit2::libgit2::index::index::Index;
use libgit2::libgit2::index::pathspec::{MatchList, MatchListBox};
#[path = "../support/pathspec_match_list.rs"]
mod support;
use libgit2::libgit2::object::blob::create_from_buffer;
use libgit2::libgit2::object::tree::{Tree, TreeBuilder};
use libgit2::libgit2::repository::repository::Repository;
use std::ffi::{CStr, CString};
use support::patterns;

const PATCH: &[u8] = b"diff --git a/a.txt b/a.txt\nnew file mode 100644\nindex 0000000..7898192\n--- /dev/null\n+++ b/a.txt\n@@ -0,0 +1 @@\n+a\n";

fn bounds_and_reborrows(list: &mut MatchListBox<'_>) {
    let len = list.as_ref().len();
    let failures = list.as_ref().failed_len();
    let mut exclusive = list.as_mut();
    for _ in 0..8 {
        let mut child = exclusive.as_mut();
        assert_eq!(child.as_mut().as_ref().len(), len);
        let shared = child.as_ref();
        let copy = shared;
        assert_eq!(copy.failed_len(), failures);
        assert_eq!(shared.is_empty(), len == 0);
        for n in [len, usize::MAX] {
            assert!(shared.entry(n).is_none());
            assert!(shared.diff_entry(n).is_none());
        }
        for n in [failures, usize::MAX] {
            assert!(shared.failed_entry(n).is_none());
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn retained_patterns_and_copied_index_paths_survive_parents() {
    let mut index = populated_index(&[c"a.txt", c"dir/b.rs", c"dir/c.rs"]);
    let owned_pattern = CString::new("*.rs").unwrap();
    let ps = patterns(&[&owned_pattern, c"missing"]);
    // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
    let mut list = unsafe {
        MatchList::from_index(
            index.as_mut(),
            ps.as_c_ptr(),
            libgit2::ffi::GIT_PATHSPEC_FIND_FAILURES,
        )
    }
    .unwrap();
    drop(owned_pattern);
    drop(ps);
    drop(index);
    assert_eq!(list.as_ref().len(), 2);
    assert_eq!(list.as_ref().entry(0), Some(c"dir/b.rs"));
    assert_eq!(list.as_ref().entry(1), Some(c"dir/c.rs"));
    assert_eq!(list.as_ref().failed_entry(0), Some(c"missing"));
    assert!(list.as_ref().diff_entry(0).is_none());
    bounds_and_reborrows(&mut list);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_large_failure_lists_and_flags() {
    let mut index = Index::new().unwrap();
    let names: Vec<_> = (0..130)
        .map(|i| CString::new(format!("absent/{i}")).unwrap())
        .collect();
    let refs: Vec<_> = names.iter().map(|s| s.as_c_str()).collect();
    let ps = patterns(&refs);
    // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
    let mut list = unsafe {
        MatchList::from_index(
            index.as_mut(),
            ps.as_c_ptr(),
            libgit2::ffi::GIT_PATHSPEC_FIND_FAILURES | libgit2::ffi::GIT_PATHSPEC_FAILURES_ONLY,
        )
    }
    .unwrap();
    drop(names);
    drop(ps);
    assert_eq!(list.as_ref().failed_len(), 130);
    for i in 0..130 {
        assert_eq!(
            list.as_ref().failed_entry(i).unwrap().to_bytes(),
            format!("absent/{i}").as_bytes()
        );
    }
    bounds_and_reborrows(&mut list);
    let empty = patterns(&[]);
    // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
    let mut list = unsafe {
        MatchList::from_index(
            index.as_mut(),
            empty.as_c_ptr(),
            libgit2::ffi::GIT_PATHSPEC_DEFAULT,
        )
    }
    .unwrap();
    bounds_and_reborrows(&mut list);
    // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
    let error = unsafe {
        MatchList::from_index(
            index.as_mut(),
            empty.as_c_ptr(),
            libgit2::ffi::GIT_PATHSPEC_NO_MATCH_ERROR,
        )
    }
    .unwrap_err();
    assert_eq!(error.raw_code(), libgit2::ffi::GIT_ENOTFOUND);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn borrowed_diff_deltas_and_independent_failures() {
    let bytes = PATCH.to_vec();
    let diff = Diff::from_buffer(&bytes).unwrap();
    drop(bytes);
    let ps = patterns(&[c"*.txt", c"missing"]);
    // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
    let mut list = unsafe {
        MatchList::from_diff(
            diff.as_ref(),
            ps.as_c_ptr(),
            libgit2::ffi::GIT_PATHSPEC_FIND_FAILURES,
        )
    }
    .unwrap();
    drop(ps);
    assert_eq!(list.as_ref().len(), 1);
    assert!(list.as_ref().entry(0).is_none());
    let delta = list.as_ref().diff_entry(0).unwrap();
    assert_eq!(delta.new_file().path(), Some(c"a.txt"));
    assert_eq!(list.as_ref().failed_entry(0), Some(c"missing"));
    bounds_and_reborrows(&mut list);
    drop(list);
    // Freeing a list must not free any delta owned by the diff.
    assert_eq!(
        diff.as_ref().get(0).unwrap().new_file().path(),
        Some(c"a.txt")
    );
    let ps = patterns(&[c"absent"]);
    assert_eq!(
        // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
        unsafe {
            MatchList::from_diff(
                diff.as_ref(),
                ps.as_c_ptr(),
                libgit2::ffi::GIT_PATHSPEC_NO_MATCH_ERROR,
            )
        }
        .unwrap_err()
        .raw_code(),
        libgit2::ffi::GIT_ENOTFOUND
    );
    // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
    let mut list = unsafe {
        MatchList::from_diff(
            diff.as_ref(),
            ps.as_c_ptr(),
            libgit2::ffi::GIT_PATHSPEC_FIND_FAILURES | libgit2::ffi::GIT_PATHSPEC_FAILURES_ONLY,
        )
    }
    .unwrap();
    bounds_and_reborrows(&mut list);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn tree_and_workdir_lists_release_sources() {
    let path = std::env::temp_dir().join(format!("crustify-match-list-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let cpath = CString::new(path.to_str().unwrap()).unwrap();
    let mut repo = Repository::init(&cpath, false).unwrap();
    std::fs::write(path.join("a.txt"), b"text").unwrap();
    std::fs::write(path.join("hidden.txt"), b"ignored").unwrap();
    std::fs::write(path.join(".gitignore"), b"hidden.txt\n").unwrap();
    let ps = patterns(&[c"*.txt", c"missing"]);
    // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
    let mut workdir = unsafe {
        MatchList::from_workdir(
            repo.as_mut(),
            ps.as_c_ptr(),
            libgit2::ffi::GIT_PATHSPEC_FIND_FAILURES,
        )
    }
    .unwrap();
    assert_eq!(workdir.as_ref().len(), 1);
    assert_eq!(workdir.as_ref().entry(0), Some(c"a.txt"));
    let missing = patterns(&[c"absent"]);
    assert_eq!(
        // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
        unsafe {
            MatchList::from_workdir(
                repo.as_mut(),
                missing.as_c_ptr(),
                libgit2::ffi::GIT_PATHSPEC_NO_MATCH_ERROR,
            )
        }
        .unwrap_err()
        .raw_code(),
        libgit2::ffi::GIT_ENOTFOUND
    );
    let blob = create_from_buffer(repo.as_ref(), b"tree text").unwrap();
    let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
    let empty_id = builder.as_mut().write().unwrap();
    let empty_tree = Tree::lookup(repo.as_ref(), empty_id.as_ref()).unwrap();
    // SAFETY: ps is an initialized heap pathspec held through its shared counted owner.
    let empty_list =
        unsafe { MatchList::from_tree(empty_tree.as_ref(), ps.as_c_ptr(), 0) }.unwrap();
    assert!(empty_list.as_ref().is_empty());
    drop(empty_tree);

    builder
        .as_mut()
        .insert(c"a.txt", blob.as_ref(), FileMode::Blob)
        .unwrap();
    let id = builder.as_mut().write().unwrap();
    drop(builder);
    let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
    // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
    let mut tree_list = unsafe {
        MatchList::from_tree(
            tree.as_ref(),
            ps.as_c_ptr(),
            libgit2::ffi::GIT_PATHSPEC_FIND_FAILURES,
        )
    }
    .unwrap();
    assert_eq!(
        // SAFETY: the fixture holds a counted shared pathspec reference for the call; no concurrent mutation.
        unsafe {
            MatchList::from_tree(
                tree.as_ref(),
                missing.as_c_ptr(),
                libgit2::ffi::GIT_PATHSPEC_NO_MATCH_ERROR,
            )
        }
        .unwrap_err()
        .raw_code(),
        libgit2::ffi::GIT_ENOTFOUND
    );
    drop(tree);
    drop(ps);
    drop(repo);
    bounds_and_reborrows(&mut workdir);
    bounds_and_reborrows(&mut tree_list);
    assert_eq!(tree_list.as_ref().failed_len(), 1);
    assert_eq!(tree_list.as_ref().entry(0), Some(c"a.txt"));
    assert_eq!(workdir.as_ref().entry(0), Some(c"a.txt"));
    std::fs::remove_dir_all(&path).unwrap();
    let bare_path = path.with_extension("bare");
    let _ = std::fs::remove_dir_all(&bare_path);
    let bare_cpath = CString::new(bare_path.to_str().unwrap()).unwrap();
    let mut bare = Repository::init(&bare_cpath, true).unwrap();
    let ps = patterns(&[c"*"]);
    // SAFETY: initialized shared pathspec; a bare repository is a supported error
    // input, and the exclusive repository handle rules out concurrent accesses.
    assert!(unsafe { MatchList::from_workdir(bare.as_mut(), ps.as_c_ptr(), 0) }.is_err());
    drop(bare);
    std::fs::remove_dir_all(bare_path).unwrap();
}

fn populated_index(paths: &[&CStr]) -> libgit2::libgit2::index::index::IndexBox {
    let index = libgit2::libgit2::index::index::Index::new().unwrap();
    for path in paths {
        let mut entry = libgit2::ffi::git_index_entry {
            path: path.as_ptr(),
            mode: 0o100644,
            ..Default::default()
        };
        entry.id.type_ = libgit2::ffi::GIT_OID_SHA1 as _;
        entry.id.id[0] = 1;
        assert_eq!(
            // SAFETY: exclusive live index; entry and NUL-terminated path are
            // copied during the call, and the fixture retains no raw pointer.
            unsafe { libgit2::ffi::git_index_add(index.as_c_ptr(), &entry) },
            0
        );
    }
    index
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn generated_diff_borrows_deltas_without_retaining_sources() {
    let path =
        std::env::temp_dir().join(format!("crustify-match-generated-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let repo = Repository::init(&CString::new(path.to_str().unwrap()).unwrap(), true).unwrap();
    let id = create_from_buffer(repo.as_ref(), b"new text").unwrap();
    let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
    builder
        .as_mut()
        .insert(c"a.txt", id.as_ref(), FileMode::Blob)
        .unwrap();
    builder
        .as_mut()
        .insert(c"b.txt", id.as_ref(), FileMode::Blob)
        .unwrap();
    let id = builder.as_mut().write().unwrap();
    let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
    let diff = Diff::tree_to_tree(repo.as_ref(), None, Some(tree.as_ref())).unwrap();
    drop(tree);
    drop(builder);
    let ps = patterns(&[c"a.txt", c"*.txt", c"*", c"missing"]);
    // SAFETY: the fixture provides a live shared counted pathspec reference;
    // no mutation occurs. The safe diff handle carries all delta keepalives.
    let mut list = unsafe {
        MatchList::from_diff(
            diff.as_ref(),
            ps.as_c_ptr(),
            libgit2::ffi::GIT_PATHSPEC_FIND_FAILURES,
        )
    }
    .unwrap();
    drop(ps);
    assert_eq!(list.as_ref().len(), 2);
    assert_eq!(
        list.as_ref().diff_entry(0).unwrap().new_file().path(),
        Some(c"a.txt")
    );
    assert_eq!(
        list.as_ref().diff_entry(1).unwrap().new_file().path(),
        Some(c"b.txt")
    );
    assert_eq!(list.as_ref().failed_entry(0), Some(c"missing"));
    bounds_and_reborrows(&mut list);
    drop(list);
    assert_eq!(diff.as_ref().len(), 2);
    drop(diff);
    drop(repo);
    std::fs::remove_dir_all(path).unwrap();
}
