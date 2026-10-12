//! Safe wrappers over `include/git2/odb.h`.
//!
//! An [`Odb`] is a `git_odb`: an object database, which reads and writes git objects
//! through a sorted list of backends (loose files, packfiles, alternates, or custom
//! ones). The C struct is private to libgit2; Rust reaches it only behind a pointer.
//!
//! # Ownership
//!
//! An odb is reference counted, and shared: libgit2 creates it with one reference,
//! `git_odb_free` releases one, and a repository given the odb (`git_repository_set_odb`,
//! `git_repository_wrap_odb`) takes a reference of its own, as does every caller of
//! `git_repository_odb`. The object is freed, with its backends, once no reference is
//! left and no repository owns it. libgit2 exports no routine that adds a reference, so
//! the Rust owner, [`OdbArc`], is one counted reference that does not clone; share one
//! with [`std::sync::Arc`] instead.
//!
//! # The owning repository
//!
//! A repository that is given an odb becomes its *owner* (`rc.owner`, set by
//! `git_repository_set_odb`, `git_repository_wrap_odb` and the repository's own lazy
//! `git_repository_odb`), and from then on every read and write of the odb goes
//! through the owner's object cache: `odb_cache()` in `odb.c` follows the owner
//! pointer, without a reference, to `&owner->objects`. `git_repository_free` clears
//! the pointer before releasing its reference, but an odb routine that loaded it just
//! before is left reading a freed repository. So an odb a repository owns must not
//! be used past that repository: an [`Odb<'r>`] borrows its owning repository for
//! `'r`, which every owner and handle carries, and an odb no repository owns is
//! `Odb<'static>`, as the constructors build it.
//!
//! The wrappers of the routines that transfer an odb to a repository must keep that
//! invariant: `git_repository_odb` returns an `OdbArc<'r>` of the repository's borrow;
//! `git_repository_set_odb` and `git_repository_wrap_odb` consume an `OdbArc<'static>`
//! (the only proof that no repository owns the odb yet, which libgit2 does not check:
//! a second owner would leave the first one reading a cache it does not hold), and
//! may hand it back shortened to the repository's `'r`. The lifetime only shrinks,
//! as `&'r T`'s does.
//!
//! # Threads
//!
//! libgit2 locks the odb itself (`docs/threading.md`: "The `git_odb` object uses
//! locking internally, and is thread-safe to use from multiple threads
//! simultaneously"): its backends and commit-graph change only under its mutex, its
//! own object cache and an owner's have their own read-write locks, and its options
//! are fixed once it is built. Every routine over an odb therefore takes the shared
//! handle, [`OdbRef`], which an [`OdbArc`] lends through `as_ref()`; nothing hands out
//! an [`OdbMut`], since a repository holding the odb may use it at any time.
//!
//! The constructors pin libgit2's runtime first (`runtime_pinned`), since they
//! allocate, and return the thread's libgit2 error, copied out, as an [`ErrorBuf`].

use core::ffi::{CStr, c_int};
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use ffibox::{CArc, CBorrowedPtr, CCell, CDrop, CNew};

use crate::ffi;
use crate::libgit2::oid::oid::OidType;
use crate::libgit2::util::errors::{ErrorBuf, last};
use crate::libgit2::util::sys::errors::runtime_pinned;

/// An object database (`git_odb`): the layout type named by [`OdbArc`] and the
/// handles, never held by value.
///
/// Build one with [`new`](Self::new) (no backends), [`open`](Self::open) (the loose
/// and packed backends of an `objects` directory, and its alternates), or their
/// `_with_oid_type` forms, which choose the object id type the database hashes
/// with. Each is `Odb<'static>`: no repository owns it. `'r` is the borrow of the
/// repository that owns the odb, through whose object cache the odb then works (see
/// the [module docs](self#the-owning-repository)); an owner or handle of such an odb
/// cannot outlive it. The lifetime only shrinks:
///
/// ```compile_fail
/// use libgit2::libgit2::odb::odb::OdbArc;
///
/// fn widen<'r>(odb: OdbArc<'r>) -> OdbArc<'static> {
///     odb
/// }
/// ```
///
/// and the exclusive handle is invariant in it, as `&mut` is in its pointee:
///
/// ```compile_fail
/// use libgit2::libgit2::odb::odb::OdbMut;
///
/// fn shrink<'a, 'r>(m: OdbMut<'a, 'static>) -> OdbMut<'a, 'r> {
///     m
/// }
/// ```
///
/// Wraps: git_odb
/// Handwritten
#[repr(transparent)]
pub struct Odb<'r>(
    ffi::git_odb,
    PhantomData<(*const (), &'r ffi::git_repository)>,
);

/// Shared borrow of an [`Odb`]. `Copy`, like `&T`; every odb routine takes it.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct OdbRef<'a, 'r>(CBorrowedPtr<'a, Odb<'r>>);

/// Exclusive borrow of an [`Odb`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref); nothing lends one, since a repository may hold the odb.
///
/// Handwritten
#[repr(transparent)]
pub struct OdbMut<'a, 'r>(
    OdbRef<'a, 'r>,
    // Invariant in the odb, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Odb<'r>>,
);

// SAFETY: `Odb` is `#[repr(transparent)]` over `git_odb` with a zero-sized marker; both
// handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Odb<'r>>` (`OdbMut` through
// `OdbRef`) with zero-sized markers and no `Drop`, hand out no reference to the odb, and
// `OdbRef` has no write operation. `OdbMut` is invariant in the odb through its
// `PhantomData<&'a mut Odb<'r>>`.
unsafe impl<'r> CCell for Odb<'r> {
    type C = ffi::git_odb;
    type Ref<'a>
        = OdbRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = OdbMut<'a, 'r>
    where
        Self: 'a;
}

// SAFETY: an odb has no thread affinity: libgit2 frees it, its backends and its mutex
// from whichever thread drops the last reference, and its reference count and owner
// are atomics. Every routine that changes it takes its mutex (backends, commit-graph)
// or a cache's rwlock, and the fields read unlocked (options, `do_fsync`) are written
// only before the odb is published. The one object an odb reaches outside itself is
// its owning repository's object cache (`odb_cache()`), read and written under that
// cache's rwlock: the repository outlives `'r`, which every owner and handle of the
// odb carries, so it is live for every such access.
unsafe impl Send for Odb<'_> {}
// SAFETY: as above: shared access from several threads is what libgit2's own locking
// serialises (`docs/threading.md`), and no Rust-side state lives in the object.
unsafe impl Sync for Odb<'_> {}
// SAFETY: `OdbRef` is a shared borrow, as `&Odb`, and `Odb` is `Sync`.
unsafe impl Send for OdbRef<'_, '_> {}
// SAFETY: as above.
unsafe impl Sync for OdbRef<'_, '_> {}
// SAFETY: `OdbMut` is an exclusive borrow, as `&mut Odb`, and `Odb` is `Send`.
unsafe impl Send for OdbMut<'_, '_> {}
// SAFETY: `&OdbMut` only reaches the shared handle, and `Odb` is `Sync`.
unsafe impl Sync for OdbMut<'_, '_> {}

impl<'a, 'r> OdbRef<'a, 'r> {
    /// Borrows the odb at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live odb that stays live for `'a`, owned by no
    /// repository or by one that outlives `'r`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_odb) -> Option<Self> {
        // SAFETY: this function's contract is `CCell::ref_from_ptr`'s: `ptr` is NULL or a
        // live `git_odb` that outlives `'a` and is not written meanwhile.
        unsafe { <Odb<'r> as CCell>::ref_from_ptr(ptr) }
    }

    /// Pointer to the C object, for the routines that take a `git_odb *` and lock it
    /// themselves.
    #[allow(dead_code)]
    pub(crate) fn as_ptr(&self) -> *mut ffi::git_odb {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'a, 'r> OdbMut<'a, 'r> {
    /// Writable pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_odb {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> OdbRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> OdbMut<'_, 'r> {
        OdbMut(self.0, PhantomData)
    }
}

impl fmt::Debug for OdbRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OdbRef").field(&self.as_ptr()).finish()
    }
}

impl fmt::Debug for OdbMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases one reference to an odb through `git_odb_free`; the last one frees the
/// odb and its backends, unless a repository still owns it. Stateless. Also builds an
/// odb no repository owns through `git_odb_new`, so [`OdbArc::new`](CArc::new) makes
/// the odb [`Odb::new`] makes, reporting a failure as `None`.
///
/// Wraps: git_odb_free
/// Wraps: git_odb_new
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbFree;

// SAFETY: `git_odb_free` drops exactly the one reference the owner holds: the odb is
// freed once, when its count reaches zero and no repository owns it. It needs no state
// beyond the pointer, and reaches no repository.
unsafe impl<'r> CDrop<Odb<'r>> for OdbFree {
    unsafe fn c_drop(&self, ptr: NonNull<Odb<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live odb whose reference
        // the caller gives up; `Odb` is transparent over `git_odb`.
        unsafe { ffi::git_odb_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `git_odb_new` writes a fresh odb with one reference, no backends and no
// owning repository (so `'static`), that the caller owns and `git_odb_free` releases;
// it leaves the slot NULL when it fails. The runtime is pinned before the call.
unsafe impl CNew<Odb<'static>> for OdbFree {
    fn c_new(&self) -> Option<NonNull<Odb<'static>>> {
        if !runtime_pinned() {
            return None;
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; the runtime is pinned, so a failure is
        // reported on live thread-local state.
        let rc = unsafe { ffi::git_odb_new(&mut out) };
        if rc < 0 {
            return None;
        }
        NonNull::new(out.cast())
    }
}

/// One counted reference to an object database, released through [`OdbFree`].
///
/// It lends the shared handle through `as_ref()`, which every odb routine takes. It
/// does not clone (libgit2 exports no up-ref) and lends no [`OdbMut`]: a repository may
/// hold another reference. The constructors on [`Odb`] build one safely; a reference
/// libgit2 hands out (`git_repository_odb`) is adopted by the wrapper of the routine
/// that returns it, with the repository's borrow as `'r`.
///
/// `'static` means no repository owns the odb (see the
/// [module docs](self#the-owning-repository)); `OdbArc<'r>` borrows the owning
/// repository, so neither it nor a handle it lends outlives the repository:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::odb::odb::OdbArc;
/// use libgit2::libgit2::repository::repository::{Repository, RepositoryRef};
///
/// // What the wrapper of `git_repository_odb` returns.
/// fn odb_of<'r>(repo: RepositoryRef<'r>) -> OdbArc<'r> {
///     unimplemented!()
/// }
///
/// let repo = Repository::new().unwrap();
/// let odb = odb_of(repo.as_ref());
/// drop(repo);
/// drop(odb);
/// ```
///
/// ```
/// use libgit2::libgit2::odb::odb::Odb;
///
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// let odb = Odb::new().unwrap();
/// let shared = std::sync::Arc::new(odb);
/// let other = std::sync::Arc::clone(&shared);
/// std::thread::spawn(move || drop(other)).join().unwrap();
/// let _handle = shared.as_ref();
/// ```
///
/// A handle borrows the reference it came from:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::odb::odb::Odb;
///
/// let handle = {
///     let odb = Odb::new().unwrap();
///     odb.as_ref()
/// };
/// let _ = handle;
/// ```
///
/// It neither clones nor lends an exclusive handle:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::odb::odb::Odb;
///
/// let odb = Odb::new().unwrap();
/// let _again = odb.clone();
/// ```
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::odb::odb::Odb;
///
/// let mut odb = Odb::new().unwrap();
/// let _exclusive = odb.get_mut();
/// ```
pub type OdbArc<'r> = CArc<Odb<'r>, OdbFree>;

/// The options `git_odb_new_ext` / `git_odb_open_ext` read: this version's struct with
/// `oid_type`, the only setting it has.
fn options(oid_type: OidType) -> ffi::git_odb_options {
    ffi::git_odb_options {
        version: ffi::GIT_ODB_OPTIONS_VERSION,
        oid_type: oid_type.raw(),
    }
}

/// Runs a constructor that stores a new reference in its out slot, and adopts it.
///
/// Pins the runtime first; a constructor that fails leaves the slot NULL (or untouched,
/// and so NULL) and the thread's libgit2 error set.
fn construct(
    fill: impl FnOnce(&mut *mut ffi::git_odb) -> c_int,
) -> Result<OdbArc<'static>, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut out = ptr::null_mut();
    if fill(&mut out) < 0 {
        return Err(last());
    }
    // SAFETY: the constructor succeeded, so `out` is the one reference it created, to
    // an odb of libgit2's allocation that no repository owns (`'static`) and that
    // `git_odb_free` releases; the caller gets it.
    let odb = unsafe { OdbArc::from_c(out) };
    Ok(odb.expect("a libgit2 odb constructor succeeded without an odb"))
}

impl Odb<'static> {
    /// A new object database with no backends, which hashes SHA-1 ids.
    ///
    /// Nothing can be read from it or written to it until a backend is added. Fails,
    /// with libgit2's error, when the odb cannot be allocated.
    ///
    /// Wraps: git_odb_new
    pub fn new() -> Result<OdbArc<'static>, ErrorBuf> {
        construct(|out| {
            // SAFETY: `out` is a writable slot, which the routine fills with a fresh
            // reference on success. The runtime is pinned.
            unsafe { ffi::git_odb_new(out) }
        })
    }

    /// A new object database with no backends, which hashes ids of `oid_type`.
    ///
    /// As [`new`](Self::new), which is this with [`OidType::Sha1`]; the object id type
    /// is the only setting `git_odb_options` holds.
    ///
    /// Wraps: git_odb_new_ext
    pub fn with_oid_type(oid_type: OidType) -> Result<OdbArc<'static>, ErrorBuf> {
        let opts = options(oid_type);
        construct(|out| {
            // SAFETY: `out` is a writable slot, filled with a fresh reference on
            // success; `opts` is a live options struct of this version, which the
            // routine copies and does not retain. The runtime is pinned.
            unsafe { ffi::git_odb_new_ext(out, &opts) }
        })
    }

    /// An object database over the `objects` directory `objects_dir`, which hashes
    /// SHA-1 ids: its loose objects and packfiles, its commit-graph, and the alternate
    /// directories its `info/alternates` names, and theirs, six levels deep, each
    /// read-only; a directory already open is skipped.
    ///
    /// The directory must exist, but need not hold anything; an alternate that does not
    /// exist is skipped. Fails, with libgit2's error, when `objects_dir` cannot be
    /// examined (`GIT_ERROR_ODB`, `failed to load object database in '<dir>'`), when an
    /// `info/alternates` file cannot be read, or when memory runs out.
    ///
    /// Wraps: git_odb_open
    pub fn open(objects_dir: &CStr) -> Result<OdbArc<'static>, ErrorBuf> {
        construct(|out| {
            // SAFETY: `out` is a writable slot, filled with a fresh reference on
            // success and set to NULL otherwise; `objects_dir` is NUL-terminated, read
            // and copied for the call only. The runtime is pinned.
            unsafe { ffi::git_odb_open(out, objects_dir.as_ptr()) }
        })
    }

    /// An object database over the `objects` directory `objects_dir`, which hashes ids
    /// of `oid_type`.
    ///
    /// As [`open`](Self::open), which is this with [`OidType::Sha1`]; the backends read
    /// and write ids of `oid_type`.
    ///
    /// Wraps: git_odb_open_ext
    pub fn open_with_oid_type(
        objects_dir: &CStr,
        oid_type: OidType,
    ) -> Result<OdbArc<'static>, ErrorBuf> {
        let opts = options(oid_type);
        construct(|out| {
            // SAFETY: as in `open`; `opts` is a live options struct of this version,
            // which the routine copies and does not retain.
            unsafe { ffi::git_odb_open_ext(out, objects_dir.as_ptr(), &opts) }
        })
    }
}

// The layout type is the C type's size, so an odb pointer converts both ways, and the
// owner is pointer-sized.
const _: () = assert!(size_of::<Odb<'static>>() == size_of::<ffi::git_odb>());
const _: () = assert!(size_of::<OdbArc<'static>>() == size_of::<*mut ffi::git_odb>());
const _: () = assert!(size_of::<Option<OdbArc<'static>>>() == size_of::<*mut ffi::git_odb>());

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::util::errors::ErrorClass;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use crate::libgit2::util::sys::errors::set;
    use core::ffi::c_void;
    use core::ptr::NonNull;
    use std::process::Command;
    use std::sync::atomic::{AtomicIsize, Ordering::SeqCst};

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

    /// `git_odb_num_backends` on a live odb.
    fn backends(odb: *mut ffi::git_odb) -> usize {
        // SAFETY: `odb` is a live odb the caller holds a reference to; the routine
        // takes its lock and reads the backend count.
        unsafe { ffi::git_odb_num_backends(odb) }
    }

    #[test]
    fn options_carry_this_version_and_the_type() {
        for kind in [OidType::Sha1, OidType::Sha256] {
            let opts = options(kind);
            assert_eq!(opts.version, ffi::GIT_ODB_OPTIONS_VERSION);
            assert_eq!(OidType::from_raw(opts.oid_type), Some(kind));
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn from_c_adopts_one_reference_and_refuses_null() {
        assert!(runtime_pinned());
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` is a writable slot, filled with a fresh reference on success.
        assert_eq!(unsafe { ffi::git_odb_new(&mut raw) }, 0);
        // SAFETY: `raw` is the one reference `git_odb_new` created, given up here.
        let odb = unsafe { OdbArc::from_c(raw) }.unwrap();
        assert_eq!(odb.as_c_ptr(), raw);
        assert_eq!(backends(odb.as_c_ptr()), 0);
        assert!(format!("{:?}", odb.as_ref()).starts_with("OdbRef("));
        drop(odb);
        // SAFETY: NULL addresses nothing; `from_c` refuses it.
        assert!(unsafe { OdbArc::from_c(ptr::null_mut()) }.is_none());
    }

    /// The policy's constructor builds the odb `Odb::new` builds, owned by no
    /// repository, and the handles' raw seams round-trip its pointer.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn policy_constructor_and_handle_seams() {
        let odb = OdbArc::new().unwrap();
        assert_eq!(backends(odb.as_c_ptr()), 0);
        // SAFETY: the arc's live odb, which outlives the handle and is owned by no
        // repository.
        let r = unsafe { OdbRef::from_ptr(odb.as_c_ptr()) }.unwrap();
        assert_eq!(r.as_ptr(), odb.as_c_ptr());
        // SAFETY: NULL.
        assert!(unsafe { OdbRef::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: the arc's live odb, reached through no other handle meanwhile.
        let shared = unsafe { OdbRef::<'_, 'static>::from_ptr(odb.as_c_ptr()) };
        let mut m = OdbMut(shared.unwrap(), PhantomData);
        assert_eq!(m.as_mut_ptr(), odb.as_c_ptr());
        assert_eq!(m.as_mut().as_ref().as_ptr(), odb.as_c_ptr());
        assert_eq!(format!("{m:?}"), format!("{:?}", odb.as_ref()));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn into_c_gives_the_reference_back() {
        let odb = Odb::new().unwrap();
        let raw = odb.into_c();
        assert_eq!(backends(raw), 0);
        // SAFETY: `raw` is the reference `into_c` surrendered, released once.
        unsafe { ffi::git_odb_free(raw) };
    }

    /// A repository given the odb holds a reference of its own: dropping the Rust one
    /// leaves the odb live for the repository, which frees it last.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_repository_keeps_its_own_reference() {
        let odb = Odb::new().unwrap();
        let raw = odb.as_c_ptr();
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a writable slot; `raw` is a live odb, which the new
        // repository takes a reference to and owns.
        assert_eq!(unsafe { ffi::git_repository_wrap_odb(&mut repo, raw) }, 0);
        drop(odb);
        let mut again = ptr::null_mut();
        // SAFETY: `repo` is live; the result is a new reference to its odb.
        assert_eq!(unsafe { ffi::git_repository_odb(&mut again, repo) }, 0);
        assert_eq!(again, raw);
        // SAFETY: `again` is the reference just taken, adopted once.
        let again = unsafe { OdbArc::from_c(again) }.unwrap();
        assert_eq!(backends(again.as_c_ptr()), 0);
        // SAFETY: `repo` is the live repository `git_repository_wrap_odb` made, freed
        // once; `again` keeps the odb alive past it.
        unsafe { ffi::git_repository_free(repo) };
        assert_eq!(backends(again.as_c_ptr()), 0);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn construct_returns_the_failure_and_adopts_nothing() {
        let err = construct(|out| {
            set(ErrorClass::Odb, c"refused by the probe");
            assert!(out.is_null());
            -1
        })
        .unwrap_err();
        assert_eq!(err.class(), Some(ErrorClass::Odb));
        assert_eq!(err.message(), c"refused by the probe");
    }

    /// The first constructor called in a process initialises libgit2 for itself.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructors_pin_the_runtime_first() {
        isolated(
            concat!(module_path!(), "::constructors_pin_the_runtime_first"),
            || {
                let odb = Odb::with_oid_type(OidType::Sha1).unwrap();
                assert_eq!(backends(odb.as_c_ptr()), 0);
            },
        );
    }

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// Allocations left before [`Countdown`] refuses one; negative while disarmed.
    static LEFT: AtomicIsize = AtomicIsize::new(-1);

    /// libc's allocator, refusing the request [`LEFT`] counts down to.
    struct Countdown;

    impl Countdown {
        /// Whether to serve a request: always while disarmed, else while the count
        /// lasts, each grant taking one; once it reaches 0 every request is refused.
        fn grant() -> bool {
            let mut left = LEFT.load(SeqCst);
            loop {
                match left {
                    ..0 => return true,
                    0 => return false,
                    _ => match LEFT.compare_exchange(left, left - 1, SeqCst, SeqCst) {
                        Ok(_) => return true,
                        Err(now) => left = now,
                    },
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
        // member set; libgit2 copies it before anything is allocated, and it is libc's
        // family, as the default one is.
        let rc = unsafe {
            ffi::git_libgit2_opts(
                ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
    }

    /// When libgit2 cannot be initialised, every constructor fails without calling it,
    /// with the error C reports before initialisation.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructors_fail_when_libgit2_cannot_start() {
        isolated(
            concat!(
                module_path!(),
                "::constructors_fail_when_libgit2_cannot_start"
            ),
            || {
                LEFT.store(0, SeqCst);
                install_countdown();
                let dir = c"/nonexistent/objects";
                let results = [
                    Odb::new(),
                    Odb::with_oid_type(OidType::Sha256),
                    Odb::open(dir),
                    Odb::open_with_oid_type(dir, OidType::Sha1),
                ];
                for result in results {
                    let err = result.unwrap_err();
                    assert_eq!(err.class(), Some(ErrorClass::Invalid));
                    assert_eq!(err.message(), c"library has not been initialized");
                }
                assert!(OdbArc::new().is_none());
            },
        );
    }

    /// Every allocation `git_odb_new_ext` makes fails in turn: each failure is an error
    /// return with nothing adopted and nothing leaked (LSan), until enough succeed.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn allocation_failures_fail_cleanly() {
        isolated(
            concat!(module_path!(), "::allocation_failures_fail_cleanly"),
            || {
                install_countdown();
                assert!(runtime_pinned());
                // The thread's error state is allocated on its first error: allocate it
                // now, so a refusal below reports itself rather than that.
                set(ErrorClass::Odb, c"warm up the thread's error state");
                let constructors: [fn() -> Result<OdbArc<'static>, ErrorBuf>; 3] = [
                    Odb::new,
                    || Odb::with_oid_type(OidType::Sha1),
                    || Odb::with_oid_type(OidType::Sha256),
                ];
                // The policy's constructor reports the first refusal as `None`.
                LEFT.store(0, SeqCst);
                let none = OdbArc::new();
                LEFT.store(-1, SeqCst);
                assert!(none.is_none());
                let mut needed = 0;
                for construct in constructors {
                    let mut refused = 0;
                    let odb = loop {
                        LEFT.store(refused, SeqCst);
                        let result = construct();
                        LEFT.store(-1, SeqCst);
                        match result {
                            Ok(odb) => break odb,
                            Err(err) => {
                                assert_eq!(err.class(), Some(ErrorClass::NoMemory), "{err:?}");
                                refused += 1;
                                assert!(refused < 64, "never succeeded");
                            }
                        }
                    };
                    assert!(refused >= 1, "the odb itself is allocated");
                    assert_eq!(backends(odb.as_c_ptr()), 0);
                    needed = refused;
                }
                // `open` builds the odb first; each of those allocations failing fails it
                // the same way. (A later one, inside the backends' setup, can leak the
                // backend just made: libgit2's `add_default_backends`.)
                let dir = std::env::temp_dir();
                let dir = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
                for refused in 0..needed {
                    LEFT.store(refused, SeqCst);
                    let result = Odb::open(&dir);
                    LEFT.store(-1, SeqCst);
                    let err = result.unwrap_err();
                    assert_eq!(err.class(), Some(ErrorClass::NoMemory), "{err:?}");
                }
            },
        );
    }
}
