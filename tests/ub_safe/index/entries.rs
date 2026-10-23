//! UB tests for the `index/index.rs` routines that lend, add and replace entries and
//! conflicts: entries and conflicts lent across reborrows and in-place rewrites;
//! additions, conflict records and tree reads freeing what they replace exactly once,
//! including on every failure; owned indexes refusing ids libgit2 would overflow on;
//! snapshot copies outliving the index's changes and the index itself; and conflicts
//! outliving their cursor.

use std::ffi::{CStr, CString};

use ffibox::CZeroable;
use libgit2::libgit2::core::types::FileMode;
use libgit2::libgit2::index::index::{
    ConflictIterator, EntryExtendedFlags, EntryFlags, Index, IndexArc, IndexBox, IndexEntry,
    IndexEntryBuf, IndexIterator, Stage,
};
use libgit2::libgit2::object::blob::create_from_buffer;
use libgit2::libgit2::object::tree::{Tree, TreeArc, TreeBuilder};
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryRef};

use super::{Scratch, c_path};
use crate::fixture::index::{add_staged, isolated_tolerating_leaks, repository_index_box};
use crate::fixture::init;
use crate::fixture::odb::write_tree;

/// An entry of mode `mode` at `path` and `stage`, of id type `kind` with first id byte
/// `byte`.
fn entry(path: &CStr, stage: u16, kind: u8, byte: u8, mode: FileMode) -> IndexEntry<'_> {
    let mut entry = IndexEntry::new(path, Oid::zeroed().as_ref(), mode);
    let mut handle = entry.as_mut();
    handle
        .set_flags(EntryFlags::from_bits_retain(u32::from(stage) << 12))
        .unwrap();
    let mut id = [0; 32];
    id[0] = byte;
    handle.id_mut().set_raw_kind(kind);
    handle.id_mut().set_id(id);
    entry
}

fn blob(path: &CStr, byte: u8) -> IndexEntry<'_> {
    entry(path, 0, OidType::Sha1.raw() as u8, byte, FileMode::Blob)
}

/// `(path, stage)` of every entry, through the lending routines.
fn listing(index: &mut IndexBox) -> Vec<(CString, Stage)> {
    let handle = index.as_mut();
    (0..handle.as_ref().entry_count())
        .map(|n| {
            let entry = handle.entry(n).unwrap();
            (entry.path().to_owned(), entry.stage())
        })
        .collect()
}

/// A repository with a working directory in `scratch`.
fn work_repo(scratch: &Scratch) -> RepositoryBox {
    Repository::init(&c_path(&scratch.0), false).unwrap()
}

/// A tree of `repo` holding `files` (`dir/name` paths one level deep at most), whose
/// blobs hold their own paths.
fn tree<'r>(repo: RepositoryRef<'r>, files: &[&str]) -> TreeArc<'r> {
    let mut root = TreeBuilder::new(repo).unwrap();
    let mut dirs: Vec<(&str, Vec<&str>)> = Vec::new();
    for file in files {
        match file.split_once('/') {
            Some((dir, name)) => match dirs.iter_mut().find(|(d, _)| *d == dir) {
                Some((_, names)) => names.push(name),
                None => dirs.push((dir, vec![name])),
            },
            None => {
                let id = create_from_buffer(repo, file.as_bytes()).unwrap();
                let name = CString::new(*file).unwrap();
                root.as_mut()
                    .insert(&name, id.as_ref(), FileMode::Blob)
                    .unwrap();
            }
        }
    }
    for (dir, names) in dirs {
        let mut sub = TreeBuilder::new(repo).unwrap();
        for name in names {
            let id = create_from_buffer(repo, format!("{dir}/{name}").as_bytes()).unwrap();
            let name = CString::new(name).unwrap();
            sub.as_mut()
                .insert(&name, id.as_ref(), FileMode::Blob)
                .unwrap();
        }
        let id = sub.as_mut().write().unwrap();
        let dir = CString::new(dir).unwrap();
        root.as_mut()
            .insert(&dir, id.as_ref(), FileMode::Tree)
            .unwrap();
    }
    let id = root.as_mut().write().unwrap();
    Tree::lookup(repo, id.as_ref()).unwrap()
}

/// Entries, lookups and conflicts lent by an exclusive handle stay readable across
/// shared reborrows and the write that rewrites every entry's flags in place; their
/// paths and ids are read after it. BSan targets this test.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn lent_entries_survive_reborrows_and_in_place_rewrites() {
    init();
    let scratch = Scratch::new("entries-lent");
    let file = c_path(&scratch.file("index"));
    let mut index = Index::open(&file).unwrap();
    let mut handle = index.as_mut();
    handle.add(blob(c"a", 1).as_ref()).unwrap();
    for (stage, byte) in [(1, 2), (2, 3)] {
        let side = entry(c"c", stage, 1, byte, FileMode::Link);
        handle.add(side.as_ref()).unwrap();
    }
    let first = handle.entry(0).unwrap();
    let by_path = handle.entry_by_path(c"a", Stage::Any).unwrap();
    let conflict = handle.conflict(c"c").unwrap();
    let path = first.path();
    let id = first.id();
    assert!(
        first
            .flags_extended()
            .contains(EntryExtendedFlags::UP_TO_DATE)
    );
    let shared = handle.as_ref();
    shared.write().unwrap();
    let again = handle.as_ref();
    assert_eq!(again.entry_count(), 3);
    // libgit2 cleared the flag in place, behind the borrows.
    assert!(
        !first
            .flags_extended()
            .contains(EntryExtendedFlags::UP_TO_DATE)
    );
    assert!(
        !by_path
            .flags_extended()
            .contains(EntryExtendedFlags::UP_TO_DATE)
    );
    assert_eq!(path, c"a");
    assert_eq!(id.id()[0], 1);
    assert_eq!(by_path.path(), path);
    assert!(conflict.theirs.is_none());
    assert_eq!(conflict.ancestor.unwrap().id().id()[0], 2);
    assert_eq!(conflict.ours.unwrap().filemode(), Some(FileMode::Link));
    assert_eq!(conflict.ours.unwrap().stage(), Stage::Ours);
    let copy = conflict.ours.unwrap().to_entry_buf();
    drop(index);
    assert_eq!(copy.as_ref().path(), c"c");
}

/// Lookups past the end, at absent paths and stages, and on an empty index lend
/// nothing.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn lookups_of_absent_entries_lend_nothing() {
    init();
    let mut index = Index::new().unwrap();
    let handle = index.as_mut();
    for position in [0, 1, usize::MAX] {
        assert!(handle.entry(position).is_none());
    }
    assert!(handle.conflict(c"").is_none());
    let mut index = Index::new().unwrap();
    let mut handle = index.as_mut();
    handle.add(blob(c"a", 1).as_ref()).unwrap();
    handle
        .add(entry(c"b", 3, 1, 2, FileMode::Blob).as_ref())
        .unwrap();
    assert!(handle.entry(2).is_none());
    assert!(handle.entry(usize::MAX).is_none());
    for stage in [
        Stage::Any,
        Stage::Normal,
        Stage::Ancestor,
        Stage::Ours,
        Stage::Theirs,
    ] {
        assert!(handle.entry_by_path(c"z", stage).is_none());
        assert!(handle.entry_by_path(c"", stage).is_none());
    }
    assert!(handle.entry_by_path(c"a", Stage::Ours).is_none());
    assert!(handle.entry_by_path(c"b", Stage::Normal).is_none());
    assert_eq!(
        handle.entry_by_path(c"b", Stage::Any).unwrap().stage(),
        Stage::Theirs
    );
    assert!(handle.conflict(c"a").is_none());
    assert!(handle.conflict(c"z").is_none());
    assert!(handle.conflict(c"b").unwrap().theirs.unwrap().is_conflict());
}

/// Every addition route frees the copies it replaces, the entries it collides with
/// and the copies it refuses exactly once: replacement in place, file/directory
/// collisions both ways, conflict sides replacing normal entries, paths of every
/// length, refused modes and paths.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn additions_free_what_they_replace_once() {
    init();
    let mut index = Index::new().unwrap();
    for round in 0..3u8 {
        let mut handle = index.as_mut();
        handle.add(blob(c"a", round).as_ref()).unwrap();
        handle.add(blob(c"a", round + 1).as_ref()).unwrap();
        assert_eq!(handle.entry(0).unwrap().id().id()[0], round + 1);
        // A file at a parent directory goes, then the files under a new file.
        handle.add(blob(c"a/b", 1).as_ref()).unwrap();
        handle.add(blob(c"a/c", 1).as_ref()).unwrap();
        assert!(handle.entry_by_path(c"a", Stage::Normal).is_none());
        handle.add(blob(c"a", 9).as_ref()).unwrap();
        assert_eq!(listing(&mut index), [(c"a".to_owned(), Stage::Normal)]);
        let mut handle = index.as_mut();
        // Conflict sides replace the normal entries at their paths.
        let sides = [1, 2, 3].map(|stage| entry(c"a", 0, 1, stage, FileMode::Blob));
        let [ancestor, ours, theirs] = sides.each_ref().map(|side| Some(side.as_ref()));
        handle.add_conflict(ancestor, ours, theirs).unwrap();
        handle.add_conflict(None, ours, None).unwrap();
        assert_eq!(handle.as_ref().entry_count(), 3);
        handle.add(blob(c"a", 4).as_ref()).unwrap();
        assert_eq!(handle.as_ref().entry_count(), 4);
        handle.remove_conflict(c"a").unwrap();
        // Paths past the name-length mask, and past a tree name's limit.
        for len in [0xffe, 0xfff, 0x1000, 70_000] {
            let path = CString::new(vec![b'p'; len]).unwrap();
            handle.add(blob(&path, 1).as_ref()).unwrap();
            let lent = handle.entry_by_path(&path, Stage::Normal).unwrap();
            assert_eq!(lent.path().to_bytes().len(), len);
            assert_eq!(lent.flags().bits() & 0xfff, len.min(0xfff) as u32);
        }
        // Refusals free the copy they made.
        for mode in [FileMode::Tree, FileMode::Unreadable] {
            assert_eq!(
                handle
                    .add(entry(c"m", 0, 1, 1, mode).as_ref())
                    .unwrap_err()
                    .raw_code(),
                -1
            );
        }
        for path in [c".git/config", c"../a", c"a/../b", c".git"] {
            assert_eq!(
                handle.add(blob(path, 1).as_ref()).unwrap_err().raw_code(),
                -1
            );
        }
        let tree = entry(c"x", 0, 1, 1, FileMode::Tree);
        let refused = handle.add_conflict(Some(blob(c"x", 1).as_ref()), None, Some(tree.as_ref()));
        assert_eq!(refused.unwrap_err().raw_code(), -1);
        handle.add(blob(c"", 1).as_ref()).unwrap();
        assert!(handle.entry_by_path(c"", Stage::Normal).is_some());
        handle.clear().unwrap();
    }
}

/// On an index a repository owns, a conflict whose second side names an object the
/// repository lacks keeps the first side added and frees the rest; ids of another type
/// are refused before libgit2 looks them up, and buffers become blobs whose entries
/// replace and resolve conflicts, freeing what they move.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owned_index_additions_fail_and_resolve_without_leaks() {
    init();
    let scratch = Scratch::new("entries-owned");
    let repo = work_repo(&scratch);
    let present = create_from_buffer(repo.as_ref(), b"present").unwrap();
    let mut index = repository_index_box(&repo);
    let mut handle = index.as_mut();
    let mut ancestor = blob(c"f", 0);
    ancestor.as_mut().set_id(present.as_ref());
    let missing = blob(c"f", 0xee);
    let error = handle.add_conflict(
        Some(ancestor.as_ref()),
        Some(missing.as_ref()),
        Some(missing.as_ref()),
    );
    assert_eq!(error.unwrap_err().raw_code(), -1);
    let conflict = handle.conflict(c"f").unwrap();
    assert!(conflict.ancestor.is_some() && conflict.ours.is_none() && conflict.theirs.is_none());
    for kind in [OidType::Sha256.raw() as u8, 0, 3, u8::MAX] {
        let foreign = entry(c"g", 0, kind, 1, FileMode::Blob);
        assert_eq!(handle.add(foreign.as_ref()).unwrap_err().raw_code(), -1);
        let refused = handle.add_conflict(None, Some(foreign.as_ref()), None);
        assert_eq!(refused.unwrap_err().raw_code(), -1);
    }
    // Conflict stages are refused: libgit2 would free the new entry while adding it.
    for stage in 1..=3 {
        let side = entry(c"f", stage, 1, 1, FileMode::Blob);
        let refused = handle.add_from_buffer(side.as_ref(), b"side").unwrap_err();
        assert_eq!(refused.raw_code(), -1);
    }
    assert!(handle.conflict(c"f").unwrap().ancestor.is_some());
    // The buffer's blob replaces any id, foreign ones included.
    let foreign = entry(c"f", 0, OidType::Sha256.raw() as u8, 1, FileMode::Blob);
    handle
        .add_from_buffer(foreign.as_ref(), b"resolved")
        .unwrap();
    assert!(handle.conflict(c"f").is_none());
    assert_eq!(handle.as_ref().reuc_entry_count(), 1);
    for data in [&b""[..], b"x", &[0; 70_000][..]] {
        handle
            .add_from_buffer(blob(c"g", 1).as_ref(), data)
            .unwrap();
        let lent = handle.entry_by_path(c"g", Stage::Normal).unwrap();
        assert_eq!(lent.file_size() as usize, data.len());
        assert_eq!(lent.id().raw_kind(), OidType::Sha1.raw() as u8);
    }
    let link = entry(c"l", 0, 1, 0, FileMode::Link);
    handle.add_from_buffer(link.as_ref(), b"target").unwrap();
    for mode in [FileMode::Commit, FileMode::Tree] {
        let refused = entry(c"m", 0, 1, 0, mode);
        assert_eq!(
            handle
                .add_from_buffer(refused.as_ref(), b"")
                .unwrap_err()
                .raw_code(),
            -1
        );
    }
    assert_eq!(
        handle
            .add_from_buffer(blob(c".git/x", 0).as_ref(), b"")
            .unwrap_err()
            .raw_code(),
        -1
    );
    // A blob that cannot be written: its loose object's directory is a file.
    let other = Scratch::new("entries-owned-other");
    let id = create_from_buffer(work_repo(&other).as_ref(), b"unwritable").unwrap();
    let hex = id.as_ref().to_hex_string().unwrap();
    std::fs::write(scratch.0.join(".git/objects").join(&hex[..2]), b"").unwrap();
    let error = handle.add_from_buffer(blob(c"u", 0).as_ref(), b"unwritable");
    assert!(error.unwrap_err().raw_code() < 0);
    assert!(handle.entry_by_path(c"u", Stage::Normal).is_none());
    drop(index);
    drop(repo);
}

/// `data` longer than libgit2's `u32` file size is refused before it is read; a lazily
/// zeroed allocation costs no memory.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn oversized_buffers_are_refused_unread() {
    init();
    let scratch = Scratch::new("entries-oversized");
    let repo = work_repo(&scratch);
    let mut index = repository_index_box(&repo);
    let data = vec![0u8; u32::MAX as usize + 1];
    let error = index
        .as_mut()
        .add_from_buffer(blob(c"big", 0).as_ref(), &data)
        .unwrap_err();
    assert_eq!(
        (error.raw_code(), error.message()),
        (-1, c"buffer is too large")
    );
    assert_eq!(index.as_ref().entry_count(), 0);
    // Without an owner, nothing is read either.
    let mut unowned = Index::new().unwrap();
    let error = unowned
        .as_mut()
        .add_from_buffer(blob(c"big", 0).as_ref(), &data)
        .unwrap_err();
    assert_eq!(error.raw_code(), -1);
}

/// Reading trees frees every previous entry, conflict and record once; reading a tree
/// whose subtree is gone, or one of another repository and object id type, leaves
/// nothing behind; a populated index keeps the metadata of matching entries.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn tree_reads_free_previous_contents_once() {
    init();
    let scratch = Scratch::new("entries-read-tree");
    let repo = work_repo(&scratch);
    let small = tree(repo.as_ref(), &["a", "d/x", "d/y"]);
    let large = tree(repo.as_ref(), &["a", "b", "d/x", "e/z", "e/w"]);
    let mut index = Index::new().unwrap();
    let mut handle = index.as_mut();
    handle
        .add(entry(c"c", 2, 1, 1, FileMode::Blob).as_ref())
        .unwrap();
    for round in 0..3 {
        handle.read_tree(small.as_ref()).unwrap();
        assert_eq!(handle.as_ref().entry_count(), 3);
        assert!(!handle.as_ref().has_conflicts());
        let copy = handle.entry(0).unwrap().to_entry_buf();
        let mut stat = copy.as_ref().to_index_entry();
        stat.as_mut().set_file_size(round + 7);
        handle.add(stat.as_ref()).unwrap();
        handle.read_tree(large.as_ref()).unwrap();
        assert_eq!(handle.entry(0).unwrap().file_size(), round + 7);
        assert_eq!(handle.as_ref().entry_count(), 5);
    }
    drop(small);
    drop(large);
    assert_eq!(listing(&mut index).len(), 5);

    // A subtree whose object is gone fails the walk; the index keeps its entries.
    let broken = tree(repo.as_ref(), &["q/r"]);
    let sub = broken.as_ref().get_by_name(c"q").unwrap().id().to_oid();
    let hex = sub.as_ref().to_hex_string().unwrap();
    std::fs::remove_file(
        scratch
            .0
            .join(".git/objects")
            .join(&hex[..2])
            .join(&hex[2..]),
    )
    .unwrap();
    let error = index.as_mut().read_tree(broken.as_ref()).unwrap_err();
    assert!(error.raw_code() < 0);
    assert_eq!(listing(&mut index).len(), 5);

    // A SHA-256 repository's tree, into a SHA-1 index and a SHA-256 one.
    let sha256_dir = Scratch::new("entries-read-tree-256");
    let mut opts = libgit2::libgit2::repository::repository::InitOptions::new();
    opts.as_mut().set_oid_type(Some(OidType::Sha256));
    let sha256 = Repository::init_ext(&c_path(&sha256_dir.0), opts.as_mut()).unwrap();
    let foreign = tree(sha256.as_ref(), &["a", "k/l"]);
    index.as_mut().read_tree(foreign.as_ref()).unwrap();
    assert_eq!(
        index.as_mut().entry(0).unwrap().id().raw_kind(),
        OidType::Sha256.raw() as u8
    );
    let mut typed = Index::with_oid_type(OidType::Sha256).unwrap();
    typed.as_mut().read_tree(foreign.as_ref()).unwrap();
    drop(foreign);
    drop(sha256);
    assert_eq!(listing(&mut typed).len(), 2);
    drop(broken);
    drop(repo);
    assert_eq!(listing(&mut index).len(), 2);
}

/// A tree entry whose name an index cannot hold (`.git`) fails the read, leaving the
/// index as it was. libgit2 leaks the entry's joined path (`read_tree_cb`), so the test
/// runs in a child with LeakSanitizer off.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn refused_tree_entry_names_leave_the_index_as_it_was() {
    isolated_tolerating_leaks(
        concat!(
            module_path!(),
            "::refused_tree_entry_names_leave_the_index_as_it_was"
        ),
        || {
            init();
            let scratch = Scratch::new("entries-refused-names");
            let repo = work_repo(&scratch);
            let kept = tree(repo.as_ref(), &["a", "d/x"]);
            let mut index = Index::new().unwrap();
            index.as_mut().read_tree(kept.as_ref()).unwrap();
            // A tree entry named `.git`, which an index cannot hold, after one it can.
            let id = create_from_buffer(repo.as_ref(), b"dot").unwrap();
            let mut raw = Vec::new();
            for name in [&b".git"[..], b"ok"] {
                raw.extend_from_slice(b"100644 ");
                raw.extend_from_slice(name);
                raw.push(0);
                raw.extend_from_slice(&id.as_ref().id()[..20]);
            }
            let odb = repo.as_ref().odb().unwrap();
            let dot =
                Oid::from_raw(&write_tree(&odb, &raw).unwrap().id[..20], OidType::Sha1).unwrap();
            drop(odb);
            let dot = Tree::lookup(repo.as_ref(), dot.as_ref()).unwrap();
            let error = index.as_mut().read_tree(dot.as_ref()).unwrap_err();
            assert_eq!(
                (error.raw_code(), error.message()),
                (-1, c"invalid path: '.git'")
            );
            assert_eq!(listing(&mut index).len(), 2);
            drop(dot);
            drop(kept);
            drop(repo);
        },
    );
}

/// A walk that fails after reading entries (a subtree whose object is gone, after a
/// blob) leaves the index as it was. libgit2 leaks the entries it read
/// (`git_index_read_tree` disposes of their vector alone), so the test runs in a child
/// with LeakSanitizer off.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn failed_tree_reads_after_entries_leave_the_index_as_it_was() {
    isolated_tolerating_leaks(
        concat!(
            module_path!(),
            "::failed_tree_reads_after_entries_leave_the_index_as_it_was"
        ),
        || {
            init();
            let scratch = Scratch::new("entries-failed-read");
            let repo = work_repo(&scratch);
            let kept = tree(repo.as_ref(), &["k", "d/x"]);
            let broken = tree(repo.as_ref(), &["a", "b", "q/r"]);
            let sub = broken.as_ref().get_by_name(c"q").unwrap().id().to_oid();
            let hex = sub.as_ref().to_hex_string().unwrap();
            let object = scratch
                .0
                .join(".git/objects")
                .join(&hex[..2])
                .join(&hex[2..]);
            std::fs::remove_file(object).unwrap();
            let mut index = Index::new().unwrap();
            index.as_mut().read_tree(kept.as_ref()).unwrap();
            for _ in 0..2 {
                let error = index.as_mut().read_tree(broken.as_ref()).unwrap_err();
                assert!(error.raw_code() < 0);
                let paths: Vec<_> = listing(&mut index).into_iter().map(|(p, _)| p).collect();
                assert_eq!(paths, [c"d/x".to_owned(), c"k".to_owned()]);
            }
            drop(broken);
            drop(kept);
            drop(repo);
        },
    );
}

/// Snapshot copies outlive later steps, the index's changes behind the snapshot (an
/// entry replaced in place, removed, the index cleared) and the index itself; a
/// drained iterator stays drained. BSan targets this test.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn snapshot_copies_outlive_index_changes_and_the_index() {
    init();
    let index: IndexArc = Index::new().unwrap().into();
    for (n, path) in [c"b", c"a", c"c/d"].into_iter().enumerate() {
        add_staged(&index, path, 0, 1, n as u8 + 1, 0o100644);
    }
    let mut iterator = IndexIterator::new(&index).unwrap();
    let mut steps = iterator.as_mut();
    let first: IndexEntryBuf = steps.next().unwrap();
    // Replaced in place, behind the snapshot.
    add_staged(&index, c"b", 0, 1, 0x7f, 0o100755);
    add_staged(&index, c"new", 0, 1, 1, 0o100644);
    let second = steps.next().unwrap();
    assert_eq!(second.as_ref().path(), c"b");
    assert_eq!(second.as_ref().id().id()[0], 0x7f);
    drop(index);
    let rest: Vec<_> = iterator.as_mut().collect();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].path(), c"c/d");
    assert!(iterator.as_mut().next().is_none());
    assert!(iterator.as_mut().next().is_none());
    drop(iterator);
    assert_eq!(first.as_ref().path(), c"a");
    assert_eq!(first.as_ref().id().id()[0], 2);
    assert_eq!(first.clone(), first);

    // Removed and cleared entries stay in the snapshot until it goes.
    let mut sole = Index::new().unwrap();
    for path in [c"x", c"y"] {
        sole.as_mut().add(blob(path, 1).as_ref()).unwrap();
    }
    let index: IndexArc = sole.into();
    let mut iterator = IndexIterator::new(&index).unwrap();
    let x = iterator.as_mut().next().unwrap();
    let mut other = IndexIterator::new(&index).unwrap();
    drop(index);
    let y = iterator.as_mut().next().unwrap();
    assert_eq!((x.path(), y.path()), (c"x", c"y"));
    drop(iterator);
    let paths: Vec<_> = other.as_mut().map(|e| e.path().to_owned()).collect();
    assert_eq!(paths, [c"x".to_owned(), c"y".to_owned()]);
}

/// Conflicts outlive their cursor while the index stays borrowed; cursors over the
/// same index in turn see the same conflicts, and a drained cursor stays drained.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn conflicts_outlive_their_cursor() {
    init();
    let mut index = Index::new().unwrap();
    let mut handle = index.as_mut();
    handle.add(blob(c"aa-resolved", 1).as_ref()).unwrap();
    for (path, mask) in [(c"a", 7u8), (c"b", 1), (c"c", 2), (c"d", 4), (c"e", 6)] {
        let side = blob(path, mask);
        let pick = |bit: u8| (mask & bit != 0).then(|| side.as_ref());
        handle.add_conflict(pick(1), pick(2), pick(4)).unwrap();
    }
    let seen = {
        let mut cursor = ConflictIterator::new(index.as_mut()).unwrap();
        let conflicts: Vec<_> = cursor.as_mut().collect();
        assert!(cursor.as_mut().next().is_none());
        drop(cursor);
        conflicts
            .iter()
            .map(|c| {
                let side = c.ancestor.or(c.ours).or(c.theirs).unwrap();
                let mask = [c.ancestor, c.ours, c.theirs]
                    .iter()
                    .enumerate()
                    .map(|(n, s)| u8::from(s.is_some()) << n)
                    .sum::<u8>();
                (side.path().to_owned(), mask)
            })
            .collect::<Vec<_>>()
    };
    let expected: Vec<_> = [(c"a", 7u8), (c"b", 1), (c"c", 2), (c"d", 4), (c"e", 6)]
        .iter()
        .map(|&(p, m)| (p.to_owned(), m))
        .collect();
    assert_eq!(seen, expected);
    let mut cursor = ConflictIterator::new(index.as_mut()).unwrap();
    let mut steps = cursor.as_mut();
    let first = steps.next().unwrap();
    let mut reborrow = steps.as_mut();
    let second = reborrow.next().unwrap();
    drop(cursor);
    assert_eq!(first.ancestor.unwrap().path(), c"a");
    assert_eq!(second.ancestor.unwrap().path(), c"b");
    assert_eq!(index.as_ref().entry_count(), 9);
}

/// Threads fill, read and iterate their own indexes at once; TSan watches libgit2's
/// shared state on the way.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn threads_add_and_read_their_own_indexes() {
    init();
    std::thread::scope(|s| {
        for t in 0..4u8 {
            s.spawn(move || {
                let mut index = Index::new().unwrap();
                for n in 0..32u8 {
                    let path = CString::new(format!("t{t}/f{n}")).unwrap();
                    index.as_mut().add(blob(&path, n).as_ref()).unwrap();
                }
                let side = blob(c"conflict", t);
                index
                    .as_mut()
                    .add_conflict(Some(side.as_ref()), Some(side.as_ref()), None)
                    .unwrap();
                assert!(index.as_mut().conflict(c"conflict").is_some());
                let index: IndexArc = index.into();
                let mut iterator = IndexIterator::new(&index).unwrap();
                assert_eq!(iterator.as_mut().count(), 34);
            });
        }
    });
}

/// Snapshot copies cross threads, as their `Send` and `Sync` say: shared by reference
/// with readers that clone them, then moved, compared and dropped on other threads,
/// after the index they were copied from is gone. TSan watches.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn snapshot_copies_cross_threads() {
    init();
    let mut sole = Index::new().unwrap();
    for n in 0..8u8 {
        let path = CString::new(format!("f{n}")).unwrap();
        sole.as_mut().add(blob(&path, n).as_ref()).unwrap();
    }
    let sole: IndexArc = sole.into();
    let copies: Vec<IndexEntryBuf> = IndexIterator::new(&sole).unwrap().as_mut().collect();
    drop(sole);
    let clones: Vec<Vec<IndexEntryBuf>> = std::thread::scope(|s| {
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let copies = &copies;
                s.spawn(move || {
                    for (n, copy) in copies.iter().enumerate() {
                        assert_eq!(copy.path().to_bytes(), format!("f{n}").as_bytes());
                        assert_eq!(copy.as_ref().stage(), Stage::Normal);
                    }
                    copies.clone()
                })
            })
            .collect();
        readers.into_iter().map(|r| r.join().unwrap()).collect()
    });
    std::thread::scope(|s| {
        for moved in clones {
            let copies = &copies;
            s.spawn(move || {
                assert_eq!(&moved, copies);
                drop(moved);
            });
        }
    });
    std::thread::spawn(move || drop(copies)).join().unwrap();
}
