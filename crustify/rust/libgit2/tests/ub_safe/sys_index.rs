//! Public NAME-entry borrows, input copying, lazy sort, clear and parent release.
#![forbid(unsafe_code)]

use libgit2::libgit2::index::index::Index;
use std::ffi::CString;

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
