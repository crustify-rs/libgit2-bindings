//! UB tests for the `index/index.rs` routines that read, write and remove entries:
//! borrowed paths across writes, reads and reborrows; entries freed by every removal
//! route, by forced reads and by scans whose callbacks read the lent paths, reenter
//! another scan or panic; tree writes refusing ids that libgit2 would overflow on;
//! writes refusing an empty path that libgit2's racily clean scan would read past; and
//! a shared repository index outliving its repository.

use std::ffi::{CStr, CString};
use std::panic::{AssertUnwindSafe, catch_unwind};

use libgit2::libgit2::core::strarray::StrArrayView;
use libgit2::libgit2::index::index::{
    Capabilities, CapabilitySetting, Index, IndexArc, IndexBox, MatchedPathCallback,
    MatchedPathResult, Stage,
};
use libgit2::libgit2::oid::oid::OidType;
use libgit2::libgit2::repository::repository::Repository;

use super::Scratch;
use crate::fixture::index::{Owner, add_staged, repository_index, set_repository_index};
use crate::fixture::{TempRepo, init, isolated};

/// Adds an entry to either owner (raw, through the fixture: a shared index yields no
/// exclusive handle for `IndexMut::add`).
fn add(index: &impl Owner, path: &CStr, stage: u16, kind: OidType, byte: u8, mode: u32) {
    add_staged(
        index,
        path,
        stage,
        u8::try_from(kind.raw()).unwrap(),
        byte,
        mode,
    );
}

fn seeded(paths: &[(&CStr, u16)]) -> IndexBox {
    let index = Index::new().unwrap();
    for (n, &(path, stage)) in paths.iter().enumerate() {
        add(
            &index,
            path,
            stage,
            OidType::Sha1,
            u8::try_from(n).unwrap() + 1,
            0o100_644,
        );
    }
    index
}

/// The path an index lends lives as long as the handle's borrow of the owner, across
/// writes and reads through other handles of the same owner. `BSan` targets this test.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn path_borrows_outlive_writes_and_reads() {
    init();
    let scratch = Scratch::new("edits-path");
    let file = CString::new(scratch.file("index").to_str().unwrap()).unwrap();
    let mut index = Index::open(&file).unwrap();
    add(&index, c"a", 0, OidType::Sha1, 1, 0o100_644);
    let shared = index.as_ref();
    let path = shared.path().unwrap();
    let again = index.as_ref();
    again.write().unwrap();
    assert_eq!(again.version(), 2);
    assert_eq!(path, file.as_c_str());
    assert_eq!(shared.path(), again.path());
    let mut exclusive = index.as_mut();
    exclusive.read(true).unwrap();
    exclusive.set_version(4).unwrap();
    let path = exclusive.as_ref().path().unwrap().to_owned();
    exclusive.as_mut().read(false).unwrap();
    assert_eq!(path, file);
    drop(index);
    assert!(Index::new().unwrap().as_ref().path().is_none());

    // A repository's shared index keeps its path past the repository.
    let repo = TempRepo::new("edits-path-repo");
    let shared = repository_index(&repo);
    let expected = repo.path.join("index");
    drop(repo);
    let path = shared.as_ref().path().unwrap();
    assert_eq!(path.to_bytes(), expected.to_str().unwrap().as_bytes());
    assert_eq!(shared.as_ref().write_tree().unwrap_err().raw_code(), -1);
    drop(shared);
}

/// Every removal route frees entries once, including conflicts moved to the
/// resolve-undo records, repeated removals and directories emptied twice.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn removals_free_entries_once() {
    init();
    let mut index = seeded(&[
        (c"a", 0),
        (c"c", 1),
        (c"c", 2),
        (c"c", 3),
        (c"d/x", 0),
        (c"d/y", 2),
        (c"d/z/w", 0),
        (c"e", 0),
        (c"e", 3),
    ]);
    let mut handle = index.as_mut();
    handle.remove(c"a", Stage::Normal).unwrap();
    assert!(handle.remove(c"a", Stage::Normal).is_err());
    handle.remove(c"e", Stage::Any).unwrap();
    handle.remove(c"e", Stage::Any).unwrap();
    assert!(handle.remove(c"e", Stage::Any).is_err());
    handle.remove_bypath(c"c").unwrap();
    handle.remove_bypath(c"c").unwrap();
    assert_eq!(handle.as_ref().reuc_entry_count(), 1);
    handle.remove_directory(c"d", Stage::Normal).unwrap();
    handle.remove_directory(c"d", Stage::Normal).unwrap();
    assert!(handle.remove_directory(c"d", Stage::Any).is_err());
    handle.remove_directory(c"d", Stage::Ours).unwrap();
    handle.remove_directory(c"", Stage::Normal).unwrap();
    assert_eq!(index.as_ref().entry_count(), 0);
}

/// Scans whose callbacks read every lent string to its end while C frees the entries
/// already scanned, skip and abort, start another scan on another index, and panic;
/// the index is consistent after each. `BSan` targets this test.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn scan_callbacks_read_lent_paths_while_entries_go() {
    init();
    let paths = [c"a", c"b/c", c"b/d", c"\xff", c"long-path-of-some-length"];
    let mut index = seeded(&paths.map(|p| (p, 0)));
    let mut seen = Vec::new();
    let mut f = |path: &CStr, pattern: Option<&CStr>| {
        seen.push((path.to_owned(), pattern.map(CStr::to_owned)));
        if path.to_bytes().starts_with(b"long") {
            MatchedPathResult::SKIP
        } else {
            MatchedPathResult::APPLY
        }
    };
    let mut callback = MatchedPathCallback::new(&mut f);
    index
        .as_mut()
        .remove_all(StrArrayView::default().as_ref(), Some(&mut callback))
        .unwrap();
    drop(callback);
    assert_eq!(seen.len(), paths.len());
    assert_eq!(index.as_ref().entry_count(), 1);

    // A callback scanning another index with its own callback.
    let mut index = seeded(&paths.map(|p| (p, 0)));
    let mut inner_seen = 0;
    let mut outer = |path: &CStr, _: Option<&CStr>| {
        let mut other = seeded(&[(path, 0), (c"zz", 0)]);
        let mut g = |_: &CStr, _: Option<&CStr>| {
            inner_seen += 1;
            MatchedPathResult::APPLY
        };
        let mut inner = MatchedPathCallback::new(&mut g);
        let names = [path];
        let patterns = StrArrayView::new(&names);
        other
            .as_mut()
            .remove_all(patterns.as_ref(), Some(&mut inner))
            .unwrap();
        assert_eq!(other.as_ref().entry_count(), 1);
        MatchedPathResult::from_code(if path == c"b/d" { -42 } else { 0 })
    };
    let mut callback = MatchedPathCallback::new(&mut outer);
    let patterns = StrArrayView::new(&[c"b/*", c"a"]);
    let error = index
        .as_mut()
        .remove_all(patterns.as_ref(), Some(&mut callback))
        .unwrap_err();
    assert_eq!(error.raw_code(), -42);
    assert_eq!(callback.stopped(), Some(-42));
    drop(callback);
    assert!(inner_seen >= 2);
    assert_eq!(index.as_ref().entry_count(), 3);

    // A panic stops the scan; libgit2 cleans up before it resumes.
    let mut f = |path: &CStr, _: Option<&CStr>| -> MatchedPathResult {
        assert!(path != c"\xff", "stop here");
        MatchedPathResult::APPLY
    };
    let mut callback = MatchedPathCallback::new(&mut f);
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        drop(
            index
                .as_mut()
                .remove_all(StrArrayView::default().as_ref(), Some(&mut callback)),
        );
    }));
    assert!(panicked.is_err());
    // The same callback scans again, this time to completion with no match.
    let missing = StrArrayView::new(&[c"missing"]);
    index
        .as_mut()
        .remove_all(missing.as_ref(), Some(&mut callback))
        .unwrap();
    drop(callback);
    index
        .as_mut()
        .remove_all(StrArrayView::default().as_ref(), None)
        .unwrap();
    assert_eq!(index.as_ref().entry_count(), 0);
}

/// An index with no owner: refused before any callback, and nothing changes.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn update_all_without_an_owner_runs_no_callback() {
    init();
    let mut index = seeded(&[(c"a", 0)]);
    let mut calls = 0;
    let mut f = |_: &CStr, _: Option<&CStr>| {
        calls += 1;
        MatchedPathResult::APPLY
    };
    let mut callback = MatchedPathCallback::new(&mut f);
    for patterns in [StrArrayView::default(), StrArrayView::new(&[c"", c"a"])] {
        let error = index
            .as_mut()
            .update_all(patterns.as_ref(), Some(&mut callback))
            .unwrap_err();
        assert_eq!(error.raw_code(), -1);
        assert!(index.as_mut().update_all(patterns.as_ref(), None).is_err());
    }
    drop(callback);
    assert_eq!(calls, 0);
    assert_eq!(index.as_ref().entry_count(), 1);
}

/// Ids of another type than the repository's, which libgit2 would look up with an
/// overflowing path buffer, are refused through both routes; nothing is written.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn tree_writes_refuse_foreign_ids_without_overflow() {
    init();
    let repo = TempRepo::new("edits-foreign");
    let index = Index::with_oid_type(OidType::Sha256).unwrap();
    add(&index, c"a", 0, OidType::Sha256, 1, 0o100_644);
    add(&index, c"dir/b", 0, OidType::Sha256, 2, 0o100_644);
    let shared: IndexArc = index.into();
    let mut repo = repo;
    set_repository_index(&mut repo, &shared);
    assert_eq!(shared.as_ref().write_tree().unwrap_err().raw_code(), -1);
    // A SHA-1 index holding a SHA-256 id, added before any repository owned it (an
    // owned index looks each added id up, overflowing alike).
    let index = Index::new().unwrap();
    add(&index, c"x", 0, OidType::Sha256, 3, 0o100_644);
    let shared_sha1: IndexArc = index.into();
    let mut other = TempRepo::new("edits-foreign-other");
    set_repository_index(&mut other, &shared_sha1);
    assert_eq!(
        shared_sha1.as_ref().write_tree().unwrap_err().raw_code(),
        -1
    );
    drop(other);
    drop(repo);
}

/// Names too long for a tree are refused before libgit2 writes one; nothing is written.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
#[cfg_attr(
    bsan,
    ignore = "its large inputs outlast the BorrowSanitizer gate, over an hour alone; the other variants run it"
)]
fn tree_writes_refuse_long_names() {
    init();
    let long = CString::new("n".repeat(70_000)).unwrap();
    let index = seeded(&[(&long, 0)]);
    let repo = TempRepo::new("edits-long");
    let shared: IndexArc = index.into();
    let mut repo = repo;
    set_repository_index(&mut repo, &shared);
    let error = shared.as_ref().write_tree().unwrap_err();
    assert_eq!(error.message(), c"tree entry path too long");
    drop(repo);
    drop(shared);
}

/// Case-sensitivity changes re-sort entries and records an index lends nothing from;
/// removals by path then find the re-sorted entries.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn capability_changes_resort_before_removals() {
    init();
    let mut index = seeded(&[(c"B", 0), (c"a", 0), (c"A", 1), (c"A", 2)]);
    let mut handle = index.as_mut();
    for bits in [7, 0, 1] {
        let caps = Capabilities::from_bits(bits).unwrap();
        handle.set_caps(CapabilitySetting::Explicit(caps)).unwrap();
    }
    assert!(handle.set_caps(CapabilitySetting::FromOwner).is_err());
    handle.remove_bypath(c"a").unwrap();
    handle.remove(c"b", Stage::Normal).unwrap();
    assert_eq!(handle.as_ref().reuc_entry_count(), 1);
    drop(index);
}

/// An index file of version 2 holding regular-file entries at `paths`, each modified in
/// 2038, so newer than the file (racily clean), with a zero checksum, which libgit2
/// accepts (`index.skipHash`).
fn racy_index_file(paths: &[&str]) -> Vec<u8> {
    let mut file = b"DIRC".to_vec();
    file.extend_from_slice(&2u32.to_be_bytes());
    file.extend_from_slice(&u32::try_from(paths.len()).unwrap().to_be_bytes());
    for path in paths {
        let start = file.len();
        // ctime, mtime (seconds, nanoseconds), dev, ino, mode, uid, gid, size.
        for field in [0u32, 0, 0x7FFF_FFFF, 0, 0, 0, 0o100_644, 0, 0, 0] {
            file.extend_from_slice(&field.to_be_bytes());
        }
        file.extend_from_slice(&[0x11; 20]);
        file.extend_from_slice(&u16::try_from(path.len()).unwrap().to_be_bytes());
        file.extend_from_slice(path.as_bytes());
        file.push(0);
        while !(file.len() - start).is_multiple_of(8) {
            file.push(0);
        }
    }
    file.extend_from_slice(&[0; 20]);
    file
}

/// Regression for the review report `ub_safe/index-write-empty-path-overread`: writing
/// an index that a repository with a working directory owns, holding the racily clean
/// entries `""` and `"a"`, made libgit2 parse their paths as a pathspec and read one
/// byte past the empty one (`parse_optimized_patterns`). `write` refuses it, through the
/// repository's own index and an index installed in a repository, and writes it once no
/// such repository owns it.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn writes_refuse_empty_paths_libgit2_would_scan() {
    init();
    let scratch = Scratch::new("edits-racy");
    let work = scratch.file("work");
    let repo = Repository::init(&CString::new(work.to_str().unwrap()).unwrap(), false).unwrap();
    let racy = racy_index_file(&["", "a"]);
    std::fs::write(work.join(".git/index"), &racy).unwrap();
    let refused = |index: &IndexArc| {
        let error = index.as_ref().write().unwrap_err();
        assert_eq!(error.raw_code(), -1);
        assert_eq!(
            error.message(),
            c"cannot write an index holding an entry with an empty path beside others"
        );
    };

    let own = repo.as_ref().index().unwrap();
    assert_eq!(own.as_ref().entry_count(), 2);
    refused(&own);
    assert_eq!(std::fs::read(work.join(".git/index")).unwrap(), racy);
    drop(own);

    let file = scratch.file("racy.index");
    std::fs::write(&file, &racy).unwrap();
    let index: IndexArc = Index::open(&CString::new(file.to_str().unwrap()).unwrap())
        .unwrap()
        .into();
    let mut repo = repo;
    repo.as_mut().set_index(Some(&index));
    refused(&index);
    assert_eq!(std::fs::read(&file).unwrap(), racy);
    // Without an owning repository libgit2 scans nothing, and writes the index.
    drop(repo);
    index.as_ref().write().unwrap();
    assert_eq!(index.as_ref().entry_count(), 2);
}

/// A single racily clean empty path is no pathspec libgit2 parses: `write` lets it scan
/// the working directory, in a child (a diff may load filters), and write the index.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn writes_scan_a_lone_empty_path() {
    isolated(
        concat!(module_path!(), "::writes_scan_a_lone_empty_path"),
        || {
            let scratch = Scratch::new("edits-racy-alone");
            let work = scratch.file("work");
            let path = CString::new(work.to_str().unwrap()).unwrap();
            let repo = Repository::init(&path, false).unwrap();
            std::fs::write(work.join(".git/index"), racy_index_file(&[""])).unwrap();
            let index = repo.as_ref().index().unwrap();
            index.as_ref().write().unwrap();
            assert_eq!(index.as_ref().entry_count(), 1);
        },
    );
}
