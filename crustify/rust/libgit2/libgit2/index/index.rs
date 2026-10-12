//! Safe wrappers over `include/git2/index.h`.
//!
//! An [`Index`] is a `git_index`: the staging area, a sorted list of entries plus the
//! conflict (`REUC`, `NAME`) and tree-cache extensions, optionally backed by an index
//! file on disk. The public headers declare it opaque, so Rust reaches it only behind a
//! pointer and only through libgit2's routines; it has no field accessors.
//!
//! # Ownership
//!
//! libgit2 counts references to an index (`GIT_REFCOUNT_*` over its private `rc`
//! field) and frees it when the last one is released by `git_index_free`, the down-ref.
//! It publishes no up_ref, no copy and no way to read the count, so Rust tells the two
//! kinds of reference apart by where they came from:
//!
//! | reference | from | owner | handles |
//! |---|---|---|---|
//! | the only one | [`Index::new`], [`Index::open`] and their `_with_oid_type` forms; later the merge, cherry-pick, revert and apply routines that build a fresh index | [`IndexBox`] | [`IndexRef`], [`IndexMut`] |
//! | one of several | an [`IndexBox`] converted with `.into()`; later `git_repository_index` and `git_rebase_inmemory_index`, which count one more reference to an index the repository or rebase keeps | [`IndexArc`] | [`IndexRef`] |
//!
//! Both owners release their reference through [`IndexFree`]. An [`IndexArc`] has no
//! `Clone` (there is no up_ref to count a copy with) and no exclusive handle: nothing
//! proves its reference is the only one, and another holder may change the index
//! behind it, invalidating whatever an [`IndexRef`] borrowed from it.
//!
//! An [`IndexBox`] stays the only reference only while every routine handed its pointer
//! returns without keeping one. Index writers and `git_checkout_index` can count a
//! reference for the duration of a call or a borrow tied to the handle;
//! `git_repository_set_index` keeps one for the repository. [`IndexIterator::new`]
//! retains one independently of the caller's borrow. Such an independently retaining
//! wrapper must take an [`IndexArc`] (into which the box converts), never an
//! [`IndexMut`] or [`IndexRef`] of a box.
//! [`ConflictIterator::new`] instead borrows without retaining: its lifetime
//! keeps the index alive and unmodified.
//!
//! `git_index_owner` lends the repository that keeps a reference to the index, if any,
//! without counting it: libgit2 clears it when that repository releases the index.
//!
//! # Threads
//!
//! libgit2 synchronises only the reference count: two threads must not use one index
//! at once, and a shared index's other holders are not Rust values whose access this
//! crate controls. So neither the index nor its owners and handles are `Send` or
//! `Sync`:
//!
//! ```compile_fail,E0277
//! use libgit2::libgit2::index::index::Index;
//!
//! let index = Index::new().unwrap();
//! std::thread::spawn(move || drop(index));
//! ```

use core::ffi::{CStr, c_int};
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::num::TryFromIntError;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};

use ffibox::{CArc, CBorrowedPtr, CBox, CCell, CNew, CZeroable, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::core::strarray::StrArrayRef;
use crate::libgit2::core::types::FileMode;
use crate::libgit2::index::pathspec::patterns_are_parsable;
use crate::libgit2::object::object::id_fits;
use crate::libgit2::oid::oid::{Oid, OidMut, OidRef, OidType};
use crate::libgit2::raw::OdbDataVec;
use crate::libgit2::repository::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use crate::libgit2::util::sys::errors::{runtime_pinned, set};

define_ctype!(
    /// A git index (`git_index`): the staging area, its entries and extensions,
    /// optionally backed by an index file.
    ///
    /// Opaque, and only ever behind a pointer (see the [module docs](self#ownership)):
    /// owned as an [`IndexBox`], which [`new`](Self::new), [`open`](Self::open) and
    /// their `_with_oid_type` forms build, or as an [`IndexArc`] when the reference is
    /// shared; borrowed as an [`IndexRef`] / [`IndexMut`] from either.
    ///
    /// There is no value to hold: the layout is libgit2's own.
    ///
    /// ```compile_fail,E0423
    /// use libgit2::libgit2::index::index::Index;
    ///
    /// let _ = Index(Default::default(), core::marker::PhantomData);
    /// ```
    ///
    /// Wraps: git_index
    Index, IndexRef, IndexMut, ffi::git_index
);

/// Releases one counted reference to an index through `git_index_free`, which frees the
/// index with its entries, extensions and path once no reference is left and no
/// repository keeps it; builds a fresh in-memory index through `git_index_new`.
///
/// `git_index_free` is the only releaser for every reference libgit2 hands out, so the
/// policy settles both [`IndexBox`] and [`IndexArc`]. It counts no copies: libgit2
/// publishes no up_ref.
///
/// Wraps: git_index_free
/// Wraps: git_index_new
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct IndexFree;

impl_cdrop!(IndexFree, Index, ffi::git_index_free);

// SAFETY: `Index::new` returns a fresh index from `git_index_new`, fully initialised,
// holding its only reference (count 1, no owning repository), which `git_index_free`
// releases; it fails only when libgit2 does (out of memory, or it cannot be
// initialised).
unsafe impl CNew<Index> for IndexFree {
    fn c_new(&self) -> Option<NonNull<Index>> {
        Index::new()
            .ok()
            .and_then(|index| NonNull::new(index.into_raw()))
    }
}

/// The only reference to an [`Index`], released by `git_index_free` on drop; the owner
/// with an [`IndexMut`]. Converts into an [`IndexArc`] with `.into()`.
///
/// Its handles borrow it, so neither outlives it:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::index::index::Index;
///
/// let index = Index::new().unwrap();
/// let handle = index.as_ref();
/// drop(index);
/// let _ = handle;
/// ```
///
/// and its exclusive handle is exclusive:
///
/// ```compile_fail,E0499
/// use libgit2::libgit2::index::index::Index;
///
/// let mut index = Index::new().unwrap();
/// let first = index.as_mut();
/// let second = index.as_mut();
/// let _ = (first, second);
/// ```
///
/// It has no `Clone`: libgit2 publishes no copy routine.
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::index::index::Index;
///
/// let _ = Index::new().unwrap().clone();
/// ```
pub type IndexBox = CBox<Index, IndexFree>;

/// One of several counted references to an [`Index`], released by `git_index_free` on
/// drop. Shared, so it hands out only [`IndexRef`]s, and has no `Clone`: libgit2
/// publishes no up_ref to count a copy with.
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::index::index::{Index, IndexArc};
///
/// let mut index: IndexArc = Index::new().unwrap().into();
/// let _ = index.get_mut();
/// ```
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::index::index::{Index, IndexArc};
///
/// let index: IndexArc = Index::new().unwrap().into();
/// let _ = index.clone();
/// ```
///
/// Nor does it convert back into an [`IndexBox`]: nothing proves its reference is the
/// only one.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::index::index::{Index, IndexArc, IndexBox};
///
/// let index: IndexArc = Index::new().unwrap().into();
/// let _ = IndexBox::try_from(index);
/// ```
pub type IndexArc = CArc<Index, IndexFree>;

/// The index a constructor returning `rc` handed out, or libgit2's error.
fn constructed(rc: c_int, index: Option<IndexBox>) -> Result<IndexBox, ErrorBuf> {
    index.filter(|_| rc >= 0).ok_or_else(last)
}

define_ctype!(
    /// The options for creating or opening an index (`git_index_options`): the struct's
    /// version, and the type of the object ids the index holds.
    ///
    /// A plain value of two integers, resource-free: `Copy`, held inline, never
    /// released, and read by libgit2 only for the call it is passed to. Build one with
    /// [`new`](Self::new) (or `Default`), libgit2's own defaults, or
    /// [`with_oid_type`](Self::with_oid_type); read and change it through
    /// [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut):
    ///
    /// ```
    /// use libgit2::libgit2::index::index::IndexOptions;
    /// use libgit2::libgit2::oid::oid::OidType;
    ///
    /// # if cfg!(miri) { return; }
    /// let mut options = IndexOptions::new();
    /// assert_eq!(options.as_ref().oid_type(), None); // SHA-1, libgit2's default
    /// options.as_mut().set_oid_type(Some(OidType::Sha256));
    /// assert_eq!(options, IndexOptions::with_oid_type(OidType::Sha256));
    /// ```
    ///
    /// The version is always this binding's `GIT_INDEX_OPTIONS_VERSION`: it tells
    /// libgit2 the struct's layout, which is this type's, so there is no setter. The
    /// object id type is a named [`OidType`] or unset; nothing stores another value, so
    /// libgit2's assertion that a set type is valid never fires.
    ///
    /// Its handles borrow it:
    ///
    /// ```compile_fail,E0597
    /// use libgit2::libgit2::index::index::IndexOptions;
    ///
    /// let handle;
    /// {
    ///     let options = IndexOptions::new();
    ///     handle = options.as_ref();
    /// }
    /// let _ = handle.version();
    /// ```
    ///
    /// The shared handle only reads:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::index::index::IndexOptions;
    ///
    /// let options = IndexOptions::new();
    /// options.as_ref().set_oid_type(None);
    /// ```
    ///
    /// Wraps: git_index_options
    #[derive(Clone, Copy)]
    IndexOptions, IndexOptionsRef, IndexOptionsMut, ffi::git_index_options
);

// SAFETY: `IndexOptions` is two integers: no pointer, no interior mutability, no thread
// affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for IndexOptions {}
// SAFETY: `IndexOptionsMut` is an exclusive borrow of such a value, as
// `&mut IndexOptions`.
unsafe impl Send for IndexOptionsMut<'_> {}

// SAFETY: shared access only reads the integers (every write goes through
// `IndexOptionsMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for IndexOptions {}
// SAFETY: as above; `IndexOptionsRef` is a shared borrow, as `&IndexOptions`.
unsafe impl Send for IndexOptionsRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for IndexOptionsRef<'_> {}
// SAFETY: `&IndexOptionsMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for IndexOptionsMut<'_> {}

impl IndexOptions {
    /// libgit2's default options, as `GIT_INDEX_OPTIONS_INIT` spells them: this version,
    /// and no object id type, which the index constructors read as SHA-1.
    ///
    /// `git_index_options_init` only fails for a version other than
    /// `GIT_INDEX_OPTIONS_VERSION`, and with that version it reaches no global, no
    /// allocator and no thread state, so this needs no initialised library.
    ///
    /// Wraps: git_index_options_init
    pub fn new() -> Self {
        let mut raw = MaybeUninit::<ffi::git_index_options>::uninit();
        // SAFETY: `raw` is writable storage for one `git_index_options`, which
        // `git_index_options_init` overwrites whole with the template for this version;
        // nothing else reads it during the call, and C keeps no pointer to it.
        let rc = unsafe {
            ffi::git_index_options_init(raw.as_mut_ptr(), ffi::GIT_INDEX_OPTIONS_VERSION)
        };
        assert_eq!(rc, 0, "git_index_options_init refused its own version");
        // SAFETY: a 0 return means the template was copied in, so both integers are
        // initialised: the version, and an `oid_type` of 0.
        Self(unsafe { raw.assume_init() }, PhantomData)
    }

    /// The default options with the object id type set to `oid_type`.
    pub fn with_oid_type(oid_type: OidType) -> Self {
        let mut options = Self::new();
        options.as_mut().set_oid_type(Some(oid_type));
        options
    }

    /// The raw `git_index_options`, for C code that takes the struct by value or keeps
    /// it; it owns nothing.
    pub fn into_raw(self) -> ffi::git_index_options {
        self.0
    }
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for IndexOptions {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.as_ref(), other.as_ref());
        (a.version(), a.raw_oid_type()) == (b.version(), b.raw_oid_type())
    }
}

impl Eq for IndexOptions {}

impl fmt::Debug for IndexOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl IndexOptionsRef<'_> {
    /// The struct's version, `GIT_INDEX_OPTIONS_VERSION`.
    ///
    /// Field: git_index_options.version
    pub fn version(&self) -> u32 {
        // SAFETY: a read through the shared handle's pointer to live options; the field
        // is a plain integer.
        unsafe { addr_of!((*self.as_ptr()).version).read() }
    }

    /// The type of the object ids in the index; `None` when unset, which libgit2 reads
    /// as [`OidType::default`] (SHA-1).
    ///
    /// Field: git_index_options.oid_type
    pub fn oid_type(&self) -> Option<OidType> {
        OidType::from_raw(self.raw_oid_type())
    }

    /// `oid_type` as stored: 0, or a named type's value (see the setter).
    fn raw_oid_type(&self) -> ffi::git_oid_t {
        // SAFETY: a read through the shared handle's pointer to live options; the field
        // is a plain integer.
        unsafe { addr_of!((*self.as_ptr()).oid_type).read() }
    }
}

impl fmt::Debug for IndexOptionsRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexOptions")
            .field("version", &self.version())
            .field("oid_type", &self.oid_type())
            .finish()
    }
}

impl IndexOptionsMut<'_> {
    /// Sets the type of the object ids in the index, or unsets it with `None` (stored
    /// as 0) so that libgit2 picks its default.
    ///
    /// Field: git_index_options.oid_type
    pub fn set_oid_type(&mut self, oid_type: Option<OidType>) {
        let raw = oid_type.map_or(0, OidType::raw);
        // SAFETY: a write through the exclusive handle's pointer to live options of 0 or
        // a named type, the only values `git_index_open_ext` accepts without asserting.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).oid_type).write(raw) }
    }
}

/// Refuses an empty path, with a `GIT_ERROR_INDEX` error, before libgit2 sees it: see
/// [`Index::open`].
fn path_is_usable(path: &CStr) -> bool {
    if path.is_empty() {
        set(
            ErrorClass::Index,
            c"failed to open index: the path is empty",
        );
        return false;
    }
    true
}

impl Index {
    /// A fresh, empty in-memory index, backed by no file and no repository: libgit2
    /// refuses to read or write it, and fails the routines that need a repository's
    /// object database or working directory (adding by path, writing a tree), but every
    /// in-memory operation works on it. Its object ids are SHA1
    /// ([`with_oid_type`](Self::with_oid_type) chooses).
    ///
    /// Fails, with libgit2's error, only when it runs out of memory or cannot be
    /// initialised. [`IndexBox::new`](CBox::new) and [`IndexArc::new`](CArc::new) build
    /// the same index, reporting failure as `None`.
    ///
    /// Wraps: git_index_new
    pub fn new() -> Result<IndexBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, which libgit2 writes only on success. The
        // runtime is pinned, so the allocation reads a settled allocator and a failure is
        // reported on live thread-local state.
        let rc = unsafe { ffi::git_index_new(&mut out) };
        // SAFETY: `git_index_new` leaves `out` NULL on failure (adopted as `None`), or
        // writes a fresh index holding its only reference, which the caller owns, released
        // by `git_index_free` as `IndexFree` does.
        constructed(rc, unsafe { IndexBox::from_c(out) })
    }

    /// An index backed by the index file at `path`, without a repository; its contents
    /// are read from the file when it exists, and the index is empty when it does not
    /// (nor is the file created until the index is written). Its object ids are SHA1
    /// ([`open_with_oid_type`](Self::open_with_oid_type) chooses).
    ///
    /// The path is copied, not retained. The routines that need a repository fail on the
    /// index as on [`new`](Self::new)'s.
    ///
    /// An empty `path` is refused, with a `GIT_ERROR_INDEX` error, before libgit2 sees
    /// it. libgit2 accepts it, but writing the index it then builds hands `memcpy` a NULL
    /// source (`git_filebuf_open_withsize`, `src/util/filebuf.c`), which is undefined
    /// behaviour.
    ///
    /// Fails, with libgit2's error, when the file exists but cannot be read (a directory,
    /// say) or is not a valid index, and when libgit2 runs out of memory or cannot be
    /// initialised; libgit2 reports each of these as its generic error (`-1`).
    ///
    /// libgit2 leaks a few bytes when it refuses a version 4 file whose last entry runs
    /// past its end (a truncated file, say): its reader returns before freeing the path
    /// it decompressed for that entry.
    ///
    /// Wraps: git_index_open
    pub fn open(path: &CStr) -> Result<IndexBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if !path_is_usable(path) {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, which libgit2 writes only on success; `path` is
        // a non-NULL NUL-terminated string, read and copied for the call, never retained.
        // The runtime is pinned.
        let rc = unsafe { ffi::git_index_open(&mut out, path.as_ptr()) };
        // SAFETY: `git_index_open` leaves `out` NULL on failure (freeing the half-built
        // index itself; adopted as `None`), or writes a fresh index holding its only
        // reference, which the caller owns, released by `git_index_free` as `IndexFree`
        // does.
        constructed(rc, unsafe { IndexBox::from_c(out) })
    }

    /// As [`new`](Self::new), for an index whose object ids are of `oid_type`; the type
    /// is the only setting `git_index_options` holds. [`new`](Self::new) is this with
    /// [`OidType::Sha1`].
    ///
    /// libgit2 asserts the type is one it names, which [`OidType`] guarantees.
    ///
    /// Wraps: git_index_new_ext
    pub fn with_oid_type(oid_type: OidType) -> Result<IndexBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let opts = IndexOptions::with_oid_type(oid_type);
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, which libgit2 writes only on success; `opts`
        // is a live options struct of this version naming a valid type, which the
        // routine reads for the call and does not retain. The runtime is pinned.
        let rc = unsafe { ffi::git_index_new_ext(&mut out, opts.as_ref().as_ptr()) };
        // SAFETY: as in `new`: `out` is NULL, or a fresh index holding its only
        // reference, released by `git_index_free` as `IndexFree` does.
        constructed(rc, unsafe { IndexBox::from_c(out) })
    }

    /// As [`open`](Self::open), for an index whose object ids are of `oid_type`: an
    /// existing file is read as an index of that type, and fails to read as one of
    /// another. [`open`](Self::open) is this with [`OidType::Sha1`]; the type is the
    /// only setting `git_index_options` holds, and libgit2 asserts it is one it names,
    /// which [`OidType`] guarantees.
    ///
    /// An empty `path` is refused as [`open`](Self::open) refuses it.
    ///
    /// Wraps: git_index_open_ext
    pub fn open_with_oid_type(path: &CStr, oid_type: OidType) -> Result<IndexBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if !path_is_usable(path) {
            return Err(last());
        }
        let opts = IndexOptions::with_oid_type(oid_type);
        let opts = opts.as_ref();
        let mut out = ptr::null_mut();
        // SAFETY: as in `open`; `opts` borrows a live options struct of this version
        // naming a valid type, which the routine reads for the call and does not retain.
        let rc = unsafe { ffi::git_index_open_ext(&mut out, path.as_ptr(), opts.as_ptr()) };
        // SAFETY: as in `open`.
        constructed(rc, unsafe { IndexBox::from_c(out) })
    }
}

/// The NUL-terminated key libgit2's extension routines search for `signature`: its four
/// bytes and a terminator. `None`, with a `GIT_ERROR_INVALID` error, when the signature
/// holds a NUL byte: libgit2 copies all four bytes into an extension it adds, but
/// compares keys with `strncmp`, which stops at the first NUL (`extension_srch`), so such
/// an extension would match every key sharing the bytes before it.
fn extension_key(signature: &[u8; 4]) -> Option<[u8; 5]> {
    if signature.contains(&0) {
        set(
            ErrorClass::Invalid,
            c"an index extension signature cannot hold a NUL byte",
        );
        return None;
    }
    let [a, b, c, d] = *signature;
    Some([a, b, c, d, 0])
}

impl IndexRef<'_> {
    /// The number of entries in the index, conflict entries (one per side) included.
    ///
    /// Wraps: git_index_entrycount
    pub fn entry_count(&self) -> usize {
        // SAFETY: the handle addresses a live index, whose entry vector's length libgit2
        // only reads.
        unsafe { ffi::git_index_entrycount(self.as_ptr()) }
    }

    /// Whether any entry is a conflict side: of a stage other than
    /// [`Stage::Normal`].
    ///
    /// Wraps: git_index_has_conflicts
    pub fn has_conflicts(&self) -> bool {
        // SAFETY: the handle addresses a live index; libgit2 only reads its entries' flags
        // and returns no pointer.
        unsafe { ffi::git_index_has_conflicts(self.as_ptr()) > 0 }
    }

    /// The capabilities the index applies: those it was built with (none), last set, or
    /// read from its owning repository's configuration. Never
    /// [`CapabilitySetting::FromOwner`]: libgit2 resolves that when it is set.
    ///
    /// Wraps: git_index_caps
    pub fn capabilities(&self) -> Capabilities {
        // SAFETY: the handle addresses a live index, whose three capability bits libgit2
        // only reads.
        let bits = unsafe { ffi::git_index_caps(self.as_ptr()) };
        // libgit2 composes the result from the three published bits alone.
        Capabilities::from_bits(bits).unwrap_or(Capabilities::all())
    }

    /// Whether a repository keeps this index as its own: the index of a repository's
    /// working directory ([`RepositoryRef::index`]) or one installed with
    /// [`RepositoryMut::set_index`], until the repository releases it.
    ///
    /// libgit2 lends that repository uncounted, and an index may outlive it, so there is
    /// no handle to it here: nothing ties the index's owners to the repository's borrow.
    /// [`is_owned_by`](Self::is_owned_by) tells whether a repository at hand is the one.
    ///
    /// A sole owner ([`IndexBox`]) never has one: a repository counts a reference of its
    /// own.
    ///
    /// [`RepositoryMut::set_index`]: crate::libgit2::repository::repository::RepositoryMut::set_index
    ///
    /// Wraps: git_index_owner
    pub fn has_owner(&self) -> bool {
        // SAFETY: the handle addresses a live index, whose owner field libgit2 only reads;
        // the repository pointer is not dereferenced.
        !unsafe { ffi::git_index_owner(self.as_ptr()) }.is_null()
    }

    /// Whether `repo` is the repository keeping this index as its own (see
    /// [`has_owner`](Self::has_owner)).
    ///
    /// Wraps: git_index_owner
    pub fn is_owned_by(&self, repo: RepositoryRef<'_>) -> bool {
        // SAFETY: as in `has_owner`; the pointer is only compared.
        let owner = unsafe { ffi::git_index_owner(self.as_ptr()) };
        ptr::eq(owner.cast_const(), repo.as_ptr())
    }
}

impl IndexMut<'_> {
    /// Removes every entry, conflict and resolved conflict (`REUC`), rename record
    /// (`NAME`), the tree cache and every extension, leaving an empty index that still
    /// reads from and writes to its file. Only the index in memory changes until it is
    /// written.
    ///
    /// Fails only with libgit2's error, which a sole owner's index never reports: the
    /// removals allocate only while a snapshot iterator reads the index.
    ///
    /// Wraps: git_index_clear
    pub fn clear(&mut self) -> Result<(), ErrorBuf> {
        // SAFETY: the exclusive handle addresses a live index that no borrow reads: every
        // entry, name and extension it frees is unreachable from Rust.
        let rc = unsafe { ffi::git_index_clear(self.as_mut_ptr()) };
        if rc < 0 { Err(last()) } else { Ok(()) }
    }

    /// Adds, or updates, the entry for the file at `path`, relative to the working
    /// directory of the repository that owns the index, whose contents it writes to the
    /// object database as a blob (through the repository's filters); ignore rules are not
    /// consulted. Resolves a conflict at `path`, moving its sides to the resolved-conflict
    /// (`REUC`) records. A directory holding a repository is added as a submodule entry.
    ///
    /// The index must belong to a repository with a working directory. An exclusive
    /// handle comes only from a sole owner, which no repository shares, and no wrapper
    /// yet lends one to a repository's own index, so through the safe API alone this
    /// fails as below for want of a repository.
    ///
    /// Fails with libgit2's error when the index has no owning repository (as a sole
    /// owner's never has: see [`IndexRef::has_owner`]), with [`ErrorCode::BareRepo`] when
    /// that repository is bare, [`ErrorCode::Directory`] for a directory that is not a
    /// repository, [`ErrorCode::NotFound`] when the file does not exist, and with
    /// libgit2's error for a path an index cannot hold (`""`, `"d/"`, `".git/config"`,
    /// `"./a"`; [`ErrorCode::InvalidSpec`] for one too long for the file system) or a
    /// file it cannot read.
    ///
    /// libgit2 leaks the entry it built, and the nested repository it opened, when a
    /// directory holds a repository that is no submodule and has no commit
    /// ([`ErrorCode::UnbornBranch`]) or cannot be opened (`add_repo_as_submodule`,
    /// `src/libgit2/index.c`).
    ///
    /// [`ErrorCode::BareRepo`]: crate::libgit2::util::errors::ErrorCode::BareRepo
    /// [`ErrorCode::Directory`]: crate::libgit2::util::errors::ErrorCode::Directory
    /// [`ErrorCode::NotFound`]: crate::libgit2::util::errors::ErrorCode::NotFound
    /// [`ErrorCode::InvalidSpec`]: crate::libgit2::util::errors::ErrorCode::InvalidSpec
    /// [`ErrorCode::UnbornBranch`]: crate::libgit2::util::errors::ErrorCode::UnbornBranch
    ///
    /// Wraps: git_index_add_bypath
    pub fn add_path(&mut self, path: &CStr) -> Result<(), RepositoryError> {
        // SAFETY: the exclusive handle addresses a live index that no borrow reads, and
        // `path` is a NUL-terminated string read and copied for the call. Whatever libgit2
        // reaches through the owning repository (its object database, filters, submodule
        // cache) is the repository's, used on this thread, which neither is `Send`.
        let rc = unsafe { ffi::git_index_add_bypath(self.as_mut_ptr(), path.as_ptr()) };
        if rc < 0 {
            Err(RepositoryError::from_libgit2(rc))
        } else {
            Ok(())
        }
    }

    /// Adds, or updates, the entries of the files in the owning repository's working
    /// directory that match `pathspec` (every file for an empty one), as
    /// [`add_path`](Self::add_path) does for one, and removes the entries of tracked
    /// files that are gone. Ignored files are skipped unless already tracked, or unless
    /// `flags` is exactly [`AddFlags::FORCE`] (see [`AddFlags`]);
    /// [`AddFlags::CHECK_PATHSPEC`] fails the call, with [`ErrorCode::InvalidSpec`], when
    /// a pattern names an ignored, untracked file. Resolves the conflicts of the paths it
    /// adds.
    ///
    /// `callback` hears of each matching path before it is applied, with the pattern
    /// that matched it, and decides ([`MatchedPathResult`]): a negative result stops the
    /// scan, which then fails with that code; a panic stops it with `GIT_EUSER` and
    /// resumes once libgit2 has returned.
    ///
    /// The index must belong to a repository, as for [`add_path`](Self::add_path), which
    /// no exclusive handle the safe API hands out does yet. Refused, with a
    /// `GIT_ERROR_INDEX` error and libgit2's generic code, when the index has no owning
    /// repository (see [`IndexRef::has_owner`]), as a sole owner's never has. libgit2
    /// instead asks the missing repository whether it is bare, and reports a bare
    /// repository ([`ErrorCode::BareRepo`]). A pathspec mixing empty and non-empty
    /// patterns is refused, with a `GIT_ERROR_INVALID` error, before libgit2 reads past
    /// its empty ones (see [`Pathspec::new`]).
    ///
    /// Otherwise fails with [`ErrorCode::BareRepo`] for a bare repository, and with
    /// libgit2's error when a file cannot be read or written as a blob.
    ///
    /// [`ErrorCode::BareRepo`]: crate::libgit2::util::errors::ErrorCode::BareRepo
    /// [`ErrorCode::InvalidSpec`]: crate::libgit2::util::errors::ErrorCode::InvalidSpec
    /// [`Pathspec::new`]: crate::libgit2::index::pathspec::Pathspec::new
    ///
    /// Wraps: git_index_add_all
    pub fn add_all(
        &mut self,
        pathspec: StrArrayRef<'_>,
        flags: AddFlags,
        callback: Option<&mut MatchedPathCallback<'_>>,
    ) -> Result<(), RepositoryError> {
        if !self.as_ref().has_owner() {
            set(
                ErrorClass::Index,
                c"cannot add all; the index is not backed up by a repository",
            );
            return Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()));
        }
        if !patterns_are_parsable(pathspec) {
            return Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()));
        }
        let index = self.as_mut_ptr();
        let scan = |callback, payload| {
            // SAFETY: `index` is the exclusive handle's live index, which the callback
            // cannot reach; `pathspec` is a live array of patterns, none empty beside a
            // non-empty one, read for the call. The callback pair is NULL, or from
            // `scan_with`, which keeps its payload live and exclusive for the call.
            unsafe {
                ffi::git_index_add_all(index, pathspec.as_ptr(), flags.bits(), callback, payload)
            }
        };
        let rc = match callback {
            Some(callback) => callback.scan_with(scan),
            None => scan(None, ptr::null_mut()),
        };
        if rc < 0 {
            Err(RepositoryError::from_libgit2(rc))
        } else {
            Ok(())
        }
    }

    /// The position of the first entry at `path`, of any stage, in path order; `None`
    /// when the index has none. Compares as the index's
    /// [`Capabilities::IGNORE_CASE`] says.
    ///
    /// Takes the exclusive handle: libgit2 sorts the entries first.
    ///
    /// Wraps: git_index_find
    pub fn find(&mut self, path: &CStr) -> Option<usize> {
        let mut position = 0;
        // SAFETY: the exclusive handle addresses a live index, whose entry vector libgit2
        // may sort, which no borrow reads; `position` is a writable slot, and `path` a
        // NUL-terminated string read for the call.
        let rc = unsafe { ffi::git_index_find(&mut position, self.as_mut_ptr(), path.as_ptr()) };
        (rc >= 0).then_some(position)
    }

    /// The position of the first entry whose path starts with `prefix`, of any stage,
    /// in path order; `None` when no path does. Suffix a directory's path with `/` to
    /// find the first entry inside it. The empty prefix finds the first entry. The
    /// prefix is compared byte for byte, even when the index ignores case.
    ///
    /// Deliberate divergences: when an entry's path is `prefix` itself and it has
    /// several stages (a conflict), libgit2's binary search stops at whichever stage it
    /// meets first, not necessarily the first one; this backs up to the first. In an
    /// index that ignores case, libgit2 searches in case-folded order and then compares
    /// the one entry it lands on byte for byte, so it misses a matching path after
    /// another that differs only in case (`"a/"` in `["A/x", "a/y"]`); this scans the
    /// entries instead.
    ///
    /// Takes the exclusive handle: libgit2 sorts the entries first.
    ///
    /// Wraps: git_index_find_prefix
    pub fn find_prefix(&mut self, prefix: &CStr) -> Option<usize> {
        let caps = self.as_ref().capabilities();
        if caps.contains(Capabilities::IGNORE_CASE) {
            let count = self.as_ref().entry_count();
            return (0..count).find(|&position| self.path_at_starts_with(position, prefix));
        }
        let mut position = 0;
        // SAFETY: as in `find`.
        let rc = unsafe {
            ffi::git_index_find_prefix(&mut position, self.as_mut_ptr(), prefix.as_ptr())
        };
        if rc < 0 {
            return None;
        }
        while position > 0 && self.path_at_starts_with(position - 1, prefix) {
            position -= 1;
        }
        Some(position)
    }

    /// Whether the entry at `position` exists and its path starts with `prefix`, as
    /// libgit2 checks the entry its prefix search found (`git__prefixcmp`).
    fn path_at_starts_with(&mut self, position: usize, prefix: &CStr) -> bool {
        // SAFETY: the exclusive handle addresses a live index, whose entry vector
        // libgit2 may sort, which no borrow reads. It returns NULL past the end, or one of
        // the index's entries, with its path in its own allocation, which nothing writes
        // or frees while the handle, used only here, stays borrowed.
        let entry = unsafe {
            IndexEntryRef::from_ptr(ffi::git_index_get_byindex(self.as_mut_ptr(), position))
        };
        entry.is_some_and(|entry| entry.path().to_bytes().starts_with(prefix.to_bytes()))
    }

    /// Removes the conflict sides (entries of a stage other than
    /// [`Stage::Normal`]) at `path`, leaving its normal entry, if any; succeeds without
    /// change when `path` has no conflict.
    ///
    /// Fails with [`ErrorCode::NotFound`] when the index has no entry at `path` at all.
    ///
    /// [`ErrorCode::NotFound`]: crate::libgit2::util::errors::ErrorCode::NotFound
    ///
    /// Wraps: git_index_conflict_remove
    pub fn remove_conflict(&mut self, path: &CStr) -> Result<(), RepositoryError> {
        // SAFETY: the exclusive handle addresses a live index that no borrow reads, so the
        // entries libgit2 frees are unreachable from Rust; `path` is a NUL-terminated
        // string read for the call.
        let rc = unsafe { ffi::git_index_conflict_remove(self.as_mut_ptr(), path.as_ptr()) };
        if rc < 0 {
            Err(RepositoryError::from_libgit2(rc))
        } else {
            Ok(())
        }
    }

    /// Removes every conflict side: every entry of a stage other than
    /// [`Stage::Normal`].
    ///
    /// Fails only with libgit2's error, which a sole owner's index never reports, as
    /// for [`clear`](Self::clear).
    ///
    /// Wraps: git_index_conflict_cleanup
    pub fn clear_conflicts(&mut self) -> Result<(), ErrorBuf> {
        // SAFETY: as in `remove_conflict`.
        let rc = unsafe { ffi::git_index_conflict_cleanup(self.as_mut_ptr()) };
        if rc < 0 { Err(last()) } else { Ok(()) }
    }

    /// A copy of the data of the extension with `signature`, its header excluded; `None`
    /// when the index has none. The extensions are those libgit2 does not interpret, read
    /// from the index file or set with [`set_extension`](Self::set_extension): never the
    /// tree cache (`TREE`) or the conflict records (`REUC`, `NAME`) libgit2 parses.
    ///
    /// The copy is in libgit2's allocator, which it is released to.
    ///
    /// A signature holding a NUL byte is refused, with a `GIT_ERROR_INVALID` error: see
    /// [`set_extension`](Self::set_extension). Otherwise fails only with libgit2's error
    /// when the copy cannot be allocated.
    ///
    /// Takes the exclusive handle: libgit2 sorts the extensions first.
    ///
    /// Wraps: git_index_extension_lookup
    pub fn extension(&mut self, signature: &[u8; 4]) -> Result<Option<OdbDataVec<u8>>, ErrorBuf> {
        let key = extension_key(signature).ok_or_else(last)?;
        let mut out = ffi::git_buf {
            ptr: ptr::null_mut(),
            reserved: 0,
            size: 0,
        };
        // SAFETY: `out` is a writable `git_buf` owning nothing, which libgit2 overwrites
        // without freeing; the exclusive handle addresses a live index, whose extension
        // vector libgit2 may sort; `key` is a NUL-terminated string read for the call.
        let rc = unsafe {
            ffi::git_index_extension_lookup(&mut out, self.as_mut_ptr(), key.as_ptr().cast())
        };
        if rc == ErrorCode::NotFound.raw() {
            return Ok(None);
        }
        if rc < 0 {
            return Err(last());
        }
        // SAFETY: on success `out.ptr` is a fresh `git__malloc` allocation of exactly
        // `out.size` bytes, all copied from the extension and owned by the caller; it is
        // not NUL-terminated and `reserved` stays 0, so it is not a `Buf`'s. libgit2's
        // allocator settles it, through `git__free`, as `OdbDataArrayFree` does.
        let data = unsafe { OdbDataVec::from_raw_parts_or_empty(out.ptr.cast::<u8>(), out.size) };
        Ok(Some(data.expect("a successful lookup allocates its copy")))
    }

    /// Sets the extension with `signature` to `data`, replacing any it has; libgit2
    /// writes it to the index file after its own extensions, and keeps the data as is.
    ///
    /// libgit2 does not check the signature. Git reads an extension whose signature does
    /// not start with an uppercase ASCII letter as one the reader must understand, so an
    /// index file holding one is unreadable (libgit2 fails with "unsupported mandatory
    /// extension"); one named after libgit2's own (`TREE`, `REUC`, `NAME`) is written
    /// beside them, and read back as theirs.
    ///
    /// A signature holding a NUL byte is refused, with a `GIT_ERROR_INVALID` error, before
    /// libgit2 sees it: libgit2 keeps all four bytes, but compares only those before the
    /// NUL when it searches for one. So is `data` of 4 GiB or more (`u32::MAX` bytes),
    /// whose size the extension header cannot hold. Otherwise fails only with libgit2's
    /// error when it runs out of memory.
    ///
    /// libgit2 leaks the new extension when it runs out of memory growing its list.
    ///
    /// Wraps: git_index_extension_add
    pub fn set_extension(&mut self, signature: &[u8; 4], data: &[u8]) -> Result<(), ErrorBuf> {
        let key = extension_key(signature).ok_or_else(last)?;
        if !u32::try_from(data.len()).is_ok_and(|len| len < u32::MAX) {
            set(
                ErrorClass::Invalid,
                c"index extension data cannot reach 4 GiB",
            );
            return Err(last());
        }
        // SAFETY: the exclusive handle's live index lends nothing, so the extension it
        // replaces is unreachable from Rust; `key` holds four bytes and a terminator;
        // `data` is `data.len()` (< `u32::MAX`) readable bytes at a non-NULL pointer.
        let rc = unsafe {
            ffi::git_index_extension_add(
                self.as_mut_ptr(),
                key.as_ptr().cast(),
                data.as_ptr().cast(),
                data.len(),
            )
        };
        if rc < 0 { Err(last()) } else { Ok(()) }
    }

    /// Removes the extension with `signature`; `false` when the index has none.
    ///
    /// A signature holding a NUL byte is refused, with a `GIT_ERROR_INVALID` error: see
    /// [`set_extension`](Self::set_extension).
    ///
    /// Wraps: git_index_extension_remove
    pub fn remove_extension(&mut self, signature: &[u8; 4]) -> Result<bool, ErrorBuf> {
        let key = extension_key(signature).ok_or_else(last)?;
        // SAFETY: as in `set_extension`, for the extension libgit2 frees; `key` is a
        // NUL-terminated string read for the call.
        let rc = unsafe { ffi::git_index_extension_remove(self.as_mut_ptr(), key.as_ptr().cast()) };
        if rc == ErrorCode::NotFound.raw() {
            return Ok(false);
        }
        if rc < 0 { Err(last()) } else { Ok(true) }
    }
}

/// Flags for adding files that match a pathspec.
///
/// These are copied integers with no foreign storage or lifecycle. Unknown bits
/// are retained, and combinations are passed to C unchanged. In particular, C's
/// workdir-diff implementation includes ignored files only when the entire flags
/// value equals `FORCE`, not when `FORCE` is combined with another flag.
///
/// Wraps: git_index_add_option_t
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AddFlags(ffi::git_index_add_option_t);

impl AddFlags {
    /// Normal addition, without any flags.
    pub const DEFAULT: Self = Self(ffi::GIT_INDEX_ADD_DEFAULT);
    /// Request adding ignored files.
    pub const FORCE: Self = Self(ffi::GIT_INDEX_ADD_FORCE);
    /// Request literal pathspec matching instead of wildcard matching.
    pub const DISABLE_PATHSPEC_MATCH: Self = Self(ffi::GIT_INDEX_ADD_DISABLE_PATHSPEC_MATCH);
    /// Check for explicitly named ignored files, unless `FORCE` is also set.
    pub const CHECK_PATHSPEC: Self = Self(ffi::GIT_INDEX_ADD_CHECK_PATHSPEC);

    /// An empty flag set, equivalent to `DEFAULT`.
    pub const fn empty() -> Self {
        Self::DEFAULT
    }

    /// Retains all bits, including those not named by this version of libgit2.
    pub const fn from_bits_retain(bits: core::ffi::c_uint) -> Self {
        Self(bits)
    }

    /// The unchanged unsigned flags for the C API.
    pub const fn bits(self) -> core::ffi::c_uint {
        self.0
    }

    /// Whether every bit in `other` is present (true for the empty set).
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Combines two sets without normalizing or discarding any bits.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Removes every bit present in `other`.
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl core::ops::BitOr for AddFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for AddFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Explicit filesystem capabilities affecting index actions.
///
/// Only the three published capability bits are accepted. The C value `-1` is
/// not a set of capabilities: use [`CapabilitySetting::FromOwner`] to request
/// reading the owning repository's configuration instead. Values own no foreign
/// resources and may be freely copied, shared or moved between threads.
///
/// Wraps: git_index_capability_t
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Capabilities(ffi::git_index_capability_t);

impl Capabilities {
    /// Match index paths case-insensitively.
    pub const IGNORE_CASE: Self = Self(ffi::GIT_INDEX_CAPABILITY_IGNORE_CASE);
    /// Do not trust filesystem executable-mode bits.
    pub const NO_FILEMODE: Self = Self(ffi::GIT_INDEX_CAPABILITY_NO_FILEMODE);
    /// The filesystem does not support symbolic links.
    pub const NO_SYMLINKS: Self = Self(ffi::GIT_INDEX_CAPABILITY_NO_SYMLINKS);

    const KNOWN: c_int = Self::IGNORE_CASE.0 | Self::NO_FILEMODE.0 | Self::NO_SYMLINKS.0;

    /// No capability overrides.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// All three explicit capabilities, not the `FROM_OWNER` command.
    pub const fn all() -> Self {
        Self(Self::KNOWN)
    }

    /// Checks that `bits` contains only explicit, published capability bits.
    /// Returns `None` for `FROM_OWNER` and for any unknown bit.
    pub const fn from_bits(bits: c_int) -> Option<Self> {
        if bits & !Self::KNOWN == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// The explicit capability bits, suitable for the C API.
    pub const fn bits(self) -> c_int {
        self.0
    }

    /// Whether every bit in `other` is present (true for the empty set).
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Combines two explicit sets; cannot produce the `FROM_OWNER` command.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Removes every bit present in `other`.
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl core::ops::BitOr for Capabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for Capabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Selects either an explicit capability set or the owning repository's config.
///
/// This Rust enum is not passed across FFI: [`to_raw`](Self::to_raw) supplies the
/// signed C value. `FromOwner` requires an index with an owning repository; C
/// reports an error otherwise. Unlike explicit sets, this command cannot be ORed:
///
/// ```compile_fail
/// use libgit2::libgit2::index::index::{Capabilities, CapabilitySetting};
/// let _ = CapabilitySetting::FromOwner | Capabilities::IGNORE_CASE;
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CapabilitySetting {
    /// Use exactly this explicit set.
    Explicit(Capabilities),
    /// Read `core.ignorecase`, `core.filemode` and `core.symlinks` from the owner.
    FromOwner,
}

impl CapabilitySetting {
    /// The signed C argument; only `FromOwner` produces the sentinel `-1`.
    pub const fn to_raw(self) -> c_int {
        match self {
            Self::Explicit(caps) => caps.bits(),
            Self::FromOwner => ffi::GIT_INDEX_CAPABILITY_FROM_OWNER,
        }
    }
}

impl From<Capabilities> for CapabilitySetting {
    fn from(caps: Capabilities) -> Self {
        Self::Explicit(caps)
    }
}

/// The result of a matched-path callback: zero applies the operation, a positive
/// code skips this path, and a negative code aborts the scan. Negative codes are
/// returned unchanged by libgit2; positive codes are consumed by the scan.
///
/// All C `int` values are valid, including application-specific abort codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MatchedPathResult(c_int);

impl MatchedPathResult {
    /// Apply the requested add, remove, or update to this path.
    pub const APPLY: Self = Self(0);
    /// Leave this path alone, and continue scanning.
    pub const SKIP: Self = Self(1);
    /// Abort with libgit2's user-callback error (`GIT_EUSER`).
    pub const ABORT: Self = Self(ffi::GIT_EUSER);

    /// Preserve an arbitrary C callback result without restricting its magnitude.
    pub const fn from_code(code: c_int) -> Self {
        Self(code)
    }

    /// The exact code returned to libgit2.
    pub const fn code(self) -> c_int {
        self.0
    }
}

type MatchedPathFn<'f> = dyn FnMut(&CStr, Option<&CStr>) -> MatchedPathResult + 'f;

/// A synchronous add/remove/update callback borrowing its closure exclusively.
///
/// `path` is the index/workdir path. `matched_pathspec` is the parsed pattern
/// matching it, or `None` when the pathspec holds no usable pattern (it is empty,
/// its entries are all empty, or each is a `#` comment or starts with a line
/// break), and then every path matches. Both are immutable, potentially non-UTF-8
/// strings, lent only for this invocation. Copy a string to keep it: C may delete
/// its storage immediately after the callback returns.
/// Neither string nor the payload transfers ownership, in any of the C callers.
///
/// A panic aborts the scan with `GIT_EUSER` and resumes after C returns and
/// cleans up. Each new invocation/scan resets the stop state. The handle cannot
/// be sent or shared across threads; its exclusive borrow prevents same-handle
/// reentry, while calling a different callback is allowed.
///
/// Neither argument may escape the callback:
/// ```compile_fail
/// use std::ffi::CStr;
/// use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
/// let mut kept = None;
/// let mut f = |path: &CStr, _: Option<&CStr>| {
///     kept = Some(path);
///     MatchedPathResult::APPLY
/// };
/// let _ = MatchedPathCallback::new(&mut f);
/// ```
/// ```compile_fail
/// use std::ffi::CStr;
/// use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
/// let mut kept = None;
/// let mut f = |_: &CStr, pattern: Option<&CStr>| {
///     kept = pattern;
///     MatchedPathResult::APPLY
/// };
/// let _ = MatchedPathCallback::new(&mut f);
/// ```
/// The closure and its captured state must outlive the handle:
/// ```compile_fail
/// use std::ffi::CStr;
/// use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
/// let mut owned = Vec::new();
/// let mut f = move |_: &CStr, _: Option<&CStr>| {
///     owned.push(1);
///     MatchedPathResult::APPLY
/// };
/// let mut callback = MatchedPathCallback::new(&mut f);
/// drop(f);
/// callback.call(c"file", None);
/// ```
/// Exclusive calls cannot overlap:
/// ```compile_fail
/// use std::ffi::CStr;
/// use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
/// let mut f = |_: &CStr, _: Option<&CStr>| MatchedPathResult::APPLY;
/// let mut callback = MatchedPathCallback::new(&mut f);
/// let first = &mut callback;
/// callback.call(c"file", None);
/// first.call(c"file", None);
/// ```
/// ```compile_fail
/// use std::ffi::CStr;
/// use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
/// let mut f = |_: &CStr, _: Option<&CStr>| MatchedPathResult::APPLY;
/// let callback = MatchedPathCallback::new(&mut f);
/// std::thread::scope(|s| { s.spawn(move || drop(callback)); });
/// ```
///
/// Wraps: git_index_matched_path_cb
/// Handwritten
pub struct MatchedPathCallback<'f> {
    f: &'f mut MatchedPathFn<'f>,
    stopped: Option<c_int>,
    panic: Option<Box<dyn std::any::Any + Send>>,
}

impl<'f> MatchedPathCallback<'f> {
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(&CStr, Option<&CStr>) -> MatchedPathResult + 'f,
    {
        Self {
            f,
            stopped: None,
            panic: None,
        }
    }

    /// Invoke through the same trampoline C uses. Any panic resumes here, never
    /// across the C ABI. The input borrows last only for the closure's call.
    pub fn call(&mut self, path: &CStr, matched_pathspec: Option<&CStr>) -> MatchedPathResult {
        let code = self.scan_with(|callback, payload| {
            // SAFETY: scan_with supplies this live, exclusively borrowed handle;
            // both string pointers are immutable and valid through the call.
            unsafe {
                callback.expect("the trampoline is set")(
                    path.as_ptr(),
                    matched_pathspec.map_or(ptr::null(), CStr::as_ptr),
                    payload,
                )
            }
        });
        MatchedPathResult::from_code(code)
    }

    /// The negative callback code that stopped the last call/scan, including
    /// `GIT_EUSER` for a panic. A skipped path does not stop the scan.
    pub fn stopped(&self) -> Option<c_int> {
        self.stopped
    }

    /// Internal seam for the higher-layer add/remove/update wrappers. `scan`
    /// must use the pair synchronously on this thread, never retain it or invoke
    /// it recursively, and keep both input strings immutable for each call.
    /// No Rust reborrow of this handle occurs while C holds the payload pointer.
    pub(crate) fn scan_with<R>(
        &mut self,
        scan: impl FnOnce(ffi::git_index_matched_path_cb, *mut core::ffi::c_void) -> R,
    ) -> R {
        self.stopped = None;
        let result = scan(Some(Self::trampoline), ptr::from_mut(self).cast());
        if let Some(panic) = self.panic.take() {
            std::panic::resume_unwind(panic);
        }
        result
    }

    /// # Safety
    /// `payload` is a live exclusively used MatchedPathCallback on this thread.
    /// `path` is a non-null immutable NUL string; `pattern` is null or such a string.
    /// Both strings live for the invocation; no recursive or concurrent call occurs.
    unsafe extern "C" fn trampoline(
        path: *const core::ffi::c_char,
        pattern: *const core::ffi::c_char,
        payload: *mut core::ffi::c_void,
    ) -> c_int {
        // SAFETY: the internal seam guarantees the live exclusive handle and forbids
        // overlapping invocations or accesses to it during this call.
        let callback = unsafe { &mut *payload.cast::<Self>() };
        if let Some(code) = callback.stopped {
            return code;
        }
        // SAFETY: the contract supplies a non-null immutable NUL string for this call.
        let path = unsafe { CStr::from_ptr(path) };
        let pattern = if pattern.is_null() {
            None
        } else {
            // SAFETY: non-null pattern is an immutable NUL string for this call.
            Some(unsafe { CStr::from_ptr(pattern) })
        };
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (callback.f)(path, pattern)));
        let code = match result {
            Ok(result) => result.code(),
            Err(panic) => {
                callback.panic = Some(panic);
                MatchedPathResult::ABORT.code()
            }
        };
        if code < 0 {
            callback.stopped = Some(code);
        }
        code
    }
}

/// An index stage, or the wildcard accepted by stage-matching operations.
/// `Any` is not a stored entry stage; operations requiring a concrete stage must
/// reject it. Raw values not named by the header are never transmuted into Rust.
///
/// Wraps: git_index_stage_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Stage {
    Any = ffi::GIT_INDEX_STAGE_ANY,
    Normal = ffi::GIT_INDEX_STAGE_NORMAL,
    Ancestor = ffi::GIT_INDEX_STAGE_ANCESTOR,
    Ours = ffi::GIT_INDEX_STAGE_OURS,
    Theirs = ffi::GIT_INDEX_STAGE_THEIRS,
}

impl Stage {
    /// Decode one of this version's named stage values, including the wildcard.
    pub const fn from_raw(raw: ffi::git_index_stage_t) -> Option<Self> {
        match raw {
            ffi::GIT_INDEX_STAGE_ANY => Some(Self::Any),
            ffi::GIT_INDEX_STAGE_NORMAL => Some(Self::Normal),
            ffi::GIT_INDEX_STAGE_ANCESTOR => Some(Self::Ancestor),
            ffi::GIT_INDEX_STAGE_OURS => Some(Self::Ours),
            ffi::GIT_INDEX_STAGE_THEIRS => Some(Self::Theirs),
            _ => None,
        }
    }

    /// Value accepted by C's stage-matching APIs.
    pub const fn raw(self) -> ffi::git_index_stage_t {
        self as ffi::git_index_stage_t
    }
}

/// The longest name a tree entry stores whole.
const TREE_NAME_MAX: usize = u16::MAX as usize;

/// `Ok` for a routine that returned `rc` successfully, its failure otherwise; a
/// positive code is a failure too (see [`IndexMut::read`]).
fn status(rc: c_int) -> Result<(), RepositoryError> {
    if rc == 0 {
        Ok(())
    } else {
        Err(RepositoryError::from_libgit2(rc))
    }
}

impl IndexRef<'_> {
    /// Refuses, before libgit2 writes the index's entries as trees of `repo`, an entry it
    /// would mishandle: one whose id it would look up in `repo` though the id is of
    /// another type (libgit2 formats the loose object's path from the id's own size,
    /// overflowing the path buffer, as `id_fits` describes), and one whose path has a
    /// component longer than a tree entry's 65535-byte name (libgit2 truncates the
    /// name's length and reports success for a tree that does not parse; see
    /// `TreeBuilderMut::insert_entry`).
    ///
    /// An index with conflicts is not refused: libgit2 fails it with `GIT_EUNMERGED`
    /// before it reads any entry.
    fn tree_writable(&self, repo: RepositoryRef<'_>) -> Result<(), RepositoryError> {
        let ptr = self.as_ptr().cast_mut();
        // SAFETY: a live index, only read; the count is a vector length.
        let count = unsafe { ffi::git_index_entrycount(ptr) };
        let mut entries = Vec::with_capacity(count);
        for position in 0..count {
            // SAFETY: the handle keeps the index live, used on this thread only; the routine
            // sorts the entry vector and returns a live entry within the count. Nothing
            // removes or replaces entries until this function returns.
            let entry = unsafe { ffi::git_index_get_byindex(ptr, position) };
            // SAFETY: `entry` is within the count, so a live entry of the index.
            if unsafe { ffi::git_index_entry_stage(entry) } != 0 {
                return Ok(());
            }
            entries.push(entry);
        }
        for entry in entries {
            // SAFETY: each entry is live (see above); its fields are projected, read, and
            // its path is the index's NUL-terminated copy, unchanged meanwhile.
            let (mode, path, id) = unsafe {
                (
                    ptr::addr_of!((*entry).mode).read(),
                    CStr::from_ptr(ptr::addr_of!((*entry).path).read()),
                    OidRef::from_ptr(ptr::addr_of!((*entry).id).cast_mut()),
                )
            };
            let id = id.expect("an entry's id is inline");
            if path
                .to_bytes()
                .split(|&b| b == b'/')
                .any(|c| c.len() > TREE_NAME_MAX)
            {
                set(ErrorClass::Invalid, c"tree entry path too long");
                return Err(RepositoryError::from_libgit2(-1));
            }
            if mode != ffi::GIT_FILEMODE_COMMIT && !id.is_zero() {
                id_fits(repo, id)?;
            }
        }
        Ok(())
    }

    /// Refuses, before libgit2 writes an index that a repository with a working
    /// directory owns, one holding an entry with an empty path beside another entry.
    /// libgit2 first diffs the racily clean entries against the working directory,
    /// with their paths as the pathspec (`truncate_racily_clean`), and parsing an empty
    /// pattern beside a non-empty one reads past the empty one's terminator (see
    /// `patterns_are_parsable`). Which entries are racy depends on the timestamp
    /// libgit2 last read the index file with, which it does not publish, so every
    /// entry that can be counts: those neither up to date nor a submodule.
    fn racy_paths_parsable(&self) -> Result<(), RepositoryError> {
        let ptr = self.as_ptr().cast_mut();
        // SAFETY: a live index, whose owner field libgit2 only reads.
        let owner = unsafe { ffi::git_index_owner(ptr) };
        // SAFETY: a non-NULL owner is the live repository keeping the index (libgit2
        // clears the field when it releases the index), used on this thread for this
        // call only.
        let Some(repo) = (unsafe { RepositoryRef::from_ptr(owner) }) else {
            return Ok(());
        };
        if repo.workdir().is_none() {
            return Ok(());
        }
        let (mut empty, mut named) = (false, false);
        // SAFETY: a live index, only read; the count is a vector length.
        let count = unsafe { ffi::git_index_entrycount(ptr) };
        for position in 0..count {
            // SAFETY: the handle keeps the index live, used on this thread only; the
            // routine sorts the entry vector and returns a live entry within the count.
            let raw = unsafe { ffi::git_index_get_byindex(ptr, position) };
            // SAFETY: `raw` is that entry, with its path in its own allocation. Nothing
            // removes, replaces or writes entries until this function returns.
            let Some(entry) = (unsafe { IndexEntryRef::from_ptr(raw) }) else {
                continue;
            };
            let submodule = entry.mode() & 0o170000 == ffi::GIT_FILEMODE_COMMIT;
            let up_to_date = entry
                .flags_extended()
                .contains(EntryExtendedFlags::UP_TO_DATE);
            if submodule || up_to_date {
                continue;
            }
            if entry.path().is_empty() {
                empty = true;
            } else {
                named = true;
            }
        }
        if empty && named {
            set(
                ErrorClass::Index,
                c"cannot write an index holding an entry with an empty path beside others",
            );
            return Err(RepositoryError::from_libgit2(-1));
        }
        Ok(())
    }
}

impl<'a> IndexRef<'a> {
    /// The path of the index file backing the index, as it was opened; `None` for an
    /// in-memory index ([`Index::new`]).
    ///
    /// The path is the index's own copy, which libgit2 sets when the index is built
    /// and frees with it, so it lives as long as this handle's borrow of the owner:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::index::index::Index;
    ///
    /// let index = Index::open(c"/tmp/index").unwrap();
    /// let path = index.as_ref().path();
    /// drop(index);
    /// let _ = path;
    /// ```
    ///
    /// Wraps: git_index_path
    pub fn path(&self) -> Option<&'a CStr> {
        // SAFETY: the handle keeps the index live for `'a`; the routine reads the path
        // field, NULL or a NUL-terminated string the index owns until it is freed.
        let path = unsafe { ffi::git_index_path(self.as_ptr()) };
        // SAFETY: a non-NULL path is that string, never changed or freed before the
        // index, which outlives `'a`.
        (!path.is_null()).then(|| unsafe { CStr::from_ptr(path) })
    }

    /// The version the index is written in: 2, 3 or 4. An index starts at 2 and takes
    /// the version of each index file it reads.
    ///
    /// Wraps: git_index_version
    pub fn version(&self) -> u32 {
        // SAFETY: a live index; the routine only reads its version field, despite its
        // non-const parameter.
        unsafe { ffi::git_index_version(self.as_ptr().cast_mut()) }
    }

    /// Writes the index to its file atomically, through a `.lock` file renamed over
    /// it; the index is clean afterwards.
    ///
    /// Before it writes, an index a repository with a working directory owns has the
    /// size of each entry whose timestamp is as recent as the index file's reset to 0
    /// ("racily clean" entries), so the next status looks at their contents again.
    /// Writing also sorts the entries and resolve-undo records, and marks entries
    /// needing the extended flags; it adds and removes none, so it takes the shared
    /// handle, as a repository's shared index ([`IndexArc`]) needs: the index is used
    /// on this thread only, and nothing an [`IndexRef`] lends is freed.
    ///
    /// Fails, with libgit2's error, for an in-memory index ("the index is in-memory
    /// only", code `-1`), when the lock file exists ([`ErrorCode::Locked`]) and when
    /// the file cannot be written.
    ///
    /// Refused, with a `GIT_ERROR_INDEX` error (code `-1`), before libgit2 sees it: an
    /// index that a repository with a working directory owns, holding an entry with an
    /// empty path (which an index file can hold) beside another entry that is neither
    /// up to date nor a submodule. libgit2 would pass the racily clean entries' paths
    /// to a diff as its pathspec, and read past the empty one (see
    /// [`Pathspec::new`](crate::libgit2::index::pathspec::Pathspec::new)). Which
    /// entries are racily clean depends on the index file's timestamp, which libgit2
    /// keeps private, so such an index is refused even when libgit2 would find none.
    ///
    /// [`ErrorCode::Locked`]: crate::libgit2::util::errors::ErrorCode::Locked
    ///
    /// Wraps: git_index_write
    pub fn write(&self) -> Result<(), RepositoryError> {
        self.racy_paths_parsable()?;
        // SAFETY: a live index used on this thread only: libgit2 counts a reference for
        // the call, writes the file and the index's bookkeeping (stamp, checksum, dirty
        // flag), sorts its vectors and resets entry sizes in place, and frees no entry.
        status(unsafe { ffi::git_index_write(self.as_ptr().cast_mut()) })
    }

    /// Writes the index's entries as trees in the object database of the repository
    /// that owns the index, and returns the root tree's id; as
    /// [`write_tree_to`](Self::write_tree_to) the owner.
    ///
    /// Fails, with a `GIT_ERROR_INDEX` error (code `-1`), for an index no repository
    /// owns: every index built by [`Index::new`] or [`Index::open`] (a repository's
    /// own, which [`RepositoryRef::index`] lends as an [`IndexArc`], has an owner).
    ///
    /// [`RepositoryRef::index`]: crate::libgit2::repository::repository::RepositoryRef::index
    ///
    /// Wraps: git_index_write_tree
    pub fn write_tree(&self) -> Result<Oid, RepositoryError> {
        // SAFETY: a live index; the routine reads the repository the index's count
        // names as its owner, NULL or a live repository (libgit2 clears it when the
        // repository is freed), and keeps nothing.
        let owner = unsafe { ffi::git_index_owner(self.as_ptr()) };
        // SAFETY: a non-NULL owner is live for this call, used on this thread only.
        if let Some(repo) = unsafe { RepositoryRef::from_ptr(owner) } {
            self.tree_writable(repo)?;
        }
        let mut id = Oid::zeroed();
        // SAFETY: `out` addresses `id`; a live index used on this thread only, whose
        // owner (if any) libgit2 writes the trees to, checked above for the ids it
        // looks up and the names it stores. libgit2 sorts the entries, replaces the
        // private tree cache and frees no entry, so the shared handle suffices.
        let rc = unsafe {
            ffi::git_index_write_tree(id.as_mut().as_mut_ptr(), self.as_ptr().cast_mut())
        };
        status(rc).map(|()| id)
    }

    /// Writes the index's entries as trees in `repo`'s object database, and returns
    /// the root tree's id. The index caches the trees it wrote (its tree cache), so a
    /// later write with no change in between returns the same id without writing.
    ///
    /// Fails with [`ErrorCode::Unmerged`] when the index has conflicts, and with
    /// libgit2's error when an entry is not a valid tree entry (an invalid mode or
    /// name, a zero id, an object `repo` does not have, of the type the mode wants)
    /// or a tree cannot be written.
    ///
    /// Refused, with a `GIT_ERROR_INVALID` error (code `-1`), before libgit2 sees it:
    /// an index with an entry whose id libgit2 would look up in `repo` (any mode but a
    /// submodule's, a non-zero id) though it is not of `repo`'s object id type, which
    /// overflows libgit2's path buffer; and one with a path component longer than
    /// 65535 bytes ("tree entry path too long"), which libgit2 would store truncated in
    /// a tree it reports as written. These refusals precede libgit2's own checks of the
    /// other entries.
    ///
    /// [`ErrorCode::Unmerged`]: crate::libgit2::util::errors::ErrorCode::Unmerged
    ///
    /// Wraps: git_index_write_tree_to
    pub fn write_tree_to(&self, repo: RepositoryRef<'_>) -> Result<Oid, RepositoryError> {
        self.tree_writable(repo)?;
        let mut id = Oid::zeroed();
        // SAFETY: `out` addresses `id`; a live index used on this thread only, as in
        // `write_tree`; `repo` a live repository, used on this thread only, whose object
        // database libgit2 writes through this shared handle, as `RepositoryRef`
        // allows, with the ids it looks up and the names it stores checked above.
        let rc = unsafe {
            ffi::git_index_write_tree_to(
                id.as_mut().as_mut_ptr(),
                self.as_ptr().cast_mut(),
                repo.as_ptr().cast_mut(),
            )
        };
        status(rc).map(|()| id)
    }
}

impl IndexMut<'_> {
    /// Rereads the index from its file. With `force`, the index is always replaced by
    /// the file's contents, discarding unwritten changes, and cleared when there is no
    /// file; without it, the file is read only when its checksum differs from the one
    /// the index last read or wrote, and a missing file leaves the index as it is.
    /// The index is clean afterwards.
    ///
    /// Fails, with a `GIT_ERROR_INDEX` error, for an in-memory index ("the index is
    /// in-memory only", code `-1`), when the file cannot be read or is not a valid
    /// index; a failed parse leaves the index cleared.
    ///
    /// libgit2 can return a positive code (`1`) on one failure, when the file it found
    /// is gone before it reads the timestamp; it is reported as [`ErrorCode::Generic`].
    ///
    /// [`ErrorCode::Generic`]: crate::libgit2::util::errors::ErrorCode::Generic
    ///
    /// Wraps: git_index_read
    pub fn read(&mut self, force: bool) -> Result<(), RepositoryError> {
        // SAFETY: the exclusive handle excludes every borrow of the index's entries,
        // which the read may free; the path is the index's own.
        status(unsafe { ffi::git_index_read(self.as_mut_ptr(), c_int::from(force)) })
    }

    /// Sets the version the index is written in: 2, 3 or 4. Writing version 2 may
    /// still produce a version 3 file, when an entry needs the extended flags.
    ///
    /// Fails, with a `GIT_ERROR_INDEX` error ("invalid version number", code `-1`), for
    /// another version, leaving the index's unchanged.
    ///
    /// Wraps: git_index_set_version
    pub fn set_version(&mut self, version: u32) -> Result<(), RepositoryError> {
        // SAFETY: a live index, written through the exclusive handle.
        status(unsafe { ffi::git_index_set_version(self.as_mut_ptr(), version) })
    }

    /// Sets how the index treats the filesystem: case-insensitive paths, untrusted
    /// executable bits, no symbolic links. A change of case sensitivity re-sorts the
    /// entries and resolve-undo records, and changes how paths are looked up.
    ///
    /// [`CapabilitySetting::FromOwner`] reads `core.ignorecase`, `core.filemode` and
    /// `core.symlinks` from the repository owning the index, leaving each capability
    /// whose setting it cannot read as it was. It fails, with a `GIT_ERROR_INDEX` error
    /// (code `-1`), for an index no repository owns, leaving the capabilities
    /// unchanged; an index reached through this exclusive handle has no owner, since
    /// a repository's index is shared ([`IndexArc`]).
    ///
    /// Wraps: git_index_set_caps
    pub fn set_caps(&mut self, caps: CapabilitySetting) -> Result<(), RepositoryError> {
        // SAFETY: a live index, written through the exclusive handle, which excludes
        // any borrow of the entries the re-sort reorders; the value is three named bits
        // or the FROM_OWNER command.
        status(unsafe { ffi::git_index_set_caps(self.as_mut_ptr(), caps.to_raw()) })
    }

    /// Removes the entry at `path` at `stage`, or, for [`Stage::Any`], the first entry
    /// at `path` in index order (its lowest stage, as [`find`](Self::find) finds it),
    /// freeing it.
    ///
    /// Fails with [`ErrorCode::NotFound`] (and a `GIT_ERROR_INDEX` error) when there is
    /// no such entry.
    ///
    /// A deliberate divergence: given [`Stage::Any`], libgit2 removes whichever stage
    /// its binary search meets first, and also drops the path's stage 3 entry, which it
    /// keeps, from its path lookup table (it computes the table's key from the wildcard
    /// with `GIT_INDEX_ENTRY_STAGE_SET`). This passes the stage of the entry
    /// [`find`](Self::find) finds instead.
    ///
    /// [`ErrorCode::NotFound`]: crate::libgit2::util::errors::ErrorCode::NotFound
    ///
    /// Wraps: git_index_remove
    pub fn remove(&mut self, path: &CStr, stage: Stage) -> Result<(), RepositoryError> {
        let stage = match stage {
            // Without an entry at `path`, libgit2 reports the wildcard as not found, and
            // the table holds no entry at `path` to drop.
            Stage::Any => self.first_stage_at(path).unwrap_or(Stage::Any),
            stage => stage,
        };
        // SAFETY: the exclusive handle excludes every borrow of the entry freed; `path`
        // is NUL-terminated, read for the call; the stage is a named one (-1..=3).
        status(unsafe { ffi::git_index_remove(self.as_mut_ptr(), path.as_ptr(), stage.raw()) })
    }

    /// The stage of the first entry at `path`, as [`find`](Self::find) finds it; `None`
    /// when the index has none.
    fn first_stage_at(&mut self, path: &CStr) -> Option<Stage> {
        let position = self.find(path)?;
        // SAFETY: the exclusive handle addresses a live index, whose entry vector
        // libgit2 may sort, which no borrow reads. `position` is within the count, so it
        // returns one of the index's entries, which nothing writes or frees while the
        // handle, used only here, stays borrowed.
        let entry = unsafe {
            IndexEntryRef::from_ptr(ffi::git_index_get_byindex(self.as_mut_ptr(), position))
        }?;
        // SAFETY: `entry` addresses a live entry, whose flags libgit2 only reads.
        Stage::from_raw(unsafe { ffi::git_index_entry_stage(entry.as_ptr()) })
    }

    /// Removes the stage-0 entry at `path`, then resolves a conflict at `path` by
    /// moving its stage 1-3 entries to the resolve-undo records. Neither missing is an
    /// error.
    ///
    /// Wraps: git_index_remove_bypath
    pub fn remove_bypath(&mut self, path: &CStr) -> Result<(), RepositoryError> {
        // SAFETY: the exclusive handle excludes every borrow of the entries freed or
        // moved; `path` is a non-NULL NUL-terminated string, read for the call.
        status(unsafe { ffi::git_index_remove_bypath(self.as_mut_ptr(), path.as_ptr()) })
    }

    /// Removes every entry at `stage` under the directory `dir` (`dir` and a `/`,
    /// which is added when missing; the empty `dir` is the whole index).
    ///
    /// [`Stage::Any`] is refused, with a `GIT_ERROR_INVALID` error (code `-1`), before
    /// libgit2 sees it: libgit2 compares each entry's stage with `-1` literally, so
    /// it would remove nothing and report success.
    ///
    /// Wraps: git_index_remove_directory
    pub fn remove_directory(&mut self, dir: &CStr, stage: Stage) -> Result<(), RepositoryError> {
        if stage == Stage::Any {
            set(
                ErrorClass::Invalid,
                c"cannot remove a directory at any stage: give a concrete stage",
            );
            return Err(RepositoryError::from_libgit2(-1));
        }
        // SAFETY: the exclusive handle excludes every borrow of the entries freed; `dir`
        // is NUL-terminated, copied for the call; the stage is a concrete one (0..=3).
        status(unsafe {
            ffi::git_index_remove_directory(self.as_mut_ptr(), dir.as_ptr(), stage.raw())
        })
    }

    /// Removes the entries matching `pathspec` (all of them for an empty one), at any
    /// stage, each through [`remove_bypath`](Self::remove_bypath): a conflicted path's
    /// stages move to the resolve-undo records.
    ///
    /// `callback`, if any, sees each matching path and the pattern it matched before
    /// the entry goes ([`MatchedPathCallback`] describes its results); a negative
    /// result stops the scan, and is returned as the error's code
    /// ([`raw_code`](RepositoryError::raw_code)), with libgit2's generic message when
    /// the callback set no error. A panic stops the scan as `GIT_EUSER` and resumes
    /// once libgit2 returned.
    ///
    /// The callback cannot reach the index while it runs: libgit2 lends it the
    /// entry's own path, freed at once by a change of the index, and this handle is
    /// borrowed exclusively for the scan.
    ///
    /// ```compile_fail,E0499
    /// use std::ffi::CStr;
    /// use libgit2::libgit2::core::strarray::StrArrayView;
    /// use libgit2::libgit2::index::index::{Index, MatchedPathCallback, MatchedPathResult};
    ///
    /// let mut index = Index::new().unwrap();
    /// let mut handle = index.as_mut();
    /// let mut f = |path: &CStr, _: Option<&CStr>| {
    ///     let _ = handle.remove_bypath(path);
    ///     MatchedPathResult::SKIP
    /// };
    /// let mut callback = MatchedPathCallback::new(&mut f);
    /// let _ = index.as_mut().remove_all(StrArrayView::default().as_ref(), Some(&mut callback));
    /// ```
    ///
    /// A pathspec mixing empty and non-empty patterns is refused, with a
    /// `GIT_ERROR_INVALID` error (code `-1`), before libgit2 sees it, as
    /// [`Pathspec::new`](crate::libgit2::index::pathspec::Pathspec::new) refuses it.
    ///
    /// Wraps: git_index_remove_all
    pub fn remove_all(
        &mut self,
        pathspec: StrArrayRef<'_>,
        callback: Option<&mut MatchedPathCallback<'_>>,
    ) -> Result<(), RepositoryError> {
        if !patterns_are_parsable(pathspec) {
            return Err(RepositoryError::from_libgit2(-1));
        }
        let index = self.as_mut_ptr();
        let rc = match callback {
            Some(callback) => callback.scan_with(|cb, payload| {
                // SAFETY: the exclusive handle keeps the index live and unreachable from
                // the callback, so the entry path lent to it stays put; `pathspec` is a
                // live parsable array, read for the call; the trampoline and payload
                // come from `scan_with`, used synchronously on this thread.
                unsafe { ffi::git_index_remove_all(index, pathspec.as_ptr(), cb, payload) }
            }),
            // SAFETY: as above, without a callback.
            None => unsafe {
                ffi::git_index_remove_all(index, pathspec.as_ptr(), None, ptr::null_mut())
            },
        };
        status(rc)
    }

    /// Updates the entries matching `pathspec` (all of them for an empty one) from the
    /// working directory of the repository owning the index: a changed file is added
    /// again (its blob written), a deleted one removed as by
    /// [`remove_bypath`](Self::remove_bypath).
    ///
    /// `callback` works as for [`remove_all`](Self::remove_all); libgit2 lends it the
    /// path of the working-directory change, which the scan's diff owns.
    ///
    /// Fails, with a `GIT_ERROR_INDEX` error (code `-1`), for an index no repository
    /// owns, before the callback runs. An index reached through this exclusive handle
    /// has no owner, since a repository's index is shared ([`IndexArc`]).
    ///
    /// A pathspec mixing empty and non-empty patterns is refused as
    /// [`remove_all`](Self::remove_all) refuses it.
    ///
    /// Wraps: git_index_update_all
    pub fn update_all(
        &mut self,
        pathspec: StrArrayRef<'_>,
        callback: Option<&mut MatchedPathCallback<'_>>,
    ) -> Result<(), RepositoryError> {
        if !patterns_are_parsable(pathspec) {
            return Err(RepositoryError::from_libgit2(-1));
        }
        let index = self.as_mut_ptr();
        let rc = match callback {
            Some(callback) => callback.scan_with(|cb, payload| {
                // SAFETY: the exclusive handle keeps the index live and unreachable from
                // the callback; `pathspec` is a live parsable array, read for the call;
                // the trampoline and payload come from `scan_with`, used synchronously
                // on this thread.
                unsafe { ffi::git_index_update_all(index, pathspec.as_ptr(), cb, payload) }
            }),
            // SAFETY: as above, without a callback.
            None => unsafe {
                ffi::git_index_update_all(index, pathspec.as_ptr(), None, ptr::null_mut())
            },
        };
        status(rc)
    }
}

/// The combinable flags of an index entry. Name-length and stage bits, and any
/// future unnamed bits, survive a round trip through this value.
///
/// This is the C enum's integer layout, not the narrower `git_index_entry.flags`
/// field. It owns no resources and requires no foreign allocation or teardown.
///
/// Wraps: git_index_entry_flag_t
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EntryFlags(ffi::git_index_entry_flag_t);

impl EntryFlags {
    /// The entry carries extended flags on disk.
    pub const EXTENDED: Self = Self(ffi::GIT_INDEX_ENTRY_EXTENDED);
    /// Assume the entry is valid (Git's assume-unchanged bit).
    pub const VALID: Self = Self(ffi::GIT_INDEX_ENTRY_VALID);

    /// No flags set.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Preserves all bits, including those not named by this enum.
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// The underlying bit pattern.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every bit in `flags` is present.
    pub const fn contains(self, flags: Self) -> bool {
        self.0 & flags.0 == flags.0
    }

    /// Adds the selected bits without changing any others.
    pub fn insert(&mut self, flags: Self) {
        self.0 |= flags.0;
    }

    /// Removes the selected bits without changing any others.
    pub fn remove(&mut self, flags: Self) {
        self.0 &= !flags.0;
    }
}

impl core::ops::BitOr for EntryFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for EntryFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

define_ctype!(
    /// An opaque, C-allocated snapshot iterator over an index's entries.
    ///
    /// The snapshot retains a counted index reference and its own vector of
    /// entry pointers. C defers freeing removed entries until readers finish.
    /// There are no public fields, clone operations, or in-place initializers.
    /// Neither this type nor its handles/owner is `Send` or `Sync`: the retained
    /// index can still be accessed by other C owners without a common lock.
    ///
    /// Wraps: git_index_iterator
    IndexIterator, IndexIteratorRef, IndexIteratorMut, ffi::git_index_iterator
);

/// Releases the snapshot vector, its counted index reference, and the iterator.
///
/// Wraps: git_index_iterator_free
#[derive(Clone, Copy, Debug, Default)]
pub struct IndexIteratorFree;

impl_cdrop!(
    IndexIteratorFree,
    IndexIterator,
    ffi::git_index_iterator_free
);

/// The sole owner of an [`IndexIterator`]. Its shared/exclusive handles borrow
/// it; neither can outlive it. The retained index may be dropped first.
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::index::index::{Index, IndexArc, IndexIterator};
/// let index: IndexArc = Index::new().unwrap().into();
/// let iterator = IndexIterator::new(&index).unwrap();
/// let handle = iterator.as_ref();
/// drop(iterator);
/// let _ = handle;
/// ```
///
/// ```compile_fail,E0502
/// use libgit2::libgit2::index::index::{Index, IndexArc, IndexIterator};
/// let index: IndexArc = Index::new().unwrap().into();
/// let mut iterator = IndexIterator::new(&index).unwrap();
/// let mut handle = iterator.as_mut();
/// let shared = handle.as_ref();
/// let exclusive = handle.as_mut();
/// let _ = (shared, exclusive);
/// ```
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::index::index::{Index, IndexArc, IndexIterator};
/// let index: IndexArc = Index::new().unwrap().into();
/// let iterator = IndexIterator::new(&index).unwrap();
/// std::thread::spawn(move || drop(iterator));
/// ```
pub type IndexIteratorBox = CBox<IndexIterator, IndexIteratorFree>;

impl IndexIterator {
    /// Snapshots the index in path order, retaining its own counted reference.
    /// The returned owner is independent of `index`'s Rust lifetime.
    ///
    /// Requires a shared owner because C keeps an additional reference until
    /// iterator destruction. Convert an [`IndexBox`] with `.into()` first;
    /// accepting its borrowed handle would leave a supposedly sole owner while
    /// the iterator still retains the index. Snapshot creation can sort the
    /// private entry vector, but does not change the entries themselves.
    ///
    /// A sole-owner borrow cannot be retained:
    ///
    /// ```compile_fail,E0308
    /// use libgit2::libgit2::index::index::{Index, IndexIterator};
    /// let index = Index::new().unwrap();
    /// let _ = IndexIterator::new(&index);
    /// ```
    ///
    /// Wraps: git_index_iterator_new
    pub fn new(index: &IndexArc) -> Result<IndexIteratorBox, ErrorBuf> {
        let mut out = ptr::null_mut();
        // SAFETY: the arc keeps a live index and the runtime alive; it has no
        // exclusive handle. C borrows the output slot for this call, retaining
        // its own counted index reference only on success. Neither owner is
        // Send/Sync, and no Rust reference covers C's index or entry vector.
        let rc = unsafe { ffi::git_index_iterator_new(&mut out, index.as_c_ptr()) };
        if rc < 0 {
            return Err(last());
        }
        // SAFETY: success writes a fresh, initialized, uniquely owned iterator
        // paired with git_index_iterator_free; its retained index needs no Rust
        // keepalive. Failure leaves out untouched and was handled above.
        Ok(unsafe { IndexIteratorBox::from_c(out) }.expect("successful iterator constructor"))
    }
}

/// Opaque conflict cursor, borrowing one exclusively held index for `'index`.
///
/// C neither retains the index nor snapshots its entries. The exclusive borrow
/// prevents every other safe index operation until the cursor is dropped. No
/// fields are public in C, and there is no clone or reference-count operation.
/// Unlike the ordinary index iterator, this cursor does not retain an index ref.
///
/// The index cannot be dropped first:
/// ```compile_fail,E0505
/// use libgit2::libgit2::index::index::{Index, ConflictIterator};
/// let mut index = Index::new().unwrap();
/// let cursor = ConflictIterator::new(index.as_mut()).unwrap();
/// drop(index);
/// drop(cursor);
/// ```
/// Nor borrowed again while iteration is live:
/// ```compile_fail,E0499
/// use libgit2::libgit2::index::index::{Index, ConflictIterator};
/// let mut index = Index::new().unwrap();
/// let cursor = ConflictIterator::new(index.as_mut()).unwrap();
/// let second = index.as_mut();
/// drop((cursor, second));
/// ```
/// A shared index cannot guarantee that another C holder leaves entries alone:
/// ```compile_fail
/// use libgit2::libgit2::index::index::{Index, IndexArc, ConflictIterator};
/// let index: IndexArc = Index::new().unwrap().into();
/// let cursor = ConflictIterator::new(index.as_ref());
/// ```
///
/// Wraps: git_index_conflict_iterator
/// Handwritten
#[repr(transparent)]
pub struct ConflictIterator<'index>(
    ffi::git_index_conflict_iterator,
    core::marker::PhantomData<&'index mut Index>,
);

/// Copyable shared handle to a conflict cursor; it cannot advance the cursor.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ConflictIteratorRef<'a, 'index>(ffibox::CBorrowedPtr<'a, ConflictIterator<'index>>);

/// Exclusive cursor handle. Reborrowing freezes the original handle.
///
/// ```compile_fail,E0502
/// use libgit2::libgit2::index::index::{Index, ConflictIterator};
/// let mut index = Index::new().unwrap();
/// let mut cursor = ConflictIterator::new(index.as_mut()).unwrap();
/// let mut handle = cursor.as_mut();
/// let shared = handle.as_ref();
/// let exclusive = handle.as_mut();
/// drop((shared, exclusive));
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::index::index::ConflictIteratorMut;
/// fn shorten<'a, 'i>(m: ConflictIteratorMut<'a, 'static>) -> ConflictIteratorMut<'a, 'i> { m }
/// ```
///
/// Handwritten
#[repr(transparent)]
pub struct ConflictIteratorMut<'a, 'index>(
    ConflictIteratorRef<'a, 'index>,
    core::marker::PhantomData<&'a mut ConflictIterator<'index>>,
);

// SAFETY: all three types are transparent over their C layout or CBorrowedPtr
// respectively, with zero-sized lifetime markers and no handle Drop. The mutable
// handle is invariant in the pointee, including its index lifetime. No reference
// to foreign storage is formed. Index's marker withholds Send and Sync.
unsafe impl<'index> ffibox::CCell for ConflictIterator<'index> {
    type C = ffi::git_index_conflict_iterator;
    type Ref<'a>
        = ConflictIteratorRef<'a, 'index>
    where
        Self: 'a;
    type Mut<'a>
        = ConflictIteratorMut<'a, 'index>
    where
        Self: 'a;
}

impl ConflictIteratorRef<'_, '_> {
    #[allow(dead_code)] // Used by the following symbol wave.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_index_conflict_iterator {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'index> ConflictIteratorMut<'_, 'index> {
    #[allow(dead_code)] // Used by the following symbol wave.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_index_conflict_iterator {
        self.0.0.as_non_null().as_ptr().cast()
    }

    pub fn as_ref(&self) -> ConflictIteratorRef<'_, 'index> {
        self.0
    }

    pub fn as_mut(&mut self) -> ConflictIteratorMut<'_, 'index> {
        ConflictIteratorMut(self.0, core::marker::PhantomData)
    }
}

/// Frees the cursor allocation only; the borrowed index is not released.
///
/// Wraps: git_index_conflict_iterator_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct ConflictIteratorFree;

// SAFETY: each live initialized cursor from git_index_conflict_iterator_new is
// released once by git_index_conflict_iterator_free; there are no clone policies.
unsafe impl<'index> ffibox::CDrop<ConflictIterator<'index>> for ConflictIteratorFree {
    unsafe fn c_drop(&self, cursor: NonNull<ConflictIterator<'index>>) {
        // SAFETY: the caller transfers the sole C allocation; the wrapper is
        // layout-compatible. C frees only the cursor and retains nothing.
        unsafe { ffi::git_index_conflict_iterator_free(cursor.as_ptr().cast()) }
    }
}

/// Sole C-allocated owner, with the index kept exclusively borrowed until drop.
/// Neither owners nor handles can cross threads.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::index::index::ConflictIteratorBox;
/// fn send<T: Send>() {}
/// send::<ConflictIteratorBox<'static>>();
/// ```
/// ```compile_fail,E0277
/// use libgit2::libgit2::index::index::ConflictIteratorBox;
/// fn sync<T: Sync>() {}
/// sync::<ConflictIteratorBox<'static>>();
/// ```
/// ```compile_fail,E0505
/// use libgit2::libgit2::index::index::{Index, ConflictIterator};
/// let mut index = Index::new().unwrap();
/// let cursor = ConflictIterator::new(index.as_mut()).unwrap();
/// let handle = cursor.as_ref();
/// drop(cursor);
/// let _copy = handle;
/// ```
pub type ConflictIteratorBox<'index> = CBox<ConflictIterator<'index>, ConflictIteratorFree>;

impl<'index> ConflictIterator<'index> {
    /// Creates a cursor positioned before the first conflict. The index must
    /// have a sole owner: an `IndexArc` cannot provide this exclusive handle.
    /// Returns the C allocation error without retaining a borrow on failure.
    ///
    /// Wraps: git_index_conflict_iterator_new
    pub fn new(mut index: IndexMut<'index>) -> Result<ConflictIteratorBox<'index>, ErrorBuf> {
        let mut out = ptr::null_mut();
        // SAFETY: index is live and exclusively borrowed for the returned
        // cursor's lifetime. C stores its pointer without retaining a ref; out
        // is a writable slot and C writes a fresh initialized cursor on success.
        let rc = unsafe { ffi::git_index_conflict_iterator_new(&mut out, index.as_mut_ptr()) };
        if rc < 0 {
            return Err(last());
        }
        // SAFETY: success transfers one initialized allocation paired with the
        // policy's destructor. The index borrow is carried in the layout type.
        unsafe { ConflictIteratorBox::from_c(out) }.ok_or_else(last)
    }
}

/// Extended entry bitmasks, including the on-disk mask and the in-memory flag.
/// Unknown bits are retained, not interpreted as enum discriminants. This is a
/// scalar value, not a C allocation; copying it copies no pointers or resources.
/// The entry field itself is `u16`; the enum's ABI representation is preserved.
///
/// Wraps: git_index_entry_extended_flag_t
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EntryExtendedFlags(ffi::git_index_entry_extended_flag_t);

impl EntryExtendedFlags {
    pub const EMPTY: Self = Self(0);
    pub const INTENT_TO_ADD: Self = Self(ffi::GIT_INDEX_ENTRY_INTENT_TO_ADD);
    pub const SKIP_WORKTREE: Self = Self(ffi::GIT_INDEX_ENTRY_SKIP_WORKTREE);
    /// The union of flags serialized to disk (not a third independent bit).
    pub const ON_DISK: Self = Self(ffi::GIT_INDEX_ENTRY_EXTENDED_FLAGS);
    pub const UP_TO_DATE: Self = Self(ffi::GIT_INDEX_ENTRY_UPTODATE);

    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every bit of `flags` is present, including unknown bits.
    pub const fn contains(self, flags: Self) -> bool {
        self.0 & flags.0 == flags.0
    }

    pub fn insert(&mut self, flags: Self) {
        self.0 |= flags.0;
    }

    pub fn remove(&mut self, flags: Self) {
        self.0 &= !flags.0;
    }
}

impl core::ops::BitOr for EntryExtendedFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitAnd for EntryExtendedFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

define_ctype!(
    /// The index's truncated timestamp, not an authoritative wall-clock time.
    ///
    /// All bit patterns are preserved, including nanoseconds outside 0..1_000_000_000.
    /// C compares and serializes these integers without normalization. This value
    /// owns no resources and needs no C constructor or destructor.
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::index::index::IndexTime;
    /// let mut time = IndexTime::new(1, 2);
    /// let shared = time.as_ref();
    /// time.as_mut().set_seconds(3);
    /// let _ = shared.seconds();
    /// ```
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::index::index::IndexTime;
    /// let time = IndexTime::new(1, 2);
    /// let shared = time.as_ref();
    /// drop(time);
    /// let _ = shared.seconds();
    /// ```
    ///
    /// Wraps: git_index_time
    IndexTime, IndexTimeRef, IndexTimeMut, ffi::git_index_time
);

// SAFETY: both fields are integers accepting all bit patterns; zero has no resources
// or additional invariants and is accepted by all timestamp accessors and C consumers.
unsafe impl CZeroable for IndexTime {}

impl IndexTime {
    /// Constructs an index timestamp without normalization or truncation.
    pub fn new(seconds: i32, nanoseconds: u32) -> Self {
        let mut value = Self::zeroed();
        value.as_mut().set_seconds(seconds);
        value.as_mut().set_nanoseconds(nanoseconds);
        value
    }
}

impl Default for IndexTime {
    fn default() -> Self {
        Self::zeroed()
    }
}

impl Clone for IndexTime {
    fn clone(&self) -> Self {
        self.as_ref().to_time()
    }
}

impl IndexTimeRef<'_> {
    /// The signed, truncated seconds component.
    ///
    /// Field: git_index_time.seconds
    pub fn seconds(&self) -> i32 {
        // SAFETY: the shared handle keeps this initialized scalar field readable.
        unsafe { ptr::addr_of!((*self.as_ptr()).seconds).read() }
    }

    /// The raw nanoseconds component, without normalization.
    ///
    /// Field: git_index_time.nanoseconds
    pub fn nanoseconds(&self) -> u32 {
        // SAFETY: the shared handle keeps this initialized scalar field readable.
        unsafe { ptr::addr_of!((*self.as_ptr()).nanoseconds).read() }
    }

    /// Copies both integers into independent Rust-owned storage.
    pub fn to_time(&self) -> IndexTime {
        IndexTime::new(self.seconds(), self.nanoseconds())
    }
}

impl IndexTimeMut<'_> {
    /// Replaces the seconds component.
    pub fn set_seconds(&mut self, seconds: i32) {
        // SAFETY: this exclusive handle grants the only access to the initialized
        // scalar field for the write; every i32 is valid.
        unsafe { ptr::addr_of_mut!((*self.as_mut_ptr()).seconds).write(seconds) }
    }

    /// Replaces the nanoseconds component; all u32 values are preserved.
    pub fn set_nanoseconds(&mut self, nanoseconds: u32) {
        // SAFETY: this exclusive handle grants the only access to the initialized
        // scalar field for the write; every u32 is valid.
        unsafe { ptr::addr_of_mut!((*self.as_mut_ptr()).nanoseconds).write(nanoseconds) }
    }
}

/// A file entry of an index (`git_index_entry`): the path, object id and mode of one
/// staged file, its stage and flags, and the truncated `stat` data that tells libgit2
/// whether the working-directory file changed.
///
/// Build one to hand to the index routines that take an entry, which copy it, path and
/// all, and keep nothing:
///
/// ```
/// use libgit2::libgit2::core::types::FileMode;
/// use libgit2::libgit2::index::index::{EntryExtendedFlags, IndexEntry, IndexTime};
/// use libgit2::libgit2::oid::oid::Oid;
/// use ffibox::CZeroable;
///
/// let mut entry = IndexEntry::new(c"src/lib.rs", Oid::zeroed().as_ref(), FileMode::Blob);
/// assert_eq!(entry.as_ref().path(), c"src/lib.rs");
/// assert_eq!(entry.as_ref().filemode(), Some(FileMode::Blob));
/// let mut handle = entry.as_mut();
/// handle.set_file_size(42);
/// handle.set_mtime(IndexTime::new(1_700_000_000, 5).as_ref());
/// handle.set_flags_extended(EntryExtendedFlags::INTENT_TO_ADD).unwrap();
/// assert_eq!(entry.as_ref().mtime().seconds(), 1_700_000_000);
/// ```
///
/// `#[repr(transparent)]` over `git_index_entry`, so a pointer to it is a pointer to the
/// C struct, which C reads in place. `'p` is the lifetime of the path it points at, so
/// an entry cannot outlive it:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::core::types::FileMode;
/// use libgit2::libgit2::index::index::IndexEntry;
/// use libgit2::libgit2::oid::oid::Oid;
/// use ffibox::CZeroable;
/// use std::ffi::CString;
///
/// let entry;
/// {
///     let path = CString::new("file.txt").unwrap();
///     entry = IndexEntry::new(&path, Oid::zeroed().as_ref(), FileMode::Blob);
/// }
/// let _ = entry.as_ref().path();
/// ```
///
/// The exclusive handle is invariant in `'p`, so one cannot be shrunk to store a
/// shorter-lived path than its entry was built for:
///
/// ```compile_fail
/// use libgit2::libgit2::index::index::IndexEntryMut;
///
/// fn shrink<'a, 'p>(m: IndexEntryMut<'a, 'static>) -> IndexEntryMut<'a, 'p> {
///     m
/// }
/// ```
///
/// The shared handle only reads:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::core::types::FileMode;
/// use libgit2::libgit2::index::index::IndexEntry;
/// use libgit2::libgit2::oid::oid::Oid;
/// use ffibox::CZeroable;
///
/// let entry = IndexEntry::new(c"a", Oid::zeroed().as_ref(), FileMode::Blob);
/// entry.as_ref().set_file_size(1);
/// ```
///
/// libgit2 publishes no routine that allocates, copies or frees an entry for its
/// caller. The entries an index holds are its own (each allocated with its path inline,
/// and freed with the index or when removed), and the index routines only lend them as
/// `const git_index_entry *`; those are reached as an [`IndexEntryRef`] borrowing the
/// index, never as an owner. So an entry built here is a plain value, with no teardown.
/// Every field holds a value its getter accepts: integers, an id's bytes, and a path
/// that is never NULL.
///
/// The value is `Copy`, `Send` and `Sync`, as integers and a `&'p CStr` are: nothing
/// writes through the path, and only the exclusive handle writes the fields. Its
/// handles are neither, because the entries an index lends are rewritten in place by C
/// (when `git_index_add` replaces one, and when writing the index clears every entry's
/// [`UP_TO_DATE`](EntryExtendedFlags::UP_TO_DATE) flag), and an index is not shared
/// across threads:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::index::index::IndexEntryRef;
///
/// fn send<T: Send>() {}
/// send::<IndexEntryRef<'static, 'static>>();
/// ```
///
/// Equal when every field is, the id by its type and bytes and the path by its contents.
///
/// Wraps: git_index_entry
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct IndexEntry<'p>(ffi::git_index_entry, PhantomData<&'p CStr>);

/// Shared borrow of an [`IndexEntry`]: one built in Rust, or one an index lends. `Copy`,
/// like `&T`; the getters live here.
///
/// Neither the handle nor the fields it projects in place outlive the entry:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::core::types::FileMode;
/// use libgit2::libgit2::index::index::IndexEntry;
/// use libgit2::libgit2::oid::oid::Oid;
/// use ffibox::CZeroable;
///
/// let id;
/// {
///     let entry = IndexEntry::new(c"a", Oid::zeroed().as_ref(), FileMode::Blob);
///     id = entry.as_ref().id();
/// }
/// let _ = id.raw_kind();
/// ```
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct IndexEntryRef<'a, 'p>(CBorrowedPtr<'a, IndexEntry<'p>>);

/// Exclusive borrow of an [`IndexEntry`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// A shared reborrow freezes it until the reborrow's last use:
///
/// ```compile_fail,E0502
/// use libgit2::libgit2::core::types::FileMode;
/// use libgit2::libgit2::index::index::IndexEntry;
/// use libgit2::libgit2::oid::oid::Oid;
/// use ffibox::CZeroable;
///
/// let mut entry = IndexEntry::new(c"a", Oid::zeroed().as_ref(), FileMode::Blob);
/// let mut handle = entry.as_mut();
/// let ctime = handle.as_ref().ctime();
/// handle.ctime_mut().set_seconds(1);
/// let _ = ctime.seconds();
/// ```
///
/// Handwritten
#[repr(transparent)]
pub struct IndexEntryMut<'a, 'p>(
    IndexEntryRef<'a, 'p>,
    // Invariant in the entry, and so in `'p`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut IndexEntry<'p>>,
);

// SAFETY: `IndexEntry` is `#[repr(transparent)]` over `git_index_entry` with a zero-sized
// marker; both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, IndexEntry<'p>>`
// (`IndexEntryMut` through `IndexEntryRef`) with zero-sized markers and no `Drop`, hand
// out no reference to the entry, and `IndexEntryRef` has no write operation.
// `IndexEntryMut` is invariant in the entry through its
// `PhantomData<&'a mut IndexEntry<'p>>`.
unsafe impl<'p> CCell for IndexEntry<'p> {
    type C = ffi::git_index_entry;
    type Ref<'a>
        = IndexEntryRef<'a, 'p>
    where
        Self: 'a;
    type Mut<'a>
        = IndexEntryMut<'a, 'p>
    where
        Self: 'a;
}

// SAFETY: an entry value is integers, an id's bytes and a string pointer, a `&'p CStr`'s
// (per the handles' invariant): a shared, immutable borrow of `CStr`, which is `Sync`, so
// it may be read from any thread. Nothing in the struct has thread affinity or interior
// mutability, and nothing is released on drop. The handles, which may borrow an entry
// an index owns, implement neither trait.
unsafe impl Send for IndexEntry<'_> {}
// SAFETY: as above, and shared access to a value only reads its fields (every write goes
// through `IndexEntryMut`, which excludes other handles), so concurrent readers race
// nothing.
unsafe impl Sync for IndexEntry<'_> {}

impl<'p> IndexEntry<'p> {
    /// An entry for the file at `path` (relative to the working directory, components
    /// separated by `/`), naming the object `id` with mode `filemode`, at stage 0, with
    /// every other field zero.
    ///
    /// Nothing is checked here: the routine the entry is handed to checks it when it
    /// runs, refusing for instance an invalid path or mode.
    pub fn new(path: &'p CStr, id: OidRef<'_>, filemode: FileMode) -> Self {
        Self(
            ffi::git_index_entry {
                mode: filemode.raw(),
                id: copy_id(id),
                path: path.as_ptr(),
                ..Default::default()
            },
            PhantomData,
        )
    }

    /// The raw `git_index_entry`, for C code that takes the struct; its path is borrowed
    /// for `'p`, and nothing is to be freed.
    pub fn into_raw(self) -> ffi::git_index_entry {
        self.0
    }

    /// Shared handle to the entry — the getters, and the pointer for C calls that read
    /// it.
    pub fn as_ref(&self) -> IndexEntryRef<'_, 'p> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        IndexEntryRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the entry — the setters.
    pub fn as_mut(&mut self) -> IndexEntryMut<'_, 'p> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        IndexEntryMut(IndexEntryRef(ptr), PhantomData)
    }
}

/// The `git_oid` `id` holds, copied field by field (`OidRef`'s comparisons go through C).
fn copy_id(id: OidRef<'_>) -> ffi::git_oid {
    ffi::git_oid {
        type_: id.raw_kind(),
        id: id.id(),
    }
}

impl PartialEq for IndexEntry<'_> {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.as_ref(), other.as_ref());
        let time = |t: IndexTimeRef<'_>| (t.seconds(), t.nanoseconds());
        let (x, y) = (a.id(), b.id());
        time(a.ctime()) == time(b.ctime())
            && time(a.mtime()) == time(b.mtime())
            && (a.dev(), a.ino(), a.mode(), a.uid(), a.gid(), a.file_size())
                == (b.dev(), b.ino(), b.mode(), b.uid(), b.gid(), b.file_size())
            && (x.raw_kind(), x.id()) == (y.raw_kind(), y.id())
            && (a.flags(), a.flags_extended()) == (b.flags(), b.flags_extended())
            && a.path() == b.path()
    }
}

impl Eq for IndexEntry<'_> {}

impl fmt::Debug for IndexEntry<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'p> IndexEntryRef<'a, 'p> {
    /// Borrows the entry at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live, initialised entry that stays live, and that
    /// nothing writes, for `'a`, whose `path` is non-NULL and addresses a NUL-terminated
    /// string that stays live and unchanged for `'p`. For an entry an index lends, both
    /// end before the index is next changed or released: the path lives in the entry's
    /// own allocation.
    pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_index_entry) -> Option<Self> {
        let ptr = NonNull::new(ptr.cast_mut().cast::<IndexEntry<'p>>())?;
        // SAFETY: layout-preserving cast per `#[repr(transparent)]`; the caller upholds
        // liveness, the lifetimes and the absence of writers.
        Some(Self(unsafe { CBorrowedPtr::new(ptr) }))
    }

    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_index_entry *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_index_entry {
        self.0.as_non_null().as_ptr().cast()
    }

    /// When the file's metadata last changed, truncated to 32-bit seconds; in place, for
    /// as long as the entry is borrowed.
    ///
    /// Field: git_index_entry.ctime
    pub fn ctime(&self) -> IndexTimeRef<'a> {
        let p = self.as_ptr();
        // SAFETY: the field projected from the shared handle's pointer to a live entry is
        // an initialised `git_index_time`, which lives for `'a` with the entry and which
        // nothing writes meanwhile (per the handle's contract); `IndexTimeRef` only
        // reads it.
        unsafe { IndexTimeRef::from_ptr(addr_of!((*p).ctime).cast_mut()) }
            .expect("a field is never NULL")
    }

    /// When the file's contents last changed, truncated to 32-bit seconds; in place, for
    /// as long as the entry is borrowed.
    ///
    /// Field: git_index_entry.mtime
    pub fn mtime(&self) -> IndexTimeRef<'a> {
        let p = self.as_ptr();
        // SAFETY: as `ctime`, for `mtime`.
        unsafe { IndexTimeRef::from_ptr(addr_of!((*p).mtime).cast_mut()) }
            .expect("a field is never NULL")
    }

    /// The device number as stored, truncated to 32 bits. git writes the device
    /// holding the file (`st_dev`), but libgit2 stores the file's `st_rdev` when it
    /// stats it (adding by path, updating), which is 0 for a regular file.
    ///
    /// Field: git_index_entry.dev
    pub fn dev(&self) -> u32 {
        // SAFETY: a read through the shared handle's pointer to a live entry; the field
        // is a plain integer.
        unsafe { addr_of!((*self.as_ptr()).dev).read() }
    }

    /// The file's inode number, truncated to 32 bits.
    ///
    /// Field: git_index_entry.ino
    pub fn ino(&self) -> u32 {
        // SAFETY: as `dev`, for `ino`.
        unsafe { addr_of!((*self.as_ptr()).ino).read() }
    }

    /// The entry's mode as stored: a [`FileMode`] value for every entry libgit2 checks
    /// on the way in (`git_index_add`, `git_index_conflict_add`) or stats in the working
    /// directory, but whatever an index file it read spelled, and a tree's raw mode
    /// for the entries `git_index_read_tree` adds (git's legacy `0100664`, say).
    ///
    /// Field: git_index_entry.mode
    pub fn mode(&self) -> u32 {
        // SAFETY: as `dev`, for `mode`.
        unsafe { addr_of!((*self.as_ptr()).mode).read() }
    }

    /// The entry's mode as a [`FileMode`]; `None` for a mode without a variant, which
    /// [`mode`](Self::mode) has.
    ///
    /// Field: git_index_entry.mode
    pub fn filemode(&self) -> Option<FileMode> {
        FileMode::from_raw(self.mode())
    }

    /// The file owner's user id, truncated to 32 bits.
    ///
    /// Field: git_index_entry.uid
    pub fn uid(&self) -> u32 {
        // SAFETY: as `dev`, for `uid`.
        unsafe { addr_of!((*self.as_ptr()).uid).read() }
    }

    /// The file owner's group id, truncated to 32 bits.
    ///
    /// Field: git_index_entry.gid
    pub fn gid(&self) -> u32 {
        // SAFETY: as `dev`, for `gid`.
        unsafe { addr_of!((*self.as_ptr()).gid).read() }
    }

    /// The file's size in bytes, truncated to 32 bits.
    ///
    /// Field: git_index_entry.file_size
    pub fn file_size(&self) -> u32 {
        // SAFETY: as `dev`, for `file_size`.
        unsafe { addr_of!((*self.as_ptr()).file_size).read() }
    }

    /// The id of the blob (or a submodule's commit) the entry names, in place, for as
    /// long as the entry is borrowed.
    ///
    /// Field: git_index_entry.id
    pub fn id(&self) -> OidRef<'a> {
        let p = self.as_ptr();
        // SAFETY: the field projected from the shared handle's pointer to a live entry is
        // an initialised `git_oid`, which lives for `'a` with the entry and which nothing
        // writes meanwhile (per the handle's contract); `OidRef` only reads it.
        unsafe { OidRef::from_ptr(addr_of!((*p).id).cast_mut()) }.expect("a field is never NULL")
    }

    /// The entry's 16 flag bits, widened: the path length (`GIT_INDEX_ENTRY_NAMEMASK`),
    /// the stage (`GIT_INDEX_ENTRY_STAGEMASK`), and [`EntryFlags::EXTENDED`] and
    /// [`EntryFlags::VALID`].
    ///
    /// Field: git_index_entry.flags
    pub fn flags(&self) -> EntryFlags {
        // SAFETY: as `dev`, for `flags`.
        let raw = unsafe { addr_of!((*self.as_ptr()).flags).read() };
        EntryFlags::from_bits_retain(raw.into())
    }

    /// The entry's 16 extended flag bits, widened: those written to disk
    /// ([`EntryExtendedFlags::ON_DISK`]) and those libgit2 only keeps in memory.
    ///
    /// Field: git_index_entry.flags_extended
    pub fn flags_extended(&self) -> EntryExtendedFlags {
        // SAFETY: as `dev`, for `flags_extended`.
        let raw = unsafe { addr_of!((*self.as_ptr()).flags_extended).read() };
        EntryExtendedFlags::from_bits_retain(raw.into())
    }

    /// The file's path, relative to the working directory, borrowed for `'p`.
    ///
    /// Field: git_index_entry.path
    pub fn path(&self) -> &'p CStr {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to a live entry. Per the
        // handles' invariant, which `new`, `set_path` and `from_ptr` keep, `path` is
        // non-NULL and addresses a NUL-terminated string live and unchanged for `'p`.
        unsafe { CStr::from_ptr(addr_of!((*p).path).read()) }
    }

    /// A copy of the entry, as a value of its own, borrowing the same path.
    pub fn to_index_entry(&self) -> IndexEntry<'p> {
        // SAFETY: a read of the whole struct through the shared handle's pointer to a live,
        // initialised entry; it is plain data, and the copy's path keeps the handles'
        // invariant for `'p`, as this entry's does.
        IndexEntry(unsafe { self.as_ptr().read() }, PhantomData)
    }
}

impl fmt::Debug for IndexEntryRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let time = |t: IndexTimeRef<'_>| (t.seconds(), t.nanoseconds());
        f.debug_struct("IndexEntry")
            .field("ctime", &time(self.ctime()))
            .field("mtime", &time(self.mtime()))
            .field("dev", &self.dev())
            .field("ino", &self.ino())
            .field("mode", &format_args!("{:o}", self.mode()))
            .field("uid", &self.uid())
            .field("gid", &self.gid())
            .field("file_size", &self.file_size())
            .field("id", &self.id())
            .field("flags", &self.flags())
            .field("flags_extended", &self.flags_extended())
            .field("path", &self.path())
            .finish()
    }
}

impl<'p> IndexEntryMut<'_, 'p> {
    /// Writable pointer to the C struct, for the setters.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_index_entry {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> IndexEntryRef<'_, 'p> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> IndexEntryMut<'_, 'p> {
        IndexEntryMut(self.0, PhantomData)
    }

    /// The metadata change time, in place, borrowed exclusively to change it.
    ///
    /// Field: git_index_entry.ctime
    pub fn ctime_mut(&mut self) -> IndexTimeMut<'_> {
        let p = self.as_mut_ptr();
        // SAFETY: the field projected from the exclusive handle's pointer to a live entry
        // is an initialised `git_index_time`, any of whose values may be written; the
        // result borrows this handle exclusively, so nothing else reaches the entry
        // meanwhile.
        unsafe { IndexTimeMut::from_ptr(addr_of_mut!((*p).ctime)) }.expect("a field is never NULL")
    }

    /// Replaces the metadata change time with a copy of `time`.
    ///
    /// Field: git_index_entry.ctime
    pub fn set_ctime(&mut self, time: IndexTimeRef<'_>) {
        let mut ctime = self.ctime_mut();
        ctime.set_seconds(time.seconds());
        ctime.set_nanoseconds(time.nanoseconds());
    }

    /// The contents change time, in place, borrowed exclusively to change it.
    ///
    /// Field: git_index_entry.mtime
    pub fn mtime_mut(&mut self) -> IndexTimeMut<'_> {
        let p = self.as_mut_ptr();
        // SAFETY: as `ctime_mut`, for `mtime`.
        unsafe { IndexTimeMut::from_ptr(addr_of_mut!((*p).mtime)) }.expect("a field is never NULL")
    }

    /// Replaces the contents change time with a copy of `time`.
    ///
    /// Field: git_index_entry.mtime
    pub fn set_mtime(&mut self, time: IndexTimeRef<'_>) {
        let mut mtime = self.mtime_mut();
        mtime.set_seconds(time.seconds());
        mtime.set_nanoseconds(time.nanoseconds());
    }

    /// Replaces the device number.
    ///
    /// Field: git_index_entry.dev
    pub fn set_dev(&mut self, dev: u32) {
        // SAFETY: a write through the exclusive handle's pointer to a live entry; every
        // `u32` is valid.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).dev).write(dev) }
    }

    /// Replaces the inode number.
    ///
    /// Field: git_index_entry.ino
    pub fn set_ino(&mut self, ino: u32) {
        // SAFETY: as `set_dev`, for `ino`.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).ino).write(ino) }
    }

    /// Replaces the mode with any value, named or not; the index routines refuse an
    /// entry whose mode is not one of the insertable [`FileMode`]s.
    ///
    /// Field: git_index_entry.mode
    pub fn set_mode(&mut self, mode: u32) {
        // SAFETY: as `set_dev`, for `mode`.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).mode).write(mode) }
    }

    /// Replaces the mode with `filemode`'s value.
    ///
    /// Field: git_index_entry.mode
    pub fn set_filemode(&mut self, filemode: FileMode) {
        self.set_mode(filemode.raw());
    }

    /// Replaces the owner's user id.
    ///
    /// Field: git_index_entry.uid
    pub fn set_uid(&mut self, uid: u32) {
        // SAFETY: as `set_dev`, for `uid`.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).uid).write(uid) }
    }

    /// Replaces the owner's group id.
    ///
    /// Field: git_index_entry.gid
    pub fn set_gid(&mut self, gid: u32) {
        // SAFETY: as `set_dev`, for `gid`.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).gid).write(gid) }
    }

    /// Replaces the file size.
    ///
    /// Field: git_index_entry.file_size
    pub fn set_file_size(&mut self, file_size: u32) {
        // SAFETY: as `set_dev`, for `file_size`.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).file_size).write(file_size) }
    }

    /// The id the entry names, in place, borrowed exclusively to change it.
    ///
    /// Field: git_index_entry.id
    pub fn id_mut(&mut self) -> OidMut<'_> {
        let p = self.as_mut_ptr();
        // SAFETY: the field projected from the exclusive handle's pointer to a live entry
        // is an initialised `git_oid`, any of whose bytes may be written; the result
        // borrows this handle exclusively, so nothing else reaches the entry meanwhile.
        unsafe { OidMut::from_ptr(addr_of_mut!((*p).id)) }.expect("a field is never NULL")
    }

    /// Replaces the id with a copy of `id`, type and all 32 bytes.
    ///
    /// Field: git_index_entry.id
    pub fn set_id(&mut self, id: OidRef<'_>) {
        let id = copy_id(id);
        // SAFETY: a write through the exclusive handle's pointer to a live entry; the
        // field is plain bytes, and `id` was copied out before the write.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).id).write(id) }
    }

    /// Replaces the 16 flag bits with `flags`, stage and path-length bits included.
    ///
    /// Fails, changing nothing, when `flags` has a bit above the field's 16.
    ///
    /// Field: git_index_entry.flags
    pub fn set_flags(&mut self, flags: EntryFlags) -> Result<(), TryFromIntError> {
        let raw = u16::try_from(flags.bits())?;
        // SAFETY: as `set_dev`, for `flags`; every `u16` is valid.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).flags).write(raw) }
        Ok(())
    }

    /// Replaces the 16 extended flag bits with `flags`, the in-memory ones included.
    ///
    /// Fails, changing nothing, when `flags` has a bit above the field's 16.
    ///
    /// Field: git_index_entry.flags_extended
    pub fn set_flags_extended(&mut self, flags: EntryExtendedFlags) -> Result<(), TryFromIntError> {
        let raw = u16::try_from(flags.bits())?;
        // SAFETY: as `set_dev`, for `flags_extended`; every `u16` is valid.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).flags_extended).write(raw) }
        Ok(())
    }

    /// Replaces the path with `path`, borrowed for `'p`. The previous path is only
    /// borrowed, so nothing is released.
    ///
    /// Field: git_index_entry.path
    pub fn set_path(&mut self, path: &'p CStr) {
        // SAFETY: a write through the exclusive handle's pointer to a live entry; the
        // pointer stored is a `&'p CStr`'s, non-NULL and live and unchanged for `'p`,
        // which keeps the handles' invariant.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).path).write(path.as_ptr()) }
    }
}

impl fmt::Debug for IndexEntryMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// An `IndexEntry` is a `git_index_entry`, and a handle one pointer.
const _: () = assert!(
    size_of::<IndexEntry<'static>>() == size_of::<ffi::git_index_entry>()
        && align_of::<IndexEntry<'static>>() == align_of::<ffi::git_index_entry>()
        && size_of::<IndexEntryRef<'static, 'static>>() == size_of::<*const ()>()
        && size_of::<IndexOptions>() == size_of::<ffi::git_index_options>()
        && align_of::<IndexOptions>() == align_of::<ffi::git_index_options>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::oid::oid::Oid;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use std::ffi::CString;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::SeqCst};

    #[test]
    fn stage_values_are_checked() {
        for value in -8..=8 {
            assert_eq!(
                Stage::from_raw(value).map(Stage::raw),
                (-1..=3).contains(&value).then_some(value)
            );
        }
        assert_eq!(Stage::from_raw(i32::MIN), None);
        assert_eq!(Stage::from_raw(i32::MAX), None);
        assert_eq!(
            size_of::<Stage>(),
            size_of::<crate::ffi::git_index_stage_t>()
        );
        assert_eq!(
            align_of::<Stage>(),
            align_of::<crate::ffi::git_index_stage_t>()
        );
    }

    #[test]
    fn entry_flags_value_operations_and_layout() {
        assert_eq!(
            size_of::<EntryFlags>(),
            size_of::<ffi::git_index_entry_flag_t>()
        );
        assert_eq!(
            align_of::<EntryFlags>(),
            align_of::<ffi::git_index_entry_flag_t>()
        );
        assert_eq!(EntryFlags::default(), EntryFlags::empty());
        assert!(EntryFlags::empty().contains(EntryFlags::empty()));
        let flags = EntryFlags::EXTENDED | EntryFlags::VALID;
        assert!(flags.contains(EntryFlags::EXTENDED));
        assert!(flags.contains(EntryFlags::VALID));
        assert!(flags.contains(flags));
        assert!(!EntryFlags::EXTENDED.contains(flags));
        let mut flags = flags;
        flags |= EntryFlags::from_bits_retain(0x8000_1234);
        assert_eq!(flags.bits(), 0x8000_d234);
        flags.remove(EntryFlags::EXTENDED | EntryFlags::VALID);
        assert_eq!(flags.bits(), 0x8000_1234);
        let copy = flags;
        assert_eq!(copy, flags);
        assert!(format!("{flags:?}").starts_with("EntryFlags("));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn iterator_constructor_allocation_failures_are_balanced() {
        isolated(
            concat!(
                module_path!(),
                "::iterator_constructor_allocation_failures_are_balanced"
            ),
            || {
                install_countdown();
                assert!(runtime_pinned());
                set(ErrorClass::Invalid, c"allocate error state");
                for populated in [false, true] {
                    let index: IndexArc = Index::new().unwrap().into();
                    if populated {
                        let entry = ffi::git_index_entry {
                            mode: 0o100644,
                            path: c"entry".as_ptr(),
                            ..Default::default()
                        };
                        // SAFETY: live index with no outstanding entry borrow; C copies
                        // this initialized entry and static NUL path during the call.
                        assert_eq!(unsafe { ffi::git_index_add(index.as_c_ptr(), &entry) }, 0);
                    }
                    let baseline = LIVE.load(SeqCst);
                    let allocations = if populated { 2 } else { 1 };
                    for n in 0..=allocations {
                        ALLOWED.store(n, SeqCst);
                        REFUSING.store(true, SeqCst);
                        let result = IndexIterator::new(&index);
                        REFUSING.store(false, SeqCst);
                        if n < allocations {
                            let error = result.unwrap_err();
                            assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                            assert_eq!(error.message(), c"Out of memory");
                        } else {
                            let iterator = result.unwrap();
                            let ptr = iterator.into_c();
                            // SAFETY: the sole live iterator just relinquished above,
                            // with its retained index reference; adopt exactly once.
                            drop(unsafe { IndexIteratorBox::from_c(ptr) }.unwrap());
                        }
                        assert_eq!(LIVE.load(SeqCst), baseline);
                    }
                    // Failure in snapshot allocation must also balance index rc/readers;
                    // LeakSanitizer observes the final index release below.
                    drop(index);
                }
            },
        );
    }

    #[test]
    fn extended_flags_preserve_bits_and_combine() {
        use EntryExtendedFlags as F;
        assert_eq!(F::INTENT_TO_ADD | F::SKIP_WORKTREE, F::ON_DISK);
        assert!(!F::ON_DISK.contains(F::UP_TO_DATE));
        for bits in 0..=u16::MAX as u32 {
            let mut f = F::from_bits_retain(bits);
            assert_eq!(f.bits(), bits);
            f.insert(F::UP_TO_DATE);
            assert!(f.contains(F::UP_TO_DATE));
            f.remove(F::ON_DISK);
            assert_eq!(f.bits(), (bits | 4) & !0x6000);
            assert_eq!((f & F::ON_DISK), F::EMPTY);
        }
        assert_eq!(F::from_bits_retain(u32::MAX).bits(), u32::MAX);
        assert_eq!(F::default(), F::EMPTY);
        assert_eq!(format!("{:?}", F::EMPTY), "EntryExtendedFlags(0)");
    }

    /// Miri exercises the hand-written transparent handle layout without FFI.
    /// This Rust-owned opaque placeholder is never passed to libgit2 or freed as
    /// a C cursor; only pointer identity and reborrow layout are tested.
    #[test]
    fn conflict_cursor_handle_layout() {
        use core::mem::{align_of, size_of};
        assert_eq!(
            size_of::<ConflictIterator<'_>>(),
            size_of::<ffi::git_index_conflict_iterator>()
        );
        assert_eq!(
            align_of::<ConflictIterator<'_>>(),
            align_of::<ffi::git_index_conflict_iterator>()
        );
        assert_eq!(
            size_of::<EntryExtendedFlags>(),
            size_of::<ffi::git_index_entry_extended_flag_t>()
        );
        assert_eq!(
            align_of::<EntryExtendedFlags>(),
            align_of::<ffi::git_index_entry_extended_flag_t>()
        );
        assert_eq!(
            size_of::<ConflictIteratorMut<'_, '_>>(),
            size_of::<*mut ()>()
        );
        assert_eq!(
            size_of::<ConflictIteratorRef<'_, '_>>(),
            size_of::<*mut ()>()
        );
        // SAFETY: bindgen represents this opaque declaration by a zero-sized
        // byte array, for which every bit pattern is valid. It is only a Rust
        // layout fixture, never used as a real C cursor.
        let raw = unsafe { core::mem::zeroed::<ffi::git_index_conflict_iterator>() };
        let mut slot = ConflictIterator(raw, core::marker::PhantomData);
        let p = NonNull::from(&mut slot);
        // SAFETY: initialized Rust-owned opaque layout lives through all handles;
        // no access reaches C, and the original place is not reborrowed meanwhile.
        let shared = ConflictIteratorRef(unsafe { ffibox::CBorrowedPtr::new(p) });
        let mut handle = ConflictIteratorMut(shared, core::marker::PhantomData);
        assert_eq!(
            handle.as_mut_ptr().cast::<ConflictIterator<'_>>(),
            p.as_ptr()
        );
        assert_eq!(
            handle.as_mut().as_ref().as_ptr().cast_mut().cast(),
            p.as_ptr()
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn conflict_cursor_round_trip_and_allocation_failure() {
        isolated(
            concat!(
                module_path!(),
                "::conflict_cursor_round_trip_and_allocation_failure"
            ),
            || {
                install_countdown();
                let mut index = Index::new().unwrap();
                set(ErrorClass::Invalid, c"preallocate error state");
                ALLOWED.store(0, SeqCst);
                REFUSING.store(true, SeqCst);
                let failed = ConflictIterator::new(index.as_mut());
                REFUSING.store(false, SeqCst);
                let error = failed.err().unwrap();
                assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                assert_eq!(error.message(), c"Out of memory");
                let before = LIVE.load(SeqCst);
                for _ in 0..3 {
                    let mut cursor = ConflictIterator::new(index.as_mut()).unwrap();
                    let p = cursor.as_c_ptr();
                    assert_eq!(cursor.as_ref().as_ptr(), p);
                    assert_eq!(cursor.as_mut().as_mut().as_mut_ptr(), p);
                    let p = cursor.into_c();
                    // SAFETY: this pointer is the just-released sole C allocation;
                    // index remains live and unused until the readopted owner drops.
                    let cursor = unsafe { ConflictIteratorBox::from_c(p) }.unwrap();
                    drop(cursor);
                    assert_eq!(LIVE.load(SeqCst), before);
                }
                // SAFETY: C explicitly accepts NULL as a no-op destructor argument.
                unsafe { ffi::git_index_conflict_iterator_free(ptr::null_mut()) };
            },
        );
    }

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process, where nothing has initialised libgit2.
    fn isolated(test: &str, body: fn()) {
        isolated_with(test, body, true);
    }

    /// As [`isolated`], with LeakSanitizer off in the child when `detect_leaks` is false:
    /// for bodies that drive libgit2 down a path on which it leaks.
    fn isolated_with(test: &str, body: fn(), detect_leaks: bool) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name);
        if !detect_leaks {
            let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
            let separator = if inherited.is_empty() { "" } else { ":" };
            command.env(
                "ASAN_OPTIONS",
                format!("{inherited}{separator}detect_leaks=0"),
            );
        }
        let out = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The routines of `git2/index.h` that read, write and remove entries, beside the
    /// refusals the wrappers make before libgit2 sees an argument.
    mod edits {
        use super::*;
        use crate::libgit2::core::strarray::StrArrayView;
        use crate::libgit2::repository::repository::{Repository, RepositoryBox};
        use crate::libgit2::util::errors::ErrorCode;

        /// A regular-file entry at `path` and `stage`, of id type `kind` with first byte
        /// `byte` (0 for the zero id), added raw: `git_index_add` is a later batch.
        fn add(index: &mut IndexMut<'_>, path: &CStr, stage: u16, kind: u8, byte: u8, mode: u32) {
            let mut entry = ffi::git_index_entry {
                mode,
                path: path.as_ptr(),
                flags: stage << 12, // GIT_INDEX_ENTRY_STAGESHIFT
                ..Default::default()
            };
            entry.id.type_ = kind;
            entry.id.id[0] = byte;
            // SAFETY: a live index, exclusively borrowed with no entry borrow; C copies
            // the initialised entry and its NUL-terminated path.
            assert_eq!(unsafe { ffi::git_index_add(index.as_mut_ptr(), &entry) }, 0);
        }

        fn paths(index: IndexRef<'_>) -> Vec<(CString, i32)> {
            let ptr = index.as_ptr().cast_mut();
            // SAFETY: a live index, only read; each path is copied before the next call.
            unsafe {
                (0..ffi::git_index_entrycount(ptr))
                    .map(|i| {
                        let e = ffi::git_index_get_byindex(ptr, i);
                        (
                            CStr::from_ptr((*e).path).to_owned(),
                            ffi::git_index_entry_stage(e),
                        )
                    })
                    .collect()
            }
        }

        fn scratch_repo(tag: &str) -> (std::path::PathBuf, RepositoryBox) {
            let dir = std::env::temp_dir()
                .join(format!("crustify-index-edits-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            (dir, Repository::init(&path, true).unwrap())
        }

        #[test]
        fn status_reports_positive_codes_as_generic() {
            assert!(status(0).is_ok());
            if cfg!(miri) {
                return;
            }
            assert_eq!(status(1).unwrap_err().code(), Some(ErrorCode::Generic));
            assert_eq!(status(-3).unwrap_err().code(), Some(ErrorCode::NotFound));
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn mixed_empty_pathspecs_are_refused_before_the_scan() {
            let mut index = Index::new().unwrap();
            add(&mut index.as_mut(), c"a", 0, 1, 1, 0o100644);
            let patterns = StrArrayView::new(&[c"", c"a"]);
            let mut calls = 0;
            let mut f = |_: &CStr, _: Option<&CStr>| {
                calls += 1;
                MatchedPathResult::APPLY
            };
            let mut callback = MatchedPathCallback::new(&mut f);
            let removed = index
                .as_mut()
                .remove_all(patterns.as_ref(), Some(&mut callback));
            let updated = index
                .as_mut()
                .update_all(patterns.as_ref(), Some(&mut callback));
            drop(callback);
            for error in [removed.unwrap_err(), updated.unwrap_err()] {
                assert_eq!(error.class(), Some(ErrorClass::Invalid));
                assert_eq!(error.raw_code(), -1);
            }
            assert_eq!(calls, 0);
            assert_eq!(paths(index.as_ref()), [(c"a".to_owned(), 0)]);
        }

        /// libgit2 would remove nothing and succeed: `Stage::Any` is refused instead.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn removing_a_directory_at_any_stage_is_refused() {
            let mut index = Index::new().unwrap();
            add(&mut index.as_mut(), c"dir/a", 0, 1, 1, 0o100644);
            let error = index
                .as_mut()
                .remove_directory(c"dir", Stage::Any)
                .unwrap_err();
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
            assert_eq!(paths(index.as_ref()), [(c"dir/a".to_owned(), 0)]);
            let (raw, any) = (index.as_c_ptr(), Stage::Any.raw());
            // SAFETY: a live index; the stage is C's wildcard, which it compares literally.
            let rc = unsafe { ffi::git_index_remove_directory(raw, c"dir".as_ptr(), any) };
            assert_eq!(rc, 0);
            assert_eq!(paths(index.as_ref()), [(c"dir/a".to_owned(), 0)]);
            index
                .as_mut()
                .remove_directory(c"dir", Stage::Normal)
                .unwrap();
            assert!(paths(index.as_ref()).is_empty());
        }

        /// Ids libgit2 would look up in a repository of another id type, and names too
        /// long for a tree entry, are refused before any tree is written; a conflicted
        /// index is left to libgit2's `GIT_EUNMERGED`, and submodule and zero ids are not
        /// looked up.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn tree_writes_refuse_foreign_ids_and_long_names() {
            let (dir, repo) = scratch_repo("refusals");
            let sha256 = OidType::Sha256.raw() as u8;
            let sha1 = OidType::Sha1.raw() as u8;

            let mut index = Index::new().unwrap();
            add(&mut index.as_mut(), c"a", 0, sha256, 1, 0o100644);
            let error = index.as_ref().write_tree_to(repo.as_ref()).unwrap_err();
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
            assert_eq!(
                error.message(),
                c"specified object ID type does not match repository object ID type"
            );

            let mut index = Index::new().unwrap();
            add(
                &mut index.as_mut(),
                c"sub",
                0,
                sha256,
                1,
                ffi::GIT_FILEMODE_COMMIT,
            );
            add(&mut index.as_mut(), c"zero", 0, sha256, 0, 0o100644);
            let error = index.as_ref().write_tree_to(repo.as_ref()).unwrap_err();
            // libgit2's own refusal of the zero id, after no lookup.
            assert_eq!(error.class(), Some(ErrorClass::Tree));

            let mut index = Index::new().unwrap();
            add(&mut index.as_mut(), c"a", 1, sha256, 1, 0o100644);
            add(&mut index.as_mut(), c"a", 2, sha256, 2, 0o100644);
            let error = index.as_ref().write_tree_to(repo.as_ref()).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::Unmerged));

            let long = CString::new(format!("dir/{}", "n".repeat(TREE_NAME_MAX + 1))).unwrap();
            let mut index = Index::new().unwrap();
            add(&mut index.as_mut(), &long, 0, sha1, 1, 0o100644);
            let error = index.as_ref().write_tree_to(repo.as_ref()).unwrap_err();
            assert_eq!(error.message(), c"tree entry path too long");
            let just = CString::new("n".repeat(TREE_NAME_MAX)).unwrap();
            let mut index = Index::new().unwrap();
            add(&mut index.as_mut(), &just, 0, sha1, 1, 0o100644);
            let error = index.as_ref().write_tree_to(repo.as_ref()).unwrap_err();
            // Not refused: libgit2 looks the (missing) object up.
            assert_eq!(error.class(), Some(ErrorClass::Tree));
            drop(repo);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// A deliberate divergence: libgit2 removes whichever stage of a conflict its
        /// search meets for `GIT_INDEX_STAGE_ANY` (stage 2 of three), and drops the stage 3
        /// entry it keeps from its path lookup table, so `git_index_get_bypath` no longer
        /// finds it. `remove` takes the first stage, and the table stays whole.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn removal_at_any_stage_takes_the_first_and_keeps_lookups() {
            let conflicted = || {
                let mut index = Index::new().unwrap();
                for stage in 1..=3 {
                    add(&mut index.as_mut(), c"a", stage, 1, stage as u8, 0o100644);
                }
                index
            };
            let found = |index: &mut IndexBox| -> Vec<bool> {
                (1..=3)
                    .map(|stage| {
                        // SAFETY: a live index and a NUL-terminated path, read for the call;
                        // the entry returned is only compared with NULL.
                        let entry = unsafe {
                            ffi::git_index_get_bypath(index.as_c_ptr(), c"a".as_ptr(), stage)
                        };
                        !entry.is_null()
                    })
                    .collect()
            };

            let mut raw = conflicted();
            // SAFETY: a live index, exclusively borrowed with no entry borrow; the path is
            // read for the call; the stage is C's wildcard.
            let rc = unsafe { ffi::git_index_remove(raw.as_c_ptr(), c"a".as_ptr(), -1) };
            assert_eq!(rc, 0);
            let kept = [(c"a".to_owned(), 1), (c"a".to_owned(), 3)];
            assert_eq!(paths(raw.as_ref()), kept);
            assert_eq!(found(&mut raw), [true, false, false]);

            let mut index = conflicted();
            index.as_mut().remove(c"a", Stage::Any).unwrap();
            let kept = [(c"a".to_owned(), 2), (c"a".to_owned(), 3)];
            assert_eq!(paths(index.as_ref()), kept);
            assert_eq!(found(&mut index), [false, true, true]);
            index.as_mut().remove(c"a", Stage::Any).unwrap();
            index.as_mut().remove(c"a", Stage::Any).unwrap();
            assert!(paths(index.as_ref()).is_empty());
            let error = index.as_mut().remove(c"a", Stage::Any).unwrap_err();
            assert_eq!(
                (error.code(), error.class(), error.message()),
                (
                    Some(ErrorCode::NotFound),
                    Some(ErrorClass::Index),
                    c"index does not contain a at stage -1"
                )
            );

            // Case-insensitively, the first entry `find` finds goes.
            let mut index = Index::new().unwrap();
            let caps = CapabilitySetting::Explicit(Capabilities::IGNORE_CASE);
            index.as_mut().set_caps(caps).unwrap();
            add(&mut index.as_mut(), c"A", 2, 1, 2, 0o100644);
            add(&mut index.as_mut(), c"A", 3, 1, 3, 0o100644);
            index.as_mut().remove(c"a", Stage::Any).unwrap();
            assert_eq!(paths(index.as_ref()), [(c"A".to_owned(), 3)]);
        }

        /// `write` refuses an index a repository with a working directory owns when it
        /// holds an empty path beside another entry libgit2 may scan as racily clean;
        /// submodules and indexes without such a repository are left to libgit2.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn writes_refuse_empty_paths_beside_scannable_entries() {
            let dir = std::env::temp_dir()
                .join(format!("crustify-index-edits-racy-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let mut repo = Repository::init(&path, false).unwrap();
            let in_memory = c"failed to write index: The index is in-memory only";
            // Entries as an index file holds them: `git_index_add` marks its entries up
            // to date (until the next write), and an index file's are not.
            let owned = |entries: &[(&CStr, u32)], repo: &mut RepositoryBox| {
                let mut index = Index::new().unwrap();
                for (n, &(path, mode)) in entries.iter().enumerate() {
                    add(&mut index.as_mut(), path, 0, 1, n as u8 + 1, mode);
                }
                for position in 0..entries.len() {
                    // SAFETY: a live index lending no entry; `position` is within the
                    // count, so the entry is live, and its flag is written in place.
                    unsafe {
                        let entry = ffi::git_index_get_byindex(index.as_c_ptr(), position);
                        let flags = &raw mut (*entry.cast_mut()).flags_extended;
                        *flags &= !(ffi::GIT_INDEX_ENTRY_UPTODATE as u16);
                    }
                }
                let index: IndexArc = index.into();
                repo.as_mut().set_index(Some(&index));
                index
            };

            for entries in [
                &[(c"", 0o100644), (c"a", 0o100644)][..],
                &[
                    (c"a", 0o100644),
                    (c"", 0o100755),
                    (c"sub", ffi::GIT_FILEMODE_COMMIT),
                ],
            ] {
                let index = owned(entries, &mut repo);
                let error = index.as_ref().write().unwrap_err();
                assert_eq!(
                    (error.raw_code(), error.class(), error.message()),
                    (
                        -1,
                        Some(ErrorClass::Index),
                        c"cannot write an index holding an entry with an empty path beside others"
                    )
                );
            }
            // Left to libgit2, which scans none of these as racily clean beside `""`.
            for entries in [
                &[(c"", 0o100644)][..],
                &[(c"", 0o100644), (c"sub", ffi::GIT_FILEMODE_COMMIT)],
                &[(c"", ffi::GIT_FILEMODE_COMMIT), (c"a", 0o100644)],
                &[(c"a", 0o100644), (c"b", 0o100644)],
            ] {
                let index = owned(entries, &mut repo);
                let error = index.as_ref().write().unwrap_err();
                assert_eq!(error.message(), in_memory);
            }
            // Up to date, as `git_index_add` leaves its entries.
            let mut index = Index::new().unwrap();
            add(&mut index.as_mut(), c"", 0, 1, 1, 0o100644);
            add(&mut index.as_mut(), c"a", 0, 1, 2, 0o100644);
            let index: IndexArc = index.into();
            repo.as_mut().set_index(Some(&index));
            assert_eq!(index.as_ref().write().unwrap_err().message(), in_memory);
            let index = owned(&[(c"", 0o100644), (c"a", 0o100644)], &mut repo);
            drop(repo);
            assert_eq!(index.as_ref().write().unwrap_err().message(), in_memory);
            let (bare_dir, mut bare) = scratch_repo("racy-bare");
            let index = owned(&[(c"", 0o100644), (c"a", 0o100644)], &mut bare);
            assert_eq!(index.as_ref().write().unwrap_err().message(), in_memory);
            drop((index, bare));
            let _ = std::fs::remove_dir_all(bare_dir);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// The repository owning an index is checked as `write_tree_to`'s argument is.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn owner_tree_writes_refuse_foreign_ids() {
            let (dir, repo) = scratch_repo("owner");
            let mut built = Index::with_oid_type(OidType::Sha256).unwrap();
            add(
                &mut built.as_mut(),
                c"a",
                0,
                OidType::Sha256.raw() as u8,
                1,
                0o100644,
            );
            let shared: IndexArc = built.into();
            let mut repo = repo;
            repo.as_mut().set_index(Some(&shared));
            let error = shared.as_ref().write_tree().unwrap_err();
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
            // Once the repository is gone, the index has no owner to write to.
            drop(repo);
            let error = shared.as_ref().write_tree().unwrap_err();
            assert_eq!(error.class(), Some(ErrorClass::Index));
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// The policy's constructor hands out a fresh index, which `from_raw` adopts and
    /// `into_raw` gives back, released once.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn policy_constructor_round_trips_through_raw() {
        let fresh = IndexFree.c_new().unwrap();
        // SAFETY: a fresh index holding its only reference, which `IndexFree` releases.
        let index = unsafe { IndexBox::from_raw(fresh.as_ptr()) }.unwrap();
        let raw = index.into_raw();
        assert_eq!(raw, fresh.as_ptr());
        // SAFETY: the reference `into_raw` gave back, adopted once.
        let index: IndexArc = unsafe { IndexBox::from_raw(raw) }.unwrap().into();
        // SAFETY: the arc's live index, only read.
        assert_eq!(unsafe { ffi::git_index_entrycount(index.as_c_ptr()) }, 0);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn options_carry_this_version_and_the_type() {
        for kind in [OidType::Sha1, OidType::Sha256] {
            let opts = IndexOptions::with_oid_type(kind).into_raw();
            assert_eq!(opts.version, ffi::GIT_INDEX_OPTIONS_VERSION);
            assert_eq!(OidType::from_raw(opts.oid_type), Some(kind));
        }
    }

    /// A failure, or a success without an index, reports the thread's error and adopts
    /// nothing.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructed_reports_the_threads_error() {
        assert!(runtime_pinned());
        set(ErrorClass::Index, c"refused here");
        let error = constructed(-1, None).unwrap_err();
        assert_eq!(
            (error.class(), error.message()),
            (Some(ErrorClass::Index), c"refused here")
        );
        set(ErrorClass::Invalid, c"no index after all");
        let error = constructed(0, None).unwrap_err();
        assert_eq!(error.message(), c"no index after all");
    }

    /// A deliberate divergence: libgit2 opens an empty path, and writing that index is
    /// undefined behaviour (a NULL `memcpy` source in `git_filebuf_open_withsize`), so
    /// `open` refuses it. C's acceptance is checked without the write.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn empty_path_is_refused_where_c_accepts_it() {
        for built in [
            Index::open(c""),
            Index::open_with_oid_type(c"", OidType::Sha256),
        ] {
            let error = built.unwrap_err();
            assert_eq!(
                (error.class(), error.message()),
                (
                    Some(ErrorClass::Index),
                    c"failed to open index: the path is empty"
                )
            );
        }
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot and a NUL-terminated path, read for the call; the
        // runtime is pinned (by `open` above).
        let rc = unsafe { ffi::git_index_open(&mut out, c"".as_ptr()) };
        assert_eq!(rc, 0);
        // SAFETY: the fresh index C handed out, adopted once and never written.
        let c_index = unsafe { IndexBox::from_c(out) }.unwrap();
        // SAFETY: a live index, only read; the path is copied before the next call.
        let path = unsafe { CStr::from_ptr(ffi::git_index_path(c_index.as_c_ptr())) };
        assert_eq!(path, c"");
    }

    static REFUSING: AtomicBool = AtomicBool::new(false);
    static ALLOWED: AtomicIsize = AtomicIsize::new(0);
    /// Blocks [`Countdown`] handed out and has not had back.
    static LIVE: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut core::ffi::c_void;
        fn realloc(p: *mut core::ffi::c_void, n: usize) -> *mut core::ffi::c_void;
        fn free(p: *mut core::ffi::c_void);
    }

    /// Whether the next request is refused: while `REFUSING` is set, after `ALLOWED`
    /// more requests.
    fn refuse() -> bool {
        REFUSING.load(SeqCst) && ALLOWED.fetch_sub(1, SeqCst) <= 0
    }

    /// libc's allocator, refusing requests per [`refuse`].
    struct Countdown;

    // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
    unsafe impl Alloc for Countdown {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            if refuse() {
                return None;
            }
            // SAFETY: plain libc allocation.
            let block = NonNull::new(unsafe { malloc(len) }.cast());
            if block.is_some() {
                LIVE.fetch_add(1, SeqCst);
            }
            block
        }
        unsafe fn realloc(
            p: Option<NonNull<u8>>,
            len: usize,
            _: Option<&CStr>,
            _: i32,
        ) -> Option<NonNull<u8>> {
            if refuse() {
                return None;
            }
            let fresh = p.is_none();
            let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
            // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
            let block = NonNull::new(unsafe { realloc(p, len) }.cast());
            if fresh && block.is_some() {
                LIVE.fetch_add(1, SeqCst);
            }
            block
        }
        unsafe fn free(p: NonNull<u8>) {
            LIVE.fetch_sub(1, SeqCst);
            // SAFETY: `p` is a live libc block the caller gives up.
            unsafe { free(p.as_ptr().cast()) }
        }
    }

    /// Runs `build` with the `n`th allocation from now refused (and every later one),
    /// for `n` = 0, 1, ... until it succeeds; asserts each failure reports libgit2's
    /// out-of-memory error. Returns the number of failures.
    fn refuse_each_allocation(build: impl Fn() -> Result<IndexBox, ErrorBuf>) -> isize {
        // The thread's error state, allocated now, so a refusal reports out of memory
        // rather than a thread state it could not allocate.
        set(ErrorClass::Invalid, c"error state allocated");
        for n in 0.. {
            ALLOWED.store(n, SeqCst);
            REFUSING.store(true, SeqCst);
            let built = build();
            REFUSING.store(false, SeqCst);
            match built {
                Ok(index) => {
                    drop(index);
                    return n;
                }
                Err(error) => assert_eq!(
                    (error.class(), error.message()),
                    (Some(ErrorClass::NoMemory), c"Out of memory"),
                    "allocation {n}"
                ),
            }
        }
        unreachable!()
    }

    /// Installs [`Countdown`] as libgit2's allocator, before anything is allocated.
    fn install_countdown() {
        let allocator = Allocator::new::<Countdown>();
        // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with every
        // member set; libgit2 copies it before anything is allocated.
        let rc = unsafe {
            ffi::git_libgit2_opts(
                ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
    }

    /// NAME construction must free partial strings and leave the vector unchanged
    /// when any allocation (including vector growth) fails.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn name_allocation_failures_release_partial_records() {
        isolated(
            concat!(
                module_path!(),
                "::name_allocation_failures_release_partial_records"
            ),
            || {
                install_countdown();
                assert!(runtime_pinned());
                let mut failures = 0;
                let mut successes = 0;
                for allowed in 0..=8 {
                    let before = LIVE.load(SeqCst);
                    let mut index = Index::new().unwrap();
                    // Fill the initial vector so the next insertion must grow it.
                    for _ in 0..8 {
                        index
                            .as_mut()
                            .add_name(Some(c"a"), Some(c"o"), Some(c"t"))
                            .unwrap();
                    }
                    ALLOWED.store(allowed, SeqCst);
                    REFUSING.store(true, SeqCst);
                    let result = index.as_mut().add_name(Some(c"b"), Some(c"p"), Some(c"u"));
                    REFUSING.store(false, SeqCst);
                    if result.is_err() {
                        failures += 1;
                        assert_eq!(index.as_ref().name_entry_count(), 8);
                    } else {
                        successes += 1;
                        assert_eq!(index.as_ref().name_entry_count(), 9);
                    }
                    index.as_mut().clear_names().unwrap();
                    drop(index);
                    assert_eq!(LIVE.load(SeqCst), before, "allocation allowance {allowed}");
                }
                assert!(failures >= 4 && successes > 0);
            },
        );
    }

    /// A REUC record is allocated whole, path inline (`reuc_entry_alloc`), before it is
    /// inserted. When that allocation or the vector's growth fails, libgit2 frees the new
    /// record and the index keeps the records it had, the one at the same path included:
    /// `git_vector_insert_sorted` grows a full vector before it looks for a duplicate,
    /// so even a replacement can fail there.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reuc_record_allocation_failures_release_the_new_record() {
        isolated(
            concat!(
                module_path!(),
                "::reuc_record_allocation_failures_release_the_new_record"
            ),
            || {
                use crate::libgit2::index::sys::index::ReucSide;

                install_countdown();
                assert!(runtime_pinned());
                set(ErrorClass::Invalid, c"allocate error state");
                let id = Oid::zeroed();
                let side = |mode| Some(ReucSide::new(mode, id.as_ref()));
                let mut failures = 0;
                let mut successes = 0;
                for (path, new_path) in [(c"p8", true), (c"p0", false)] {
                    for allowed in 0..=2 {
                        let before = LIVE.load(SeqCst);
                        let mut index = Index::new().unwrap();
                        // Fill the initial vector so that the next insertion grows it.
                        for i in 0..8 {
                            let path = CString::new(format!("p{i}")).unwrap();
                            let blob = side(FileMode::Blob);
                            index.as_mut().add_reuc(&path, blob, None, None).unwrap();
                        }
                        ALLOWED.store(allowed, SeqCst);
                        REFUSING.store(true, SeqCst);
                        let link = side(FileMode::Link);
                        let result = index.as_mut().add_reuc(path, None, link, None);
                        REFUSING.store(false, SeqCst);
                        let count = index.as_ref().reuc_entry_count();
                        let handle = index.as_mut();
                        let record = handle.reuc_entry_by_path(path);
                        match result {
                            Err(error) => {
                                failures += 1;
                                assert_eq!(
                                    (error.class(), error.message()),
                                    (Some(ErrorClass::NoMemory), c"Out of memory")
                                );
                                assert_eq!(count, 8);
                                let kept = record.map(|r| r.modes());
                                let old = [ffi::GIT_FILEMODE_BLOB, 0, 0];
                                assert_eq!(kept, (!new_path).then_some(old));
                            }
                            Ok(()) => {
                                successes += 1;
                                assert_eq!(count, if new_path { 9 } else { 8 });
                                let modes = record.unwrap().modes();
                                assert_eq!(modes, [0, ffi::GIT_FILEMODE_LINK, 0]);
                            }
                        }
                        drop(index);
                        assert_eq!(LIVE.load(SeqCst), before, "{path:?}, allowance {allowed}");
                    }
                }
                // The record, then the growth, for either path.
                assert_eq!((failures, successes), (4, 2));
            },
        );
    }

    /// When libgit2 cannot be initialised, neither constructor calls it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn uninitialisable_library_is_reported() {
        isolated(
            concat!(module_path!(), "::uninitialisable_library_is_reported"),
            || {
                install_countdown();
                ALLOWED.store(0, SeqCst);
                REFUSING.store(true, SeqCst);
                for built in [
                    Index::new(),
                    Index::open(c"index"),
                    Index::with_oid_type(OidType::Sha256),
                    Index::open_with_oid_type(c"index", OidType::Sha256),
                ] {
                    let error = built.unwrap_err();
                    assert_eq!(
                        (error.class(), error.message()),
                        (
                            Some(ErrorClass::Invalid),
                            c"library has not been initialized"
                        )
                    );
                }
                assert!(IndexBox::new().is_none());
                assert!(IndexArc::new().is_none());
            },
        );
    }

    /// Writes an index file of version `version` with `names` at `file`, through C.
    fn write_index_file(file: &CStr, version: u32, names: &[&CStr]) {
        let index = Index::open(file).unwrap();
        for name in names {
            let mut entry = ffi::git_index_entry {
                mode: 0o100644,
                path: name.as_ptr(),
                ..Default::default()
            };
            entry.id.id[0] = 1;
            entry.id.type_ = ffi::GIT_OID_SHA1 as _;
            // SAFETY: the box's live index; `entry` and its path are read and copied for
            // the call.
            let rc = unsafe { ffi::git_index_add(index.as_c_ptr(), &entry) };
            assert_eq!(rc, 0);
        }
        // SAFETY: the box's live index.
        let rc = unsafe { ffi::git_index_set_version(index.as_c_ptr(), version) };
        assert_eq!(rc, 0);
        // SAFETY: the box's live index, backed by `file`.
        let rc = unsafe { ffi::git_index_write(index.as_c_ptr()) };
        assert_eq!(rc, 0);
    }

    /// A C defect `open` inherits: libgit2 leaks the decompressed path of a version 4
    /// entry that runs past the end of the file (`read_entry` returns before freeing
    /// `tmp_path`, `src/libgit2/index.c`); a version 2 file cut the same way leaks
    /// nothing. Counted through the allocator, with LeakSanitizer off in the child.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn version_4_entry_cut_short_leaks_its_path_in_c() {
        isolated_with(
            concat!(
                module_path!(),
                "::version_4_entry_cut_short_leaks_its_path_in_c"
            ),
            || {
                install_countdown();
                assert!(runtime_pinned());
                let dir = std::env::temp_dir()
                    .join(format!("crustify-index-unit-leak-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                let file = CString::new(dir.join("index").to_str().unwrap()).unwrap();
                for (version, leaked) in [(2, 0), (4, 1)] {
                    let _ = std::fs::remove_file(dir.join("index"));
                    write_index_file(&file, version, &[c"dir/a", c"dir/b"]);
                    let bytes = std::fs::read(dir.join("index")).unwrap();
                    std::fs::write(dir.join("index"), &bytes[..bytes.len() - 1]).unwrap();
                    // The thread's error state settles on the first failure.
                    assert!(Index::open(&file).is_err());
                    let before = LIVE.load(SeqCst);
                    assert!(Index::open(&file).is_err());
                    assert_eq!(LIVE.load(SeqCst) - before, leaked, "version {version}");
                }
                let _ = std::fs::remove_dir_all(&dir);
            },
            false,
        );
    }

    /// Every allocation `Index::new` and `Index::open` make can fail: each failure is
    /// reported as out of memory, with the half-built index freed (LeakSanitizer).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn allocation_failures_are_reported_and_released() {
        isolated(
            concat!(
                module_path!(),
                "::allocation_failures_are_reported_and_released"
            ),
            || {
                install_countdown();
                assert!(runtime_pinned());

                assert!(refuse_each_allocation(Index::new) > 0);
                assert!(refuse_each_allocation(|| Index::with_oid_type(OidType::Sha256)) > 0);

                let dir = std::env::temp_dir()
                    .join(format!("crustify-index-unit-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                let file = CString::new(dir.join("index").to_str().unwrap()).unwrap();
                let missing = refuse_each_allocation(|| Index::open(&file));
                assert!(missing > 0);

                write_index_file(&file, 2, &[c"a", c"b/c", c"d"]);
                assert!(refuse_each_allocation(|| Index::open(&file)) > missing);
                assert!(
                    refuse_each_allocation(|| Index::open_with_oid_type(&file, OidType::Sha1))
                        > missing
                );
                let _ = std::fs::remove_dir_all(&dir);
            },
        );
    }

    mod option_unit_tests {
        use super::{AddFlags, Capabilities, CapabilitySetting};
        use crate::ffi;

        #[test]
        fn scalar_layouts_and_traits() {
            assert_eq!(
                size_of::<AddFlags>(),
                size_of::<ffi::git_index_add_option_t>()
            );
            assert_eq!(
                align_of::<AddFlags>(),
                align_of::<ffi::git_index_add_option_t>()
            );
            assert_eq!(
                size_of::<Capabilities>(),
                size_of::<ffi::git_index_capability_t>()
            );
            assert_eq!(
                align_of::<Capabilities>(),
                align_of::<ffi::git_index_capability_t>()
            );
            assert!(!core::mem::needs_drop::<AddFlags>());
            assert!(!core::mem::needs_drop::<Capabilities>());
            assert_eq!(AddFlags::default(), AddFlags::empty());
            assert_eq!(Capabilities::default(), Capabilities::empty());
            assert_eq!(format!("{:?}", AddFlags::FORCE), "AddFlags(1)");
            assert_eq!(
                format!("{:?}", Capabilities::NO_SYMLINKS),
                "Capabilities(4)"
            );
            assert_eq!(format!("{:?}", CapabilitySetting::FromOwner), "FromOwner");
            let setting = CapabilitySetting::from(Capabilities::all());
            assert_eq!(setting, CapabilitySetting::Explicit(Capabilities::all()));
            // Exercise Clone explicitly: these are independent, resource-free scalars.
            #[allow(clippy::clone_on_copy)]
            {
                assert_eq!(AddFlags::FORCE.clone(), AddFlags::FORCE);
                assert_eq!(Capabilities::all().clone(), Capabilities::all());
                assert_eq!(setting.clone(), setting);
            }
        }

        #[test]
        fn capability_validation_rejects_commands_and_unknown_bits() {
            for bits in [8, 9, 15, i32::MAX, i32::MIN, -1, -2, -8] {
                assert_eq!(Capabilities::from_bits(bits), None);
            }
            for bits in 0..8 {
                let caps = Capabilities::from_bits(bits).unwrap();
                assert_eq!(caps.bits(), bits);
                assert_eq!(CapabilitySetting::from(caps).to_raw(), bits);
            }
        }
    }

    mod matched_path {
        use super::*;
        use std::ffi::CString;
        use std::panic::{AssertUnwindSafe, catch_unwind};

        #[test]
        fn stopped_trampoline_is_not_reentered_and_scan_resets() {
            let mut calls = 0;
            let mut f = |_: &CStr, _: Option<&CStr>| {
                calls += 1;
                MatchedPathResult::from_code(-999)
            };
            let mut callback = MatchedPathCallback::new(&mut f);
            for _ in 0..2 {
                callback.scan_with(|cb, payload| {
                    for _ in 0..3 {
                        assert_eq!(
                            // SAFETY: live exclusive payload from scan_with and static
                            // immutable NUL strings, invoked synchronously without reentry.
                            unsafe { cb.unwrap()(c"a".as_ptr(), ptr::null(), payload) },
                            -999
                        );
                    }
                });
            }
            drop(callback);
            assert_eq!(calls, 2);
        }

        /// Exercises actual C -> Rust payload writes, both C invokers, and cleanup
        /// after a callback panic. BSan targets this exact test, not the workspace.
        /// Its scans add files, whose first filter loads race in libgit2: it runs in
        /// a child process.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn foreign_scan_payload_and_panic_cleanup() {
            isolated(
                concat!(module_path!(), "::foreign_scan_payload_and_panic_cleanup"),
                || {
                    use crate::libgit2::repository::repository::Repository;
                    let dir = std::env::temp_dir()
                        .join(format!("crustify-matched-callback-{}", std::process::id()));
                    let _ = std::fs::remove_dir_all(&dir);
                    let path = CString::new(dir.to_str().unwrap()).unwrap();
                    let repo = Repository::init(&path, false).unwrap();
                    let index = repo.as_ref().index().unwrap();
                    std::fs::write(dir.join("file"), "first").unwrap();
                    let empty = ffi::git_strarray {
                        strings: ptr::null_mut(),
                        count: 0,
                    };
                    let mut seen = Vec::new();
                    let mut f = |path: &CStr, pattern: Option<&CStr>| {
                        seen.push((path.to_owned(), pattern.map(CStr::to_owned)));
                        MatchedPathResult::APPLY
                    };
                    let mut cb = MatchedPathCallback::new(&mut f);
                    assert_eq!(
                        // SAFETY: live repository-backed index used exclusively during the call;
                        // empty pathspec and callback payload are synchronous borrowed inputs.
                        cb.scan_with(|callback, payload| unsafe {
                            ffi::git_index_add_all(index.as_c_ptr(), &empty, 0, callback, payload)
                        }),
                        0
                    );
                    std::fs::write(dir.join("file"), "a different length").unwrap();
                    assert_eq!(
                        // SAFETY: same as above; neither index nor callback is reentered.
                        cb.scan_with(|callback, payload| unsafe {
                            ffi::git_index_update_all(index.as_c_ptr(), &empty, callback, payload)
                        }),
                        0
                    );
                    drop(cb);
                    assert_eq!(
                        seen,
                        [(c"file".to_owned(), None), (c"file".to_owned(), None)]
                    );

                    let mut f = |_: &CStr, _: Option<&CStr>| -> MatchedPathResult {
                        panic!("after C cleanup")
                    };
                    let mut cb = MatchedPathCallback::new(&mut f);
                    let mut returned_from_c = false;
                    let panic = catch_unwind(AssertUnwindSafe(|| {
                        cb.scan_with(|callback, payload| {
                            // SAFETY: live exclusively used index and synchronous callback pair.
                            let code = unsafe {
                                ffi::git_index_remove_all(
                                    index.as_c_ptr(),
                                    &empty,
                                    callback,
                                    payload,
                                )
                            };
                            returned_from_c = true;
                            assert_eq!(code, MatchedPathResult::ABORT.code());
                        })
                    }));
                    assert!(panic.is_err());
                    assert!(returned_from_c);
                    // SAFETY: the live index has not changed: the panic aborted before removal.
                    assert_eq!(unsafe { ffi::git_index_entrycount(index.as_c_ptr()) }, 1);
                    drop(index);
                    drop(repo);
                    std::fs::remove_dir_all(dir).unwrap();
                },
            );
        }

        /// In a child process, as its scans add files (and so load filters).
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn workdir_scan_skip_abort_and_removed_paths() {
            isolated(
                concat!(
                    module_path!(),
                    "::workdir_scan_skip_abort_and_removed_paths"
                ),
                || {
                    use crate::libgit2::repository::repository::Repository;
                    use std::cell::Cell;
                    let dir = std::env::temp_dir()
                        .join(format!("crustify-matched-workdir-{}", std::process::id()));
                    let _ = std::fs::remove_dir_all(&dir);
                    let repo =
                        Repository::init(&CString::new(dir.to_str().unwrap()).unwrap(), false)
                            .unwrap();
                    let index = repo.as_ref().index().unwrap();
                    std::fs::write(dir.join("file"), "contents").unwrap();
                    std::fs::write(dir.join("outside"), "unmatched").unwrap();
                    let mut pattern = c"file".as_ptr().cast_mut();
                    let paths = ffi::git_strarray {
                        strings: &mut pattern,
                        count: 1,
                    };
                    let result = Cell::new(MatchedPathResult::SKIP);
                    let mut f = |path: &CStr, pattern: Option<&CStr>| {
                        assert_eq!((path, pattern), (c"file", Some(c"file")));
                        result.get()
                    };
                    let mut cb = MatchedPathCallback::new(&mut f);
                    for (answer, expected_code, count) in [
                        (MatchedPathResult::SKIP, 0, 0),
                        (MatchedPathResult::from_code(-999), -999, 0),
                        (MatchedPathResult::APPLY, 0, 1),
                    ] {
                        result.set(answer);
                        // SAFETY: live exclusively used index and borrowed pathspec; scan_with
                        // supplies a synchronous callback pair that does not access the index.
                        let code = cb.scan_with(|callback, payload| unsafe {
                            ffi::git_index_add_all(index.as_c_ptr(), &paths, 0, callback, payload)
                        });
                        assert_eq!(code, expected_code);
                        assert_eq!(
                            // SAFETY: the live index is read only after the scan finishes.
                            unsafe { ffi::git_index_entrycount(index.as_c_ptr()) },
                            count
                        );
                    }
                    std::fs::remove_file(dir.join("file")).unwrap();
                    assert_eq!(
                        // SAFETY: same synchronous exclusive-index contract; C removes the missing
                        // workdir path only after the callback releases its borrowed path string.
                        cb.scan_with(|callback, payload| unsafe {
                            ffi::git_index_update_all(index.as_c_ptr(), &paths, callback, payload)
                        }),
                        0
                    );
                    // SAFETY: the live index is only read after the callback and scan return.
                    assert_eq!(unsafe { ffi::git_index_entrycount(index.as_c_ptr()) }, 0);
                    drop(index);
                    drop(repo);
                    std::fs::remove_dir_all(dir).unwrap();
                },
            );
        }

        /// libgit2 ignores an all-empty pathspec and drops `#` and line-break patterns;
        /// with none left, every path matches without one, while a usable pattern beside
        /// them still filters. An empty pattern beside others is never passed: libgit2
        /// reads past it (`pathspec::patterns_are_parsable`).
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn unusable_patterns_match_every_path_without_a_pattern() {
            let index = Index::new().unwrap();
            for path in [c"a", c"b"] {
                let mut entry = ffi::git_index_entry {
                    mode: 0o100644,
                    path: path.as_ptr(),
                    ..Default::default()
                };
                entry.id.type_ = ffi::GIT_OID_SHA1 as _;
                entry.id.id[0] = 1;
                // SAFETY: the fresh index is exclusively held and C copies this entry
                // and its immutable NUL path before returning.
                assert_eq!(unsafe { ffi::git_index_add(index.as_c_ptr(), &entry) }, 0);
            }
            for (patterns, expected) in [
                (vec![c""], vec![(c"a", None), (c"b", None)]),
                (vec![c"", c""], vec![(c"a", None), (c"b", None)]),
                (
                    vec![c"#a", c"\nb", c"\r\nb"],
                    vec![(c"a", None), (c"b", None)],
                ),
                (vec![c"#a", c"b"], vec![(c"b", Some(c"b"))]),
            ] {
                let mut patterns: Vec<_> = patterns
                    .into_iter()
                    .map(|p| p.as_ptr().cast_mut())
                    .collect();
                let paths = ffi::git_strarray {
                    strings: patterns.as_mut_ptr(),
                    count: patterns.len(),
                };
                let mut seen = Vec::new();
                let mut f = |path: &CStr, pattern: Option<&CStr>| {
                    seen.push((path.to_owned(), pattern.map(CStr::to_owned)));
                    MatchedPathResult::SKIP
                };
                let mut cb = MatchedPathCallback::new(&mut f);
                // SAFETY: live exclusively used index and borrowed pathspec; scan_with
                // supplies a synchronous callback pair that does not access the index.
                let code = cb.scan_with(|callback, payload| unsafe {
                    ffi::git_index_remove_all(index.as_c_ptr(), &paths, callback, payload)
                });
                assert_eq!(code, 0);
                drop(cb);
                let expected: Vec<_> = expected
                    .into_iter()
                    .map(|(path, pattern)| (path.to_owned(), pattern.map(CStr::to_owned)))
                    .collect();
                assert_eq!(seen, expected);
            }
            // SAFETY: the live index is only read after every scan returns.
            assert_eq!(unsafe { ffi::git_index_entrycount(index.as_c_ptr()) }, 2);
        }
    }

    /// An entry lent as a raw pointer, as an index lends its own: borrowed through
    /// `from_ptr`, read through the projected fields, copied out with its path, and
    /// handed back to C through `as_ptr` unchanged. NULL borrows nothing.
    #[test]
    fn entry_borrowed_from_a_raw_pointer() {
        // SAFETY: NULL is accepted by the contract and yields no handle.
        assert!(unsafe { IndexEntryRef::<'_, '_>::from_ptr(ptr::null()) }.is_none());
        let path = CString::new("dir/lent").unwrap();
        let mut raw = ffi::git_index_entry {
            ctime: ffi::git_index_time {
                seconds: -1,
                nanoseconds: u32::MAX,
            },
            dev: 1,
            ino: 2,
            mode: 0o100664,
            uid: 3,
            gid: 4,
            file_size: 5,
            flags: 0x3fff,
            flags_extended: u16::MAX,
            path: path.as_ptr(),
            ..Default::default()
        };
        raw.id.type_ = 7;
        raw.id.id[31] = 0xab;
        let mut copy = {
            // SAFETY: `raw` is initialised, outlives the handle and is not written while
            // it lives; its path is a live NUL-terminated string that outlives `copy`.
            let entry = unsafe { IndexEntryRef::from_ptr(&raw const raw) }.unwrap();
            assert_eq!(entry.as_ptr(), &raw const raw);
            assert_eq!(
                (entry.ctime().seconds(), entry.ctime().nanoseconds()),
                (-1, u32::MAX)
            );
            assert_eq!(
                (entry.mtime().seconds(), entry.mtime().nanoseconds()),
                (0, 0)
            );
            assert_eq!(
                (
                    entry.dev(),
                    entry.ino(),
                    entry.uid(),
                    entry.gid(),
                    entry.file_size()
                ),
                (1, 2, 3, 4, 5)
            );
            assert_eq!((entry.mode(), entry.filemode()), (0o100664, None));
            assert_eq!((entry.id().raw_kind(), entry.id().id()[31]), (7, 0xab));
            assert_eq!(entry.flags().bits(), 0x3fff);
            assert_eq!(entry.flags_extended().bits(), u32::from(u16::MAX));
            assert_eq!(entry.path(), path.as_c_str());
            assert!(format!("{entry:?}").contains("mode: 100664"));
            entry.to_index_entry()
        };
        copy.as_mut().set_dev(9);
        assert_eq!((raw.dev, copy.as_ref().dev()), (1, 9));
        assert_eq!(copy.into_raw().path, path.as_ptr());
    }

    /// The flag setters refuse a value wider than the 16-bit fields, leaving them as they
    /// were, and store every value that fits unchanged.
    #[test]
    fn entry_flag_setters_refuse_bits_above_sixteen() {
        let mut entry = IndexEntry::new(c"f", Oid::zeroed().as_ref(), FileMode::Blob);
        let mut handle = entry.as_mut();
        handle
            .set_flags(EntryFlags::from_bits_retain(0xffff))
            .unwrap();
        handle
            .set_flags_extended(EntryExtendedFlags::from_bits_retain(0x8001))
            .unwrap();
        for bits in [0x1_0000, 0x8000_0000, u32::MAX] {
            assert!(
                handle
                    .set_flags(EntryFlags::from_bits_retain(bits))
                    .is_err()
            );
            assert!(
                handle
                    .set_flags_extended(EntryExtendedFlags::from_bits_retain(bits))
                    .is_err()
            );
        }
        assert_eq!(handle.as_ref().flags().bits(), 0xffff);
        assert_eq!(handle.as_ref().flags_extended().bits(), 0x8001);
        let raw = entry.into_raw();
        assert_eq!((raw.flags, raw.flags_extended), (0xffff, 0x8001));
    }

    /// The constructor zeroes every field it is not given, and equality and the debug
    /// form follow every field, the path by contents.
    #[test]
    fn entry_constructor_equality_and_debug() {
        let mut id = Oid::zeroed();
        id.as_mut().set_kind(OidType::Sha256);
        let entry = IndexEntry::new(c"a/b", id.as_ref(), FileMode::Link);
        let raw = entry.into_raw();
        assert_eq!(raw.mode, ffi::GIT_FILEMODE_LINK);
        assert_eq!(raw.id.type_, ffi::GIT_OID_SHA256 as u8);
        let zero = ffi::git_index_entry::default();
        assert_eq!(
            (
                raw.dev,
                raw.ino,
                raw.uid,
                raw.gid,
                raw.file_size,
                raw.flags,
                raw.flags_extended
            ),
            (zero.dev, zero.ino, zero.uid, zero.gid, zero.file_size, 0, 0)
        );
        let other_path = CString::new("a/b").unwrap();
        let mut same = IndexEntry::new(&other_path, id.as_ref(), FileMode::Link);
        assert_eq!(entry, same);
        same.as_mut().ctime_mut().set_nanoseconds(1);
        assert_ne!(entry, same);
        let mut moved = entry;
        moved.as_mut().id_mut().set_kind(OidType::Sha1);
        assert_ne!(entry, moved);
        let debug = format!("{entry:?}");
        assert!(
            debug.starts_with("IndexEntry {") && debug.contains("\"a/b\""),
            "{debug}"
        );
    }

    /// The options' accessors and comparisons, on a value Rust builds itself (Miri cannot
    /// call `git_index_options_init`): unset reads as `None`, and only named types or 0
    /// are ever stored.
    #[test]
    fn options_accessors_store_named_types_or_zero() {
        let raw = ffi::git_index_options {
            version: ffi::GIT_INDEX_OPTIONS_VERSION,
            oid_type: 0,
        };
        let mut options = IndexOptions(raw, PhantomData);
        assert_eq!(options.as_ref().version(), ffi::GIT_INDEX_OPTIONS_VERSION);
        assert_eq!(options.as_ref().oid_type(), None);
        let unset = options;
        for kind in [Some(OidType::Sha256), Some(OidType::Sha1), None] {
            options.as_mut().set_oid_type(kind);
            assert_eq!(options.as_ref().oid_type(), kind);
            assert_eq!(options.into_raw().oid_type, kind.map_or(0, OidType::raw));
        }
        assert_eq!(options, unset);
        options.as_mut().set_oid_type(Some(OidType::Sha1));
        assert_ne!(options, unset);
        assert_eq!(
            format!("{options:?}"),
            "IndexOptions { version: 1, oid_type: Some(Sha1) }"
        );
    }

    /// The index and extension routines' internal paths: refusals before libgit2, the
    /// divergence from C's `git_index_add_all` without a repository, and allocation
    /// failures.
    mod symbols {
        use super::*;
        use crate::libgit2::core::strarray::StrArrayView;
        use crate::libgit2::repository::repository::{Repository, RepositoryBox};
        use std::path::PathBuf;

        /// A fresh repository with a working directory under the temporary directory,
        /// removed with it.
        struct Scratch {
            dir: PathBuf,
            repo: Option<RepositoryBox>,
        }

        impl Scratch {
            fn new(tag: &str) -> Self {
                let dir = std::env::temp_dir().join(format!(
                    "crustify-index-symbols-{}-{tag}",
                    std::process::id()
                ));
                let _ = std::fs::remove_dir_all(&dir);
                let path = CString::new(dir.to_str().unwrap()).unwrap();
                let repo = Some(Repository::init(&path, false).unwrap());
                Self { dir, repo }
            }

            /// The repository's index, with `body` given an exclusive handle over it.
            fn with_index(&self, body: impl FnOnce(IndexMut<'_>)) {
                let index = self.repo.as_ref().unwrap().as_ref().index().unwrap();
                // SAFETY: the arc's counted reference keeps the index live past the
                // handle, and nothing else reaches the index or its repository while
                // `body` runs: the test holds no other reference to either.
                let handle = unsafe { IndexMut::from_ptr(index.as_c_ptr()) }.unwrap();
                body(handle);
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                drop(self.repo.take());
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }

        #[test]
        fn extension_keys_terminate_the_signature() {
            assert_eq!(extension_key(b"ZOPT"), Some(*b"ZOPT\0"));
            assert_eq!(extension_key(b"\xff\x01a~"), Some(*b"\xff\x01a~\0"));
        }

        /// A NUL byte anywhere in a signature is refused by every extension routine,
        /// before libgit2 sees it, and changes nothing.
        #[test]
        #[cfg_attr(miri, ignore = "sets libgit2's error")]
        fn signatures_holding_nul_bytes_are_refused() {
            let mut index = Index::new().unwrap();
            index.as_mut().set_extension(b"ZOPT", b"kept").unwrap();
            for signature in [b"\0OPT", b"Z\0PT", b"ZO\0T", b"ZOP\0", b"\0\0\0\0"] {
                let errors = [
                    index.as_mut().extension(signature).unwrap_err(),
                    index.as_mut().set_extension(signature, b"x").unwrap_err(),
                    index.as_mut().remove_extension(signature).unwrap_err(),
                ];
                for error in errors {
                    assert_eq!(
                        (error.class(), error.message()),
                        (
                            Some(ErrorClass::Invalid),
                            c"an index extension signature cannot hold a NUL byte"
                        )
                    );
                }
            }
            let kept = index.as_mut().extension(b"ZOPT").unwrap().unwrap();
            assert_eq!(kept.as_slice(), b"kept");
        }

        /// Data the extension header cannot size is refused before libgit2, which
        /// asserts on it; the buffer is never written, so its pages stay unmapped.
        #[test]
        #[cfg_attr(miri, ignore = "allocates 4 GiB and sets libgit2's error")]
        fn extension_data_of_4_gib_is_refused() {
            let mut index = Index::new().unwrap();
            let huge = vec![0u8; u32::MAX as usize];
            let error = index.as_mut().set_extension(b"ZBIG", &huge).unwrap_err();
            assert_eq!(
                (error.class(), error.message()),
                (
                    Some(ErrorClass::Invalid),
                    c"index extension data cannot reach 4 GiB"
                )
            );
            assert!(index.as_mut().extension(b"ZBIG").unwrap().is_none());
        }

        /// A deliberate divergence: without a repository, `git_index_add_all` asks
        /// `git_repository_is_bare(NULL)`, whose argument check fails, and reports a bare
        /// repository; `add_all` refuses with the index error libgit2's own scan reports.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn add_all_without_a_repository_is_refused_where_c_reports_a_bare_one() {
            let mut index = Index::new().unwrap();
            let mut calls = 0;
            let mut f = |_: &CStr, _: Option<&CStr>| {
                calls += 1;
                MatchedPathResult::APPLY
            };
            let mut callback = MatchedPathCallback::new(&mut f);
            let pathspec = StrArrayView::default();
            for callback in [None, Some(&mut callback)] {
                let error = index
                    .as_mut()
                    .add_all(pathspec.as_ref(), AddFlags::DEFAULT, callback)
                    .unwrap_err();
                assert_eq!(
                    (error.code(), error.class(), error.message()),
                    (
                        Some(ErrorCode::Generic),
                        Some(ErrorClass::Index),
                        c"cannot add all; the index is not backed up by a repository"
                    )
                );
            }
            drop(callback);
            assert_eq!(calls, 0);

            // SAFETY: the box's live index, borrowed exclusively; an empty pathspec and
            // no callback.
            let rc = unsafe {
                ffi::git_index_add_all(
                    index.as_c_ptr(),
                    pathspec.as_ref().as_ptr(),
                    0,
                    None,
                    ptr::null_mut(),
                )
            };
            assert_eq!(rc, ErrorCode::BareRepo.raw());
            assert_eq!(
                last().message(),
                c"cannot index add all. This operation is not allowed against bare repositories."
            );

            // `add_path` is left to libgit2, which checks for the repository first.
            let error = index.as_mut().add_path(c"file").unwrap_err();
            assert_eq!(
                (error.code(), error.class(), error.message()),
                (
                    Some(ErrorCode::Generic),
                    Some(ErrorClass::Index),
                    c"could not initialize index entry. Index is not backed up by an existing repository."
                )
            );
        }

        /// A pathspec mixing empty and non-empty patterns is refused before libgit2
        /// parses it, once the index is known to have a repository, which C checks first.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn add_all_refuses_mixed_empty_patterns() {
            let scratch = Scratch::new("mixed");
            std::fs::write(scratch.dir.join("file"), "contents").unwrap();
            scratch.with_index(|mut index| {
                for patterns in [&[c"", c"file"][..], &[c"file", c""], &[c"a", c"", c"b"]] {
                    let pathspec = StrArrayView::new(patterns);
                    let error = index
                        .add_all(pathspec.as_ref(), AddFlags::DEFAULT, None)
                        .unwrap_err();
                    assert_eq!(
                        (error.code(), error.class()),
                        (Some(ErrorCode::Generic), Some(ErrorClass::Invalid))
                    );
                }
                assert_eq!(index.as_ref().entry_count(), 0);
            });
        }

        /// A directory whose submodule lookup cannot parse `.gitmodules` fails
        /// `add_path` with libgit2's configuration error, leaving the index unchanged.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn add_path_reports_a_failed_submodule_lookup() {
            let scratch = Scratch::new("submodule-lookup");
            std::fs::create_dir(scratch.dir.join("dir")).unwrap();
            std::fs::write(scratch.dir.join("dir/file"), "contents").unwrap();
            std::fs::write(scratch.dir.join(".gitmodules"), "[submodule").unwrap();
            scratch.with_index(|mut index| {
                let error = index.add_path(c"dir").unwrap_err();
                assert_eq!(error.class(), Some(ErrorClass::Config));
                assert_eq!(index.as_ref().entry_count(), 0);
            });
        }

        /// A deliberate divergence: libgit2's prefix search stops at whichever stage of
        /// an entry whose path is the prefix it meets first; `find_prefix` backs up to the
        /// first stage, as `find` and the header promise.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn find_prefix_backs_up_to_the_first_stage() {
            let mut index = Index::new().unwrap();
            for (path, stage) in [(c"a", 0u16), (c"a", 1), (c"a", 2), (c"a", 3), (c"ab", 0)] {
                let mut entry = ffi::git_index_entry {
                    mode: 0o100644,
                    path: path.as_ptr(),
                    // `GIT_INDEX_ENTRY_STAGESHIFT`, a header macro.
                    flags: stage << 12,
                    ..Default::default()
                };
                entry.id.id[0] = 1;
                entry.id.type_ = ffi::GIT_OID_SHA1 as _;
                // SAFETY: the box's live index, borrowed exclusively; `entry` and its
                // path are read and copied for the call.
                assert_eq!(unsafe { ffi::git_index_add(index.as_c_ptr(), &entry) }, 0);
            }
            let mut position = usize::MAX;
            // SAFETY: a writable slot, the box's live index and a NUL-terminated prefix.
            let rc = unsafe {
                ffi::git_index_find_prefix(&mut position, index.as_c_ptr(), c"a".as_ptr())
            };
            assert_eq!((rc, position), (0, 2));
            assert_eq!(index.as_mut().find_prefix(c"a"), Some(0));
            assert_eq!(index.as_mut().find(c"a"), Some(0));
            assert_eq!(index.as_mut().find_prefix(c"ab"), Some(4));
            assert_eq!(index.as_mut().find_prefix(c"a/"), None);
            assert_eq!(index.as_mut().find_prefix(c"b"), None);
        }

        /// A deliberate divergence: in an index that ignores case, libgit2 lands on the
        /// first entry in case-folded order and compares it byte for byte, missing a
        /// later match; `find_prefix` returns the first entry starting with the prefix.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn find_prefix_scans_case_insensitive_indexes() {
            let mut index = Index::new().unwrap();
            let caps = CapabilitySetting::Explicit(Capabilities::IGNORE_CASE);
            index.as_mut().set_caps(caps).unwrap();
            for (path, stage) in [(c"A/x", 0u16), (c"a/y", 0), (c"b", 2), (c"b", 3)] {
                let mut entry = ffi::git_index_entry {
                    mode: 0o100644,
                    path: path.as_ptr(),
                    // `GIT_INDEX_ENTRY_STAGESHIFT`, a header macro.
                    flags: stage << 12,
                    ..Default::default()
                };
                entry.id.id[0] = 1;
                entry.id.type_ = ffi::GIT_OID_SHA1 as _;
                // SAFETY: the box's live index, borrowed exclusively; `entry` and its
                // path are read and copied for the call.
                assert_eq!(unsafe { ffi::git_index_add(index.as_c_ptr(), &entry) }, 0);
            }
            let mut position = usize::MAX;
            // SAFETY: a writable slot, the box's live index and a NUL-terminated prefix.
            let rc = unsafe {
                ffi::git_index_find_prefix(&mut position, index.as_c_ptr(), c"a/".as_ptr())
            };
            assert_eq!((rc, position), (ErrorCode::NotFound.raw(), usize::MAX));
            assert_eq!(index.as_mut().find_prefix(c"a/"), Some(1));
            assert_eq!(index.as_mut().find_prefix(c"A/"), Some(0));
            assert_eq!(index.as_mut().find_prefix(c"a/y"), Some(1));
            assert_eq!(index.as_mut().find_prefix(c""), Some(0));
            assert_eq!(index.as_mut().find_prefix(c"b"), Some(2));
            assert_eq!(index.as_mut().find_prefix(c"B"), None);
            assert_eq!(index.as_mut().find_prefix(c"c"), None);
        }

        /// Every allocation the extension routines make can fail: the lookup reports it
        /// and leaks nothing; so does setting an extension, except where libgit2 runs out
        /// of memory growing its list after allocating the extension, which it leaks
        /// (`git_index_extension_add`, `src/libgit2/index.c`). Counted through the
        /// allocator, with LeakSanitizer off in the child for that leak.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn extension_allocation_failures() {
            isolated_with(
                concat!(module_path!(), "::extension_allocation_failures"),
                || {
                    install_countdown();
                    assert!(runtime_pinned());
                    set(ErrorClass::Invalid, c"error state allocated");
                    let mut index = Index::new().unwrap();
                    index.as_mut().set_extension(b"ZONE", b"one").unwrap();

                    // The lookup's one allocation, the copy.
                    let before = LIVE.load(SeqCst);
                    ALLOWED.store(0, SeqCst);
                    REFUSING.store(true, SeqCst);
                    let error = index.as_mut().extension(b"ZONE").unwrap_err();
                    REFUSING.store(false, SeqCst);
                    assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                    assert_eq!(LIVE.load(SeqCst), before);

                    // Replacing allocates the extension alone.
                    ALLOWED.store(0, SeqCst);
                    REFUSING.store(true, SeqCst);
                    let error = index.as_mut().set_extension(b"ZONE", b"two").unwrap_err();
                    REFUSING.store(false, SeqCst);
                    assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                    assert_eq!(LIVE.load(SeqCst), before);
                    let kept = index.as_mut().extension(b"ZONE").unwrap().unwrap();
                    assert_eq!(kept.as_slice(), b"one");
                    drop(kept);

                    // A cleared index has no list left, so a new extension grows one.
                    index.as_mut().clear().unwrap();
                    let before = LIVE.load(SeqCst);
                    let mut leaked = Vec::new();
                    for allowed in 0..=2 {
                        ALLOWED.store(allowed, SeqCst);
                        REFUSING.store(true, SeqCst);
                        let result = index.as_mut().set_extension(b"ZNEW", b"new");
                        REFUSING.store(false, SeqCst);
                        let live = LIVE.load(SeqCst) - before;
                        match result {
                            Err(error) => {
                                assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                                assert!(index.as_mut().extension(b"ZNEW").unwrap().is_none());
                                leaked.push(live);
                            }
                            Ok(()) => {
                                // The extension and the list, beside what failed
                                // attempts leaked.
                                assert_eq!(live, 2 + leaked.iter().sum::<isize>());
                                break;
                            }
                        }
                    }
                    assert_eq!(leaked, [0, 1]);
                    drop(index);
                },
                false,
            );
        }

        /// The routines that only free never fail on a sole owner's index, even with
        /// every allocation refused.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn removals_do_not_allocate() {
            isolated(
                concat!(module_path!(), "::removals_do_not_allocate"),
                || {
                    install_countdown();
                    set(ErrorClass::Invalid, c"error state allocated");
                    let mut index = Index::new().unwrap();
                    index.as_mut().set_extension(b"ZONE", b"one").unwrap();
                    index
                        .as_mut()
                        .add_name(Some(c"a"), Some(c"b"), None)
                        .unwrap();
                    let before = LIVE.load(SeqCst);
                    ALLOWED.store(0, SeqCst);
                    REFUSING.store(true, SeqCst);
                    assert_eq!(index.as_mut().remove_extension(b"ZONE"), Ok(true));
                    assert_eq!(index.as_mut().remove_extension(b"ZONE"), Ok(false));
                    index.as_mut().clear_conflicts().unwrap();
                    index.as_mut().clear().unwrap();
                    assert_eq!(index.as_mut().find(c"a"), None);
                    assert_eq!(index.as_mut().find_prefix(c""), None);
                    REFUSING.store(false, SeqCst);
                    assert!(LIVE.load(SeqCst) < before);
                    assert_eq!(index.as_ref().name_entry_count(), 0);
                },
            );
        }

        /// What the entry getters read off entries libgit2 fills itself, as their docs
        /// say: `dev` holds the file's `st_rdev` (0 for a regular file), not the device
        /// holding it, and `git_index_read_tree` keeps a tree's raw mode, even one with
        /// no [`FileMode`] (git's legacy `0100664`). In a child: adding by path loads the
        /// filters.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn entry_getters_read_what_libgit2_stores() {
            isolated(
                concat!(module_path!(), "::entry_getters_read_what_libgit2_stores"),
                || {
                    use crate::libgit2::object::tree::Tree;
                    use std::os::unix::fs::MetadataExt;

                    let scratch = Scratch::new("stored");
                    let file = scratch.dir.join("file");
                    std::fs::write(&file, "contents").unwrap();
                    let stat = std::fs::metadata(&file).unwrap();
                    assert_ne!(stat.dev(), stat.rdev());
                    scratch.with_index(|mut index| {
                        index.add_path(c"file").unwrap();
                        // SAFETY: the exclusive handle's live index, whose one entry is
                        // read here and neither changed nor released meanwhile.
                        let entry = unsafe {
                            IndexEntryRef::from_ptr(ffi::git_index_get_byindex(
                                index.as_mut_ptr(),
                                0,
                            ))
                        }
                        .unwrap();
                        assert_eq!(entry.path(), c"file");
                        assert_eq!(
                            (entry.dev(), entry.ino()),
                            (stat.rdev() as u32, stat.ino() as u32)
                        );
                        assert_eq!(entry.dev(), 0);
                        assert_eq!(entry.filemode(), Some(FileMode::Blob));
                    });

                    let repo = scratch.repo.as_ref().unwrap();
                    let odb = repo.as_ref().odb().unwrap();
                    let data = [&b"100664 legacy\0"[..], &[0x11; 20]].concat();
                    let mut id = Oid::zeroed();
                    // SAFETY: a live odb, a writable id, and `data` read for the call.
                    let rc = unsafe {
                        ffi::git_odb_write(
                            id.as_mut().as_mut_ptr(),
                            odb.as_c_ptr(),
                            data.as_ptr().cast(),
                            data.len(),
                            ffi::GIT_OBJECT_TREE,
                        )
                    };
                    assert_eq!(rc, 0);
                    let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
                    let index = Index::new().unwrap();
                    // SAFETY: the box's live index, which nothing else reaches, and a live
                    // tree, whose entries libgit2 copies for the call.
                    let rc = unsafe { ffi::git_index_read_tree(index.as_c_ptr(), tree.as_c_ptr()) };
                    assert_eq!(rc, 0);
                    // SAFETY: as above, for the index's one entry.
                    let entry = unsafe {
                        IndexEntryRef::from_ptr(ffi::git_index_get_byindex(index.as_c_ptr(), 0))
                    }
                    .unwrap();
                    assert_eq!(entry.path(), c"legacy");
                    assert_eq!((entry.mode(), entry.filemode()), (0o100664, None));
                    assert_eq!(entry.id().id()[..20], [0x11; 20]);
                },
            );
        }
    }

    #[test]
    fn value_clone_and_borrowed_projection() {
        let mut time = IndexTime::default();
        let independent = time.clone();
        assert_eq!(independent.as_ref().seconds(), 0);
        assert_eq!(independent.as_ref().nanoseconds(), 0);
        time.as_mut().set_seconds(i32::MIN);
        assert_eq!(independent.as_ref().seconds(), 0);
        let mut raw = ffi::git_index_time {
            seconds: -17,
            nanoseconds: u32::MAX,
        };
        // SAFETY: raw is initialized, aligned, and exclusively borrowed until handle
        // expires; it owns no resources and neither field has restricted validity.
        let mut handle = unsafe { IndexTimeMut::from_ptr(&raw mut raw) }.unwrap();
        assert_eq!(handle.as_ref().seconds(), -17);
        handle.as_mut().set_nanoseconds(5);
        assert_eq!(handle.as_ref().nanoseconds(), 5);
        assert_eq!(size_of::<IndexTime>(), size_of::<ffi::git_index_time>());
        assert_eq!(align_of::<IndexTime>(), align_of::<ffi::git_index_time>());
    }
}
