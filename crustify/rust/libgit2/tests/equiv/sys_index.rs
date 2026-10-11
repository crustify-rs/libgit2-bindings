//! Independent NAME vectors compared after every add, lazy sort and clear.
use libgit2::ffi;
use libgit2::libgit2::index::index::{Index, IndexBox};
use std::ffi::{CStr, CString};
use std::ptr;

type Names = [Option<CString>; 3];

fn raw_names(index: &IndexBox) -> Vec<Names> {
    // SAFETY: this test alone accesses the index; C may sort, but each string is
    // copied before any mutation or drop. Null fields mean absent names.
    unsafe {
        (0..ffi::git_index_name_entrycount(index.as_c_ptr()))
            .map(|i| {
                let entry = ffi::git_index_name_get_byindex(index.as_c_ptr(), i);
                [
                    ptr::addr_of!((*entry).ancestor).read(),
                    ptr::addr_of!((*entry).ours).read(),
                    ptr::addr_of!((*entry).theirs).read(),
                ]
                .map(|p| {
                    if p.is_null() {
                        None
                    } else {
                        Some(CStr::from_ptr(p).to_owned())
                    }
                })
            })
            .collect()
    }
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
