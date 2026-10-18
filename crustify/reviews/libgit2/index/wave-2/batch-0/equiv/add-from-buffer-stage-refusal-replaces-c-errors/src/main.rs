//! `IndexMut::add_from_buffer` refuses an entry of a conflict stage before libgit2
//! sees it, whatever the index and the other arguments. libgit2 only misbehaves on
//! such an entry once it inserts it (`git_index_add_from_buffer`,
//! `src/libgit2/index.c:1557-1563`), which needs an index a repository owns, a file or
//! link mode and at most `UINT32_MAX` bytes. Before that, it fails safely with its own
//! errors, and the wrapper's refusal replaces them.
//!
//! Through the safe API an exclusive handle only ever reaches an index no repository
//! owns, so every stage 1-3 call the wrapper refuses is one libgit2 fails cleanly with
//! "could not initialize index entry. Index is not backed up by an existing
//! repository." The reproducer runs three steps on two independent in-memory indexes,
//! one through raw C and one through the safe API: a stage 2 entry of a file mode, a
//! stage 3 entry of a tree mode, then a stage 0 file (the baseline both agree on). It
//! compares the return code, the error class and the message after each step, and the
//! entry count.
//!
//! Run from this directory:
//!
//! ```sh
//! LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run
//! ```
//!
//! On the affected commit the first step's messages differ and it exits 101; it exits 0
//! once the wrapper refuses only what libgit2 would insert.

use std::ffi::{CStr, CString, c_int};
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::core::types::FileMode;
use libgit2::libgit2::index::index::{Index, IndexEntry, Stage};
use libgit2::libgit2::oid::oid::{Oid, OidType};

/// A step's outcome: code, then class and message when it failed, and the entry count.
type Observation = (c_int, Option<(c_int, CString)>, usize);

fn entry(stage: Stage, mode: u32) -> IndexEntry<'static> {
    let id = Oid::from_raw(&[0; 20], OidType::Sha1).unwrap();
    let mut entry = IndexEntry::new(c"f", id.as_ref(), FileMode::Blob);
    let mut handle = entry.as_mut();
    handle.set_mode(mode);
    handle.set_stage(stage).unwrap();
    entry
}

fn main() {
    let mut rust = Index::new().unwrap();
    let mut c = ptr::null_mut();
    // SAFETY: the runtime is pinned by `Index::new`; a writable slot.
    assert_eq!(unsafe { ffi::git_index_new(&mut c) }, 0);

    let steps = [
        ("stage 2 file", entry(Stage::Ours, 0o100644)),
        ("stage 3 tree", entry(Stage::Theirs, 0o040000)),
        ("stage 0 file", entry(Stage::Normal, 0o100644)),
    ];
    let mut failed = false;
    for (step, entry) in &steps {
        // SAFETY: a live index lending no entry; the entry, its static path and the
        // byte are read for the call.
        let rc = unsafe {
            ffi::git_index_add_from_buffer(c, &entry.into_raw(), b"x".as_ptr().cast(), 1)
        };
        // SAFETY: as above; the error is copied before the next call.
        let expected: Observation = unsafe {
            let error = ffi::git_error_last();
            (
                rc,
                (rc != 0).then(|| ((*error).klass, CStr::from_ptr((*error).message).to_owned())),
                ffi::git_index_entrycount(c),
            )
        };
        let actual: Observation = match rust.as_mut().add_from_buffer(entry.as_ref(), b"x") {
            Ok(()) => (0, None, rust.as_ref().entry_count()),
            Err(e) => (
                e.raw_code(),
                Some((e.raw_class(), e.message().to_owned())),
                rust.as_ref().entry_count(),
            ),
        };
        println!("{step}:\n  C:    {expected:?}\n  Rust: {actual:?}");
        failed |= expected != actual;
    }
    // SAFETY: the only reference to the C index, released once.
    unsafe { ffi::git_index_free(c) };
    assert!(!failed, "the safe API's errors differ from libgit2's");
}
