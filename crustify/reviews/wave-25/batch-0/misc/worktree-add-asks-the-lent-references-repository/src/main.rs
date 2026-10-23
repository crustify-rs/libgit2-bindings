//! `Worktree::add` with a reference of another repository: libgit2 asks that other
//! repository whether the branch is checked out, so the branch of that name in the
//! repository added to ends up checked out in two of its working trees.
//!
//! Exits 1 on the affected commit (the second add succeeds, and both worktrees' `HEAD`
//! name the same branch); exits 0 once `Worktree::add` passes libgit2 the repository's
//! own reference of that name (the second add is refused as "already checked out").
//!
//! The fixture (a commit, a reference lookup) goes through `ffi`, as no safe wrapper
//! of `git_odb_write` or `git_reference_lookup` exists yet; the scenario itself only
//! uses the safe API.

use std::ffi::{CStr, CString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::ExitCode;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::core::global;
use libgit2::libgit2::refs::refs::ReferenceBox;
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
use libgit2::libgit2::repository::worktree::{AddOptions, Worktree};

fn c_path(path: &Path) -> CString {
    CString::new(path.as_os_str().as_bytes()).unwrap()
}

/// A non-bare repository at `path` with one commit, which `main` and `feature` name.
fn committed(path: &Path) -> RepositoryBox {
    let repo = Repository::init(&c_path(path), false).unwrap();
    let mut odb = ptr::null_mut();
    // SAFETY: a writable slot and the live repository; the odb is released below.
    assert_eq!(unsafe { ffi::git_repository_odb(&mut odb, repo.as_c_ptr()) }, 0);
    let write = |data: &[u8], kind: ffi::git_object_t| {
        let mut id = ffi::git_oid::default();
        let (bytes, len) = (data.as_ptr().cast(), data.len());
        // SAFETY: a writable id, the live odb and `data` readable for `len` bytes at a
        // non-NULL address.
        assert_eq!(unsafe { ffi::git_odb_write(&mut id, odb, bytes, len, kind) }, 0);
        let mut hex = [0 as std::ffi::c_char; 65];
        // SAFETY: `hex` bounds the write, and the routine NUL-terminates in it.
        unsafe { ffi::git_oid_tostr(hex.as_mut_ptr(), hex.len(), &id) };
        // SAFETY: NUL-terminated within `hex`.
        unsafe { CStr::from_ptr(hex.as_ptr()) }.to_str().unwrap().to_owned()
    };
    let who = "A U Thor <author@example.com> 1700000000 +0000";
    let tree = write(b"", ffi::GIT_OBJECT_TREE);
    let commit = format!("tree {tree}\nauthor {who}\ncommitter {who}\n\nfirst\n");
    let commit = write(commit.as_bytes(), ffi::GIT_OBJECT_COMMIT);
    // SAFETY: the odb reference taken above, released once.
    unsafe { ffi::git_odb_free(odb) };
    for branch in ["main", "feature"] {
        fs::write(path.join(".git/refs/heads").join(branch), format!("{commit}\n")).unwrap();
    }
    fs::write(path.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    repo
}

/// `git_reference_lookup(name)` in `repo`, adopted.
fn lookup<'r>(repo: &'r RepositoryBox, name: &CStr) -> ReferenceBox<'r> {
    let mut out = ptr::null_mut();
    // SAFETY: a writable slot, the live repository and a NUL-terminated name.
    let rc = unsafe { ffi::git_reference_lookup(&mut out, repo.as_c_ptr(), name.as_ptr()) };
    assert_eq!(rc, 0, "{name:?}");
    // SAFETY: a fresh attached reference owned here alone, released by
    // `git_reference_free`; `repo` outlives it.
    unsafe { ReferenceBox::from_c(out) }.unwrap()
}

fn main() -> ExitCode {
    let _runtime = global::init().unwrap();
    let base = std::env::temp_dir().join(format!("worktree-add-lent-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("wt")).unwrap();
    let main = committed(&base.join("main"));
    let other = committed(&base.join("other"));
    let lent = lookup(&other, c"refs/heads/feature");
    let mut opts = AddOptions::new();
    opts.as_mut().set_reference(Some(lent.as_ref()));
    let add = |name: &CStr, rel: &str| {
        Worktree::add(main.as_ref(), name, &c_path(&base.join(rel)), Some(opts.as_ref()))
    };
    let head = |name: &str| {
        fs::read_to_string(base.join("main/.git/worktrees").join(name).join("HEAD")).unwrap()
    };

    let first = add(c"one", "wt/one");
    assert!(first.is_ok(), "{first:?}");
    assert_eq!(head("one"), "ref: refs/heads/feature\n");

    // `main`'s `feature` is checked out in `one` now: a second worktree on it must be
    // refused, as it is with `main`'s own reference.
    let second = add(c"two", "wt/two");
    let outcome = match &second {
        Ok(_) => {
            eprintln!(
                "DEFECT: a second worktree was added on `feature`; HEADs: {:?} and {:?}",
                head("one").trim(),
                head("two").trim()
            );
            ExitCode::FAILURE
        }
        Err(e) if e.message() == c"reference refs/heads/feature is already checked out" => {
            eprintln!("refused: {e}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("unexpected failure: {e:?}");
            ExitCode::FAILURE
        }
    };
    drop(second);
    drop(first);
    drop(opts);
    drop(lent);
    drop((main, other));
    let _ = fs::remove_dir_all(&base);
    outcome
}
