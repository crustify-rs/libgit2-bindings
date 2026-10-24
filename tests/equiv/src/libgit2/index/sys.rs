//! Independent NAME and REUC vectors compared after every add, lookup, lazy sort,
//! replacement, removal and clear.
#[path = "../../../../support/reuc.rs"]
mod reuc;

use std::ffi::{CStr, CString, c_int};
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::index::{Index, IndexBox};
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
use reuc::{IndexFile, Record, Snapshot, snapshot, sorted};

mod reuc_edits;

type Names = [Option<CString>; 3];

fn raw_names(index: &IndexBox) -> Vec<Names> {
    // This test alone accesses the index; C may sort, but each string is copied before
    // any mutation or drop. Null fields mean absent names.
    // SAFETY: as above.
    let count = unsafe { ffi::git_index_name_entrycount(index.as_c_ptr()) };
    (0..count)
        .map(|i| {
            // SAFETY: as above; `i` is below the count, so the entry is live.
            let entry = unsafe { ffi::git_index_name_get_byindex(index.as_c_ptr(), i) };
            // SAFETY: as above, the live entry; the projection forms no reference to it.
            let ancestor = unsafe { &raw const (*entry).ancestor };
            // SAFETY: as above.
            let ours = unsafe { &raw const (*entry).ours };
            // SAFETY: as above.
            let theirs = unsafe { &raw const (*entry).theirs };
            [ancestor, ours, theirs].map(|p| {
                // SAFETY: as above, an initialised field.
                let p = unsafe { p.read() };
                // SAFETY: a non-NULL field is a NUL-terminated string; it is copied.
                (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_owned())
            })
        })
        .collect()
}

fn safe_names(index: &mut IndexBox) -> Vec<Names> {
    (0..index.as_ref().name_entry_count())
        .map(|i| {
            let mut handle = index.as_mut();
            let entry = handle.name_entry(i).unwrap();
            [entry.ancestor(), entry.ours(), entry.theirs()].map(|s| s.map(CStr::to_owned))
        })
        .collect()
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn name_lifecycle_matches_c() {
    let mut safe = Index::new().unwrap();
    let raw = Index::new().unwrap();
    let scenarios = [
        [Some(c"z"), Some(c"our-z"), None],
        [None, Some(c"our-b"), Some(c"their-b")],
        [Some(c"a"), None, Some(c"their-a")],
        [Some(c""), Some(c""), Some(c"\xff")],
        [Some(c"a"), Some(c"duplicate"), Some(c"their-other")],
        [None, Some(c"our-a"), Some(c"their-a")],
    ];
    for _ in 0..2 {
        for names in scenarios {
            // SAFETY: independently owned index; two or three valid NUL-terminated
            // inputs, which C duplicates and never retains.
            let rc = unsafe {
                ffi::git_index_name_add(
                    raw.as_c_ptr(),
                    names[0].map_or(ptr::null(), CStr::as_ptr),
                    names[1].map_or(ptr::null(), CStr::as_ptr),
                    names[2].map_or(ptr::null(), CStr::as_ptr),
                )
            };
            assert_eq!(rc, 0);
            assert!(safe.as_mut().add_name(names[0], names[1], names[2]).is_ok());
            assert_eq!(safe_names(&mut safe), raw_names(&raw));
        }
        for position in [scenarios.len(), usize::MAX] {
            // SAFETY: live index; all positions are checked by C, no borrow retained.
            assert!(unsafe { ffi::git_index_name_get_byindex(raw.as_c_ptr(), position) }.is_null());
            assert!(safe.as_mut().name_entry(position).is_none());
        }
        for _ in 0..2 {
            // SAFETY: this index and its entries have no outstanding borrows.
            assert_eq!(unsafe { ffi::git_index_name_clear(raw.as_c_ptr()) }, 0);
            safe.as_mut().clear_names().unwrap();
            assert_eq!(safe_names(&mut safe), raw_names(&raw));
        }
    }
}

/// Copies one C REUC record out through its raw fields.
///
/// # Safety
/// `entry` is null or a live record of an index nobody mutates during the call.
unsafe fn raw_snapshot(entry: *const ffi::git_index_reuc_entry) -> Option<Snapshot> {
    if entry.is_null() {
        return None;
    }
    {
        // SAFETY: per the contract, a live record: its path is a NUL-terminated string,
        // copied before returning.
        let oid_ptr = unsafe { ptr::addr_of!((*entry).oid) };
        // SAFETY: as above.
        let oids = unsafe { oid_ptr.read() };
        // SAFETY: as above.
        let path_ptr = unsafe { ptr::addr_of!((*entry).path) };
        // SAFETY: as above.
        let read2 = unsafe { path_ptr.read() };
        // SAFETY: as above.
        let cstr = unsafe { CStr::from_ptr(read2) };
        // SAFETY: as above.
        let mode_ptr = unsafe { ptr::addr_of!((*entry).mode) };
        // SAFETY: as above.
        unsafe {
            Some((
                cstr.to_bytes().to_vec(),
                mode_ptr.read(),
                oids.map(|oid| (oid.type_, oid.id)),
            ))
        }
    }
}

fn raw_reuc(index: &IndexBox) -> Vec<Snapshot> {
    // SAFETY: this test alone accesses the index.
    let count = unsafe { ffi::git_index_reuc_entrycount(index.as_c_ptr()) };
    (0..count)
        .map(|i| {
            // SAFETY: as above; `i` is below the count, so the record is live.
            let record = unsafe { ffi::git_index_reuc_get_byindex(index.as_c_ptr(), i) };
            // SAFETY: as above; every record is copied before any mutation or drop.
            unsafe { raw_snapshot(record) }.unwrap()
        })
        .collect()
}

fn safe_reuc(index: &mut IndexBox) -> Vec<Snapshot> {
    let handle = index.as_mut();
    (0..)
        .map_while(|i| handle.reuc_entry(i))
        .map(|entry| {
            let snap = snapshot(entry);
            let sides = [entry.ancestor(), entry.ours(), entry.theirs()];
            for (side, (mode, (kind, id))) in sides.iter().zip(snap.1.iter().zip(snap.2)) {
                assert_eq!(side.map(|s| s.mode), (*mode != 0).then_some(*mode));
                if let Some(side) = side {
                    assert_eq!((side.id.raw_kind(), side.id.id()), (kind, id));
                }
            }
            snap
        })
        .collect()
}

/// Both lookups of `path`, in C and through the wrapper.
fn lookups_match(raw: &IndexBox, safe: &mut IndexBox, path: &CStr) {
    // SAFETY: a live index nobody mutates while the record is copied; `path` is read
    // for the call only.
    let index_reuc_get_bypath =
        unsafe { ffi::git_index_reuc_get_bypath(raw.as_c_ptr(), path.as_ptr()) };
    // SAFETY: as above.
    let expected = unsafe { raw_snapshot(index_reuc_get_bypath) };
    let handle = safe.as_mut();
    assert_eq!(
        handle.reuc_entry_by_path(path).map(snapshot),
        expected,
        "{path:?}"
    );
}

fn positions_match(raw: &IndexBox, safe: &mut IndexBox, position: usize) {
    let index = raw.as_c_ptr();
    // SAFETY: as in `lookups_match`; C checks the position.
    let index_reuc_get_byindex = unsafe { ffi::git_index_reuc_get_byindex(index, position) };
    // SAFETY: as above.
    let expected = unsafe { raw_snapshot(index_reuc_get_byindex) };
    assert_eq!(safe.as_mut().reuc_entry(position).map(snapshot), expected);
}

fn reuc_add(index: &IndexBox, path: &CStr, modes: [c_int; 3], ids: &[Oid; 3]) -> c_int {
    let ids = ids
        .each_ref()
        .map(|id| ptr::from_ref(id).cast::<ffi::git_oid>());
    // SAFETY: a live index without outstanding REUC borrows (the old same-path record
    // is freed); `path` and the transparent `Oid`s are copied for the call.
    unsafe {
        ffi::git_index_reuc_add(
            index.as_c_ptr(),
            path.as_ptr(),
            modes[0],
            ids[0],
            modes[1],
            ids[1],
            modes[2],
            ids[2],
        )
    }
}

fn set_caps(index: &IndexBox, caps: c_int) {
    // SAFETY: a live index without outstanding REUC borrows; C re-sorts its vectors.
    let rc = unsafe { ffi::git_index_set_caps(index.as_c_ptr(), caps) };
    assert_eq!(rc, 0);
}

/// Records read from the same file by two independent indexes, SHA1 and SHA256,
/// including unnamed modes, absent sides, non-UTF-8 paths and case-folded lookups.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reuc_file_records_match_c() {
    let records = [
        Record::new(b"b/file", [0o100_644, 0o100_644, 0o100_755], 1),
        Record::new(b"A", [0, 0o100_644, 0o120_000], 2),
        Record::new(b"a", [0o160_000, 0, 0o40000], 3),
        Record::new(b"\xff", [u32::MAX, 1, 0], 4),
        Record::new(b"c d", [0o100_644, 0, 0], 5),
    ];
    for kind in [OidType::Sha1, OidType::Sha256] {
        let file = IndexFile::new("equiv", &sorted(&records), kind);
        let raw = file.open(kind);
        let mut safe = file.open(kind);
        let expected = raw_reuc(&raw);
        assert_eq!(expected.len(), records.len());
        assert_eq!(safe_reuc(&mut safe), expected);
        for caps in [0, ffi::GIT_INDEX_CAPABILITY_IGNORE_CASE, 0] {
            set_caps(&raw, caps);
            set_caps(&safe, caps);
            assert_eq!(safe_reuc(&mut safe), raw_reuc(&raw));
            for path in [
                c"A", c"a", c"B/FILE", c"b/file", c"\xff", c"c d", c"", c"zz",
            ] {
                lookups_match(&raw, &mut safe, path);
            }
            for position in [0, records.len() - 1, records.len(), usize::MAX] {
                positions_match(&raw, &mut safe, position);
            }
        }
    }
}

/// Records built by the add constructor, compared after every add, replacement,
/// removal and clear, including C's unsigned wrap of negative modes.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reuc_add_sequence_matches_c() {
    let raw = Index::new().unwrap();
    let mut safe = Index::new().unwrap();
    let ids = [7u8, 8, 9].map(|b| Oid::from_raw(&[b; 20], OidType::Sha1).unwrap());
    let steps: [(&CStr, [c_int; 3]); 6] = [
        (c"z", [0o100_644, 0o100_644, 0o100_644]),
        (c"m", [0, 0o100_755, 0]),
        (c"a", [-1, 0, i32::MIN]),
        (c"m", [0o120_000, 0, 0o100_644]),
        (c"", [0, 0, 0]),
        (c"a", [0, 0, 0o100_644]),
    ];
    for _ in 0..2 {
        for (path, modes) in steps {
            assert_eq!(reuc_add(&raw, path, modes, &ids), 0);
            assert_eq!(reuc_add(&safe, path, modes, &ids), 0);
            assert_eq!(safe_reuc(&mut safe), raw_reuc(&raw));
            lookups_match(&raw, &mut safe, path);
            positions_match(&raw, &mut safe, 4);
        }
        // SAFETY: both indexes are live, with no outstanding REUC borrows.
        unsafe { assert_eq!(ffi::git_index_reuc_remove(raw.as_c_ptr(), 1), 0) };
        // SAFETY: as above.
        unsafe { assert_eq!(ffi::git_index_reuc_remove(safe.as_c_ptr(), 1), 0) };
        assert_eq!(safe_reuc(&mut safe), raw_reuc(&raw));
        lookups_match(&raw, &mut safe, c"m");
        // SAFETY: as above.
        unsafe { assert_eq!(ffi::git_index_reuc_clear(raw.as_c_ptr()), 0) };
        // SAFETY: as above.
        unsafe { assert_eq!(ffi::git_index_reuc_clear(safe.as_c_ptr()), 0) };
        assert_eq!(safe_reuc(&mut safe), raw_reuc(&raw));
        assert_eq!(
            safe_reuc(&mut safe),
            Vec::<(Vec<u8>, [u32; 3], [(u8, [u8; 32]); 3])>::new()
        );
        positions_match(&raw, &mut safe, 0);
        lookups_match(&raw, &mut safe, c"z");
    }
}
