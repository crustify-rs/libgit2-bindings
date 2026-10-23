//! Safe wrappers over `include/git2/refs.h`.
//!
//! A [`Reference`] is a `git_reference`: a named pointer at an object id (a direct
//! reference) or at another reference's name (a symbolic one), read from a
//! repository's reference database. It is opaque: libgit2 publishes no field. libgit2
//! allocates it, with the name inline, and hands the caller sole ownership; nothing
//! counts references to it. `git_reference_free` releases it and `git_reference_dup`
//! deep-copies it, so Rust owns one through a [`ReferenceBox`], whose `Clone` is the
//! copy.
//!
//! A reference libgit2 read from a repository is attached to that repository's
//! reference database: it holds a counted reference on the database, which keeps the
//! database alive, and a copy holds another. The database's pointer back to its
//! repository is not counted, and the operations that reach the repository through a
//! reference (`git_reference_owner`, resolving, renaming, retargeting, deleting,
//! peeling; and `git_reference_dup`, through the database's count) would read a freed
//! repository once the reference outlived it. So a reference carries the lifetime
//! `'r` of the repository borrow it was read through, and lives no longer than it.
//!
//! A reference database backend builds its references itself, through
//! `git_reference__alloc` and `git_reference__alloc_symbolic` (`git2/sys/refs.h`), and
//! hands them to libgit2, which attaches them to its database. Until then such a
//! reference has no database: `git_reference_dup` and `git_reference_owner` would
//! dereference NULL. It is an [`UnattachedReference`], a distinct type with no copy,
//! built by [`UnattachedReference::direct`] and [`UnattachedReference::symbolic`] and
//! released by the same `git_reference_free`, the one policy both owners share
//! ([`ReferenceFree`]).
//!
//! An attached reference is neither `Send` nor `Sync`: what it reaches through its
//! repository is the repository's, which libgit2 does not make safe to share across
//! threads, and its repository borrow is not known to be shareable. An unattached
//! reference reaches nothing but its own allocation, so it is both.
//!
//! The constructors and the copy can fail and set the calling thread's libgit2 error,
//! so they pin libgit2's runtime first (`runtime_pinned`) and report the error, copied
//! out, as an [`ErrorBuf`] (the copy as `None` from `try_clone`, or an abort from
//! `Clone`).

use core::ffi::CStr;
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use ffibox::{CBorrowedPtr, CBox, CCell, CDrop, CDupClone, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::oid::oid::OidRef;
use crate::libgit2::util::errors::{ErrorBuf, last};
use crate::libgit2::util::sys::errors::runtime_pinned;

/// A Git reference read from a repository (`git_reference`): a name, and the object
/// id or the other reference's name it points at.
///
/// libgit2 allocates and frees it, so it is only ever reached through a
/// [`ReferenceBox`] and its handles. It is opaque: no field is published.
///
/// Every `Reference` is attached to a reference database, as every reference
/// libgit2's non-`sys` routines hand out is: adopting a pointer into a
/// [`ReferenceBox`] (`from_c`) asserts that, beside ownership. A reference a backend
/// builds is an [`UnattachedReference`].
///
/// `'r` is the borrow of the repository the reference was read through: operations on
/// the reference reach that repository, so it lives no longer than the borrow. The
/// lifetime only shrinks:
///
/// ```compile_fail
/// use libgit2::libgit2::refs::refs::ReferenceBox;
///
/// fn widen<'r>(reference: ReferenceBox<'r>) -> ReferenceBox<'static> {
///     reference
/// }
/// ```
///
/// and a reference stays on the thread that read it:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::refs::refs::ReferenceBox;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(reference: ReferenceBox<'static>) {
///     send(reference)
/// }
/// ```
///
/// Wraps: `git_reference`
/// Handwritten
#[repr(transparent)]
pub struct Reference<'r>(ffi::git_reference, PhantomData<(*const (), &'r ())>);

/// Shared borrow of a [`Reference`]. `Copy`, like `&T`.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ReferenceRef<'a, 'r>(CBorrowedPtr<'a, Reference<'r>>);

/// Exclusive borrow of a [`Reference`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// Handwritten
#[repr(transparent)]
pub struct ReferenceMut<'a, 'r>(
    ReferenceRef<'a, 'r>,
    // Invariant in the reference, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Reference<'r>>,
);

// SAFETY: `Reference` is `#[repr(transparent)]` over `git_reference` with a zero-sized
// marker; both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Reference<'r>>`
// (`ReferenceMut` through `ReferenceRef`) with zero-sized markers and no `Drop`, hand
// out no reference to the C object, and `ReferenceRef` has no write operation.
// `ReferenceMut` is invariant in the reference through its
// `PhantomData<&'a mut Reference<'r>>`.
unsafe impl<'r> CCell for Reference<'r> {
    type C = ffi::git_reference;
    type Mut<'a>
        = ReferenceMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = ReferenceRef<'a, 'r>
    where
        Self: 'a;
}

impl<'r> ReferenceRef<'_, 'r> {
    /// Borrows a raw pointer; `None` if null.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live reference attached to a reference
    /// database, which no one frees or writes during `'a`, and whose repository
    /// outlives `'r`.
    pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_reference) -> Option<Self> {
        // SAFETY: this function's contract is `CCell::ref_from_ptr`'s: `ptr` is NULL or a
        // live `git_reference` that outlives `'a` and is not written meanwhile.
        unsafe { <Reference<'r> as CCell>::ref_from_ptr(ptr.cast_mut()) }
    }

    /// Read-only pointer to the C object.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_reference {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'r> ReferenceMut<'_, 'r> {
    /// Borrows a raw pointer exclusively; `None` if null.
    ///
    /// # Safety
    ///
    /// As [`ReferenceRef::from_ptr`], plus: nothing else reaches the reference while
    /// the result lives.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_reference) -> Option<Self> {
        // SAFETY: the caller upholds `ReferenceRef::from_ptr`'s contract and
        // exclusivity.
        unsafe { ReferenceRef::from_ptr(ptr) }.map(|r| ReferenceMut(r, PhantomData))
    }

    /// Writable pointer to the C object.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_reference {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> ReferenceRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> ReferenceMut<'_, 'r> {
        ReferenceMut(self.0, PhantomData)
    }
}

impl fmt::Debug for ReferenceRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reference").finish_non_exhaustive()
    }
}

impl fmt::Debug for ReferenceMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

define_ctype!(
    /// A reference a reference database backend built, not yet handed to libgit2
    /// (`git_reference` from `git2/sys/refs.h`): a name, and the object id or the
    /// other reference's name it points at, with no reference database.
    ///
    /// libgit2 allocates and frees it, so it is only ever reached through an
    /// [`UnattachedReferenceBox`], built by [`direct`](Self::direct) or
    /// [`symbolic`](Self::symbolic), and its handles. It is opaque: no field is
    /// published.
    ///
    /// It has no database, so it reaches no repository and has no lifetime, but it
    /// cannot be copied either: `git_reference_dup` counts the copy against the
    /// database.
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::oid::oid::{Oid, OidType};
    /// use libgit2::libgit2::refs::refs::UnattachedReference;
    ///
    /// let id = Oid::from_raw(&[0x11; 20], OidType::Sha1).unwrap();
    /// let reference = UnattachedReference::direct(c"refs/heads/main", id.as_ref(), None).unwrap();
    /// let _copy = reference.clone();
    /// ```
    ///
    /// Wraps: git_reference
    UnattachedReference,
    UnattachedReferenceRef,
    UnattachedReferenceMut,
    ffi::git_reference
);

// SAFETY: an unattached reference is one `git__calloc`'d struct, with its name inline,
// and for a symbolic one a `git__strdup`'d target, all from libgit2's process-wide
// allocator, which frees on any thread; its database is NULL, so it shares no state
// with anything else. No C routine keeps thread-local state about it.
unsafe impl Send for UnattachedReference {}
// SAFETY: libgit2 never writes a reference through a `const git_reference *`, the only
// access a shared handle grants, and builds its fields once, before the reference is
// shared; an unattached one has no database refcount for a reader to touch.
unsafe impl Sync for UnattachedReference {}

impl fmt::Debug for UnattachedReferenceRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnattachedReference")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for UnattachedReferenceMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases a reference through `git_reference_free`, which frees a symbolic
/// reference's target, gives back its count on its reference database (if any), and
/// frees the struct; and copies an attached one through `git_reference_dup`.
///
/// The free needs no repository: the database outlives the reference through its
/// count. The copy is a new allocation with the same name, target and peeled id,
/// counting one more reference against the same database, so it is attached to the
/// same repository, for the same `'r`. An [`UnattachedReference`] has no copy: its
/// database is NULL, which `git_reference_dup` would count against.
///
/// Wraps: `git_reference_free`
/// Wraps: `git_reference_dup`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct ReferenceFree;

// SAFETY: `git_reference_free` releases a live reference exactly once in every state
// the owners reach: attached (it gives back the database count the reference holds)
// or not (NULL database, skipped), direct or symbolic (it frees the target only for a
// symbolic one). It needs no state beyond the pointer, and no repository.
unsafe impl<'r> CDrop<Reference<'r>> for ReferenceFree {
    unsafe fn c_drop(&self, ptr: NonNull<Reference<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live reference whose
        // ownership the caller gives up; `Reference` is transparent over
        // `git_reference`.
        unsafe { ffi::git_reference_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `git_reference_dup` returns a fresh allocation (`git_reference__alloc` or
// `__alloc_symbolic`, the name and symbolic target copied) that `git_reference_free`
// releases, or NULL on failure; it only reads `ptr`'s fields and increments its
// database's atomic count, leaving `ptr` valid. The copy shares the database, which a
// reference never writes but for that count: a counted, shared object, as the
// original's. Every `Reference` is attached, so the database it counts against is not
// NULL.
unsafe impl<'r> CDupClone<Reference<'r>> for ReferenceFree {
    unsafe fn c_dup(&self, ptr: NonNull<Reference<'r>>) -> Option<NonNull<Reference<'r>>> {
        if !runtime_pinned() {
            return None;
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a live slot; `ptr` addresses a live attached reference, per
        // `c_dup`'s contract and `Reference`'s invariant, read and its database count
        // incremented for the call only. The runtime is pinned, so a failure is reported
        // on live thread-local state.
        let rc = unsafe { ffi::git_reference_dup(&raw mut out, ptr.as_ptr().cast()) };
        if rc < 0 {
            return None;
        }
        NonNull::new(out.cast())
    }
}

impl_cdrop!(ReferenceFree, UnattachedReference, ffi::git_reference_free);

/// An owned [`Reference`], freed by `git_reference_free` on drop and copied by
/// `git_reference_dup` on `clone` (`try_clone` reports a failed copy as `None`).
pub type ReferenceBox<'r> = CBox<Reference<'r>, ReferenceFree>;

/// An owned [`UnattachedReference`], freed by `git_reference_free` on drop.
pub type UnattachedReferenceBox = CBox<UnattachedReference, ReferenceFree>;

impl UnattachedReference {
    /// A new direct reference named `name`, pointing at `target`, with `peel` as the
    /// id of the object it peels to (a tag's target, for a tag reference), as a backend
    /// reads one from storage.
    ///
    /// Without `peel`, the reference has no peeled id: an all-zero one of `target`'s
    /// type. libgit2 copies `name` and the ids without validating either.
    ///
    /// Wraps: `git_reference__alloc`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's out-of-memory error, when libgit2 cannot allocate it.
    pub fn direct(
        name: &CStr,
        target: OidRef<'_>,
        peel: Option<OidRef<'_>>,
    ) -> Result<UnattachedReferenceBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let peel = peel.map_or(ptr::null(), |peel| peel.as_ptr());
        // SAFETY: `name` is NUL-terminated and `target` a live `git_oid`, both non-NULL
        // as libgit2 asserts, and `peel` NULL or a live `git_oid`; all are copied for
        // the call only. The result is NULL or a fresh reference the caller owns. The
        // runtime is pinned, so a failure is reported on live thread-local state.
        let reference = unsafe { ffi::git_reference__alloc(name.as_ptr(), target.as_ptr(), peel) };
        // SAFETY: NULL, or a fresh reference with no database that nothing else
        // references, released by `git_reference_free` as `ReferenceFree` does.
        unsafe { UnattachedReferenceBox::from_c(reference) }.ok_or_else(last)
    }

    /// A new symbolic reference named `name`, pointing at the reference named
    /// `target`, as a backend reads one from storage. libgit2 copies both names without
    /// validating either.
    ///
    /// Wraps: `git_reference__alloc_symbolic`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's out-of-memory error, when libgit2 cannot allocate it or
    /// its copy of `target`.
    pub fn symbolic(name: &CStr, target: &CStr) -> Result<UnattachedReferenceBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        // SAFETY: `name` and `target` are NUL-terminated and non-NULL, as libgit2
        // asserts, and copied for the call only. The result is NULL (the struct freed
        // when the target's copy fails) or a fresh reference the caller owns. The
        // runtime is pinned, so a failure is reported on live thread-local state.
        let reference =
            unsafe { ffi::git_reference__alloc_symbolic(name.as_ptr(), target.as_ptr()) };
        // SAFETY: NULL, or a fresh reference with no database that nothing else
        // references, released by `git_reference_free` as `ReferenceFree` does.
        unsafe { UnattachedReferenceBox::from_c(reference) }.ok_or_else(last)
    }
}

#[cfg(test)]
mod unit_tests {
    use core::ffi::c_void;
    use core::mem::size_of;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering::SeqCst;

    use super::*;
    use crate::libgit2::oid::oid::{Oid, OidType};
    use crate::libgit2::util::errors::ErrorClass;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process, where nothing has initialised libgit2.
    fn isolated(test: &str, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Allocations [`Countdown`] still grants before refusing every one.
    static GRANTS: AtomicUsize = AtomicUsize::new(usize::MAX);

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// libc's allocator, refusing every request once [`GRANTS`] runs out.
    struct Countdown;

    impl Countdown {
        /// Takes one grant; the children that install it allocate on one thread.
        fn grant() -> bool {
            match GRANTS.load(SeqCst) {
                usize::MAX => true,
                0 => false,
                n => {
                    GRANTS.store(n - 1, SeqCst);
                    true
                }
            }
        }
    }

    // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
    unsafe impl Alloc for Countdown {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            if !Self::grant() {
                return None;
            }
            // SAFETY: plain libc allocation.
            NonNull::new(unsafe { malloc(len) }.cast())
        }

        unsafe fn realloc(
            p: Option<NonNull<u8>>,
            len: usize,
            _: Option<&CStr>,
            _: i32,
        ) -> Option<NonNull<u8>> {
            if !Self::grant() {
                return None;
            }
            let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
            // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
            NonNull::new(unsafe { realloc(p, len) }.cast())
        }

        unsafe fn free(p: NonNull<u8>) {
            // SAFETY: `p` is a live libc block the caller gives up.
            unsafe { free(p.as_ptr().cast()) }
        }
    }

    /// Installs [`Countdown`] as libgit2's allocator, before anything is allocated.
    fn install_countdown() {
        let allocator = Allocator::new::<Countdown>();
        // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with every
        // member set; libgit2 copies it before anything is allocated.
        let rc = unsafe {
            ffi::git_libgit2_opts(
                ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
    }

    /// A fresh bare repository in a temporary directory, freed and removed on drop;
    /// stands in for the `git_repository` wrapper.
    struct TempRepo {
        path: PathBuf,
        repo: *mut ffi::git_repository,
    }

    impl TempRepo {
        fn new(tag: &str) -> Self {
            assert!(runtime_pinned());
            let path = std::env::temp_dir()
                .join(format!("crustify-refs-unit-{}-{tag}", std::process::id()));
            drop(std::fs::remove_dir_all(&path));
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
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
            let mut repo = ptr::null_mut();
            // SAFETY: `repo` is a live slot, `c_path` NUL-terminated and `opts` a live
            // options struct, all read for the call only.
            let rc = unsafe {
                ffi::git_repository_init_ext(&raw mut repo, c_path.as_ptr(), &raw mut opts)
            };
            assert_eq!(rc, 0, "{:?}", last());
            Self { path, repo }
        }

        /// `git_reference_lookup(name)`, adopted as an owned reference borrowing the
        /// repository.
        fn lookup(&self, name: &CStr) -> ReferenceBox<'_> {
            let mut out = ptr::null_mut();
            // SAFETY: `out` is a live slot, `self.repo` live and `name` NUL-terminated,
            // read for the call only.
            let rc = unsafe { ffi::git_reference_lookup(&raw mut out, self.repo, name.as_ptr()) };
            assert_eq!(rc, 0, "{:?}", last());
            // SAFETY: a fresh attached reference the caller owns, released by
            // `git_reference_free`; it borrows `self`, which frees the repository.
            unsafe { ReferenceBox::from_c(out) }.unwrap()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            // SAFETY: the live repository `new` produced, freed once; every reference
            // read from it borrowed `self` and is gone.
            unsafe { ffi::git_repository_free(self.repo) };
            drop(std::fs::remove_dir_all(&self.path));
        }
    }

    fn name(reference: *const ffi::git_reference) -> CString {
        // SAFETY: `reference` is a live reference; its NUL-terminated name lives as
        // long as it.
        let reference_name = unsafe { ffi::git_reference_name(reference) };
        // SAFETY: as above.
        unsafe { CStr::from_ptr(reference_name) }.to_owned()
    }

    fn symbolic_target(reference: *const ffi::git_reference) -> Option<CString> {
        // SAFETY: `reference` is a live reference; the target lives as long as it.
        let target = unsafe { ffi::git_reference_symbolic_target(reference) };
        // SAFETY: NULL, or the reference's NUL-terminated target.
        (!target.is_null()).then(|| unsafe { CStr::from_ptr(target) }.to_owned())
    }

    fn sha1(byte: u8) -> Oid {
        Oid::from_raw(&[byte; 20], OidType::Sha1).unwrap()
    }

    #[test]
    fn layout_matches_c() {
        let pointer = size_of::<*mut ffi::git_reference>();
        assert_eq!(size_of::<Reference<'_>>(), size_of::<ffi::git_reference>());
        assert_eq!(size_of::<ReferenceRef<'_, '_>>(), pointer);
        assert_eq!(size_of::<ReferenceMut<'_, '_>>(), pointer);
        assert_eq!(size_of::<ReferenceBox<'_>>(), pointer);
        assert_eq!(size_of::<Option<ReferenceBox<'_>>>(), pointer);
        assert_eq!(size_of::<UnattachedReferenceRef<'_>>(), pointer);
        assert_eq!(size_of::<UnattachedReferenceMut<'_>>(), pointer);
        assert_eq!(size_of::<UnattachedReferenceBox>(), pointer);
        assert_eq!(size_of::<Option<UnattachedReferenceBox>>(), pointer);
    }

    #[test]
    fn null_pointers_borrow_nothing() {
        // SAFETY: NULL, which `from_ptr` accepts.
        assert!(unsafe { ReferenceRef::from_ptr(ptr::null()) }.is_none());
        // SAFETY: as above.
        assert!(unsafe { ReferenceMut::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: as above.
        assert!(unsafe { UnattachedReferenceBox::from_c(ptr::null_mut()) }.is_none());
        // SAFETY: as above.
        assert!(unsafe { ReferenceBox::from_c(ptr::null_mut()) }.is_none());
    }

    /// A borrowed handle over a reference the caller keeps reaches the same object as
    /// its owner's, and reborrows keep it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn handles_from_raw_pointers_reach_the_reference() {
        let repo = TempRepo::new("handles");
        let mut head = repo.lookup(c"HEAD");
        let raw = head.as_c_ptr();
        // SAFETY: `raw` is `head`'s live attached reference, read but not written or
        // freed while the handle lives; its repository outlives `repo`'s borrow.
        let shared = unsafe { ReferenceRef::from_ptr(raw) }.unwrap();
        assert_eq!(shared.as_ptr(), raw.cast_const());
        assert_eq!(head.as_ref().as_ptr(), raw.cast_const());
        assert_eq!(name(shared.as_ptr()).as_c_str(), c"HEAD");
        assert_eq!(format!("{shared:?}"), "Reference { .. }");
        // SAFETY: as above, and nothing else reaches `raw` while `exclusive` lives.
        let mut exclusive = unsafe { ReferenceMut::from_ptr(raw) }.unwrap();
        assert_eq!(exclusive.as_mut_ptr(), raw);
        assert_eq!(exclusive.as_mut().as_mut_ptr(), raw);
        assert_eq!(exclusive.as_ref().as_ptr(), raw.cast_const());
        assert_eq!(format!("{exclusive:?}"), "Reference { .. }");
        assert_eq!(head.as_mut().as_mut_ptr(), raw);
    }

    /// `clone` is `git_reference_dup`: a new allocation with the same name and
    /// target, attached to the same repository, which outlives the original.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn clones_are_independent_copies() {
        let repo = TempRepo::new("clone");
        let head = repo.lookup(c"HEAD");
        let copy = head.clone();
        let copy2 = copy.try_clone().unwrap();
        let (h, c, c2) = (head.as_c_ptr(), copy.as_c_ptr(), copy2.as_c_ptr());
        assert_ne!(h, c);
        assert_ne!(c, c2);
        for p in [c, c2] {
            assert_eq!(name(p).as_c_str(), c"HEAD");
            assert_eq!(symbolic_target(p), symbolic_target(h));
            // SAFETY: both are live references.
            assert_eq!(unsafe { ffi::git_reference_cmp(h, p) }, 0);
            // SAFETY: both are live attached references of `repo`.
            let reference_owner = unsafe { ffi::git_reference_owner(p) };
            // SAFETY: as above.
            let owners = unsafe { (reference_owner, ffi::git_reference_owner(h)) };
            assert_eq!(owners.0, owners.1);
        }
        drop(head);
        drop(copy);
        // The last copy keeps its database: it still reaches the repository.
        // SAFETY: `c2` is live and attached.
        assert_eq!(unsafe { ffi::git_reference_owner(c2) }, repo.repo);
        assert_eq!(name(c2).as_c_str(), c"HEAD");
    }

    /// The raw seam: a reference `git_reference__alloc` built, adopted, surrendered
    /// and adopted again, is released once.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn unattached_raw_seam_round_trips() {
        assert!(runtime_pinned());
        let id = sha1(0x5A);
        // SAFETY: NUL-terminated name and a live id, copied; the result is a fresh
        // reference the caller owns.
        let raw = unsafe {
            ffi::git_reference__alloc(
                c"refs/heads/raw".as_ptr(),
                id.as_ref().as_ptr(),
                ptr::null(),
            )
        };
        // SAFETY: a fresh unattached reference, released by `git_reference_free`.
        let mut owned = unsafe { UnattachedReferenceBox::from_c(raw) }.unwrap();
        assert_eq!(owned.as_c_ptr(), raw);
        assert_eq!(owned.as_ref().as_ptr(), raw.cast_const());
        assert_eq!(owned.as_mut().as_mut_ptr(), raw);
        assert_eq!(
            format!("{:?}", owned.as_ref()),
            "UnattachedReference { .. }"
        );
        assert_eq!(
            format!("{:?}", owned.as_mut()),
            "UnattachedReference { .. }"
        );
        let surrendered = owned.into_c();
        assert_eq!(surrendered, raw);
        // SAFETY: the pointer `into_c` gave up, adopted once more.
        let owned = unsafe { UnattachedReferenceBox::from_c(surrendered) }.unwrap();
        assert_eq!(name(owned.as_ref().as_ptr()).as_c_str(), c"refs/heads/raw");
    }

    /// Each constructor builds what its C routine does.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructors_fill_the_reference() {
        let id = sha1(0x11);
        let peeled = sha1(0x22);
        let direct =
            UnattachedReference::direct(c"refs/tags/v1", id.as_ref(), Some(peeled.as_ref()))
                .unwrap();
        let p = direct.as_ref().as_ptr();
        assert_eq!(name(p).as_c_str(), c"refs/tags/v1");
        // SAFETY: `p` is live.
        let kind = unsafe { ffi::git_reference_type(p) };
        assert_eq!(kind, ffi::GIT_REFERENCE_DIRECT);
        // SAFETY: `p` is live and direct.
        let target = unsafe { ffi::git_reference_target(p) };
        // SAFETY: as above.
        let peel = unsafe { ffi::git_reference_target_peel(p) };
        // SAFETY: the ids live as long as `p`.
        let target = unsafe { *target };
        // SAFETY: as above.
        let peel = unsafe { *peel };
        assert_eq!(target.id, id.as_ref().id());
        assert_eq!(peel.id, peeled.as_ref().id());
        assert_eq!(symbolic_target(p), None);

        let symbolic = UnattachedReference::symbolic(c"HEAD", c"refs/heads/main").unwrap();
        let p = symbolic.as_ref().as_ptr();
        assert_eq!(name(p).as_c_str(), c"HEAD");
        // SAFETY: `p` is live.
        let kind = unsafe { ffi::git_reference_type(p) };
        assert_eq!(kind, ffi::GIT_REFERENCE_SYMBOLIC);
        assert_eq!(symbolic_target(p).as_deref(), Some(c"refs/heads/main"));
    }

    /// Every allocation failure is reported as libgit2's out-of-memory error, and a
    /// failed copy leaves the original usable.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn allocation_failures_are_reported() {
        isolated(
            concat!(module_path!(), "::allocation_failures_are_reported"),
            || {
                install_countdown();
                let repo = TempRepo::new("oom");
                let head = repo.lookup(c"HEAD");
                let id = sha1(0x33);
                let oom = |e: ErrorBuf| {
                    assert_eq!(e.class(), Some(ErrorClass::NoMemory));
                    assert_eq!(e.message(), c"Out of memory");
                };

                // The struct, then the target's copy.
                for grants in [0, 1] {
                    GRANTS.store(grants, SeqCst);
                    assert!(head.try_clone().is_none());
                    GRANTS.store(usize::MAX, SeqCst);
                    oom(last());
                }
                GRANTS.store(0, SeqCst);
                let e = UnattachedReference::direct(c"refs/heads/x", id.as_ref(), None);
                GRANTS.store(usize::MAX, SeqCst);
                oom(e.unwrap_err());
                for grants in [0, 1] {
                    GRANTS.store(grants, SeqCst);
                    let e = UnattachedReference::symbolic(c"HEAD", c"refs/heads/x");
                    GRANTS.store(usize::MAX, SeqCst);
                    oom(e.unwrap_err());
                }

                // The original survives its failed copies.
                let copy = head.try_clone().unwrap();
                assert_eq!(name(copy.as_ref().as_ptr()).as_c_str(), c"HEAD");
                drop(head);
                drop(copy);
            },
        );
    }

    /// When libgit2 cannot be initialised, the constructors report C's static "not
    /// initialized" error without calling into it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructors_without_runtime_fail_cleanly() {
        isolated(
            concat!(
                module_path!(),
                "::constructors_without_runtime_fail_cleanly"
            ),
            || {
                install_countdown();
                GRANTS.store(0, SeqCst);
                let id = Oid::zero(OidType::Sha1);
                let errors = [
                    UnattachedReference::direct(c"refs/heads/x", id.as_ref(), None).unwrap_err(),
                    UnattachedReference::symbolic(c"HEAD", c"refs/heads/x").unwrap_err(),
                ];
                GRANTS.store(usize::MAX, SeqCst);
                assert!(!runtime_pinned(), "libgit2 could not initialise");
                for e in errors {
                    assert_eq!(e.class(), Some(ErrorClass::Invalid));
                    assert_eq!(e.message(), c"library has not been initialized");
                }
            },
        );
    }
}
