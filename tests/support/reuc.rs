//! Index files carrying a REUC extension, which `Index::open` reads into index-owned
//! resolve-undo records: the safe way to build them before a safe add exists.
#![forbid(unsafe_code)]
#![expect(
    dead_code,
    reason = "each suite that includes the shared support uses part of it"
)]
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use libgit2::libgit2::src::libgit2::index::{Index, IndexBox, ReucEntryRef};
use libgit2::libgit2::src::libgit2::oid::OidType;

/// One record as written: its path, three modes (0 for an absent side), and the ids,
/// of which only those with a non-zero mode reach the file.
#[derive(Clone, Debug)]
pub(super) struct Record {
    pub path: Vec<u8>,
    pub modes: [u32; 3],
    pub ids: [[u8; 32]; 3],
}

impl Record {
    /// A record whose ids are derived from `seed`.
    pub(super) fn new(path: &[u8], modes: [u32; 3], seed: u8) -> Self {
        let ids = [0, 1, 2].map(|side| {
            let mut id = [0; 32];
            for (i, byte) in id.iter_mut().enumerate() {
                *byte = seed
                    .wrapping_mul(31)
                    .wrapping_add(side * 7)
                    .wrapping_add(u8::try_from(i).unwrap());
            }
            id
        });
        Self {
            path: path.to_vec(),
            modes,
            ids,
        }
    }
}

/// The bytes of an index of `kind` with no entries and one REUC extension holding
/// `records` in the given order, and an all-zero checksum (which libgit2 accepts).
pub(super) fn index_bytes(records: &[Record], kind: OidType) -> Vec<u8> {
    let size = kind.size();
    let mut data = Vec::new();
    for record in records {
        data.extend_from_slice(&record.path);
        data.push(0);
        for mode in record.modes {
            data.extend_from_slice(format!("{mode:o}").as_bytes());
            data.push(0);
        }
        for (mode, id) in record.modes.iter().zip(&record.ids) {
            if *mode != 0 {
                data.extend_from_slice(&id[..size]);
            }
        }
    }
    extension_bytes(&data, kind)
}

/// The bytes of an index of `kind` with no entries and one REUC extension whose
/// payload is `data`, verbatim, and an all-zero checksum.
pub(super) fn extension_bytes(data: &[u8], kind: OidType) -> Vec<u8> {
    let mut bytes = b"DIRC\0\0\0\x02\0\0\0\0REUC".to_vec();
    bytes.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(data);
    bytes.extend(std::iter::repeat_n(0, kind.size()));
    bytes
}

/// An index file in the temporary directory, removed on drop.
pub(super) struct IndexFile {
    pub path: PathBuf,
    pub c_path: CString,
}

impl IndexFile {
    pub(super) fn new(tag: &str, records: &[Record], kind: OidType) -> Self {
        Self::with_bytes(tag, &index_bytes(records, kind))
    }

    /// A file holding `bytes` verbatim.
    pub(super) fn with_bytes(tag: &str, bytes: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "crustify-reuc-{}-{tag}-{}.index",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        let c_path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        Self { path, c_path }
    }

    /// A fresh, independently owned index read from the file.
    pub(super) fn open(&self, kind: OidType) -> IndexBox {
        Index::open_with_oid_type(&self.c_path, kind).unwrap()
    }
}

impl Drop for IndexFile {
    fn drop(&mut self) {
        drop(std::fs::remove_file(&self.path));
    }
}

/// What a record holds, copied out: path, modes, and each slot's type byte and id.
pub(super) type Snapshot = (Vec<u8>, [u32; 3], [(u8, [u8; 32]); 3]);

/// Copies a record out through the safe handle.
pub(super) fn snapshot(entry: ReucEntryRef<'_>) -> Snapshot {
    let ids = entry.ids();
    let mut slots = [(0, [0; 32]); 3];
    for (slot, id) in slots.iter_mut().zip(ids.iter()) {
        *slot = (id.raw_kind(), id.id());
    }
    (entry.path().to_bytes().to_vec(), entry.modes(), slots)
}

/// `records` sorted by path, as C keeps (and writes) them.
pub(super) fn sorted(records: &[Record]) -> Vec<Record> {
    let mut records = records.to_vec();
    records.sort_by(|a, b| a.path.cmp(&b.path));
    records
}
