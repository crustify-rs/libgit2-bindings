//! Equivalence tests for the `index/index.rs` routines that lend, add and replace
//! entries and conflicts: `IndexMut::{entry, entry_by_path, conflict, add,
//! add_from_buffer, add_conflict, read_tree}`, `IndexEntryRef::{stage, is_conflict}` and
//! the two iterators' `next` against `git_index_get_byindex`, `git_index_get_bypath`,
//! `git_index_conflict_get`, `git_index_add`, `git_index_add_from_buffer`,
//! `git_index_conflict_add`, `git_index_read_tree`, `git_index_entry_stage`,
//! `git_index_entry_is_conflict`, `git_index_iterator_next` and
//! `git_index_conflict_next`, on independently owned indexes (and repositories),
//! compared after every step of a sequence: status, error, every entry's fields, the
//! entries each lookup lends, the resolved-conflict count and the blob ids written.
//!
//! `Stage::Any` lookups diverge on purpose (see `IndexMut::entry_by_path`): the
//! corrected behaviour's regression is a unit test, and only concrete stages compare
//! here.

use ffibox::CZeroable;
use libgit2::libgit2::core::types::FileMode;
use libgit2::libgit2::index::index::{
    Capabilities, ConflictIterator, EntryExtendedFlags, EntryFlags, IndexEntry, IndexEntryRef,
    IndexIterator, IndexMut, Stage,
};
use libgit2::libgit2::object::blob;
use libgit2::libgit2::object::tree::{Tree, TreeArc, TreeBuilder, TreeBuilderBox};
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::{RepositoryBox, RepositoryError, RepositoryRef};

use super::*;

/// A status and the thread's error when it failed.
type Status = (c_int, Option<(c_int, CString)>);

fn c_status(rc: c_int) -> Status {
    (rc, (rc != 0).then(c_last))
}

fn rust_status(result: Result<(), RepositoryError>) -> Status {
    match result {
        Ok(()) => (0, None),
        Err(e) => (e.raw_code(), Some((e.raw_class(), e.message().to_owned()))),
    }
}

/// Every field of an entry, copied.
#[derive(Clone, Debug, PartialEq)]
struct Fields {
    path: CString,
    times: [(i32, u32); 2],
    stat: [u32; 6],
    id: (u8, [u8; 32]),
    flags: (u16, u16),
}

/// The fields of the entry at `e`, NULL for `None`.
fn c_fields(e: *const ffi::git_index_entry) -> Option<Fields> {
    {
        // SAFETY: NULL or a live entry, copied before any other call.
        let e = unsafe { e.as_ref()? };
        // SAFETY: as above.
        unsafe {
            Some(Fields {
                path: CStr::from_ptr(e.path).to_owned(),
                times: [
                    (e.ctime.seconds, e.ctime.nanoseconds),
                    (e.mtime.seconds, e.mtime.nanoseconds),
                ],
                stat: [e.dev, e.ino, e.mode, e.uid, e.gid, e.file_size],
                id: (e.id.type_, e.id.id),
                flags: (e.flags, e.flags_extended),
            })
        }
    }
}

/// The same, through the safe getters.
fn rust_fields(e: IndexEntryRef<'_, '_>) -> Fields {
    let (c, m, id) = (e.ctime(), e.mtime(), e.id());
    Fields {
        path: e.path().to_owned(),
        times: [
            (c.seconds(), c.nanoseconds()),
            (m.seconds(), m.nanoseconds()),
        ],
        stat: [e.dev(), e.ino(), e.mode(), e.uid(), e.gid(), e.file_size()],
        id: (id.raw_kind(), id.id()),
        flags: (
            u16::try_from(e.flags().bits()).unwrap(),
            u16::try_from(e.flags_extended().bits()).unwrap(),
        ),
    }
}

/// Every entry and the resolved-conflict count, through C.
fn c_observe(index: *mut ffi::git_index) -> (Vec<Fields>, usize) {
    {
        // SAFETY: a live index lending no entry; each is copied before the next call.
        let index_entrycount = unsafe { ffi::git_index_entrycount(index) };
        // SAFETY: as above.
        let entries = unsafe {
            (0..index_entrycount)
                .map(|n| c_fields(ffi::git_index_get_byindex(index, n)).unwrap())
                .collect()
        };
        // SAFETY: as above.
        unsafe { (entries, ffi::git_index_reuc_entrycount(index)) }
    }
}

/// The same, through the safe lending routines.
fn rust_observe(index: &IndexMut<'_>) -> (Vec<Fields>, usize) {
    let shared = index.as_ref();
    let entries = (0..shared.entry_count())
        .map(|n| rust_fields(index.entry(n).unwrap()))
        .collect();
    (entries, shared.reuc_entry_count())
}

/// An entry of mode `mode` at `path` and `stage`, of id type `kind` with first id byte
/// `byte`, and with extended flags `extended`.
fn entry(path: &CStr, stage: u16, kind: u8, byte: u8, mode: u32, extended: u16) -> IndexEntry<'_> {
    let mut entry = IndexEntry::new(path, Oid::zeroed().as_ref(), FileMode::Blob);
    let mut handle = entry.as_mut();
    handle.set_mode(mode);
    handle
        .set_flags(EntryFlags::from_bits_retain(u32::from(stage) << 12))
        .unwrap();
    handle
        .set_flags_extended(EntryExtendedFlags::from_bits_retain(extended.into()))
        .unwrap();
    handle.set_dev(u32::from(byte) + 1);
    handle.set_file_size(u32::from(byte) * 3);
    handle.mtime_mut().set_seconds(i32::from(byte));
    handle.id_mut().set_raw_kind(kind);
    let mut id = [0; 32];
    id[0] = byte;
    handle.id_mut().set_id(id);
    entry
}

/// The entry an `(path, stage, kind, byte, mode, extended)` row describes.
type Row<'a> = (&'a CStr, u16, u8, u8, u32, u16);

fn row_entry<'a>(row: &Row<'a>) -> IndexEntry<'a> {
    entry(row.0, row.1, row.2, row.3, row.4, row.5)
}

/// Both indexes agree on their entries and resolved-conflict count.
fn assert_same(c: *mut ffi::git_index, rust: &IndexMut<'_>, step: &str) {
    assert_eq!(c_observe(c), rust_observe(rust), "{step}");
}

/// Adding entries, valid and refused, replacing, colliding and conflicting, agrees step
/// by step.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn add_sequences_match() {
    init();
    let c = c_open(None).1.unwrap();
    let mut rust = Index::new().unwrap();
    let long = CString::new(vec![b'l'; 0x1003]).unwrap();
    let rows: &[Row<'_>] = &[
        (c"a", 0, 1, 1, 0o100_644, 0),
        (c"a", 0, 1, 2, 0o100_755, 0),
        (c"b", 0, 1, 3, 0o120_000, 0x2000),
        (c"b", 0, 1, 3, 0o120_000, 0xFFFF),
        (c"sub", 0, 2, 4, 0o160_000, 0),
        (c"a/b", 0, 1, 5, 0o100_644, 0),
        (c"a/c", 0, 1, 5, 0o100_644, 0),
        (c"a", 0, 1, 6, 0o100_644, 0),
        (c"c", 2, 1, 7, 0o100_644, 0),
        (c"c", 0, 1, 8, 0o100_644, 0),
        (c"c", 3, 1, 9, 0o100_644, 0),
        (c"c/d", 2, 1, 9, 0o100_644, 0),
        (c"t", 0, 1, 1, 0o040_000, 0),
        (c"u", 0, 1, 1, 0o100_664, 0),
        (c"u", 0, 1, 1, 0, 0),
        (c"x", 0, 0, 0, 0o100_644, 0),
        (c".git/config", 0, 1, 1, 0o100_644, 0),
        (c"../up", 0, 1, 1, 0o100_644, 0),
        (c"d/./e", 0, 1, 1, 0o100_644, 0),
        (c"d//e", 0, 1, 1, 0o100_644, 0),
        (c"trailing/", 0, 1, 1, 0o100_644, 0),
        (c"", 0, 1, 1, 0o100_644, 0),
        (&long, 0, 1, 1, 0o100_644, 0),
        (c"\xff\xfe", 1, 3, 0xFF, 0o100_644, 0),
    ];
    for row in rows {
        let entry = row_entry(row);
        clear();
        // SAFETY: a live index lending no entry; C copies the entry and its path.
        let expected = c_status(unsafe { ffi::git_index_add(c.0, &entry.into_raw()) });
        clear();
        let actual = rust_status(rust.as_mut().add(entry.as_ref()));
        assert_eq!(expected, actual, "{row:?}");
        assert_same(c.0, &rust.as_mut(), &format!("{row:?}"));
    }
}

/// Lookups by position, by path and stage, and of conflicts lend the same entries, or
/// none, with the same error, in case-sensitive and case-insensitive indexes.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn lookups_match() {
    init();
    let c = c_open(None).1.unwrap();
    let mut rust = Index::new().unwrap();
    let rows: &[Row<'_>] = &[
        (c"B", 0, 1, 1, 0o100_644, 0),
        (c"a", 1, 1, 2, 0o100_644, 0),
        (c"a", 3, 1, 3, 0o100_644, 0),
        (c"b", 0, 1, 4, 0o100_644, 0),
        (c"b", 2, 1, 5, 0o100_644, 0),
        (c"d/e", 1, 1, 6, 0o100_644, 0),
        (c"d/e", 2, 1, 7, 0o100_644, 0),
        (c"d/e", 3, 1, 8, 0o100_644, 0),
        (c"z", 0, 1, 9, 0o100_644, 0),
    ];
    for row in rows {
        let entry = row_entry(row);
        // SAFETY: as in `add_sequences_match`.
        assert_eq!(unsafe { ffi::git_index_add(c.0, &entry.into_raw()) }, 0);
        rust.as_mut().add(entry.as_ref()).unwrap();
    }
    let paths = [
        c"a", c"A", c"b", c"B", c"d/e", c"D/E", c"d", c"z", c"", c"missing",
    ];
    for ignore_case in [false, true, false] {
        let caps = if ignore_case {
            Capabilities::IGNORE_CASE
        } else {
            Capabilities::empty()
        };
        // SAFETY: a live index lending no entry.
        assert_eq!(unsafe { ffi::git_index_set_caps(c.0, caps.bits()) }, 0);
        rust.as_mut().set_caps(caps.into()).unwrap();
        let handle = rust.as_mut();
        for position in [0, 3, 8, 9, 10, usize::MAX] {
            // SAFETY: a live index lending no entry; the entry is copied at once.
            let expected = c_fields(unsafe { ffi::git_index_get_byindex(c.0, position) });
            assert_eq!(
                expected,
                handle.entry(position).map(rust_fields),
                "{position}"
            );
        }
        for path in paths {
            for stage in [Stage::Normal, Stage::Ancestor, Stage::Ours, Stage::Theirs] {
                clear();
                // SAFETY: as above; `path` is read for the call.
                let found = unsafe { ffi::git_index_get_bypath(c.0, path.as_ptr(), stage.raw()) };
                let expected = (c_fields(found), c_last());
                clear();
                let lent = handle.entry_by_path(path, stage).map(rust_fields);
                assert_eq!(
                    expected,
                    (lent, c_last()),
                    "{path:?} {stage:?} {ignore_case}"
                );
            }
            let mut slots = [ptr::null(); 3];
            let [a, o, t] = &mut slots;
            clear();
            // SAFETY: three writable slots, a live index and a NUL-terminated key.
            let rc = unsafe { ffi::git_index_conflict_get(a, o, t, c.0, path.as_ptr()) };
            let expected = (rc == 0).then(|| slots.map(c_fields));
            let expected_error = c_last();
            clear();
            let actual = handle.conflict(path).map(|conflict| {
                [conflict.ancestor, conflict.ours, conflict.theirs].map(|s| s.map(rust_fields))
            });
            assert_eq!((expected, expected_error), (actual, c_last()), "{path:?}");
            if rc != 0 {
                assert_eq!(rc, ffi::GIT_ENOTFOUND);
            }
        }
    }
}

/// Recording conflicts agrees step by step: sides replacing normal entries, partial and
/// differing-path sides, refused modes and paths, and nothing at all.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn conflict_add_sequences_match() {
    init();
    let c = c_open(None).1.unwrap();
    let mut rust = Index::new().unwrap();
    for row in [(c"f", 0, 1, 1, 0o100_644, 0), (c"g", 0, 1, 2, 0o100_644, 0)] {
        let entry = row_entry(&row);
        // SAFETY: as in `add_sequences_match`.
        assert_eq!(unsafe { ffi::git_index_add(c.0, &entry.into_raw()) }, 0);
        rust.as_mut().add(entry.as_ref()).unwrap();
    }
    let side = |path, byte, mode| Some(entry(path, 0, 1, byte, mode, 0));
    let steps = [
        [
            side(c"f", 1, 0o100_644),
            side(c"f", 2, 0o100_644),
            side(c"f", 3, 0o100_755),
        ],
        [None, side(c"f", 4, 0o120_000), None],
        [side(c"g", 5, 0o100_644), None, side(c"h", 6, 0o100_644)],
        [None, None, None],
        [side(c"i", 7, 0o100_644), side(c"i", 8, 0o040_000), None],
        [
            side(c"j", 7, 0o100_644),
            side(c".git/j", 8, 0o100_644),
            None,
        ],
        [
            side(c"k", 9, 0o160_000),
            side(c"k", 9, 0o160_000),
            side(c"k", 9, 0o160_000),
        ],
        [None, side(c"f/x", 1, 0o100_644), side(c"f/x", 2, 0o100_644)],
        [side(c"", 1, 0o100_644), None, None],
    ];
    for (n, sides) in steps.iter().enumerate() {
        let lent = sides.each_ref().map(|s| s.as_ref().map(|e| e.as_ref()));
        let raw = sides.map(|s| s.map(IndexEntry::into_raw));
        let [a, o, t] = raw
            .each_ref()
            .map(|s| s.as_ref().map_or(ptr::null(), ptr::from_ref));
        clear();
        // SAFETY: a live index lending no entry; each side NULL or a live entry.
        let expected = c_status(unsafe { ffi::git_index_conflict_add(c.0, a, o, t) });
        clear();
        let actual = rust_status(rust.as_mut().add_conflict(lent[0], lent[1], lent[2]));
        assert_eq!(expected, actual, "step {n}");
        assert_same(c.0, &rust.as_mut(), &format!("step {n}"));
    }
}

/// `entry_stage` and `entry_is_conflict` of every flags value.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn stage_bits_match() {
    for bits in 0..=u16::MAX {
        let entry = entry(c"a", 0, 1, 0, 0o100_644, 0);
        let mut entry = entry;
        entry
            .as_mut()
            .set_flags(EntryFlags::from_bits_retain(bits.into()))
            .unwrap();
        let raw = entry.into_raw();
        let ptr = &raw const raw;
        // SAFETY: a live entry, only read.
        let index_entry_stage = unsafe { ffi::git_index_entry_stage(ptr) };
        // SAFETY: as above.
        let (stage, conflict) =
            unsafe { (index_entry_stage, ffi::git_index_entry_is_conflict(ptr)) };
        assert_eq!(entry.as_ref().stage().raw(), stage, "{bits:#x}");
        assert_eq!(entry.as_ref().is_conflict(), conflict != 0, "{bits:#x}");
    }
}

/// The same conflicts and entries, `(a, o, t)` sides on each of `paths`.
fn seed_conflicts(c: *mut ffi::git_index, rust: &mut IndexBox) {
    let resolved = entry(c"aa-resolved", 0, 1, 1, 0o100_644, 0);
    // SAFETY: as in `add_sequences_match`.
    assert_eq!(unsafe { ffi::git_index_add(c, &resolved.into_raw()) }, 0);
    rust.as_mut().add(resolved.as_ref()).unwrap();
    for (path, mask) in [
        (c"a", 7u8),
        (c"b", 1),
        (c"b/c", 6),
        (c"c", 2),
        (c"d", 4),
        (c"e", 5),
    ] {
        let sides =
            [1u8, 2, 4].map(|bit| (mask & bit != 0).then(|| entry(path, 0, 1, bit, 0o100_644, 0)));
        let lent = sides.each_ref().map(|s| s.as_ref().map(|e| e.as_ref()));
        let raw = sides.map(|s| s.map(IndexEntry::into_raw));
        let [a, o, t] = raw
            .each_ref()
            .map(|s| s.as_ref().map_or(ptr::null(), ptr::from_ref));
        // SAFETY: as in `conflict_add_sequences_match`.
        assert_eq!(unsafe { ffi::git_index_conflict_add(c, a, o, t) }, 0);
        rust.as_mut()
            .add_conflict(lent[0], lent[1], lent[2])
            .unwrap();
    }
    let normal = entry(c"b", 0, 1, 9, 0o100_644, 0);
    // SAFETY: as above.
    assert_eq!(unsafe { ffi::git_index_add(c, &normal.into_raw()) }, 0);
    rust.as_mut().add(normal.as_ref()).unwrap();
}

/// Iterating conflicts yields the same sides, step by step, then stays done.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn conflict_iteration_matches() {
    init();
    for seeded in [false, true] {
        let c = c_open(None).1.unwrap();
        let mut rust = Index::new().unwrap();
        if seeded {
            seed_conflicts(c.0, &mut rust);
        }
        let mut raw = ptr::null_mut();
        // SAFETY: a writable slot and a live index, unchanged until the cursor is freed.
        let rc = unsafe { ffi::git_index_conflict_iterator_new(&raw mut raw, c.0) };
        assert_eq!(rc, 0);
        let mut cursor = ConflictIterator::new(rust.as_mut()).unwrap();
        let mut steps = cursor.as_mut();
        for step in 0..9 {
            let mut slots = [ptr::null(); 3];
            let [a, o, t] = &mut slots;
            // SAFETY: a live cursor over its unchanged index; three writable slots.
            let rc = unsafe { ffi::git_index_conflict_next(a, o, t, raw) };
            let expected = (rc == 0).then(|| slots.map(c_fields));
            let actual = steps.next().map(|conflict| {
                [conflict.ancestor, conflict.ours, conflict.theirs].map(|s| s.map(rust_fields))
            });
            assert_eq!(expected, actual, "{seeded} {step}");
            assert!(rc == 0 || rc == ffi::GIT_ITEROVER);
        }
        // SAFETY: the cursor allocated above, freed once.
        unsafe { ffi::git_index_conflict_iterator_free(raw) };
        drop(cursor);
        assert_same(c.0, &rust.as_mut(), "after iteration");
    }
}

/// Iterating a snapshot yields the same entries, step by step, whatever the index does
/// meanwhile (entries added, replaced in place and removed), then stays done.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn snapshot_iteration_matches() {
    init();
    let c = c_open(None).1.unwrap();
    let mut rust = Index::new().unwrap();
    seed_conflicts(c.0, &mut rust);
    let rust: IndexArc = rust.into();
    let mut raw = ptr::null_mut();
    // SAFETY: a writable slot and a live index; C counts a reference of its own.
    assert_eq!(unsafe { ffi::git_index_iterator_new(&raw mut raw, c.0) }, 0);
    let mut iterator = IndexIterator::new(&rust).unwrap();
    let c_next = || {
        let mut out = ptr::null();
        // SAFETY: a live iterator; `out` is written only on success, and copied at once.
        let rc = unsafe { ffi::git_index_iterator_next(&raw mut out, raw) };
        (rc == 0).then(|| c_fields(out).unwrap())
    };
    for step in 0..14 {
        let expected = c_next();
        let actual = iterator.as_mut().next().map(|e| rust_fields(e.as_ref()));
        assert_eq!(expected, actual, "{step}");
        if step == 2 {
            // The same changes to both indexes, behind the snapshots.
            for index in [c.0, rust.as_c_ptr()] {
                let replaced = entry(c"aa-resolved", 0, 1, 0x55, 0o100_755, 0);
                let added = entry(c"zz", 0, 1, 1, 0o100_644, 0);
                // SAFETY: a live index whose entries no borrow reads (the snapshots
                // copy or own theirs); C copies each entry and frees removed ones late.
                unsafe { assert_eq!(ffi::git_index_add(index, &replaced.into_raw()), 0) };
                // SAFETY: as above.
                unsafe { assert_eq!(ffi::git_index_add(index, &added.into_raw()), 0) };
                // SAFETY: as above.
                unsafe { assert_eq!(ffi::git_index_remove(index, c"e".as_ptr(), 3), 0) };
            }
        }
    }
    // SAFETY: the iterator allocated above, freed once.
    unsafe { ffi::git_index_iterator_free(raw) };
}

/// A non-bare repository, with the index C and Rust each own a reference to.
struct Repo {
    _dir: Scratch,
    repo: Option<RepositoryBox>,
}

impl Repo {
    fn new(tag: &str) -> Self {
        let dir = Scratch::new(tag);
        let repo = Some(Repository::init(&c_path(&dir.0), false).unwrap());
        Self { _dir: dir, repo }
    }

    fn repo(&self) -> RepositoryRef<'_> {
        self.repo.as_ref().unwrap().as_ref()
    }

    /// One more counted reference to the repository's index, raw.
    fn c_index(&self) -> CIndex {
        let mut out = ptr::null_mut();
        let repo = self.repo.as_ref().unwrap().as_c_ptr();
        // SAFETY: a writable slot and a live repository.
        let rc = unsafe { ffi::git_repository_index(&raw mut out, repo) };
        assert_eq!(rc, 0);
        CIndex(out)
    }

    /// The same, adopted as a sole owner, for the exclusive handle no safe wrapper hands
    /// out for a repository's index; sound while the test reaches neither the repository
    /// nor another reference to its index as long as it uses the box.
    fn index_box(&self) -> IndexBox {
        let raw = self.c_index();
        let ptr = raw.0;
        let _ = core::mem::ManuallyDrop::new(raw);
        // SAFETY: a counted reference, released once by `git_index_free`; see above.
        unsafe { IndexBox::from_c(ptr) }.unwrap()
    }

    /// A tree of `files` (paths at most one directory deep), each blob holding its path.
    fn tree(&self, files: &[&str]) -> TreeArc<'_> {
        let repo = self.repo();
        let blob = |data: &str| blob::create_from_buffer(repo, data.as_bytes()).unwrap();
        let mut root = TreeBuilder::new(repo).unwrap();
        let mut dirs: Vec<(&str, TreeBuilderBox<'_>)> = Vec::new();
        for file in files {
            let (builder, name) = match file.split_once('/') {
                Some((dir, name)) => {
                    if !dirs.iter().any(|(d, _)| *d == dir) {
                        dirs.push((dir, TreeBuilder::new(repo).unwrap()));
                    }
                    (
                        &mut dirs.iter_mut().find(|(d, _)| *d == dir).unwrap().1,
                        name,
                    )
                }
                None => (&mut root, *file),
            };
            let name = CString::new(name).unwrap();
            builder
                .as_mut()
                .insert(&name, blob(file).as_ref(), FileMode::Blob)
                .unwrap();
        }
        for (dir, mut builder) in dirs {
            let id = builder.as_mut().write().unwrap();
            let dir = CString::new(dir).unwrap();
            root.as_mut()
                .insert(&dir, id.as_ref(), FileMode::Tree)
                .unwrap();
        }
        let id = root.as_mut().write().unwrap();
        Tree::lookup(repo, id.as_ref()).unwrap()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        drop(self.repo.take());
    }
}

/// Adding from buffers, without an owner and with one, agrees step by step: blobs
/// written alike, conflicts resolved alike, modes and sizes refused alike. Every entry
/// is of stage 0: libgit2 frees an entry of another stage while adding it, which
/// `add_from_buffer` refuses (a unit test holds that regression).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
#[expect(
    clippy::used_underscore_binding,
    reason = "the binding is held for its drop, and read here on purpose"
)]
fn add_from_buffer_sequences_match() {
    init();
    let c = c_open(None).1.unwrap();
    let mut rust = Index::new().unwrap();
    let file = entry(c"f", 0, 1, 1, 0o100_644, 0);
    clear();
    // SAFETY: a live index; `file` and the bytes are read for the call.
    let expected = c_status(unsafe {
        ffi::git_index_add_from_buffer(c.0, &file.into_raw(), b"x".as_ptr().cast(), 1)
    });
    clear();
    let actual = rust_status(rust.as_mut().add_from_buffer(file.as_ref(), b"x"));
    assert_eq!(expected, actual);

    let (c_repo, rust_repo) = (Repo::new("entries-buffer-c"), Repo::new("entries-buffer-r"));
    let c = c_repo.c_index();
    let mut rust = rust_repo.index_box();
    // Conflict sides naming objects both repositories have, of the same ids.
    let sides = [1u8, 2, 3].map(|n| {
        let [c_id, rust_id] =
            [&c_repo, &rust_repo].map(|repo| blob::create_from_buffer(repo.repo(), &[n]).unwrap());
        assert_eq!(c_id.as_ref().id(), rust_id.as_ref().id());
        let mut side = entry(c"f", 0, 1, n, 0o100_644, 0);
        side.as_mut().set_id(rust_id.as_ref());
        side
    });
    let lent: Vec<_> = sides.iter().map(|s| s.as_ref()).collect();
    let raw: Vec<_> = sides.iter().map(|s| s.into_raw()).collect();
    // SAFETY: a live index; each side a live entry naming an object the repository has.
    let rc = unsafe {
        ffi::git_index_conflict_add(c.0, &raw const raw[0], &raw const raw[1], &raw const raw[2])
    };
    assert_eq!(rc, 0);
    rust.as_mut()
        .add_conflict(Some(lent[0]), Some(lent[1]), Some(lent[2]))
        .unwrap();
    assert_same(c.0, &rust.as_mut(), "conflicted");
    let big = vec![7u8; 70_000];
    let steps: &[(Row<'_>, &[u8])] = &[
        ((c"f", 0, 1, 0, 0o100_644, 0), b"resolved"),
        ((c"f", 0, 1, 0, 0o100_755, 0), b"again"),
        ((c"g", 0, 2, 9, 0o120_000, 0x2000), b"target"),
        ((c"h", 0, 1, 0, 0o100_644, 0), b""),
        ((c"h", 0, 1, 0, 0o100_644, 0), &big),
        ((c"i", 0, 1, 0, 0o160_000, 0), b"x"),
        ((c"i", 0, 1, 0, 0o040_000, 0), b"x"),
        ((c".git/i", 0, 1, 0, 0o100_644, 0), b"x"),
        ((c"", 0, 1, 0, 0o100_644, 0), b"x"),
        ((c"g/h", 0, 1, 0, 0o100_644, 0), b"x"),
    ];
    for (row, data) in steps {
        let entry = row_entry(row);
        clear();
        // SAFETY: a live index lending no entry; the entry and bytes are read for the
        // call.
        let expected = c_status(unsafe {
            ffi::git_index_add_from_buffer(c.0, &entry.into_raw(), data.as_ptr().cast(), data.len())
        });
        clear();
        let actual = rust_status(rust.as_mut().add_from_buffer(entry.as_ref(), data));
        assert_eq!(expected, actual, "{row:?}");
        assert_same(c.0, &rust.as_mut(), &format!("{row:?}"));
    }
    // A blob neither can write: its loose object's directory is a file.
    let id = blob::create_from_buffer(Repo::new("entries-buffer-id").repo(), b"stuck").unwrap();
    let hex = id.as_ref().to_hex_string().unwrap();
    for repo in [&c_repo, &rust_repo] {
        std::fs::write(repo._dir.0.join(".git/objects").join(&hex[..2]), b"").unwrap();
    }
    let stuck = entry(c"stuck", 0, 1, 0, 0o100_644, 0);
    clear();
    // SAFETY: as above.
    let expected = c_status(unsafe {
        ffi::git_index_add_from_buffer(c.0, &stuck.into_raw(), b"stuck".as_ptr().cast(), 5)
    });
    clear();
    let actual = rust_status(rust.as_mut().add_from_buffer(stuck.as_ref(), b"stuck"));
    let code_and_class = |s: &Status| (s.0, s.1.as_ref().map(|e| e.0));
    assert_eq!(code_and_class(&expected), code_and_class(&actual));
    assert!(expected.0 < 0);
    assert_same(c.0, &rust.as_mut(), "stuck");
    drop(rust);
    drop(c);
}

/// A case's name, its two indexes (C's, Rust's), and the mode and data of its entries.
type BufferCase<'a> = (&'a str, &'a dyn Fn() -> (CIndex, IndexBox), u32, &'a [u8]);

/// Entries of a conflict stage fail alike wherever libgit2 refuses them before adding
/// them, safely: without an owner, with a mode other than a file's or a link's, and with
/// more than `u32::MAX` bytes. `add_from_buffer` refuses only the entries libgit2 would
/// add (and free), keeping libgit2's errors for these (`equiv/add-from-buffer-stage-
/// refusal-replaces-c-errors`).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn conflict_stage_buffers_fail_as_c_before_adding() {
    init();
    let big = vec![0u8; u32::MAX as usize + 1];
    let unowned = || (c_open(None).1.unwrap(), Index::new().unwrap());
    let (c_repo, rust_repo) = (Repo::new("entries-stage-c"), Repo::new("entries-stage-r"));
    let owned = || (c_repo.c_index(), rust_repo.index_box());
    let cases: &[BufferCase<'_>] = &[
        ("unowned file", &unowned, 0o100_644, b"x"),
        ("unowned link", &unowned, 0o120_000, b"x"),
        ("unowned tree", &unowned, 0o040_000, b"x"),
        ("unowned oversized", &unowned, 0o100_644, &big),
        ("owned submodule", &owned, 0o160_000, b"x"),
        ("owned tree", &owned, 0o040_000, b"x"),
        ("owned oversized", &owned, 0o100_755, &big),
    ];
    for (case, indexes, mode, data) in cases {
        let (c, mut rust) = indexes();
        for stage in 1..=3 {
            let side = entry(c"f", stage, 1, 0, *mode, 0);
            clear();
            // SAFETY: a live index lending no entry; the entry and the bytes are read
            // for the call, which fails before libgit2 adds anything.
            let expected = c_status(unsafe {
                ffi::git_index_add_from_buffer(
                    c.0,
                    &side.into_raw(),
                    data.as_ptr().cast(),
                    data.len(),
                )
            });
            clear();
            let actual = rust_status(rust.as_mut().add_from_buffer(side.as_ref(), data));
            assert!(expected.0 < 0, "{case}, stage {stage}");
            assert_eq!(expected, actual, "{case}, stage {stage}");
            assert_same(c.0, &rust.as_mut(), &format!("{case}, stage {stage}"));
        }
        drop(rust);
        drop(c);
    }
}

/// On an index a repository owns, `add` and `add_conflict` look ids up as C does: a
/// missing object fails alike, and a partial conflict stays alike.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owned_additions_match() {
    init();
    let (c_repo, rust_repo) = (Repo::new("entries-owned-c"), Repo::new("entries-owned-r"));
    let c = c_repo.c_index();
    let mut rust = rust_repo.index_box();
    let mut present = entry(c"p", 0, 1, 0, 0o100_644, 0);
    for repo in [&c_repo, &rust_repo] {
        let id = blob::create_from_buffer(repo.repo(), b"present").unwrap();
        present.as_mut().set_id(id.as_ref());
    }
    let missing = entry(c"p", 0, 1, 0xEE, 0o100_644, 0);
    let submodule = entry(c"s", 0, 1, 0xEE, 0o160_000, 0);
    for step in 0..4 {
        let (a, o, t) = match step {
            0 => (Some(present.as_ref()), Some(missing.as_ref()), None),
            1 => (None, Some(present.as_ref()), Some(present.as_ref())),
            2 => (Some(submodule.as_ref()), None, None),
            _ => (None, None, Some(missing.as_ref())),
        };
        let copies = [a, o, t].map(|s| s.map(|e| e.to_index_entry().into_raw()));
        let [ra, ro, rt] = copies
            .each_ref()
            .map(|s| s.as_ref().map_or(ptr::null(), ptr::from_ref));
        clear();
        // SAFETY: a live index lending no entry; each side NULL or a live entry.
        let expected = c_status(unsafe { ffi::git_index_conflict_add(c.0, ra, ro, rt) });
        clear();
        let actual = rust_status(rust.as_mut().add_conflict(a, o, t));
        // C leaves a stale error behind a success (the odb's packed lookup).
        assert_eq!(expected.0, actual.0, "conflict {step}");
        if expected.0 != 0 {
            assert_eq!(expected, actual, "conflict {step}");
        }
        assert_same(c.0, &rust.as_mut(), &format!("conflict {step}"));
    }
    for entry in [&present, &missing, &submodule] {
        clear();
        // SAFETY: as above.
        let expected = c_status(unsafe { ffi::git_index_add(c.0, &entry.into_raw()) });
        clear();
        let actual = rust_status(rust.as_mut().add(entry.as_ref()));
        assert_eq!(expected.0, actual.0);
        if expected.0 != 0 {
            assert_eq!(expected, actual);
        }
        assert_same(c.0, &rust.as_mut(), "add");
    }
    drop(rust);
    drop(c);
}

/// Reading trees agrees step by step: over a populated index whose matching entries
/// keep their metadata, over conflicts and records, a tree whose subtree is gone, and
/// trees of another object id type.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
#[expect(
    clippy::used_underscore_binding,
    reason = "the binding is held for its drop, and read here on purpose"
)]
fn read_tree_sequences_match() {
    init();
    let (c_repo, rust_repo) = (Repo::new("entries-tree-c"), Repo::new("entries-tree-r"));
    let c = c_open(None).1.unwrap();
    let mut rust = Index::new().unwrap();
    let shapes: [&[&str]; 4] = [
        &["a", "d/x", "d/y"],
        &["a", "b", "d/x", "e/z"],
        &[],
        &["a", "b", "d/x", "e/z"],
    ];
    let seed = [
        entry(c"c", 2, 1, 1, 0o100_644, 0),
        entry(c"a", 0, 1, 1, 0o100_644, 0),
    ];
    for row in &seed {
        // SAFETY: as in `add_sequences_match`.
        assert_eq!(unsafe { ffi::git_index_add(c.0, &row.into_raw()) }, 0);
        rust.as_mut().add(row.as_ref()).unwrap();
    }
    for (n, files) in shapes.iter().enumerate() {
        let (c_tree, rust_tree) = (c_repo.tree(files), rust_repo.tree(files));
        clear();
        // SAFETY: a live index lending no entry, and a live tree.
        let expected = c_status(unsafe { ffi::git_index_read_tree(c.0, c_tree.as_c_ptr()) });
        clear();
        let actual = rust_status(rust.as_mut().read_tree(rust_tree.as_ref()));
        assert_eq!(expected, actual, "tree {n}");
        assert_same(c.0, &rust.as_mut(), &format!("tree {n}"));
        // New metadata on the first entry, kept by the next read when it matches.
        if n == 0 {
            for index in [c.0, rust.as_c_ptr()] {
                {
                    // SAFETY: a live index lending no entry.
                    let entry = unsafe { ffi::git_index_get_byindex(index, 0) };
                    // SAFETY: as above; the entry is copied, changed and added back before
                    // any other call.
                    let mut first = unsafe { *entry };
                    first.file_size = 77;
                    first.ino = 5;
                    // SAFETY: as above.
                    unsafe { assert_eq!(ffi::git_index_add(index, &raw const first), 0) };
                }
            }
        }
    }
    // A tree whose subtree object is gone.
    let broken = [&c_repo, &rust_repo].map(|repo| {
        let tree = repo.tree(&["q/r"]);
        let sub = tree.as_ref().get_by_name(c"q").unwrap().id().to_oid();
        let hex = sub.as_ref().to_hex_string().unwrap();
        let objects = repo._dir.0.join(".git/objects");
        std::fs::remove_file(objects.join(&hex[..2]).join(&hex[2..])).unwrap();
        tree
    });
    clear();
    // SAFETY: as above.
    let expected = c_status(unsafe { ffi::git_index_read_tree(c.0, broken[0].as_c_ptr()) });
    clear();
    let actual = rust_status(rust.as_mut().read_tree(broken[1].as_ref()));
    assert_eq!(expected, actual, "broken");
    assert_same(c.0, &rust.as_mut(), "broken");
    drop(broken);
    // A SHA-256 repository's trees, into these SHA-1 indexes and SHA-256 ones.
    let sha256 = [0, 1].map(|n| {
        let dir = Scratch::new(&format!("entries-tree-256-{n}"));
        let mut opts = InitOptions::new();
        opts.as_mut().set_oid_type(Some(OidType::Sha256));
        let repo = Repository::init_ext(&c_path(&dir.0), opts.as_mut()).unwrap();
        Repo {
            _dir: dir,
            repo: Some(repo),
        }
    });
    let trees = [sha256[0].tree(&["a", "k/l"]), sha256[1].tree(&["a", "k/l"])];
    let c256 = {
        let opts = libgit2::libgit2::index::index::IndexOptions::with_oid_type(OidType::Sha256);
        let mut out = ptr::null_mut();
        let raw = opts.into_raw();
        // SAFETY: a writable slot and live options of this version, read for the call.
        let rc = unsafe { ffi::git_index_new_ext(&raw mut out, &raw const raw) };
        assert_eq!(rc, 0);
        CIndex(out)
    };
    let mut rust256 = Index::with_oid_type(OidType::Sha256).unwrap();
    for (c_index, rust_index) in [(c.0, &mut rust), (c256.0, &mut rust256)] {
        clear();
        // SAFETY: as above.
        let expected = c_status(unsafe { ffi::git_index_read_tree(c_index, trees[0].as_c_ptr()) });
        clear();
        let actual = rust_status(rust_index.as_mut().read_tree(trees[1].as_ref()));
        assert_eq!(expected, actual, "sha256");
        assert_same(c_index, &rust_index.as_mut(), "sha256");
    }
}

/// Reading a tree with an entry whose name an index cannot hold (`.git`) fails alike,
/// leaving both indexes as they were. libgit2 leaks the entry's joined path
/// (`read_tree_cb`) on either side, so the test runs in a child with `LeakSanitizer` off.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn refused_tree_entry_names_match() {
    isolated_tolerating_leaks(
        concat!(module_path!(), "::refused_tree_entry_names_match"),
        || {
            init();
            let (c_repo, rust_repo) = (Repo::new("entries-names-c"), Repo::new("entries-names-r"));
            let c = c_open(None).1.unwrap();
            let mut rust = Index::new().unwrap();
            let (c_tree, rust_tree) = (c_repo.tree(&["a", "d/x"]), rust_repo.tree(&["a", "d/x"]));
            // SAFETY: a live index lending no entry, and a live tree.
            let rc = unsafe { ffi::git_index_read_tree(c.0, c_tree.as_c_ptr()) };
            assert_eq!(rc, 0);
            rust.as_mut().read_tree(rust_tree.as_ref()).unwrap();
            // A tree entry named `.git`, which an index cannot hold, after one it can.
            let dot = [&c_repo, &rust_repo].map(|repo| {
                let id = blob::create_from_buffer(repo.repo(), b"dot").unwrap();
                let mut raw = Vec::new();
                for name in [&b".git"[..], b"ok"] {
                    raw.extend_from_slice(b"100644 ");
                    raw.extend_from_slice(name);
                    raw.push(0);
                    raw.extend_from_slice(&id.as_ref().id()[..20]);
                }
                let odb = repo.repo().odb().unwrap();
                let mut tree = ffi::git_oid::default();
                // SAFETY: a live odb, a writable id and `raw` live for its length, read for the
                // call.
                let rc = unsafe {
                    ffi::git_odb_write(
                        &raw mut tree,
                        odb.as_c_ptr(),
                        raw.as_ptr().cast(),
                        raw.len(),
                        ffi::GIT_OBJECT_TREE,
                    )
                };
                assert_eq!(rc, 0);
                let tree = Oid::from_raw(&tree.id[..20], OidType::Sha1).unwrap();
                Tree::lookup(repo.repo(), tree.as_ref()).unwrap()
            });
            clear();
            // SAFETY: as above.
            let expected = c_status(unsafe { ffi::git_index_read_tree(c.0, dot[0].as_c_ptr()) });
            clear();
            let actual = rust_status(rust.as_mut().read_tree(dot[1].as_ref()));
            assert_eq!(expected, actual, "dot");
            assert_eq!(expected.0, -1);
            assert_same(c.0, &rust.as_mut(), "dot");
            drop(dot);
        },
    );
}
