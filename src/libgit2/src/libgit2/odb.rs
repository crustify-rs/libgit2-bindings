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
//! The constructors pin libgit2's runtime first (`runtime_pinned`), since they
//! allocate, and return the thread's libgit2 error, copied out, as an [`ErrorBuf`].

use core::ffi::{CStr, c_char, c_int};
use core::fmt;
use core::marker::PhantomData;
use core::mem::{ManuallyDrop, MaybeUninit, align_of, size_of};
use core::ptr::{self, NonNull};

use ffibox::{
    CAlloc, CAllocZeroed, CArc, CBorrowedPtr, CBox, CCell, CDrop, CNew, CPlainElem, CStrBox, CVec,
    CZeroable, impl_cdrop_str, impl_cdupclone_str, impl_clenclone, impl_clendrop,
};

use crate::ffi;
use crate::libgit2::src::libgit2::oid::OidType;
use crate::libgit2::src::util::errors::{ErrorBuf, last, runtime_pinned};

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
/// Wraps: `git_odb`
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
    type Mut<'a>
        = OdbMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = OdbRef<'a, 'r>
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

impl<'r> OdbRef<'_, 'r> {
    /// Borrows the odb at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live odb that stays live for `'a`, owned by no
    /// repository or by one that outlives `'r`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_odb) -> Option<Self> {
        // SAFETY: this function's contract is `CCell::ref_from_ptr`'s: `ptr` is NULL or a
        // live `git_odb` that outlives `'a` and is not written meanwhile.
        unsafe { <Odb<'r> as CCell>::ref_from_ptr(ptr) }
    }

    /// Pointer to the C object, for the routines that take a `git_odb *` and lock it
    /// themselves.
    pub(crate) fn as_ptr(&self) -> *mut ffi::git_odb {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'r> OdbMut<'_, 'r> {
    /// Writable pointer to the C object.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_odb {
        self.0.0.as_non_null().as_ptr().cast()
    }

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
///
/// Wraps: `git_odb_free`
/// Wraps: `git_odb_new`
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
    /// Wraps: `git_odb_new`
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
    /// Wraps: `git_odb_new_ext`
    ///
    /// # Errors
    ///
    /// Fails as [`new`](Self::new) does.
    pub fn with_oid_type(oid_type: OidType) -> Result<OdbArc<'static>, ErrorBuf> {
        let opts = options(oid_type);
        construct(|out| {
            // SAFETY: `out` is a writable slot, filled with a fresh reference on
            // success; `opts` is a live options struct of this version, which the
            // routine copies and does not retain. The runtime is pinned.
            unsafe { ffi::git_odb_new_ext(out, &raw const opts) }
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
    /// Wraps: `git_odb_open`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, when `objects_dir` cannot be examined (`GIT_ERROR_ODB`, `failed
    /// to load object database in '<dir>'`), when an `info/alternates` file cannot be read, or when
    /// memory runs out.
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
    /// Wraps: `git_odb_open_ext`
    ///
    /// # Errors
    ///
    /// Fails as [`open`](Self::open) does.
    pub fn open_with_oid_type(
        objects_dir: &CStr,
        oid_type: OidType,
    ) -> Result<OdbArc<'static>, ErrorBuf> {
        let opts = options(oid_type);
        construct(|out| {
            // SAFETY: as in `open`; `opts` is a live options struct of this version,
            // which the routine copies and does not retain.
            unsafe { ffi::git_odb_open_ext(out, objects_dir.as_ptr(), &raw const opts) }
        })
    }
}

// The layout type is the C type's size, so an odb pointer converts both ways, and the
// owner is pointer-sized.
const _: () = assert!(size_of::<Odb<'static>>() == size_of::<ffi::git_odb>());

const _: () = assert!(size_of::<OdbArc<'static>>() == size_of::<*mut ffi::git_odb>());

const _: () = assert!(size_of::<Option<OdbArc<'static>>>() == size_of::<*mut ffi::git_odb>());

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
///
/// Wraps: `git_odb_backend_data_alloc`
/// Wraps: `git_odb_backend_data_free`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbDataFree;

/// One `T` in libgit2's allocator.
pub type OdbDataBox<T> = CBox<T, OdbDataFree>;

// SAFETY: `git_odb_backend_data_free` releases the allocation exactly once, without
// reading it, whether or not it holds a `T`; `self` carries no state.
unsafe impl<T> CDrop<T> for OdbDataFree {
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
///
/// Wraps: `git_odb_backend_data_alloc`
/// Wraps: `git_odb_backend_data_free`
/// Handwritten
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

impl_clendrop!(OdbDataArrayFree, data_free_len);

impl_clenclone!(OdbDataArrayFree, data_dup_len, align = DATA_ALIGN);

impl OdbDataArrayFree {
    /// `len` uninitialised elements; `None` when the allocation fails or its size
    /// overflows. An empty request allocates nothing.
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
///
/// Wraps: `git_odb_backend_data_alloc`
/// Wraps: `git_odb_backend_data_free`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbDataStrFree;

/// One NUL-terminated string in libgit2's allocator.
pub type OdbDataStr = CStrBox<OdbDataStrFree>;

impl_cdrop_str!(OdbDataStrFree, str_free);

impl_cdupclone_str!(OdbDataStrFree, str_dup_raw);

impl OdbDataStrFree {
    /// A copy of `s` in libgit2's allocator; `None` when the allocation fails.
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
///
/// Wraps: `git_odb_backend_data_alloc`
/// Wraps: `git_odb_backend_data_free`
/// Handwritten
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
}
