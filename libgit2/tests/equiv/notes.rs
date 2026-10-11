//! Independent C/Rust note iterators, compared through multi-step C traversal.
use core::ptr;
use libgit2::ffi;
use libgit2::libgit2::index::notes::{NoteIterator, NoteIteratorBox};
use libgit2::libgit2::object::commit::Commit;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
use libgit2::libgit2::util::errors::last;

const FIXTURE: &str = "../../../tests/resources/testrepo.git";

/// Advances both iterators in lock step, past exhaustion, comparing every result and
/// out-parameter, then releases the raw one. Returns the number of notes seen.
fn traverse_together(safe: NoteIteratorBox<'_>, raw: *mut ffi::git_note_iterator) -> usize {
    let mut count = 0;
    let mut exhausted = 0;
    for _ in 0..128 {
        let (mut a, mut b, mut c, mut d) = (
            ffi::git_oid::default(),
            ffi::git_oid::default(),
            ffi::git_oid::default(),
            ffi::git_oid::default(),
        );
        // SAFETY: initialized, disjoint output ids; two live sole iterators,
        // advanced without any outstanding handles; their repository remains live.
        let (x, y) = unsafe {
            (
                ffi::git_note_next(&mut a, &mut b, raw),
                ffi::git_note_next(&mut c, &mut d, safe.as_c_ptr()),
            )
        };
        assert_eq!(x, y);
        assert_eq!((a.id, b.id), (c.id, d.id));
        if x == ffi::GIT_ITEROVER {
            exhausted += 1;
            if exhausted == 2 {
                break;
            }
            continue;
        }
        assert_eq!(x, 0);
        count += 1;
    }
    assert_eq!(exhausted, 2);
    drop(safe);
    // SAFETY: the independent C iterator is still owned here, released once.
    unsafe { ffi::git_note_iterator_free(raw) };
    count
}

fn raw_new(
    repo: &RepositoryBox,
    name: Option<&core::ffi::CStr>,
) -> (i32, *mut ffi::git_note_iterator) {
    let mut raw = ptr::null_mut();
    // SAFETY: valid repository, optional NUL string, and null-initialized slot.
    let rc = unsafe {
        ffi::git_note_iterator_new(
            &mut raw,
            repo.as_c_ptr(),
            name.map_or(ptr::null(), |s| s.as_ptr()),
        )
    };
    (rc, raw)
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn constructors_and_traversal_agree() {
    let repo = Repository::open(c"../../../tests/resources/testrepo.git").unwrap();
    let id = Oid::from_string(c"d07b0f9a8c89f1d9e74dc4fce6421dec5ef8a659", OidType::Sha1).unwrap();
    let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    for from_commit in [false, true] {
        let safe = if from_commit {
            NoteIterator::from_commit(commit.as_ref())
        } else {
            NoteIterator::new(repo.as_ref(), Some(c"refs/notes/fanout"))
        }
        .unwrap();
        let (rc, raw) = if from_commit {
            let mut raw = ptr::null_mut();
            // SAFETY: live commit and writable fresh output; the successful iterator
            // owns independent traversal state, released by traverse_together.
            let rc = unsafe { ffi::git_note_commit_iterator_new(&mut raw, commit.as_c_ptr()) };
            (rc, raw)
        } else {
            raw_new(&repo, Some(c"refs/notes/fanout"))
        };
        assert_eq!(rc, 0);
        assert_eq!(traverse_together(safe, raw), 1);
    }
    for name in [
        None,
        Some(c"refs/notes/absent"),
        Some(c""),
        Some(c"refs/notes/bad..name"),
    ] {
        let safe = NoteIterator::new(repo.as_ref(), name).err().unwrap();
        let (rc, raw) = raw_new(&repo, name);
        let c_error = last();
        assert_eq!(safe.raw_code(), rc, "{name:?}");
        assert_eq!(safe.raw_class(), c_error.raw_class(), "{name:?}");
        assert_eq!(safe.message(), c_error.message(), "{name:?}");
        assert!(raw.is_null());
    }
}

/// The default notes reference comes from `core.notesRef`, read through a fresh
/// repository whose objects are the fixture's (an alternate) and whose own refs
/// point at the fixture's notes commit, directly and symbolically.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn default_and_symbolic_names_agree() {
    let path = std::env::temp_dir().join(format!("crustify-equiv-notes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    drop(Repository::init(&cpath, true).unwrap());
    let objects = std::fs::canonicalize(format!("{FIXTURE}/objects")).unwrap();
    std::fs::write(
        path.join("objects/info/alternates"),
        format!("{}\n", objects.display()),
    )
    .unwrap();
    let fanout = std::fs::read_to_string(format!("{FIXTURE}/refs/notes/fanout")).unwrap();
    std::fs::create_dir_all(path.join("refs/notes")).unwrap();
    std::fs::write(path.join("refs/notes/fanout"), &fanout).unwrap();
    std::fs::write(path.join("refs/notes/link"), "ref: refs/notes/fanout\n").unwrap();
    let mut config = std::fs::read_to_string(path.join("config")).unwrap();
    config.push_str("[core]\n\tnotesRef = refs/notes/link\n");
    std::fs::write(path.join("config"), config).unwrap();
    let repo = Repository::open(&cpath).unwrap();
    for name in [None, Some(c"refs/notes/link"), Some(c"refs/notes/fanout")] {
        for _ in 0..2 {
            let safe = NoteIterator::new(repo.as_ref(), name).unwrap();
            let (rc, raw) = raw_new(&repo, name);
            assert_eq!(rc, 0, "{name:?}");
            assert_eq!(traverse_together(safe, raw), 1, "{name:?}");
        }
    }
    drop(repo);
    // A valueless `core.notesRef` names the empty reference, which neither resolves.
    let mut config = std::fs::read_to_string(path.join("config")).unwrap();
    config.push_str("\tnotesRef\n");
    std::fs::write(path.join("config"), config).unwrap();
    let repo = Repository::open(&cpath).unwrap();
    for _ in 0..2 {
        let safe = NoteIterator::new(repo.as_ref(), None).err().unwrap();
        let (rc, raw) = raw_new(&repo, None);
        let c_error = last();
        assert!(raw.is_null());
        assert_eq!(safe.raw_code(), rc);
        assert_eq!(safe.raw_class(), c_error.raw_class());
        assert_eq!(safe.message(), c_error.message());
    }
    drop(repo);
    std::fs::remove_dir_all(path).unwrap();
}
