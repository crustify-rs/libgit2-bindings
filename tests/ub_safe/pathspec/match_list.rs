//! Lists own copied names and failures, but borrow diff deltas. Repeated reborrows
//! and C-retained pathspec references exercise the foreign boundary under `BSan`.
//! Every constructor is safe; no unsafe operation is permitted in these tests.
#![forbid(unsafe_code)]

use libgit2::libgit2::core::types::FileMode;
use libgit2::libgit2::diff::diff::Diff;
use libgit2::libgit2::index::index::Index;
use libgit2::libgit2::index::pathspec::{MatchList, MatchListBox, PathspecFlags};
#[path = "../../support/pathspec_match_list.rs"]
mod support;
use std::ffi::CString;

use libgit2::libgit2::object::blob::create_from_buffer;
use libgit2::libgit2::object::tree::{Tree, TreeBuilder};
use libgit2::libgit2::repository::repository::Repository;
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
    let path = CString::new("dir/b.rs").unwrap();
    let mut index = crate::fixture::match_list_index(&[c"a.txt", &path, c"dir/c.rs"]);
    drop(path);
    let owned_pattern = CString::new("*.rs").unwrap();
    let ps = patterns(&[&owned_pattern, c"missing"]);
    let mut list =
        MatchList::from_index(index.as_mut(), &ps, PathspecFlags::FIND_FAILURES).unwrap();
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
    let refs: Vec<_> = names.iter().map(CString::as_c_str).collect();
    let ps = patterns(&refs);
    let mut list = MatchList::from_index(
        index.as_mut(),
        &ps,
        PathspecFlags::FIND_FAILURES | PathspecFlags::FAILURES_ONLY,
    )
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
    let mut list = MatchList::from_index(index.as_mut(), &empty, PathspecFlags::DEFAULT).unwrap();
    bounds_and_reborrows(&mut list);
    let error =
        MatchList::from_index(index.as_mut(), &empty, PathspecFlags::NO_MATCH_ERROR).unwrap_err();
    assert_eq!(error.raw_code(), libgit2::ffi::GIT_ENOTFOUND);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn borrowed_diff_deltas_and_independent_failures() {
    let bytes = PATCH.to_vec();
    let diff = Diff::from_buffer(&bytes).unwrap();
    drop(bytes);
    let ps = patterns(&[c"*.txt", c"missing"]);
    let mut list = MatchList::from_diff(diff.as_ref(), &ps, PathspecFlags::FIND_FAILURES).unwrap();
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
        MatchList::from_diff(diff.as_ref(), &ps, PathspecFlags::NO_MATCH_ERROR,)
            .unwrap_err()
            .raw_code(),
        libgit2::ffi::GIT_ENOTFOUND
    );
    let mut list = MatchList::from_diff(
        diff.as_ref(),
        &ps,
        PathspecFlags::FIND_FAILURES | PathspecFlags::FAILURES_ONLY,
    )
    .unwrap();
    bounds_and_reborrows(&mut list);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn tree_and_workdir_lists_release_sources() {
    let path = std::env::temp_dir().join(format!("crustify-match-list-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&path));
    let cpath = CString::new(path.to_str().unwrap()).unwrap();
    let mut repo = Repository::init(&cpath, false).unwrap();
    std::fs::write(path.join("a.txt"), b"text").unwrap();
    std::fs::write(path.join("hidden.txt"), b"ignored").unwrap();
    std::fs::write(path.join(".gitignore"), b"hidden.txt\n").unwrap();
    let ps = patterns(&[c"*.txt", c"missing"]);
    let mut workdir =
        MatchList::from_workdir(repo.as_mut(), &ps, PathspecFlags::FIND_FAILURES).unwrap();
    assert_eq!(workdir.as_ref().len(), 1);
    assert_eq!(workdir.as_ref().entry(0), Some(c"a.txt"));
    let missing = patterns(&[c"absent"]);
    assert_eq!(
        MatchList::from_workdir(repo.as_mut(), &missing, PathspecFlags::NO_MATCH_ERROR,)
            .unwrap_err()
            .raw_code(),
        libgit2::ffi::GIT_ENOTFOUND
    );
    let blob = create_from_buffer(repo.as_ref(), b"tree text").unwrap();
    let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
    let empty_id = builder.as_mut().write().unwrap();
    let empty_tree = Tree::lookup(repo.as_ref(), empty_id.as_ref()).unwrap();
    let empty_list =
        MatchList::from_tree(empty_tree.as_ref(), &ps, PathspecFlags::DEFAULT).unwrap();
    assert!(empty_list.as_ref().is_empty());
    drop(empty_tree);

    builder
        .as_mut()
        .insert(c"a.txt", blob.as_ref(), FileMode::Blob)
        .unwrap();
    let id = builder.as_mut().write().unwrap();
    drop(builder);
    let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
    let mut tree_list =
        MatchList::from_tree(tree.as_ref(), &ps, PathspecFlags::FIND_FAILURES).unwrap();
    assert_eq!(
        MatchList::from_tree(tree.as_ref(), &missing, PathspecFlags::NO_MATCH_ERROR,)
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
    drop(std::fs::remove_dir_all(&bare_path));
    let bare_cpath = CString::new(bare_path.to_str().unwrap()).unwrap();
    let mut bare = Repository::init(&bare_cpath, true).unwrap();
    let ps = patterns(&[c"*"]);
    assert!(MatchList::from_workdir(bare.as_mut(), &ps, PathspecFlags::DEFAULT).is_err());
    drop(bare);
    std::fs::remove_dir_all(bare_path).unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn generated_diff_borrows_deltas_without_retaining_sources() {
    let path =
        std::env::temp_dir().join(format!("crustify-match-generated-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&path));
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
    let mut list = MatchList::from_diff(diff.as_ref(), &ps, PathspecFlags::FIND_FAILURES).unwrap();
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
