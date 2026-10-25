//! Safe wrappers over `include/git2/refdb.h`.
//!
//! A [`Refdb`] is a `git_refdb`: a repository's reference database, an opaque C object
//! that dispatches every reference read and write to one backend (`git_refdb_backend`,
//! the loose-and-packed files backend or the reftable one, or a custom one).
//!
//! # Ownership
//!
//! A refdb is reference counted, but libgit2 publishes only the down-ref,
//! `git_refdb_free`, which releases the object and its backend once the last reference
//! goes. The count is raised only inside libgit2: `git_repository_refdb` hands out a
//! new reference to the database the repository keeps, and `git_repository_set_refdb`
//! takes one of its own. So Rust holds a refdb in one of two owners, both released by
//! [`RefdbFree`]:
//!
//! - [`RefdbBox`], a reference known to be the only one: what [`Refdb::open`] and [`Refdb::new`]
//!   build. It lends the exclusive [`RefdbMut`] handle.
//! - [`RefdbArc`], one of several references, the others held by libgit2 (a repository's own
//!   database). It lends the shared [`RefdbRef`] handle only, and does not clone, having no
//!   `up_ref` to call. A [`RefdbBox`] converts into one (`RefdbArc::from`) before libgit2 is given
//!   a reference of its own.
//!
//! # The repository
//!
//! Every refdb points back at the repository it was built for, without a reference
//! to it: libgit2's files and reftable backends keep the same pointer, and read and
//! update the repository (its configuration cache, its paths, its object database)
//! on every reference operation. A refdb therefore borrows its repository: `'r` is the
//! repository's lifetime, and an owner must be dropped within it. A repository that
//! drops its own database while Rust still holds a reference leaves that reference
//! alive, and pointing at a freed repository; `'r` rules that out. Freeing a refdb
//! does not touch the repository with libgit2's own backends; a custom backend's
//! `free` member might.
//!
//! # Backends
//!
//! `git_refdb_new` builds a refdb without a backend, which only
//! `git_refdb_set_backend` makes usable: every routine that reaches the backend
//! dereferences it unchecked (`git_refdb_compress` crashes on one). [`Refdb::new`]
//! therefore takes the backend too, and hands out the database only once it has one,
//! so every handle designates a refdb with a backend.
//!
//! The reftable backend fsyncs every table it writes, through libgit2's `p_fsync`,
//! which counts its calls in an unsynchronised global (`p_fsync__cnt`): reftable writes
//! on different threads race on that counter, whatever refdbs and repositories they go
//! through. Wrappers of the routines that write (`git_refdb_compress` among them) must
//! serialise them, or leave that to their caller.
//!
//! The constructors borrow the repository as a [`RepositoryRef`], which supplies `'r`.
//! [`Refdb::new`] takes the backend as a raw pointer: `git_refdb_backend` is not
//! wrapped yet, and the run wrapping it replaces that seam with an owned backend.
//!
//! The constructors can fail and set the calling thread's libgit2 error, so they pin
//! libgit2's runtime first (`runtime_pinned`) and return the error, copied out, as an
//! [`ErrorBuf`].
//!
//! # The storage format
//!
//! [`RefdbType`] is a `git_refdb_t`: the storage format of a repository's reference
//! database, which `git_repository_init_ext` takes through
//! `git_repository_init_options.refdb_type` and records as the
//! `extensions.refstorage` setting, and which opening a repository reads back from
//! it. libgit2 builds both named formats: loose and packed files, and reftable.
//!
//! Values the header does not name travel through the C enum too. A zeroed
//! `refdb_type` means "the default format", [`RefdbType::default`]. Any other value
//! is written as `extensions.refstorage = unknown`, after which the repository fails
//! to open with `GIT_EINVALID` ("unknown refstorage format 'unknown'"): initialisation
//! reports that failure, but leaves the unopenable repository on disk. So a raw value
//! is converted fallibly ([`RefdbType::from_raw`]) and never transmuted, and an options
//! wrapper takes the typed value.

use core::ffi::c_int;
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use ffibox::{CArc, CBorrowedPtr, CBox, CCell, CDrop, CRefSeam};

use crate::ffi;
use crate::libgit2::src::libgit2::repository::RepositoryRef;
use crate::libgit2::src::util::errors::{ErrorBuf, last, runtime_pinned};

/// A reference database (`git_refdb`) of the repository it borrows for `'r`.
///
/// libgit2 allocates and frees it, so it is only ever reached through a [`RefdbBox`]
/// or a [`RefdbArc`] and their handles. It is opaque: no field is published. See the
/// [module docs](self) for its ownership and its tie to the repository.
///
/// The exclusive handle is invariant in `'r`, as `&mut` is in its pointee:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::refdb::RefdbMut;
///
/// fn shrink<'a, 'r>(m: RefdbMut<'a, 'static>) -> RefdbMut<'a, 'r> {
///     m
/// }
/// ```
///
/// An owner cannot outlive the repository it borrows:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::refdb::RefdbBox;
///
/// fn keep<'r>(db: RefdbBox<'r>) -> RefdbBox<'static> {
///     db
/// }
/// ```
///
/// A handle cannot outlive its owner, nor alias an exclusive one:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::src::libgit2::refdb::{RefdbBox, RefdbRef};
///
/// fn escape(make: impl Fn() -> RefdbBox<'static>) {
///     let r: RefdbRef<'_, 'static>;
///     {
///         let db = make();
///         r = db.as_ref();
///     }
///     let _ = r;
/// }
/// ```
///
/// ```compile_fail,E0499
/// use libgit2::libgit2::src::libgit2::refdb::RefdbBox;
///
/// fn alias(mut db: RefdbBox<'static>) {
///     let a = db.as_mut();
///     let b = db.as_mut();
///     drop((a, b));
/// }
/// ```
///
/// No refdb crosses threads: libgit2's backends reach the repository unsynchronised.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::refdb::RefdbBox;
///
/// fn send<T: Send>(_: T) {}
/// fn check(db: RefdbBox<'static>) {
///     send(db);
/// }
/// ```
///
/// Wraps: `git_refdb`
/// Handwritten
#[repr(transparent)]
pub struct Refdb<'r>(
    ffi::git_refdb,
    PhantomData<(*const (), &'r ffi::git_repository)>,
);

/// Shared borrow of a [`Refdb`]. `Copy`, like `&T`.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct RefdbRef<'a, 'r>(CBorrowedPtr<'a, Refdb<'r>>);

/// Exclusive borrow of a [`Refdb`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// Handwritten
#[repr(transparent)]
pub struct RefdbMut<'a, 'r>(
    RefdbRef<'a, 'r>,
    // Invariant in the refdb, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Refdb<'r>>,
);

// SAFETY: `Refdb` is `#[repr(transparent)]` over `git_refdb` with a zero-sized marker;
// both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Refdb<'r>>`
// (`RefdbMut` through `RefdbRef`) with zero-sized markers and no `Drop`, hand out no
// reference to the refdb, and `RefdbRef` has no write operation. `RefdbMut` is
// invariant in the refdb through its `PhantomData<&'a mut Refdb<'r>>`.
unsafe impl<'r> CCell for Refdb<'r> {
    type C = ffi::git_refdb;
    type Mut<'a>
        = RefdbMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = RefdbRef<'a, 'r>
    where
        Self: 'a;
}

impl<'r> RefdbRef<'_, 'r> {
    /// Borrows the refdb at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live refdb with a backend, of a repository
    /// that outlives `'r`, which stays live for `'a` and is not written through
    /// another handle meanwhile.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_refdb) -> Option<Self> {
        // SAFETY: this function's contract is `CCell::ref_from_ptr`'s: `ptr` is NULL or a
        // live `git_refdb` that outlives `'a` and is not written meanwhile.
        unsafe { <Refdb<'r> as CCell>::ref_from_ptr(ptr) }
    }

    /// Read-only pointer to the C object.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_refdb {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'r> RefdbMut<'_, 'r> {
    /// Writable pointer to the C object.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_refdb {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> RefdbRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> RefdbMut<'_, 'r> {
        RefdbMut(self.0, PhantomData)
    }
}

impl fmt::Debug for RefdbRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Refdb").finish_non_exhaustive()
    }
}

impl fmt::Debug for RefdbMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases one reference to a refdb through `git_refdb_free`, the down-ref: the last
/// one frees the backend (through its `free` member) and then the refdb, unless a
/// repository still owns it, in which case the repository frees it when it lets go.
///
/// Wraps: `git_refdb_free`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct RefdbFree;

// SAFETY: `git_refdb_free` drops exactly the one reference the owner holds, whichever
// of a box or an arc holds it: the refdb is freed once, when its count reaches zero
// and no repository owns it. It needs no state beyond the pointer.
unsafe impl<'r> CDrop<Refdb<'r>> for RefdbFree {
    unsafe fn c_drop(&self, ptr: NonNull<Refdb<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live refdb whose reference
        // the caller gives up; `Refdb` is transparent over `git_refdb`. Its repository
        // outlives `'r`, which the owner being dropped is still within.
        unsafe { ffi::git_refdb_free(ptr.as_ptr().cast()) }
    }
}

/// The only reference to a [`Refdb`], released by `git_refdb_free` on drop.
pub type RefdbBox<'r> = CBox<Refdb<'r>, RefdbFree>;

/// One of several references to a [`Refdb`], released by `git_refdb_free` on drop;
/// shared access only.
pub type RefdbArc<'r> = CArc<Refdb<'r>, RefdbFree>;

/// The refdb a constructor returning `rc` handed out, or libgit2's error.
fn constructed(rc: c_int, db: Option<RefdbBox<'_>>) -> Result<RefdbBox<'_>, ErrorBuf> {
    db.filter(|_| rc >= 0).ok_or_else(last)
}

impl<'r> Refdb<'r> {
    /// The reference database of `repo` with the backend its reference storage format
    /// names: the files backend (loose and packed references), or the reftable one.
    ///
    /// The refdb is independent of the database the repository itself keeps (which
    /// `git_repository_refdb` lends): both read and write the same storage.
    ///
    /// The refdb and its backend keep a pointer to `repo`, through which its operations
    /// read and update the repository: it borrows `repo` for `'r`, and, as neither is
    /// `Send`, stays on the repository's thread. The repository cannot go first:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::src::libgit2::refdb::Refdb;
    /// use libgit2::libgit2::src::libgit2::repository::Repository;
    ///
    /// let repo = Repository::new().unwrap();
    /// let db = Refdb::open(repo.as_ref()).unwrap();
    /// drop(repo);
    /// drop(db);
    /// ```
    ///
    /// Wraps: `git_refdb_open`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, when the backend cannot be set up, or when the
    /// repository's reference storage format is unknown (`GIT_EINVALID`, "unknown
    /// reference storage format").
    pub fn open(repo: RepositoryRef<'r>) -> Result<RefdbBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `repo` a live repository, as libgit2 asserts,
        // which the refdb and its backend keep pointers to for `'r`, the handle's borrow,
        // on this thread, which neither may leave. libgit2 writes the slot NULL on
        // failure. The runtime is pinned, so a failure is reported on live thread-local
        // state.
        let rc = unsafe { ffi::git_refdb_open(&raw mut out, repo.as_ptr().cast_mut()) };
        // SAFETY: `git_refdb_open` leaves `out` NULL on failure (adopted as `None`), or
        // writes a fresh refdb with one reference, held by nothing else and released by
        // `git_refdb_free` as `RefdbFree` does. Its repository outlives `'r`, the
        // handle's borrow.
        constructed(rc, unsafe { RefdbBox::from_c(out) })
    }

    /// A reference database of `repo` dispatching to `backend`: `git_refdb_new`, then
    /// `git_refdb_set_backend`.
    ///
    /// On success the refdb owns `backend`, and releases it through its `free` member
    /// when the last reference to the refdb is dropped. On failure the caller still
    /// owns it.
    ///
    /// `repo` is borrowed as [`open`](Self::open) borrows it.
    ///
    /// Raw seam: `git_refdb_backend` is not wrapped yet; the run wrapping it replaces
    /// `backend` with an owned backend.
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error:
    ///
    /// - when `backend`'s `version` is not one libgit2 knows (`GIT_ERROR_INVALID`, "invalid version
    ///   `n` on `git_refdb_backend`");
    /// - when a member libgit2 requires is unset, or `lock` is set without `unlock`
    ///   (`GIT_EINVALID`, "incomplete refdb backend implementation");
    /// - when libgit2 runs out of memory.
    ///
    /// # Safety
    ///
    /// `backend` must address a live `git_refdb_backend` that nothing else owns, whose
    /// members, once libgit2 accepts them, behave as `git2/sys/refdb_backend.h`
    /// specifies for a backend of `repo`, and whose `free` member releases it. When the
    /// call succeeds, the caller must no longer use or free `backend`.
    ///
    /// Wraps: `git_refdb_new`
    pub unsafe fn new(
        repo: RepositoryRef<'r>,
        backend: NonNull<ffi::git_refdb_backend>,
    ) -> Result<RefdbBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: as in `open`; `git_refdb_new` only records `repo`.
        let rc = unsafe { ffi::git_refdb_new(&raw mut out, repo.as_ptr().cast_mut()) };
        // Owned before the backend is set: a refdb without a backend frees nothing but
        // itself, so a refused backend stays the caller's.
        // SAFETY: `git_refdb_new` leaves `out` NULL on failure (adopted as `None`), or
        // writes a fresh refdb with one reference, held by nothing else and released by
        // `git_refdb_free` as `RefdbFree` does. Its repository outlives `'r` per this
        // function's contract.
        let mut db = constructed(rc, unsafe { RefdbBox::from_c(out) })?;
        // SAFETY: the box's pointer to a fresh refdb nothing else references, and a live
        // backend per this function's contract. libgit2 reads its version and members,
        // and takes it only on success; the fresh refdb has no backend to free first.
        let rc = unsafe { ffi::git_refdb_set_backend(db.as_mut().as_mut_ptr(), backend.as_ptr()) };
        if rc < 0 {
            return Err(last());
        }
        Ok(db)
    }
}

// The layout type is the C type's size, so a refdb pointer converts both ways.
const _: () = assert!(size_of::<Refdb<'static>>() == size_of::<ffi::git_refdb>());

const _: () = assert!(size_of::<RefdbBox<'static>>() == size_of::<*mut ffi::git_refdb>());

/// The storage format of a repository's reference database (`git_refdb_t`), as the
/// `extensions.refstorage` setting names it.
///
/// [`from_raw`](Self::from_raw) gives `None` for every value the header does not name,
/// `0` included, which an options struct reads as [`RefdbType::default`].
///
/// Wraps: `git_refdb_t`
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum RefdbType {
    /// Loose reference files under `refs/`, plus `packed-refs` (`files`).
    Files = ffi::GIT_REFDB_FILES,
    /// A reftable stack under `reftable/` (`reftable`).
    Reftable = ffi::GIT_REFDB_REFTABLE,
}

impl RefdbType {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_refdb_t) -> Option<Self> {
        match raw {
            ffi::GIT_REFDB_FILES => Some(Self::Files),
            ffi::GIT_REFDB_REFTABLE => Some(Self::Reftable),
            _ => None,
        }
    }

    /// The C value, as libgit2's options take it.
    #[must_use]
    pub const fn raw(self) -> ffi::git_refdb_t {
        self as ffi::git_refdb_t
    }
}

/// libgit2's default format, the one a zeroed `refdb_type` selects and a repository
/// without an `extensions.refstorage` setting has.
impl Default for RefdbType {
    fn default() -> Self {
        Self::Files
    }
}

impl TryFrom<ffi::git_refdb_t> for RefdbType {
    type Error = UnnamedRefdbType;

    fn try_from(raw: ffi::git_refdb_t) -> Result<Self, UnnamedRefdbType> {
        Self::from_raw(raw).ok_or(UnnamedRefdbType(raw))
    }
}

impl From<RefdbType> for ffi::git_refdb_t {
    fn from(value: RefdbType) -> ffi::git_refdb_t {
        value.raw()
    }
}

/// A reference database format that this version of libgit2's header does not name:
/// `0`, which an options struct reads as the default, or a value libgit2 records as
/// an unknown format. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedRefdbType(ffi::git_refdb_t);

impl UnnamedRefdbType {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::git_refdb_t {
        self.0
    }
}

impl fmt::Display for UnnamedRefdbType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 reference database type {} is not named by git2/refdb.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedRefdbType {}

#[cfg(test)]
mod unit_tests {
    use std::ffi::CString;
    use std::path::PathBuf;

    use super::*;
    use crate::libgit2::src::libgit2::repository::RepositoryBox;
    use crate::libgit2::src::util::errors::ErrorClass;

    /// A fresh bare repository with reference storage `format`, freed and removed on
    /// drop.
    struct TempRepo {
        path: PathBuf,
        repo: RepositoryBox,
    }

    impl TempRepo {
        fn new(tag: &str, format: ffi::git_refdb_t) -> Self {
            assert!(runtime_pinned());
            let path = std::env::temp_dir()
                .join(format!("crustify-refdb-unit-{}-{tag}", std::process::id()));
            drop(std::fs::remove_dir_all(&path));
            let c_path = CString::new(path.to_str().unwrap()).unwrap();
            let mut opts = ffi::git_repository_init_options::default();
            // SAFETY: `opts` is a live options struct, filled with the version's defaults.
            let rc = unsafe {
                ffi::git_repository_init_options_init(
                    &raw mut opts,
                    ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
                )
            };
            assert_eq!(rc, 0);
            opts.flags = ffi::GIT_REPOSITORY_INIT_BARE | ffi::GIT_REPOSITORY_INIT_MKDIR;
            opts.refdb_type = format;
            let mut repo = ptr::null_mut();
            // SAFETY: a live slot, a NUL-terminated path and live options, read for the
            // call.
            let rc = unsafe {
                ffi::git_repository_init_ext(&raw mut repo, c_path.as_ptr(), &raw mut opts)
            };
            assert_eq!(rc, 0, "{:?}", last());
            Self {
                path,
                // SAFETY: the fresh repository the call wrote, owned here alone.
                repo: unsafe { RepositoryBox::from_c(repo) }.unwrap(),
            }
        }

        /// A refdb of this repository, which the borrow keeps alive.
        fn open(&self) -> RefdbBox<'_> {
            Refdb::open(self.repo.as_ref()).unwrap()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            // The repository is freed after (refdbs borrowing it are gone); freeing
            // touches no file.
            drop(std::fs::remove_dir_all(&self.path));
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn handles_reborrow_and_debug() {
        let repo = TempRepo::new("handles", ffi::GIT_REFDB_FILES);
        let mut db = repo.open();
        assert_eq!(format!("{:?}", db.as_ref()), "Refdb { .. }");
        let mut m = db.as_mut();
        assert_eq!(format!("{m:?}"), "Refdb { .. }");
        let p = m.as_mut().as_mut_ptr();
        assert_eq!(m.as_ref().as_ptr(), p.cast_const());
        assert_eq!(db.as_c_ptr(), p);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn from_ptr_borrows_without_releasing() {
        let repo = TempRepo::new("from-ptr", ffi::GIT_REFDB_FILES);
        let db = repo.open();
        // SAFETY: NULL.
        assert!(unsafe { RefdbRef::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: the box's live refdb, which outlives the handle and is not written
        // meanwhile.
        let r = unsafe { RefdbRef::from_ptr(db.as_c_ptr()) }.unwrap();
        assert_eq!(r.as_ptr(), db.as_c_ptr().cast_const());
        // The box still holds its reference: compressing reaches the backend.
        // SAFETY: the box's live refdb, with a backend.
        assert_eq!(unsafe { ffi::git_refdb_compress(db.as_c_ptr()) }, 0);
    }

    /// The raw constructor's result adopted through `from_c` is released once by the
    /// policy (`LSan` flags a leak, `ASan` a double free).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn from_c_adopts_a_raw_refdb() {
        let repo = TempRepo::new("from-c", ffi::GIT_REFDB_FILES);
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot and the live repository.
        let rc = unsafe { ffi::git_refdb_open(&raw mut out, repo.repo.as_c_ptr()) };
        assert_eq!(rc, 0);
        // SAFETY: a fresh refdb with one reference, held by nothing else.
        let db: RefdbBox<'_> = unsafe { RefdbBox::from_c(out) }.unwrap();
        let arc = RefdbArc::from(db);
        assert_eq!(arc.as_c_ptr(), out);
        drop(arc);
        // SAFETY: NULL.
        assert!(unsafe { RefdbBox::from_c(ptr::null_mut()) }.is_none());
    }

    /// A repository's own refdb, lent by `git_repository_refdb`, is one reference of
    /// several: dropping the arc releases that reference only, and the repository
    /// lends the same database again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn arc_releases_one_reference_of_a_repository_refdb() {
        let repo = TempRepo::new("arc", ffi::GIT_REFDB_FILES);
        let lend = || {
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot and the live repository.
            let rc = unsafe { ffi::git_repository_refdb(&raw mut out, repo.repo.as_c_ptr()) };
            assert_eq!(rc, 0);
            // SAFETY: a new reference to the repository's refdb, given to the caller;
            // the repository outlives the arc.
            unsafe { RefdbArc::<'_>::from_c(out) }.unwrap()
        };
        let first = lend();
        let second = lend();
        assert!(RefdbArc::ptr_eq(&first, &second));
        let p = first.as_c_ptr();
        drop(first);
        drop(second);
        let third = lend();
        assert_eq!(third.as_c_ptr(), p);
        // SAFETY: the arc's live refdb, with a backend.
        assert_eq!(unsafe { ffi::git_refdb_compress(third.as_c_ptr()) }, 0);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_follows_the_storage_format() {
        for (tag, format) in [
            ("files", ffi::GIT_REFDB_FILES),
            ("reftable", ffi::GIT_REFDB_REFTABLE),
        ] {
            let repo = TempRepo::new(tag, format);
            let a = repo.open();
            let b = repo.open();
            assert_ne!(a.as_c_ptr(), b.as_c_ptr());
            // SAFETY: the box's live refdb, with a backend.
            assert_eq!(unsafe { ffi::git_refdb_compress(a.as_c_ptr()) }, 0);
            drop(b);
            drop(a);
        }
    }

    /// `new` hands out the refdb only with a backend; a refused backend stays the
    /// caller's, untouched.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_refuses_bad_backends() {
        let repo = TempRepo::new("bad-backend", ffi::GIT_REFDB_FILES);
        let mut backend = ffi::git_refdb_backend::default();
        for version in [0, ffi::GIT_REFDB_BACKEND_VERSION + 1, u32::MAX] {
            backend.version = version;
            // SAFETY: a live backend; the call fails, so it stays the caller's.
            let err =
                unsafe { Refdb::new(repo.repo.as_ref(), NonNull::from(&mut backend)) }.unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Invalid));
            let want = format!(
                "invalid version {} on git_refdb_backend",
                version.cast_signed()
            );
            assert_eq!(err.message().to_str().unwrap(), want);
        }
        backend.version = ffi::GIT_REFDB_BACKEND_VERSION;
        // SAFETY: as above.
        let err =
            unsafe { Refdb::new(repo.repo.as_ref(), NonNull::from(&mut backend)) }.unwrap_err();
        assert_eq!(err.class(), Some(ErrorClass::Reference));
        assert_eq!(err.message(), c"incomplete refdb backend implementation");
        assert_eq!(backend.version, ffi::GIT_REFDB_BACKEND_VERSION);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_takes_an_fs_backend() {
        let repo = TempRepo::new("fs-backend", ffi::GIT_REFDB_FILES);
        let mut backend = ptr::null_mut();
        // SAFETY: a writable slot and the live repository.
        let rc = unsafe { ffi::git_refdb_backend_fs(&raw mut backend, repo.repo.as_c_ptr()) };
        assert_eq!(rc, 0);
        // SAFETY: a fresh files backend of this repository, given to the refdb.
        let db = unsafe { Refdb::new(repo.repo.as_ref(), NonNull::new(backend).unwrap()) }.unwrap();
        // SAFETY: the box's live refdb, with the backend just set.
        assert_eq!(unsafe { ffi::git_refdb_compress(db.as_c_ptr()) }, 0);
    }

    /// `RefdbType`, `git_refdb_t`.
    mod refdb_type {
        use super::super::*;

        /// Every `git_refdb_t` constant, beside the variant that must carry it.
        const TYPES: [(ffi::git_refdb_t, RefdbType); 2] = [
            (ffi::GIT_REFDB_FILES, RefdbType::Files),
            (ffi::GIT_REFDB_REFTABLE, RefdbType::Reftable),
        ];

        #[test]
        fn variants_carry_the_c_values() {
            for (raw, kind) in TYPES {
                assert_eq!(kind.raw(), raw);
                assert_eq!(ffi::git_refdb_t::from(kind), raw);
                assert_eq!(RefdbType::from_raw(raw), Some(kind));
                assert_eq!(RefdbType::try_from(raw), Ok(kind));
            }
            assert_eq!(size_of::<RefdbType>(), size_of::<ffi::git_refdb_t>());
            assert_eq!(
                size_of::<Option<RefdbType>>(),
                size_of::<ffi::git_refdb_t>()
            );
        }

        #[test]
        fn unnamed_values_are_refused() {
            for raw in (0..=300)
                .filter(|raw| TYPES.iter().all(|&(named, _)| named != *raw))
                .chain([ffi::git_refdb_t::MAX - 1, ffi::git_refdb_t::MAX])
            {
                assert_eq!(RefdbType::from_raw(raw), None, "{raw}");
                let e = RefdbType::try_from(raw).unwrap_err();
                assert_eq!(e.raw(), raw);
                assert_eq!(e, UnnamedRefdbType(raw));
            }
        }

        #[test]
        fn default_is_the_files_format() {
            assert_eq!(RefdbType::default(), RefdbType::Files);
            assert_eq!(RefdbType::default().raw(), ffi::GIT_REFDB_FILES);
        }

        #[test]
        fn errors_describe_the_value() {
            let e = RefdbType::try_from(0).unwrap_err();
            assert_eq!(
                e.to_string(),
                "libgit2 reference database type 0 is not named by git2/refdb.h"
            );
            assert_eq!(format!("{e:?}"), "UnnamedRefdbType(0)");
            assert_eq!(format!("{:?}", RefdbType::Reftable), "Reftable");
            let source: &dyn std::error::Error = &e;
            assert!(source.source().is_none());
        }
    }
}
