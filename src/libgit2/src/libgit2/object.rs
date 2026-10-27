//! Safe wrappers over `include/git2/object.h`.
//!
//! An [`Object`] is a `git_object`: a parsed commit, tree, blob or tag, the common
//! head of `git_commit`, `git_tree`, `git_blob` and `git_tag`. It is opaque: libgit2
//! publishes no field.
//!
//! # Ownership
//!
//! An object is reference counted and shared. Each repository keeps an object cache,
//! which holds a reference of its own to every object it stores and hands the same
//! pointer, one more reference counted on it, to every lookup of that id; whoever
//! looked an object up owns one reference, `git_object_free` gives it back, and
//! `git_object_dup` takes another on the same pointer. The object is freed with the
//! last reference. The cache stores commits, trees and tags under a size limit, and by
//! default no blob: a blob is parsed afresh for each lookup, which then holds its only
//! reference. So the Rust owner is an [`ObjectArc`], one counted reference whose
//! `Clone` is `git_object_dup`, and which never proves itself the only one: the cache
//! may hold another, and a later lookup take one more. It lends the shared handle,
//! [`ObjectRef`], and never [`ObjectMut`].
//!
//! # The repository
//!
//! An object keeps the repository it was looked up in (`object.repo`), without a
//! count, and `git_repository_free` does not clear it. Freeing an object never reads
//! it, but every routine that looks something up through an object does
//! (`git_object_owner`, `git_object_peel`, `git_object_lookup_bypath`, a commit's
//! parents and tree, a tag's target), and fills that repository's object cache. So an
//! object carries the lifetime `'r` of the repository borrow it was looked up through,
//! and lives no longer than it, as a [`Reference`](crate::libgit2::src::libgit2::refs::Reference)
//! does.
//!
//! # Threads
//!
//! Neither an object nor its owner is `Send` or `Sync`. The count is atomic and the
//! cache locked, but what an object reaches is its repository, which is neither, and
//! `git_commit`'s lazily computed summary and body are written through a commit
//! pointer after it is shared.
//!
//! # Looking objects up
//!
//! [`Object::lookup`] and [`Object::lookup_prefix`] build an owner from a repository
//! and an id; [`ObjectRef::peel`] and [`ObjectRef::lookup_bypath`] derive one from an
//! object, through its repository: a tag's target or a commit's tree, and the object
//! at a path under a tree, each one more counted reference of the repository's cache
//! or a fresh parse. The lookups by id take an id whose type libgit2 never checks
//! against the repository's. A full-length
//! id goes to the object database as is, whose loose backend formats it into a path
//! sized for the repository's id type: an id of another type overflows that path (a
//! SHA-256 id in a SHA-1 repository) or leaves part of it unwritten (the converse). A
//! prefix is copied into a key that keeps the id's type (`git_oid__cpy_prefix`), which
//! the loose backend formats into a stack buffer before reading it back: for a type
//! byte no `git_oid_t` names, nothing is written and the buffer is read uninitialised.
//! So the wrappers refuse an id of another type than the repository's, at any length,
//! before the call, with libgit2's own wording for the mismatch.
//!
//! # Id options
//!
//! An [`ObjectIdOptions`] is a `git_object_id_options`: how `git_object_id_from_buffer`
//! and `git_object_id_from_file` compute an object's id from its raw content — the
//! object type its header names (`object_type`, a blob when 0), the id type to hash
//! with (`oid_type`, `GIT_OID_DEFAULT` when 0), and the filters to run a blob's content
//! through first (`filters`, none when NULL). libgit2 never allocates, keeps or frees
//! one: the caller builds it with `git_object_id_options_init`, and each routine copies
//! the fields it needs for the call, refusing (with -1 and a `GIT_ERROR_INVALID`
//! error) an object type that is not a commit, tree, blob or tag, and an id type that
//! is neither SHA-1 nor SHA-256. No routine checks `version`.
//!
//! So nothing it points at is owned. Rust holds the struct by value as an
//! [`ObjectIdOptions<'f>`] (filled by [`ObjectIdOptions::new`]), which borrows the
//! filter list for `'f`, *exclusively*: applying a list writes each filter's state in
//! it (the stream's payload slot, the list's scratch buffer), so the list is set from
//! a [`FilterListMut`] and read back as a [`FilterListRef`] no longer than the options'
//! own shared borrow. The routines taking the options (their own runs) take them
//! through [`ObjectIdOptionsMut`] for that reason, although C declares them `const`.
//! The object and id types are set from [`ObjectType`] and [`OidType`], or `None` for
//! the default, so a stored value is always one libgit2 accepts.
//! The options are neither `Send` nor `Sync`, as the list they borrow is not.
//!
//! # Types and contents
//!
//! An object's type is one of the four [`ObjectType`]s, which [`ObjectType::name`] and
//! [`ObjectType::from_name`] spell as libgit2's object headers do (`commit`, `tree`,
//! `blob`, `tag`); [`type_is_valid`] tells those types from the other values a raw
//! `git_object_t` may hold. [`content_is_valid`] parses an object's raw content, as its
//! type would be parsed from the object database, without a repository.

use core::ffi::CStr;
use core::fmt;
use core::hash::Hash;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull};

use ffibox::{CArc, CBorrowedPtr, CCell, CDrop, CMutSeam, CRefClone, CRefSeam, CZeroable};

use crate::ffi;
use crate::libgit2::src::libgit2::buf::{Buf, BufVal};
use crate::libgit2::src::libgit2::filter::{FilterListMut, FilterListRef};
use crate::libgit2::src::libgit2::oid::{Oid, OidRef, OidType};
use crate::libgit2::src::libgit2::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::src::util::errors::{
    ErrorBuf, ErrorClass, ErrorCode, last, runtime_pinned, set,
};

/// The type of a Git object: what its content is, which fixes how libgit2 parses it.
///
/// libgit2 answers `GIT_OBJECT_INVALID` where it has no type, and its lookups take
/// `GIT_OBJECT_ANY` to accept any type ([`raw_or_any`](Self::raw_or_any));
/// [`from_raw`](Self::from_raw) gives `None` for both, and for every value
/// `git_object_type_is_valid` refuses.
///
/// Wraps: `git_object_t`
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
#[non_exhaustive]
pub enum ObjectType {
    /// A commit object.
    Commit = ffi::GIT_OBJECT_COMMIT,
    /// A tree (directory listing) object.
    Tree = ffi::GIT_OBJECT_TREE,
    /// A file revision object.
    Blob = ffi::GIT_OBJECT_BLOB,
    /// An annotated tag object.
    Tag = ffi::GIT_OBJECT_TAG,
}

impl ObjectType {
    /// The variant whose C value is `raw`; `None` for `GIT_OBJECT_ANY`,
    /// `GIT_OBJECT_INVALID` and every value the header does not name.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_object_t) -> Option<Self> {
        match raw {
            ffi::GIT_OBJECT_COMMIT => Some(Self::Commit),
            ffi::GIT_OBJECT_TREE => Some(Self::Tree),
            ffi::GIT_OBJECT_BLOB => Some(Self::Blob),
            ffi::GIT_OBJECT_TAG => Some(Self::Tag),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    #[must_use]
    pub const fn raw(self) -> ffi::git_object_t {
        self as ffi::git_object_t
    }

    /// The C value of a type filter, as lookups and peels take it: `kind`'s value, or
    /// `GIT_OBJECT_ANY` for `None`, which accepts an object of any type.
    #[must_use]
    pub const fn raw_or_any(kind: Option<Self>) -> ffi::git_object_t {
        match kind {
            Some(kind) => kind.raw(),
            None => ffi::GIT_OBJECT_ANY,
        }
    }
}

impl TryFrom<ffi::git_object_t> for ObjectType {
    type Error = NotAnObjectType;

    fn try_from(raw: ffi::git_object_t) -> Result<Self, NotAnObjectType> {
        Self::from_raw(raw).ok_or(NotAnObjectType(raw))
    }
}

impl From<ObjectType> for ffi::git_object_t {
    fn from(value: ObjectType) -> ffi::git_object_t {
        value.raw()
    }
}

/// A `git_object_t` value that is no object's type: `GIT_OBJECT_ANY`,
/// `GIT_OBJECT_INVALID`, or a value the header does not name. [`raw`](Self::raw) has
/// it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotAnObjectType(ffi::git_object_t);

impl NotAnObjectType {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::git_object_t {
        self.0
    }
}

impl fmt::Display for NotAnObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "libgit2 value {} is not an object type", self.0)
    }
}

impl std::error::Error for NotAnObjectType {}

/// A parsed git object (`git_object`): a commit, tree, blob or tag read from a
/// repository's object database.
///
/// libgit2 allocates, counts and frees it, so it is only ever reached through an
/// [`ObjectArc`] and its handles, built by [`lookup`](Self::lookup) and
/// [`lookup_prefix`](Self::lookup_prefix), or derived from another object by
/// [`ObjectRef::peel`] and [`ObjectRef::lookup_bypath`]. It is opaque: no field is
/// published.
///
/// `'r` is the borrow of the repository the object was looked up in: routines that
/// look further objects up through it reach that repository, so it lives no longer
/// than the borrow (see the [module docs](self#the-repository)). The lifetime only
/// shrinks:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::object::ObjectArc;
///
/// fn widen<'r>(object: ObjectArc<'r>) -> ObjectArc<'static> {
///     object
/// }
/// ```
///
/// and an object stays on the thread that looked it up:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::object::ObjectArc;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(object: ObjectArc<'static>) {
///     send(object)
/// }
/// ```
///
/// Wraps: `git_object`
/// Handwritten
#[repr(transparent)]
pub struct Object<'r>(ffi::git_object, PhantomData<(*const (), &'r ())>);

/// Shared borrow of an [`Object`]. `Copy`, like `&T`.
///
/// It stays on the thread of the object's repository, as the owner does:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::object::ObjectRef;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(handle: ObjectRef<'static, 'static>) {
///     send(handle)
/// }
/// ```
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ObjectRef<'a, 'r>(CBorrowedPtr<'a, Object<'r>>);

/// Exclusive borrow of an [`Object`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// No owner hands one out: every object is shared with its repository's cache.
///
/// Handwritten
#[repr(transparent)]
pub struct ObjectMut<'a, 'r>(
    ObjectRef<'a, 'r>,
    // Invariant in the object, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Object<'r>>,
);

// SAFETY: `Object` is `#[repr(transparent)]` over `git_object` with a zero-sized
// marker; both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Object<'r>>`
// (`ObjectMut` through `ObjectRef`) with zero-sized markers and no `Drop`, hand out no
// reference to the C object, and `ObjectRef` has no write operation. `ObjectMut` is
// invariant in the object through its `PhantomData<&'a mut Object<'r>>`.
unsafe impl<'r> CCell for Object<'r> {
    type C = ffi::git_object;
    type Mut<'a>
        = ObjectMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = ObjectRef<'a, 'r>
    where
        Self: 'a;
}

impl<'a, 'r: 'a> CRefSeam<'a> for ObjectRef<'a, 'r> {
    type Cell = Object<'r>;
}

impl<'a, 'r: 'a> CMutSeam<'a> for ObjectMut<'a, 'r> {
    type Cell = Object<'r>;
}

impl<'a, 'r> ObjectRef<'a, 'r> {
    /// Another counted reference to the object, as `git_object_dup` takes one: the
    /// same object, now also kept alive by the result, which may outlive this borrow
    /// but not the repository.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::object::Object;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let object = Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap();
    /// let copy = object.as_ref().to_object();
    /// drop(object);
    /// drop(repo);
    /// drop(copy);
    /// ```
    ///
    /// Wraps: `git_object_dup`
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a handle is never NULL"
    )]
    pub fn to_object(&self) -> ObjectArc<'r> {
        let object = self.0.as_non_null();
        // SAFETY: the handle addresses a live object for `'a`, whose reference its
        // holder keeps meanwhile; one more is counted on it here.
        if !unsafe { ObjectFree.c_up_ref(object) } {
            unreachable!("git_object_dup cannot fail");
        }
        // SAFETY: the reference just counted, given to the result, which `git_object_free`
        // releases; the object's repository outlives `'r`, which the result carries.
        unsafe { ObjectArc::from_raw(object.as_ptr()) }.expect("a handle is never NULL")
    }

    /// The object's id, of its repository's id type: the id it was looked up by, or the
    /// full id a prefix found. Borrowed for as long as the handle: libgit2 keeps it in the
    /// object, which writes it once, when it is parsed.
    ///
    /// Wraps: `git_object_id`
    ///
    /// # Panics
    ///
    /// Panics, with `a live object has an id`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: the handle addresses a live object for `'a`; libgit2 only reads it, and
        // returns the address of the id cached in it (`&obj->cached.oid`).
        let id = unsafe { ffi::git_object_id(self.as_ptr()) };
        // SAFETY: the id is part of the object, so it is non-NULL and lives as long as the
        // object, past `'a`; nothing writes it once the object is cached or returned.
        unsafe { OidRef::from_ptr(id.cast_mut()) }.expect("a live object has an id")
    }

    /// The repository the object was looked up in, borrowed for `'r`, as the shared
    /// handle it was looked up through (every [`Object`] is built from a
    /// [`RepositoryRef<'r>`](RepositoryRef), or derived from such an object).
    ///
    /// The result is that handle again, so it lets nothing happen that the object's own
    /// lookups do not: it outlives the object, but not the repository's borrow.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::object::Object;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let object = Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap();
    /// let owner = object.as_ref().owner();
    /// drop(repo);
    /// let _ = owner.path();
    /// ```
    ///
    /// Wraps: `git_object_owner`
    ///
    /// # Panics
    ///
    /// Panics, with `an object always has a repository`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn owner(&self) -> RepositoryRef<'r> {
        // SAFETY: the handle addresses a live object for `'a`, which libgit2 only reads.
        let repo = unsafe { ffi::git_object_owner(self.as_ptr()) };
        // SAFETY: `obj->repo` is the repository the object was looked up in, stored
        // without a count and never cleared. An `Object<'r>` is only built from a
        // `RepositoryRef<'r>` (`lookup`, `lookup_prefix`) or from another `Object<'r>`
        // (`peel`, `lookup_bypath`, `to_object`), so that repository is live and shared,
        // not exclusively borrowed, for `'r`, on this thread: the handle returned is the
        // one the object was looked up through.
        unsafe { RepositoryRef::from_ptr(repo) }.expect("an object always has a repository")
    }

    /// The object this one peels to, of type `kind`: the same object when it is already
    /// of that type (one more reference, as [`to_object`](Self::to_object) takes one); a
    /// tag's target, followed through nested tags, for a tag; a commit's tree for a
    /// commit. For `None`, the first object of another type than this one: what a
    /// tag's chain of tags ends in, or a commit's tree.
    ///
    /// Each step looks the next object up in the repository, through the cache it
    /// shares with every lookup, so the result lives no longer than the repository's
    /// borrow `'r`, as this object does.
    ///
    /// Wraps: `git_object_peel`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::InvalidSpec`] (class `GIT_ERROR_OBJECT`, "the
    /// `git_object` of id '…' can not be successfully peeled into a …") when the types
    /// cannot be combined: a blob or tree peeled to any other type, `None` included;
    /// a commit peeled to a commit's parent types; and with [`ErrorCode::Peel`] (same
    /// message) when a tag's chain ends in an object of another type than `kind`, such
    /// as a tag of a commit peeled to a blob. Fails with libgit2's error when a target
    /// cannot be read from the object database.
    ///
    /// # Panics
    ///
    /// Panics, with `a peel succeeded without an object`, only if libgit2 breaks its contract.
    pub fn peel(&self, kind: Option<ObjectType>) -> Result<ObjectArc<'r>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; the handle addresses a
        // live object for `'a`, whose repository, borrowed for `'r`, is live and used
        // on this thread only, as `RepositoryRef` allows: libgit2 counts one more
        // reference on this object, or looks its targets up through the repository,
        // filling the cache; the target type is a named one or `GIT_OBJECT_ANY`, which
        // the routine's argument check accepts.
        let rc = unsafe {
            ffi::git_object_peel(&raw mut out, self.as_ptr(), ObjectType::raw_or_any(kind))
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live object of the
        // same repository, borrowed for `'r`, given to the result and released by
        // `git_object_free`.
        let object = unsafe { ObjectArc::from_c(out) };
        Ok(object.expect("a peel succeeded without an object"))
    }

    /// The object at `path` under this one, of type `kind`, or of any type for `None`.
    /// This object is first peeled to a tree ([`peel`](Self::peel) with
    /// [`ObjectType::Tree`]: itself, a commit's tree, or what a tag chain ends in),
    /// then each `/`-separated component of `path` is looked up in turn, the last
    /// naming the entry whose object is looked up in the repository.
    ///
    /// The components are matched as the tree spells them: `.` and `..` name nothing,
    /// a trailing `/` is accepted only after a tree, and an empty component (an empty
    /// `path`, or a leading or doubled `/`) is an invalid path.
    ///
    /// The result lives no longer than the repository's borrow `'r`, as this object
    /// does.
    ///
    /// Wraps: `git_object_lookup_bypath`
    ///
    /// # Errors
    ///
    /// Fails as [`peel`](Self::peel) does when this object cannot be peeled to a tree
    /// (a blob, or a tag of one); with [`ErrorCode::NotFound`] (class
    /// `GIT_ERROR_TREE`) for an invalid path, a component the tree lacks, or a `/` after
    /// an entry that is not a tree; with [`ErrorCode::InvalidSpec`] (class
    /// `GIT_ERROR_OBJECT`, "object at path '…' is not of the asked-for type …") when
    /// the entry's object is not of type `kind`; and with libgit2's error when a tree
    /// or the object cannot be read.
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without an object`, only if libgit2 breaks its contract.
    pub fn lookup_bypath(
        &self,
        path: &CStr,
        kind: Option<ObjectType>,
    ) -> Result<ObjectArc<'r>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: as in `peel`: `out` is a writable slot, the handle addresses a live
        // object whose repository is live for `'r` and used on this thread only, where
        // libgit2 looks the trees and the entry's object up; `path` is NUL-terminated
        // and read for the call only; the type is a named one or `GIT_OBJECT_ANY`.
        let rc = unsafe {
            ffi::git_object_lookup_bypath(
                &raw mut out,
                self.as_ptr(),
                path.as_ptr(),
                ObjectType::raw_or_any(kind),
            )
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: as in `peel`: one counted reference to a live object of the
        // repository, given to the result.
        let object = unsafe { ObjectArc::from_c(out) };
        Ok(object.expect("a lookup succeeded without an object"))
    }

    /// The object's type: what it was parsed as when it was looked up, which never
    /// changes.
    ///
    /// Wraps: `git_object_type`
    ///
    /// # Panics
    ///
    /// Panics, with `a parsed object always has a named type`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn kind(&self) -> ObjectType {
        // SAFETY: the handle addresses a live object for `'a`; `git_object_type` reads
        // the type cached in it when it was parsed, which nothing writes afterwards.
        let raw = unsafe { ffi::git_object_type(self.as_ptr()) };
        ObjectType::from_raw(raw).expect("a parsed object always has a named type")
    }

    /// The shortest abbreviation of the object's id, in hex, that names it alone in its
    /// repository's object database, and is at least the repository's `core.abbrev`
    /// long (7 digits by default and for `auto`; `no`, `false` or `off`, or a length past
    /// the id's, asks for the full id).
    ///
    /// libgit2 starts at that length and adds a digit while the prefix names several
    /// objects, so the abbreviation is unambiguous until objects sharing it are added.
    /// It reads the repository's configuration and loads its object database, through
    /// the repository the object was looked up in.
    ///
    /// The result is a fresh buffer: libgit2 overwrites the `git_buf` it is given
    /// without releasing what it held, which a fresh one does not hold.
    ///
    /// Wraps: `git_object_short_id`
    ///
    /// # Errors
    ///
    /// Fails with `-1` and a `GIT_ERROR_CONFIG` error when `core.abbrev` is under 4
    /// ("invalid oid abbreviation setting: '…'") or none of the above ("failed to map
    /// '…'", `true` included), and with libgit2's error when the
    /// configuration or the object database cannot be read.
    pub fn short_id(&self) -> Result<BufVal, RepositoryError> {
        let mut out = Buf::new();
        // SAFETY: `out` is an empty buffer libgit2 fills; the handle addresses a live
        // object for `'a`, whose repository, borrowed for `'r`, is live and used on this
        // thread only, as `RepositoryRef` allows: libgit2 reads its configuration and
        // loads its object database through it, cache fills, and only reads the object.
        let rc = unsafe { ffi::git_object_short_id(out.as_mut().as_mut_ptr(), self.as_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        Ok(out)
    }
}

impl<'r> ObjectMut<'_, 'r> {
    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> ObjectRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> ObjectMut<'_, 'r> {
        ObjectMut(self.0, PhantomData)
    }
}

impl fmt::Debug for ObjectRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Object").finish_non_exhaustive()
    }
}

impl fmt::Debug for ObjectMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases one counted reference to an object through `git_object_free`, and takes
/// another through `git_object_dup`.
///
/// The free decrements the object's atomic count and, at zero, frees it through the
/// free of its type, which never reads its repository: it needs no state beyond the
/// pointer. The copy increments the same count and hands back the same pointer, so a
/// copy is the same object, of the same repository, for the same `'r`. Neither can
/// fail, and neither allocates.
///
/// Wraps: `git_object_free`
/// Wraps: `git_object_dup`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct ObjectFree;

// SAFETY: `git_object_free` gives back exactly one counted reference of a live object,
// in every state the owners reach (a NULL pointer, which it skips, never reaches it);
// the object outlives the call through that reference, and is freed only once the
// count, which the cache's reference is part of, reaches zero.
unsafe impl<'r> CDrop<Object<'r>> for ObjectFree {
    unsafe fn c_drop(&self, ptr: NonNull<Object<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live object of which the
        // caller gives up one reference; `Object` is transparent over `git_object`.
        unsafe { ffi::git_object_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `git_object_dup` atomically increments the count of the live object it is
// given and stores that same pointer in `dest`, returning 0: the object then owes one
// more `git_object_free`, which this policy performs, and `ptr` stays valid. It never
// fails. `c_is_sole_owner` keeps its `false` default: the repository's cache counts a
// reference no owner sees, and a later lookup may take another.
unsafe impl<'r> CRefClone<Object<'r>> for ObjectFree {
    unsafe fn c_up_ref(&self, ptr: NonNull<Object<'r>>) -> bool {
        let mut dest = ptr::null_mut();
        // SAFETY: `dest` is a writable slot; `ptr` addresses a live object whose
        // reference the caller holds, per `c_up_ref`'s contract.
        let rc = unsafe { ffi::git_object_dup(&raw mut dest, ptr.as_ptr().cast()) };
        debug_assert_eq!(dest, ptr.as_ptr().cast());
        rc == 0
    }
}

/// One counted reference to an [`Object`], given back by `git_object_free` on drop;
/// `clone` takes another through `git_object_dup`, on the same object.
pub type ObjectArc<'r> = CArc<Object<'r>, ObjectFree>;

impl<'r> Object<'r> {
    /// The object `id` names in `repo`, of type `kind`, or of any type for `None`.
    ///
    /// The repository's cache answers when it holds the object, with the object it
    /// holds; otherwise libgit2 reads it from the object database, parses it and, for
    /// a commit, tree or tag under the cache's size limit, caches it, so that every
    /// later lookup of the id yields the same object while the cache keeps it.
    ///
    /// The object reads the repository it was looked up in, so it lives no longer than
    /// this borrow of it:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::object::Object;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let object = Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap();
    /// drop(repo);
    /// drop(object);
    /// ```
    ///
    /// Wraps: `git_object_lookup`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when the object database has no such object
    /// (the zero id included), when the object is not of type `kind` (class
    /// `GIT_ERROR_INVALID`), and for a repository without a git directory, which has no
    /// object database to load (`GIT_ERROR_INVALID`, "path cannot exist in
    /// repository"); and with libgit2's error when the object database cannot be
    /// loaded or read, or the object cannot be parsed.
    ///
    /// Refuses, with code `-1` and libgit2's `GIT_ERROR_INVALID` error "specified
    /// object ID type does not match repository object ID type", an `id` whose type is
    /// not `repo`'s, which libgit2 would format out of bounds (see the
    /// [module docs](self#looking-objects-up)).
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without an object`, only if libgit2 breaks its contract.
    pub fn lookup(
        repo: RepositoryRef<'r>,
        id: OidRef<'_>,
        kind: Option<ObjectType>,
    ) -> Result<ObjectArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `repo` is a live
        // repository on this thread, whose odb libgit2 loads and whose cache it fills
        // through this shared handle, as `RepositoryRef` allows; `id` is a live id of
        // the repository's type (checked above), read for the call only; the type is a
        // named one or `GIT_OBJECT_ANY`.
        let rc = unsafe {
            ffi::git_object_lookup(
                &raw mut out,
                repo.as_ptr().cast_mut(),
                id.as_ptr(),
                ObjectType::raw_or_any(kind),
            )
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live object of
        // `repo`, borrowed for `'r`, given to the result and released by
        // `git_object_free`.
        let object = unsafe { ObjectArc::from_c(out) };
        Ok(object.expect("a lookup succeeded without an object"))
    }

    /// The object whose id starts with the first `len` hex digits of `id`, in `repo`,
    /// of type `kind`, or of any type for `None`. A `len` past the repository's full
    /// length (40 hex digits for SHA-1, 64 for SHA-256) means the full id, which is
    /// [`lookup`](Self::lookup).
    ///
    /// The digits past `len` are ignored, and every object in the database is a
    /// candidate, whatever its type: the prefix must name exactly one, which must then
    /// be of type `kind`.
    ///
    /// Wraps: `git_object_lookup_prefix`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::Ambiguous`] when `len` is under 4 (`GIT_ERROR_OBJECT`,
    /// "ambiguous lookup - OID prefix is too short") or the prefix names several
    /// objects, with [`ErrorCode::NotFound`] when it names none or one of another type,
    /// and as [`lookup`](Self::lookup) otherwise; an `id` of another type than `repo`'s
    /// is refused as it is, whatever `len`.
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without an object`, only if libgit2 breaks its contract.
    pub fn lookup_prefix(
        repo: RepositoryRef<'r>,
        id: OidRef<'_>,
        len: usize,
        kind: Option<ObjectType>,
    ) -> Result<ObjectArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: as in `lookup`; libgit2 clamps `len` to the repository's length, and
        // copies at most `(len + 1) / 2` of `id`'s `GIT_OID_MAX_SIZE` bytes, with its
        // type, into the key it searches with.
        let rc = unsafe {
            ffi::git_object_lookup_prefix(
                &raw mut out,
                repo.as_ptr().cast_mut(),
                id.as_ptr(),
                len,
                ObjectType::raw_or_any(kind),
            )
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live object of
        // `repo`, borrowed for `'r`, given to the result and released by
        // `git_object_free`.
        let object = unsafe { ObjectArc::from_c(out) };
        Ok(object.expect("a lookup succeeded without an object"))
    }
}

/// `Ok` when `id` is of `repo`'s object id type, the refusal otherwise; see the
/// [module docs](self#looking-objects-up). The typed lookups (`git_tree_lookup`,
/// `git_tag_lookup`, `git_blob_lookup`, ...) forward to the same routines, and refuse the
/// same ids; so does every other wrapper handing an id to a routine that reads it from
/// the object database.
pub(crate) fn id_fits(repo: RepositoryRef<'_>, id: OidRef<'_>) -> Result<(), RepositoryError> {
    if u32::from(id.raw_kind()) == repo.oid_type().raw() {
        return Ok(());
    }
    set(
        ErrorClass::Invalid,
        c"specified object ID type does not match repository object ID type",
    );
    Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()))
}

/// Options for computing an object's id from its raw content
/// (`git_object_id_options`), held by value in Rust storage.
///
/// Built by [`new`](Self::new) (or [`Default`]) with libgit2's defaults: the content is
/// a blob's, hashed with the default id type (SHA-1), unfiltered. Read it through
/// [`as_ref`](Self::as_ref) and change it through [`as_mut`](Self::as_mut):
///
/// ```
/// use libgit2::libgit2::src::libgit2::object::{ObjectIdOptions, ObjectType};
/// use libgit2::libgit2::src::libgit2::oid::OidType;
///
/// # if cfg!(miri) { return; }
/// let mut opts = ObjectIdOptions::new();
/// assert_eq!(opts.as_ref().version(), ObjectIdOptions::VERSION);
/// assert_eq!(opts.as_ref().object_type(), None);
/// opts.as_mut().set_object_type(Some(ObjectType::Commit));
/// opts.as_mut().set_oid_type(Some(OidType::Sha1));
/// assert_eq!(opts.as_ref().object_type(), Some(ObjectType::Commit));
/// assert!(opts.as_ref().filters().is_none());
/// ```
///
/// `#[repr(transparent)]` over `git_object_id_options`, so a pointer to it is a pointer
/// to the C struct, which C reads in place. `'f` is the exclusive borrow of the filter
/// list it points at (see the [module docs](self#id-options)). The options therefore
/// outlive neither the list:
///
/// ```compile_fail,E0597
/// use libgit2::ffi;
/// use libgit2::libgit2::src::libgit2::filter::FilterList;
/// use libgit2::libgit2::src::libgit2::object::ObjectIdOptions;
/// use libgit2::libgit2::src::libgit2::repository::Repository;
///
/// let repo = Repository::new().unwrap();
/// let mut opts = ObjectIdOptions::new();
/// {
///     let mut list = FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
///     opts.as_mut().set_filters(Some(list.as_mut()));
/// }
/// let _ = opts.as_ref().filters();
/// ```
///
/// nor the repository the list borrows:
///
/// ```compile_fail,E0505
/// use libgit2::ffi;
/// use libgit2::libgit2::src::libgit2::filter::FilterList;
/// use libgit2::libgit2::src::libgit2::object::ObjectIdOptions;
/// use libgit2::libgit2::src::libgit2::repository::Repository;
///
/// let repo = Repository::new().unwrap();
/// let mut list = FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
/// let mut opts = ObjectIdOptions::new();
/// opts.as_mut().set_filters(Some(list.as_mut()));
/// drop(repo);
/// let _ = opts.as_ref().filters();
/// ```
///
/// and no other handle to the list is used while the options borrow it, shared or
/// exclusive:
///
/// ```compile_fail,E0502
/// use libgit2::ffi;
/// use libgit2::libgit2::src::libgit2::filter::FilterList;
/// use libgit2::libgit2::src::libgit2::object::ObjectIdOptions;
/// use libgit2::libgit2::src::libgit2::repository::Repository;
///
/// let repo = Repository::new().unwrap();
/// let mut list = FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
/// let mut opts = ObjectIdOptions::new();
/// opts.as_mut().set_filters(Some(list.as_mut()));
/// let _shared = list.as_ref();
/// let _ = opts.as_ref().filters();
/// ```
///
/// A list read back through the options lives no longer than their shared borrow, so
/// it cannot be held while they are changed:
///
/// ```compile_fail,E0502
/// use libgit2::ffi;
/// use libgit2::libgit2::src::libgit2::filter::FilterList;
/// use libgit2::libgit2::src::libgit2::object::ObjectIdOptions;
/// use libgit2::libgit2::src::libgit2::repository::Repository;
///
/// let repo = Repository::new().unwrap();
/// let mut list = FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
/// let mut opts = ObjectIdOptions::new();
/// opts.as_mut().set_filters(Some(list.as_mut()));
/// let read = opts.as_ref().filters();
/// opts.as_mut().set_filters(None);
/// let _ = read;
/// ```
///
/// The exclusive handle is invariant in `'f`, so one cannot be shrunk to accept a
/// shorter borrow than its options were built for:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::object::ObjectIdOptionsMut;
///
/// fn shrink<'a, 'f>(m: ObjectIdOptionsMut<'a, 'static>) -> ObjectIdOptionsMut<'a, 'f> {
///     m
/// }
/// ```
///
/// Neither `Send` nor `Sync`, as the filter list it borrows is not:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::object::ObjectIdOptions;
///
/// fn send<T: Send>(_: T) {}
/// send(ObjectIdOptions::new());
/// ```
///
/// Not `Clone`: a copy would alias the exclusive borrow of the list. Build another with
/// [`new`](Self::new).
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::src::libgit2::object::ObjectIdOptions;
///
/// let opts = ObjectIdOptions::new();
/// let _copy = opts.clone();
/// ```
///
/// Wraps: `git_object_id_options`
/// Handwritten
#[repr(transparent)]
pub struct ObjectIdOptions<'f>(
    ffi::git_object_id_options,
    PhantomData<(*const (), &'f mut ())>,
);

/// Shared borrow of an [`ObjectIdOptions`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ObjectIdOptionsRef<'a, 'f>(CBorrowedPtr<'a, ObjectIdOptions<'f>>);

/// Exclusive borrow of an [`ObjectIdOptions`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct ObjectIdOptionsMut<'a, 'f>(
    ObjectIdOptionsRef<'a, 'f>,
    // Invariant in the options, and so in `'f`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut ObjectIdOptions<'f>>,
);

// SAFETY: `ObjectIdOptions` is `#[repr(transparent)]` over `git_object_id_options` with
// a zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, ObjectIdOptions<'f>>` (`ObjectIdOptionsMut` through
// `ObjectIdOptionsRef`) with zero-sized markers and no `Drop`, hand out no reference to
// the options, and `ObjectIdOptionsRef` has no write operation. `ObjectIdOptionsMut` is
// invariant in the options through its `PhantomData<&'a mut ObjectIdOptions<'f>>`.
unsafe impl<'f> CCell for ObjectIdOptions<'f> {
    type C = ffi::git_object_id_options;
    type Mut<'a>
        = ObjectIdOptionsMut<'a, 'f>
    where
        Self: 'a;
    type Ref<'a>
        = ObjectIdOptionsRef<'a, 'f>
    where
        Self: 'a;
}

impl<'a, 'f: 'a> CRefSeam<'a> for ObjectIdOptionsRef<'a, 'f> {
    type Cell = ObjectIdOptions<'f>;
}

impl<'a, 'f: 'a> CMutSeam<'a> for ObjectIdOptionsMut<'a, 'f> {
    type Cell = ObjectIdOptions<'f>;
}

impl<'f> ObjectIdOptions<'f> {
    /// The structure version this crate's header declares
    /// (`GIT_OBJECT_ID_OPTIONS_VERSION`), the newest `git_object_id_options_init`
    /// accepts.
    pub const VERSION: u32 = ffi::GIT_OBJECT_ID_OPTIONS_VERSION;

    /// libgit2's default options, as `git_object_id_options_init` fills them for
    /// [`VERSION`](Self::VERSION) and `GIT_OBJECT_ID_OPTIONS_INIT` spells them: the
    /// default object type (blob) and id type (SHA-1), and no filter.
    ///
    /// `git_object_id_options_init` only fails for a version it does not know, and with
    /// its own it reaches no global, no allocator and no thread state, so this needs no
    /// initialised library.
    ///
    /// Wraps: `git_object_id_options_init`
    ///
    /// # Panics
    ///
    /// Panics only if libgit2 refuses the version of the options it defines itself, which it never
    /// does.
    #[must_use]
    pub fn new() -> Self {
        let mut raw = MaybeUninit::<ffi::git_object_id_options>::uninit();
        // SAFETY: `raw` is writable storage for one `git_object_id_options`, which the
        // routine overwrites whole with the template for this version (a memcpy);
        // nothing else reads it during the call.
        let rc = unsafe { ffi::git_object_id_options_init(raw.as_mut_ptr(), Self::VERSION) };
        assert_eq!(rc, 0, "git_object_id_options_init refused its own version");
        // SAFETY: a 0 return means the template was copied in: every field is
        // initialised, both types 0 and the filter list NULL.
        Self(unsafe { raw.assume_init() }, PhantomData)
    }

    /// Shared handle to the options — the getters, and the pointer for C calls that
    /// read them.
    #[must_use]
    pub fn as_ref(&self) -> ObjectIdOptionsRef<'_, 'f> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        ObjectIdOptionsRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the options — the setters, and the pointer for C calls that
    /// apply their filter list.
    pub fn as_mut(&mut self) -> ObjectIdOptionsMut<'_, 'f> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        ObjectIdOptionsMut(ObjectIdOptionsRef(ptr), PhantomData)
    }
}

impl Default for ObjectIdOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ObjectIdOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'f> ObjectIdOptionsRef<'a, 'f> {
    /// The structure's version, `GIT_OBJECT_ID_OPTIONS_VERSION`; fixed by
    /// [`ObjectIdOptions::new`], and read by no routine taking the options.
    ///
    /// Field: `git_object_id_options.version`
    #[must_use]
    pub fn version(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let version_ptr = unsafe { &raw const (*p).version };
        // SAFETY: a read through the shared handle's pointer to live options; any value
        // is a valid `u32`.
        unsafe { version_ptr.read() }
    }

    /// The type of the object whose content is hashed, which its header names;
    /// `None` means the default, a blob.
    ///
    /// Field: `git_object_id_options.object_type`
    #[must_use]
    pub fn object_type(&self) -> Option<ObjectType> {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let object_type_ptr = unsafe { &raw const (*p).object_type };
        // SAFETY: a read through the shared handle's pointer to live options; any value
        // is a valid `git_object_t`.
        let kind = unsafe { object_type_ptr.read() };
        // 0, as `new` leaves it and `set_object_type(None)` writes it, or a named type:
        // the only values the setter stores.
        ObjectType::from_raw(kind)
    }

    /// The type of id computed; `None` means the default, `GIT_OID_DEFAULT` (SHA-1).
    ///
    /// Field: `git_object_id_options.oid_type`
    #[must_use]
    pub fn oid_type(&self) -> Option<OidType> {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let oid_type_ptr = unsafe { &raw const (*p).oid_type };
        // SAFETY: a read through the shared handle's pointer to live options; any value
        // is a valid `git_oid_t`.
        let kind = unsafe { oid_type_ptr.read() };
        OidType::from_raw(kind)
    }

    /// The filters a blob's content is run through before it is hashed, as the
    /// options borrow them; `None` means none. libgit2 ignores them for any other
    /// object type, and when the list is empty.
    ///
    /// The handle lives no longer than this shared borrow of the options, which keep
    /// the list exclusively for `'f`.
    ///
    /// Field: `git_object_id_options.filters`
    #[must_use]
    pub fn filters(&self) -> Option<FilterListRef<'a, 'f>> {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let filters_ptr = unsafe { &raw const (*p).filters };
        // SAFETY: a read through the shared handle's pointer to live options.
        let list = unsafe { filters_ptr.read() };
        // SAFETY: the field is NULL (as `new` leaves it) or was stored by `set_filters`
        // from a `FilterListMut<'f, 'r>`: a live list, built for a repository that
        // outlives `'r`, and so `'f`, lent to the options alone for `'f`, which outlives
        // `'a`; while the options are shared for `'a` nothing writes it, since every
        // write goes through `ObjectIdOptionsMut`. `FilterList<'r>` is covariant in
        // `'r`, so the list is a valid `FilterList<'f>`.
        unsafe { FilterListRef::from_ptr(list) }
    }
}

impl fmt::Debug for ObjectIdOptionsRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObjectIdOptions")
            .field("version", &self.version())
            .field("object_type", &self.object_type())
            .field("oid_type", &self.oid_type())
            .field("filters", &self.filters())
            .finish()
    }
}

impl<'f> ObjectIdOptionsMut<'_, 'f> {
    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> ObjectIdOptionsRef<'_, 'f> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> ObjectIdOptionsMut<'_, 'f> {
        ObjectIdOptionsMut(self.0, PhantomData)
    }

    /// Sets the type of the object whose content is hashed; `None` means the default,
    /// a blob.
    ///
    /// Field: `git_object_id_options.object_type`
    pub fn set_object_type(&mut self, kind: Option<ObjectType>) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let object_type_ptr = unsafe { &raw mut (*p).object_type };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { object_type_ptr.write(kind.map_or(0, ObjectType::raw)) }
    }

    /// Sets the type of id computed; `None` means the default, SHA-1.
    ///
    /// Field: `git_object_id_options.oid_type`
    pub fn set_oid_type(&mut self, kind: Option<OidType>) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let oid_type_ptr = unsafe { &raw mut (*p).oid_type };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { oid_type_ptr.write(kind.map_or(0, OidType::raw)) }
    }

    /// Sets the filters a blob's content is run through before it is hashed; `None`
    /// means none. The previous list is only borrowed, so nothing is released.
    ///
    /// The list is borrowed exclusively for `'f`, and through it the repository it was
    /// built for, which then both outlive the options; no other handle to it is used
    /// meanwhile, since applying it writes its filters' state.
    ///
    /// Field: `git_object_id_options.filters`
    pub fn set_filters<'r>(&mut self, filters: Option<FilterListMut<'f, 'r>>) {
        let list = filters.map_or(ptr::null_mut(), |mut l| l.as_mut_ptr());
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let filters_ptr = unsafe { &raw mut (*p).filters };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop. The list is NULL or one a
        // `FilterListMut<'f, 'r>` (`'r: 'f`) vouches for, moved into the options, as
        // `filters()` requires.
        unsafe { filters_ptr.write(list) }
    }
}

impl fmt::Debug for ObjectIdOptionsMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// `ObjectIdOptions` is the C struct itself, and each handle one pointer to it.
const _: () = assert!(
    size_of::<ObjectIdOptions<'static>>() == size_of::<ffi::git_object_id_options>()
        && align_of::<ObjectIdOptions<'static>>() == align_of::<ffi::git_object_id_options>()
        && size_of::<ObjectIdOptionsMut<'static, 'static>>() == size_of::<*mut ()>()
);

impl ObjectType {
    /// The type's name, as object headers spell it: `"commit"`, `"tree"`, `"blob"` or
    /// `"tag"`. The string is libgit2's, static.
    ///
    /// ```
    /// # if cfg!(miri) { return; } // Miri cannot call libgit2.
    /// use libgit2::libgit2::src::libgit2::object::ObjectType;
    ///
    /// assert_eq!(ObjectType::Commit.name(), "commit");
    /// assert_eq!(ObjectType::from_name(c"tag"), Some(ObjectType::Tag));
    /// ```
    ///
    /// Wraps: `git_object_type2string`
    ///
    /// # Panics
    ///
    /// Panics, with `libgit2's object type names are ASCII`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn name(self) -> &'static str {
        // SAFETY: no pointer argument; libgit2 returns a string of its static table of
        // object types, NUL-terminated, never freed or written.
        let object_type2string = unsafe { ffi::git_object_type2string(self.raw()) };
        // SAFETY: as above.
        let name = unsafe { CStr::from_ptr(object_type2string) };
        name.to_str()
            .expect("libgit2's object type names are ASCII")
    }

    /// The type whose [`name`](Self::name) `name` starts with, as libgit2 matches it:
    /// `"commit"`, and anything longer that starts with it such as `"commits"`, is
    /// [`Commit`](Self::Commit). The match is case-sensitive; `None` for any other
    /// name, the empty one included.
    ///
    /// Wraps: `git_object_string2type`
    #[must_use]
    pub fn from_name(name: &CStr) -> Option<Self> {
        // SAFETY: `name` is a NUL-terminated string, read up to its NUL for the call
        // only; the routine touches nothing else.
        let raw = unsafe { ffi::git_object_string2type(name.as_ptr()) };
        Self::from_raw(raw)
    }
}

/// Whether `raw` is the C value of an object's type: one of the four
/// [`ObjectType`]s. Every other value is not, `GIT_OBJECT_ANY` and
/// `GIT_OBJECT_INVALID` included; a valid value converts with [`ObjectType::from_raw`].
///
/// Wraps: `git_object_type_is_valid`
#[must_use]
pub fn type_is_valid(raw: ffi::git_object_t) -> bool {
    // SAFETY: a plain value, checked against libgit2's static table of object types.
    unsafe { ffi::git_object_type_is_valid(raw) != 0 }
}

/// Whether `content` is the well-formed raw content (no `<type> <size>` header) of an
/// object of type `kind`, whose object ids are of type `oid_type`: a commit, tree or
/// tag is parsed as libgit2 parses one read from an object database, and every blob is
/// valid. No repository is involved: the ids `content` names need not exist anywhere.
///
/// libgit2 reports malformed content as `Ok(false)`, and leaves the parser's error, which
/// says what is wrong, as the calling thread's last error ([`last`] reads it), as the
/// header documents; a blob, and well-formed content, leave the last error alone.
///
/// ```
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// use libgit2::libgit2::src::libgit2::object::{ObjectType, content_is_valid};
/// use libgit2::libgit2::src::libgit2::oid::OidType;
///
/// let tag = b"object a65fedf39aefe402d3bb6e24df4d4f5fe4547750\ntype commit\ntag v1\n";
/// assert_eq!(
///     content_is_valid(tag, ObjectType::Tag, OidType::Sha1),
///     Ok(true)
/// );
/// assert_eq!(
///     content_is_valid(tag, ObjectType::Tag, OidType::Sha256),
///     Ok(false)
/// );
/// assert_eq!(
///     content_is_valid(b"", ObjectType::Commit, OidType::Sha1),
///     Ok(false)
/// );
/// assert_eq!(
///     content_is_valid(b"", ObjectType::Tree, OidType::Sha1),
///     Ok(true)
/// );
/// assert_eq!(
///     content_is_valid(b"\xff", ObjectType::Blob, OidType::Sha1),
///     Ok(true)
/// );
/// ```
///
/// Wraps: `git_object_rawcontent_is_valid`
///
/// # Errors
///
/// Fails, with libgit2's error, when libgit2 runs out of memory parsing, or cannot be
/// initialised.
pub fn content_is_valid(
    content: &[u8],
    kind: ObjectType,
    oid_type: OidType,
) -> Result<bool, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    // libgit2 refuses a NULL buffer whatever the length; an empty slice's dangling
    // pointer would pass, but nothing is to be read from it, so lend a real byte.
    let buf = if content.is_empty() {
        c"".as_ptr()
    } else {
        content.as_ptr().cast()
    };
    let mut valid = 0;
    // SAFETY: `valid` is a writable `int`, written on success; `buf` addresses
    // `content.len()` readable bytes (or one, for no content), which libgit2 hashes and
    // parses for the call only: the object it parses them into, whose tree entries
    // point into them, is freed before it returns. `kind` and `oid_type` are named
    // types. The runtime is pinned, so the object's allocation reads a settled
    // allocator and a failure is reported on live thread-local state.
    let rc = unsafe {
        ffi::git_object_rawcontent_is_valid(
            &raw mut valid,
            buf,
            content.len(),
            kind.raw(),
            oid_type.raw(),
        )
    };
    if rc < 0 {
        return Err(last());
    }
    Ok(valid != 0)
}

/// The id `data` would have as the raw content (no `<type> <size>` header) of an object,
/// as `opts` say (see [`ObjectIdOptions`]): of the type they name, a blob by default,
/// hashed into an id of the type they name, SHA-1 by default, a blob's content first
/// run through their filters, if they have any. `None` means the default options.
/// Nothing is written, and no repository is read but the one the filters were loaded
/// for.
///
/// The options are taken exclusively: applying their filter list writes its filters'
/// state.
///
/// ```
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// use libgit2::libgit2::src::libgit2::object::{ObjectIdOptions, ObjectType, id_from_buffer};
/// use libgit2::libgit2::src::libgit2::oid::OidType;
///
/// let blob = id_from_buffer(b"hello\n", None).unwrap();
/// assert_eq!(
///     blob.as_ref().to_hex_string().unwrap(),
///     "ce013625030ba8dba906f756967f9e9ca394464a"
/// );
///
/// let mut opts = ObjectIdOptions::new();
/// opts.as_mut().set_object_type(Some(ObjectType::Tree));
/// opts.as_mut().set_oid_type(Some(OidType::Sha256));
/// let tree = id_from_buffer(b"", Some(opts.as_mut())).unwrap();
/// assert_eq!(tree.as_ref().kind(), Some(OidType::Sha256));
/// ```
///
/// Wraps: `git_object_id_from_buffer`
///
/// # Errors
///
/// Fails, with the filter's code and error, when a filter fails, as a CRLF filter does
/// under `core.safecrlf`; and with libgit2's error when it runs out of memory, or
/// cannot be initialised.
pub fn id_from_buffer(
    data: &[u8],
    opts: Option<ObjectIdOptionsMut<'_, '_>>,
) -> Result<Oid, RepositoryError> {
    if !runtime_pinned() {
        return Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()));
    }
    // An empty slice's pointer is dangling, and libgit2 hands it to the hash and the
    // filters whatever the length: lend a live byte instead.
    let buf = if data.is_empty() {
        c"".as_ptr()
    } else {
        data.as_ptr().cast()
    };
    let opts = opts.map_or(ptr::null(), |mut o| o.as_mut_ptr().cast_const());
    let mut id = Oid::zeroed();
    let out = id.as_mut().as_mut_ptr();
    // SAFETY: `out` addresses `id`, written only on success; `buf` addresses `data.len()`
    // readable bytes (or one, for no data), read for the call; `opts` is NULL or live
    // options borrowed exclusively, holding named types (all the setters store) and a
    // NULL list or one lent to them alone, which libgit2 applies on this thread. The
    // runtime is pinned, so allocations and errors reach live state.
    let rc = unsafe { ffi::git_object_id_from_buffer(out, buf.cast(), data.len(), opts) };
    if rc < 0 {
        Err(RepositoryError::from_libgit2(rc))
    } else {
        Ok(id)
    }
}

/// The id the content of the file at `path` would have as the raw content of an
/// object, as `opts` say, read and hashed as [`id_from_buffer`] hashes a buffer. A
/// relative `path` is taken from the current directory.
///
/// No attribute or repository setting applies, only the options' filters: so a blob's
/// id may differ from the one adding the file to a repository gives, which
/// [`RepositoryRef::hash_file`](crate::libgit2::src::libgit2::repository::RepositoryRef::hash_file)
/// computes.
///
/// Wraps: `git_object_id_from_file`
///
/// # Errors
///
/// Fails with [`ErrorCode::NotFound`] (class `GIT_ERROR_OS`) when there is no such
/// file; with libgit2's error (`GIT_ERROR_OS`) when it cannot be opened or read in
/// full (a directory, say, or a file that shrinks meanwhile); and as
/// [`id_from_buffer`] does otherwise.
pub fn id_from_file(
    path: &CStr,
    opts: Option<ObjectIdOptionsMut<'_, '_>>,
) -> Result<Oid, RepositoryError> {
    if !runtime_pinned() {
        return Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()));
    }
    let opts = opts.map_or(ptr::null(), |mut o| o.as_mut_ptr().cast_const());
    let mut id = Oid::zeroed();
    let out = id.as_mut().as_mut_ptr();
    // SAFETY: `out` addresses `id`, written only on success; `path` is NUL-terminated
    // and read for the call; `opts` as in `id_from_buffer`. libgit2 opens, reads and
    // closes the file within the call. The runtime is pinned.
    let rc = unsafe { ffi::git_object_id_from_file(out, path.as_ptr(), opts) };
    if rc < 0 {
        Err(RepositoryError::from_libgit2(rc))
    } else {
        Ok(id)
    }
}

// An `Object` is a `git_object`, never held by value; the owner is one pointer.
const _: () = assert!(
    size_of::<Object<'static>>() == size_of::<ffi::git_object>()
        && size_of::<ObjectArc<'static>>() == size_of::<*mut ffi::git_object>()
        && size_of::<Option<ObjectArc<'static>>>() == size_of::<*mut ffi::git_object>()
);

#[cfg(test)]
mod unit_tests {
    use core::ffi::CStr;
    use core::mem::size_of;
    use std::ffi::CString;

    use super::*;
    use crate::libgit2::src::libgit2::oid::{Oid, OidType};
    use crate::libgit2::src::libgit2::repository::{Repository, RepositoryBox};
    use crate::libgit2::src::util::errors::runtime_pinned;

    /// `testrepo.git`: `HEAD` at commit `a65fedf`, whose tree is `944c0f6`.
    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/testrepo.git\0"
    );
    const COMMIT: &CStr = c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
    const TREE: &CStr = c"944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";

    fn open() -> RepositoryBox {
        assert!(runtime_pinned());
        Repository::open(CStr::from_bytes_with_nul(TESTREPO.as_bytes()).unwrap()).unwrap()
    }

    fn sha1(hex: &CStr) -> Oid {
        Oid::from_string(hex, OidType::Sha1).unwrap()
    }

    /// C's id and type of a live object.
    fn view(object: ObjectRef<'_, '_>) -> (String, ffi::git_object_t) {
        let p = object.as_ptr();
        {
            // SAFETY: `p` is a live object the handle borrows; its id lives as long as it
            // and is formatted at once.
            let object_id = unsafe { ffi::git_object_id(p) };
            // SAFETY: as above.
            let id = unsafe { OidRef::from_ptr(object_id.cast_mut()).unwrap() };
            // SAFETY: as above.
            unsafe { (id.to_hex_string().unwrap(), ffi::git_object_type(p)) }
        }
    }

    /// The refusal an id of another type than the repository's meets.
    fn assert_refused(result: Result<ObjectArc<'_>, RepositoryError>) {
        let e = result.unwrap_err();
        assert_eq!(e.raw_code(), -1);
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        assert_eq!(
            e.message(),
            c"specified object ID type does not match repository object ID type"
        );
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<Object<'_>>(), size_of::<ffi::git_object>());
        assert_eq!(
            size_of::<ObjectArc<'_>>(),
            size_of::<*mut ffi::git_object>()
        );
        assert_eq!(
            size_of::<ObjectRef<'_, '_>>(),
            size_of::<*const ffi::git_object>()
        );
        assert_eq!(
            size_of::<ObjectMut<'_, '_>>(),
            size_of::<*const ffi::git_object>()
        );
        assert_eq!(size_of::<ObjectFree>(), 0);
    }

    /// The handles' seams over a pointer nothing reads, so Miri runs them: `git_object`
    /// is opaque, of size zero, so a dangling aligned pointer addresses all of it.
    #[test]
    fn handles_without_c() {
        // SAFETY: NULL is refused without being read.
        assert!(unsafe { ObjectRef::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: as above.
        assert!(unsafe { ObjectMut::from_ptr(ptr::null_mut()) }.is_none());
        let p = NonNull::<ffi::git_object>::dangling().as_ptr();
        // SAFETY: a zero-sized object at an aligned pointer, never read or written, and
        // reached by this handle alone.
        let mut m = unsafe { ObjectMut::from_ptr(p) }.unwrap();
        assert_eq!(m.as_mut_ptr(), p);
        assert_eq!(m.as_mut().as_mut_ptr(), p);
        let r = m.as_ref();
        assert_eq!(r.as_ptr(), p.cast_const());
        assert_eq!(format!("{r:?}"), "Object { .. }");
        assert_eq!(format!("{m:?}"), "Object { .. }");
    }

    /// An id of another type than the repository's is refused before libgit2 formats
    /// it out of bounds (a full id), or formats nothing into a buffer it then reads (a
    /// prefix of an unnamed type), at every length; the same bytes typed as the
    /// repository's find the object. Corrects the C routines, which check nothing.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn ids_of_another_type_are_refused_before_c() {
        let repo = open();
        let repo = repo.as_ref();
        let commit = sha1(COMMIT);
        let mut wide = Oid::from_raw(&commit.as_ref().id(), OidType::Sha256).unwrap();
        let lens = [0, 3, 4, 7, 39, 40, 41, 64, usize::MAX];
        for kind in [u8::try_from(OidType::Sha256.raw()).unwrap(), 0, 9, u8::MAX] {
            wide.as_mut().set_raw_kind(kind);
            assert_refused(Object::lookup(repo, wide.as_ref(), None));
            assert_refused(Object::lookup(
                repo,
                wide.as_ref(),
                Some(ObjectType::Commit),
            ));
            for len in lens {
                assert_refused(Object::lookup_prefix(repo, wide.as_ref(), len, None));
            }
        }
        wide.as_mut().set_kind(OidType::Sha1);
        let expected = (COMMIT.to_str().unwrap().to_owned(), ffi::GIT_OBJECT_COMMIT);
        assert_eq!(
            view(Object::lookup(repo, wide.as_ref(), None).unwrap().as_ref()),
            expected
        );
        for len in [7, 39, 40, usize::MAX] {
            let found = Object::lookup_prefix(repo, wide.as_ref(), len, None).unwrap();
            assert_eq!(view(found.as_ref()), expected);
        }
    }

    /// The policy driven by hand: the up-ref counts one more reference on the same
    /// pointer, which an adopted owner and the free each settle; `to_object` takes one
    /// through a borrowed handle.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn policy_counts_references_on_one_object() {
        let repo = open();
        let tree = Object::lookup(repo.as_ref(), sha1(TREE).as_ref(), None).unwrap();
        let p = NonNull::new(tree.as_ptr()).unwrap();
        // SAFETY: `tree` holds a reference to the live object at `p`.
        assert!(unsafe { ObjectFree.c_up_ref(p) });
        // SAFETY: the reference just counted, given to the owner.
        let adopted = unsafe { ObjectArc::from_raw(p.as_ptr()) }.unwrap();
        assert!(ObjectArc::ptr_eq(&adopted, &tree));
        let raw = adopted.into_c();
        // SAFETY: `raw` is the reference `into_c` gave up, released once here.
        unsafe { ObjectFree.c_drop(NonNull::new(raw).unwrap().cast()) };

        // SAFETY: `tree` keeps the object alive while the handle lives.
        let borrowed = unsafe { ObjectRef::from_ptr(tree.as_c_ptr()) }.unwrap();
        let copy = borrowed.to_object();
        assert!(ObjectArc::ptr_eq(&copy, &tree));
        drop(tree);
        assert_eq!(
            view(copy.as_ref()),
            (TREE.to_str().unwrap().into(), ffi::GIT_OBJECT_TREE)
        );
    }

    /// A peel and a lookup by path derive an owned reference from a borrowed handle:
    /// the same object for a peel to its own type, otherwise one looked up through the
    /// repository; the policy settles each.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn peels_and_path_lookups_derive_owned_references() {
        let repo = open();
        let commit = Object::lookup(repo.as_ref(), sha1(COMMIT).as_ref(), None).unwrap();
        let same = commit.as_ref().peel(Some(ObjectType::Commit)).unwrap();
        assert!(ObjectArc::ptr_eq(&same, &commit));
        let tree = commit.as_ref().peel(None).unwrap();
        assert_eq!(
            view(tree.as_ref()),
            (TREE.to_str().unwrap().into(), ffi::GIT_OBJECT_TREE)
        );
        assert!(ObjectArc::ptr_eq(
            &tree,
            &Object::lookup(repo.as_ref(), sha1(TREE).as_ref(), None).unwrap()
        ));
        let e = tree.as_ref().peel(Some(ObjectType::Commit)).unwrap_err();
        assert_eq!(e.code(), Some(ErrorCode::InvalidSpec));
        assert_eq!(e.class(), Some(ErrorClass::Object));

        let readme = commit.as_ref().lookup_bypath(c"README", None).unwrap();
        let from_tree = tree
            .as_ref()
            .lookup_bypath(c"README", Some(ObjectType::Blob))
            .unwrap();
        assert_eq!(view(readme.as_ref()), view(from_tree.as_ref()));
        assert_eq!(view(readme.as_ref()).1, ffi::GIT_OBJECT_BLOB);
        let e = commit
            .as_ref()
            .lookup_bypath(c"README", Some(ObjectType::Tree))
            .unwrap_err();
        assert_eq!(e.code(), Some(ErrorCode::InvalidSpec));
        let e = commit.as_ref().lookup_bypath(c"nope", None).unwrap_err();
        assert_eq!(e.code(), Some(ErrorCode::NotFound));
        assert_eq!(e.class(), Some(ErrorClass::Tree));
        drop((commit, same, tree));
        assert_eq!(view(readme.as_ref()).1, ffi::GIT_OBJECT_BLOB);
    }

    /// Every type's name is libgit2's and reads back as the type; a name is matched as a
    /// prefix, case-sensitively; only the four types' values are valid, as `from_raw`
    /// agrees.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn type_names_round_trip() {
        for (kind, name) in [
            (ObjectType::Commit, "commit"),
            (ObjectType::Tree, "tree"),
            (ObjectType::Blob, "blob"),
            (ObjectType::Tag, "tag"),
        ] {
            assert_eq!(kind.name(), name);
            let c_name = CString::new(name).unwrap();
            assert_eq!(ObjectType::from_name(&c_name), Some(kind));
            let longer = CString::new(format!("{name}s")).unwrap();
            assert_eq!(ObjectType::from_name(&longer), Some(kind));
            let upper = CString::new(name.to_uppercase()).unwrap();
            assert_eq!(ObjectType::from_name(&upper), None);
        }
        for name in [c"", c"tre", c"ofs-delta", c" commit", c"any"] {
            assert_eq!(ObjectType::from_name(name), None, "{name:?}");
        }
        for raw in [i32::MIN, -3, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, i32::MAX] {
            assert_eq!(
                type_is_valid(raw),
                ObjectType::from_raw(raw).is_some(),
                "{raw}"
            );
        }
    }

    /// An object's type is the one it was parsed as, through every owner and handle.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn kinds_are_the_parsed_types() {
        let repo = open();
        for (hex, kind) in [
            (COMMIT, ObjectType::Commit),
            (TREE, ObjectType::Tree),
            (
                c"a8233120f6ad708f843d861ce2b7228ec4e3dec6",
                ObjectType::Blob,
            ),
            (c"b25fa35b38051e4ae45d4222e795f9df2e43f1d1", ObjectType::Tag),
        ] {
            let object = Object::lookup(repo.as_ref(), sha1(hex).as_ref(), None).unwrap();
            assert_eq!(object.as_ref().kind(), kind);
            assert_eq!(object.clone().as_ref().kind(), kind);
            assert_eq!(view(object.as_ref()).1, kind.raw());
        }
    }

    /// Copies `from` into `to`, recursively.
    fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// A copy of `testrepo.git` whose configuration sets `core.abbrev` to `abbrev`
    /// (none for `None`), removed on drop.
    struct Abbreviating(std::path::PathBuf);

    impl Abbreviating {
        fn new(tag: &str, abbrev: Option<&str>) -> Self {
            use std::fmt::Write as _;
            let path = std::env::temp_dir()
                .join(format!("crustify-object-unit-{}-{tag}", std::process::id()));
            drop(std::fs::remove_dir_all(&path));
            let source = TESTREPO.trim_end_matches('\0');
            copy_dir(std::path::Path::new(source), &path);
            if let Some(abbrev) = abbrev {
                let config = path.join("config");
                let mut text = std::fs::read_to_string(&config).unwrap();
                let _ = write!(text, "[core]\n\tabbrev = {abbrev}\n");
                std::fs::write(&config, text).unwrap();
            }
            Self(path)
        }

        fn open(&self) -> RepositoryBox {
            assert!(runtime_pinned());
            let path = CString::new(self.0.to_str().unwrap()).unwrap();
            Repository::open(&path).unwrap()
        }
    }

    impl Drop for Abbreviating {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    /// The short id starts at `core.abbrev` digits (7 by default), grows past an
    /// ambiguous prefix (`498bc` names a commit and a blob), is the full id for
    /// `core.abbrev=no`, and fails for a setting under 4.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn short_ids_follow_core_abbrev() {
        const AMBIGUOUS: &CStr = c"498bc0906810bd43c6fbc73385fecb7f2d04be3a";
        let short = |repo: &RepositoryBox, hex: &CStr| {
            let object = Object::lookup(repo.as_ref(), sha1(hex).as_ref(), None).unwrap();
            let id = object.as_ref().short_id().unwrap();
            id.as_ref().to_str().unwrap().to_owned()
        };

        let repo = open();
        assert_eq!(short(&repo, COMMIT), "a65fedf");
        assert_eq!(short(&repo, AMBIGUOUS), "498bc09");

        let four = Abbreviating::new("abbrev-4", Some("4"));
        let repo = four.open();
        assert_eq!(short(&repo, COMMIT), "a65f");
        assert_eq!(short(&repo, AMBIGUOUS), "498bc0");

        let full = Abbreviating::new("abbrev-no", Some("no"));
        let repo = full.open();
        assert_eq!(short(&repo, COMMIT), COMMIT.to_str().unwrap());

        let three = Abbreviating::new("abbrev-3", Some("3"));
        let repo = three.open();
        let object = Object::lookup(repo.as_ref(), sha1(COMMIT).as_ref(), None).unwrap();
        let e = object.as_ref().short_id().unwrap_err();
        assert_eq!(e.raw_code(), -1);
        assert_eq!(e.class(), Some(ErrorClass::Config));
        assert_eq!(e.message(), c"invalid oid abbreviation setting: '3'");
    }

    /// Blobs are valid without a parse and leave the last error alone; malformed
    /// content is `Ok(false)` with the parser's error; no content is read through a real
    /// byte, and is a valid (empty) tree but no valid commit or tag.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn content_validity_is_the_parsers() {
        assert!(runtime_pinned());
        let tag = b"object a65fedf39aefe402d3bb6e24df4d4f5fe4547750\ntype commit\ntag v1\n";
        set(ErrorClass::Invalid, c"untouched");
        for kind in [OidType::Sha1, OidType::Sha256] {
            assert_eq!(content_is_valid(b"", ObjectType::Blob, kind), Ok(true));
            assert_eq!(
                content_is_valid(b"\0\xff", ObjectType::Blob, kind),
                Ok(true)
            );
        }
        assert_eq!(
            content_is_valid(tag, ObjectType::Tag, OidType::Sha1),
            Ok(true)
        );
        assert_eq!(
            content_is_valid(b"", ObjectType::Tree, OidType::Sha1),
            Ok(true)
        );
        assert_eq!(last().message(), c"untouched");

        assert_eq!(
            content_is_valid(tag, ObjectType::Tag, OidType::Sha256),
            Ok(false)
        );
        let e = last();
        assert_eq!(e.class(), Some(ErrorClass::Tag));
        assert_eq!(e.message(), c"failed to parse tag: object field invalid");
        assert_eq!(
            content_is_valid(b"", ObjectType::Commit, OidType::Sha1),
            Ok(false)
        );
        assert_eq!(
            content_is_valid(b"", ObjectType::Tag, OidType::Sha1),
            Ok(false)
        );
        assert_eq!(
            content_is_valid(tag, ObjectType::Commit, OidType::Sha1),
            Ok(false)
        );
        assert_eq!(
            content_is_valid(&tag[..tag.len() - 1], ObjectType::Tag, OidType::Sha1),
            Ok(false)
        );
    }

    /// The cache's reference is never visible, so an owner never proves itself sole:
    /// no exclusive handle, no box.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn an_owner_is_never_sole() {
        let repo = open();
        let mut commit = Object::lookup(repo.as_ref(), sha1(COMMIT).as_ref(), None).unwrap();
        assert!(commit.get_mut().is_none());
        let commit = commit.try_into_box().unwrap_err();
        assert_eq!(format!("{:?}", commit.as_ref()), "Object { .. }");
    }

    /// The options as `GIT_OBJECT_ID_OPTIONS_INIT` spells them, without C, for Miri.
    fn template() -> ObjectIdOptions<'static> {
        ObjectIdOptions(
            ffi::git_object_id_options {
                version: ObjectIdOptions::VERSION,
                object_type: 0,
                oid_type: 0,
                filters: ptr::null_mut(),
            },
            PhantomData,
        )
    }

    /// The raw fields of the options.
    fn fields(opts: ObjectIdOptionsRef<'_, '_>) -> (u32, i32, u32, *mut ffi::git_filter_list) {
        let p = opts.as_ptr();
        // SAFETY: a live value the handle borrows, copied out field by field.
        let version2 = unsafe { (*p).version };
        // SAFETY: as above.
        let object_type2 = unsafe { (*p).object_type };
        // SAFETY: as above.
        let oid_type2 = unsafe { (*p).oid_type };
        // SAFETY: as above.
        unsafe { (version2, object_type2, oid_type2, (*p).filters) }
    }

    #[test]
    fn options_layout_matches_c() {
        assert_eq!(
            size_of::<ObjectIdOptions<'_>>(),
            size_of::<ffi::git_object_id_options>()
        );
        assert_eq!(
            align_of::<ObjectIdOptions<'_>>(),
            align_of::<ffi::git_object_id_options>()
        );
        assert_eq!(
            size_of::<ObjectIdOptionsRef<'_, '_>>(),
            size_of::<*const ()>()
        );
        assert_eq!(
            size_of::<ObjectIdOptionsMut<'_, '_>>(),
            size_of::<*const ()>()
        );
    }

    /// `new` (and `Default`) is `git_object_id_options_init`'s template, which C fills
    /// the same for every version it accepts and refuses, untouched, past it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_the_template() {
        let expected = (ObjectIdOptions::VERSION, 0, 0, ptr::null_mut());
        assert_eq!(fields(ObjectIdOptions::new().as_ref()), expected);
        assert_eq!(fields(ObjectIdOptions::default().as_ref()), expected);
        assert_eq!(fields(template().as_ref()), expected);
        let mut raw = ffi::git_object_id_options {
            version: 7,
            object_type: 9,
            oid_type: 9,
            filters: ptr::null_mut(),
        };
        assert!(runtime_pinned());
        // SAFETY: writable options; a refused version leaves them alone.
        let rc =
            unsafe { ffi::git_object_id_options_init(&raw mut raw, ObjectIdOptions::VERSION + 1) };
        assert_eq!((rc, raw.version, raw.object_type), (-1, 7, 9));
    }

    /// The setters write one field each, `None` writing the C default, and the getters
    /// read them back; the list stored is the very pointer lent, and only that. A
    /// dangling list pointer stands in for a list: `git_filter_list` is opaque, so
    /// nothing reads through it, and Miri runs this.
    #[test]
    fn accessors_round_trip_without_c() {
        let mut opts = template();
        assert_eq!(opts.as_ref().version(), 1);
        assert_eq!(opts.as_ref().object_type(), None);
        assert_eq!(opts.as_ref().oid_type(), None);
        assert!(opts.as_ref().filters().is_none());
        assert_eq!(
            format!("{opts:?}"),
            "ObjectIdOptions { version: 1, object_type: None, oid_type: None, filters: None }"
        );
        for kind in [
            ObjectType::Commit,
            ObjectType::Tree,
            ObjectType::Blob,
            ObjectType::Tag,
        ] {
            opts.as_mut().set_object_type(Some(kind));
            assert_eq!(opts.as_ref().object_type(), Some(kind));
            assert_eq!(fields(opts.as_ref()).1, kind.raw());
        }
        for kind in [OidType::Sha1, OidType::Sha256] {
            opts.as_mut().set_oid_type(Some(kind));
            assert_eq!(opts.as_ref().oid_type(), Some(kind));
            assert_eq!(fields(opts.as_ref()).2, kind.raw());
        }
        let list = NonNull::<ffi::git_filter_list>::dangling().as_ptr();
        // SAFETY: a zero-sized list at an aligned pointer, never read or written, and
        // reached through this handle alone for as long as the options live.
        let handle = unsafe { FilterListMut::from_ptr(list) }.unwrap();
        {
            let mut m = opts.as_mut();
            m.as_mut().set_filters(Some(handle));
            assert_eq!(m.as_ref().filters().unwrap().as_ptr(), list.cast_const());
            assert_eq!(m.as_mut().as_mut_ptr().cast_const(), m.as_ref().as_ptr());
            assert_eq!(
                format!("{m:?}"),
                "ObjectIdOptions { version: 1, object_type: Some(Tag), oid_type: Some(Sha256), \
                 filters: Some(FilterList { .. }) }"
            );
            m.set_object_type(None);
            m.set_oid_type(None);
            m.set_filters(None);
        }
        assert_eq!(fields(opts.as_ref()), (1, 0, 0, ptr::null_mut()));
    }

    /// libgit2's allocator for the tests below, which refuse it.
    mod refused {
        use core::ffi::c_void;
        use std::process::Command;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering::SeqCst;

        use super::super::*;
        use crate::libgit2::src::util::alloc::{Alloc, Allocator};
        use crate::libgit2::src::util::errors::ErrorClass;

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

        /// Whether [`Refusing`] refuses every request.
        static REFUSE: AtomicBool = AtomicBool::new(false);

        unsafe extern "C" {
            fn malloc(n: usize) -> *mut c_void;
            fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
            fn free(p: *mut c_void);
        }

        /// libc's allocator, refusing every request while [`REFUSE`] is set.
        struct Refusing;

        // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
        unsafe impl Alloc for Refusing {
            fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                if REFUSE.load(SeqCst) {
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
                if REFUSE.load(SeqCst) {
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

        /// Installs [`Refusing`] as libgit2's allocator, before anything is allocated.
        fn install_refusing() {
            let allocator = Allocator::new::<Refusing>();
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

        const COMMIT: &[u8] = b"tree 944c0f6e4dfa41595e6eb3ceecdb14f50fe18162\n\
            author A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\nm\n";

        /// A parse whose object cannot be allocated is libgit2's out-of-memory error, not
        /// invalid content; with the allocation granted the same content is valid. A blob
        /// needs no allocation.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn out_of_memory_is_an_error() {
            isolated(
                concat!(module_path!(), "::out_of_memory_is_an_error"),
                || {
                    install_refusing();
                    // The thread's error state, allocated now, so a refusal reports out
                    // of memory rather than a thread state it could not allocate.
                    set(ErrorClass::Invalid, c"error state allocated");
                    REFUSE.store(true, SeqCst);
                    let refused = content_is_valid(COMMIT, ObjectType::Commit, OidType::Sha1);
                    let blob = content_is_valid(COMMIT, ObjectType::Blob, OidType::Sha1);
                    REFUSE.store(false, SeqCst);
                    let e = refused.unwrap_err();
                    assert_eq!(
                        (e.class(), e.message()),
                        (Some(ErrorClass::NoMemory), c"Out of memory")
                    );
                    assert_eq!(blob, Ok(true));
                    assert_eq!(
                        content_is_valid(COMMIT, ObjectType::Commit, OidType::Sha1),
                        Ok(true)
                    );
                },
            );
        }

        /// When libgit2 cannot be initialised, the check reports it without calling
        /// `git_object_rawcontent_is_valid`.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn uninitialisable_library_is_reported() {
            isolated(
                concat!(module_path!(), "::uninitialisable_library_is_reported"),
                || {
                    install_refusing();
                    REFUSE.store(true, SeqCst);
                    let e = content_is_valid(b"", ObjectType::Blob, OidType::Sha1).unwrap_err();
                    REFUSE.store(false, SeqCst);
                    assert_eq!(
                        (e.class(), e.message()),
                        (
                            Some(ErrorClass::Invalid),
                            c"library has not been initialized"
                        )
                    );
                },
            );
        }
    }

    /// An empty buffer reaches C as one live byte with a length of 0, which hashes as
    /// the empty blob, through the default options and through none alike; a missing
    /// file keeps libgit2's code.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn ids_of_empty_buffers_lend_a_live_byte() {
        assert!(runtime_pinned());
        let hex = |id: Oid| id.as_ref().to_hex_string().unwrap();
        let empty = hex(id_from_buffer(&[], None).unwrap());
        assert_eq!(empty, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        let mut opts = ObjectIdOptions::new();
        assert_eq!(
            hex(id_from_buffer(&b"x"[..0], Some(opts.as_mut())).unwrap()),
            empty
        );
        let missing = id_from_file(c"/nonexistent/crustify", None).unwrap_err();
        assert_eq!(missing.code(), Some(ErrorCode::NotFound));
        assert_eq!(missing.class(), Some(ErrorClass::Os));
    }
}
