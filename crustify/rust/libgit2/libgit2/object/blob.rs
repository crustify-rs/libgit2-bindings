//! Safe wrappers over `include/git2/blob.h`.
//!
//! # Blobs
//!
//! A [`Blob`] is a `git_blob`: the content of a file, as a repository's object
//! database stores it. libgit2 publishes no field. The struct starts with the
//! `git_object` every parsed object does, so a blob is an
//! [`Object`](crate::libgit2::object::object::Object) of type blob, and shares its
//! ownership: one atomic count, which `git_blob_dup` (`git_object_dup`) increments on
//! the same pointer and `git_blob_free` (`git_object_free`) decrements, freeing the
//! blob, and the reference it holds to the object database's copy of the content, at
//! zero. The repository's cache keeps no blob by default, so a lookup usually parses a
//! fresh one its caller alone holds; with a cache limit set for blobs
//! (`GIT_OPT_SET_CACHE_OBJECT_LIMIT`) it shares them as it does every other object. So
//! the Rust owner is a [`BlobArc`], one counted reference that never proves itself the
//! only one, and lends the shared handle [`BlobRef`] only.
//!
//! A blob keeps, uncounted, the repository it was looked up in, which
//! `git_blob_owner` and `git_blob_filter` reach: it carries the lifetime `'r` of that
//! repository's borrow, as an object does, and is neither `Send` nor `Sync`.
//! [`Blob::lookup`] and [`Blob::lookup_prefix`] build one, refusing an id of another
//! type than the repository's as [`Object::lookup`](crate::libgit2::object::object::Object::lookup)
//! does; [`Blob::from_object`] and [`BlobRef::from_object`] take an object of type blob
//! as one, and [`Blob::into_object`] and [`BlobRef::as_object`] give a blob back as an
//! object.
//!
//! # Filter options
//!
//! A [`FilterOptions`] is a `git_blob_filter_options`, the options of `git_blob_filter`:
//! a structure version, the [`FilterFlags`] it applies, a deprecated `reserved`
//! pointer, and the id of the commit whose `.gitattributes`
//! [`FilterFlag::AttributesFromCommit`] loads. It is a plain value the caller owns:
//! libgit2 never allocates or frees one, and `git_blob_filter` refuses a version of 0
//! or above [`FilterOptions::VERSION`] with `GIT_ERROR_INVALID`, then copies it and
//! reads its flags and commit id for the call. [`FilterOptions::new`] fills one
//! through `git_blob_filter_options_init`, as `GIT_BLOB_FILTER_OPTIONS_INIT` does: the
//! current version, [`FilterFlags::OPTIONS_DEFAULT`], no `reserved` pointer and the
//! zero id. All-zero is not that default (libgit2 refuses version 0), so there is no
//! `zeroed`.
//!
//! libgit2 is built with `GIT_DEPRECATE_HARD`, under which `reserved` is a `void *`
//! that no routine reads (without it, the field is the `git_oid *commit_id` that
//! `git_blob_filter` would read instead of `attr_commit_id`). Rust never reads through
//! it either, and lets safe code only clear it.
//!
//! # Filter flags
//!
//! A [`FilterFlag`] is a `git_blob_filter_flag_t`, one bit of the `uint32_t` `flags`
//! of `git_blob_filter_options`; a [`FilterFlags`] set ORs them. `git_blob_filter`
//! tests the four bits the header names, one at a time, and ignores every other, so a
//! set keeps any bit it is given ([`FilterFlags::from_bits_retain`]) and passes it
//! through unchanged.
//!
//! The empty set is [`FilterFlags`]'s `Default`, as for every flag set; the options'
//! own default (`GIT_BLOB_FILTER_OPTIONS_INIT`, and `git_blob_filter_options_init`) is
//! [`FilterFlags::OPTIONS_DEFAULT`], which holds [`FilterFlag::CheckForBinary`].
//!
//! # Binary content
//!
//! [`data_is_binary`] applies the binary-content heuristic `git_blob_is_binary` uses
//! (core git's) to a byte slice. libgit2 reads the bytes for the call only and touches
//! neither its allocator nor the thread's error state, so the check needs no
//! initialised runtime and is safe from any thread.

use core::ffi::c_int;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{BitOr, BitOrAssign};
use core::ptr::{self, NonNull, addr_of, addr_of_mut};

use ffibox::{CArc, CBorrowedPtr, CCell, CDrop, CRefClone, define_ctype};

use crate::ffi;
use crate::libgit2::object::object::{ObjectArc, ObjectRef, id_fits};
use crate::libgit2::oid::oid::{OidMut, OidRef};
use crate::libgit2::repository::repository::{RepositoryError, RepositoryRef};

/// One option of `git_blob_filter`, a single bit of a [`FilterFlags`] set
/// (`git_blob_filter_flag_t`).
///
/// [`from_raw`](Self::from_raw) gives `None` for any value without a variant: `0`,
/// a combination of bits, or a bit this version of libgit2's header does not name.
///
/// Wraps: git_blob_filter_flag_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum FilterFlag {
    /// Leave a binary blob unfiltered: `git_blob_filter` succeeds without writing its
    /// output when the blob's content looks binary.
    CheckForBinary = ffi::GIT_BLOB_FILTER_CHECK_FOR_BINARY,
    /// Do not load the system-wide `gitattributes` (in `/etc`, or the system's
    /// equivalent).
    NoSystemAttributes = ffi::GIT_BLOB_FILTER_NO_SYSTEM_ATTRIBUTES,
    /// Also load the `.gitattributes` file of the `HEAD` commit.
    AttributesFromHead = ffi::GIT_BLOB_FILTER_ATTRIBUTES_FROM_HEAD,
    /// Also load the `.gitattributes` file of the commit the options'
    /// `attr_commit_id` names.
    AttributesFromCommit = ffi::GIT_BLOB_FILTER_ATTRIBUTES_FROM_COMMIT,
}

impl FilterFlag {
    /// Every variant, in ascending bit order.
    pub const ALL: [FilterFlag; 4] = [
        Self::CheckForBinary,
        Self::NoSystemAttributes,
        Self::AttributesFromHead,
        Self::AttributesFromCommit,
    ];

    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_blob_filter_flag_t) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i] as ffi::git_blob_filter_flag_t == raw {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }

    /// The C value, a single `git_blob_filter_flag_t` bit.
    pub const fn raw(self) -> ffi::git_blob_filter_flag_t {
        self as ffi::git_blob_filter_flag_t
    }

    /// The flag's bit in `git_blob_filter_options.flags`.
    const fn bit(self) -> u32 {
        self as u32
    }
}

impl TryFrom<ffi::git_blob_filter_flag_t> for FilterFlag {
    type Error = UnnamedFilterFlag;

    fn try_from(raw: ffi::git_blob_filter_flag_t) -> Result<Self, UnnamedFilterFlag> {
        Self::from_raw(raw).ok_or(UnnamedFilterFlag(raw))
    }
}

impl From<FilterFlag> for ffi::git_blob_filter_flag_t {
    fn from(value: FilterFlag) -> ffi::git_blob_filter_flag_t {
        value.raw()
    }
}

// A `FilterFlag` passes where C takes a `git_blob_filter_flag_t`.
const _: () = assert!(
    size_of::<FilterFlag>() == size_of::<ffi::git_blob_filter_flag_t>()
        && align_of::<FilterFlag>() == align_of::<ffi::git_blob_filter_flag_t>()
);

/// A blob filter flag value that this version of libgit2's header does not name: `0`,
/// a combination of flags, or a later header's bit. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedFilterFlag(ffi::git_blob_filter_flag_t);

impl UnnamedFilterFlag {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_blob_filter_flag_t {
        self.0
    }
}

impl fmt::Display for UnnamedFilterFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 blob filter flag {:#x} is not named by git2/blob.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedFilterFlag {}

/// A set of [`FilterFlag`]s: the `uint32_t` combination of `GIT_BLOB_FILTER_*` bits
/// that `git_blob_filter_options.flags` holds. The default is the empty set;
/// [`OPTIONS_DEFAULT`](Self::OPTIONS_DEFAULT) is the options' default.
///
/// It keeps every bit it is given, named or not, as the field does (`git_blob_filter`
/// ignores the bits it does not name), so [`bits`](Self::bits) returns what was
/// stored; [`from_bits_truncate`](Self::from_bits_truncate) keeps the named ones only.
/// [`iter`](Self::iter) and [`contains`](Self::contains) see the named flags, and
/// [`unnamed_bits`](Self::unnamed_bits) the rest.
///
/// Wraps: git_blob_filter_flag_t
/// Handwritten
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FilterFlags(u32);

impl FilterFlags {
    /// The bits of every [`FilterFlag`].
    const NAMED: u32 = {
        let mut bits = 0;
        let mut i = 0;
        while i < FilterFlag::ALL.len() {
            bits |= FilterFlag::ALL[i].bit();
            i += 1;
        }
        bits
    };

    /// The flags `GIT_BLOB_FILTER_OPTIONS_INIT` and `git_blob_filter_options_init`
    /// set: [`FilterFlag::CheckForBinary`] alone.
    pub const OPTIONS_DEFAULT: Self = Self(FilterFlag::CheckForBinary.bit());

    /// The set with no flag: binary blobs are filtered too, and only the system and
    /// working-directory attributes are loaded.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The set of every bit of `bits`, as `git_blob_filter_options.flags` holds them;
    /// bits that no [`FilterFlag`] names are kept.
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// The set of the bits of `bits` that a [`FilterFlag`] names; the rest are dropped.
    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & Self::NAMED)
    }

    /// The bits of the set, as `git_blob_filter_options.flags` holds them.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The bits of the set that no [`FilterFlag`] names.
    pub const fn unnamed_bits(self) -> u32 {
        self.0 & !Self::NAMED
    }

    /// Whether the set has no bit, named or not.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `flag` is in the set.
    pub const fn contains(self, flag: FilterFlag) -> bool {
        self.0 & flag.bit() != 0
    }

    /// Adds `flag` to the set.
    pub fn insert(&mut self, flag: FilterFlag) {
        self.0 |= flag.bit();
    }

    /// Removes `flag` from the set.
    pub fn remove(&mut self, flag: FilterFlag) {
        self.0 &= !flag.bit();
    }

    /// The named flags in the set, in ascending bit order.
    pub fn iter(self) -> FilterFlagsIter {
        FilterFlagsIter { set: self, next: 0 }
    }
}

impl From<FilterFlag> for FilterFlags {
    fn from(flag: FilterFlag) -> Self {
        Self(flag.bit())
    }
}

impl BitOr for FilterFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<FilterFlag> for FilterFlags {
    type Output = Self;

    fn bitor(self, rhs: FilterFlag) -> Self {
        self | Self::from(rhs)
    }
}

impl BitOr for FilterFlag {
    type Output = FilterFlags;

    fn bitor(self, rhs: Self) -> FilterFlags {
        FilterFlags::from(self) | rhs
    }
}

impl BitOrAssign for FilterFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitOrAssign<FilterFlag> for FilterFlags {
    fn bitor_assign(&mut self, rhs: FilterFlag) {
        self.insert(rhs);
    }
}

impl FromIterator<FilterFlag> for FilterFlags {
    fn from_iter<I: IntoIterator<Item = FilterFlag>>(iter: I) -> Self {
        let mut set = Self::empty();
        set.extend(iter);
        set
    }
}

impl Extend<FilterFlag> for FilterFlags {
    fn extend<I: IntoIterator<Item = FilterFlag>>(&mut self, iter: I) {
        for flag in iter {
            self.insert(flag);
        }
    }
}

impl IntoIterator for FilterFlags {
    type Item = FilterFlag;
    type IntoIter = FilterFlagsIter;

    fn into_iter(self) -> FilterFlagsIter {
        self.iter()
    }
}

/// Lists the named flags, then any unnamed bits in hex:
/// `{CheckForBinary | AttributesFromHead, 0x80}`.
impl fmt::Debug for FilterFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FilterFlags {")?;
        let mut sep = "";
        for flag in self.iter() {
            write!(f, "{sep}{flag:?}")?;
            sep = " | ";
        }
        if self.unnamed_bits() != 0 {
            let sep = if sep.is_empty() { "" } else { ", " };
            write!(f, "{sep}{:#x}", self.unnamed_bits())?;
        }
        f.write_str("}")
    }
}

/// The named flags of a [`FilterFlags`] set, in ascending bit order
/// ([`FilterFlags::iter`]).
///
/// Handwritten
#[derive(Clone, Debug)]
pub struct FilterFlagsIter {
    set: FilterFlags,
    /// The index into [`FilterFlag::ALL`] of the next flag to test.
    next: usize,
}

impl Iterator for FilterFlagsIter {
    type Item = FilterFlag;

    fn next(&mut self) -> Option<FilterFlag> {
        while let Some(&flag) = FilterFlag::ALL.get(self.next) {
            self.next += 1;
            if self.set.contains(flag) {
                return Some(flag);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(FilterFlag::ALL.len().saturating_sub(self.next)))
    }
}

impl core::iter::FusedIterator for FilterFlagsIter {}

/// The content of a file as a repository stores it (`git_blob`): an
/// [`Object`](crate::libgit2::object::object::Object) of type blob.
///
/// libgit2 allocates, counts and frees it, so it is only ever reached through a
/// [`BlobArc`] and its handles, built by [`lookup`](Self::lookup) and
/// [`lookup_prefix`](Self::lookup_prefix), or taken from an object of type blob by
/// [`from_object`](Self::from_object). It is opaque: no field is published.
///
/// `'r` is the borrow of the repository the blob was looked up in, which it keeps (see
/// the [module docs](self#blobs)). The lifetime only shrinks:
///
/// ```compile_fail
/// use libgit2::libgit2::object::blob::BlobArc;
///
/// fn widen<'r>(blob: BlobArc<'r>) -> BlobArc<'static> {
///     blob
/// }
/// ```
///
/// and a blob stays on the thread that looked it up:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::blob::BlobArc;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(blob: BlobArc<'static>) {
///     send(blob)
/// }
/// ```
///
/// Wraps: git_blob
/// Handwritten
#[repr(transparent)]
pub struct Blob<'r>(ffi::git_blob, PhantomData<(*const (), &'r ())>);

/// Shared borrow of a [`Blob`]. `Copy`, like `&T`.
///
/// It stays on the thread of the blob's repository, as the owner does:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::blob::BlobRef;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(handle: BlobRef<'static, 'static>) {
///     send(handle)
/// }
/// ```
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct BlobRef<'a, 'r>(CBorrowedPtr<'a, Blob<'r>>);

/// Exclusive borrow of a [`Blob`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// No owner hands one out: a blob may be shared with its repository's cache.
///
/// Handwritten
#[repr(transparent)]
pub struct BlobMut<'a, 'r>(
    BlobRef<'a, 'r>,
    // Invariant in the blob, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Blob<'r>>,
);

// SAFETY: `Blob` is `#[repr(transparent)]` over `git_blob` with a zero-sized marker;
// both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Blob<'r>>` (`BlobMut`
// through `BlobRef`) with zero-sized markers and no `Drop`, hand out no reference to
// the C object, and `BlobRef` has no write operation. `BlobMut` is invariant in the
// blob through its `PhantomData<&'a mut Blob<'r>>`.
unsafe impl<'r> CCell for Blob<'r> {
    type C = ffi::git_blob;
    type Ref<'a>
        = BlobRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = BlobMut<'a, 'r>
    where
        Self: 'a;
}

impl<'a, 'r> BlobRef<'a, 'r> {
    /// Borrows a raw pointer; `None` if null.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live blob looked up in a repository that
    /// outlives `'r`, which stays live during `'a` (a reference to it is held for at
    /// least that long) and which no one writes during `'a`.
    pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_blob) -> Option<Self> {
        // SAFETY: a non-null pointer the caller vouches for, per this function's
        // contract; `Blob` is transparent over `git_blob`.
        NonNull::new(ptr.cast_mut()).map(|p| BlobRef(unsafe { CBorrowedPtr::new(p.cast()) }))
    }

    /// Read-only pointer to the C blob.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_blob {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The blob `object` is, borrowed as long as `object` is; `None` for an object of
    /// another type.
    pub fn from_object(object: ObjectRef<'a, 'r>) -> Option<Self> {
        let p = object.as_ptr();
        // SAFETY: the handle addresses a live object; `git_object_type` reads the type
        // it was parsed as, which never changes.
        if unsafe { ffi::git_object_type(p) } != ffi::GIT_OBJECT_BLOB {
            return None;
        }
        // SAFETY: an object of type blob was allocated and parsed as a `git_blob`,
        // whose first member is that `git_object`, so the pointer addresses the whole
        // blob; the object handle keeps it live and unwritten for `'a`, and its
        // repository outlives `'r`.
        unsafe { Self::from_ptr(p.cast()) }
    }

    /// The object this blob is, borrowed as long as the blob is.
    pub fn as_object(&self) -> ObjectRef<'a, 'r> {
        // SAFETY: a `git_blob` starts with its `git_object`, live and unwritten for
        // `'a` as the blob is, of a repository that outlives `'r`.
        unsafe { ObjectRef::from_ptr(self.as_ptr().cast()) }.expect("a handle is never NULL")
    }

    /// Another counted reference to the blob, as `git_blob_dup` takes one: the same
    /// blob, now also kept alive by the result, which may outlive this borrow but not
    /// the repository.
    ///
    /// Wraps: git_blob_dup
    pub fn to_blob(&self) -> BlobArc<'r> {
        let blob = self.0.as_non_null();
        // SAFETY: the handle addresses a live blob for `'a`, whose reference its holder
        // keeps meanwhile; one more is counted on it here.
        if !unsafe { BlobFree.c_up_ref(blob) } {
            unreachable!("git_blob_dup cannot fail");
        }
        // SAFETY: the reference just counted, given to the result, which `git_blob_free`
        // releases; the blob's repository outlives `'r`, which the result carries.
        unsafe { BlobArc::from_raw(blob.as_ptr()) }.expect("a handle is never NULL")
    }
}

impl<'a, 'r> BlobMut<'a, 'r> {
    /// Borrows a raw pointer exclusively; `None` if null.
    ///
    /// # Safety
    ///
    /// As [`BlobRef::from_ptr`], plus: nothing else reaches the blob while the result
    /// lives, its repository's cache included.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_blob) -> Option<Self> {
        // SAFETY: the caller upholds `BlobRef::from_ptr`'s contract and exclusivity.
        unsafe { BlobRef::from_ptr(ptr) }.map(|r| BlobMut(r, PhantomData))
    }

    /// Writable pointer to the C blob.
    #[allow(dead_code)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_blob {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> BlobRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> BlobMut<'_, 'r> {
        BlobMut(self.0, PhantomData)
    }
}

impl fmt::Debug for BlobRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blob").finish_non_exhaustive()
    }
}

impl fmt::Debug for BlobMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases one counted reference to a blob through `git_blob_free`, and takes another
/// through `git_blob_dup`.
///
/// Both are the object routines on the blob's `git_object`: the free decrements its
/// atomic count and, at zero, gives back the blob's reference to the object
/// database's copy of its content and frees the struct, never reading the repository;
/// the copy increments the same count and hands back the same pointer. Neither can
/// fail, and neither allocates.
///
/// Wraps: git_blob_free
/// Wraps: git_blob_dup
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct BlobFree;

// SAFETY: `git_blob_free` gives back exactly one counted reference of a live blob, in
// every state the owners reach (a NULL pointer, which it skips, never reaches it); the
// blob outlives the call through that reference, and is freed only once the count,
// which a cache's reference is part of, reaches zero.
unsafe impl<'r> CDrop<Blob<'r>> for BlobFree {
    unsafe fn c_drop(&self, ptr: NonNull<Blob<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live blob of which the
        // caller gives up one reference; `Blob` is transparent over `git_blob`.
        unsafe { ffi::git_blob_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `git_blob_dup` atomically increments the count of the live blob it is given
// and stores that same pointer in `out`, returning 0: the blob then owes one more
// `git_blob_free`, which this policy performs, and `ptr` stays valid. It never fails.
// `c_is_sole_owner` keeps its `false` default: a cache that keeps blobs counts a
// reference no owner sees, and a later lookup may take another.
unsafe impl<'r> CRefClone<Blob<'r>> for BlobFree {
    unsafe fn c_up_ref(&self, ptr: NonNull<Blob<'r>>) -> bool {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `ptr` addresses a live blob whose reference
        // the caller holds, per `c_up_ref`'s contract.
        let rc = unsafe { ffi::git_blob_dup(&mut out, ptr.as_ptr().cast()) };
        debug_assert_eq!(out, ptr.as_ptr().cast());
        rc == 0
    }
}

/// One counted reference to a [`Blob`], given back by `git_blob_free` on drop; `clone`
/// takes another through `git_blob_dup`, on the same blob.
///
/// Its handles borrow it, so none outlives it:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::object::blob::{BlobArc, BlobRef};
///
/// fn escape<'r>(blob: BlobArc<'r>) -> BlobRef<'static, 'r> {
///     let r = blob.as_ref();
///     drop(blob);
///     r
/// }
/// ```
///
/// and it lends no exclusive handle, which needs the reference proved sole:
///
/// ```
/// # if cfg!(miri) { return; }
/// # use libgit2::libgit2::object::blob::Blob;
/// # use libgit2::libgit2::oid::oid::{Oid, OidType};
/// # use libgit2::libgit2::repository::repository::Repository;
/// # let _init = libgit2::libgit2::core::global::init().unwrap();
/// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/resources/testrepo.git\0");
/// # let repo = Repository::open(std::ffi::CStr::from_bytes_with_nul(path.as_bytes()).unwrap()).unwrap();
/// # let id = Oid::from_string(c"a8233120f6ad708f843d861ce2b7228ec4e3dec6", OidType::Sha1).unwrap();
/// let mut blob = Blob::lookup(repo.as_ref(), id.as_ref()).unwrap();
/// assert!(blob.get_mut().is_none());
/// ```
pub type BlobArc<'r> = CArc<Blob<'r>, BlobFree>;

impl<'r> Blob<'r> {
    /// The blob `id` names in `repo`: [`Object::lookup`] for type blob.
    ///
    /// libgit2 reads it from the object database and parses it afresh, or, when the
    /// cache keeps blobs and holds this one, answers with the blob it holds.
    ///
    /// Fails with [`ErrorCode::NotFound`] when the object database has no such object
    /// (the zero id included), when the object is not a blob (class
    /// `GIT_ERROR_INVALID`), and for a repository without a git directory
    /// (`GIT_ERROR_INVALID`, "path cannot exist in repository"); and with libgit2's
    /// error when the object database cannot be loaded or read.
    ///
    /// Refuses, with code `-1` and libgit2's `GIT_ERROR_INVALID` error "specified
    /// object ID type does not match repository object ID type", an `id` whose type is
    /// not `repo`'s, which libgit2 would format out of bounds, as [`Object::lookup`]
    /// does.
    ///
    /// The blob keeps the repository it was looked up in, so it lives no longer than
    /// this borrow of it:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::object::blob::Blob;
    /// # use libgit2::libgit2::oid::oid::{Oid, OidType};
    /// # use libgit2::libgit2::repository::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let blob = Blob::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// drop(repo);
    /// drop(blob);
    /// ```
    ///
    /// [`Object::lookup`]: crate::libgit2::object::object::Object::lookup
    /// [`ErrorCode::NotFound`]: crate::libgit2::util::errors::ErrorCode::NotFound
    ///
    /// Wraps: git_blob_lookup
    pub fn lookup(repo: RepositoryRef<'r>, id: OidRef<'_>) -> Result<BlobArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `repo` is a live
        // repository on this thread, whose odb libgit2 loads and whose cache it reads
        // and fills through this shared handle, as `RepositoryRef` allows; `id` is a
        // live id of the repository's type (checked above), read for the call only.
        let rc = unsafe { ffi::git_blob_lookup(&mut out, repo.as_ptr().cast_mut(), id.as_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live blob of
        // `repo`, borrowed for `'r`, given to the result and released by
        // `git_blob_free`.
        let blob = unsafe { BlobArc::from_c(out) };
        Ok(blob.expect("a lookup succeeded without a blob"))
    }

    /// The blob whose id starts with the first `len` hex digits of `id`, in `repo`:
    /// [`Object::lookup_prefix`] for type blob. A `len` past the repository's full
    /// length means the full id, which is [`lookup`](Self::lookup).
    ///
    /// The digits past `len` are ignored, and every object in the database is a
    /// candidate, whatever its type: the prefix must name exactly one, which must then
    /// be a blob.
    ///
    /// Fails with [`ErrorCode::Ambiguous`] when `len` is under 4 or the prefix names
    /// several objects, with [`ErrorCode::NotFound`] when it names none or one that is
    /// not a blob, and as [`lookup`](Self::lookup) otherwise; an `id` of another type
    /// than `repo`'s is refused as it is, whatever `len`.
    ///
    /// [`Object::lookup_prefix`]: crate::libgit2::object::object::Object::lookup_prefix
    /// [`ErrorCode::Ambiguous`]: crate::libgit2::util::errors::ErrorCode::Ambiguous
    /// [`ErrorCode::NotFound`]: crate::libgit2::util::errors::ErrorCode::NotFound
    ///
    /// Wraps: git_blob_lookup_prefix
    pub fn lookup_prefix(
        repo: RepositoryRef<'r>,
        id: OidRef<'_>,
        len: usize,
    ) -> Result<BlobArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: as in `lookup`; libgit2 clamps `len` to the repository's length, and
        // copies at most `(len + 1) / 2` of `id`'s `GIT_OID_MAX_SIZE` bytes, with its
        // type, into the key it searches with.
        let rc = unsafe {
            ffi::git_blob_lookup_prefix(&mut out, repo.as_ptr().cast_mut(), id.as_ptr(), len)
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live blob of
        // `repo`, borrowed for `'r`, given to the result and released by
        // `git_blob_free`.
        let blob = unsafe { BlobArc::from_c(out) };
        Ok(blob.expect("a lookup succeeded without a blob"))
    }

    /// The blob an object of type blob is, owning the reference `object` held; the
    /// object, untouched, when it is of another type.
    pub fn from_object(object: ObjectArc<'r>) -> Result<BlobArc<'r>, ObjectArc<'r>> {
        if BlobRef::from_object(object.as_ref()).is_none() {
            return Err(object);
        }
        let raw = object.into_c();
        // SAFETY: the counted reference `object` held, to an object of type blob, which
        // is a `git_blob` starting with that `git_object`; `git_blob_free` gives it back
        // as `git_object_free` would. Its repository outlives `'r`.
        let blob = unsafe { BlobArc::from_c(raw.cast()) };
        Ok(blob.expect("an owner is never NULL"))
    }

    /// The object `blob` is, owning the reference `blob` held.
    pub fn into_object(blob: BlobArc<'r>) -> ObjectArc<'r> {
        let raw = blob.into_c();
        // SAFETY: the counted reference `blob` held, to a `git_blob`, which starts with
        // its `git_object`; `git_object_free` gives it back as `git_blob_free` would.
        // Its repository outlives `'r`.
        let object = unsafe { ObjectArc::from_c(raw.cast()) };
        object.expect("an owner is never NULL")
    }
}

// A `Blob` is a `git_blob`, never held by value; the owner is one pointer.
const _: () = assert!(
    size_of::<Blob<'static>>() == size_of::<ffi::git_blob>()
        && size_of::<BlobArc<'static>>() == size_of::<*mut ffi::git_blob>()
        && size_of::<Option<BlobArc<'static>>>() == size_of::<*mut ffi::git_blob>()
);

define_ctype!(
    /// The options of `git_blob_filter` (`git_blob_filter_options`): a structure
    /// version, the [`FilterFlags`] to apply, a deprecated `reserved` pointer, and the
    /// commit whose `.gitattributes` [`FilterFlag::AttributesFromCommit`] loads.
    ///
    /// A plain value (see the [module docs](self#filter-options)): `Copy`, held
    /// inline, never released. Build one with [`new`](Self::new), the header's
    /// defaults; read it through [`as_ref`](Self::as_ref) and write it through
    /// [`as_mut`](Self::as_mut). It compares and hashes field by field, `reserved` by
    /// whether it is set.
    ///
    /// A handle cannot outlive the options:
    ///
    /// ```compile_fail,E0597
    /// use libgit2::libgit2::object::blob::FilterOptions;
    ///
    /// let handle = {
    ///     let opts = FilterOptions::new();
    ///     opts.as_ref()
    /// };
    /// let _ = handle.flags();
    /// ```
    ///
    /// nor coexist with an exclusive one:
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::object::blob::{FilterFlags, FilterOptions};
    ///
    /// let mut opts = FilterOptions::new();
    /// let mut m = opts.as_mut();
    /// let r = opts.as_ref();
    /// m.set_flags(FilterFlags::empty());
    /// let _ = r.flags();
    /// ```
    ///
    /// and the commit id it lends lives no longer than the handle's borrow:
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::object::blob::{FilterFlags, FilterOptions};
    ///
    /// let mut opts = FilterOptions::new();
    /// let id = opts.as_ref().attr_commit_id();
    /// opts.as_mut().attr_commit_id_mut().set_raw_kind(1);
    /// let _ = id.raw_kind();
    /// ```
    ///
    /// Wraps: git_blob_filter_options
    #[derive(Clone, Copy)]
    FilterOptions, FilterOptionsRef, FilterOptionsMut, ffi::git_blob_filter_options
);

// SAFETY: a `FilterOptions` is a version, flags, an id and the `reserved` pointer,
// which nothing dereferences: no libgit2 routine reads it under `GIT_DEPRECATE_HARD`,
// and Rust only tests it against NULL or clears it. So the value has no interior
// mutability, no thread affinity and nothing to release, and moving one to another
// thread moves bytes.
unsafe impl Send for FilterOptions {}
// SAFETY: `FilterOptionsMut` is an exclusive borrow of such a value, as
// `&mut FilterOptions`.
unsafe impl Send for FilterOptionsMut<'_> {}

// SAFETY: shared access only reads the fields (every write goes through
// `FilterOptionsMut`, which excludes other handles), so concurrent readers race
// nothing.
unsafe impl Sync for FilterOptions {}
// SAFETY: as above; `FilterOptionsRef` is a shared borrow, as `&FilterOptions`.
unsafe impl Send for FilterOptionsRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for FilterOptionsRef<'_> {}
// SAFETY: `&FilterOptionsMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for FilterOptionsMut<'_> {}

impl FilterOptions {
    /// The structure version this crate's header declares
    /// (`GIT_BLOB_FILTER_OPTIONS_VERSION`), the only one libgit2 accepts.
    pub const VERSION: c_int = ffi::GIT_BLOB_FILTER_OPTIONS_VERSION as c_int;

    /// The default options, as `git_blob_filter_options_init` fills them for
    /// [`VERSION`](Self::VERSION) and `GIT_BLOB_FILTER_OPTIONS_INIT` spells them: the
    /// current version, [`FilterFlags::OPTIONS_DEFAULT`] (binary blobs are left
    /// unfiltered), no `reserved` pointer and the zero id, of no type. Needs no
    /// initialised libgit2.
    ///
    /// Wraps: git_blob_filter_options_init
    pub fn new() -> FilterOptions {
        let mut raw = MaybeUninit::<ffi::git_blob_filter_options>::uninit();
        // SAFETY: `raw` is writable storage for one `git_blob_filter_options`. With the
        // header's own version the routine's version check passes, so it only copies
        // the `GIT_BLOB_FILTER_OPTIONS_INIT` template over all of `raw`: no allocation
        // and no error state, so no initialised runtime is needed.
        let rc = unsafe {
            ffi::git_blob_filter_options_init(
                raw.as_mut_ptr(),
                ffi::GIT_BLOB_FILTER_OPTIONS_VERSION,
            )
        };
        assert_eq!(
            rc, 0,
            "git_blob_filter_options_init refused its own version"
        );
        // SAFETY: on success the template was copied over the whole struct, padding
        // aside, so every field is initialised.
        FilterOptions(unsafe { raw.assume_init() }, PhantomData)
    }
}

impl Default for FilterOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The `git_blob_filter_options` value as it is, for code that already holds one by
/// value. Its `reserved` pointer is kept, and never read.
impl From<ffi::git_blob_filter_options> for FilterOptions {
    fn from(raw: ffi::git_blob_filter_options) -> FilterOptions {
        FilterOptions(raw, PhantomData)
    }
}

/// The `git_blob_filter_options` value, for code that passes one by value.
impl From<FilterOptions> for ffi::git_blob_filter_options {
    fn from(opts: FilterOptions) -> ffi::git_blob_filter_options {
        opts.0
    }
}

impl<'a> FilterOptionsRef<'a> {
    /// The structure version. [`FilterOptions::new`] sets [`FilterOptions::VERSION`];
    /// libgit2 reads the field as an `unsigned int`, and refuses the options with
    /// `GIT_ERROR_INVALID` if it is 0 or above that, so a negative version is refused
    /// too.
    ///
    /// Field: git_blob_filter_options.version
    pub fn version(&self) -> c_int {
        // SAFETY: a read through the shared handle's pointer to live options; any value
        // is a valid `int`.
        unsafe { addr_of!((*self.as_ptr()).version).read() }
    }

    /// The filter flags, every stored bit kept.
    ///
    /// Field: git_blob_filter_options.flags
    pub fn flags(&self) -> FilterFlags {
        // SAFETY: a read through the shared handle's pointer to live options; any value
        // is a valid `uint32_t`.
        let bits = unsafe { addr_of!((*self.as_ptr()).flags).read() };
        FilterFlags::from_bits_retain(bits)
    }

    /// Whether the deprecated `reserved` pointer is set. libgit2 never reads it (see
    /// the [module docs](self#filter-options)); [`FilterOptions::new`] leaves it
    /// unset, and only a value converted from a raw `git_blob_filter_options` can
    /// carry one.
    ///
    /// Field: git_blob_filter_options.reserved
    pub fn has_reserved(&self) -> bool {
        // SAFETY: a read of the pointer itself through the shared handle's pointer to
        // live options; it is compared with NULL and never dereferenced.
        !unsafe { addr_of!((*self.as_ptr()).reserved).read() }.is_null()
    }

    /// The id of the commit whose `.gitattributes` `git_blob_filter` loads when the
    /// flags hold [`FilterFlag::AttributesFromCommit`], borrowed in place for as long
    /// as the options are. [`FilterOptions::new`] leaves the zero id, of no type.
    ///
    /// Field: git_blob_filter_options.attr_commit_id
    pub fn attr_commit_id(&self) -> OidRef<'a> {
        let p = self.as_ptr().cast_mut();
        // SAFETY: the projection stays inside the live options this handle borrows.
        let id = unsafe { addr_of_mut!((*p).attr_commit_id) };
        // SAFETY: an embedded, initialised `git_oid` that lives as long as the options,
        // which the shared handle keeps borrowed (and so unwritten) for `'a`. The
        // pointer is not NULL.
        unsafe { OidRef::from_ptr(id) }.expect("an embedded id is never NULL")
    }

    /// A copy of the options, as a value of its own.
    pub fn to_options(&self) -> FilterOptions {
        // SAFETY: a read of the whole struct through the shared handle's pointer to
        // live, initialised options; it is plain data (the `reserved` pointer is copied
        // as a value, not followed).
        FilterOptions(unsafe { self.as_ptr().read() }, PhantomData)
    }
}

impl fmt::Debug for FilterOptionsRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterOptions")
            .field("version", &self.version())
            .field("flags", &self.flags())
            .field("has_reserved", &self.has_reserved())
            .field("attr_commit_id", &self.attr_commit_id())
            .finish()
    }
}

impl fmt::Debug for FilterOptionsMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for FilterOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'b> PartialEq<FilterOptionsRef<'b>> for FilterOptionsRef<'_> {
    /// Field by field, every flag bit and all 33 bytes of the id included; `reserved`
    /// by whether it is set.
    fn eq(&self, other: &FilterOptionsRef<'b>) -> bool {
        // The id's type byte and raw bytes, as `git_oid_equal` compares them, in Rust.
        let (id, other_id) = (self.attr_commit_id(), other.attr_commit_id());
        self.version() == other.version()
            && self.flags() == other.flags()
            && self.has_reserved() == other.has_reserved()
            && id.raw_kind() == other_id.raw_kind()
            && id.id() == other_id.id()
    }
}

impl Eq for FilterOptionsRef<'_> {}

impl Hash for FilterOptionsRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.version().hash(state);
        self.flags().hash(state);
        self.has_reserved().hash(state);
        self.attr_commit_id().hash(state);
    }
}

impl PartialEq for FilterOptions {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for FilterOptions {}

impl Hash for FilterOptions {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

impl<'a> FilterOptionsMut<'a> {
    /// Replaces the structure version. Any value is accepted; libgit2 refuses options
    /// whose version, read as an `unsigned int`, is 0 or above
    /// [`FilterOptions::VERSION`].
    ///
    /// Field: git_blob_filter_options.version
    pub fn set_version(&mut self, version: c_int) {
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is a plain integer.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).version).write(version) }
    }

    /// Replaces the filter flags, every bit as given.
    ///
    /// Field: git_blob_filter_options.flags
    pub fn set_flags(&mut self, flags: FilterFlags) {
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is a plain integer.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).flags).write(flags.bits()) }
    }

    /// Unsets the deprecated `reserved` pointer, as [`FilterOptions::new`] leaves it.
    /// Nothing it pointed at is released: the options never own it.
    ///
    /// Field: git_blob_filter_options.reserved
    pub fn clear_reserved(&mut self) {
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // old pointer is not owned, so overwriting it releases nothing.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).reserved).write(ptr::null_mut()) }
    }

    /// The commit id, borrowed exclusively in place, for writing it field by field.
    ///
    /// Field: git_blob_filter_options.attr_commit_id
    pub fn attr_commit_id_mut(&mut self) -> OidMut<'_> {
        let p = self.as_mut_ptr();
        // SAFETY: the projection stays inside the live options this handle borrows.
        let id = unsafe { addr_of_mut!((*p).attr_commit_id) };
        // SAFETY: an embedded, initialised `git_oid` that lives as long as the options,
        // borrowed exclusively through this handle, frozen while the result lives. The
        // pointer is not NULL.
        unsafe { OidMut::from_ptr(id) }.expect("an embedded id is never NULL")
    }

    /// Replaces the commit id with a copy of `id`, its type included.
    ///
    /// Field: git_blob_filter_options.attr_commit_id
    pub fn set_attr_commit_id(&mut self, id: OidRef<'_>) {
        // SAFETY: `id` is a shared handle to a live `git_oid`, read whole; it cannot be
        // this field, which the exclusive handle borrows. The write goes through the
        // exclusive handle's pointer to live options, over a plain-data field.
        unsafe {
            let id = id.as_ptr().read();
            addr_of_mut!((*self.as_mut_ptr()).attr_commit_id).write(id);
        }
    }
}

// A `FilterOptions` is a `git_blob_filter_options`, so C reads a value Rust holds in
// place.
const _: () = assert!(
    size_of::<FilterOptions>() == size_of::<ffi::git_blob_filter_options>()
        && align_of::<FilterOptions>() == align_of::<ffi::git_blob_filter_options>()
);

/// Whether `data` is most likely binary, by core git's heuristic: it is binary when it
/// opens with a UTF-16 or UTF-32 byte-order mark, when it holds a NUL byte, or when
/// fewer than 128 printable bytes come for each non-printable one. Printable bytes are
/// those above `0x1f` but DEL (space included), and backspace, escape and form feed;
/// tab, line feed, vertical tab and carriage return count as neither; every other byte
/// below `0x20`, and DEL, is non-printable. A UTF-8 byte-order mark is skipped, and
/// empty data is text.
///
/// Unlike `git_blob_is_binary`, it reads the whole slice, not its first 8000 bytes.
///
/// libgit2 counts printable and non-printable bytes in C `int`s, which overflow (UB)
/// once a slice holds more than `INT_MAX` of either. A slice that long never reaches it:
/// the same heuristic runs in Rust, with counters as wide as the slice's length.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::object::blob::data_is_binary;
///
/// assert!(!data_is_binary(b"hello, world\n"));
/// assert!(data_is_binary(b"hello\0world"));
/// assert!(data_is_binary(b"\xff\xfeh\0i\0"));
/// assert!(!data_is_binary(b""));
/// ```
///
/// Wraps: git_blob_data_is_binary
pub fn data_is_binary(data: &[u8]) -> bool {
    if !fits_c_counters(data.len()) {
        return wide::is_binary(data);
    }
    // SAFETY: `data` is live for `data.len()` bytes, read for the call only and never
    // retained or written; with a length of 0 libgit2 does not read the pointer at all.
    // The length is at most `INT_MAX`, so neither of libgit2's `int` counters, each
    // bounded by it, overflows. No allocation or error state is touched, so no runtime
    // pin is needed.
    let rc = unsafe { ffi::git_blob_data_is_binary(data.as_ptr().cast(), data.len()) };
    rc != 0
}

/// Whether libgit2's `int` counters can count a slice of `len` bytes: each counts a
/// subset of its bytes, so at most `INT_MAX` of them never overflows either.
fn fits_c_counters(len: usize) -> bool {
    len <= c_int::MAX as usize
}

/// `git_str_is_binary` (`src/util/str.c`), over slices longer than its `int` counters
/// can count.
mod wide {
    /// The bytes a byte-order mark at the start of `data` takes, and whether it marks
    /// UTF-16 or UTF-32 (which make the data binary), as `git_str_detect_bom` reads it.
    pub(super) fn bom(data: &[u8]) -> (usize, bool) {
        match data {
            [0, 0, 0xfe, 0xff, ..] => (4, true),
            [0xef, 0xbb, 0xbf, ..] => (3, false),
            [0xfe, 0xff, ..] => (2, true),
            [0xff, 0xfe, 0, 0, ..] => (4, true),
            [0xff, 0xfe, ..] => (2, true),
            _ => (0, false),
        }
    }

    /// [`super::data_is_binary`]'s heuristic, with `usize` counters.
    pub(super) fn is_binary(data: &[u8]) -> bool {
        let (skip, wide_bom) = bom(data);
        if wide_bom {
            return true;
        }
        let (mut printable, mut nonprintable) = (0usize, 0usize);
        for &c in &data[skip..] {
            if (c > 0x1f && c != 0x7f) || matches!(c, 0x08 | 0x1b | 0x0c) {
                printable += 1;
            } else if c == 0 {
                return true;
            } else if !matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
                nonprintable += 1;
            }
        }
        (printable >> 7) < nonprintable
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::object::object::Object;
    use crate::libgit2::oid::oid::{Oid, OidType};
    use crate::libgit2::repository::repository::{Repository, RepositoryBox};
    use crate::libgit2::util::errors::ErrorClass;
    use crate::libgit2::util::sys::errors::runtime_pinned;
    use core::ffi::CStr;
    use ffibox::CZeroable;

    /// The bytes of a `git_oid`'s id.
    const MAX_OID: usize = ffi::GIT_OID_MAX_SIZE as usize;

    /// Values probed for `from_raw`: zero, each bit, their neighbours and combinations.
    fn probes() -> impl Iterator<Item = u32> {
        (0..=64u32).chain((0..32).map(|s| 1u32 << s)).chain([
            u32::MAX,
            u32::MAX - 1,
            0x8000_0000,
            0x0f,
            0x10,
        ])
    }

    #[test]
    fn from_raw_round_trips_each_named_flag() {
        for flag in FilterFlag::ALL {
            assert_eq!(FilterFlag::from_raw(flag.raw()), Some(flag));
            assert_eq!(FilterFlag::try_from(flag.raw()), Ok(flag));
            assert_eq!(ffi::git_blob_filter_flag_t::from(flag), flag.raw());
            assert_eq!(flag.raw().count_ones(), 1, "{flag:?}");
        }
        let named = probes()
            .filter(|&r| FilterFlag::from_raw(r).is_some())
            .count();
        // Each named bit appears twice in `probes`: in the range and as a shift.
        assert_eq!(named, 2 * FilterFlag::ALL.len());
    }

    #[test]
    fn unnamed_values_are_refused_with_their_value() {
        for raw in probes().filter(|&r| FilterFlag::from_raw(r).is_none()) {
            let err = FilterFlag::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(
                err.to_string(),
                format!("libgit2 blob filter flag {raw:#x} is not named by git2/blob.h")
            );
        }
    }

    #[test]
    fn set_operations() {
        let mut set = FilterFlags::empty();
        assert!(set.is_empty() && set == FilterFlags::default());
        assert_eq!(set.iter().next(), None);
        set.insert(FilterFlag::AttributesFromHead);
        set |= FilterFlag::CheckForBinary;
        assert_eq!(set.bits(), 0b101);
        assert!(set.contains(FilterFlag::CheckForBinary));
        assert!(!set.contains(FilterFlag::NoSystemAttributes));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            [FilterFlag::CheckForBinary, FilterFlag::AttributesFromHead]
        );
        set.remove(FilterFlag::CheckForBinary);
        set.remove(FilterFlag::CheckForBinary);
        assert_eq!(set, FilterFlag::AttributesFromHead.into());
        set |= FilterFlags::from_bits_retain(0x8000_0000);
        assert_eq!(set.unnamed_bits(), 0x8000_0000);
        assert!(!set.is_empty());
        assert_eq!(
            FilterFlags::from_bits_truncate(set.bits()),
            FilterFlag::AttributesFromHead.into()
        );
        let all: FilterFlags = FilterFlag::ALL.into_iter().collect();
        assert_eq!(all.bits(), 0b1111);
        assert_eq!(all, FilterFlags::from_bits_truncate(u32::MAX));
        assert_eq!(all.unnamed_bits(), 0);
        assert_eq!(
            FilterFlag::NoSystemAttributes | FilterFlag::AttributesFromCommit,
            FilterFlags::from_bits_retain(0b1010)
        );
        let mut ext = FilterFlags::from(FilterFlag::CheckForBinary);
        ext.extend([FilterFlag::AttributesFromCommit]);
        ext |= FilterFlags::from(FilterFlag::NoSystemAttributes);
        assert_eq!(ext.into_iter().count(), 3);
        assert_eq!(
            FilterFlags::OPTIONS_DEFAULT,
            FilterFlag::CheckForBinary.into()
        );
    }

    #[test]
    fn iter_is_fused_and_bounded() {
        let mut it = FilterFlags::from_bits_retain(u32::MAX).iter();
        assert_eq!(it.size_hint(), (0, Some(4)));
        assert_eq!(it.by_ref().count(), 4);
        assert_eq!(it.size_hint(), (0, Some(0)));
        assert_eq!(it.next(), None);
        assert_eq!(it.clone().next(), None);
    }

    #[test]
    fn debug_lists_named_then_unnamed_bits() {
        assert_eq!(format!("{:?}", FilterFlags::empty()), "FilterFlags {}");
        assert_eq!(
            format!(
                "{:?}",
                FilterFlag::CheckForBinary | FilterFlag::AttributesFromHead
            ),
            "FilterFlags {CheckForBinary | AttributesFromHead}"
        );
        assert_eq!(
            format!("{:?}", FilterFlags::from_bits_retain(0x81)),
            "FilterFlags {CheckForBinary, 0x80}"
        );
        assert_eq!(
            format!("{:?}", FilterFlags::from_bits_retain(0x80)),
            "FilterFlags {0x80}"
        );
    }

    /// Inputs around every branch of the heuristic: each byte-order mark and its
    /// truncations, NULs before and after one, each class of byte alone, and the
    /// printable-to-non-printable ratio on both sides of 128.
    fn corpus() -> Vec<Vec<u8>> {
        let mut inputs: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"a".to_vec(),
            b"\0".to_vec(),
            b"\xef\xbb\xbf".to_vec(),
            b"\xef\xbb\xbfplain".to_vec(),
            b"\xef\xbb\xbf\0".to_vec(),
            b"\xef\xbb".to_vec(),
            b"\xfe\xff".to_vec(),
            b"\xfe\xfe".to_vec(),
            b"\xff\xfe".to_vec(),
            b"\xff\xfe\0".to_vec(),
            b"\xff\xfe\0\0".to_vec(),
            b"\xff\xfe\0a".to_vec(),
            b"\0\0\xfe\xff".to_vec(),
            b"\0\0\xfe".to_vec(),
            b"\0\0\xfe\xfe".to_vec(),
            b"\xff\xff".to_vec(),
            b"line\r\n\tindented\x0b\x0c".to_vec(),
            b"\x08\x1b\x0c".to_vec(),
            b"\x7f".to_vec(),
            b"\x01".to_vec(),
            b"\x80\xff\xa0".to_vec(),
        ];
        for (printable, nonprintable) in [(127, 1), (128, 1), (255, 1), (256, 2), (256, 3)] {
            let mut ratio = vec![b'x'; printable];
            ratio.extend(std::iter::repeat_n(0x01, nonprintable));
            inputs.push(ratio);
        }
        for ws in [b' ', b'\t', b'\n', 0x0b, 0x0c, b'\r', 0x08, 0x1b] {
            let mut run = vec![ws; 128];
            run.push(0x01);
            inputs.push(run);
        }
        inputs.extend((0..=u8::MAX).map(|c| vec![c]));
        inputs.extend((0..=u8::MAX).map(|c| vec![b'a', c, b'b']));
        inputs
    }

    /// `git_blob_data_is_binary` itself, as the C reference.
    fn c_is_binary(data: &[u8]) -> bool {
        // SAFETY: `data` is live for `data.len()` bytes, at most `INT_MAX`.
        unsafe { ffi::git_blob_data_is_binary(data.as_ptr().cast(), data.len()) != 0 }
    }

    /// The Rust heuristic that stands in for libgit2 on oversized slices agrees with it
    /// on every input libgit2 can take.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn wide_heuristic_matches_libgit2() {
        for input in corpus() {
            assert_eq!(wide::is_binary(&input), c_is_binary(&input), "{input:x?}");
            assert_eq!(data_is_binary(&input), c_is_binary(&input), "{input:x?}");
        }
    }

    /// The byte-order marks `git_str_detect_bom` recognises, and the bytes each takes.
    #[test]
    fn byte_order_marks() {
        assert_eq!(wide::bom(b"\0\0\xfe\xffx"), (4, true));
        assert_eq!(wide::bom(b"\xef\xbb\xbfx"), (3, false));
        assert_eq!(wide::bom(b"\xfe\xffx"), (2, true));
        assert_eq!(wide::bom(b"\xff\xfe\0\0x"), (4, true));
        assert_eq!(wide::bom(b"\xff\xfe\0x"), (2, true));
        assert_eq!(wide::bom(b"\xef\xbb"), (0, false));
        assert_eq!(wide::bom(b"\xff"), (0, false));
        assert_eq!(wide::bom(b""), (0, false));
    }

    /// The Rust heuristic's verdicts on each branch, without libgit2, so Miri checks
    /// its slicing past a byte-order mark too.
    #[test]
    fn wide_heuristic_verdicts() {
        for (data, binary) in [
            (&b""[..], false),
            (b"\xef\xbb\xbf", false),
            (b"\xef\xbb\xbf\0", true),
            (b"\xff\xfe", true),
            (b"\0\0\xfe\xff", true),
            (b"text\r\n\t\x0b\x0c\x08\x1b", false),
            (b"\x01", true),
            (b"\x7f", true),
            (b"\x80\xff", false),
        ] {
            assert_eq!(wide::is_binary(data), binary, "{data:x?}");
        }
        let mut ratio = vec![b'x'; 256];
        ratio.push(0x01);
        assert!(!wide::is_binary(&ratio));
        ratio.push(0x01);
        assert!(!wide::is_binary(&ratio));
        ratio.push(0x01);
        assert!(wide::is_binary(&ratio));
    }

    /// Space, form feed, backspace and escape are printable: 128 of them carry one
    /// control byte. Tab, line feed, vertical tab and carriage return count as neither,
    /// so the same control byte makes the data binary. The Rust heuristic runs under
    /// Miri too; the end-to-end check reaches libgit2.
    #[test]
    fn whitespace_classes() {
        for (ws, binary) in [
            (b' ', false),
            (0x0c, false),
            (0x08, false),
            (0x1b, false),
            (b'\t', true),
            (b'\n', true),
            (0x0b, true),
            (b'\r', true),
        ] {
            let mut run = vec![ws; 128];
            run.push(0x01);
            assert_eq!(wide::is_binary(&run), binary, "{ws:#x}");
            if !cfg!(miri) {
                assert_eq!(data_is_binary(&run), binary, "{ws:#x}");
            }
        }
    }

    /// Slices up to `INT_MAX` bytes reach libgit2; one byte more takes the Rust
    /// heuristic.
    #[test]
    fn longest_slice_for_libgit2_is_int_max() {
        assert!(fits_c_counters(0));
        assert!(fits_c_counters(c_int::MAX as usize));
        assert!(!fits_c_counters(c_int::MAX as usize + 1));
        assert!(!fits_c_counters(usize::MAX));
    }

    /// A slice one byte longer than libgit2's counters can count takes the Rust
    /// heuristic. The slice is zeroed (never touched by `vec!` past the allocator's
    /// own zero-filling, and binary at its first byte), so the test costs one
    /// allocation, not a 2 GiB scan.
    #[test]
    #[cfg_attr(miri, ignore = "allocates 2 GiB")]
    fn oversized_slice_takes_the_rust_heuristic() {
        let mut data = vec![0u8; c_int::MAX as usize + 1];
        assert!(data_is_binary(&data));
        data[..2].copy_from_slice(b"\xff\xfe");
        assert!(data_is_binary(&data));
        data[..3].copy_from_slice(b"\xef\xbb\xbf");
        assert!(data_is_binary(&data));
    }

    /// End to end, a slice whose printable bytes would overflow libgit2's `int` counter
    /// (`INT_MAX + 1` of them) is counted in Rust. It scans 2 GiB, which takes minutes
    /// under the sanitizers: run it with `--ignored`.
    #[test]
    #[ignore = "scans 2 GiB; run with --ignored"]
    fn oversized_slices_are_counted_in_rust() {
        let mut data = vec![b'a'; c_int::MAX as usize + 2];
        // One non-printable byte for 2^24 printable ones is still text.
        data[0] = 0x01;
        assert!(!data_is_binary(&data));
    }

    /// `testrepo.git`: commit `a65fedf`, whose tree `944c0f6` names the blob `a823312`
    /// (`README`, "hey there\n").
    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git\0"
    );
    const COMMIT: &CStr = c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
    const README: &CStr = c"a8233120f6ad708f843d861ce2b7228ec4e3dec6";

    fn open() -> RepositoryBox {
        assert!(runtime_pinned());
        Repository::open(CStr::from_bytes_with_nul(TESTREPO.as_bytes()).unwrap()).unwrap()
    }

    fn sha1(hex: &CStr) -> Oid {
        Oid::from_string(hex, OidType::Sha1).unwrap()
    }

    /// C's view of a live blob: its type as an object, and its content.
    fn content(blob: BlobRef<'_, '_>) -> (ffi::git_object_t, Vec<u8>) {
        let p = blob.as_ptr();
        // SAFETY: `p` is a live blob the handle borrows; its content lives as long as
        // it and is copied at once.
        unsafe {
            let size = usize::try_from(ffi::git_blob_rawsize(p)).unwrap();
            let data = ffi::git_blob_rawcontent(p).cast::<u8>();
            (
                ffi::git_object_type(p.cast()),
                core::slice::from_raw_parts(data, size).to_vec(),
            )
        }
    }

    #[test]
    fn blob_layout_matches_c() {
        assert_eq!(size_of::<Blob<'_>>(), size_of::<ffi::git_blob>());
        assert_eq!(size_of::<BlobArc<'_>>(), size_of::<*mut ffi::git_blob>());
        assert_eq!(
            size_of::<Option<BlobArc<'_>>>(),
            size_of::<*mut ffi::git_blob>()
        );
        assert_eq!(
            size_of::<BlobRef<'_, '_>>(),
            size_of::<*const ffi::git_blob>()
        );
        assert_eq!(
            size_of::<BlobMut<'_, '_>>(),
            size_of::<*const ffi::git_blob>()
        );
        assert_eq!(size_of::<BlobFree>(), 0);
        assert_eq!(
            size_of::<FilterOptions>(),
            size_of::<ffi::git_blob_filter_options>()
        );
    }

    /// The blob handles' seams over a pointer nothing reads, so Miri runs them:
    /// `git_blob` is opaque, of size zero, so a dangling aligned pointer addresses all
    /// of it.
    #[test]
    fn blob_handles_without_c() {
        // SAFETY: NULL is refused without being read.
        assert!(unsafe { BlobRef::from_ptr(ptr::null()) }.is_none());
        // SAFETY: as above.
        assert!(unsafe { BlobMut::from_ptr(ptr::null_mut()) }.is_none());
        let p = NonNull::<ffi::git_blob>::dangling().as_ptr();
        // SAFETY: a zero-sized blob at an aligned pointer, never read or written, and
        // reached by this handle alone.
        let mut m = unsafe { BlobMut::from_ptr(p) }.unwrap();
        assert_eq!(m.as_mut_ptr(), p);
        assert_eq!(m.as_mut().as_mut_ptr(), p);
        let r = m.as_ref();
        assert_eq!(r.as_ptr(), p.cast_const());
        assert_eq!(format!("{r:?}"), "Blob { .. }");
        assert_eq!(format!("{m:?}"), "Blob { .. }");
    }

    /// An id of another type than the repository's is refused before libgit2 formats
    /// it out of bounds, at every length; the same bytes typed as the repository's
    /// find the blob. Corrects `git_blob_lookup` and `git_blob_lookup_prefix`, which
    /// check nothing.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn blob_ids_of_another_type_are_refused_before_c() {
        let repo = open();
        let repo = repo.as_ref();
        let readme = sha1(README);
        let mut wide = Oid::from_raw(&readme.as_ref().id(), OidType::Sha256).unwrap();
        for kind in [OidType::Sha256.raw() as u8, 0, 9, u8::MAX] {
            wide.as_mut().set_raw_kind(kind);
            let e = Blob::lookup(repo, wide.as_ref()).unwrap_err();
            assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
            assert_eq!(
                e.message(),
                c"specified object ID type does not match repository object ID type"
            );
            for len in [0, 3, 4, 7, 39, 40, 41, 64, usize::MAX] {
                let e = Blob::lookup_prefix(repo, wide.as_ref(), len).unwrap_err();
                assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
            }
        }
        wide.as_mut().set_kind(OidType::Sha1);
        let blob = Blob::lookup(repo, wide.as_ref()).unwrap();
        assert_eq!(
            content(blob.as_ref()),
            (ffi::GIT_OBJECT_BLOB, b"hey there\n".to_vec())
        );
        for len in [7, 40, usize::MAX] {
            let found = Blob::lookup_prefix(repo, wide.as_ref(), len).unwrap();
            assert_eq!(content(found.as_ref()), content(blob.as_ref()));
        }
    }

    /// The policy driven by hand: the up-ref counts one more reference on the same
    /// pointer, which an adopted owner and the free each settle; `to_blob` takes one
    /// through a borrowed handle.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn blob_policy_counts_references_on_one_blob() {
        let repo = open();
        let blob = Blob::lookup(repo.as_ref(), sha1(README).as_ref()).unwrap();
        let p = NonNull::new(blob.as_ptr()).unwrap();
        // SAFETY: `blob` holds a reference to the live blob at `p`.
        assert!(unsafe { BlobFree.c_up_ref(p) });
        // SAFETY: the reference just counted, given to the owner.
        let adopted = unsafe { BlobArc::from_raw(p.as_ptr()) }.unwrap();
        assert!(BlobArc::ptr_eq(&adopted, &blob));
        let raw = adopted.into_c();
        // SAFETY: `raw` is the reference `into_c` gave up, released once here.
        unsafe { BlobFree.c_drop(NonNull::new(raw).unwrap().cast()) };

        // SAFETY: `blob` keeps the blob alive while the handle lives.
        let borrowed = unsafe { BlobRef::from_ptr(blob.as_c_ptr()) }.unwrap();
        let copy = borrowed.to_blob();
        assert!(BlobArc::ptr_eq(&copy, &blob));
        drop(blob);
        assert_eq!(content(copy.as_ref()).1, b"hey there\n");
        let mut copy = copy;
        assert!(copy.get_mut().is_none());
    }

    /// An object converts to a blob only when it is one, in both the borrowed and the
    /// owned forms; the blob converts back to the same object, and every conversion
    /// moves its reference rather than counting another.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn objects_convert_to_blobs_only_when_they_are_blobs() {
        let repo = open();
        let commit = Object::lookup(repo.as_ref(), sha1(COMMIT).as_ref(), None).unwrap();
        assert!(BlobRef::from_object(commit.as_ref()).is_none());
        let commit = Blob::from_object(commit).unwrap_err();
        let readme = commit.as_ref().lookup_bypath(c"README", None).unwrap();
        let p = readme.as_c_ptr();

        let borrowed = BlobRef::from_object(readme.as_ref()).unwrap();
        assert_eq!(borrowed.as_ptr(), p.cast_const().cast());
        assert_eq!(borrowed.as_object().as_ptr(), p.cast_const());
        assert_eq!(content(borrowed).1, b"hey there\n");

        let blob = Blob::from_object(readme).unwrap();
        assert_eq!(blob.as_c_ptr(), p.cast());
        let clone = blob.clone();
        let object = Blob::into_object(blob);
        assert_eq!(object.as_c_ptr(), p);
        drop(object);
        assert_eq!(content(clone.as_ref()).1, b"hey there\n");
        let back = Blob::from_object(Blob::into_object(clone)).unwrap();
        assert_eq!(back.as_c_ptr(), p.cast());
    }

    /// Options as C spells them, without libgit2, for Miri.
    fn raw_options(reserved: *mut core::ffi::c_void) -> ffi::git_blob_filter_options {
        ffi::git_blob_filter_options {
            version: 1,
            flags: ffi::GIT_BLOB_FILTER_CHECK_FOR_BINARY,
            reserved,
            attr_commit_id: ffi::git_oid::default(),
        }
    }

    /// `new` is `GIT_BLOB_FILTER_OPTIONS_INIT`, field by field.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn filter_options_new_is_the_template() {
        let opts = FilterOptions::new();
        let r = opts.as_ref();
        assert_eq!(r.version(), FilterOptions::VERSION);
        assert_eq!(r.version(), 1);
        assert_eq!(r.flags(), FilterFlags::OPTIONS_DEFAULT);
        assert!(!r.has_reserved());
        assert_eq!(r.attr_commit_id().raw_kind(), 0);
        assert_eq!(r.attr_commit_id().id(), [0; MAX_OID]);
        assert_eq!(opts, FilterOptions::default());
        assert_eq!(opts, FilterOptions::from(raw_options(ptr::null_mut())));
    }

    /// Every accessor reads back what its setter wrote, through every handle, and the
    /// raw conversions keep every field, `reserved` included, which only `clear`
    /// changes. Miri checks each projection.
    #[test]
    fn filter_options_accessors_round_trip() {
        let mut slot = 0u64;
        let dangling = ptr::from_mut(&mut slot).cast::<core::ffi::c_void>();
        let mut opts = FilterOptions::from(raw_options(dangling));
        assert!(opts.as_ref().has_reserved());
        let copy = opts;
        assert_eq!(ffi::git_blob_filter_options::from(copy).reserved, dangling);

        let mut m = opts.as_mut();
        m.set_version(-1);
        m.set_flags(FilterFlags::from_bits_retain(0x8000_0005));
        m.attr_commit_id_mut().set_raw_kind(1);
        m.attr_commit_id_mut().set_id([0xab; MAX_OID]);
        assert_eq!(m.as_ref().version(), -1);
        assert_eq!(m.as_ref().flags().bits(), 0x8000_0005);
        assert_eq!(m.as_ref().attr_commit_id().raw_kind(), 1);
        m.clear_reserved();
        assert!(!m.as_ref().has_reserved());
        assert_ne!(opts, copy);
        assert_eq!(slot, 0);

        let mut other = Oid::zeroed();
        other.as_mut().set_raw_kind(2);
        other.as_mut().set_id([0x5a; MAX_OID]);
        opts.as_mut().set_attr_commit_id(other.as_ref());
        let id = opts.as_ref().attr_commit_id();
        assert_eq!((id.raw_kind(), id.id()), (2, [0x5a; MAX_OID]));

        let raw = ffi::git_blob_filter_options::from(opts);
        assert_eq!(raw.version, -1);
        assert_eq!(raw.flags, 0x8000_0005);
        assert!(raw.reserved.is_null());
        assert_eq!(raw.attr_commit_id.type_, 2);
        assert_eq!(raw.attr_commit_id.id, [0x5a; MAX_OID]);
        assert_eq!(opts.as_ref().to_options(), opts);
    }

    /// Equality and hashing go field by field, `reserved` by whether it is set; the
    /// debug form names each field.
    #[test]
    fn filter_options_compare_hash_and_debug() {
        use std::collections::hash_map::DefaultHasher;
        fn hash(opts: &FilterOptions) -> u64 {
            let mut h = DefaultHasher::new();
            opts.hash(&mut h);
            h.finish()
        }
        let mut slot = 0u8;
        let base = FilterOptions::from(raw_options(ptr::null_mut()));
        let other_reserved = FilterOptions::from(raw_options(ptr::from_mut(&mut slot).cast()));
        assert_ne!(base, other_reserved);
        let mut variants = vec![other_reserved];
        let mut v = base;
        v.as_mut().set_version(2);
        variants.push(v);
        let mut v = base;
        v.as_mut().set_flags(FilterFlags::empty());
        variants.push(v);
        let mut v = base;
        v.as_mut().attr_commit_id_mut().set_raw_kind(1);
        variants.push(v);
        for v in &variants {
            assert_ne!(*v, base, "{v:?}");
        }
        let same = base;
        assert_eq!(same, base);
        assert_eq!(hash(&same), hash(&base));
        let mut cleared = other_reserved;
        cleared.as_mut().clear_reserved();
        assert_eq!(cleared, base);
        assert_eq!(hash(&cleared), hash(&base));

        let shown = format!("{base:?}");
        assert!(
            shown.starts_with(
                "FilterOptions { version: 1, flags: FilterFlags {CheckForBinary}, \
                 has_reserved: false, attr_commit_id: "
            ),
            "{shown}"
        );
        let mut m = v;
        assert_eq!(format!("{:?}", m.as_mut()), format!("{v:?}"));
    }
}
