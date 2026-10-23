//! UB tests for the unsafe API of `git2/refdb.h`: the `git_refdb` constructor
//! `Refdb::new`, whose backend is a raw seam until that type is wrapped, beside the
//! safe `Refdb::open` it is compared with.
//!
//! `new` is called within its contract: a live backend of the repository, which the
//! refdb borrows (the fixture's borrow supplies `'r`) on one thread. Both reference storage formats
//! are covered. Every refdb built is freed once with its backend (ASan, LSan), and the backend
//! reaches the repository only while it is live (a compaction through each refdb runs the backend's
//! repository reads). A refused backend stays the caller's, untouched, and is freed
//! by the caller once (a double free if libgit2 had taken it), or handed to a later
//! `new` that takes it.
//!
//! The reftable backend fsyncs every table it writes, through libgit2's `p_fsync`,
//! which counts its calls in an unsynchronised global (`p_fsync__cnt`): reftable
//! writes on two threads race on it, whatever repositories they write. The tests
//! writing reftable storage therefore hold [`REFTABLE`].

use std::ffi::CString;
use std::ptr::{self, NonNull};
use std::sync::Mutex;

use libgit2::ffi;
use libgit2::libgit2::refs::refdb::{Refdb, RefdbArc, RefdbBox, RefdbType};
use libgit2::libgit2::repository::repository::{InitFlag, InitOptions, Repository, RepositoryBox};
use libgit2::libgit2::util::errors::ErrorClass;

/// A fresh bare repository with reference storage `format`, freed and removed on drop.
struct Repo {
    path: std::path::PathBuf,
    repo: RepositoryBox,
}

impl Repo {
    fn new(tag: &str, format: ffi::git_refdb_t) -> Self {
        // SAFETY: balanced by nothing: the reference lives for the process.
        assert!(unsafe { ffi::git_libgit2_init() } > 0);
        let path = std::env::temp_dir().join(format!(
            "crustify-refdb-ub-unsafe-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let mut opts = InitOptions::new();
        let mut m = opts.as_mut();
        m.set_flags(InitFlag::Bare | InitFlag::Mkdir);
        m.set_refdb_type(Some(RefdbType::from_raw(format).unwrap()));
        let repo = Repository::init_ext(&c_path, opts.as_mut()).unwrap();
        Self { path, repo }
    }

    /// `Refdb::open` on this repository, which the borrow keeps alive.
    fn open(&self) -> RefdbBox<'_> {
        Refdb::open(self.repo.as_ref()).unwrap()
    }

    /// A fresh files backend of this repository.
    fn fs_backend(&self) -> NonNull<ffi::git_refdb_backend> {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot and the live repository.
        let rc = unsafe { ffi::git_refdb_backend_fs(&mut out, self.repo.as_c_ptr()) };
        assert_eq!(rc, 0);
        NonNull::new(out).unwrap()
    }

    /// `Refdb::new` on this repository with `backend`.
    ///
    /// # Safety
    ///
    /// `backend` must be a live backend of this repository that nothing else owns.
    unsafe fn with_backend(
        &self,
        backend: NonNull<ffi::git_refdb_backend>,
    ) -> Result<RefdbBox<'_>, libgit2::libgit2::util::errors::ErrorBuf> {
        // SAFETY: a backend of this repository per this function's contract.
        unsafe { Refdb::new(self.repo.as_ref(), backend) }
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        // The repository is freed after (refdbs borrowing it are gone); freeing touches
        // no file.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Compacts through the refdb, which runs its backend over the repository.
fn compress(db: *mut ffi::git_refdb) {
    // SAFETY: a live refdb with a backend, of a live repository.
    assert_eq!(unsafe { ffi::git_refdb_compress(db) }, 0);
}

/// Held by the tests that write reftable storage; see the module docs.
static REFTABLE: Mutex<()> = Mutex::new(());

const FORMATS: [(&str, ffi::git_refdb_t); 2] = [
    ("files", ffi::GIT_REFDB_FILES),
    ("reftable", ffi::GIT_REFDB_REFTABLE),
];

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_builds_independent_refdbs() {
    let _reftable = REFTABLE.lock().unwrap_or_else(|e| e.into_inner());
    for (tag, format) in FORMATS {
        let repo = Repo::new(&format!("open-{tag}"), format);
        let dbs: Vec<_> = (0..4).map(|_| repo.open()).collect();
        for (i, a) in dbs.iter().enumerate() {
            compress(a.as_c_ptr());
            for b in &dbs[i + 1..] {
                assert_ne!(a.as_c_ptr(), b.as_c_ptr());
            }
        }
        // Dropped in reverse, then interleaved with fresh ones.
        drop(dbs);
        let a = repo.open();
        let b = RefdbArc::from(repo.open());
        compress(b.as_c_ptr());
        drop(a);
        drop(b);
    }
}

/// A refdb outlives several repositories opened and freed meanwhile, but not its own.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_borrows_only_its_own_repository() {
    let _reftable = REFTABLE.lock().unwrap_or_else(|e| e.into_inner());
    let repo = Repo::new("open-own", ffi::GIT_REFDB_FILES);
    let db = repo.open();
    for i in 0..3 {
        let other = Repo::new(&format!("open-other-{i}"), ffi::GIT_REFDB_REFTABLE);
        let other_db = other.open();
        compress(other_db.as_c_ptr());
        drop(other_db);
        drop(other);
        compress(db.as_c_ptr());
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn new_takes_a_backend_it_accepts() {
    let repo = Repo::new("new-fs", ffi::GIT_REFDB_FILES);
    for _ in 0..4 {
        // SAFETY: a fresh backend of the repository, owned by nothing else.
        let db = unsafe { repo.with_backend(repo.fs_backend()) }.unwrap();
        compress(db.as_c_ptr());
        let arc = RefdbArc::from(db);
        compress(arc.as_c_ptr());
    }
}

/// Every refusal leaves the backend untouched and the caller's: a stack backend
/// libgit2 never frees, and a heap one the caller frees or hands on.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn new_leaves_a_refused_backend_to_its_caller() {
    let repo = Repo::new("new-refused", ffi::GIT_REFDB_FILES);
    let mut stack = ffi::git_refdb_backend::default();
    for version in [
        0,
        ffi::GIT_REFDB_BACKEND_VERSION + 1,
        i32::MAX as u32,
        u32::MAX,
    ] {
        stack.version = version;
        // SAFETY: a live backend nothing else owns; refused for its version.
        let err = unsafe { repo.with_backend(NonNull::from(&mut stack)) }.unwrap_err();
        assert_eq!(err.class(), Some(ErrorClass::Invalid));
    }
    stack.version = ffi::GIT_REFDB_BACKEND_VERSION;
    // SAFETY: as above; refused for its unset members.
    let err = unsafe { repo.with_backend(NonNull::from(&mut stack)) }.unwrap_err();
    assert_eq!(err.class(), Some(ErrorClass::Reference));
    assert_eq!(stack.version, ffi::GIT_REFDB_BACKEND_VERSION);

    // A heap backend refused for its version, then for `lock` without `unlock`, then
    // accepted once restored: the refdb frees it, once.
    let backend = repo.fs_backend();
    let b = backend.as_ptr();
    // SAFETY: the live backend, which only this test reaches until it is handed over.
    let unlock = unsafe {
        (*b).version = 0;
        (*b).unlock
    };
    // SAFETY: a live backend nothing else owns.
    assert!(unsafe { repo.with_backend(backend) }.is_err());
    // SAFETY: as above, still ours after the refusal.
    unsafe {
        (*b).version = ffi::GIT_REFDB_BACKEND_VERSION;
        (*b).unlock = None;
    }
    // SAFETY: a live backend nothing else owns.
    let err = unsafe { repo.with_backend(backend) }.unwrap_err();
    assert_eq!(err.message(), c"incomplete refdb backend implementation");
    // SAFETY: as above.
    unsafe { (*b).unlock = unlock };
    // SAFETY: a complete backend of the repository, nothing else owning it.
    let db = unsafe { repo.with_backend(backend) }.unwrap();
    compress(db.as_c_ptr());
    drop(db);

    // A refused heap backend its caller frees through its own member.
    let backend = repo.fs_backend();
    // SAFETY: the live backend, ours alone.
    unsafe { (*backend.as_ptr()).version = u32::MAX };
    // SAFETY: a live backend nothing else owns.
    assert!(unsafe { repo.with_backend(backend) }.is_err());
    // SAFETY: the refused backend, still ours, released once.
    unsafe { ((*backend.as_ptr()).free.unwrap())(backend.as_ptr()) };
}
