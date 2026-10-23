#![forbid(unsafe_code)]
#[path = "../support/diff.rs"]
mod support;
use std::ffi::CString;

use libgit2::ffi;
use libgit2::libgit2::diff::diff::{Diff, DiffArc};
use libgit2::libgit2::index::index::Index;
use libgit2::libgit2::object::tree::Tree;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::{InitFlag, InitOptions, Repository};
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

/// Trees and indices of the other object-id type than the repository's reach every
/// generator, both ways round: libgit2 copies each id with its type and compares by
/// type first, so the deltas stay in bounds and carry the input's own type, while
/// `id_abbrev` follows the repository's.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
#[expect(
    clippy::too_many_lines,
    reason = "one scenario, its steps run and compared in order"
)]
fn foreign_object_id_type_inputs_stay_in_bounds() {
    let res = format!("{}/c/tests/resources", env!("CARGO_MANIFEST_DIR"));
    let bare256 =
        Repository::open(&CString::new(format!("{res}/testrepo_256.git")).unwrap()).unwrap();
    let flat = Oid::from_string(
        c"f2a108f86a3b4fd9ad75ed55e9cb3cb46e348fca3b9dba3db64f7c9f64b8a736",
        OidType::Sha256,
    )
    .unwrap();
    let tree256 = Tree::lookup(bare256.as_ref(), flat.as_ref()).unwrap();
    let index256 = Index::open_with_oid_type(
        &CString::new(format!("{res}/git-sha256.index")).unwrap(),
        OidType::Sha256,
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("crustify-diff-sha256-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    let mut opts = InitOptions::new();
    opts.as_mut().set_oid_type(Some(OidType::Sha256));
    opts.as_mut().set_flags(InitFlag::Mkpath.into());
    let repo256 =
        Repository::init_ext(&CString::new(dir.to_str().unwrap()).unwrap(), opts.as_mut()).unwrap();
    std::fs::write(dir.join("file"), b"wd\n").unwrap();
    let fixture = Repo::new();
    std::fs::write(fixture.path.join("file"), b"wd\n").unwrap();
    let repo = fixture.get().as_ref();
    let tree1 = fixture.tree(b"old\n");
    let index1 = Index::new().unwrap();
    let r256 = repo256.as_ref();
    let (sha1, sha256) = (
        u8::try_from(OidType::Sha1.raw()).unwrap(),
        u8::try_from(OidType::Sha256.raw()).unwrap(),
    );
    let (added, deleted, modified) = (
        ffi::GIT_DELTA_ADDED,
        ffi::GIT_DELTA_DELETED,
        ffi::GIT_DELTA_MODIFIED,
    );
    // (diff, delta count, distinct (status, old type, new type, old/new abbrev)).
    let cases = [
        (
            Diff::tree_to_tree(repo, Some(tree256.as_ref()), Some(tree1.as_ref())),
            4,
            vec![(added, sha1, sha1, 0, 40), (deleted, sha256, sha1, 40, 0)],
        ),
        (
            Diff::tree_to_index(repo, Some(tree1.as_ref()), Some(index256.as_ref())),
            4345,
            vec![(added, sha1, sha256, 0, 40), (deleted, sha1, sha1, 40, 0)],
        ),
        (
            Diff::index_to_workdir(repo, Some(index256.as_ref())),
            4344,
            vec![(deleted, sha256, sha1, 40, 0)],
        ),
        (
            Diff::tree_to_workdir(repo, Some(tree256.as_ref())),
            3,
            vec![(deleted, sha256, sha1, 40, 0)],
        ),
        (
            Diff::tree_to_workdir_with_index(repo, Some(tree256.as_ref())),
            3,
            vec![(deleted, sha256, sha1, 40, 0)],
        ),
        (
            Diff::index_to_index(repo, index1.as_ref(), index256.as_ref()),
            4344,
            vec![(added, sha1, sha256, 0, 40)],
        ),
        (
            Diff::tree_to_tree(r256, Some(tree1.as_ref()), Some(tree256.as_ref())),
            4,
            vec![
                (added, sha256, sha256, 0, 64),
                (deleted, sha1, sha256, 64, 0),
            ],
        ),
        (
            Diff::tree_to_index(r256, Some(tree1.as_ref()), Some(index1.as_ref())),
            1,
            vec![(deleted, sha1, sha256, 64, 0)],
        ),
        (
            Diff::tree_to_workdir(r256, Some(tree1.as_ref())),
            1,
            vec![(modified, sha1, sha256, 64, 64)],
        ),
        (
            Diff::index_to_index(r256, index1.as_ref(), index256.as_ref()),
            4344,
            vec![(added, sha256, sha256, 0, 64)],
        ),
    ];
    for (diff, len, expected) in cases {
        let diff = diff.unwrap();
        let diff = diff.as_ref();
        assert_eq!(diff.len(), len);
        let found: std::collections::BTreeSet<_> = (0..len)
            .map(|i| {
                let (status, _, _, _, old, new) = support::delta(diff.get(i).unwrap());
                (status, old.2, new.2, old.6, new.6)
            })
            .collect();
        assert_eq!(found, expected.into_iter().collect());
    }
    drop(tree256);
    drop(bare256);
    drop(repo256);
    drop(std::fs::remove_dir_all(&dir));
}
