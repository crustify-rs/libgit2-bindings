#![forbid(unsafe_code)]
#[path = "../support/diff.rs"]
mod support;
use libgit2::libgit2::diff::diff::{Diff, DiffArc};
use libgit2::libgit2::index::index::Index;
use libgit2::libgit2::oid::oid::OidType;
use support::{PATCH, RENAME, Repo};

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parsed_copy_survives_input_and_owner_reborrows() {
    for _ in 0..16 {
        let input = [PATCH, RENAME].concat();
        let mut diff = Diff::from_buffer(&input).unwrap();
        drop(input);
        assert_eq!(diff.as_ref().len(), 2);
        let expected = support::delta(diff.as_ref().get(0).unwrap());
        {
            let mut exclusive = diff.as_mut();
            let child = exclusive.as_mut();
            assert_eq!(support::delta(child.as_ref().get(0).unwrap()), expected);
        }
        let file = diff.as_ref().get(0).unwrap().old_file();
        assert_eq!(file.path(), Some(c"file"));
        assert_eq!(file.id_abbrev(), 7);
        assert_eq!(diff.as_ref().get(1).unwrap().similarity(), 100);
        assert!(diff.as_ref().get(2).is_none());
        assert!(diff.as_ref().get(usize::MAX).is_none());
        let shared: DiffArc<'static> = diff.into();
        assert_eq!(support::delta(shared.as_ref().get(0).unwrap()), expected);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parse_empty_errors_and_explicit_kind() {
    assert!(Diff::from_buffer(b"").unwrap().as_ref().is_empty());
    for input in [
        b"garbage".as_slice(),
        b"diff --git a/a b/a\n@@ broken\n",
        b"\0",
        &PATCH[..PATCH.len() - 5],
    ] {
        assert!(Diff::from_buffer(input).is_err());
    }
    for kind in [OidType::Sha1, OidType::Sha256] {
        let d = Diff::from_buffer_with_oid_type(PATCH, kind).unwrap();
        assert_eq!(
            d.as_ref().get(0).unwrap().new_file().id().kind(),
            Some(kind)
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn generated_inputs_can_drop_before_diff() {
    let fixture = Repo::new();
    let repo = fixture.get().as_ref();
    let old = fixture.tree(b"old\n");
    let new = fixture.tree(b"new\n");
    let diff = Diff::tree_to_tree(repo, Some(old.as_ref()), Some(new.as_ref())).unwrap();
    drop(old);
    drop(new);
    assert_eq!(diff.as_ref().len(), 1);
    assert_eq!(
        diff.as_ref().get(0).unwrap().old_file().path(),
        Some(c"file")
    );
    assert!(
        Diff::tree_to_tree(repo, None, None)
            .unwrap()
            .as_ref()
            .is_empty()
    );
    let tree = fixture.tree(b"old\n");
    let index = Index::new().unwrap();
    let second = Index::new().unwrap();
    assert!(
        Diff::index_to_workdir(repo, Some(index.as_ref()))
            .unwrap()
            .as_ref()
            .is_empty()
    );
    let del = Diff::tree_to_index(repo, Some(tree.as_ref()), Some(index.as_ref())).unwrap();
    assert_eq!(del.as_ref().len(), 1);
    let empty = Diff::index_to_index(repo, index.as_ref(), second.as_ref()).unwrap();
    drop(index);
    drop(second);
    drop(tree);
    assert!(empty.as_ref().is_empty());
    assert_eq!(
        del.as_ref().get(0).unwrap().new_file().path(),
        Some(c"file")
    );
    assert!(
        Diff::tree_to_index(repo, None, None)
            .unwrap()
            .as_ref()
            .is_empty()
    );
    assert!(
        Diff::index_to_workdir(repo, None)
            .unwrap()
            .as_ref()
            .is_empty()
    );
    assert!(
        Diff::tree_to_workdir(repo, None)
            .unwrap()
            .as_ref()
            .is_empty()
    );
    assert!(
        Diff::tree_to_workdir_with_index(repo, None)
            .unwrap()
            .as_ref()
            .is_empty()
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn independent_thread_local_diffs() {
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..4 {
                    let d = Diff::from_buffer(PATCH).unwrap();
                    assert_eq!(d.as_ref().get(0).unwrap().nfiles(), 2);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
