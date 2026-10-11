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

mod edits;

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

/// Pointer-free options remain independent values under every combination and
/// extreme integer input; these tests can also run under Miri without C calls.
#[test]
fn scalar_options_exhaustive() {
    use libgit2::libgit2::index::index::{AddFlags, Capabilities, CapabilitySetting};

    for a in 0..8 {
        for b in 0..8 {
            let lhs = AddFlags::from_bits_retain(a);
            let rhs = AddFlags::from_bits_retain(b);
            assert_eq!((lhs | rhs).bits(), a | b);
            assert_eq!(lhs.contains(rhs), a & b == b);
            assert_eq!(lhs.without(rhs).bits(), a & !b);
            let mut assigned = lhs;
            assigned |= rhs;
            assert_eq!(assigned, lhs.union(rhs));

            let lhs = Capabilities::from_bits(a as i32).unwrap();
            let rhs = Capabilities::from_bits(b as i32).unwrap();
            assert_eq!((lhs | rhs).bits(), (a | b) as i32);
            assert_eq!(lhs.contains(rhs), a & b == b);
            assert_eq!(lhs.without(rhs).bits(), (a & !b) as i32);
            let mut assigned = lhs;
            assigned |= rhs;
            assert_eq!(assigned, lhs.union(rhs));
            assert_eq!(CapabilitySetting::from(assigned).to_raw(), (a | b) as i32);
        }
    }
    for bits in [0, 8, 1 << 31, u32::MAX] {
        let flags = AddFlags::from_bits_retain(bits);
        assert_eq!(flags.bits(), bits);
        assert_eq!((flags | AddFlags::FORCE).bits(), bits | 1);
        assert_eq!(flags.without(flags), AddFlags::empty());
    }
    assert_eq!(CapabilitySetting::FromOwner.to_raw(), -1);
}

/// Auto Send/Sync is appropriate: each option is just a copied integer, with no
/// hidden foreign state. Shared readers cannot observe writes to another copy.
#[test]
fn scalar_options_threads() {
    use libgit2::libgit2::index::index::{AddFlags, Capabilities, CapabilitySetting};

    let flags = AddFlags::FORCE | AddFlags::CHECK_PATHSPEC | AddFlags::DISABLE_PATHSPEC_MATCH;
    let caps = Capabilities::IGNORE_CASE | Capabilities::NO_FILEMODE | Capabilities::NO_SYMLINKS;
    let setting = CapabilitySetting::FromOwner;
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let shared = (&flags, &caps, &setting);
            scope.spawn(move || {
                let mut local = *shared.0;
                local |= AddFlags::from_bits_retain(1 << 31);
                assert_eq!(shared.0.bits(), 7);
                assert_eq!(local.bits(), 7 | 1 << 31);
                assert_eq!(*shared.1, Capabilities::all());
                assert_eq!(shared.2.to_raw(), -1);
            });
        }
    });
    let handle = std::thread::spawn(move || (flags, caps, setting));
    assert_eq!(handle.join().unwrap(), (flags, caps, setting));
}

mod matched_path_callback {
    use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
    use std::ffi::{CStr, CString};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn borrowed_strings_payload_reborrows_and_nested_callbacks() {
        let mut copies = Vec::new();
        let mut inner_fn = |path: &CStr, pattern: Option<&CStr>| {
            copies.push((path.to_owned(), pattern.map(CStr::to_owned)));
            MatchedPathResult::SKIP
        };
        let mut inner = MatchedPathCallback::new(&mut inner_fn);
        let mut outer_fn = |path: &CStr, pattern: Option<&CStr>| inner.call(path, pattern);
        let mut outer = MatchedPathCallback::new(&mut outer_fn);
        for i in 0..32 {
            let path = CString::new(vec![b'a', 0xff, b'0' + i % 10]).unwrap();
            let pattern = CString::new(b"*\xfe".to_vec()).unwrap();
            assert_eq!(outer.call(&path, Some(&pattern)), MatchedPathResult::SKIP);
            assert_eq!(outer.call(c"", None), MatchedPathResult::SKIP);
            assert_eq!(outer.stopped(), None);
        }
        drop(outer);
        drop(outer_fn);
        drop(inner);
        drop(inner_fn);
        assert_eq!(copies.len(), 64);
        assert_eq!(copies[0].0.to_bytes(), b"a\xff0");
        assert_eq!(copies[0].1.as_ref().unwrap().to_bytes(), b"*\xfe");
        assert_eq!(copies[1], (c"".to_owned(), None));
    }

    #[test]
    fn arbitrary_codes_and_reuse_after_abort() {
        for code in [i32::MIN, -12345, -7, -1, 0, 1, 256, i32::MAX] {
            let mut f = |_: &CStr, _: Option<&CStr>| MatchedPathResult::from_code(code);
            let mut callback = MatchedPathCallback::new(&mut f);
            for pattern in [None, Some(c""), Some(c"*.rs")] {
                assert_eq!(callback.call(c"file.rs", pattern).code(), code);
                assert_eq!(callback.stopped(), (code < 0).then_some(code));
            }
        }
    }

    #[test]
    fn panic_is_resumed_and_callback_can_be_reused() {
        let mut first = true;
        let mut f = |_: &CStr, _: Option<&CStr>| {
            if std::mem::take(&mut first) {
                std::panic::panic_any(String::from("matched-path panic"));
            }
            MatchedPathResult::APPLY
        };
        let mut callback = MatchedPathCallback::new(&mut f);
        let panic = catch_unwind(AssertUnwindSafe(|| callback.call(c"file", None))).unwrap_err();
        assert_eq!(
            panic.downcast_ref::<String>().unwrap(),
            "matched-path panic"
        );
        assert_eq!(callback.stopped(), Some(MatchedPathResult::ABORT.code()));
        assert_eq!(callback.call(c"file", Some(c"*")), MatchedPathResult::APPLY);
        assert_eq!(callback.stopped(), None);
    }
}

/// Stage decoding is pure Rust, including extreme/unknown discriminants.
#[test]
fn stage_decoding_is_total() {
    use libgit2::libgit2::index::index::Stage;
    for n in [i32::MIN, -2, -1, 0, 1, 2, 3, 4, i32::MAX] {
        let decoded = Stage::from_raw(n);
        assert_eq!(decoded.is_some(), (-1..=3).contains(&n));
        if let Some(stage) = decoded {
            assert_eq!(stage.raw(), n);
        }
    }
}

/// A snapshot holds its own counted index reference. Multiple snapshots may be
/// freed in any order, including after their original Rust index owner is gone.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn iterator_snapshot_owners_and_reborrows() {
    use libgit2::libgit2::index::index::{IndexIterator, IndexIteratorRef};

    fn read(_: IndexIteratorRef<'_>) {}
    for populated in [false, true] {
        for parent_first in [false, true] {
            let mut index = Index::new().unwrap();
            if populated {
                for path in [c"z", c"dir/a", c"b"] {
                    assert_eq!(add(&mut index, path), 0);
                }
            }
            let index: IndexArc = index.into();
            let mut first = IndexIterator::new(&index).unwrap();
            let second = IndexIterator::new(&index).unwrap();
            let mut exclusive = first.as_mut();
            read(exclusive.as_ref());
            {
                let reborrow = exclusive.as_mut();
                read(reborrow.as_ref());
            }
            read(exclusive.as_ref());
            let shared = first.as_ref();
            read(shared);
            read(shared);
            if parent_first {
                drop(index);
                read(first.as_ref());
                drop(first);
                read(second.as_ref());
                drop(second);
            } else {
                drop(second);
                drop(first);
                // The snapshots must release just their own counted references.
                drop(IndexIterator::new(&index).unwrap());
                drop(index);
            }
        }
    }
}

#[test]
fn entry_flags_all_storage_bits() {
    use libgit2::libgit2::index::index::EntryFlags;
    for bits in 0..=u16::MAX {
        let mut flags = EntryFlags::from_bits_retain(u32::from(bits));
        flags.insert(EntryFlags::EXTENDED);
        flags.remove(EntryFlags::VALID);
        std::hint::black_box(flags.bits());
    }
}

/// C retains the parent pointer across Rust handle reborrows. No Rust reference
/// covers C memory, and the owner releases the cursor before releasing its index.
mod conflict_cursor {
    #![forbid(unsafe_code)]
    use libgit2::libgit2::index::index::{ConflictIterator, EntryExtendedFlags, Index};

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn retained_parent_and_reborrows() {
        let mut index = Index::new().unwrap();
        for _ in 0..32 {
            let mut parent = index.as_mut();
            let mut cursor = ConflictIterator::new(parent.as_mut()).unwrap();
            let first = cursor.as_ref();
            let second = first;
            let _ = (first, second);
            {
                let mut handle = cursor.as_mut();
                let mut nested = handle.as_mut();
                let _shared = nested.as_ref();
                let _exclusive = nested.as_mut();
            }
            drop(cursor);
            // The parent is usable again after cursor destruction.
            let _parent = parent.as_mut();
        }
    }

    #[test]
    fn extended_flag_values_are_plain_data() {
        // Scalar flag values, unlike C owners, are naturally Send + Sync.
        let flags = EntryExtendedFlags::from_bits_retain(u32::MAX);
        std::thread::scope(|s| {
            s.spawn(|| assert!(flags.contains(EntryExtendedFlags::ON_DISK)));
            s.spawn(move || assert_eq!(flags.bits(), u32::MAX));
        });
    }
}

#[test]
fn timestamp_extremes_and_reborrows() {
    use libgit2::libgit2::index::index::IndexTime;
    let mut value = IndexTime::default();
    for seconds in [i32::MIN, -1, 0, 1, i32::MAX] {
        for nanos in [0, 1, 999_999_999, 1_000_000_000, u32::MAX] {
            let mut exclusive = value.as_mut();
            exclusive.as_mut().set_seconds(seconds);
            exclusive.as_mut().set_nanoseconds(nanos);
            let shared = exclusive.as_ref();
            let another = shared;
            assert_eq!((shared.seconds(), another.nanoseconds()), (seconds, nanos));
            let copy = shared.to_time();
            exclusive.set_seconds(0);
            assert_eq!(copy.as_ref().seconds(), seconds);
        }
    }
}

mod entry_value {
    //! UB tests for the `IndexEntry` value (`git_index_entry`) and its handles: every
    //! getter and setter, exclusive and shared reborrows, copies that outlive the handle
    //! they came from, entries borrowing paths of several lifetimes, and values shared
    //! and moved across threads.

    use std::ffi::{CStr, CString};

    use ffibox::CZeroable;
    use libgit2::libgit2::core::types::FileMode;
    use libgit2::libgit2::index::index::{
        EntryExtendedFlags, EntryFlags, IndexEntry, IndexEntryMut, IndexEntryRef, IndexTime,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidType};

    fn id(kind: OidType, byte: u8) -> Oid {
        let mut oid = Oid::zero(kind);
        oid.as_mut().set_id([byte; 32]);
        oid
    }

    /// Every field takes its extreme values through the exclusive handle, reborrowed
    /// at each step, and reads back the same through shared reborrows and copies.
    #[test]
    fn every_field_round_trips_through_reborrows() {
        let first = CString::new("dir/first").unwrap();
        let second = CString::new("second").unwrap();
        let mut entry = IndexEntry::new(&first, Oid::zeroed().as_ref(), FileMode::Blob);
        for (step, value) in [0, 1, 0x7fff_ffff, u32::MAX].into_iter().enumerate() {
            let mut handle: IndexEntryMut<'_, '_> = entry.as_mut();
            {
                let mut inner = handle.as_mut();
                inner.set_dev(value);
                inner.set_ino(value ^ 1);
                inner.set_uid(value.wrapping_add(2));
                inner.set_gid(value.wrapping_add(3));
                inner.set_file_size(value.wrapping_add(4));
                inner.set_mode(value);
            }
            let seconds = value as i32;
            handle.set_ctime(IndexTime::new(seconds, value).as_ref());
            handle.mtime_mut().set_seconds(seconds.wrapping_neg());
            handle.mtime_mut().set_nanoseconds(!value);
            handle.set_id(id(OidType::Sha256, step as u8).as_ref());
            handle
                .set_flags(EntryFlags::from_bits_retain(value & 0xffff))
                .unwrap();
            handle
                .set_flags_extended(EntryExtendedFlags::from_bits_retain(value >> 16))
                .unwrap();
            handle.set_path(if step % 2 == 0 { &second } else { &first });
            let shared: IndexEntryRef<'_, '_> = handle.as_ref();
            let again = shared;
            assert_eq!(shared.dev(), value);
            assert_eq!(again.ino(), value ^ 1);
            assert_eq!(shared.uid(), value.wrapping_add(2));
            assert_eq!(shared.gid(), value.wrapping_add(3));
            assert_eq!(shared.file_size(), value.wrapping_add(4));
            assert_eq!(shared.mode(), value);
            assert_eq!(shared.ctime().seconds(), seconds);
            assert_eq!(shared.ctime().nanoseconds(), value);
            assert_eq!(shared.mtime().seconds(), seconds.wrapping_neg());
            assert_eq!(shared.mtime().nanoseconds(), !value);
            assert_eq!(shared.id().kind(), Some(OidType::Sha256));
            assert_eq!(shared.id().id(), [step as u8; 32]);
            assert_eq!(shared.flags().bits(), value & 0xffff);
            assert_eq!(shared.flags_extended().bits(), value >> 16);
            let copy = shared.to_index_entry();
            handle.set_dev(!value);
            assert_eq!(copy.as_ref().dev(), value);
            assert_eq!(handle.as_ref().path(), copy.as_ref().path());
        }
        let mode_cases = [
            (FileMode::Unreadable, 0),
            (FileMode::Tree, 0o040000),
            (FileMode::Blob, 0o100644),
            (FileMode::BlobExecutable, 0o100755),
            (FileMode::Link, 0o120000),
            (FileMode::Commit, 0o160000),
        ];
        for (mode, raw) in mode_cases {
            entry.as_mut().set_filemode(mode);
            assert_eq!(
                (entry.as_ref().mode(), entry.as_ref().filemode()),
                (raw, Some(mode))
            );
        }
        entry.as_mut().set_mode(0o100664);
        assert_eq!(entry.as_ref().filemode(), None);
    }

    /// The in-place id and time handles write the entry's own bytes, and the shared
    /// projections they leave behind see the writes.
    #[test]
    fn projected_fields_write_in_place() {
        let mut entry = IndexEntry::new(c"p", id(OidType::Sha1, 1).as_ref(), FileMode::Blob);
        {
            let mut handle = entry.as_mut();
            let mut oid = handle.id_mut();
            oid.set_kind(OidType::Sha256);
            let mut bytes = oid.as_ref().id();
            bytes[31] = 0xee;
            oid.set_id(bytes);
        }
        {
            let mut handle = entry.as_mut();
            let mut ctime = handle.ctime_mut();
            ctime.set_seconds(i32::MIN);
            ctime.set_nanoseconds(1_000_000_000);
        }
        {
            let mut handle = entry.as_mut();
            handle.set_mtime(IndexTime::new(-5, 6).as_ref());
            let debug = format!("{handle:?}");
            assert!(debug.contains("mtime: (-5, 6)"), "{debug}");
        }
        let shared = entry.as_ref();
        let id = shared.id();
        let ctime = shared.ctime();
        assert_eq!(
            (id.kind(), id.id()[0], id.id()[31]),
            (Some(OidType::Sha256), 1, 0xee)
        );
        assert_eq!(
            (ctime.seconds(), ctime.nanoseconds()),
            (i32::MIN, 1_000_000_000)
        );
        assert_eq!(
            (shared.mtime().seconds(), shared.mtime().nanoseconds()),
            (-5, 6)
        );
        assert_eq!(shared.path(), c"p");
    }

    /// Entries borrowing a vector of owned paths, a `'static` literal and a shorter-lived
    /// string, swapped between them; every handle and copy is dropped before the paths.
    #[test]
    fn entries_borrow_paths_of_several_lifetimes() {
        let paths: Vec<CString> = (0..64)
            .map(|i| CString::new(format!("{i}/{}", "x".repeat(i))).unwrap())
            .collect();
        let mut entries: Vec<IndexEntry<'_>> = paths
            .iter()
            .map(|p| IndexEntry::new(p, Oid::zeroed().as_ref(), FileMode::Blob))
            .collect();
        let handles: Vec<IndexEntryRef<'_, '_>> = entries.iter().map(IndexEntry::as_ref).collect();
        for (handle, path) in handles.iter().zip(&paths) {
            assert_eq!(handle.path(), path.as_c_str());
        }
        let copies: Vec<IndexEntry<'_>> = handles.iter().map(|h| h.to_index_entry()).collect();
        drop(handles);
        for (i, entry) in entries.iter_mut().enumerate() {
            entry.as_mut().set_path(&paths[63 - i]);
        }
        assert_eq!(entries[0].as_ref().path(), paths[63].as_c_str());
        assert_eq!(copies[0].as_ref().path(), paths[0].as_c_str());
        let literal: &'static CStr = c"static";
        entries[1].as_mut().set_path(literal);
        {
            let short = CString::new("short").unwrap();
            let mut scoped = entries[2];
            scoped.as_mut().set_path(&short);
            assert_eq!(scoped.as_ref().path(), c"short");
            assert_eq!(entries[2].as_ref().path(), paths[61].as_c_str());
        }
        assert_eq!(entries[1].as_ref().path(), literal);
        assert_ne!(entries[1], copies[1]);
        drop((entries, copies));
    }

    /// The value is `Send` and `Sync`: threads read one entry together and each mutate a
    /// copy of their own, while the paths outlive the scope.
    #[test]
    fn entry_values_cross_threads() {
        let path = CString::new("threaded/path").unwrap();
        let entry = IndexEntry::new(
            &path,
            id(OidType::Sha1, 9).as_ref(),
            FileMode::BlobExecutable,
        );
        std::thread::scope(|s| {
            for i in 0..8u32 {
                let shared = &entry;
                let mut mine = entry;
                s.spawn(move || {
                    assert_eq!(shared.as_ref().path(), c"threaded/path");
                    assert_eq!(shared.as_ref().id().id()[0], 9);
                    mine.as_mut().set_file_size(i);
                    mine.as_mut().set_flags(EntryFlags::VALID).unwrap();
                    assert_eq!(mine.as_ref().file_size(), i);
                    assert_ne!(&mine, shared);
                });
            }
        });
        assert_eq!(entry.as_ref().file_size(), 0);
    }
}

mod options_value {
    //! UB tests for the `IndexOptions` value (`git_index_options`): its constructors,
    //! handles and reborrows, the indexes built from it, and values shared and built
    //! across threads.

    use libgit2::libgit2::index::index::{Index, IndexOptions, IndexOptionsRef};
    use libgit2::libgit2::oid::oid::OidType;

    /// Defaults, typed options and every setter value read back through shared and
    /// exclusive reborrows; the C-built value matches the one Rust compares it with.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn options_handles_and_reborrows() {
        let mut options = IndexOptions::default();
        assert_eq!(options, IndexOptions::new());
        let shared: IndexOptionsRef<'_> = options.as_ref();
        assert_eq!((shared.version(), shared.oid_type()), (1, None));
        for kind in [Some(OidType::Sha256), None, Some(OidType::Sha1)] {
            let mut handle = options.as_mut();
            handle.as_mut().set_oid_type(kind);
            assert_eq!(handle.as_ref().oid_type(), kind);
        }
        assert_eq!(options, IndexOptions::with_oid_type(OidType::Sha1));
        let copy = options;
        options.as_mut().set_oid_type(None);
        assert_eq!(copy.as_ref().oid_type(), Some(OidType::Sha1));
        assert_eq!(
            format!("{copy:?}"),
            "IndexOptions { version: 1, oid_type: Some(Sha1) }"
        );
    }

    /// Threads build options through C concurrently, share one value, and build
    /// indexes from typed options.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn options_cross_threads() {
        let shared = IndexOptions::with_oid_type(OidType::Sha256);
        std::thread::scope(|s| {
            for i in 0..8 {
                let shared = &shared;
                s.spawn(move || {
                    let kind = if i % 2 == 0 {
                        OidType::Sha1
                    } else {
                        OidType::Sha256
                    };
                    let mut mine = IndexOptions::new();
                    mine.as_mut().set_oid_type(Some(kind));
                    assert_eq!(mine.as_ref().oid_type(), Some(kind));
                    assert_eq!(shared.as_ref().oid_type(), Some(OidType::Sha256));
                    drop(Index::with_oid_type(kind).unwrap());
                });
            }
        });
    }
}
