//! Public NAME- and REUC-entry borrows, input copying, lazy sort, clear, replacement
//! and parent release.
#![forbid(unsafe_code)]

#[path = "../support/reuc.rs"]
mod reuc;

use crate::fixture::index::{add_reuc, clear_reuc, remove_reuc, set_ignore_case};
use libgit2::libgit2::index::index::Index;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::util::errors::ErrorClass;
use reuc::{IndexFile, Record, extension_bytes, snapshot, sorted};
use std::ffi::CString;

mod reuc_edits;

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn names_survive_inputs_and_reborrow_before_clear() {
    let mut index = Index::new().unwrap();
    for mask in [3, 5, 6, 7] {
        index.as_mut().clear_names().unwrap();
        for i in (0..32).rev() {
            let inputs = [
                CString::new(format!("ancestor-{i:02}")).unwrap(),
                CString::new(format!("ours-{i:02}")).unwrap(),
                CString::new([b't', 0xff, i + 1]).unwrap(),
            ];
            index
                .as_mut()
                .add_name(
                    (mask & 1 != 0).then_some(inputs[0].as_c_str()),
                    (mask & 2 != 0).then_some(inputs[1].as_c_str()),
                    (mask & 4 != 0).then_some(inputs[2].as_c_str()),
                )
                .unwrap();
        }
        assert_eq!(index.as_ref().name_entry_count(), 32);
        let mut handle = index.as_mut();
        for i in 0..32 {
            let mut reborrow = handle.as_mut();
            let entry = reborrow.name_entry(i).unwrap();
            let copy = entry;
            assert_eq!(entry.ancestor().is_some(), mask & 1 != 0);
            assert_eq!(entry.ours().is_some(), mask & 2 != 0);
            assert_eq!(entry.theirs().is_some(), mask & 4 != 0);
            assert_eq!(copy.ancestor(), entry.ancestor());
            if let Some(name) = entry.ancestor() {
                assert_eq!(name.to_bytes(), format!("ancestor-{i:02}").as_bytes());
            }
        }
        assert!(handle.name_entry(32).is_none());
        assert!(handle.name_entry(usize::MAX).is_none());
        handle.clear_names().unwrap();
        handle.clear_names().unwrap();
        assert!(handle.name_entry(0).is_none());
    }
    index
        .as_mut()
        .add_name(Some(c""), Some(c"\xff"), None)
        .unwrap();
    let saved = {
        let mut handle = index.as_mut();
        let entry = handle.name_entry(0).unwrap();
        assert_eq!(entry.ancestor(), Some(c""));
        entry.ours().unwrap().to_owned()
    };
    drop(index);
    assert_eq!(saved.as_c_str(), c"\xff");
}

/// The reader can produce all-null and single-name records that name_add cannot.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn names_from_file_support_all_nullable_fields() {
    let path = std::env::temp_dir().join(format!("crustify-index-b4-{}.index", std::process::id()));
    let cpath = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    for mask in 0..8 {
        let mut names = Vec::new();
        for bit in 0..3 {
            if mask & (1 << bit) != 0 {
                names.push(b'a' + bit);
            }
            names.push(0);
        }
        let mut bytes = b"DIRC\0\0\0\x02\0\0\0\0NAME".to_vec();
        bytes.extend_from_slice(&(names.len() as u32).to_be_bytes());
        bytes.extend(names);
        bytes.extend([0; 20]); // libgit2 accepts an omitted (all-zero) checksum.
        std::fs::write(&path, bytes).unwrap();
        let mut index = Index::open(&cpath).unwrap();
        let mut handle = index.as_mut();
        let entry = handle.name_entry(0).unwrap();
        for (bit, name) in [entry.ancestor(), entry.ours(), entry.theirs()]
            .iter()
            .enumerate()
        {
            assert_eq!(name.is_some(), mask & (1 << bit) != 0);
            if let Some(name) = name {
                assert_eq!(name.to_bytes(), &[b'a' + bit as u8]);
            }
        }
        // Force a sort of parser-produced records, including those with neither
        // ancestor nor ours (which the add constructor does not itself produce).
        handle
            .add_name(None, Some(c"extra"), Some(c"extra"))
            .unwrap();
        assert!(handle.name_entry(0).is_some());
        assert!(handle.name_entry(1).is_some());
    }
    std::fs::remove_file(path).unwrap();
}

/// Records read from a file: many held at once through one shared index handle,
/// looked up by position and path, then copied past the index's release.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reuc_entries_from_file_borrow_index() {
    for kind in [OidType::Sha1, OidType::Sha256] {
        let mut records: Vec<Record> = (0..48u8)
            .map(|i| {
                let modes = match i % 4 {
                    0 => [0o100644, 0o100644, 0o100755],
                    1 => [0, 0o100644, 0o120000],
                    2 => [0o160000, 0, 0o40000],
                    _ => [u32::MAX, 0o100644, 0],
                };
                Record::new(format!("dir/{i:02}-\u{e9}").as_bytes(), modes, i)
            })
            .collect();
        records.push(Record::new(b"\xff\x01", [1, 2, 3], 99));
        let file = IndexFile::new("borrow", &records, kind);
        let records = sorted(&records);
        let mut index = file.open(kind);
        let handle = index.as_mut();
        let entries: Vec<_> = (0..records.len())
            .map(|i| handle.reuc_entry(i).unwrap())
            .collect();
        assert!(handle.reuc_entry(records.len()).is_none());
        assert!(handle.reuc_entry(usize::MAX).is_none());
        for (entry, record) in entries.iter().zip(&records) {
            let (path, modes, ids) = snapshot(*entry);
            assert_eq!((&path, modes), (&record.path, record.modes));
            for (side, (mode, (kind_byte, id))) in modes.iter().zip(ids).enumerate() {
                if *mode == 0 {
                    assert_eq!((kind_byte, id), (0, [0; 32]));
                } else {
                    assert_eq!(kind_byte, kind.raw() as u8);
                    assert_eq!(id[..kind.size()], record.ids[side][..kind.size()]);
                }
            }
            let by_path = handle
                .reuc_entry_by_path(&CString::new(path).unwrap())
                .unwrap();
            assert_eq!(snapshot(by_path), snapshot(*entry));
            assert_eq!(
                [entry.ancestor(), entry.ours(), entry.theirs()].map(|s| s.map(|s| s.mode)),
                record.modes.map(|m| (m != 0).then_some(m))
            );
        }
        assert!(handle.reuc_entry_by_path(c"dir").is_none());
        assert!(handle.reuc_entry_by_path(c"").is_none());
        assert!(handle.reuc_entry_by_path(c"DIR/00-\xc3\xa9").is_none());
        let kept = entries[0].path().to_owned();
        let id = entries[1].ours().unwrap().id.to_oid();
        drop(entries);
        drop(index);
        assert_eq!(kept.to_bytes(), records[0].path.as_slice());
        assert_eq!(
            id.as_ref().id()[..kind.size()],
            records[1].ids[1][..kind.size()]
        );
    }
}

/// Records the add constructor builds: replaced in place by a same-path add, removed
/// and cleared, with every borrow ended before each mutation.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reuc_entries_survive_until_replacement_and_clear() {
    let mut index = Index::new().unwrap();
    let ids = [1u8, 2, 3].map(|b| Oid::from_raw(&[b; 20], OidType::Sha1).unwrap());
    let refs = [&ids[0], &ids[1], &ids[2]];
    for round in 0..3 {
        for i in (0..24).rev() {
            let path = CString::new(format!("p{i:02}")).unwrap();
            assert_eq!(add_reuc(&mut index, &path, [0o100644, -1, 0], refs), 0);
        }
        let handle = index.as_mut();
        let first = handle.reuc_entry(0).unwrap();
        let last = handle.reuc_entry_by_path(c"p23").unwrap();
        assert_eq!(first.path(), c"p00");
        assert_eq!(first.modes(), [0o100644, u32::MAX, 0]);
        assert_eq!(last.ours().unwrap().id.id()[..20], [2; 20]);
        assert!(last.theirs().is_none());
        // A same-path add frees the old record; the borrow has ended first.
        let path = first.path().to_owned();
        assert_eq!(add_reuc(&mut index, &path, [0, 0, 0o100755], refs), 0);
        let handle = index.as_mut();
        let replaced = handle.reuc_entry(0).unwrap();
        assert_eq!(replaced.path(), c"p00");
        assert!(replaced.ancestor().is_none() && replaced.ours().is_none());
        assert_eq!(replaced.theirs().unwrap().id.id()[..20], [3; 20]);
        assert_eq!(remove_reuc(&mut index, 0), 0);
        assert_eq!(index.as_mut().reuc_entry(0).unwrap().path(), c"p01");
        if round == 1 {
            break;
        }
        assert_eq!(clear_reuc(&mut index), 0);
        assert!(index.as_mut().reuc_entry(0).is_none());
    }
}

/// Records lent by the exclusive handle's `&self` stay borrowed while that handle writes
/// the index through its shared reborrow, case-sensitive and ignore-case: writing sorts
/// the REUC vector (and copies the entries into a case-sensitive order) but frees no
/// record, so every held record still reads as it did.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reuc_records_outlive_writes_through_the_shared_reborrow() {
    let dir = std::env::temp_dir().join(format!("crustify-reuc-write-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = CString::new(dir.join("index").to_str().unwrap()).unwrap();
    let ids = [1u8, 2, 3].map(|b| Oid::from_raw(&[b; 20], OidType::Sha1).unwrap());
    let refs = [&ids[0], &ids[1], &ids[2]];
    for ignore_case in [false, true] {
        let mut index = Index::open(&file).unwrap();
        assert_eq!(set_ignore_case(&mut index, ignore_case), 0);
        for i in (0..12).rev() {
            let path = CString::new(format!("{}{i:02}", ["p", "P"][i % 2])).unwrap();
            assert_eq!(
                add_reuc(&mut index, &path, [0o100644, 0, 0o100755], refs),
                0
            );
        }
        let handle = index.as_mut();
        let held: Vec<_> = (0..12).map(|i| handle.reuc_entry(i).unwrap()).collect();
        let found = handle.reuc_entry_by_path(c"p04").unwrap();
        let before: Vec<_> = held.iter().map(|r| r.path().to_owned()).collect();
        for _ in 0..2 {
            handle.as_ref().write().unwrap();
        }
        for (record, path) in held.iter().zip(&before) {
            assert_eq!(record.path(), path.as_c_str());
            assert_eq!(record.modes(), [0o100644, 0, 0o100755]);
            assert_eq!(record.theirs().unwrap().id.id()[..20], [3; 20]);
        }
        assert_eq!(found.path(), c"p04");
        assert_eq!(found.ancestor().unwrap().id.id()[..20], [1; 20]);
        drop(index);
        // The file keeps the writer's order, case-folded for an ignore-case index, and the
        // reader trusts it.
        let mut reread = Index::open(&file).unwrap();
        let handle = reread.as_mut();
        let paths: Vec<_> = (0..12)
            .map(|i| handle.reuc_entry(i).unwrap().path().to_owned())
            .collect();
        assert_eq!(paths, before);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Case-folded lookups after the index re-sorts its records for ignore-case.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reuc_lookup_follows_ignore_case() {
    let records = [
        Record::new(b"B", [0o100644; 3], 1),
        Record::new(b"a", [0o100644; 3], 2),
        Record::new(b"C", [0o100644; 3], 3),
    ];
    let file = IndexFile::new("icase", &sorted(&records), OidType::Sha1);
    let mut index = file.open(OidType::Sha1);
    assert!(index.as_mut().reuc_entry_by_path(c"A").is_none());
    assert_eq!(set_ignore_case(&mut index, true), 0);
    let handle = index.as_mut();
    let order: Vec<_> = (0..3)
        .map(|i| handle.reuc_entry(i).unwrap().path().to_owned())
        .collect();
    assert_eq!(order, [c"a", c"B", c"C"].map(ToOwned::to_owned));
    assert_eq!(handle.reuc_entry_by_path(c"A").unwrap().path(), c"a");
    assert_eq!(handle.reuc_entry_by_path(c"c").unwrap().path(), c"C");
    assert_eq!(set_ignore_case(&mut index, false), 0);
    assert!(index.as_mut().reuc_entry_by_path(c"A").is_none());
}

/// NUL-terminated fields, as a REUC record spells its path and modes.
fn fields(parts: &[&str]) -> Vec<u8> {
    parts.iter().flat_map(|p| p.bytes().chain([0])).collect()
}

/// Malformed REUC payloads: the reader refuses each, releasing the record it was
/// filling and those it had read, and lends nothing.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reuc_malformed_extensions_refused() {
    let id = [0x5a; 20];
    let good = [fields(&["a", "100644", "0", "0"]), id.to_vec()].concat();
    let cases: [(&str, Vec<u8>); 6] = [
        ("unterminated path", b"path".to_vec()),
        ("path only", fields(&["path"])),
        (
            "non-octal mode",
            [&good[..], &fields(&["b", "9", "0", "0", "x"])].concat(),
        ),
        (
            "mode past u32",
            fields(&["b", "77777777777", "0", "0", "x"]),
        ),
        (
            "all-absent last record",
            [&good[..], &fields(&["b", "0", "0", "0"])].concat(),
        ),
        (
            "missing id",
            [&good[..], &fields(&["b", "100644", "0", "0"]), &id[..19]].concat(),
        ),
    ];
    for (what, data) in cases {
        let file = IndexFile::with_bytes("bad", &extension_bytes(&data, OidType::Sha1));
        let err = Index::open(&file.c_path).map(drop).unwrap_err();
        assert_eq!(err.class(), Some(ErrorClass::Index), "{what}");
    }
    // The same prefix alone is read.
    let file = IndexFile::with_bytes("good", &extension_bytes(&good, OidType::Sha1));
    let mut index = Index::open(&file.c_path).unwrap();
    let handle = index.as_mut();
    let entry = handle.reuc_entry(0).unwrap();
    assert_eq!((entry.path(), entry.modes()), (c"a", [0o100644, 0, 0]));
    assert_eq!(entry.ancestor().unwrap().id.id()[..20], id);
}
