//! `checkout::ProgressCallback`'s docs (commit 3b654f097) promise a call after every file
//! a checkout removes or writes and every conflict it resolves, with "the count so far".
//! A forced tree checkout over an index holding conflicts removes them from the index,
//! counts each removal as a completed step, and reports none: here the closure sees the
//! baseline `(None, 0, 2)`, then `(Some("a.txt"), 2, 2)`, never step 1. Exits 1 while the
//! documented contract does not hold.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::{fs, ptr};

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::checkout::{CheckoutOptions, ProgressCallback};

const FORCE: u32 = 1 << 1;

extern "C" fn forward(path: *const c_char, completed: usize, total: usize, payload: *mut c_void) {
    // SAFETY: `payload` is the `ProgressCallback` `main` lends for the checkout alone.
    let callback = unsafe { &mut *payload.cast::<ProgressCallback<'_>>() };
    // SAFETY: libgit2 passes NULL or a NUL-terminated string live for the call.
    let path = (!path.is_null()).then(|| unsafe { CStr::from_ptr(path) });
    callback.call(path, completed, total);
}

fn main() {
    // SAFETY: no precondition.
    unsafe { ffi::git_libgit2_init() };
    let dir = std::env::temp_dir().join(format!("progress-skips-{}", std::process::id()));
    drop(fs::remove_dir_all(&dir));
    fs::create_dir_all(&dir).unwrap();
    let c_dir = CString::new(dir.to_str().unwrap()).unwrap();
    let (mut repo, mut index) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: live slots and a NUL-terminated path; the results are owned here (leaked).
    unsafe {
        assert_eq!(ffi::git_repository_init(&raw mut repo, c_dir.as_ptr(), 0), 0);
        assert_eq!(ffi::git_repository_index(&raw mut index, repo), 0);
    }
    fs::write(dir.join("a.txt"), "a\n").unwrap();
    let mut id = ffi::git_oid::default();
    let mut tree = ptr::null_mut();
    // SAFETY: the live index and repository; live out-parameters.
    unsafe {
        assert_eq!(ffi::git_index_add_bypath(index, c"a.txt".as_ptr()), 0);
        assert_eq!(ffi::git_index_write_tree(&raw mut id, index), 0);
        assert_eq!(ffi::git_tree_lookup(&raw mut tree, repo, &raw const id), 0);
    }
    // A content conflict on `c.txt` (not in the tree, not on disk): three index entries.
    let side = |content: &str| {
        let mut id = ffi::git_oid::default();
        // SAFETY: live out-parameter and buffer, read for the call.
        let rc = unsafe {
            ffi::git_blob_create_from_buffer(&raw mut id, repo, content.as_ptr().cast(), content.len())
        };
        assert_eq!(rc, 0);
        ffi::git_index_entry { mode: 0o100_644, id, path: c"c.txt".as_ptr(), ..Default::default() }
    };
    let (anc, ours, theirs) = (side("0\n"), side("1\n"), side("2\n"));
    // SAFETY: the live index; the entries are read for the call.
    let rc = unsafe { ffi::git_index_conflict_add(index, &raw const anc, &raw const ours, &raw const theirs) };
    assert_eq!(rc, 0);

    let seen = RefCell::new(Vec::new());
    let mut record = |path: Option<&CStr>, completed: usize, total: usize| {
        seen.borrow_mut().push((path.map(CStr::to_owned), completed, total));
    };
    let mut callback = ProgressCallback::new(&mut record);
    let mut options = CheckoutOptions::new();
    options.as_mut().set_strategy(FORCE);
    // SAFETY: `forward` is sound for libgit2's arguments; the payload outlives the checkout.
    unsafe {
        options
            .as_mut()
            .set_progress_callback(Some(forward), ptr::from_mut(&mut callback).cast());
    }
    // SAFETY: `CheckoutOptions` is `#[repr(transparent)]` over `git_checkout_options`.
    let raw = ptr::from_ref(&options).cast::<ffi::git_checkout_options>();
    // SAFETY: the live repository and tree; live options.
    let rc = unsafe { ffi::git_checkout_tree(repo, tree.cast(), raw) };
    drop(callback);
    drop(fs::remove_dir_all(&dir));
    assert_eq!(rc, 0);
    let seen = seen.into_inner();
    println!("calls: {seen:?}");
    let total = seen[0].2;
    // The docs promise the baseline, then one call per step, each with the count so far.
    let promised: Vec<usize> = (0..=total).collect();
    let reported: Vec<usize> = seen.iter().map(|call| call.1).collect();
    if reported != promised {
        eprintln!(
            "documented contract broken: {total} steps counted, completed counts reported \
             {reported:?}, documented {promised:?}"
        );
        std::process::exit(1);
    }
}
