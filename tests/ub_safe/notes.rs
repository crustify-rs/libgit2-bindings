//! The safe note-iterator lifecycle: C-owned tree references and repository borrows.
#![forbid(unsafe_code)]
use libgit2::libgit2::index::notes::NoteIterator;
use libgit2::libgit2::object::commit::Commit;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::Repository;

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn construction_release_and_reborrows() {
    let repo = Repository::open(c"c/tests/resources/testrepo.git").unwrap();
    let id = Oid::from_string(c"d07b0f9a8c89f1d9e74dc4fce6421dec5ef8a659", OidType::Sha1).unwrap();
    for _ in 0..16 {
        let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
        let mut from_commit = NoteIterator::from_commit(commit.as_ref()).unwrap();
        drop(commit); // C retained its own root tree, not this commit.
        let name = std::ffi::CString::new("refs/notes/fanout").unwrap();
        let mut named = NoteIterator::new(repo.as_ref(), Some(&name)).unwrap();
        drop(name); // Not a stored borrow.
        {
            let mut exclusive = named.as_mut();
            let shared = exclusive.as_ref();
            let _copy = shared;
            let _reborrow = exclusive.as_mut();
        }
        let _ = from_commit.as_ref();
        let _ = from_commit.as_mut();
        drop(named);
        drop(from_commit);
    }
    assert!(NoteIterator::new(repo.as_ref(), Some(c"refs/notes/missing")).is_err());
    assert!(NoteIterator::new(repo.as_ref(), None).is_err());
    assert!(NoteIterator::new(repo.as_ref(), Some(c"")).is_err());
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn independent_iterators_on_threads() {
    let workers: Vec<_> = (0..4)
        .map(|_| std::thread::spawn(construction_release_and_reborrows))
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
}

/// References to a blob, a tree, a missing object or (symbolically) a blob reach C's
/// uninitialised tree free in `git_note_iterator_new`; the safe constructor refuses
/// each before C is called, repeatedly, and the repository stays usable.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn non_commit_targets_are_refused() {
    use libgit2::libgit2::object::blob;
    use libgit2::libgit2::util::errors::ErrorCode;
    let path = std::env::temp_dir().join(format!("crustify-ub-notes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    let repo = Repository::init(&cpath, true).unwrap();
    let blob = blob::create_from_buffer(repo.as_ref(), b"note\n").unwrap();
    let refs = path.join("refs/notes");
    std::fs::create_dir_all(&refs).unwrap();
    let hex = blob.as_ref().to_hex_string().unwrap();
    std::fs::write(refs.join("blob"), format!("{hex}\n")).unwrap();
    std::fs::write(
        refs.join("tree"),
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904\n",
    )
    .unwrap();
    std::fs::write(
        refs.join("missing"),
        "1111111111111111111111111111111111111111\n",
    )
    .unwrap();
    std::fs::write(refs.join("symbolic"), "ref: refs/notes/blob\n").unwrap();
    std::fs::write(refs.join("commits"), "ref: refs/notes/tree\n").unwrap();
    for _ in 0..8 {
        for name in [
            Some(c"refs/notes/blob"),
            Some(c"refs/notes/tree"),
            Some(c"refs/notes/missing"),
            Some(c"refs/notes/symbolic"),
            None,
        ] {
            let error = NoteIterator::new(repo.as_ref(), name).err().unwrap();
            assert_eq!(error.code(), Some(ErrorCode::NotFound), "{name:?}");
        }
    }
    drop(repo);
    std::fs::remove_dir_all(path).unwrap();
}

/// `sizeof(tree_iterator)` on x86_64, the iterator allocation of both constructors.
const TREE_ITERATOR_SIZE: usize = 336;

/// The iterator allocation fails after C took its root-tree lookup: both constructors
/// report the failure, C releases the tree (LSan), and the same call succeeds once
/// memory is back. Process-wide allocator, so alone in a child.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn iterator_allocation_failure_releases_the_tree() {
    use libgit2::libgit2::util::errors::ErrorClass;

    use crate::fixture::annotated_commit::{refuse, refused};
    use crate::fixture::isolated;
    isolated(
        concat!(
            module_path!(),
            "::iterator_allocation_failure_releases_the_tree"
        ),
        || {
            let repo = Repository::open(c"c/tests/resources/testrepo.git").unwrap();
            let id = Oid::from_string(c"d07b0f9a8c89f1d9e74dc4fce6421dec5ef8a659", OidType::Sha1)
                .unwrap();
            let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
            drop(commit.as_ref().tree().unwrap()); // Cache the tree: no lookup allocation.
            refuse(TREE_ITERATOR_SIZE, 1);
            let error = NoteIterator::from_commit(commit.as_ref()).err().unwrap();
            assert!(refused());
            assert_eq!(error.class(), Some(ErrorClass::NoMemory));
            drop(NoteIterator::from_commit(commit.as_ref()).unwrap());
            let name = Some(c"refs/notes/fanout");
            drop(NoteIterator::new(repo.as_ref(), name).unwrap());
            refuse(TREE_ITERATOR_SIZE, 1);
            let error = NoteIterator::new(repo.as_ref(), name).err().unwrap();
            assert!(refused());
            assert_eq!(error.class(), Some(ErrorClass::NoMemory));
            let mut it = NoteIterator::new(repo.as_ref(), name).unwrap();
            let _ = it.as_mut();
        },
    );
}
