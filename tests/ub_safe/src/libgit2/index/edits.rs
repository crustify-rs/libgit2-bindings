//! UB tests for the `index/index.rs` routines that read, write and remove entries:
//! borrowed paths across writes, reads and reborrows; entries freed by every removal
//! route, by forced reads and by scans whose callbacks read the lent paths, reenter
//! another scan or panic; tree writes refusing ids that libgit2 would overflow on;
//! writes refusing an empty path that libgit2's racily clean scan would read past; and
//! a shared repository index outliving its repository.

use std::ffi::CString;

use libgit2::libgit2::src::libgit2::index::{Index, IndexArc};
use libgit2::libgit2::src::libgit2::repository::Repository;

use super::Scratch;
use crate::fixture::{init, isolated};

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
