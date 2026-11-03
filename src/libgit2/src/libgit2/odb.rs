//! `git2/sys/odb_backend.h`: the interface custom object-database backends implement.
//!
//! The header also publishes libgit2.so's only exported raw byte allocator,
//! `git_odb_backend_data_alloc` / `git_odb_backend_data_free`: a backend's `read`
//! callbacks return buffers from it, which libgit2 later releases with its own
//! `git__free`. Both routines ignore their `backend` argument (`GIT_UNUSED`) and
//! forward to `git__malloc` / `git__free`, so the policies below are stateless and
//! pass NULL for it.
//!
//! [`OdbDataFree`] owns one object ([`OdbDataBox`]), [`OdbDataArrayFree`] a counted
//! run of elements ([`OdbDataVec`]). Neither zeroes on its own: `git__malloc` is a
//! plain `malloc`, so zeroed constructors clear the storage in Rust, as `git__calloc`
//! does in C.
//!
//! The same pair settles libgit2's strings: `git_strarray_dispose`,
//! `git_signature_free` and `git_buf_dispose` release them with `git__free`, and
//! `git__strdup` fills `git__malloc` storage. [`OdbDataStrFree`] owns one
//! NUL-terminated string ([`OdbDataStr`]); [`OdbDataStrVec`] a counted run of them,
//! laid out as a `git_strarray`'s `strings` / `count`.
//!
//! Not every string libgit2 hands out is one of its allocations: a `git_buf` owns
//! its `ptr` only while `reserved > 0`, and otherwise points at a static empty
//! string that `git_buf_dispose` leaves alone (`buf.c`); an empty result is filled
//! without allocating. [`BufMut::take_str`] applies that rule, so a `git_buf`'s `ptr`
//! is never adopted through `from_raw` directly.
//!
//! [`BufMut::take_str`]: crate::libgit2::src::libgit2::buf::BufMut::take_str
//!
//! The first allocation pins libgit2's runtime with a `git_libgit2_init` reference
//! of its own, held for the rest of the process (`runtime_pinned`, from
//! `util::sys::errors`). C callers get NULL before initialisation instead, through a
//! path that corrupts foreign thread-local state; the strategies deliberately diverge
//! from it.
//!
//! Storage handed out by a custom `GIT_OPT_SET_ALLOCATOR` allocator is released
//! through whichever allocator is current when the owner drops, so the allocator must
//! not be replaced while an owner is live — the contract of that option, not of these
//! policies.
//!
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
//! Two routines break that locking, and are `unsafe` for it, their contract excluding
//! every other thread for the call: [`OdbRef::write_multi_pack_index`] walks the
//! backends, and the packed backend its packs, without the lock, and
//! [`OdbRef::clear_commit_graph`] frees a commit-graph whose file a revision walk reads
//! past the lock. No type of this crate proves that exclusion: an `OdbArc<'static>`
//! may still be one a leaked repository owns.
//!
//! # Reading and writing
//!
//! [`OdbRef::read`], [`read_prefix`](OdbRef::read_prefix),
//! [`read_header`](OdbRef::read_header), [`exists`](OdbRef::exists) and
//! [`exists_prefix`](OdbRef::exists_prefix) look an object up by id. libgit2 never checks
//! the id's type against the odb's: its loose backend formats the id into a path sized
//! for the odb's type, so a SHA-256 id in a SHA-1 odb writes past that buffer and the
//! converse leaves part of it unwritten. The wrappers learn the odb's type first (from
//! `git_odb_expand_ids`, which clears an entry it cannot expand to that type, without a
//! lookup), and refuse an id of another type before the call: the readers fail with
//! `GIT_ERROR_INVALID`, and the existence checks answer `false`.
//!
//! An object read is an [`OdbObjectArc`]: one counted reference to libgit2's
//! [`OdbObject`], which the odb's cache (or its owning repository's) may share, and
//! which outlives the odb. [`OdbRef::write`] hashes with the odb's type and returns the
//! id. The routines fail with an [`OdbError`], whose code tells a missing object
//! ([`ErrorCode::NotFound`]) from an ambiguous prefix ([`ErrorCode::Ambiguous`]).
//!
//! A walk over every object calls a [`ForeachCallback`] (`git_odb_foreach_cb`) with each
//! id.
//!
//! The constructors pin libgit2's runtime first (`runtime_pinned`), since they
//! allocate, and return the thread's libgit2 error, copied out, as an [`ErrorBuf`].
//!
//! # Plain values
//!
//! Two structs of the header own nothing and are held by value: [`OdbOptions`], the
//! options the `_ext` constructors read, and [`OdbExpandId`], one query of
//! `git_odb_expand_ids`, which a caller fills and the routine answers in place.

use core::any::Any;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::{ManuallyDrop, MaybeUninit, align_of, size_of};
use core::num::NonZero;
use core::ops::{BitOr, BitOrAssign, ControlFlow};
use core::ptr::{self, NonNull};
use std::panic::{self, AssertUnwindSafe};

use ffibox::{
    CAlloc, CAllocZeroed, CArc, CBorrowedPtr, CBox, CCell, CDrop, CMutSeam, CNew, CPlainElem,
    CRefClone, CRefSeam, CStrBox, CVec, CZeroable, define_ctype, impl_cdrop_str,
    impl_cdupclone_str, impl_clenclone, impl_clendrop,
};

use crate::ffi;
use crate::libgit2::src::libgit2::filter::StreamError;
use crate::libgit2::src::libgit2::indexer::IndexerProgressMut;
use crate::libgit2::src::libgit2::object::ObjectType;
use crate::libgit2::src::libgit2::oid::{Oid, OidMut, OidRef, OidType, UnnamedOidType};
use crate::libgit2::src::util::errors::{
    ErrorBuf, ErrorClass, ErrorCode, last, runtime_pinned, set,
};

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
/// use libgit2::libgit2::src::libgit2::odb::OdbArc;
///
/// fn widen<'r>(odb: OdbArc<'r>) -> OdbArc<'static> {
///     odb
/// }
/// ```
///
/// and the exclusive handle is invariant in it, as `&mut` is in its pointee:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::odb::OdbMut;
///
/// fn shrink<'a, 'r>(m: OdbMut<'a, 'static>) -> OdbMut<'a, 'r> {
///     m
/// }
/// ```
///
/// Wraps `ty::git_odb`
#[doc(alias = "git_odb")]
#[repr(transparent)]
pub struct Odb<'r>(
    ffi::git_odb,
    PhantomData<(*const (), &'r ffi::git_repository)>,
);

/// Shared borrow of an [`Odb`]. `Copy`, like `&T`; every odb routine takes it.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct OdbRef<'a, 'r>(CBorrowedPtr<'a, Odb<'r>>);

/// Exclusive borrow of an [`Odb`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref); nothing lends one, since a repository may hold the odb.
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
    type Mut<'a>
        = OdbMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = OdbRef<'a, 'r>
    where
        Self: 'a;
}

impl<'a, 'r: 'a> CRefSeam<'a> for OdbRef<'a, 'r> {
    type Cell = Odb<'r>;
}

impl<'a, 'r: 'a> CMutSeam<'a> for OdbMut<'a, 'r> {
    type Cell = Odb<'r>;
}

// SAFETY: an odb has no thread affinity: libgit2 frees it, its backends and its mutex
// from whichever thread drops the last reference, and its reference count and owner
// are atomics. Every routine a safe method calls that changes it takes its mutex
// (backends, commit-graph) or a cache's rwlock, and the fields read unlocked (options,
// `do_fsync`) are written only before the odb is published. The two routines that touch
// shared state without the lock (`git_odb_write_multi_pack_index`,
// `git_odb_set_commit_graph`) are reached only through `unsafe` methods whose contract
// excludes every other thread for the call. The one object an odb reaches outside itself is
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

impl<'r> OdbMut<'_, 'r> {
    /// Reborrows shared.
    #[must_use]
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
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbFree;

// SAFETY: `git_odb_free` drops exactly the one reference the owner holds: the odb is
// freed once, when its count reaches zero and no repository owns it. It needs no state
// beyond the pointer, and reaches no repository.
unsafe impl<'r> CDrop<Odb<'r>> for OdbFree {
    /// Wraps `fn::git_odb_free`
    #[doc(alias = "git_odb_free")]
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
    /// Wraps `fn::git_odb_new`
    #[doc(alias = "git_odb_new")]
    fn c_new(&self) -> Option<NonNull<Odb<'static>>> {
        if !runtime_pinned() {
            return None;
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; the runtime is pinned, so a failure is
        // reported on live thread-local state.
        let rc = unsafe { ffi::git_odb_new(&raw mut out) };
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
/// use libgit2::libgit2::src::libgit2::odb::OdbArc;
/// use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryRef};
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
/// use libgit2::libgit2::src::libgit2::odb::Odb;
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
/// use libgit2::libgit2::src::libgit2::odb::Odb;
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
/// use libgit2::libgit2::src::libgit2::odb::Odb;
///
/// let odb = Odb::new().unwrap();
/// let _again = odb.clone();
/// ```
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::src::libgit2::odb::Odb;
///
/// let mut odb = Odb::new().unwrap();
/// let _exclusive = odb.get_mut();
/// ```
pub type OdbArc<'r> = CArc<Odb<'r>, OdbFree>;

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
    /// Nothing can be read from it or written to it until a backend is added
    /// ([`OdbRef::add_disk_alternate`] adds a read-only one).
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, when the odb cannot be allocated.
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
    /// Wraps `fn::git_odb_new_ext`
    ///
    /// # Errors
    ///
    /// Fails as [`new`](Self::new) does.
    #[doc(alias = "git_odb_new_ext")]
    pub fn with_oid_type(oid_type: OidType) -> Result<OdbArc<'static>, ErrorBuf> {
        let opts = OdbOptions::with_oid_type(oid_type);
        construct(|out| {
            // SAFETY: `out` is a writable slot, filled with a fresh reference on
            // success; `opts` is a live options struct of this version, which the
            // routine copies and does not retain. The runtime is pinned.
            unsafe { ffi::git_odb_new_ext(out, opts.as_ref().as_ptr()) }
        })
    }

    /// An object database over the `objects` directory `objects_dir`, which hashes
    /// SHA-1 ids: its loose objects and packfiles, its commit-graph, and the alternate
    /// directories its `info/alternates` names, and theirs, six levels deep, each
    /// read-only; a directory already open is skipped.
    ///
    /// The directory must exist, but need not hold anything; an alternate that does not exist is
    /// skipped.
    ///
    /// Wraps `fn::git_odb_open`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, when `objects_dir` cannot be examined (`GIT_ERROR_ODB`, `failed
    /// to load object database in '<dir>'`), when an `info/alternates` file cannot be read, or when
    /// memory runs out.
    #[doc(alias = "git_odb_open")]
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
    /// Wraps `fn::git_odb_open_ext`
    ///
    /// # Errors
    ///
    /// Fails as [`open`](Self::open) does.
    #[doc(alias = "git_odb_open_ext")]
    pub fn open_with_oid_type(
        objects_dir: &CStr,
        oid_type: OidType,
    ) -> Result<OdbArc<'static>, ErrorBuf> {
        let opts = OdbOptions::with_oid_type(oid_type);
        construct(|out| {
            // SAFETY: as in `open`; `opts` is a live options struct of this version,
            // which the routine copies and does not retain.
            unsafe { ffi::git_odb_open_ext(out, objects_dir.as_ptr(), opts.as_ref().as_ptr()) }
        })
    }
}

// The layout type is the C type's size, so an odb pointer converts both ways, and the
// owner is pointer-sized.
const _: () = assert!(size_of::<Odb<'static>>() == size_of::<ffi::git_odb>());

const _: () = assert!(size_of::<OdbArc<'static>>() == size_of::<*mut ffi::git_odb>());

const _: () = assert!(size_of::<Option<OdbArc<'static>>>() == size_of::<*mut ffi::git_odb>());

define_ctype!(
    /// The options of an object database (`git_odb_options`): the structure version, and
    /// the object id type the odb hashes with.
    ///
    /// A plain value of 8 bytes, resource-free: `Copy`, held inline, never released. Build
    /// one with [`new`](Self::new) (or `Default`), then read and write its fields through
    /// [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut):
    ///
    /// ```
    /// use libgit2::libgit2::src::libgit2::odb::OdbOptions;
    /// use libgit2::libgit2::src::libgit2::oid::OidType;
    ///
    /// let mut opts = OdbOptions::new();
    /// assert_eq!(opts.as_ref().oid_type(), None);
    /// opts.as_mut().set_oid_type(Some(OidType::Sha256));
    /// assert_eq!(opts.as_ref().oid_type(), Some(OidType::Sha256));
    /// assert_eq!(opts.as_ref().version(), OdbOptions::VERSION);
    /// ```
    ///
    /// libgit2 does not check `oid_type`: an odb built with a type it does not name hashes
    /// and sizes ids as no type at all. So the value only ever holds 0, which libgit2 reads
    /// as its default, SHA-1, or a named [`OidType`]; there is no conversion from a raw
    /// `git_odb_options`, and the setter takes an `Option<OidType>`. The version is
    /// checked by every routine that reads the options, so any version may be stored.
    ///
    /// A handle borrows the value, so it cannot outlive it:
    ///
    /// ```compile_fail,E0597
    /// use libgit2::libgit2::src::libgit2::odb::OdbOptions;
    ///
    /// let handle = {
    ///     let opts = OdbOptions::new();
    ///     opts.as_ref()
    /// };
    /// let _ = handle.version();
    /// ```
    ///
    /// and the shared handle only reads:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::src::libgit2::odb::OdbOptions;
    ///
    /// let opts = OdbOptions::new();
    /// opts.as_ref().set_oid_type(None);
    /// ```
    ///
    /// Wraps `ty::git_odb_options`
    #[doc(alias = "git_odb_options")]
    #[derive(Clone, Copy)]
    OdbOptions,
    OdbOptionsRef,
    OdbOptionsMut,
    ffi::git_odb_options
);

// SAFETY: an `OdbOptions` is 8 plain bytes: no pointer, no interior mutability, no thread
// affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for OdbOptions {}

// SAFETY: `OdbOptionsMut` is an exclusive borrow of such a value, as `&mut OdbOptions`.
unsafe impl Send for OdbOptionsMut<'_> {}

// SAFETY: shared access only reads the fields (every write goes through `OdbOptionsMut`,
// which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for OdbOptions {}

// SAFETY: as above; `OdbOptionsRef` is a shared borrow, as `&OdbOptions`.
unsafe impl Send for OdbOptionsRef<'_> {}

// SAFETY: as above.
unsafe impl Sync for OdbOptionsRef<'_> {}

// SAFETY: `&OdbOptionsMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for OdbOptionsMut<'_> {}

impl OdbOptions {
    /// The structure version this crate's header declares (`GIT_ODB_OPTIONS_VERSION`), the
    /// only one libgit2 accepts.
    pub const VERSION: u32 = ffi::GIT_ODB_OPTIONS_VERSION;

    /// The default options, as `git_odb_options_init` fills them for
    /// [`VERSION`](Self::VERSION) and `GIT_ODB_OPTIONS_INIT` spells them: the current
    /// version, and `oid_type` 0, which libgit2 reads as SHA-1. Needs no initialised
    /// libgit2.
    ///
    /// Wraps `fn::git_odb_options_init`
    ///
    /// # Panics
    ///
    /// Panics only if libgit2 refuses the version of the options it defines itself, which it
    /// never does.
    #[doc(alias = "git_odb_options_init")]
    #[must_use]
    pub fn new() -> OdbOptions {
        let mut raw = ffi::git_odb_options::default();
        // SAFETY: `raw` is a live `git_odb_options` the call may write, and is not retained.
        // With the header's own version the routine's version check passes, so it only
        // copies the 8-byte `GIT_ODB_OPTIONS_INIT` template into `raw`: no allocation and no
        // error state, so no initialised runtime is needed.
        let rc = unsafe { ffi::git_odb_options_init(&raw mut raw, Self::VERSION) };
        assert_eq!(rc, 0, "git_odb_options_init refused its own version");
        OdbOptions(raw, PhantomData)
    }

    /// The default options with `oid_type` set: what the constructors on [`Odb`] pass
    /// libgit2.
    #[must_use]
    pub fn with_oid_type(oid_type: OidType) -> OdbOptions {
        let mut opts = OdbOptions::new();
        opts.as_mut().set_oid_type(Some(oid_type));
        opts
    }
}

impl Default for OdbOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The `git_odb_options` value, for code that passes one by value.
impl From<OdbOptions> for ffi::git_odb_options {
    fn from(opts: OdbOptions) -> ffi::git_odb_options {
        opts.0
    }
}

impl OdbOptionsRef<'_> {
    /// The structure version. [`OdbOptions::new`] sets [`OdbOptions::VERSION`]; libgit2
    /// refuses the options with `GIT_ERROR_INVALID` if it is 0 or above that.
    ///
    /// Wraps `fld::git_odb_options.version`
    #[doc(alias = "git_odb_options.version")]
    #[must_use]
    pub fn version(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let version_ptr = unsafe { &raw const (*self.as_ptr()).version };
        // SAFETY: a read through the shared handle's pointer to a live `git_odb_options`;
        // any value is a valid `u32`.
        unsafe { version_ptr.read() }
    }

    /// The object id type the odb hashes with; `None` for 0, which libgit2 reads as its
    /// default, [`OidType::Sha1`].
    ///
    /// Wraps `fld::git_odb_options.oid_type`
    #[doc(alias = "git_odb_options.oid_type")]
    #[must_use]
    pub fn oid_type(&self) -> Option<OidType> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let oid_type_ptr = unsafe { &raw const (*self.as_ptr()).oid_type };
        // SAFETY: a read through the shared handle's pointer to a live `git_odb_options`;
        // any value is a valid `git_oid_t`.
        let raw = unsafe { oid_type_ptr.read() };
        OidType::from_raw(raw)
    }

    /// A copy of the options, as a value of its own.
    #[must_use]
    pub fn to_options(&self) -> OdbOptions {
        let mut opts = OdbOptions::new();
        let mut m = opts.as_mut();
        m.set_version(self.version());
        m.set_oid_type(self.oid_type());
        opts
    }
}

impl OdbOptionsMut<'_> {
    /// Replaces the structure version. Any value is accepted; libgit2 refuses options whose
    /// version is 0 or above [`OdbOptions::VERSION`].
    ///
    /// Wraps `fld::git_odb_options.version`
    #[doc(alias = "git_odb_options.version")]
    pub fn set_version(&mut self, version: u32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let version_ptr = unsafe { &raw mut (*self.as_mut_ptr()).version };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_odb_options`; the field is a plain integer.
        unsafe { version_ptr.write(version) }
    }

    /// Replaces the object id type; `None` stores 0, libgit2's default (SHA-1).
    ///
    /// Wraps `fld::git_odb_options.oid_type`
    #[doc(alias = "git_odb_options.oid_type")]
    pub fn set_oid_type(&mut self, oid_type: Option<OidType>) {
        let raw = oid_type.map_or(0, OidType::raw);
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let oid_type_ptr = unsafe { &raw mut (*self.as_mut_ptr()).oid_type };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_odb_options`; the field is a plain integer, and `raw` is 0 or a named type.
        unsafe { oid_type_ptr.write(raw) }
    }
}

impl fmt::Debug for OdbOptionsRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OdbOptions")
            .field("version", &self.version())
            .field("oid_type", &self.oid_type())
            .finish()
    }
}

impl fmt::Debug for OdbOptionsMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for OdbOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'b> PartialEq<OdbOptionsRef<'b>> for OdbOptionsRef<'_> {
    /// Field by field.
    fn eq(&self, other: &OdbOptionsRef<'b>) -> bool {
        self.version() == other.version() && self.oid_type() == other.oid_type()
    }
}

impl Eq for OdbOptionsRef<'_> {}

impl Hash for OdbOptionsRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.version().hash(state);
        self.oid_type().hash(state);
    }
}

impl PartialEq for OdbOptions {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for OdbOptions {}

impl Hash for OdbOptions {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

// An `OdbOptions` is a `git_odb_options`, so C reads a value Rust holds in place.
const _: () = assert!(
    size_of::<OdbOptions>() == size_of::<ffi::git_odb_options>()
        && align_of::<OdbOptions>() == align_of::<ffi::git_odb_options>()
);

define_ctype!(
    /// One query of `git_odb_expand_ids` (`git_odb_expand_id`): an object id, or a prefix
    /// of one, and the type the object must have; the routine writes the answer over it.
    ///
    /// A plain value of 40 bytes, resource-free: `Copy`, held inline, never released.
    /// Build one with [`new`](Self::new), or start from [`OdbExpandId::default`] /
    /// [`OdbExpandId::zeroed`](ffibox::CZeroable::zeroed), which queries nothing; read and
    /// write its fields through [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut).
    ///
    /// The fields are in and out at once. On the way in, `length` is how many hex digits
    /// of `id` to match (a full id's own hex size for a full id; fewer than
    /// `GIT_OID_MINPREFIXLEN`, 4, finds nothing), and `type` the type the object must have,
    /// `GIT_OBJECT_ANY` or 0 for any. On the way out, a query that found exactly one
    /// object of a matching type holds its full id, the id type's hex size as `length`
    /// and the object's type; any other, the id type's zero id, `length` 0 and `type` 0.
    ///
    /// Every value is valid for the type: libgit2 reads no more than the 32 id bytes
    /// whatever `length` says, and a `type` it does not know matches nothing.
    ///
    /// ```
    /// use libgit2::libgit2::src::libgit2::object::ObjectType;
    /// use libgit2::libgit2::src::libgit2::odb::OdbExpandId;
    /// use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    ///
    /// # if cfg!(miri) { return; } // Miri cannot call libgit2.
    /// let prefix = Oid::from_prefix(b"4a202b34", OidType::Sha1).unwrap();
    /// let mut query = OdbExpandId::new(prefix.as_ref(), 8, Some(ObjectType::Blob));
    /// assert_eq!(query.as_ref().length(), 8);
    /// query.as_mut().set_kind(None);
    /// assert_eq!(query.as_ref().raw_kind(), libgit2::ffi::GIT_OBJECT_ANY);
    /// ```
    ///
    /// Wraps `ty::git_odb_expand_id`
    #[doc(alias = "git_odb_expand_id")]
    #[derive(Clone, Copy)]
    OdbExpandId,
    OdbExpandIdRef,
    OdbExpandIdMut,
    ffi::git_odb_expand_id
);

// SAFETY: all zero is libgit2's typeless `GIT_OID_NONE` id, `length` 0 and `type` 0, which
// `git_odb_expand_ids` reads as "any type" and answers as not found, the very state it
// writes for a miss; every getter reads plain bytes, and there is no teardown.
unsafe impl CZeroable for OdbExpandId {}

// SAFETY: an `OdbExpandId` is 40 plain bytes: no pointer, no interior mutability, no thread
// affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for OdbExpandId {}

// SAFETY: `OdbExpandIdMut` is an exclusive borrow of such a value, as `&mut OdbExpandId`.
unsafe impl Send for OdbExpandIdMut<'_> {}

// SAFETY: shared access only reads the fields (every write goes through `OdbExpandIdMut`,
// which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for OdbExpandId {}

// SAFETY: as above; `OdbExpandIdRef` is a shared borrow, as `&OdbExpandId`.
unsafe impl Send for OdbExpandIdRef<'_> {}

// SAFETY: as above.
unsafe impl Sync for OdbExpandIdRef<'_> {}

// SAFETY: `&OdbExpandIdMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for OdbExpandIdMut<'_> {}

impl OdbExpandId {
    /// A query for the object whose id starts with the first `length` hex digits of `id`,
    /// of type `kind`, or of any type for `None` (`GIT_OBJECT_ANY`).
    #[must_use]
    pub fn new(id: OidRef<'_>, length: u16, kind: Option<ObjectType>) -> OdbExpandId {
        let mut query = OdbExpandId::zeroed();
        let mut m = query.as_mut();
        m.set_id(id);
        m.set_length(length);
        m.set_kind(kind);
        query
    }
}

/// The empty query, as [`zeroed`](ffibox::CZeroable::zeroed) gives it.
impl Default for OdbExpandId {
    fn default() -> OdbExpandId {
        OdbExpandId::zeroed()
    }
}

/// The `git_odb_expand_id` value as it is, for code that already holds one by value.
impl From<ffi::git_odb_expand_id> for OdbExpandId {
    fn from(raw: ffi::git_odb_expand_id) -> OdbExpandId {
        OdbExpandId(raw, PhantomData)
    }
}

/// The `git_odb_expand_id` value, for code that passes one by value.
impl From<OdbExpandId> for ffi::git_odb_expand_id {
    fn from(query: OdbExpandId) -> ffi::git_odb_expand_id {
        query.0
    }
}

impl<'a> OdbExpandIdRef<'a> {
    /// The object id, or the prefix of one, in place, for as long as the query is
    /// borrowed.
    ///
    /// Wraps `fld::git_odb_expand_id.id`
    #[doc(alias = "git_odb_expand_id.id")]
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a field is never NULL"
    )]
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: the shared handle's pointer addresses a live query; the projection forms
        // no reference to it.
        let id = unsafe { &raw const (*self.as_ptr()).id };
        // SAFETY: `id` addresses an initialised `git_oid`, which lives for `'a` with the
        // query and which nothing writes meanwhile (every write needs the exclusive
        // handle); `OidRef` only reads it.
        unsafe { OidRef::from_ptr(id.cast_mut()) }.expect("a field is never NULL")
    }

    /// How many hex digits of [`id`](Self::id) the query matches; after
    /// `git_odb_expand_ids`, the id type's hex size when it found the object, 0 when not.
    ///
    /// Wraps `fld::git_odb_expand_id.length`
    #[doc(alias = "git_odb_expand_id.length")]
    #[must_use]
    pub fn length(&self) -> u16 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let length_ptr = unsafe { &raw const (*self.as_ptr()).length };
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_odb_expand_id`; any value is a valid `u16`.
        unsafe { length_ptr.read() }
    }

    /// The object's type: the type the query asks for, or the type found. `None` for any
    /// type (`GIT_OBJECT_ANY` or 0, which a miss leaves) and for a value no object type
    /// has; [`raw_kind`](Self::raw_kind) tells them apart.
    ///
    /// Wraps `fld::git_odb_expand_id.type`
    #[doc(alias = "git_odb_expand_id.type")]
    #[must_use]
    pub fn kind(&self) -> Option<ObjectType> {
        ObjectType::from_raw(self.raw_kind())
    }

    /// The type as libgit2 stores it, `GIT_OBJECT_ANY` and 0 included.
    ///
    /// Wraps `fld::git_odb_expand_id.type`
    #[doc(alias = "git_odb_expand_id.type")]
    #[must_use]
    pub fn raw_kind(&self) -> ffi::git_object_t {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let type_ptr = unsafe { &raw const (*self.as_ptr()).type_ };
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_odb_expand_id`; any value is a valid `git_object_t`, a C `int`.
        unsafe { type_ptr.read() }
    }

    /// A copy of the query, as a value of its own.
    #[must_use]
    pub fn to_expand_id(&self) -> OdbExpandId {
        OdbExpandId::from(ffi::git_odb_expand_id {
            id: copy_oid(self.id()),
            length: self.length(),
            type_: self.raw_kind(),
        })
    }
}

impl OdbExpandIdMut<'_> {
    /// The object id, in place, borrowed exclusively to change it.
    ///
    /// Wraps `fld::git_odb_expand_id.id`
    #[doc(alias = "git_odb_expand_id.id")]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a field is never NULL"
    )]
    pub fn id_mut(&mut self) -> OidMut<'_> {
        // SAFETY: the exclusive handle's pointer addresses a live query; the projection
        // forms no reference to it.
        let id = unsafe { &raw mut (*self.as_mut_ptr()).id };
        // SAFETY: `id` addresses an initialised `git_oid`, any of whose bytes may be
        // written; the result borrows this handle exclusively, so nothing else reaches the
        // query meanwhile.
        unsafe { OidMut::from_ptr(id) }.expect("a field is never NULL")
    }

    /// Replaces the id with a copy of `id`, type and all 32 bytes.
    ///
    /// Wraps `fld::git_odb_expand_id.id`
    #[doc(alias = "git_odb_expand_id.id")]
    pub fn set_id(&mut self, id: OidRef<'_>) {
        let id = copy_oid(id);
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let id_ptr = unsafe { &raw mut (*self.as_mut_ptr()).id };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_odb_expand_id`; the field is plain bytes, and `id` was copied out before the
        // write, so a source inside this query is read whole first.
        unsafe { id_ptr.write(id) }
    }

    /// Replaces how many hex digits of the id the query matches. Any value is accepted:
    /// libgit2 matches a prefix only from 4 digits up, and a full id from its type's hex
    /// size up.
    ///
    /// Wraps `fld::git_odb_expand_id.length`
    #[doc(alias = "git_odb_expand_id.length")]
    pub fn set_length(&mut self, length: u16) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let length_ptr = unsafe { &raw mut (*self.as_mut_ptr()).length };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_odb_expand_id`; the field is a plain integer.
        unsafe { length_ptr.write(length) }
    }

    /// Replaces the type the object must have; `None` stores `GIT_OBJECT_ANY`, any type.
    ///
    /// Wraps `fld::git_odb_expand_id.type`
    #[doc(alias = "git_odb_expand_id.type")]
    pub fn set_kind(&mut self, kind: Option<ObjectType>) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let type_ptr = unsafe { &raw mut (*self.as_mut_ptr()).type_ };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_odb_expand_id`; the field is a plain integer.
        unsafe { type_ptr.write(ObjectType::raw_or_any(kind)) }
    }
}

/// The 33 bytes of `id`, as a value of its own.
fn copy_oid(id: OidRef<'_>) -> ffi::git_oid {
    ffi::git_oid {
        type_: id.raw_kind(),
        id: id.id(),
    }
}

impl fmt::Debug for OdbExpandIdRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OdbExpandId")
            .field("id", &self.id())
            .field("length", &self.length())
            .field("type", &self.raw_kind())
            .finish()
    }
}

impl fmt::Debug for OdbExpandIdMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for OdbExpandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'b> PartialEq<OdbExpandIdRef<'b>> for OdbExpandIdRef<'_> {
    /// Field by field: the id's 33 bytes, as `OidRef`'s `==` compares them, the length and
    /// the stored type.
    fn eq(&self, other: &OdbExpandIdRef<'b>) -> bool {
        let (x, y) = (self.id(), other.id());
        (x.raw_kind(), x.id()) == (y.raw_kind(), y.id())
            && self.length() == other.length()
            && self.raw_kind() == other.raw_kind()
    }
}

impl Eq for OdbExpandIdRef<'_> {}

impl Hash for OdbExpandIdRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
        self.length().hash(state);
        self.raw_kind().hash(state);
    }
}

impl PartialEq for OdbExpandId {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for OdbExpandId {}

impl Hash for OdbExpandId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

// An `OdbExpandId` is a `git_odb_expand_id`, so C reads and writes a run of them Rust holds
// in place.
const _: () = assert!(
    size_of::<OdbExpandId>() == size_of::<ffi::git_odb_expand_id>()
        && align_of::<OdbExpandId>() == align_of::<ffi::git_odb_expand_id>()
);

/// The failure of an odb routine: the code it returned, and the error it left as the
/// thread's last libgit2 error, copied out.
///
/// The code tells the failures apart: [`ErrorCode::NotFound`] when no object matches an
/// id or prefix, [`ErrorCode::Ambiguous`] when a prefix matches several objects or is
/// shorter than `GIT_OID_MINPREFIXLEN` (4 hex digits), [`ErrorCode::Generic`] for the
/// rest, a refused id of another type than the odb's included.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct OdbError {
    code: c_int,
    error: ErrorBuf,
}

impl OdbError {
    /// The failure a routine reported by returning `rc`, with the thread's last error.
    /// A positive `rc`, which no odb routine documents, is reported as `GIT_ERROR`.
    fn from_libgit2(rc: c_int) -> Self {
        Self {
            code: if rc < 0 { rc } else { ErrorCode::Generic.raw() },
            error: last(),
        }
    }

    /// The returned code; `None` for one the header does not name
    /// ([`raw_code`](Self::raw_code) has it).
    #[must_use]
    pub fn code(&self) -> Option<ErrorCode> {
        ErrorCode::from_raw(self.code)
    }

    /// The returned code, always negative.
    #[must_use]
    pub fn raw_code(&self) -> c_int {
        self.code
    }

    /// The error's class; `None` for a class the header does not name
    /// ([`raw_class`](Self::raw_class) has it).
    #[must_use]
    pub fn class(&self) -> Option<ErrorClass> {
        self.error.class()
    }

    /// The error's class as libgit2 stored it.
    #[must_use]
    pub fn raw_class(&self) -> c_int {
        self.error.raw_class()
    }

    /// The error message.
    #[must_use]
    pub fn message(&self) -> &CStr {
        self.error.message()
    }

    /// The error, without the code.
    #[must_use]
    pub fn error(&self) -> &ErrorBuf {
        &self.error
    }
}

/// The error, without the code.
impl From<OdbError> for ErrorBuf {
    fn from(error: OdbError) -> Self {
        error.error
    }
}

impl fmt::Debug for OdbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("OdbError");
        match self.code() {
            Some(code) => s.field("code", &code),
            None => s.field("code", &self.code),
        };
        s.field("error", &self.error).finish()
    }
}

impl fmt::Display for OdbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for OdbError {}

/// `Ok` for a routine's 0, its failure otherwise.
fn status(rc: c_int) -> Result<(), OdbError> {
    if rc == 0 {
        Ok(())
    } else {
        Err(OdbError::from_libgit2(rc))
    }
}

/// The object a reading routine returned `rc` for, adopted from its out slot.
fn read_out(rc: c_int, out: *mut ffi::git_odb_object) -> Result<OdbObjectArc, OdbError> {
    status(rc)?;
    // SAFETY: the routine succeeded, so `out` is one counted reference to a live object,
    // which the caller owns and `git_odb_object_free` releases, as `OdbObjectFree` does.
    let object = unsafe { OdbObjectArc::from_c(out) };
    Ok(object.expect("an odb read succeeded without an object"))
}

/// The message of a refused id, in the wording libgit2 uses for a repository's.
const MISMATCH: &CStr = c"specified object ID type does not match object database object ID type";

/// Options of an odb lookup (`git_odb_lookup_flags_t`), for [`OdbRef::exists_ext`].
///
/// Bits, which combine; this version of libgit2 reads [`NO_REFRESH`](Self::NO_REFRESH)
/// alone and ignores every other bit, which [`from_bits_retain`](Self::from_bits_retain)
/// keeps for forward compatibility.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LookupFlags(ffi::git_odb_lookup_flags_t);

impl LookupFlags {
    /// No option: a lookup that misses refreshes the odb and looks again.
    pub const NONE: Self = Self(0);
    /// A lookup that misses does not refresh the odb (`GIT_ODB_LOOKUP_NO_REFRESH`): for
    /// a batch of lookups of objects that may legitimately be missing, after one
    /// [`refresh`](OdbRef::refresh).
    pub const NO_REFRESH: Self = Self(ffi::GIT_ODB_LOOKUP_NO_REFRESH);

    /// The bits `bits`, every one kept, including those this version ignores.
    #[must_use]
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// The bits, as libgit2 takes them.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every bit of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for LookupFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for LookupFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl OdbRef<'_, '_> {
    /// The object id type the odb hashes with, which libgit2 publishes no accessor for.
    ///
    /// `git_odb_expand_ids` clears an entry it cannot expand to the odb's type
    /// (`git_oid_clear(&id, db->options.oid_type)`): one of length 0 is neither expanded
    /// nor looked up, so the probe takes no lock and reaches no backend. `None` only for
    /// a type this header does not name, which no constructor of this crate builds.
    fn id_type(&self) -> Option<OidType> {
        let mut query = ffi::git_odb_expand_id::default();
        // SAFETY: the handle addresses a live odb for the call, whose options the routine
        // reads (fixed once the odb is built, so shared reads race nothing); `query` is
        // one writable entry, which the routine clears in place and does not retain.
        // With length 0 it neither locks nor calls a backend, then clears this thread's
        // error state, which the pinned runtime keeps live.
        let rc = unsafe { ffi::git_odb_expand_ids(self.as_ptr().cast_mut(), &raw mut query, 1) };
        debug_assert_eq!(rc, 0, "an unexpandable entry is cleared, not an error");
        OidType::from_raw(ffi::git_oid_t::from(query.id.type_))
    }

    /// Whether `id` is of the odb's type: the only ids the backends format safely.
    fn same_type(self, id: OidRef<'_>) -> bool {
        id.kind().is_some_and(|kind| Some(kind) == self.id_type())
    }

    /// Refuses an id of another type than the odb's, with `GIT_ERROR_INVALID`
    /// [`MISMATCH`] set on the thread, as libgit2 refuses one for a repository.
    fn fits(self, id: OidRef<'_>) -> Result<(), OdbError> {
        if self.same_type(id) {
            return Ok(());
        }
        set(ErrorClass::Invalid, MISMATCH);
        Err(OdbError {
            code: ErrorCode::Generic.raw(),
            error: last(),
        })
    }

    /// Adds the loose and packed backends of the `objects` directory `path` as
    /// alternates: searched after every main backend, and never written to. Its own
    /// `info/alternates` are followed, as [`Odb::open`] follows them; a directory the odb
    /// already reads (by inode) is skipped, and so is one that does not exist.
    ///
    /// Wraps `fn::git_odb_add_disk_alternate`
    ///
    /// # Errors
    ///
    /// Fails, with `-1` and libgit2's error, when an `info/alternates` file cannot be
    /// read, when a backend cannot be built, or when memory runs out; the backends added
    /// before the failure stay.
    #[doc(alias = "git_odb_add_disk_alternate")]
    pub fn add_disk_alternate(&self, path: &CStr) -> Result<(), OdbError> {
        // SAFETY: the handle addresses a live odb, whose backend list and commit-graph
        // the routine changes under the odb's mutex only; `path` is NUL-terminated, read
        // and copied for the call, never retained.
        let rc =
            unsafe { ffi::git_odb_add_disk_alternate(self.as_ptr().cast_mut(), path.as_ptr()) };
        status(rc)
    }

    /// Whether an object with the id `id` is in the odb: its cache, then each backend,
    /// and after a miss each backend again once [refreshed](Self::refresh).
    ///
    /// The zero id is in no odb. Neither is an id of another type than the odb's, which
    /// is not looked up: libgit2 would format it out of bounds (see the
    /// [module docs](self#reading-and-writing)). A failure to take the odb's lock reads
    /// as a miss, as the header's "0 otherwise" has it.
    ///
    /// Wraps `fn::git_odb_exists`
    #[doc(alias = "git_odb_exists")]
    #[must_use]
    pub fn exists(&self, id: OidRef<'_>) -> bool {
        if !self.same_type(id) {
            return false;
        }
        // SAFETY: the handle addresses a live odb, which the routine walks under its
        // mutex and whose cache it reads under the cache's lock; `id` is a live id of
        // the odb's type (checked above), read for the call only.
        let rc = unsafe { ffi::git_odb_exists(self.as_ptr().cast_mut(), id.as_ptr()) };
        rc > 0
    }

    /// Whether an object with the id `id` is in the odb, as [`exists`](Self::exists),
    /// with `flags`: [`LookupFlags::NO_REFRESH`] skips the refresh after a miss.
    ///
    /// Wraps `fn::git_odb_exists_ext`
    #[doc(alias = "git_odb_exists_ext")]
    #[must_use]
    pub fn exists_ext(&self, id: OidRef<'_>, flags: LookupFlags) -> bool {
        if !self.same_type(id) {
            return false;
        }
        // SAFETY: as in `exists`; `flags` is plain bits, of which libgit2 reads one.
        let rc =
            unsafe { ffi::git_odb_exists_ext(self.as_ptr().cast_mut(), id.as_ptr(), flags.bits()) };
        rc > 0
    }

    /// The full id of the one object whose id starts with the first `len` hex digits of
    /// `short_id`, searched in each backend, and after a miss in each refreshed one.
    ///
    /// A `len` of the odb's [`hex_size`](OidType::hex_size) or more asks whether
    /// `short_id` itself [`exists`](Self::exists).
    ///
    /// Wraps `fn::git_odb_exists_prefix`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::Ambiguous`] for a `len` below 4, or when several objects
    /// match; with [`ErrorCode::NotFound`] when none does; with code `-1` and libgit2's
    /// `GIT_ERROR_INVALID` error [`MISMATCH`] for a `short_id` of another type than the
    /// odb's, which is not looked up; and with libgit2's error when a backend fails.
    #[doc(alias = "git_odb_exists_prefix")]
    pub fn exists_prefix(&self, short_id: OidRef<'_>, len: usize) -> Result<Oid, OdbError> {
        self.fits(short_id)?;
        let mut out = Oid::zeroed();
        // SAFETY: `out` addresses a writable id, written on success only; the handle a
        // live odb, walked under its mutex; `short_id` a live id of the odb's type
        // (checked above), read for the call: a `len` below the odb's hex size copies
        // `(len + 1) / 2` bytes of it, within its `GIT_OID_MAX_SIZE`, and any other
        // takes the full-id path.
        let rc = unsafe {
            ffi::git_odb_exists_prefix(
                out.as_mut().as_mut_ptr(),
                self.as_ptr().cast_mut(),
                short_id.as_ptr(),
                len,
            )
        };
        status(rc)?;
        Ok(out)
    }

    /// The number of backends the odb reads, alternates included.
    ///
    /// Wraps `fn::git_odb_num_backends`
    #[doc(alias = "git_odb_num_backends")]
    #[must_use]
    pub fn num_backends(&self) -> usize {
        // SAFETY: the handle addresses a live odb, whose backend count the routine reads
        // under its mutex.
        unsafe { ffi::git_odb_num_backends(self.as_ptr().cast_mut()) }
    }

    /// The object with the id `id`: from the cache when it holds it, otherwise read from
    /// the first backend that has it, checked against its id when strict hash
    /// verification is on, and cached.
    ///
    /// The object is libgit2's, counted: it outlives the odb and every other holder.
    ///
    /// Wraps `fn::git_odb_read`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when no backend has the object (the zero id
    /// included); with code `-1` and libgit2's `GIT_ERROR_INVALID` error [`MISMATCH`] for
    /// an `id` of another type than the odb's, which is not looked up; and with libgit2's
    /// error when a backend fails to read it, or its content does not hash to `id`.
    ///
    /// # Panics
    ///
    /// Panics, with `an odb read succeeded without an object`, only if libgit2 breaks its
    /// contract.
    #[doc(alias = "git_odb_read")]
    pub fn read(&self, id: OidRef<'_>) -> Result<OdbObjectArc, OdbError> {
        self.fits(id)?;
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set to one counted reference on success; the
        // handle addresses a live odb, walked under its mutex and cached under its
        // cache's lock; `id` is a live id of the odb's type (checked above), read for
        // the call only.
        let rc = unsafe { ffi::git_odb_read(&raw mut out, self.as_ptr().cast_mut(), id.as_ptr()) };
        read_out(rc, out)
    }

    /// The size and type of the object with the id `id`, read without its content where
    /// a backend can (the loose and packed ones can), otherwise from the whole object.
    ///
    /// The type is `None` only for one a custom backend reported that names no object
    /// type; libgit2's own backends refuse such an object.
    ///
    /// Wraps `fn::git_odb_read_header`
    ///
    /// # Errors
    ///
    /// Fails as [`read`](Self::read) does.
    #[doc(alias = "git_odb_read_header")]
    pub fn read_header(&self, id: OidRef<'_>) -> Result<(usize, Option<ObjectType>), OdbError> {
        self.fits(id)?;
        let (mut len, mut kind) = (0, ffi::GIT_OBJECT_INVALID);
        // SAFETY: `len` and `kind` are writable out slots; the handle addresses a live
        // odb, walked under its mutex; `id` is a live id of the odb's type (checked
        // above), read for the call only. An object the routine reads to answer is
        // released before it returns.
        let rc = unsafe {
            ffi::git_odb_read_header(
                &raw mut len,
                &raw mut kind,
                self.as_ptr().cast_mut(),
                id.as_ptr(),
            )
        };
        status(rc)?;
        Ok((len, ObjectType::from_raw(kind)))
    }

    /// The one object whose id starts with the first `len` hex digits of `short_id`, as
    /// [`read`](Self::read) reads it; a `len` past the odb's
    /// [`hex_size`](OidType::hex_size) reads as the whole id.
    ///
    /// Wraps `fn::git_odb_read_prefix`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::Ambiguous`] for a `len` below 4, or when several objects
    /// match; otherwise as [`read`](Self::read) does.
    ///
    /// # Panics
    ///
    /// Panics, with `an odb read succeeded without an object`, only if libgit2 breaks its
    /// contract.
    #[doc(alias = "git_odb_read_prefix")]
    pub fn read_prefix(&self, short_id: OidRef<'_>, len: usize) -> Result<OdbObjectArc, OdbError> {
        self.fits(short_id)?;
        let mut out = ptr::null_mut();
        // SAFETY: as in `read`; `len` is clamped to the odb's hex size before the prefix
        // is copied, so at most `GIT_OID_MAX_SIZE` bytes of `short_id` are read.
        let rc = unsafe {
            ffi::git_odb_read_prefix(
                &raw mut out,
                self.as_ptr().cast_mut(),
                short_id.as_ptr(),
                len,
            )
        };
        read_out(rc, out)
    }

    /// Reloads what each backend knows of its directory (new packfiles, a rewritten
    /// multi-pack-index) and drops a commit-graph file that changed on disk.
    ///
    /// Never needed for correctness: a lookup that misses refreshes on its own, unless
    /// [`LookupFlags::NO_REFRESH`] says otherwise.
    ///
    /// Wraps `fn::git_odb_refresh`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, when a backend cannot rescan its directory.
    #[doc(alias = "git_odb_refresh")]
    pub fn refresh(&self) -> Result<(), OdbError> {
        // SAFETY: the handle addresses a live odb, whose backends and commit-graph the
        // routine refreshes under its mutex.
        let rc = unsafe { ffi::git_odb_refresh(self.as_ptr().cast_mut()) };
        status(rc)
    }

    /// Removes the odb's commit-graph, freeing it; reads then walk commits without it.
    ///
    /// The odb loads one with its first on-disk backend ([`Odb::open`],
    /// [`add_disk_alternate`](Self::add_disk_alternate)); adding a disk backend later
    /// loads one again. Installing a commit-graph of the caller's (the routine's other
    /// use) awaits a wrapper of `git_commit_graph`.
    ///
    /// Wraps `fn::git_odb_set_commit_graph`
    ///
    /// # Safety
    ///
    /// No other thread may use the odb until the call returns: not through another
    /// handle or owner of it, and not through a repository that owns it. The routine
    /// frees the graph under the odb's lock, but libgit2's revision walk reads the
    /// graph's file after that lock is released, so a walk running meanwhile would read
    /// freed memory.
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, when the odb's lock cannot be taken.
    #[doc(alias = "git_odb_set_commit_graph")]
    pub unsafe fn clear_commit_graph(&self) -> Result<(), OdbError> {
        // SAFETY: the handle addresses a live odb; NULL asks the routine to free the
        // graph it holds and keep none, under its mutex; per this function's contract no
        // other thread holds the freed graph's file.
        let rc =
            unsafe { ffi::git_odb_set_commit_graph(self.as_ptr().cast_mut(), ptr::null_mut()) };
        status(rc)
    }

    /// Writes an object of type `kind` with the content `data` to the first main backend
    /// that writes (alternates never do), and returns its id, hashed with the odb's id
    /// type. An object the odb already has is not written again: its file's time is
    /// refreshed instead.
    ///
    /// The content is not validated against `kind`: a tree or commit libgit2 could not
    /// parse is stored as given.
    ///
    /// Wraps `fn::git_odb_write`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, when no backend can write (`GIT_ERROR_ODB`, "cannot
    /// write object - unsupported in the loaded odb backends" for an odb without one),
    /// or when the write fails.
    #[doc(alias = "git_odb_write")]
    pub fn write(&self, data: &[u8], kind: ObjectType) -> Result<Oid, OdbError> {
        let mut out = Oid::zeroed();
        // SAFETY: `out` addresses a writable id; the handle a live odb, written under its
        // mutex; `data` is valid for reads of its length (any non-NULL pointer when
        // empty), read for the call only; `kind` names an object type.
        let rc = unsafe {
            ffi::git_odb_write(
                out.as_mut().as_mut_ptr(),
                self.as_ptr().cast_mut(),
                data.as_ptr().cast(),
                data.len(),
                kind.raw(),
            )
        };
        status(rc)?;
        Ok(out)
    }

    /// Writes a `multi-pack-index` next to the packfiles of each main backend that packs,
    /// indexing every object of every pack, then reloads it.
    ///
    /// Wraps `fn::git_odb_write_multi_pack_index`
    ///
    /// # Safety
    ///
    /// No other thread may use the odb until the call returns: not through another
    /// handle or owner of it, and not through a repository that owns it. Unlike every
    /// other odb routine, this one walks the backend list without the odb's lock, and
    /// the packed backend rescans and replaces its pack list unlocked, so a concurrent
    /// read, refresh or added backend is a data race.
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, when no main backend packs (`GIT_ERROR_ODB`, "cannot
    /// write multi-pack-index - unsupported in the loaded odb backends"), or when a pack
    /// index cannot be read or the file cannot be written.
    #[doc(alias = "git_odb_write_multi_pack_index")]
    pub unsafe fn write_multi_pack_index(&self) -> Result<(), OdbError> {
        // SAFETY: the handle addresses a live odb; per this function's contract no other
        // thread uses it, so the unlocked walk of its backends and of the packed
        // backend's packs races nothing.
        let rc = unsafe { ffi::git_odb_write_multi_pack_index(self.as_ptr().cast_mut()) };
        status(rc)
    }
}

define_ctype!(
    /// An object read from an object database (`git_odb_object`): its id, its type and
    /// its raw, uncompressed content, without the `<type> <size>` header.
    ///
    /// libgit2 allocates, counts and frees it, so it is only ever reached through an
    /// [`OdbObjectArc`] and the handles it lends, from [`OdbRef::read`] or
    /// [`OdbRef::read_prefix`]. Nothing in it changes once it is read, and it keeps no
    /// pointer to the odb, which it may outlive.
    ///
    /// Wraps `ty::git_odb_object`
    #[doc(alias = "git_odb_object")]
    OdbObject,
    OdbObjectRef,
    OdbObjectMut,
    ffi::git_odb_object
);

// SAFETY: an odb object has no thread affinity: its count is atomic
// (`git_cached_obj_decref`), the last reference frees it from whichever thread drops it,
// and nothing else of it is written after it is read.
unsafe impl Send for OdbObject {}

// SAFETY: as above: every reader only reads its id, type, size and content, which are
// written once, before the object is published to the cache or the reader.
unsafe impl Sync for OdbObject {}

// SAFETY: `OdbObjectRef` is a shared borrow, as `&OdbObject`, and `OdbObject` is `Sync`.
unsafe impl Send for OdbObjectRef<'_> {}

// SAFETY: as above.
unsafe impl Sync for OdbObjectRef<'_> {}

// SAFETY: `OdbObjectMut` is an exclusive borrow, as `&mut OdbObject`, and `OdbObject` is
// `Send`; nothing lends one, since an object is always shared with the cache.
unsafe impl Send for OdbObjectMut<'_> {}

// SAFETY: `&OdbObjectMut` only reaches the shared handle, and `OdbObject` is `Sync`.
unsafe impl Sync for OdbObjectMut<'_> {}

impl<'a> OdbObjectRef<'a> {
    /// The id the object was read under; for [`OdbRef::read_prefix`], the full id it
    /// found.
    ///
    /// Wraps `fn::git_odb_object_id`
    ///
    /// # Panics
    ///
    /// Panics, with `an odb object always has an id`, only if libgit2 breaks its contract.
    #[doc(alias = "git_odb_object_id")]
    #[must_use]
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: the handle addresses a live object for `'a`, which the routine only
        // reads.
        let id = unsafe { ffi::git_odb_object_id(self.as_ptr().cast_mut()) };
        // SAFETY: `id` addresses the object's own id, never written after the read, which
        // lives as long as the object, past `'a`.
        unsafe { OidRef::from_ptr(id.cast_mut()) }.expect("an odb object always has an id")
    }

    /// The object's content: its [`size`](Self::size) bytes, NUL bytes included,
    /// borrowed in place for as long as the handle. The object holds them, and nothing
    /// writes them.
    ///
    /// The content lives no longer than the object's borrow:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::src::libgit2::object::ObjectType;
    /// use libgit2::libgit2::src::libgit2::odb::Odb;
    ///
    /// let odb = Odb::open(c".git/objects").unwrap();
    /// let id = odb.as_ref().write(b"x", ObjectType::Blob).unwrap();
    /// let object = odb.as_ref().read(id.as_ref()).unwrap();
    /// let content = object.as_ref().data();
    /// drop(object);
    /// let _ = content.len();
    /// ```
    ///
    /// Wraps `fn::git_odb_object_data`
    #[doc(alias = "git_odb_object_data")]
    #[must_use]
    pub fn data(&self) -> &'a [u8] {
        let size = self.size();
        // SAFETY: the handle addresses a live object for `'a`, which the routine only
        // reads.
        let data = unsafe { ffi::git_odb_object_data(self.as_ptr().cast_mut()) };
        if size == 0 || data.is_null() {
            // A custom backend may hand back NULL for an empty object.
            return &[];
        }
        // SAFETY: an object's buffer is the `size` initialised bytes its backend read,
        // owned by the object, freed with it and never written after the read; the
        // object lives past `'a`.
        unsafe { core::slice::from_raw_parts(data.cast::<u8>(), size) }
    }

    /// The size of the object's [`data`](Self::data), in bytes.
    ///
    /// Wraps `fn::git_odb_object_size`
    #[doc(alias = "git_odb_object_size")]
    #[must_use]
    pub fn size(&self) -> usize {
        // SAFETY: the handle addresses a live object for `'a`, which the routine only
        // reads.
        unsafe { ffi::git_odb_object_size(self.as_ptr().cast_mut()) }
    }

    /// The object's type. `None` only for one a custom backend reported that names no
    /// object type; libgit2's own backends refuse such an object.
    ///
    /// Wraps `fn::git_odb_object_type`
    #[doc(alias = "git_odb_object_type")]
    #[must_use]
    pub fn kind(&self) -> Option<ObjectType> {
        // SAFETY: the handle addresses a live object for `'a`, which the routine only
        // reads.
        let kind = unsafe { ffi::git_odb_object_type(self.as_ptr().cast_mut()) };
        ObjectType::from_raw(kind)
    }
}

impl fmt::Debug for OdbObjectRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OdbObject")
            .field("id", &self.id())
            .field("kind", &self.kind())
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for OdbObjectMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases one counted reference to an [`OdbObject`] through `git_odb_object_free`,
/// and takes another through `git_odb_object_dup`.
///
/// The free decrements the object's atomic count and, at zero, frees its content and
/// itself; the odb's cache counts a reference of its own while it keeps the object. The
/// copy increments the same count and hands back the same pointer. Neither can fail,
/// and neither allocates.
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbObjectFree;

// SAFETY: `git_odb_object_free` gives back exactly one counted reference of a live
// object (a NULL pointer, which it skips, never reaches it); the object is freed once,
// when the count, of which the cache's reference is part, reaches zero. It reads nothing
// but the object.
unsafe impl CDrop<OdbObject> for OdbObjectFree {
    /// Wraps `fn::git_odb_object_free`
    #[doc(alias = "git_odb_object_free")]
    unsafe fn c_drop(&self, ptr: NonNull<OdbObject>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live object of which the
        // caller gives up one reference; `OdbObject` is transparent over
        // `git_odb_object`.
        unsafe { ffi::git_odb_object_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `git_odb_object_dup` atomically increments the count of the live object it is
// given and stores that same pointer in `dest`, returning 0: the object then owes one
// more `git_odb_object_free`, which this policy performs. `c_is_sole_owner` keeps its
// `false` default: the cache counts a reference no owner sees.
unsafe impl CRefClone<OdbObject> for OdbObjectFree {
    /// Wraps `fn::git_odb_object_dup`
    #[doc(alias = "git_odb_object_dup")]
    unsafe fn c_up_ref(&self, ptr: NonNull<OdbObject>) -> bool {
        let mut dest = ptr::null_mut();
        // SAFETY: `dest` is a writable slot; `ptr` addresses a live object whose
        // reference the caller holds, per `c_up_ref`'s contract.
        let rc = unsafe { ffi::git_odb_object_dup(&raw mut dest, ptr.as_ptr().cast()) };
        debug_assert_eq!(dest, ptr.as_ptr().cast());
        rc == 0
    }
}

/// One counted reference to an [`OdbObject`], given back by `git_odb_object_free` on
/// drop; `clone` takes another through `git_odb_object_dup`, on the same object.
///
/// It lends the shared handle through `as_ref()`, and never the exclusive one:
/// `get_mut` answers `None`, since the cache may hold the object too, and nothing
/// copies an object.
///
/// ```
/// use libgit2::libgit2::src::libgit2::object::ObjectType;
/// use libgit2::libgit2::src::libgit2::odb::Odb;
///
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// let dir = std::env::temp_dir().join(format!("odb-object-doc-{}", std::process::id()));
/// std::fs::create_dir_all(&dir).unwrap();
/// let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
/// let odb = Odb::open(&path).unwrap();
/// let id = odb.as_ref().write(b"hello", ObjectType::Blob).unwrap();
/// let object = odb.as_ref().read(id.as_ref()).unwrap();
/// let mut again = object.clone();
/// assert!(again.get_mut().is_none());
/// drop(odb);
/// assert_eq!(again.as_ref().data(), b"hello");
/// assert_eq!(again.as_ref().kind(), Some(ObjectType::Blob));
/// assert_eq!(again.as_ref().id(), id.as_ref());
/// # drop((object, again));
/// # std::fs::remove_dir_all(&dir).unwrap();
/// ```
pub type OdbObjectArc = CArc<OdbObject, OdbObjectFree>;

const _: () = assert!(size_of::<OdbObjectArc>() == size_of::<*mut ffi::git_odb_object>());

/// The closure behind a [`ForeachCallback`].
type ForeachFn<'f> = dyn FnMut(OidRef<'_>) -> ControlFlow<NonZero<c_int>> + 'f;

/// A `git_odb_foreach_cb`: the callback an odb walk calls with the id of each object of
/// each backend, in the backends' order.
///
/// Built over a closure with [`new`](Self::new). Its `extern "C"` trampoline lends the
/// closure each id as an [`OidRef`] valid for the call only (a backend lends a local or
/// an index entry), and returns [`ControlFlow::Continue`] as 0 and
/// [`ControlFlow::Break`]`(code)` as `code`, which a non-zero type guarantees stops the
/// walk. A panic in the closure does not cross into C: the trampoline stops the walk
/// with `GIT_EUSER` and the panic resumes once control is back in Rust, from
/// [`call`](Self::call) or from the wrapper that ran the walk.
///
/// ```
/// use std::ops::ControlFlow;
///
/// use libgit2::libgit2::src::libgit2::odb::ForeachCallback;
/// use libgit2::libgit2::src::libgit2::oid::{Oid, OidRef, OidType};
///
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// let mut seen = Vec::new();
/// let mut collect = |id: OidRef<'_>| {
///     seen.push(id.to_oid());
///     ControlFlow::Continue(())
/// };
/// let mut callback = ForeachCallback::new(&mut collect);
/// let id = Oid::zero(OidType::Sha1);
/// assert_eq!(callback.call(id.as_ref()), ControlFlow::Continue(()));
/// drop(callback);
/// assert_eq!(seen, [id]);
/// ```
///
/// The id cannot outlive the call:
///
/// ```compile_fail,E0521
/// use std::ops::ControlFlow;
/// use libgit2::libgit2::src::libgit2::odb::ForeachCallback;
/// use libgit2::libgit2::src::libgit2::oid::OidRef;
///
/// fn keep<'k>(kept: &mut Option<OidRef<'k>>) {
///     let mut f = |id: OidRef<'_>| {
///         *kept = Some(id);
///         ControlFlow::Continue(())
///     };
///     let _ = ForeachCallback::new(&mut f);
/// }
/// ```
///
/// It borrows its closure exclusively and stays on its thread: libgit2 calls it
/// synchronously, and the closure need not be `Send`.
///
/// ```compile_fail,E0277
/// use std::ops::ControlFlow;
/// use libgit2::libgit2::src::libgit2::odb::ForeachCallback;
/// use libgit2::libgit2::src::libgit2::oid::OidRef;
///
/// fn send<T: Send>(_: T) {}
/// let mut f = |_: OidRef<'_>| ControlFlow::Continue(());
/// send(ForeachCallback::new(&mut f));
/// ```
///
/// Wraps `ty::git_odb_foreach_cb`
#[doc(alias = "git_odb_foreach_cb")]
pub struct ForeachCallback<'f> {
    f: &'f mut ForeachFn<'f>,
    /// A panic the closure raised inside a C walk, resumed once the walk returns.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> ForeachCallback<'f> {
    /// The callback that calls `f` with each object's id, until it breaks.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(OidRef<'_>) -> ControlFlow<NonZero<c_int>> + 'f,
    {
        Self { f, panic: None }
    }

    /// Calls the callback with `id`, through the trampoline libgit2 calls, and returns
    /// what libgit2 would read from it. A panic of the closure resumes here.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: the trampoline is never NULL"
    )]
    pub fn call(&mut self, id: OidRef<'_>) -> ControlFlow<NonZero<c_int>> {
        let rc = self.walk_with(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            // SAFETY: `payload` is this callback, which nothing else uses during the
            // walk; `id` a live id, lent for the call, which only reads it: the
            // trampoline's contract.
            unsafe { callback(id.as_ptr(), payload) }
        });
        match NonZero::new(rc) {
            None => ControlFlow::Continue(()),
            Some(code) => ControlFlow::Break(code),
        }
    }

    /// Runs `walk` with the C callback and its payload, as the wrapper of a routine that
    /// calls the callback (`git_odb_foreach`) hands them to it, then resumes a panic the
    /// closure raised meanwhile. The payload is this callback, which nothing else
    /// touches until `walk` returns.
    pub(crate) fn walk_with<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_odb_foreach_cb, *mut c_void) -> R,
    ) -> R {
        let result = walk(Some(foreach_with), ptr::from_mut(self).cast());
        if let Some(panic) = self.panic.take() {
            panic::resume_unwind(panic);
        }
        result
    }
}

impl fmt::Debug for ForeachCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForeachCallback")
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_odb_foreach_cb` over a [`ForeachCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`ForeachCallback`] that nothing else uses during the
/// call, and `id` be NULL or a `git_oid` live and unchanged for the call.
unsafe extern "C" fn foreach_with(id: *const ffi::git_oid, payload: *mut c_void) -> c_int {
    // SAFETY: per this function's contract, `payload` is a live callback borrowed for the
    // call alone.
    let callback = unsafe { &mut *payload.cast::<ForeachCallback<'_>>() };
    if callback.panic.is_some() {
        return ErrorCode::User.raw();
    }
    // SAFETY: per this function's contract, a non-NULL `id` is live and unchanged for the
    // call; the closure only reads it, through a handle that cannot outlive the call.
    let Some(id) = (unsafe { OidRef::from_ptr(id.cast_mut()) }) else {
        // libgit2's backends never pass NULL; stop the walk rather than invent an id.
        return ErrorCode::User.raw();
    };
    match panic::catch_unwind(AssertUnwindSafe(|| (callback.f)(id))) {
        Ok(ControlFlow::Continue(())) => 0,
        Ok(ControlFlow::Break(code)) => code.get(),
        Err(panic) => {
            callback.panic = Some(panic);
            ErrorCode::User.raw()
        }
    }
}

/// The alignment `git_odb_backend_data_alloc` storage is guaranteed to have.
///
/// It is `git__allocator.gmalloc` storage, in which libgit2 itself places structs
/// holding pointers, `size_t`s and 64-bit integers, so any allocator libgit2 runs on
/// aligns that far. The default `stdalloc` is glibc's `malloc`, which aligns to 16.
const DATA_ALIGN: usize = {
    let (word, wide) = (align_of::<usize>(), align_of::<u64>());
    if word > wide { word } else { wide }
};

/// `git_odb_backend_data_alloc` for at least `byte_len` bytes; `None` on failure,
/// or when libgit2 cannot be initialised.
///
/// A zero-byte request asks for one byte instead: `malloc(0)` may return NULL, which
/// would read as failure.
fn data_alloc(byte_len: usize) -> Option<NonNull<u8>> {
    if !runtime_pinned() {
        return None;
    }
    // SAFETY: the routine never reads `backend` (`GIT_UNUSED`), so NULL is accepted;
    // it returns fresh `git__malloc` storage, or NULL on failure, for any `len`. The
    // runtime is pinned, so a failure is reported on live thread-local state.
    let data = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), byte_len.max(1)) };
    NonNull::new(data.cast())
}

/// `git_odb_backend_data_free`.
///
/// # Safety
///
/// `data` must be a live allocation from libgit2's current allocator
/// (`git_odb_backend_data_alloc`), which the caller owns and does not use again.
unsafe fn data_free(data: *mut u8) {
    // SAFETY: the routine never reads `backend`; `data` is a live `git__malloc`
    // allocation the caller gives up, per this function's contract.
    unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), data.cast()) }
}

/// One object in libgit2's allocator, released through `git_odb_backend_data_free`.
///
/// A storage policy: its free never reads the object, so it settles storage both
/// before and after it holds a `T` ([`CBox::new_uninit`], [`CBox::new_zeroed`]). It
/// releases no sub-resources, so a `T` owning any is promoted to a fuller destructor
/// with `with_policy` before it acquires them.
///
/// A type aligned beyond what the allocator vouches for is rejected:
///
/// ```compile_fail,E0080
/// use libgit2::libgit2::src::libgit2::odb::OdbDataBox;
///
/// #[repr(C, align(64))]
/// pub struct wide_st { pub x: u8 }
/// ffibox::define_ctype!(Wide, WideRef, WideMut, wide_st);
///
/// let _ = OdbDataBox::<Wide>::new_uninit();
/// ```
///
/// Zeroed storage needs a type that vouches all-zero is valid:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::odb::OdbDataBox;
///
/// #[repr(C)]
/// pub struct frame_st { pub format: i32 }
/// ffibox::define_ctype!(Frame, FrameRef, FrameMut, frame_st); // not `CZeroable`
///
/// let _ = OdbDataBox::<Frame>::new_zeroed();
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbDataFree;

/// One `T` in libgit2's allocator.
pub type OdbDataBox<T> = CBox<T, OdbDataFree>;

// SAFETY: `git_odb_backend_data_free` releases the allocation exactly once, without
// reading it, whether or not it holds a `T`; `self` carries no state.
unsafe impl<T> CDrop<T> for OdbDataFree {
    /// Wraps `fn::git_odb_backend_data_free`
    #[doc(alias = "git_odb_backend_data_free")]
    #[inline]
    unsafe fn c_drop(&self, ptr: NonNull<T>) {
        // SAFETY: per `c_drop`'s contract `ptr` is a live allocation whose claim the
        // caller owns; every owner under this policy holds `git__malloc` storage.
        unsafe { data_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `c_alloc` returns fresh, uniquely-owned storage of at least `size_of::<T>()`
// bytes, aligned to `DATA_ALIGN >= align_of::<T>()` (checked at compile time), which
// `c_drop` above releases without reading.
unsafe impl<T: CCell> CAlloc<T> for OdbDataFree {
    /// Wraps `fn::git_odb_backend_data_alloc`
    #[doc(alias = "git_odb_backend_data_alloc")]
    #[inline]
    fn c_alloc(&self) -> Option<NonNull<MaybeUninit<T>>> {
        const {
            assert!(
                align_of::<T>() <= DATA_ALIGN,
                "libgit2's allocator cannot align T"
            );
        };
        data_alloc(size_of::<T>()).map(NonNull::cast)
    }
}

// SAFETY: as `CAlloc`, with every byte of the `T` zeroed before it is handed out, a
// state `T: CZeroable` vouches for; `c_drop` accepts any `T` since it never reads it.
unsafe impl<T: CZeroable> CAllocZeroed<T> for OdbDataFree {
    /// Wraps `fn::git_odb_backend_data_alloc`
    #[doc(alias = "git_odb_backend_data_alloc")]
    #[inline]
    fn c_alloc_zeroed(&self) -> Option<NonNull<T>> {
        const {
            assert!(
                align_of::<T>() <= DATA_ALIGN,
                "libgit2's allocator cannot align T"
            );
        };
        let data = data_alloc(size_of::<T>())?;
        // SAFETY: `data` is a fresh allocation of at least `size_of::<T>()` bytes.
        unsafe { data.as_ptr().write_bytes(0, size_of::<T>()) };
        Some(data.cast())
    }
}

/// A counted run of elements in libgit2's allocator, released through
/// `git_odb_backend_data_free`; the buffer a backend's `read` callback hands libgit2.
///
/// [`CVec::from_slice`] and `Clone` copy into the allocator; [`new_uninit`] and
/// [`new_zeroed`] allocate in place. An empty buffer is NULL and allocates nothing.
///
/// Element types aligned beyond what the allocator vouches for are rejected, as are
/// zero-sized ones, which need no storage:
///
/// ```compile_fail,E0080
/// use libgit2::libgit2::src::libgit2::odb::OdbDataVec;
///
/// #[derive(Clone, Copy)]
/// #[repr(align(64))]
/// struct Wide(u8);
///
/// let _ = OdbDataVec::<Wide>::from_slice(&[Wide(0)]);
/// ```
///
/// ```compile_fail,E0080
/// let _ = libgit2::libgit2::src::libgit2::odb::OdbDataArrayFree::new_uninit::<()>(4);
/// ```
///
/// [`new_uninit`]: Self::new_uninit
/// [`new_zeroed`]: Self::new_zeroed
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbDataArrayFree;

/// A counted run of `T` in libgit2's allocator.
pub type OdbDataVec<T> = CVec<T, OdbDataArrayFree>;

/// `git_odb_backend_data_free` in `CLenDrop`'s shape: the free takes no size.
///
/// # Safety
///
/// As [`data_free`].
unsafe fn data_free_len(data: *mut u8, _byte_len: usize) {
    // SAFETY: forwarded from `c_drop_len`'s contract: a live allocation the caller owns.
    unsafe { data_free(data) }
}

/// A `memdup` into libgit2's allocator: a fresh copy of the `byte_len` bytes at
/// `src`, or NULL on failure. `git__allocator` has no copy routine of its own.
///
/// # Safety
///
/// `src` must be valid for reads of `byte_len` bytes.
unsafe fn data_dup_len(src: *mut u8, byte_len: usize) -> *mut u8 {
    let Some(copy) = data_alloc(byte_len) else {
        return ptr::null_mut();
    };
    // SAFETY: the caller guarantees `byte_len` readable bytes at `src`; `copy` is a
    // fresh allocation of at least `byte_len` bytes, so the two cannot overlap.
    unsafe { ptr::copy_nonoverlapping(src, copy.as_ptr(), byte_len) };
    copy.as_ptr()
}

impl_clendrop!(
    /// Wraps `fn::git_odb_backend_data_free`
    #[doc(alias = "git_odb_backend_data_free")]
    OdbDataArrayFree,
    data_free_len
);

impl_clenclone!(OdbDataArrayFree, data_dup_len, align = DATA_ALIGN);

impl OdbDataArrayFree {
    /// `len` uninitialised elements; `None` when the allocation fails or its size
    /// overflows. An empty request allocates nothing.
    ///
    /// Wraps `fn::git_odb_backend_data_alloc`
    #[doc(alias = "git_odb_backend_data_alloc")]
    #[must_use]
    pub fn new_uninit<T>(len: usize) -> Option<OdbDataVec<MaybeUninit<T>>> {
        let data = Self::alloc_run::<T>(len)?;
        // SAFETY: `data` is NULL with `len == 0`, or fresh, uniquely-owned storage of
        // at least `len * size_of::<T>()` bytes aligned for `T`, which
        // `data_free_len` releases whatever the length; `MaybeUninit<T>` needs no
        // initialisation.
        unsafe { CVec::from_raw_parts_or_empty(data.cast(), len) }
    }

    /// `len` zeroed elements; `None` when the allocation fails or its size
    /// overflows. An empty request allocates nothing.
    ///
    /// Wraps `fn::git_odb_backend_data_alloc`
    #[doc(alias = "git_odb_backend_data_alloc")]
    #[must_use]
    pub fn new_zeroed<T: CPlainElem>(len: usize) -> Option<OdbDataVec<T>> {
        let data = Self::alloc_run::<T>(len)?;
        if !data.is_null() {
            // SAFETY: `data` is a fresh allocation of at least `len * size_of::<T>()`
            // bytes.
            unsafe { data.write_bytes(0, len) };
        }
        // SAFETY: as in `new_uninit`, and every element is zero, a valid `T` since
        // `T: CPlainElem` admits every bit pattern.
        unsafe { CVec::from_raw_parts_or_empty(data, len) }
    }

    /// Storage for `len` elements of `T`: NULL for none, `None` on failure.
    fn alloc_run<T>(len: usize) -> Option<*mut T> {
        const {
            assert!(size_of::<T>() != 0, "zero-sized elements need no storage");
            assert!(
                align_of::<T>() <= DATA_ALIGN,
                "libgit2's allocator cannot align T"
            );
        }
        if len == 0 {
            return Some(ptr::null_mut());
        }
        // A slice spans at most `isize::MAX` bytes.
        let byte_len = len
            .checked_mul(size_of::<T>())
            .filter(|&n| isize::try_from(n).is_ok())?;
        data_alloc(byte_len).map(|data| data.as_ptr().cast())
    }
}

/// A copy of `s` and its terminator in libgit2's allocator: `git__strdup` over the
/// exported routines. `None` on failure.
fn str_dup(s: &CStr) -> Option<NonNull<c_char>> {
    let bytes = s.to_bytes_with_nul();
    let copy = data_alloc(bytes.len())?;
    // SAFETY: `copy` is a fresh allocation of at least `bytes.len()` bytes, so it
    // cannot overlap the borrowed `bytes`.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), copy.as_ptr(), bytes.len()) };
    Some(copy.cast())
}

/// `git_odb_backend_data_free` in `impl_cdrop_str!`'s shape.
///
/// # Safety
///
/// As [`data_free`].
unsafe fn str_free(s: *mut c_char) {
    // SAFETY: forwarded from `c_drop`'s contract: a live string the caller owns.
    unsafe { data_free(s.cast()) }
}

/// `str_dup` in `impl_cdupclone_str!`'s shape: a fresh copy, or NULL on failure.
///
/// # Safety
///
/// `s` must be a live NUL-terminated string.
unsafe fn str_dup_raw(s: *mut c_char) -> *mut c_char {
    // SAFETY: per this function's contract `s` is NUL-terminated and stays live for
    // the call; the copy does not write through it.
    let s = unsafe { CStr::from_ptr(s) };
    str_dup(s).map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// One NUL-terminated string in libgit2's allocator, released through
/// `git_odb_backend_data_free`: the strings `git_strarray_dispose`,
/// `git_signature_free` and `git_buf_dispose` release with `git__free`.
///
/// [`dup`](Self::dup) and `Clone` copy into the allocator, as `git__strdup` does;
/// [`CStrBox::into_raw`] hands the string to C, which may release it with any of
/// those routines. A string libgit2 made is adopted with [`CStrBox::from_raw`] only
/// when libgit2 owns it: a `git_strarray` element or a `git_signature` name always
/// is, a `git_buf`'s `ptr` only while `reserved > 0` — [`BufMut::take_str`] checks
/// that, where `from_raw` would free libgit2's static empty string.
///
/// [`BufMut::take_str`]: crate::libgit2::src::libgit2::buf::BufMut::take_str
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbDataStrFree;

/// One NUL-terminated string in libgit2's allocator.
pub type OdbDataStr = CStrBox<OdbDataStrFree>;

impl_cdrop_str!(
    /// Wraps `fn::git_odb_backend_data_free`
    #[doc(alias = "git_odb_backend_data_free")]
    OdbDataStrFree,
    str_free
);

impl_cdupclone_str!(OdbDataStrFree, str_dup_raw);

impl OdbDataStrFree {
    /// A copy of `s` in libgit2's allocator; `None` when the allocation fails.
    ///
    /// Wraps `fn::git_odb_backend_data_alloc`
    #[doc(alias = "git_odb_backend_data_alloc")]
    #[must_use]
    pub fn dup(s: &CStr) -> Option<OdbDataStr> {
        let copy = str_dup(s)?;
        // SAFETY: `copy` is fresh, uniquely-owned `git__malloc` storage holding `s`
        // with its terminator, which `str_free` releases.
        unsafe { OdbDataStr::from_raw(copy.as_ptr()) }
    }
}

// An `OdbDataStr` is a non-null `char *`: its policy is zero-sized, so a run of them
// is a `char **` array with no NULL element.
const _: () = assert!(
    size_of::<OdbDataStr>() == size_of::<*mut c_char>()
        && align_of::<OdbDataStr>() == align_of::<*mut c_char>()
);

/// A counted run of strings in libgit2's allocator: the `strings` / `count` pair of a
/// `git_strarray`, which `git_strarray_dispose` releases string by string, then as a
/// whole, with `git__free`.
///
/// Each element is an [`OdbDataStr`] under [`OdbDataStrFree`], and the run itself is
/// [`OdbDataVec`] storage under [`OdbDataArrayFree`]; dropping it frees every string
/// in order, then the array, through `git_odb_backend_data_free`. No element is NULL.
/// An empty run is NULL and allocates nothing.
///
/// [`dup_all`](Self::dup_all) and `Clone` copy every string, as `git_strarray_copy`
/// does; [`from_vec`](Self::from_vec) and [`into_vec`](Self::into_vec) move them
/// between the run and a `Vec`, which is where a run grows or shrinks.
#[repr(transparent)]
pub struct OdbDataStrVec(OdbDataVec<OdbDataStr>);

impl OdbDataStrVec {
    /// An empty run: NULL, nothing to free.
    #[must_use]
    pub const fn new() -> Self {
        Self(CVec::empty_with(OdbDataArrayFree))
    }

    /// Copies of `strs`, in order; `None` when an allocation fails, after releasing
    /// whatever was copied. An empty `strs` allocates nothing.
    pub fn dup_all<S: AsRef<CStr>>(strs: &[S]) -> Option<Self> {
        Self::dup_each(strs, |s| s.as_ref())
    }

    /// `strs` moved into a run in libgit2's allocator; `None`, dropping them, when the
    /// array cannot be allocated.
    #[must_use]
    pub fn from_vec(strs: Vec<OdbDataStr>) -> Option<Self> {
        let mut run = OdbDataArrayFree::new_uninit::<OdbDataStr>(strs.len())?;
        for (slot, s) in run.as_mut_slice().iter_mut().zip(strs) {
            slot.write(s);
        }
        // SAFETY: the run has `strs.len()` slots and the loop wrote each of them.
        Some(Self(unsafe { run.assume_init() }))
    }

    /// The strings moved out into a `Vec`, in order; the array is released.
    #[must_use]
    pub fn into_vec(self) -> Vec<OdbDataStr> {
        let mut out = Vec::with_capacity(self.len());
        let (strings, count) = self.into_storage().into_raw_parts();
        for i in 0..count {
            // SAFETY: `i < count`, and `strings` holds `count` elements, so the offset stays within
            // them.
            let slot = unsafe { strings.add(i) };
            // SAFETY: each of those is a live string the run gave up, moved out exactly once.
            out.push(unsafe { slot.read() });
        }
        if !strings.is_null() {
            // SAFETY: `strings` is the run's `git__malloc` array, now emptied of its
            // owners and released once.
            unsafe { data_free(strings.cast()) };
        }
        out
    }

    /// Takes ownership of `count` strings at `strings`, a `git_strarray`'s pair.
    ///
    /// NULL with `count == 0` is the empty run, as in a zeroed `git_strarray`. Returns
    /// `None`, leaving ownership with the caller, when `strings` is NULL with a
    /// nonzero `count` or any of its elements is NULL — which `git_strarray_copy`
    /// would silently skip; libgit2's own outputs hold none.
    ///
    /// # Safety
    ///
    /// A non-null `strings` must address `count` initialised `char *` elements in an
    /// array allocated by libgit2's current allocator — as `git_strarray_dispose`
    /// expects, possibly with spare capacity — and each non-null element a
    /// NUL-terminated string from the same allocator. The caller transfers unique
    /// ownership of the array and of every string, and does not use them again unless
    /// `None` is returned.
    pub unsafe fn from_raw_parts(strings: *mut *mut c_char, count: usize) -> Option<Self> {
        if strings.is_null() {
            return (count == 0).then(Self::new);
        }
        for i in 0..count {
            // SAFETY: `i < count`, and per this function's contract `strings` holds `count`
            // elements.
            let slot = unsafe { strings.add(i) };
            // SAFETY: as above, the element is initialised.
            if unsafe { slot.read() }.is_null() {
                return None;
            }
        }
        // SAFETY: `strings` is a `git__malloc` array of `count` non-null strings from
        // the same allocator, uniquely owned by the caller, who hands them over; an
        // `OdbDataStr` is a non-null `char *` that `str_free` releases.
        let run = unsafe { OdbDataVec::from_raw_parts(strings.cast::<OdbDataStr>(), count) };
        run.map(Self)
    }

    /// Gives up the run without freeing: the `strings` / `count` pair to place in a
    /// `git_strarray`, which `git_strarray_dispose` then releases.
    #[must_use = "the returned pointer owns the strings and must be freed"]
    pub fn into_raw_parts(self) -> (*mut *mut c_char, usize) {
        let (strings, count) = self.into_storage().into_raw_parts();
        (strings.cast(), count)
    }

    /// The array, for passing to C; NULL when empty. Ownership is retained.
    #[must_use]
    pub fn as_ptr(&self) -> *mut *mut c_char {
        self.0.as_ptr().cast()
    }

    /// Number of strings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the run holds no strings.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The strings.
    #[must_use]
    pub fn as_slice(&self) -> &[OdbDataStr] {
        // SAFETY: the run owns `len` live, non-null strings at `data()` (none, at an
        // aligned dangling pointer, when NULL); the slice is bound by `&self`.
        unsafe { core::slice::from_raw_parts(self.data(), self.len()) }
    }

    /// The strings, mutably: an element replaced through it drops the old string.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [OdbDataStr] {
        // SAFETY: as `as_slice`, with `&mut self` making it exclusive.
        unsafe { core::slice::from_raw_parts_mut(self.data(), self.len()) }
    }

    /// Copies every string; `None` on failure. See `Clone`.
    pub fn try_clone(&self) -> Option<Self> {
        Self::dup_each(self.as_slice(), OdbDataStr::as_c_str)
    }

    /// The elements' base for Rust-side views: dangling when the run is NULL.
    fn data(&self) -> *mut OdbDataStr {
        let strings = self.0.as_ptr();
        if strings.is_null() {
            NonNull::dangling().as_ptr()
        } else {
            strings
        }
    }

    /// The storage, its strings no longer dropped with the run.
    fn into_storage(self) -> OdbDataVec<OdbDataStr> {
        let this = ManuallyDrop::new(self);
        // SAFETY: the storage is read out exactly once and `self` is never dropped.
        unsafe { ptr::read(&raw const this.0) }
    }

    /// Copies of every item's string in `git_strarray_copy`'s order: the array first,
    /// then each string. A failed copy releases the copies made so far, then the
    /// array, as `git_strarray_dispose` does there.
    fn dup_each<T>(items: &[T], as_c_str: impl Fn(&T) -> &CStr) -> Option<Self> {
        let mut run = OdbDataArrayFree::new_uninit::<OdbDataStr>(items.len())?;
        let slots = run.as_mut_slice();
        for (i, item) in items.iter().enumerate() {
            let Some(copy) = OdbDataStrFree::dup(as_c_str(item)) else {
                for slot in &mut slots[..i] {
                    // SAFETY: the slots before `i` were written below, and each is
                    // dropped once, before the storage is released.
                    unsafe { slot.assume_init_drop() };
                }
                return None;
            };
            slots[i].write(copy);
        }
        // SAFETY: the run has `items.len()` slots and the loop wrote each of them.
        Some(Self(unsafe { run.assume_init() }))
    }
}

impl Drop for OdbDataStrVec {
    fn drop(&mut self) {
        // SAFETY: the run owns its strings, each dropped once here; the storage, which
        // the inner `OdbDataVec` frees right after, is not read again.
        unsafe { ptr::drop_in_place(self.as_mut_slice()) }
    }
}

impl Clone for OdbDataStrVec {
    /// Copies every string; aborts when an allocation fails, as `OdbDataStr`'s
    /// `Clone` does. [`try_clone`](Self::try_clone) reports the failure instead.
    fn clone(&self) -> Self {
        self.try_clone().unwrap_or_else(|| std::process::abort())
    }
}

impl Default for OdbDataStrVec {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for OdbDataStrVec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.as_slice().iter().map(OdbDataStr::as_c_str))
            .finish()
    }
}

// --- `git2/odb_backend.h`: `git_odb_stream`, `git_odb_writepack` -----------------------

/// A stream that writes one object into an object database, or reads one out of it
/// (`git_odb_stream`), as a backend implements it: the layout type named by
/// [`OdbStreamBox`] and the handles, never held by value.
///
/// A backend allocates the stream, typically as the first member of a container of
/// its own (`fake_wstream`, `loose_writestream` and `loose_readstream` in libgit2), and
/// sets its `backend`, `mode` and members; `git_odb_open_wstream` then gives a write
/// stream its hashing context, the odb's object id type and its declared size.
/// [`OdbRef::open_write_stream`] and [`OdbRef::open_read_stream`] open one, as an
/// [`OdbStreamBox`] that [`OdbStreamFree`] releases.
///
/// `'o` is the borrow of the odb whose backend made the stream. The stream keeps that
/// backend without a reference (`backend`), and finishing a write reaches the backend
/// and its odb (`git_odb_stream_finalize_write` reads `stream->backend->odb`; the
/// stream libgit2 fakes for a backend without streaming writes calls the backend's
/// `write`), so neither an owner nor a handle outlives the odb:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::src::libgit2::object::ObjectType;
/// use libgit2::libgit2::src::libgit2::odb::Odb;
///
/// let stream = {
///     let odb = Odb::new().unwrap();
///     odb.as_ref().open_write_stream(1, ObjectType::Blob).unwrap()
/// };
/// drop(stream);
/// ```
///
/// The lifetime only shrinks, and the exclusive handle is invariant in it, as `&mut` is
/// in its pointee:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::odb::OdbStreamMut;
///
/// fn shrink<'a, 'o>(m: OdbStreamMut<'a, 'static>) -> OdbStreamMut<'a, 'o> {
///     m
/// }
/// ```
///
/// Neither the owner nor the handles are `Send` or `Sync`: a stream is a backend's
/// container, which a custom backend may tie to its thread, and libgit2 states nothing
/// about moving one.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::odb::OdbStreamBox;
///
/// fn send<T: Send>(_: T) {}
/// fn forward(stream: OdbStreamBox<'static>) {
///     send(stream);
/// }
/// ```
///
/// There are no setters. libgit2 keeps the counters, the id type and the hashing
/// context for its own checks, and a backend may size its storage from them: the stream
/// libgit2 fakes for a backend without streaming writes copies each write into a
/// buffer of the declared size, trusting `git_odb_stream_write` to refuse what does not
/// fit. Its members belong to the container its backend cast the stream to.
///
/// Wraps `ty::git_odb_stream`
#[doc(alias = "git_odb_stream")]
#[repr(transparent)]
pub struct OdbStream<'o>(
    ffi::git_odb_stream,
    PhantomData<(*const (), &'o ffi::git_odb)>,
);

/// Shared borrow of an [`OdbStream`]: its getters. `Copy`, like `&T`.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct OdbStreamRef<'a, 'o>(CBorrowedPtr<'a, OdbStream<'o>>);

/// Exclusive borrow of an [`OdbStream`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the stream's reads and writes take it.
#[repr(transparent)]
pub struct OdbStreamMut<'a, 'o>(
    OdbStreamRef<'a, 'o>,
    // Invariant in the stream, and so in `'o`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut OdbStream<'o>>,
);

// SAFETY: `OdbStream` is `#[repr(transparent)]` over `git_odb_stream` with a zero-sized
// marker; both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, OdbStream<'o>>`
// (`OdbStreamMut` through `OdbStreamRef`) with zero-sized markers and no `Drop`, hand out
// no reference to the stream, and `OdbStreamRef` has no write operation. `OdbStreamMut`
// is invariant in the stream through its `PhantomData<&'a mut OdbStream<'o>>`.
unsafe impl<'o> CCell for OdbStream<'o> {
    type C = ffi::git_odb_stream;
    type Mut<'a>
        = OdbStreamMut<'a, 'o>
    where
        Self: 'a;
    type Ref<'a>
        = OdbStreamRef<'a, 'o>
    where
        Self: 'a;
}

impl<'a, 'o: 'a> CRefSeam<'a> for OdbStreamRef<'a, 'o> {
    type Cell = OdbStream<'o>;
}

impl<'a, 'o: 'a> CMutSeam<'a> for OdbStreamMut<'a, 'o> {
    type Cell = OdbStream<'o>;
}

impl<'o> OdbStreamMut<'_, 'o> {
    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> OdbStreamRef<'_, 'o> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> OdbStreamMut<'_, 'o> {
        OdbStreamMut(self.0, PhantomData)
    }
}

impl OdbStreamRef<'_, '_> {
    /// The backend that made the stream, which libgit2 hands back to the backend's own
    /// members; NULL only for a stream no backend set up.
    ///
    /// A raw pointer until `git_odb_backend` is wrapped: the backend belongs to the odb
    /// the stream borrows for `'o`, and is reached only through it. Nothing here
    /// dereferences it.
    ///
    /// Wraps `fld::git_odb_stream.backend`
    #[doc(alias = "git_odb_stream.backend")]
    #[must_use]
    pub fn backend(&self) -> *mut ffi::git_odb_backend {
        // SAFETY: `self.as_ptr()` addresses the live stream this handle borrows; the
        // projection forms no reference to it.
        let backend_ptr = unsafe { &raw const (*self.as_ptr()).backend };
        // SAFETY: a read through the shared handle's pointer to a live stream; any
        // value is a valid raw pointer.
        unsafe { backend_ptr.read() }
    }

    /// The stream's mode, as its backend set it: the bits of `git_odb_stream_t`,
    /// [`ffi::GIT_STREAM_RDONLY`] for a read stream and [`ffi::GIT_STREAM_WRONLY`] for a
    /// write stream, or 0 when the backend set none (libgit2's loose read stream does
    /// not).
    ///
    /// libgit2 never reads it: what a stream does is what its members do
    /// ([`can_read`](Self::can_read), [`can_write`](Self::can_write)).
    ///
    /// Wraps `fld::git_odb_stream.mode`
    #[doc(alias = "git_odb_stream.mode")]
    #[must_use]
    pub fn mode(&self) -> c_uint {
        // SAFETY: as in `backend`.
        let mode_ptr = unsafe { &raw const (*self.as_ptr()).mode };
        // SAFETY: a read through the shared handle's pointer to a live stream; any
        // value is a valid `c_uint`.
        unsafe { mode_ptr.read() }
    }

    /// Whether the stream holds a hashing context, which libgit2 owns and
    /// `git_odb_stream_free` releases.
    ///
    /// `git_odb_open_wstream` gives every write stream one, over which the object's id is
    /// computed as it is written; libgit2's loose read stream holds one too. The context
    /// is libgit2-private (`git_hash_ctx`), so only its presence is published.
    ///
    /// Wraps `fld::git_odb_stream.hash_ctx`
    #[doc(alias = "git_odb_stream.hash_ctx")]
    #[must_use]
    pub fn has_hash_ctx(&self) -> bool {
        // SAFETY: as in `backend`.
        let hash_ctx_ptr = unsafe { &raw const (*self.as_ptr()).hash_ctx };
        // SAFETY: a read through the shared handle's pointer to a live stream; any
        // value is a valid raw pointer, which is only compared with NULL.
        !unsafe { hash_ctx_ptr.read() }.is_null()
    }

    /// The type of the id a write stream computes, the odb's own
    /// (`git_odb_open_wstream` copies it); `Err` with the raw value for a type the
    /// header does not name, such as the 0 of a read stream, for which libgit2 sets
    /// none.
    ///
    /// # Errors
    ///
    /// [`UnnamedOidType`], carrying the raw value, when the stream holds no known id
    /// type.
    ///
    /// Wraps `fld::git_odb_stream.oid_type`
    #[doc(alias = "git_odb_stream.oid_type")]
    pub fn oid_type(&self) -> Result<OidType, UnnamedOidType> {
        // SAFETY: as in `backend`.
        let oid_type_ptr = unsafe { &raw const (*self.as_ptr()).oid_type };
        // SAFETY: a read through the shared handle's pointer to a live stream; any
        // value is a valid `git_oid_t`, an unsigned integer.
        OidType::try_from(unsafe { oid_type_ptr.read() })
    }

    /// The size a write stream was opened for, in bytes: the length of the object's
    /// content, which `git_odb_stream_finalize_write` requires to have been written in
    /// full. 0 for a read stream, for which libgit2 sets none.
    ///
    /// Wraps `fld::git_odb_stream.declared_size`
    #[doc(alias = "git_odb_stream.declared_size")]
    #[must_use]
    pub fn declared_size(&self) -> u64 {
        // SAFETY: as in `backend`.
        let declared_size_ptr = unsafe { &raw const (*self.as_ptr()).declared_size };
        // SAFETY: a read through the shared handle's pointer to a live stream; any
        // value is a valid `u64`.
        unsafe { declared_size_ptr.read() }
    }

    /// The bytes written into a write stream so far, as `git_odb_stream_write` counts
    /// them, refused writes included: it adds a write's length before checking it
    /// against [`declared_size`](Self::declared_size). 0 for a fresh write stream and
    /// for a read stream.
    ///
    /// Wraps `fld::git_odb_stream.received_bytes`
    #[doc(alias = "git_odb_stream.received_bytes")]
    #[must_use]
    pub fn received_bytes(&self) -> u64 {
        // SAFETY: as in `backend`.
        let received_bytes_ptr = unsafe { &raw const (*self.as_ptr()).received_bytes };
        // SAFETY: a read through the shared handle's pointer to a live stream; any
        // value is a valid `u64`.
        unsafe { received_bytes_ptr.read() }
    }

    /// Whether the stream can be read: whether its backend set its `read` member, as
    /// libgit2's read streams do and its write streams do not.
    /// `git_odb_stream_read` calls the member without checking.
    ///
    /// Wraps `fld::git_odb_stream.read`
    #[doc(alias = "git_odb_stream.read")]
    #[must_use]
    pub fn can_read(&self) -> bool {
        // SAFETY: as in `backend`.
        let read_ptr = unsafe { &raw const (*self.as_ptr()).read };
        // SAFETY: a read through the shared handle's pointer to a live stream: an
        // optional function pointer, which C leaves NULL or sets.
        unsafe { read_ptr.read() }.is_some()
    }

    /// Whether the stream can be written: whether its backend set its `write` member,
    /// as libgit2's write streams do and its read streams do not.
    /// `git_odb_stream_write` calls the member without checking.
    ///
    /// Wraps `fld::git_odb_stream.write`
    #[doc(alias = "git_odb_stream.write")]
    #[must_use]
    pub fn can_write(&self) -> bool {
        // SAFETY: as in `backend`.
        let write_ptr = unsafe { &raw const (*self.as_ptr()).write };
        // SAFETY: as in `can_read`.
        unsafe { write_ptr.read() }.is_some()
    }

    /// Whether the stream can store what was written: whether its backend set its
    /// `finalize_write` member, as libgit2's write streams do and its read streams do
    /// not. `git_odb_stream_finalize_write` calls the member without checking, unless
    /// the object already exists.
    ///
    /// Wraps `fld::git_odb_stream.finalize_write`
    #[doc(alias = "git_odb_stream.finalize_write")]
    #[must_use]
    pub fn can_finalize_write(&self) -> bool {
        // SAFETY: as in `backend`.
        let finalize_write_ptr = unsafe { &raw const (*self.as_ptr()).finalize_write };
        // SAFETY: as in `can_read`.
        unsafe { finalize_write_ptr.read() }.is_some()
    }
}

impl fmt::Debug for OdbStreamRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("OdbStreamRef");
        s.field("mode", &self.mode());
        match self.oid_type() {
            Ok(oid_type) => s.field("oid_type", &oid_type),
            Err(raw) => s.field("oid_type", &raw.raw()),
        };
        s.field("declared_size", &self.declared_size())
            .field("received_bytes", &self.received_bytes())
            .field("can_read", &self.can_read())
            .field("can_write", &self.can_write())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for OdbStreamMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases a stream through `git_odb_stream_free`: its hashing context, then the
/// stream itself through its own `free` member, which releases the backend's container
/// (a loose write stream's temporary file, a read stream's mapping and inflater, the
/// fake stream's buffer). Stateless: the routine is recovered from the object.
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbStreamFree;

// SAFETY: `git_odb_stream_free` releases exactly the stream it is given, once: the
// hashing context libgit2 allocated for it (cleaned up, then `git__free`d; NULL is
// accepted), then the container through the member its backend set. Every owner holds
// a stream that a backend made, with its `free` member set, and whose context is NULL
// or libgit2's (`OdbStreamBox::from_raw`'s caller vouches for that). It reads no state
// beyond the stream; the backend's member reaches the backend at most, which `'o`
// keeps alive.
unsafe impl<'o> CDrop<OdbStream<'o>> for OdbStreamFree {
    /// Wraps `fn::git_odb_stream_free`
    ///
    /// Wraps `fld::git_odb_stream.free`
    #[doc(alias = "git_odb_stream_free")]
    #[doc(alias = "git_odb_stream.free")]
    unsafe fn c_drop(&self, ptr: NonNull<OdbStream<'o>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live stream the caller owns
        // and gives up; `OdbStream` is transparent over `git_odb_stream`.
        unsafe { ffi::git_odb_stream_free(ptr.as_ptr().cast()) }
    }
}

/// The sole owner of a stream, released through [`OdbStreamFree`]; it borrows the odb
/// whose backend made it for `'o`.
///
/// [`OdbRef::open_write_stream`] and [`OdbRef::open_read_stream`] build one. It is not
/// `Clone`: libgit2 copies no stream.
pub type OdbStreamBox<'o> = CBox<OdbStream<'o>, OdbStreamFree>;

// The layout type is the C type's size, so a stream pointer converts both ways, and the
// owner is pointer-sized.
const _: () = assert!(size_of::<OdbStream<'static>>() == size_of::<ffi::git_odb_stream>());

const _: () = assert!(size_of::<OdbStreamBox<'static>>() == size_of::<*mut ffi::git_odb_stream>());

/// What [`OdbRef::open_read_stream`] opens: the stream, and the object's length and type
/// as its backend reported them.
#[derive(Debug)]
pub struct OdbReadStream<'o> {
    /// The stream, positioned at the start of the object's content.
    pub stream: OdbStreamBox<'o>,
    /// The length of the object's content, in bytes.
    pub len: usize,
    /// The object's type; `None` for a value the header does not name, which a custom
    /// backend may report.
    pub kind: Option<ObjectType>,
}

impl<'a> OdbRef<'a, '_> {
    /// Opens a stream that writes one object of `kind`, whose content is `size` bytes
    /// long, into the odb.
    ///
    /// The first backend that writes (alternates are skipped) makes the stream, in
    /// priority order: through its own streaming writes, or else through a stream
    /// libgit2 fakes over its `write`, which buffers the whole object. libgit2 then
    /// starts hashing the object's id, over the odb's id type, and records `size`: the
    /// stream's writes must add up to exactly that before it can store the object.
    /// The stream borrows the odb.
    ///
    /// # Errors
    ///
    /// The code and libgit2's error of the backend that failed, as C returns them; a
    /// `GIT_ERROR_ODB` error when no backend writes (an odb without backends, or one with
    /// only read-only ones), or when every backend that writes passes the request on
    /// (`GIT_PASSTHROUGH`), for which libgit2 reports success without a stream.
    ///
    /// Wraps `fn::git_odb_open_wstream`
    #[doc(alias = "git_odb_open_wstream")]
    pub fn open_write_stream(
        &self,
        size: u64,
        kind: ObjectType,
    ) -> Result<OdbStreamBox<'a>, StreamError> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, which the routine fills with a fresh stream
        // on success; the odb is live for `'a` and locks itself while it picks a
        // backend; `kind` is a valid object type and `size` any value.
        let rc = unsafe {
            ffi::git_odb_open_wstream(&raw mut out, self.as_ptr().cast_mut(), size, kind.raw())
        };
        if rc < 0 {
            return Err(StreamError::from_libgit2(rc));
        }
        // SAFETY: the routine succeeded, so `out` is NULL (every backend passed) or a
        // stream its backend made for this odb, with its `free` member set and a hashing
        // context libgit2 allocated, which the caller owns and `git_odb_stream_free`
        // releases. It reaches the odb's backend, so it borrows the odb for `'a`.
        unsafe { OdbStreamBox::from_c(out.cast()) }.ok_or_else(passed)
    }

    /// Opens a stream that reads the object `id` out of the odb, with the object's length
    /// and type.
    ///
    /// Only backends that stream reads can open one: libgit2's loose objects do, its
    /// packfiles do not. The stream borrows the odb.
    ///
    /// # Errors
    ///
    /// The code and libgit2's error of the backend that failed, as C returns them:
    /// `GIT_ENOTFOUND` when no backend that streams reads has the object. A
    /// `GIT_ERROR_ODB` error when no backend streams reads, or when every one passes the
    /// request on (`GIT_PASSTHROUGH`), for which libgit2 reports success without a
    /// stream.
    ///
    /// Wraps `fn::git_odb_open_rstream`
    #[doc(alias = "git_odb_open_rstream")]
    pub fn open_read_stream(&self, id: OidRef<'_>) -> Result<OdbReadStream<'a>, StreamError> {
        let mut out = ptr::null_mut();
        let mut len = 0;
        let mut kind = ffi::GIT_OBJECT_INVALID;
        // SAFETY: the out slots are writable, and filled on success; `id` is a live
        // `git_oid`, read for the call only; the odb is live for `'a` and locks itself
        // while it picks a backend.
        let rc = unsafe {
            ffi::git_odb_open_rstream(
                &raw mut out,
                &raw mut len,
                &raw mut kind,
                self.as_ptr().cast_mut(),
                id.as_ptr(),
            )
        };
        if rc < 0 {
            return Err(StreamError::from_libgit2(rc));
        }
        // SAFETY: as in `open_write_stream`, for a stream its backend made to read from
        // this odb; its hashing context is NULL or one libgit2 allocated (the loose
        // backend's), which `git_odb_stream_free` releases.
        let stream = unsafe { OdbStreamBox::from_c(out.cast()) }.ok_or_else(passed)?;
        Ok(OdbReadStream {
            stream,
            len,
            kind: ObjectType::from_raw(kind),
        })
    }
}

/// The failure of an open that libgit2 reports as success without a stream: every
/// backend passed the request on.
fn passed() -> StreamError {
    StreamError::new(
        ErrorClass::Odb,
        c"every backend passed on opening the stream",
    )
    .with_code(ErrorCode::Passthrough)
}

/// A pack being written into an object database (`git_odb_writepack`), as a backend
/// implements it: the layout type named by [`OdbWritepackBox`] and the handles, never
/// held by value.
///
/// libgit2's pack backend makes one, as the first member of a container holding an
/// indexer (`pack_writepack`); a fetch or clone streams the downloaded pack into it,
/// [`append`](OdbWritepackMut::append) by append, and [`commit`](Self::commit)s it, which
/// moves the pack and its index into the odb. The wrapper of `git_odb_write_pack`
/// opens one; dropping it unfinished discards the pack.
///
/// `'o` is the borrow of the odb whose backend made the writepack: its indexer reads
/// that odb to resolve the bases of a thin pack, and it keeps the backend without a
/// reference, so neither an owner nor a handle outlives the odb. The exclusive handle
/// is invariant in `'o`:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::odb::OdbWritepackMut;
///
/// fn shrink<'a, 'o>(m: OdbWritepackMut<'a, 'static>) -> OdbWritepackMut<'a, 'o> {
///     m
/// }
/// ```
///
/// Neither the owner nor the handles are `Send` or `Sync`: an indexer runs its
/// progress callback, which may be tied to its thread, from whichever thread appends.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::odb::OdbWritepackBox;
///
/// fn send<T: Send>(_: T) {}
/// fn forward(writepack: OdbWritepackBox<'static>) {
///     send(writepack);
/// }
/// ```
///
/// There are no setters: the members belong to the container their backend cast the
/// writepack to.
///
/// Wraps `ty::git_odb_writepack`
#[doc(alias = "git_odb_writepack")]
#[repr(transparent)]
pub struct OdbWritepack<'o>(
    ffi::git_odb_writepack,
    PhantomData<(*const (), &'o ffi::git_odb)>,
);

/// Shared borrow of an [`OdbWritepack`]: its getters. `Copy`, like `&T`.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct OdbWritepackRef<'a, 'o>(CBorrowedPtr<'a, OdbWritepack<'o>>);

/// Exclusive borrow of an [`OdbWritepack`]: what [`append`](Self::append) takes. Reaches
/// the getters with [`as_ref`](Self::as_ref).
#[repr(transparent)]
pub struct OdbWritepackMut<'a, 'o>(
    OdbWritepackRef<'a, 'o>,
    // Invariant in the writepack, and so in `'o`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut OdbWritepack<'o>>,
);

// SAFETY: as for `OdbStream`: `OdbWritepack` is `#[repr(transparent)]` over
// `git_odb_writepack` with a zero-sized marker; both handles are `#[repr(transparent)]`
// over `CBorrowedPtr<'a, OdbWritepack<'o>>` with zero-sized markers and no `Drop`, hand
// out no reference to the writepack, and `OdbWritepackRef` has no write operation.
// `OdbWritepackMut` is invariant in the writepack through its `PhantomData`.
unsafe impl<'o> CCell for OdbWritepack<'o> {
    type C = ffi::git_odb_writepack;
    type Mut<'a>
        = OdbWritepackMut<'a, 'o>
    where
        Self: 'a;
    type Ref<'a>
        = OdbWritepackRef<'a, 'o>
    where
        Self: 'a;
}

impl<'a, 'o: 'a> CRefSeam<'a> for OdbWritepackRef<'a, 'o> {
    type Cell = OdbWritepack<'o>;
}

impl<'a, 'o: 'a> CMutSeam<'a> for OdbWritepackMut<'a, 'o> {
    type Cell = OdbWritepack<'o>;
}

impl<'o> OdbWritepackMut<'_, 'o> {
    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> OdbWritepackRef<'_, 'o> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> OdbWritepackMut<'_, 'o> {
        OdbWritepackMut(self.0, PhantomData)
    }

    /// Appends `data`, the next piece of the pack, and updates `stats` with the
    /// indexer's progress: its `append` member, given the slice's pointer and length.
    ///
    /// libgit2's pack backend writes the piece to its temporary packfile and indexes
    /// the objects it completes, calling the progress callback the writepack was opened
    /// with. The writepack reads `data` during the call only. A failed append leaves the
    /// pack incomplete; libgit2's own callers stop there and drop the writepack.
    ///
    /// # Errors
    ///
    /// The member's negative return and the thread's libgit2 error: a malformed pack
    /// (`GIT_ERROR_INDEXER`), a write that failed, or the code a progress callback
    /// returned to stop. A `GIT_ERROR_ODB` error, without calling anything, when the
    /// backend set no `append` member.
    ///
    /// Wraps `fld::git_odb_writepack.append`
    #[doc(alias = "git_odb_writepack.append")]
    pub fn append(
        &mut self,
        data: &[u8],
        mut stats: IndexerProgressMut<'_>,
    ) -> Result<(), StreamError> {
        let writepack = self.as_mut_ptr();
        // SAFETY: `writepack` is the handle's pointer to the live writepack it borrows;
        // the projection forms no reference to it.
        let append_ptr = unsafe { &raw const (*writepack).append };
        // SAFETY: a read through the exclusive handle's pointer: an optional function
        // pointer, which C leaves NULL or sets.
        let Some(append) = (unsafe { append_ptr.read() }) else {
            return Err(missing_member(c"the writepack has no append member"));
        };
        // SAFETY: `append` is the member the writepack's backend set for this
        // writepack, in place in its container; it reads `data.len()` bytes at `data`
        // (never NULL, even when empty) during the call only, and writes the progress
        // through `stats`, a live, exclusively borrowed `git_indexer_progress`. The
        // handle is exclusive, so no other call on the writepack runs meanwhile.
        let rc = unsafe {
            append(
                writepack,
                data.as_ptr().cast(),
                data.len(),
                stats.as_mut_ptr(),
            )
        };
        member_result(rc)
    }

    /// Commits the pack: its `commit` member.
    ///
    /// # Safety
    ///
    /// Committing ends the writepack: libgit2's pack backend closes the packfile and
    /// moves it, so afterwards the writepack must not be appended to or committed again,
    /// only freed.
    unsafe fn commit(&mut self, mut stats: IndexerProgressMut<'_>) -> Result<(), StreamError> {
        let writepack = self.as_mut_ptr();
        // SAFETY: as in `append`.
        let commit_ptr = unsafe { &raw const (*writepack).commit };
        // SAFETY: as in `append`.
        let Some(commit) = (unsafe { commit_ptr.read() }) else {
            return Err(missing_member(c"the writepack has no commit member"));
        };
        // SAFETY: as in `append`, for the `commit` member, which writes the final
        // progress through `stats`; the caller ends the writepack.
        let rc = unsafe { commit(writepack, stats.as_mut_ptr()) };
        member_result(rc)
    }
}

impl OdbWritepackRef<'_, '_> {
    /// The backend that made the writepack, which libgit2 hands back to the backend's
    /// own members; NULL only for a writepack no backend set up.
    ///
    /// A raw pointer until `git_odb_backend` is wrapped: the backend belongs to the odb
    /// the writepack borrows for `'o`, and is reached only through it. Nothing here
    /// dereferences it.
    ///
    /// Wraps `fld::git_odb_writepack.backend`
    #[doc(alias = "git_odb_writepack.backend")]
    #[must_use]
    pub fn backend(&self) -> *mut ffi::git_odb_backend {
        // SAFETY: `self.as_ptr()` addresses the live writepack this handle borrows; the
        // projection forms no reference to it.
        let backend_ptr = unsafe { &raw const (*self.as_ptr()).backend };
        // SAFETY: a read through the shared handle's pointer to a live writepack; any
        // value is a valid raw pointer.
        unsafe { backend_ptr.read() }
    }
}

impl fmt::Debug for OdbWritepackRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OdbWritepackRef")
            .field(&self.as_ptr())
            .finish()
    }
}

impl fmt::Debug for OdbWritepackMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl OdbWritepack<'_> {
    /// Commits the pack, then frees the writepack: its `commit` member, then its `free`
    /// member.
    ///
    /// libgit2's pack backend checks the pack's trailer, resolves its deltas, writes its
    /// index and moves both into the odb's pack directory, updating `stats` with the
    /// final progress. Committing ends a writepack, so the owner is consumed and freed
    /// whether or not the commit succeeds; a failed commit leaves nothing in the odb.
    ///
    /// # Errors
    ///
    /// The member's negative return and the thread's libgit2 error: an incomplete pack
    /// (`GIT_ERROR_INDEXER`, `incomplete pack header`), unexpected or missing trailing
    /// data, a checksum mismatch, a write or rename that failed, or the code a progress
    /// callback returned to stop. A `GIT_ERROR_ODB` error, without calling anything,
    /// when the backend set no `commit` member.
    ///
    /// Wraps `fld::git_odb_writepack.commit`
    #[doc(alias = "git_odb_writepack.commit")]
    pub fn commit(
        mut writepack: OdbWritepackBox<'_>,
        stats: IndexerProgressMut<'_>,
    ) -> Result<(), StreamError> {
        // SAFETY: `writepack` is dropped right below, so it is freed and never appended
        // to or committed again.
        let committed = unsafe { writepack.as_mut().commit(stats) };
        drop(writepack);
        committed
    }
}

/// Releases a writepack through its own `free` member, which frees the container it is
/// embedded in: the pack backend's indexer, with the temporary packfile of a pack never
/// committed, and the allocation. Stateless: the routine is recovered from the object.
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbWritepackFree;

// SAFETY: the member releases exactly the writepack it is given, once, and reads no
// state beyond it; it reaches the backend and odb at most, which `'o` keeps alive. Every
// owner holds a writepack its backend made, with its members set (the adopter vouches).
unsafe impl<'o> CDrop<OdbWritepack<'o>> for OdbWritepackFree {
    /// Wraps `fld::git_odb_writepack.free`
    #[doc(alias = "git_odb_writepack.free")]
    unsafe fn c_drop(&self, ptr: NonNull<OdbWritepack<'o>>) {
        let writepack: *mut ffi::git_odb_writepack = ptr.as_ptr().cast();
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live writepack the caller
        // owns; `OdbWritepack` is transparent over `git_odb_writepack`. The projection
        // forms no reference to it.
        let free_ptr = unsafe { &raw const (*writepack).free };
        // SAFETY: reads the initialised member.
        let free = unsafe { free_ptr.read() };
        // A writepack without one is left alone: no backend leaves the member unset.
        if let Some(free) = free {
            // SAFETY: `free` is the member the writepack's backend set for it; it releases
            // the container (the pack backend's indexer, its temporary packfile unless
            // committed, and the allocation), which the caller gives up.
            unsafe { free(writepack) }
        }
    }
}

/// The sole owner of a writepack, released through [`OdbWritepackFree`]; it borrows the
/// odb whose backend made it for `'o`.
///
/// The wrapper of `git_odb_write_pack` builds one; [`OdbWritepack::commit`] consumes
/// it. It is not `Clone`: libgit2 copies no writepack.
pub type OdbWritepackBox<'o> = CBox<OdbWritepack<'o>, OdbWritepackFree>;

const _: () = assert!(size_of::<OdbWritepack<'static>>() == size_of::<ffi::git_odb_writepack>());

const _: () =
    assert!(size_of::<OdbWritepackBox<'static>>() == size_of::<*mut ffi::git_odb_writepack>());

/// The failure of a call on a member its backend did not set.
fn missing_member(message: &CStr) -> StreamError {
    StreamError::new(ErrorClass::Odb, message)
}

/// A member's return as a result: negative is an error, described by the thread's last
/// libgit2 error; anything else is success, as libgit2's callers read it.
fn member_result(rc: c_int) -> Result<(), StreamError> {
    if rc < 0 {
        Err(StreamError::from_libgit2(rc))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod unit_tests {
    use std::ffi::{c_char, c_int, c_void};
    use std::process::Command;
    use std::sync::atomic::Ordering::SeqCst;
    use std::sync::atomic::{AtomicIsize, AtomicUsize};

    use ffibox::CRefSeam;

    use super::*;

    #[repr(C)]
    #[expect(
        unreachable_pub,
        reason = "`define_ctype!` makes its newtype `pub`, which names this layout"
    )]
    pub struct pair_st {
        a: u64,
        b: u32,
    }
    ffibox::define_ctype!(Pair, PairRef, PairMut, pair_st);

    impl PairRef<'_> {
        fn a(&self) -> u64 {
            // SAFETY: a read through the shared handle's pointer to a live `pair_st`.
            let value = unsafe { ptr::addr_of!((*self.as_ptr()).a) };
            // SAFETY: as above.
            unsafe { value.read() }
        }

        fn b(&self) -> u32 {
            // SAFETY: as above.
            let value = unsafe { ptr::addr_of!((*self.as_ptr()).b) };
            // SAFETY: as above.
            unsafe { value.read() }
        }
    }

    /// The raw C calls below need the runtime the strategies pin.
    fn init() {
        assert!(runtime_pinned());
    }

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

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// The one request size the refusing allocator turns down.
    const REFUSED: usize = 12_345;

    unsafe extern "C" fn refusing_malloc(n: usize, _: *const c_char, _: c_int) -> *mut c_void {
        if n == REFUSED {
            ptr::null_mut()
        } else {
            // SAFETY: plain libc allocation.
            unsafe { malloc(n) }
        }
    }

    unsafe extern "C" fn libc_realloc(
        p: *mut c_void,
        n: usize,
        _: *const c_char,
        _: c_int,
    ) -> *mut c_void {
        // SAFETY: `p` came from this allocator, i.e. libc, or is NULL.
        unsafe { realloc(p, n) }
    }

    unsafe extern "C" fn libc_free(p: *mut c_void) {
        // SAFETY: as above.
        unsafe { free(p) }
    }

    /// Corrects C: `git_odb_backend_data_alloc` before `git_libgit2_init` reports its
    /// failure through TLS key 0, which `ASan` flags as a wild write.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failure_before_init_lands_on_live_thread_state() {
        isolated(
            concat!(
                module_path!(),
                "::failure_before_init_lands_on_live_thread_state"
            ),
            || {
                let mut allocator = ffi::git_allocator {
                    gmalloc: Some(refusing_malloc),
                    grealloc: Some(libc_realloc),
                    gfree: Some(libc_free),
                };
                // SAFETY: a `git_allocator *` with every member set, which libgit2 copies;
                // nothing is allocated yet, so no storage changes allocator.
                let rc = unsafe {
                    ffi::git_libgit2_opts(
                        ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                        &raw mut allocator,
                    )
                };
                assert_eq!(rc, 0);

                assert!(OdbDataArrayFree::new_uninit::<u8>(REFUSED).is_none());
                // SAFETY: `git_error_last` never returns NULL.
                let error_last = unsafe { ffi::git_error_last() };
                // SAFETY: as above.
                let klass = unsafe { (*error_last).klass };
                assert_eq!(
                    klass,
                    ffi::GIT_ERROR_NOMEMORY.cast_signed(),
                    "reported on this thread's state"
                );
                // SAFETY: no arguments; the extra reference dies with the process.
                let ret = unsafe { ffi::git_libgit2_init() };
                assert!(ret >= 2, "the strategy holds a reference");
                assert!(OdbDataVec::<u8>::from_slice(b"pinned").is_some());
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn vec_adopts_a_c_allocation() {
        init();
        // SAFETY: `backend` is unused by the routine.
        let raw = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), 4) }.cast::<u8>();
        assert!(!raw.is_null());
        // SAFETY: `raw` holds 4 writable bytes.
        unsafe { raw.copy_from_nonoverlapping(b"odb!".as_ptr(), 4) };
        // SAFETY: 4 initialised bytes of fresh `git__malloc` storage, now owned here.
        let v = unsafe { OdbDataVec::<u8>::from_raw_parts(raw, 4) }.unwrap();
        assert_eq!(v.as_slice(), b"odb!");
        // `git_odb_backend_data_free` runs here; ASan flags a mismatch or a leak.
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn vec_into_raw_parts_is_released_by_c() {
        init();
        let v = OdbDataVec::<u16>::from_slice(&[1, 2, 3]).unwrap();
        let (data, len) = v.into_raw_parts();
        assert_eq!(len, 3);
        // SAFETY: `data` is the `git__malloc` buffer `v` gave up, released once here.
        unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), data.cast()) };
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn vec_uninit_promoted_after_filling() {
        init();
        let mut v = OdbDataArrayFree::new_uninit::<u32>(4).unwrap();
        for (i, slot) in v.as_mut_slice().iter_mut().enumerate() {
            slot.write(u32::try_from(i).unwrap() * 7);
        }
        // SAFETY: every element was written above.
        let v = unsafe { v.assume_init() };
        assert_eq!(v.as_slice(), &[0, 7, 14, 21]);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn box_uninit_filled_through_the_c_pointer() {
        init();
        let mut slot = OdbDataBox::<Pair>::new_uninit().unwrap();
        let c = slot.as_c_ptr();
        // SAFETY: `c` addresses fresh storage for one `pair_st`, written whole.
        unsafe { c.write(pair_st { a: u64::MAX, b: 9 }) };
        // SAFETY: the `pair_st` was fully written above and owns no resources.
        let pair = unsafe { slot.assume_init() };
        assert_eq!((pair.as_ref().a(), pair.as_ref().b()), (u64::MAX, 9));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn box_adopts_and_returns_a_c_allocation() {
        init();
        // SAFETY: `backend` is unused by the routine.
        let raw = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), size_of::<pair_st>()) }
            .cast::<pair_st>();
        assert!(!raw.is_null());
        // SAFETY: `raw` is fresh storage for one `pair_st`, malloc-aligned.
        unsafe { raw.write(pair_st { a: 1, b: 2 }) };
        // SAFETY: an initialised `pair_st` in `git__malloc` storage, now owned here.
        let pair = unsafe { OdbDataBox::<Pair>::from_c(raw) }.unwrap();
        assert_eq!(pair.as_ref().b(), 2);
        let raw = pair.into_c();
        // SAFETY: the storage `pair` gave up, released once here.
        unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), raw.cast()) };
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn dup_copies_and_never_asks_for_zero_bytes() {
        init();
        let mut src = *b"payload";
        // SAFETY: `src` is readable for its length.
        let copy = unsafe { data_dup_len(src.as_mut_ptr(), src.len()) };
        assert!(!copy.is_null() && copy != src.as_mut_ptr());
        // SAFETY: `copy` holds `src.len()` bytes copied from `src`.
        let ret = unsafe { core::slice::from_raw_parts(copy, src.len()) };
        assert_eq!(ret, &src);
        // SAFETY: `copy` is a live allocation owned here.
        unsafe { data_free(copy) };

        // SAFETY: zero bytes are trivially readable.
        let empty = unsafe { data_dup_len(src.as_mut_ptr(), 0) };
        assert!(
            !empty.is_null(),
            "a zero-byte copy is a fresh allocation, not a failure"
        );
        // SAFETY: as above.
        unsafe { data_free(empty) };
    }

    #[test]
    fn oversized_runs_are_rejected_before_allocating() {
        assert!(OdbDataArrayFree::new_uninit::<u64>(usize::MAX).is_none());
        assert!(OdbDataArrayFree::new_zeroed::<u16>(isize::MAX as usize / 2 + 1).is_none());
    }

    #[test]
    fn empty_runs_allocate_nothing() {
        let v = OdbDataArrayFree::new_uninit::<u8>(0).unwrap();
        assert!(v.is_empty() && v.as_ptr().is_null());
        let z = OdbDataArrayFree::new_zeroed::<u32>(0).unwrap();
        assert!(z.as_ptr().is_null());
        let s = OdbDataVec::<u8>::from_slice(&[]).unwrap();
        assert!(s.clone().as_ptr().is_null());
    }

    #[test]
    fn policies_are_stateless_values() {
        // Both policies carry no state, so an owner is pointer-sized (plus length)
        // and the policy can be named, copied and printed freely.
        assert_eq!(size_of::<OdbDataBox<Pair>>(), size_of::<*mut pair_st>());
        assert_eq!(
            size_of::<Option<OdbDataBox<Pair>>>(),
            size_of::<*mut pair_st>()
        );
        assert_eq!(size_of::<OdbDataVec<u8>>(), 2 * size_of::<usize>());
        let (one, run) = (OdbDataFree, OdbDataArrayFree);
        let (copy_one, copy_run) = (one, run);
        assert_eq!(format!("{one:?}{copy_one:?}"), "OdbDataFreeOdbDataFree");
        assert_eq!(
            format!("{run:?}{copy_run:?}"),
            "OdbDataArrayFreeOdbDataArrayFree"
        );
        let v = OdbDataVec::<u16>::default();
        assert!(v.is_empty() && v.as_ptr().is_null());
        assert_eq!(format!("{v:?}"), "CVec { ptr: 0x0, len: 0 }");
        let (ptr, len) = OdbDataVec::<u16>::empty_with(copy_run).into_raw_parts();
        assert!(ptr.is_null() && len == 0);
    }

    #[test]
    fn data_align_covers_libgit2_scalars() {
        assert!(DATA_ALIGN >= align_of::<*const ()>() && DATA_ALIGN >= align_of::<u64>());
        assert!(DATA_ALIGN.is_power_of_two());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_dup_copies_the_terminator() {
        init();
        let copy = str_dup(c"refs/heads/main").unwrap();
        // SAFETY: `copy` holds the 16 bytes copied from the literal, terminator included.
        let bytes = unsafe { core::slice::from_raw_parts(copy.as_ptr().cast::<u8>(), 16) };
        assert_eq!(bytes, b"refs/heads/main\0");
        // SAFETY: `copy` is a live allocation owned here.
        unsafe { str_free(copy.as_ptr()) };

        let empty = str_dup(c"").unwrap();
        // SAFETY: `empty` holds one byte, the terminator.
        assert_eq!(unsafe { empty.as_ptr().read() }, 0);
        // SAFETY: as above.
        unsafe { str_free(empty.as_ptr()) };
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_dup_raw_matches_the_source() {
        init();
        let mut src = *b"user.name\0";
        // SAFETY: `src` is NUL-terminated and outlives the call.
        let copy = unsafe { str_dup_raw(src.as_mut_ptr().cast()) };
        assert!(!copy.is_null() && copy != src.as_mut_ptr().cast());
        // SAFETY: `copy` is a live NUL-terminated string.
        assert_eq!(unsafe { CStr::from_ptr(copy) }, c"user.name");
        // SAFETY: owned here, released once.
        unsafe { str_free(copy) };
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_adopts_a_c_allocation_and_returns_it() {
        init();
        // SAFETY: `backend` is unused by the routine.
        let raw = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), 4) }.cast::<c_char>();
        assert!(!raw.is_null());
        // SAFETY: `raw` holds 4 writable bytes.
        unsafe { raw.copy_from_nonoverlapping(c"odb".as_ptr(), 4) };
        // SAFETY: a NUL-terminated string in fresh `git__malloc` storage, now owned here.
        let s = unsafe { OdbDataStr::from_raw(raw) }.unwrap();
        assert_eq!(s.to_str(), Ok("odb"));
        let t = s.clone();
        assert_ne!(s.as_ptr(), t.as_ptr());
        let raw = s.into_raw();
        // SAFETY: the string `s` gave up, released once here.
        unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), raw.cast()) };
        assert_eq!(t.as_bytes(), b"odb");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_vec_into_vec_releases_only_the_array() {
        init();
        let v = OdbDataStrVec::dup_all(&[c"a", c"bc", c"def"]).unwrap();
        let ptrs: Vec<_> = v.as_slice().iter().map(OdbDataStr::as_ptr).collect();
        let strs = v.into_vec();
        assert_eq!(
            strs.iter().map(OdbDataStr::as_ptr).collect::<Vec<_>>(),
            ptrs
        );
        assert_eq!(strs[2].as_bytes(), b"def");
        let back = OdbDataStrVec::from_vec(strs).unwrap();
        assert_eq!(
            back.as_slice()
                .iter()
                .map(OdbDataStr::as_ptr)
                .collect::<Vec<_>>(),
            ptrs
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_vec_adopts_its_own_raw_parts() {
        init();
        let v = OdbDataStrVec::dup_all(&[c"x", c"y"]).unwrap();
        let (strings, count) = v.into_raw_parts();
        // SAFETY: the pair `v` gave up: a `git__malloc` array of two owned strings.
        let w = unsafe { OdbDataStrVec::from_raw_parts(strings, count) }.unwrap();
        assert_eq!(w.as_ptr(), strings);
        assert_eq!(format!("{w:?}"), r#"["x", "y"]"#);
    }

    static LIVE: AtomicIsize = AtomicIsize::new(0);
    /// Fails the `n`th allocation from now when `n > 0`.
    static FAIL_IN: AtomicUsize = AtomicUsize::new(0);

    /// Counts `FAIL_IN` down; `true` for the allocation it names.
    fn refuse_now() -> bool {
        let mut k = FAIL_IN.load(SeqCst);
        while k > 0 {
            match FAIL_IN.compare_exchange(k, k - 1, SeqCst, SeqCst) {
                Ok(_) => return k == 1,
                Err(now) => k = now,
            }
        }
        false
    }

    unsafe extern "C" fn counting_malloc(n: usize, _: *const c_char, _: c_int) -> *mut c_void {
        if refuse_now() {
            return ptr::null_mut();
        }
        LIVE.fetch_add(1, SeqCst);
        // SAFETY: plain libc allocation.
        unsafe { malloc(n) }
    }

    unsafe extern "C" fn counting_free(p: *mut c_void) {
        if !p.is_null() {
            LIVE.fetch_sub(1, SeqCst);
        }
        // SAFETY: `p` came from this allocator, i.e. libc, or is NULL.
        unsafe { free(p) }
    }

    /// A copy failing at any allocation releases the copies made so far and the
    /// array, and a failing `try_clone` leaves its source intact.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_vec_failed_copy_releases_what_it_made() {
        isolated(
            concat!(
                module_path!(),
                "::str_vec_failed_copy_releases_what_it_made"
            ),
            || {
                let mut allocator = ffi::git_allocator {
                    gmalloc: Some(counting_malloc),
                    grealloc: Some(libc_realloc),
                    gfree: Some(counting_free),
                };
                // SAFETY: a `git_allocator *` with every member set, which libgit2 copies;
                // nothing is allocated yet, and nothing reallocates between the counts.
                let rc = unsafe {
                    ffi::git_libgit2_opts(
                        ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                        &raw mut allocator,
                    )
                };
                assert_eq!(rc, 0);
                init();
                // The first failure allocates this thread's error state; count after it.
                FAIL_IN.store(1, SeqCst);
                assert!(OdbDataStrFree::dup(c"warm").is_none());

                let strs = [c"one", c"two", c"three"];
                let before = LIVE.load(SeqCst);
                for at in 1..=4 {
                    FAIL_IN.store(at, SeqCst);
                    assert!(
                        OdbDataStrVec::dup_all(&strs).is_none(),
                        "allocation {at} fails"
                    );
                    assert_eq!(LIVE.load(SeqCst), before, "allocation {at} fails");
                }

                let v = OdbDataStrVec::dup_all(&strs).unwrap();
                let held = LIVE.load(SeqCst);
                assert_eq!(held - before, 4);
                for at in 1..=4 {
                    FAIL_IN.store(at, SeqCst);
                    assert!(v.try_clone().is_none());
                    assert_eq!(LIVE.load(SeqCst), held);
                }
                FAIL_IN.store(1, SeqCst);
                assert!(v.as_slice()[1].try_clone().is_none());
                assert_eq!(format!("{v:?}"), r#"["one", "two", "three"]"#);
                drop(v);
                assert_eq!(LIVE.load(SeqCst), before);
            },
        );
    }

    /// `Clone` on a failed copy aborts the process, as documented, for a string and
    /// for a run; nothing is handed back.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn clone_aborts_when_the_copy_fails() {
        use std::os::unix::process::ExitStatusExt;

        const ABORTING: &str = "CRUSTIFY_UNIT_ABORTING";
        if let Ok(which) = std::env::var(ABORTING) {
            let mut allocator = ffi::git_allocator {
                gmalloc: Some(counting_malloc),
                grealloc: Some(libc_realloc),
                gfree: Some(counting_free),
            };
            // SAFETY: a `git_allocator *` with every member set, which libgit2 copies;
            // nothing is allocated yet.
            let rc = unsafe {
                ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(), &raw mut allocator)
            };
            assert_eq!(rc, 0);
            init();
            let s = OdbDataStrFree::dup(c"keep").unwrap();
            let v = OdbDataStrVec::dup_all(&[c"a", c"b"]).unwrap();
            // Warm this thread's error state, which the first failure allocates.
            FAIL_IN.store(1, SeqCst);
            assert!(s.try_clone().is_none());
            FAIL_IN.store(1, SeqCst);
            let _ = match which.as_str() {
                "str" => s.clone().len(),
                "vec" => v.clone().len(),
                _ => unreachable!(),
            };
            unreachable!("clone returned");
        }
        for which in ["str", "vec"] {
            let name = concat!(module_path!(), "::clone_aborts_when_the_copy_fails");
            let name = name.split_once("::").map_or(name, |(_, rest)| rest);
            let out = Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(ABORTING, which)
                .output()
                .unwrap();
            assert_eq!(
                out.status.signal(),
                Some(SIGABRT),
                "`{which}` clone: {:?}\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    const SIGABRT: i32 = 6;

    #[test]
    fn str_vec_empty_forms_own_nothing() {
        let v = OdbDataStrVec::new();
        assert!(v.is_empty() && v.as_ptr().is_null() && v.as_slice().is_empty());
        assert_eq!(format!("{v:?}"), "[]");
        let mut d = OdbDataStrVec::default();
        assert!(d.as_mut_slice().is_empty());
        assert!(d.try_clone().unwrap().as_ptr().is_null());
        assert!(v.clone().into_vec().is_empty());
        let (strings, count) = OdbDataStrVec::dup_all::<&CStr>(&[])
            .unwrap()
            .into_raw_parts();
        assert!(strings.is_null() && count == 0);
        assert!(
            OdbDataStrVec::from_vec(Vec::new())
                .unwrap()
                .as_ptr()
                .is_null()
        );
        // SAFETY: NULL with no count is the empty run and adopts nothing.
        let e = unsafe { OdbDataStrVec::from_raw_parts(ptr::null_mut(), 0) }.unwrap();
        assert!(e.is_empty());
        // SAFETY: NULL with a count is refused without reading anything.
        assert!(unsafe { OdbDataStrVec::from_raw_parts(ptr::null_mut(), 3) }.is_none());
    }

    #[test]
    fn str_owners_are_pointer_sized() {
        assert_eq!(size_of::<OdbDataStr>(), size_of::<*mut c_char>());
        assert_eq!(size_of::<Option<OdbDataStr>>(), size_of::<*mut c_char>());
        assert_eq!(size_of::<OdbDataStrVec>(), size_of::<ffi::git_strarray>());
        let policy = OdbDataStrFree;
        let copy = policy;
        assert_eq!(
            format!("{policy:?}{copy:?}"),
            "OdbDataStrFreeOdbDataStrFree"
        );
    }

    mod own_odb {
        use core::ffi::c_void;
        use core::ptr::NonNull;
        use std::process::Command;
        use std::sync::atomic::AtomicIsize;
        use std::sync::atomic::Ordering::SeqCst;

        use ffibox::CRefSeam;

        use super::super::*;
        use crate::libgit2::src::util::alloc::{Alloc, Allocator};
        use crate::libgit2::src::util::errors::{ErrorClass, set};

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
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn options_carry_this_version_and_the_type() {
            for kind in [OidType::Sha1, OidType::Sha256] {
                let opts = ffi::git_odb_options::from(OdbOptions::with_oid_type(kind));
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
            assert_eq!(unsafe { ffi::git_odb_new(&raw mut raw) }, 0);
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
            let ret = unsafe { ffi::git_repository_wrap_odb(&raw mut repo, raw) };
            assert_eq!(ret, 0);
            drop(odb);
            let mut again = ptr::null_mut();
            // SAFETY: `repo` is live; the result is a new reference to its odb.
            assert_eq!(unsafe { ffi::git_repository_odb(&raw mut again, repo) }, 0);
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
                    ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
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
        /// return with nothing adopted and nothing leaked (`LSan`), until enough succeed.
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

    /// `OdbOptions` and `OdbExpandId`, the header's plain values.
    mod plain_values {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        use super::*;
        use crate::libgit2::src::libgit2::oid::Oid;

        fn hash_of(value: &impl Hash) -> u64 {
            let mut h = DefaultHasher::new();
            value.hash(&mut h);
            h.finish()
        }

        /// An oid of type `kind` whose id bytes count up from `seed`, built in Rust.
        fn oid(kind: u8, seed: u8) -> ffi::git_oid {
            let mut id = [0u8; 32];
            for (i, b) in id.iter_mut().enumerate() {
                *b = seed.wrapping_add(u8::try_from(i).unwrap());
            }
            ffi::git_oid { type_: kind, id }
        }

        fn raw_query(id: ffi::git_oid, length: u16, type_: ffi::git_object_t) -> OdbExpandId {
            OdbExpandId::from(ffi::git_odb_expand_id { id, length, type_ })
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn options_start_from_the_init_template() {
            let opts = OdbOptions::new();
            let raw = ffi::git_odb_options::from(opts);
            assert_eq!(
                (raw.version, raw.oid_type),
                (ffi::GIT_ODB_OPTIONS_VERSION, 0)
            );
            assert_eq!(opts.as_ref().version(), OdbOptions::VERSION);
            assert_eq!(opts.as_ref().oid_type(), None);
            assert_eq!(OdbOptions::default(), opts);
            assert_eq!(
                format!("{opts:?}"),
                "OdbOptions { version: 1, oid_type: None }"
            );
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn options_setters_store_what_the_getters_read() {
            let mut opts = OdbOptions::new();
            for kind in [Some(OidType::Sha256), Some(OidType::Sha1), None] {
                opts.as_mut().set_oid_type(kind);
                assert_eq!(opts.as_ref().oid_type(), kind);
                let raw = ffi::git_odb_options::from(opts);
                assert_eq!(raw.oid_type, kind.map_or(0, OidType::raw));
            }
            for version in [0, 2, u32::MAX, OdbOptions::VERSION] {
                let mut m = opts.as_mut();
                m.set_version(version);
                assert_eq!(m.as_ref().version(), version);
            }
            let typed = OdbOptions::with_oid_type(OidType::Sha256);
            assert_eq!(typed.as_ref().oid_type(), Some(OidType::Sha256));
            assert_eq!(typed.as_ref().version(), OdbOptions::VERSION);
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn options_copy_compare_and_hash_by_field() {
            let mut a = OdbOptions::with_oid_type(OidType::Sha1);
            a.as_mut().set_version(7);
            let copy = a.as_ref().to_options();
            assert_eq!(copy, a);
            assert_eq!(hash_of(&copy), hash_of(&a));
            assert_eq!(format!("{:?}", a.as_mut()), format!("{a:?}"));
            let mut b = a;
            b.as_mut().set_oid_type(Some(OidType::Sha256));
            assert_ne!(a, b);
            b.as_mut().set_oid_type(Some(OidType::Sha1));
            b.as_mut().set_version(8);
            assert_ne!(a, b);
            // `None` and SHA-1 store different bytes, though libgit2 builds the same odb
            // from both.
            let (mut c, d) = (a, a);
            c.as_mut().set_oid_type(None);
            assert_ne!(c, d);
        }

        #[test]
        fn zeroed_query_is_all_zero_and_default() {
            let q = OdbExpandId::zeroed();
            let raw = ffi::git_odb_expand_id::from(q);
            assert_eq!(
                (raw.id.type_, raw.id.id, raw.length, raw.type_),
                (0, [0; 32], 0, 0)
            );
            assert_eq!(OdbExpandId::default(), q);
            let r = q.as_ref();
            assert_eq!((r.length(), r.raw_kind(), r.kind()), (0, 0, None));
            assert_eq!((r.id().raw_kind(), r.id().id()), (0, [0; 32]));
        }

        #[test]
        fn query_fields_read_back_what_was_stored() {
            let q = raw_query(oid(1, 9), 40, ffi::GIT_OBJECT_TREE);
            let r = q.as_ref();
            assert_eq!(r.length(), 40);
            assert_eq!(r.kind(), Some(ObjectType::Tree));
            assert_eq!((r.id().raw_kind(), r.id().id()), (1, oid(1, 9).id));
            for (raw, kind) in [
                (ffi::GIT_OBJECT_ANY, None),
                (ffi::GIT_OBJECT_INVALID, None),
                (0, None),
                (99, None),
                (ffi::GIT_OBJECT_COMMIT, Some(ObjectType::Commit)),
                (ffi::GIT_OBJECT_BLOB, Some(ObjectType::Blob)),
                (ffi::GIT_OBJECT_TAG, Some(ObjectType::Tag)),
            ] {
                let q = raw_query(oid(2, 0), 0, raw);
                assert_eq!((q.as_ref().raw_kind(), q.as_ref().kind()), (raw, kind));
            }
        }

        #[test]
        fn query_setters_write_each_field_alone() {
            let mut q = OdbExpandId::zeroed();
            let src = raw_query(oid(2, 200), 0, 0);
            q.as_mut().set_id(src.as_ref().id());
            let raw = ffi::git_odb_expand_id::from(q);
            assert_eq!(
                (raw.id.type_, raw.id.id, raw.length, raw.type_),
                (2, oid(2, 200).id, 0, 0)
            );
            for length in [0, 3, 4, 39, 40, 64, 65, u16::MAX] {
                q.as_mut().set_length(length);
                assert_eq!(q.as_ref().length(), length);
            }
            q.as_mut().set_kind(Some(ObjectType::Blob));
            assert_eq!(q.as_ref().raw_kind(), ffi::GIT_OBJECT_BLOB);
            q.as_mut().set_kind(None);
            assert_eq!(q.as_ref().raw_kind(), ffi::GIT_OBJECT_ANY);
            assert_eq!(ffi::git_odb_expand_id::from(q).id.id, oid(2, 200).id);
        }

        #[test]
        fn query_id_is_edited_in_place() {
            let mut q = raw_query(oid(1, 0), 40, ffi::GIT_OBJECT_ANY);
            {
                let mut m = q.as_mut();
                let mut id = m.id_mut();
                id.set_raw_kind(2);
                id.set_id([0xAB; 32]);
            }
            let raw = ffi::git_odb_expand_id::from(q);
            assert_eq!((raw.id.type_, raw.id.id), (2, [0xAB; 32]));
            assert_eq!((raw.length, raw.type_), (40, ffi::GIT_OBJECT_ANY));
            // Copying a query's id over itself, through a copy, leaves it whole.
            let before = q;
            let copy = q;
            q.as_mut().set_id(copy.as_ref().id());
            assert_eq!(q, before);
        }

        #[test]
        fn query_copies_compare_and_hash_by_field() {
            let q = raw_query(oid(1, 5), 12, ffi::GIT_OBJECT_COMMIT);
            let copy = q.as_ref().to_expand_id();
            assert_eq!(copy, q);
            assert_eq!(hash_of(&copy), hash_of(&q));
            assert_eq!(hash_of(&copy.as_ref()), hash_of(&q.as_ref()));
            for other in [
                raw_query(oid(2, 5), 12, ffi::GIT_OBJECT_COMMIT),
                raw_query(oid(1, 6), 12, ffi::GIT_OBJECT_COMMIT),
                raw_query(oid(1, 5), 13, ffi::GIT_OBJECT_COMMIT),
                raw_query(oid(1, 5), 12, ffi::GIT_OBJECT_ANY),
            ] {
                assert_ne!(other, q);
            }
            let mut moved = q;
            assert_eq!(format!("{:?}", moved.as_mut()), format!("{q:?}"));
            let text = format!("{q:?}");
            assert!(
                text.starts_with("OdbExpandId { id: Oid { kind: Sha1, id: 0506"),
                "{text}"
            );
            assert!(text.ends_with("length: 12, type: 1 }"), "{text}");
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn new_query_fills_every_field() {
            let id = Oid::from_prefix(b"4a202b34", OidType::Sha1).unwrap();
            let q = OdbExpandId::new(id.as_ref(), 8, Some(ObjectType::Blob));
            let r = q.as_ref();
            assert_eq!(r.id(), id.as_ref());
            assert_eq!((r.length(), r.kind()), (8, Some(ObjectType::Blob)));
            let any = OdbExpandId::new(id.as_ref(), 40, None);
            assert_eq!(any.as_ref().raw_kind(), ffi::GIT_OBJECT_ANY);
        }

        #[test]
        fn new_query_from_a_zero_id_needs_no_c() {
            let zero = OdbExpandId::zeroed();
            let q = OdbExpandId::new(zero.as_ref().id(), 4, Some(ObjectType::Tag));
            let raw = ffi::git_odb_expand_id::from(q);
            assert_eq!(
                (raw.id.type_, raw.length, raw.type_),
                (0, 4, ffi::GIT_OBJECT_TAG)
            );
        }
    }

    /// The stream and writepack layouts, built in Rust storage with members of their own,
    /// adopted where a test needs an owner: the getters, the members' calls and the
    /// teardown, without a backend.
    mod stream {
        use core::cell::RefCell;

        use super::*;
        use crate::libgit2::src::libgit2::indexer::IndexerProgress;

        /// A stream with every field set, its members `read`/`write`/`finalize_write`
        /// present when `members` is set.
        fn raw_stream(members: bool) -> ffi::git_odb_stream {
            unsafe extern "C" fn read(
                _: *mut ffi::git_odb_stream,
                _: *mut c_char,
                _: usize,
            ) -> c_int {
                0
            }
            unsafe extern "C" fn write(
                _: *mut ffi::git_odb_stream,
                _: *const c_char,
                _: usize,
            ) -> c_int {
                0
            }
            unsafe extern "C" fn finalize(
                _: *mut ffi::git_odb_stream,
                _: *const ffi::git_oid,
            ) -> c_int {
                0
            }
            ffi::git_odb_stream {
                backend: ptr::without_provenance_mut(0x1000),
                mode: ffi::GIT_STREAM_RW,
                hash_ctx: if members {
                    ptr::without_provenance_mut(0x2000)
                } else {
                    ptr::null_mut()
                },
                oid_type: if members { ffi::GIT_OID_SHA256 } else { 77 },
                declared_size: u64::MAX,
                received_bytes: 12,
                read: members.then_some::<unsafe extern "C" fn(_, _, _) -> _>(read),
                write: members.then_some::<unsafe extern "C" fn(_, _, _) -> _>(write),
                finalize_write: members.then_some::<unsafe extern "C" fn(_, _) -> _>(finalize),
                free: None,
            }
        }

        #[test]
        fn stream_getters_read_every_field() {
            for members in [true, false] {
                let mut raw = raw_stream(members);
                // SAFETY: `raw` is a live stream in Rust storage, only read through the
                // handle while it lives.
                let s = unsafe { OdbStreamRef::from_ptr(&raw mut raw) }.unwrap();
                assert_eq!(s.backend().addr(), 0x1000);
                assert_eq!(s.mode(), ffi::GIT_STREAM_RW);
                assert_eq!(s.has_hash_ctx(), members);
                if members {
                    assert_eq!(s.oid_type(), Ok(OidType::Sha256));
                } else {
                    assert_eq!(s.oid_type().unwrap_err().raw(), 77);
                }
                assert_eq!((s.declared_size(), s.received_bytes()), (u64::MAX, 12));
                assert_eq!(
                    (s.can_read(), s.can_write(), s.can_finalize_write()),
                    (members, members, members)
                );
                let debug = format!("{s:?}");
                assert!(debug.contains("received_bytes: 12"), "{debug}");
            }
        }

        #[test]
        fn stream_exclusive_handle_reborrows_to_the_getters() {
            let mut raw = raw_stream(true);
            // SAFETY: `raw` is a live stream in Rust storage, reached only through the
            // handle while it lives.
            let mut m = unsafe { OdbStreamMut::from_ptr(&raw mut raw) }.unwrap();
            assert_eq!(m.as_mut().as_ref().received_bytes(), 12);
            assert!(m.as_ref().can_read());
            assert_eq!(format!("{m:?}"), format!("{:?}", m.as_ref()));
        }

        /// A stream in a container that records its `free` call, as a backend's does.
        #[repr(C)]
        struct Counted<'l> {
            stream: ffi::git_odb_stream,
            log: &'l RefCell<Vec<&'static str>>,
        }

        unsafe extern "C" fn counted_free(stream: *mut ffi::git_odb_stream) {
            // SAFETY: the stream heads a boxed `Counted`, given up by its owner.
            let counted = unsafe { Box::from_raw(stream.cast::<Counted<'_>>()) };
            counted.log.borrow_mut().push("free");
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn stream_owner_frees_through_git_odb_stream_free_once() {
            assert!(runtime_pinned());
            let log = RefCell::new(Vec::new());
            let mut stream = raw_stream(true);
            stream.hash_ctx = ptr::null_mut();
            stream.free = Some(counted_free);
            let counted = Box::into_raw(Box::new(Counted { stream, log: &log }));
            // SAFETY: a stream whose `free` member releases its container, which the owner
            // takes, with no hashing context; `log` outlives it.
            let owner = unsafe { OdbStreamBox::from_c(counted.cast()) }.unwrap();
            assert!(owner.as_ref().can_write());
            assert!(log.borrow().is_empty());
            drop(owner);
            assert_eq!(*log.borrow(), ["free"]);
        }

        /// A writepack in a container that records its members' calls.
        #[repr(C)]
        struct Pack<'l> {
            writepack: ffi::git_odb_writepack,
            log: &'l RefCell<Vec<String>>,
        }

        /// The log of the `Pack` a writepack heads.
        ///
        /// # Safety
        ///
        /// `wp` heads a live `Pack`.
        unsafe fn log_of<'l>(wp: *mut ffi::git_odb_writepack) -> &'l RefCell<Vec<String>> {
            // SAFETY: per this function's contract.
            unsafe { (*wp.cast::<Pack<'l>>()).log }
        }

        unsafe extern "C" fn pack_append(
            wp: *mut ffi::git_odb_writepack,
            data: *const c_void,
            size: usize,
            stats: *mut ffi::git_indexer_progress,
        ) -> c_int {
            // SAFETY: the writepack heads a live `Pack`; `stats` is writable.
            unsafe {
                log_of(wp)
                    .borrow_mut()
                    .push(format!("append {size} {}", data.is_null()));
            }
            // SAFETY: `stats` is a live, writable progress.
            unsafe { (*stats).received_bytes += size };
            0
        }

        unsafe extern "C" fn pack_commit(
            wp: *mut ffi::git_odb_writepack,
            stats: *mut ffi::git_indexer_progress,
        ) -> c_int {
            // SAFETY: as in `pack_append`.
            unsafe { log_of(wp).borrow_mut().push("commit".to_owned()) };
            // SAFETY: as in `pack_append`.
            unsafe { (*stats).indexed_objects = 9 };
            1
        }

        unsafe extern "C" fn pack_free(wp: *mut ffi::git_odb_writepack) {
            // SAFETY: the writepack heads a boxed `Pack`, given up by its owner.
            let pack = unsafe { Box::from_raw(wp.cast::<Pack<'_>>()) };
            pack.log.borrow_mut().push("free".to_owned());
        }

        /// A writepack owner over a fresh `Pack` logging into `log`, with the members
        /// `members` sets.
        fn pack(log: &RefCell<Vec<String>>, members: bool) -> OdbWritepackBox<'_> {
            let writepack = ffi::git_odb_writepack {
                backend: ptr::null_mut(),
                append: members.then_some::<unsafe extern "C" fn(_, _, _, _) -> _>(pack_append),
                commit: members.then_some::<unsafe extern "C" fn(_, _) -> _>(pack_commit),
                free: Some(pack_free),
            };
            let raw = Box::into_raw(Box::new(Pack { writepack, log }));
            // SAFETY: a writepack whose `free` member releases its container, which the
            // owner takes; `log` outlives it, as the owner's lifetime says.
            unsafe { OdbWritepackBox::from_c(raw.cast()) }.unwrap()
        }

        #[test]
        fn writepack_members_run_in_order_and_free_once() {
            let log = RefCell::new(Vec::new());
            let mut wp = pack(&log, true);
            assert!(wp.as_ref().backend().is_null());
            assert!(format!("{:?}", wp.as_mut()).starts_with("OdbWritepackRef"));
            let mut stats = IndexerProgress::default();
            wp.as_mut().append(b"abc", stats.as_mut()).unwrap();
            // An empty piece still passes a pointer, never NULL.
            wp.as_mut().as_mut().append(&[], stats.as_mut()).unwrap();
            assert_eq!(stats.as_ref().received_bytes(), 3);
            // A positive return is success, as libgit2's callers read it.
            OdbWritepack::commit(wp, stats.as_mut()).unwrap();
            assert_eq!(stats.as_ref().indexed_objects(), 9);
            assert_eq!(
                *log.borrow(),
                ["append 3 false", "append 0 false", "commit", "free"]
            );

            // Dropped unfinished: freed, never committed.
            let log = RefCell::new(Vec::new());
            drop(pack(&log, true));
            assert_eq!(*log.borrow(), ["free"]);
        }

        #[test]
        fn writepack_without_members_fails_without_calling() {
            let log = RefCell::new(Vec::new());
            let mut wp = pack(&log, false);
            let mut stats = IndexerProgress::default();
            let err = wp.as_mut().append(b"abc", stats.as_mut()).unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Odb));
            assert_eq!(err.message(), c"the writepack has no append member");
            assert_eq!(err.code(), Some(ErrorCode::Generic));
            let err = OdbWritepack::commit(wp, stats.as_mut()).unwrap_err();
            assert_eq!(err.message(), c"the writepack has no commit member");
            assert_eq!(*log.borrow(), ["free"]);
            assert_eq!(stats.as_ref().received_bytes(), 0);
        }

        #[test]
        fn writepack_without_free_is_left_alone() {
            let mut raw = ffi::git_odb_writepack {
                backend: ptr::null_mut(),
                append: None,
                commit: None,
                free: None,
            };
            // SAFETY: a writepack in Rust storage, outliving the owner, whose drop runs no
            // member since none is set.
            let owner = unsafe { OdbWritepackBox::from_c(&raw mut raw) }.unwrap();
            drop(owner);
            assert!(raw.free.is_none());
        }

        #[test]
        fn passed_reports_passthrough() {
            let err = passed();
            assert_eq!(err.code(), Some(ErrorCode::Passthrough));
            assert_eq!(err.class(), Some(ErrorClass::Odb));
        }

        unsafe extern "C" fn failing_append(
            _: *mut ffi::git_odb_writepack,
            _: *const c_void,
            _: usize,
            _: *mut ffi::git_indexer_progress,
        ) -> c_int {
            set(ErrorClass::Indexer, c"bad pack");
            -7
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_failing_member_returns_its_code_and_error() {
            let log = RefCell::new(Vec::new());
            let mut wp = pack(&log, true);
            // SAFETY: the owner's writepack, live; its `append` member is replaced by one
            // of the same signature that touches nothing.
            unsafe { (*wp.as_c_ptr()).append = Some(failing_append) };
            let mut stats = IndexerProgress::default();
            let err = wp.as_mut().append(b"x", stats.as_mut()).unwrap_err();
            assert_eq!(err.raw_code(), -7);
            assert_eq!(err.class(), Some(ErrorClass::Indexer));
            assert_eq!(err.message(), c"bad pack");
        }
    }

    /// The readers' and writers' internals: the id-type probe and the refusal of other
    /// ids it backs, the error type, the lookup flags, the object policy, and the foreach
    /// trampoline under a real `git_odb_foreach` walk.
    mod odb_ops {
        use std::ffi::CString;

        use super::super::*;

        /// The libgit2 calls below need the runtime the constructors pin.
        fn init() {
            assert!(runtime_pinned());
        }

        /// A temporary `objects` directory, removed on drop.
        struct Objects(std::path::PathBuf, CString);

        impl Objects {
            fn new(tag: &str) -> Self {
                let path = std::env::temp_dir()
                    .join(format!("crustify-odb-unit-{}-{tag}", std::process::id()));
                drop(std::fs::remove_dir_all(&path));
                std::fs::create_dir_all(&path).unwrap();
                let c = CString::new(path.to_str().unwrap()).unwrap();
                Self(path, c)
            }
        }

        impl Drop for Objects {
            fn drop(&mut self) {
                drop(std::fs::remove_dir_all(&self.0));
            }
        }

        const TYPES: [OidType; 2] = [OidType::Sha1, OidType::Sha256];

        #[test]
        fn lookup_flags_are_bits() {
            assert_eq!(LookupFlags::default(), LookupFlags::NONE);
            assert_eq!(LookupFlags::NONE.bits(), 0);
            assert_eq!(
                LookupFlags::NO_REFRESH.bits(),
                ffi::GIT_ODB_LOOKUP_NO_REFRESH
            );
            let mut flags = LookupFlags::from_bits_retain(0x80);
            assert!(!flags.contains(LookupFlags::NO_REFRESH));
            flags |= LookupFlags::NO_REFRESH;
            assert!(flags.contains(LookupFlags::NO_REFRESH));
            assert!(flags.contains(LookupFlags::NONE));
            assert_eq!(flags.bits(), 0x81);
            assert_eq!(
                LookupFlags::NONE | LookupFlags::NO_REFRESH,
                LookupFlags::NO_REFRESH
            );
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn errors_carry_the_code_and_the_thread_error() {
            init();
            set(ErrorClass::Odb, c"first");
            let error = OdbError::from_libgit2(ErrorCode::NotFound.raw());
            assert_eq!(error.code(), Some(ErrorCode::NotFound));
            assert_eq!(error.raw_code(), ffi::GIT_ENOTFOUND);
            assert_eq!(error.class(), Some(ErrorClass::Odb));
            assert_eq!(error.raw_class(), ErrorClass::Odb.raw());
            assert_eq!(error.message(), c"first");
            assert_eq!(error.to_string(), error.error().to_string());
            assert!(format!("{error:?}").contains("NotFound"));
            // A positive return, which no routine documents, is a generic failure; a code
            // the header does not name is kept raw.
            assert_eq!(OdbError::from_libgit2(1).raw_code(), ffi::GIT_ERROR);
            let unnamed = OdbError::from_libgit2(-12_345);
            assert_eq!((unnamed.code(), unnamed.raw_code()), (None, -12_345));
            assert!(format!("{unnamed:?}").contains("-12345"));
            let buf: ErrorBuf = unnamed.clone().into();
            assert_eq!(&buf, unnamed.error());
            assert_eq!(status(0), Ok(()));
            assert_eq!(status(-3).unwrap_err().raw_code(), -3);
        }

        /// The probe answers every constructor's type, reaching no backend and taking no
        /// lock, and leaves no error behind.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn the_probe_answers_the_odbs_type() {
            init();
            let dir = Objects::new("probe");
            assert_eq!(Odb::new().unwrap().as_ref().id_type(), Some(OidType::Sha1));
            assert_eq!(
                Odb::open(&dir.1).unwrap().as_ref().id_type(),
                Some(OidType::Sha1)
            );
            for kind in TYPES {
                let odb = Odb::with_oid_type(kind).unwrap();
                assert_eq!(odb.as_ref().id_type(), Some(kind));
                let odb = Odb::open_with_oid_type(&dir.1, kind).unwrap();
                set(ErrorClass::Odb, c"stale");
                assert_eq!(odb.as_ref().id_type(), Some(kind));
                // `git_odb_expand_ids` clears the thread's error.
                assert_eq!(last().raw_class(), 0, "no error is left");
            }
        }

        /// The C defect the readers correct: an id of another type, of none, or of an
        /// unnamed one is refused before libgit2 formats it, with a set error.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn ids_of_another_type_are_refused_before_the_call() {
            init();
            for kind in TYPES {
                let odb = Odb::with_oid_type(kind).unwrap();
                let handle = odb.as_ref();
                let mut own = Oid::zero(kind);
                own.as_mut().set_id([7; 32]);
                assert!(handle.same_type(own.as_ref()));
                assert!(handle.fits(own.as_ref()).is_ok());
                for raw in [0, 1, 2, 3, 0xFF] {
                    let mut id = own;
                    id.as_mut().set_raw_kind(raw);
                    if id.as_ref().kind() == Some(kind) {
                        continue;
                    }
                    assert!(!handle.same_type(id.as_ref()), "{raw}");
                    assert!(!handle.exists(id.as_ref()));
                    let error = handle.fits(id.as_ref()).unwrap_err();
                    assert_eq!(error.code(), Some(ErrorCode::Generic));
                    assert_eq!(error.class(), Some(ErrorClass::Invalid));
                    assert_eq!(error.message(), MISMATCH);
                    assert_eq!(handle.read(id.as_ref()).unwrap_err(), error);
                    assert_eq!(handle.read_prefix(id.as_ref(), 64).unwrap_err(), error);
                    assert_eq!(handle.exists_prefix(id.as_ref(), 64).unwrap_err(), error);
                    assert_eq!(handle.read_header(id.as_ref()).unwrap_err(), error);
                }
            }
        }

        /// The object policy takes and gives back references on the same object.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn the_object_policy_counts_one_object() {
            init();
            let dir = Objects::new("policy");
            let odb = Odb::open(&dir.1).unwrap();
            let id = odb.as_ref().write(b"counted", ObjectType::Blob).unwrap();
            let object = odb.as_ref().read(id.as_ref()).unwrap();
            let ptr = NonNull::new(object.as_c_ptr().cast::<OdbObject>()).unwrap();
            // SAFETY: `ptr` is the live object `object` holds a reference to.
            assert!(unsafe { OdbObjectFree.c_up_ref(ptr) });
            // SAFETY: gives back the reference just taken.
            unsafe { OdbObjectFree.c_drop(ptr) };
            assert_eq!(object.as_ref().data(), b"counted");
            assert!(format!("{:?}", object.as_ref()).contains("Blob"));
            assert_eq!(read_out(-1, ptr::null_mut()).unwrap_err().raw_code(), -1);
        }

        /// The trampoline under `git_odb_foreach`: every id of every backend, a break
        /// returned as the walk's result, and a panic resumed once the walk is back.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn the_trampoline_walks_every_object() {
            init();
            let dir = Objects::new("foreach");
            let odb = Odb::open(&dir.1).unwrap();
            let mut written: Vec<Oid> = (0..5)
                .map(|i| {
                    let data = format!("object {i}");
                    odb.as_ref()
                        .write(data.as_bytes(), ObjectType::Blob)
                        .unwrap()
                })
                .collect();
            written.sort();
            let walk = |callback: &mut ForeachCallback<'_>| {
                callback.walk_with(|cb, payload| {
                    // SAFETY: a live odb; `cb` and `payload` are the trampoline and its
                    // callback, live for the walk.
                    unsafe { ffi::git_odb_foreach(odb.as_c_ptr(), cb, payload) }
                })
            };
            let mut seen = Vec::new();
            let mut collect = |id: OidRef<'_>| {
                seen.push(id.to_oid());
                ControlFlow::Continue(())
            };
            assert_eq!(walk(&mut ForeachCallback::new(&mut collect)), 0);
            seen.sort();
            assert_eq!(seen, written);
            let mut calls = 0;
            let mut stop = |_: OidRef<'_>| {
                calls += 1;
                ControlFlow::Break(NonZero::new(-77).unwrap())
            };
            assert_eq!(walk(&mut ForeachCallback::new(&mut stop)), -77);
            assert_eq!(calls, 1);
            let mut boom = |_: OidRef<'_>| -> ControlFlow<NonZero<c_int>> { panic!("boom") };
            let mut callback = ForeachCallback::new(&mut boom);
            let caught = panic::catch_unwind(AssertUnwindSafe(|| walk(&mut callback)));
            assert_eq!(caught.unwrap_err().downcast_ref::<&str>(), Some(&"boom"));
            assert!(format!("{callback:?}").contains("panicked: false"));
        }

        /// The trampoline refuses a NULL id and a callback that already panicked.
        #[test]
        fn the_trampoline_stops_on_null_and_after_a_panic() {
            let mut calls = 0;
            let mut count = |_: OidRef<'_>| {
                calls += 1;
                ControlFlow::Continue(())
            };
            let mut callback = ForeachCallback::new(&mut count);
            let payload = ptr::from_mut(&mut callback).cast::<c_void>();
            // SAFETY: `payload` is the live callback, used by nothing else meanwhile; the
            // id is NULL, which the trampoline accepts.
            let rc = unsafe { foreach_with(ptr::null(), payload) };
            assert_eq!(rc, ErrorCode::User.raw());
            callback.panic = Some(Box::new("earlier"));
            let payload = ptr::from_mut(&mut callback).cast::<c_void>();
            let id = Oid::zero(OidType::Sha1);
            // SAFETY: as above, with a live id; the payload is taken again after the
            // write above.
            let rc = unsafe { foreach_with(id.as_ref().as_ptr(), payload) };
            assert_eq!(rc, ErrorCode::User.raw());
            drop(callback);
            assert_eq!(calls, 0);
        }
    }
}
