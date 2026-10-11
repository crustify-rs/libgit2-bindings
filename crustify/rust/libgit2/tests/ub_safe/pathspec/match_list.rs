//! Safe public getters/owner borrows; raw construction is confined to one fixture
//! until the compiled-pathspec wrapper lands. Constructor contracts have separate
//! `ub_unsafe::pathspec` tests. No unsafe operation is permitted in these tests.
#![forbid(unsafe_code)]
use libgit2::ffi;
use libgit2::libgit2::index::pathspec::MatchListBox;
use std::ffi::CString;

fn reborrow(list: &mut MatchListBox<'_>) {
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
    let pattern = CString::new("*.rs").unwrap();
    let mut list = crate::fixture::match_list_fixture(
        &[c"a.txt", &path, c"dir/c.rs"],
        &[&pattern, c"missing"],
        ffi::GIT_PATHSPEC_FIND_FAILURES,
    );
    drop(path);
    drop(pattern);
    assert_eq!(list.as_ref().len(), 2);
    assert_eq!(list.as_ref().entry(0), Some(c"dir/b.rs"));
    assert_eq!(list.as_ref().entry(1), Some(c"dir/c.rs"));
    assert_eq!(list.as_ref().failed_entry(0), Some(c"missing"));
    assert!(list.as_ref().diff_entry(0).is_none());
    reborrow(&mut list);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_large_failure_lists_and_flags() {
    let names: Vec<_> = (0..130)
        .map(|i| CString::new(format!("absent/{i}")).unwrap())
        .collect();
    let refs: Vec<_> = names.iter().map(|s| s.as_c_str()).collect();
    let mut list = crate::fixture::match_list_fixture(
        &[],
        &refs,
        ffi::GIT_PATHSPEC_FIND_FAILURES | ffi::GIT_PATHSPEC_FAILURES_ONLY,
    );
    drop(names);
    assert_eq!(list.as_ref().failed_len(), 130);
    for i in 0..130 {
        assert_eq!(
            list.as_ref().failed_entry(i).unwrap().to_bytes(),
            format!("absent/{i}").as_bytes()
        );
    }
    reborrow(&mut list);
    let mut empty = crate::fixture::match_list_fixture(&[], &[], 0);
    reborrow(&mut empty);
}
