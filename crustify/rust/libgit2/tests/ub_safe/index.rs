//! UB tests for `index/index.rs`: the `git_index` owners, built and released through
//! the safe API, alone and shared with a repository, in either drop order; the index
//! reader fed truncated and corrupted files.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use ffibox::{CArc, CBox};
use libgit2::libgit2::index::index::{Index, IndexArc, IndexBox};
use libgit2::libgit2::oid::oid::OidType;

use crate::fixture::index::{
    add, add_intent_to_add, entry_count, has_owner, isolated_tolerating_leaks, path,
    repository_index, set_repository_index, set_version, write,
};
use crate::fixture::{TempRepo, isolated};

/// A scratch directory for one test, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("crustify-index-ub-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn c_path(path: &Path) -> CString {
    CString::new(path.as_os_str().as_bytes()).unwrap()
}

/// The bytes of a valid index file with `n` entries, as libgit2 writes it.
fn index_bytes(scratch: &Scratch, n: usize) -> Vec<u8> {
    let file = scratch.file("source.index");
    let mut index = Index::open(&c_path(&file)).unwrap();
    for i in 0..n {
        let name = CString::new(format!("dir{}/file-{i}.txt", i % 3)).unwrap();
        assert_eq!(add(&mut index, &name), 0);
    }
    assert_eq!(write(&mut index), 0);
    drop(index);
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    bytes
}

/// The bytes of a valid version-`version` index file of four entries (plus an
/// intent-to-add one from version 3 on), as libgit2 writes it.
fn versioned_index_bytes(scratch: &Scratch, version: u32) -> Vec<u8> {
    let file = scratch.file("versioned.index");
    let mut index = Index::open(&c_path(&file)).unwrap();
    for name in [c"a", c"d0/f0", c"d0/f1", c"d1/x"] {
        assert_eq!(add(&mut index, name), 0);
    }
    if version >= 3 {
        assert_eq!(add_intent_to_add(&mut index, c"d1/new"), 0);
    }
    assert_eq!(set_version(&mut index, version), 0);
    assert_eq!(write(&mut index), 0);
    drop(index);
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    bytes
}

/// A tree cache over `d0` and an invalidated `d1`.
const TREE: &[u8] = b"\x005 2\n\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11d0\x002 0\n\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22\x22d1\x00-1 0\n";
/// One resolved conflict, `a`, with ancestor and ours sides.
const REUC: &[u8] = b"a\x00100644\x00100644\x000\x00\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x33\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44\x44";
/// One conflict's names, without a "theirs" side.
const NAME: &[u8] = b"anc\x00ours\x00\x00";

/// `bytes` (a whole index file) with `extensions` appended after its entries, and a
/// zero checksum, which libgit2 accepts (`index.skipHash`).
fn with_extensions(bytes: &[u8], extensions: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let mut out = bytes[..bytes.len() - 20].to_vec();
    for (signature, data) in extensions {
        out.extend_from_slice(&signature[..]);
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        out.extend_from_slice(data);
    }
    out.extend_from_slice(&[0; 20]);
    out
}

/// Every extension libgit2 knows, and an optional one it keeps unread.
fn all_extensions(bytes: &[u8]) -> Vec<u8> {
    with_extensions(
        bytes,
        &[
            (b"TREE", TREE),
            (b"REUC", REUC),
            (b"NAME", NAME),
            (b"ZOPT", b"kept as is"),
        ],
    )
}

/// A fresh index, through every constructor, and its handles; released once each.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn fresh_indexes_are_released_once() {
    let mut index = Index::new().unwrap();
    let _shared = index.as_ref();
    let _exclusive = index.as_mut();
    assert_eq!(
        (entry_count(&index), path(&index), has_owner(&index)),
        (0, None, false)
    );
    drop(index);

    let index = IndexBox::new().unwrap();
    assert_eq!(entry_count(&index), 0);
    let index = IndexArc::new().unwrap();
    let _ = index.as_ref();
    assert_eq!(entry_count(&index), 0);
}

/// Indexes of each object id type, through the typed constructors: fresh and on
/// disk, as a box and as an arc, written and read back as their own type, refused as
/// the other, and released once in either order; an empty path is refused.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn typed_indexes_are_built_and_released_once() {
    let scratch = Scratch::new("typed");
    for kind in [OidType::Sha1, OidType::Sha256] {
        let other = match kind {
            OidType::Sha1 => OidType::Sha256,
            _ => OidType::Sha1,
        };
        let mut fresh = Index::with_oid_type(kind).unwrap();
        let _shared = fresh.as_ref();
        let _exclusive = fresh.as_mut();
        assert_eq!(
            (entry_count(&fresh), path(&fresh), has_owner(&fresh)),
            (0, None, false)
        );
        assert_eq!(add(&mut fresh, c"a"), 0);
        let shared: IndexArc = fresh.into();
        assert_eq!(entry_count(&shared), 1);
        drop(shared);

        let file = c_path(&scratch.file(&format!("index-{kind:?}")));
        let mut index = Index::open_with_oid_type(&file, kind).unwrap();
        assert_eq!(path(&index).as_deref(), Some(file.as_c_str()));
        for name in [c"b", c"a", c"dir/c"] {
            assert_eq!(add(&mut index, name), 0);
        }
        assert_eq!(write(&mut index), 0);
        let reread = Index::open_with_oid_type(&file, kind).unwrap();
        assert_eq!(entry_count(&reread), 3);
        drop(index);
        assert_eq!(entry_count(&reread), 3);
        assert!(Index::open_with_oid_type(&file, other).is_err());
        assert_eq!(Index::open(&file).is_ok(), kind == OidType::Sha1);
        drop(reread);
        assert!(Index::open_with_oid_type(c"", kind).is_err());
    }
}

/// A box shared as an arc is released once, through the arc.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn shared_box_is_released_once() {
    let mut index = Index::new().unwrap();
    assert_eq!(add(&mut index, c"a"), 0);
    let shared: IndexArc = index.into();
    assert_eq!(entry_count(&shared), 1);
    let _ = shared.as_ref();
}

/// An index backed by a missing file is empty and keeps a copy of the path; the
/// caller's string may go first. An empty path is refused.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_copies_its_path() {
    let scratch = Scratch::new("copies-path");
    let file = c_path(&scratch.file("missing.index"));
    let index = Index::open(&file.clone()).unwrap();
    assert_eq!((entry_count(&index), path(&index)), (0, Some(file)));

    // libgit2 would open it, and writing that index is undefined behaviour.
    assert!(Index::open(c"").is_err());
}

/// What one index writes, another reads back.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_reads_a_written_index() {
    let scratch = Scratch::new("round-trip");
    let file = c_path(&scratch.file("index"));
    let mut index = Index::open(&file).unwrap();
    for name in [c"b", c"a", c"dir/c"] {
        assert_eq!(add(&mut index, name), 0);
    }
    assert_eq!(write(&mut index), 0);
    let reread = Index::open(&file).unwrap();
    assert_eq!(entry_count(&reread), 3);
    drop(index);
    assert_eq!(entry_count(&reread), 3);
}

/// Unreadable files fail without leaking the half-built index.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_refuses_unreadable_files() {
    let scratch = Scratch::new("unreadable");
    assert!(Index::open(&c_path(&scratch.0)).is_err());
    let garbage = scratch.file("garbage");
    std::fs::write(&garbage, b"not an index file, not even close").unwrap();
    assert!(Index::open(&c_path(&garbage)).is_err());
    let empty = scratch.file("empty");
    std::fs::write(&empty, b"").unwrap();
    assert!(Index::open(&c_path(&empty)).is_err());
}

/// Every truncation of a valid index file, and every single-byte corruption of it, is
/// read without a memory error and released (or refused) without a leak.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_survives_truncated_and_corrupted_files() {
    let scratch = Scratch::new("corrupted");
    let bytes = index_bytes(&scratch, 4);
    let file = scratch.file("index");
    let c_file = c_path(&file);
    let stride = if cfg!(bsan) { 7 } else { 1 };
    for len in (0..bytes.len()).step_by(stride) {
        std::fs::write(&file, &bytes[..len]).unwrap();
        assert!(Index::open(&c_file).is_err(), "truncated to {len}");
    }
    // A zero checksum is accepted (`index.skipHash`), so with it every corruption reaches
    // the parser's verdict rather than the checksum's.
    let body = bytes.len() - 20;
    for at in (0..body).step_by(stride) {
        for flip in [0x01, 0x80, 0xff] {
            let mut corrupt = bytes.clone();
            corrupt[at] ^= flip;
            corrupt[body..].fill(0);
            std::fs::write(&file, &corrupt).unwrap();
            if let Ok(index) = Index::open(&c_file) {
                let _ = entry_count(&index);
            }
        }
    }
}

/// Entries of every version, with every extension, are read and released; a mandatory
/// extension libgit2 does not know, or a truncated one, is refused.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_reads_every_version_and_extension() {
    let scratch = Scratch::new("versions");
    let file = scratch.file("index");
    let c_file = c_path(&file);
    for version in [2, 3, 4] {
        let bytes = versioned_index_bytes(&scratch, version);
        let entries = if version >= 3 { 5 } else { 4 };
        std::fs::write(&file, &bytes).unwrap();
        assert_eq!(entry_count(&Index::open(&c_file).unwrap()), entries);
        std::fs::write(&file, all_extensions(&bytes)).unwrap();
        assert_eq!(entry_count(&Index::open(&c_file).unwrap()), entries);
        for refused in [
            with_extensions(&bytes, &[(b"link", b"mandatory")]),
            with_extensions(&bytes, &[(b"REUC", b"a\x00")]),
            with_extensions(&bytes, &[(b"TREE", b"\x00x 0\n")]),
        ] {
            std::fs::write(&file, refused).unwrap();
            assert!(Index::open(&c_file).is_err(), "version {version}");
        }
        let mut truncated = with_extensions(&bytes, &[(b"ZOPT", b"data")]);
        let at = truncated.len() - 20 - b"data".len() - 8;
        truncated[at + 4..at + 8].copy_from_slice(&1000u32.to_be_bytes());
        std::fs::write(&file, truncated).unwrap();
        assert!(Index::open(&c_file).is_err(), "version {version}");
    }
}

/// Every truncation and single-byte corruption of `bytes`, an index file, is read
/// without a memory error, and released or refused.
fn sweep(scratch: &Scratch, bytes: &[u8]) {
    let file = scratch.file("index");
    let c_file = c_path(&file);
    let stride = if cfg!(bsan) { 7 } else { 1 };
    for len in (0..bytes.len()).step_by(stride) {
        std::fs::write(&file, &bytes[..len]).unwrap();
        assert!(Index::open(&c_file).is_err(), "truncated to {len}");
    }
    for at in (0..bytes.len() - 20).step_by(stride) {
        for flip in [0x01, 0x30, 0x80, 0xff] {
            let mut corrupt = bytes.to_vec();
            corrupt[at] ^= flip;
            std::fs::write(&file, &corrupt).unwrap();
            if let Ok(index) = Index::open(&c_file) {
                let _ = entry_count(&index);
            }
        }
    }
}

/// Every truncation and single-byte corruption of a version 2 or 3 index carrying every
/// extension is read without a memory error, and released or refused without a leak.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_survives_corrupted_extensions() {
    let scratch = Scratch::new("corrupted-extensions");
    for version in [2, 3] {
        sweep(
            &scratch,
            &all_extensions(&versioned_index_bytes(&scratch, version)),
        );
    }
}

/// As above, for a version 4 index, whose entries' paths are prefix-compressed. libgit2
/// leaks the path it decompressed when an entry runs past the end of the file, so
/// LeakSanitizer is off for this one (and only this one); the other sanitizers stay on.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_survives_corrupted_version_4_files() {
    isolated_tolerating_leaks(
        concat!(module_path!(), "::open_survives_corrupted_version_4_files"),
        || {
            let scratch = Scratch::new("corrupted-v4");
            sweep(
                &scratch,
                &all_extensions(&versioned_index_bytes(&scratch, 4)),
            );
        },
    );
}

/// A counted reference to a repository's index outlives the repository.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn shared_index_outlives_its_repository() {
    let repo = TempRepo::new("index-outlives");
    let index = repository_index(&repo);
    let again = repository_index(&repo);
    assert!(CArc::ptr_eq(&index, &again));
    assert!(has_owner(&index));
    drop(repo);
    assert!(!has_owner(&index));
    assert_eq!(entry_count(&index), 0);
    drop(again);
    assert_eq!(entry_count(&index), 0);
}

/// The repository outlives the counted references it handed out.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn repository_outlives_its_shared_index() {
    let repo = TempRepo::new("repo-outlives");
    drop(repository_index(&repo));
    let index = repository_index(&repo);
    drop(index);
    assert_eq!(entry_count(&repository_index(&repo)), 0);
}

/// A built index handed to a repository, as an arc, is released once whichever of the
/// two lets go last.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn index_shared_with_a_repository_is_released_once() {
    for arc_first in [true, false] {
        let mut repo = TempRepo::new(&format!("set-index-{arc_first}"));
        let mut index = Index::new().unwrap();
        assert_eq!(add(&mut index, c"kept"), 0);
        let index: IndexArc = index.into();
        set_repository_index(&mut repo, &index);
        assert!(has_owner(&index));
        let from_repo = repository_index(&repo);
        assert!(CArc::ptr_eq(&index, &from_repo));
        drop(from_repo);
        if arc_first {
            drop(index);
            assert_eq!(entry_count(&repository_index(&repo)), 1);
            drop(repo);
        } else {
            drop(repo);
            assert!(!has_owner(&index));
            assert_eq!(entry_count(&index), 1);
        }
    }
}

/// Replacing a repository's index releases only the repository's reference to the old
/// one.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn replaced_repository_index_stays_with_its_arc() {
    let mut repo = TempRepo::new("replaced");
    let old = repository_index(&repo);
    let new: IndexArc = Index::new().unwrap().into();
    set_repository_index(&mut repo, &new);
    assert!(!has_owner(&old));
    assert!(has_owner(&new));
    drop(repo);
    assert_eq!((entry_count(&old), entry_count(&new)), (0, 0));
}

/// Threads building and releasing their own indexes race nothing, the first of them
/// initialising libgit2 included (ThreadSanitizer).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn threads_build_their_own_indexes() {
    isolated(
        concat!(module_path!(), "::threads_build_their_own_indexes"),
        || {
            let scratch = Scratch::new("threads");
            let dir = scratch.0.clone();
            let threads: Vec<_> = (0..8)
                .map(|t| {
                    let file = c_path(&dir.join(format!("index-{t}")));
                    std::thread::spawn(move || {
                        for _ in 0..16 {
                            let mut index = Index::new().unwrap();
                            assert_eq!(add(&mut index, c"x"), 0);
                            let shared: IndexArc = index.into();
                            assert_eq!(entry_count(&shared), 1);
                            let mut on_disk = Index::open(&file).unwrap();
                            assert_eq!(add(&mut on_disk, c"y"), 0);
                            assert_eq!(write(&mut on_disk), 0);
                        }
                    })
                })
                .collect();
            for t in threads {
                t.join().unwrap();
            }
        },
    );
}

/// Both owners are one pointer wide, with a NULL niche (Miri runs this).
#[test]
fn owners_are_pointer_sized() {
    use core::mem::size_of;
    assert_eq!(size_of::<IndexBox>(), size_of::<*mut u8>());
    assert_eq!(size_of::<Option<IndexBox>>(), size_of::<*mut u8>());
    assert_eq!(size_of::<IndexArc>(), size_of::<*mut u8>());
    assert_eq!(
        size_of::<Option<CBox<Index, libgit2::libgit2::index::index::IndexFree>>>(),
        size_of::<usize>()
    );
}
