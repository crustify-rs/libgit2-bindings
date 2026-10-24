//! Safe wrappers over `include/git2/tag.h`.
//!
//! [`name_is_valid`] checks a tag name the way libgit2 checks the names it creates tags
//! under: the name must not start with `-` nor be `HEAD`, and `refs/tags/<name>` must be
//! a valid reference name. libgit2 builds that reference name in its allocator, so the
//! check pins libgit2's runtime first (initialising libgit2 on the first call), as every
//! wrapper that reaches libgit2's allocator or error state does.
//!
//! # Tag objects
//!
//! A [`Tag`] is a `git_tag`: an annotated tag, parsed from a repository's object
//! database. It is an [`Object`](crate::libgit2::src::libgit2::object::Object) of type tag:
//! the struct starts with the `git_object`, and `git_tag_lookup`, `git_tag_free` and
//! `git_tag_dup` are `git_object_lookup` (of type tag), `git_object_free` and
//! `git_object_dup` behind a cast. It is opaque: libgit2 publishes no field.
//!
//! So a tag is owned as an object is (see [`object`](crate::libgit2::src::libgit2::object)'s
//! module docs): a [`TagArc`] is one counted reference, given back by `git_tag_free`,
//! and its `Clone` takes another on the same object through `git_tag_dup`; the
//! repository's cache counts one more no owner sees, so no owner proves itself the only
//! one, and none lends [`TagMut`]. The parse fills the tag's name, tagger, message and
//! target before libgit2 hands the object out, and nothing writes them until the last
//! reference frees them with the tag (`git_tag__free`), so the shared [`TagRef`] reads
//! an object nobody changes. A tag carries the lifetime `'r` of the repository borrow it
//! was looked up through, whose cache it shares and which looking up its target reaches,
//! and is neither `Send` nor `Sync`, as an object is.
//!
//! The shared handle reads what the parse recorded, each borrowed in place for as long
//! as the handle: the tag's [`id`](TagRef::id), [`name`](TagRef::name),
//! [`tagger`](TagRef::tagger) and [`message`](TagRef::message) (either may be missing),
//! and its target's [`id`](TagRef::target_id) and [`type`](TagRef::target_kind). Its
//! [`owner`](TagRef::owner) is the repository handle it was looked up through, and
//! [`target`](TagRef::target) and [`peel`](TagRef::peel) look the target, or the end
//! of a chain of tags, up in that repository.
//!
//! A tag is an object, and an object of type tag is a tag: [`TagRef::as_object`] views
//! a tag as its object, [`TagRef::from_object`] views an object of type tag as a tag,
//! and [`Tag::from_object`] and [`Tag::into_object`] hand one counted reference from one
//! owner to the other, checking the object's type on the way in. libgit2 itself never
//! checks the type of what it is given as a tag.
//!
//! # Tags in a repository
//!
//! A tag is a reference under `refs/tags/`: a lightweight tag points at any object
//! ([`create_lightweight`]), an annotated one at a tag object that records its target,
//! name, tagger and message ([`create`] writes one from those, [`create_from_buffer`]
//! from its raw content, and [`create_annotation`] writes the object alone).
//! [`delete`] removes the reference, [`list`] and [`list_match`] list the tags'
//! names, and [`foreach`] walks the tags' references and ids. Each goes through the
//! repository's reference database (and, to create, its object database), shared
//! counted objects that libgit2 loads on first use, so each
//! takes the repository's shared handle; a repository without a git directory reaches
//! the files backend of its [`refdb`](RepositoryRef::refdb), which resolves
//! `refs/tags/` against the current directory unless a reference database was
//! installed. A [`ForeachCallback`] is the closure `git_tag_foreach` calls with each
//! tag. The annotated tag writers take the tagger as a [`SignatureRef`], which they
//! refuse without its name or its email: libgit2 prints both with `%s`.
//!
//! Two C defects shape the wrappers:
//!
//! - `git_tag_create_from_buffer` reads the tag's target with `git_odb_read`, which returns early
//!   for the zero id without writing its out-parameter, and then frees that uninitialised pointer
//!   (a use after free under `ASan`, once the stack slot holds a stale pointer). It also leaks the
//!   tag name and tagger it parsed when a later part of the buffer is malformed, and the reference
//!   name it built when looking the reference up fails other than as not found.
//!   [`create_from_buffer`] parses the buffer first ([`content_is_valid`]), whose parse frees what
//!   it allocated, refuses a zero target with the error `git_odb_read` sets for it, and answers an
//!   invalid reference name as libgit2 does, before libgit2 runs. A reference that is valid but
//!   cannot be read (a corrupt loose reference file) still leaks the name inside libgit2.
//! - `git_tag_list_match` (and `git_tag_list`) drops its partial list on failure, leaking the names
//!   collected so far, and returns 0 with an empty, NULL array; only the thread's last error tells
//!   the failure apart. [`list_match`] reports it as an error (code `-1`, with that last error),
//!   such as a symbolic reference under `refs/tags/` whose target is missing; the leak stays
//!   libgit2's.

use core::any::Any;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::fmt;
use core::marker::PhantomData;
use core::num::NonZero;
use core::ops::ControlFlow;
use core::ptr::{self, NonNull};
use std::ffi::CString;
use std::panic::{self, AssertUnwindSafe};

use ffibox::{CArc, CBorrowedPtr, CCell, CZeroable};

use crate::ffi;
use crate::libgit2::src::libgit2::object::{ObjectArc, ObjectRef, ObjectType, content_is_valid};
use crate::libgit2::src::libgit2::object_api::TagFree;
use crate::libgit2::src::libgit2::odb::OdbArc;
use crate::libgit2::src::libgit2::oid::{Oid, OidRef};
use crate::libgit2::src::libgit2::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::src::libgit2::signature::SignatureRef;
use crate::libgit2::src::libgit2::strarray::{StrArray, StrArrayVal};
use crate::libgit2::src::util::errors::{
    ErrorBuf, ErrorClass, ErrorCode, last, runtime_pinned, set,
};

/// Whether `name` is a valid tag name: it does not start with `-`, is not `HEAD`, and,
/// prefixed with `refs/tags/`, is a valid reference name (no empty, `.`-led or
/// `.lock`-ended component, no `..`, `@{`, control character, space or any of
/// `~^:?*[\`, and no trailing `.` or `/`).
///
/// The name is read for the call only. libgit2 reports an invalid name as `Ok(false)`,
/// but leaves a `GIT_ERROR_REFERENCE` error on the calling thread when the reference
/// name is what fails ("the given reference name '…' is not valid"); the wrapper keeps
/// that.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::src::libgit2::tag::name_is_valid;
///
/// assert_eq!(name_is_valid(c"v1.0"), Ok(true));
/// assert_eq!(name_is_valid(c"release/2024"), Ok(true));
/// assert_eq!(name_is_valid(c"-v1.0"), Ok(false));
/// assert_eq!(name_is_valid(c"HEAD"), Ok(false));
/// assert_eq!(name_is_valid(c"v1..0"), Ok(false));
/// ```
///
/// Wraps: `git_tag_name_is_valid`
///
/// # Errors
///
/// Fails, with libgit2's error, only when libgit2 runs out of memory building the
/// reference name, or cannot be initialised.
pub fn name_is_valid(name: &CStr) -> Result<bool, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut valid: c_int = 0;
    // SAFETY: `valid` is a writable `int`, which libgit2 writes before returning; `name`
    // is a non-NULL NUL-terminated string, read and copied for the call, never retained.
    // The runtime is pinned, so the allocation reads a settled allocator and a failure is
    // reported on live thread-local state.
    let rc = unsafe { ffi::git_tag_name_is_valid(&raw mut valid, name.as_ptr()) };
    if rc < 0 {
        return Err(last());
    }
    Ok(valid != 0)
}

/// An annotated tag (`git_tag`), parsed from a repository's object database: an
/// [`Object`](crate::libgit2::src::libgit2::object::Object) of type tag.
///
/// libgit2 allocates, counts and frees it, so it is only ever reached through a
/// [`TagArc`] and its handles, built by [`lookup`](Self::lookup) and
/// [`lookup_prefix`](Self::lookup_prefix), or adopted from an object owner by
/// [`from_object`](Self::from_object). It is opaque: no field is published.
///
/// `'r` is the borrow of the repository the tag was looked up in, as for an object (see
/// the [module docs](self#tag-objects)). The lifetime only shrinks:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::tag::TagArc;
///
/// fn widen<'r>(tag: TagArc<'r>) -> TagArc<'static> {
///     tag
/// }
/// ```
///
/// and a tag stays on the thread that looked it up:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::tag::TagArc;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(tag: TagArc<'static>) {
///     send(tag)
/// }
/// ```
///
/// Wraps: `git_tag`
/// Handwritten
#[repr(transparent)]
pub struct Tag<'r>(ffi::git_tag, PhantomData<(*const (), &'r ())>);

/// Shared borrow of a [`Tag`]. `Copy`, like `&T`.
///
/// It stays on the thread of the tag's repository, as the owner does:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::tag::TagRef;
///
/// fn sync<T: Sync>() {}
/// sync::<TagRef<'static, 'static>>();
/// ```
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct TagRef<'a, 'r>(CBorrowedPtr<'a, Tag<'r>>);

/// Exclusive borrow of a [`Tag`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// No owner hands one out: every tag is shared with its repository's cache.
///
/// Handwritten
#[repr(transparent)]
pub struct TagMut<'a, 'r>(
    TagRef<'a, 'r>,
    // Invariant in the tag, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Tag<'r>>,
);

// SAFETY: `Tag` is `#[repr(transparent)]` over `git_tag` with a zero-sized marker; both
// handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Tag<'r>>` (`TagMut` through
// `TagRef`) with zero-sized markers and no `Drop`, hand out no reference to the C
// object, and `TagRef` has no write operation. `TagMut` is invariant in the tag through
// its `PhantomData<&'a mut Tag<'r>>`.
unsafe impl<'r> CCell for Tag<'r> {
    type C = ffi::git_tag;
    type Mut<'a>
        = TagMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = TagRef<'a, 'r>
    where
        Self: 'a;
}

impl<'a, 'r> TagRef<'a, 'r> {
    /// The pointer the handle keeps, for the routines of other modules that count a
    /// reference on the tag (`object_api`'s `git_tag_dup`).
    pub(crate) fn as_non_null(self) -> NonNull<Tag<'r>> {
        self.0.as_non_null()
    }

    /// Borrows a raw pointer; `None` if null.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live tag looked up in a repository that outlives
    /// `'r`, which stays live during `'a` (a reference to it is held for at least that
    /// long) and which no one writes during `'a`.
    pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_tag) -> Option<Self> {
        // SAFETY: this function's contract is `CCell::ref_from_ptr`'s: `ptr` is NULL or a
        // live `git_tag` that outlives `'a` and is not written meanwhile.
        unsafe { <Tag<'r> as CCell>::ref_from_ptr(ptr.cast_mut()) }
    }

    /// Read-only pointer to the C object.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_tag {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The tag `object` is, borrowed as long as `object` is; `None` for an object of
    /// another type.
    #[must_use]
    pub fn from_object(object: ObjectRef<'a, 'r>) -> Option<Self> {
        if !is_tag(object) {
            return None;
        }
        // SAFETY: an object of type tag was allocated and parsed as a `git_tag`,
        // whose first member is that `git_object`, so the pointer addresses the whole
        // tag; the object handle keeps it live and unwritten for `'a`, and its
        // repository outlives `'r`.
        unsafe { Self::from_ptr(object.as_ptr().cast()) }
    }

    /// The tag as the object it is, borrowed as long as this handle: what every
    /// `git_object` routine takes.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a handle is never NULL"
    )]
    pub fn as_object(&self) -> ObjectRef<'a, 'r> {
        // SAFETY: a `git_tag` starts with its `git_object`, so the tag's pointer is its
        // object's, live and unwritten for `'a` as the tag is, of a repository that
        // outlives `'r`.
        let object = unsafe { ObjectRef::from_ptr(self.as_ptr().cast()) };
        object.expect("a handle is never NULL")
    }
}

/// Whether `object` is of type tag, as libgit2 recorded it when parsing it.
fn is_tag(object: ObjectRef<'_, '_>) -> bool {
    object.kind() == ObjectType::Tag
}

impl<'r> TagMut<'_, 'r> {
    /// Borrows a raw pointer exclusively; `None` if null.
    ///
    /// # Safety
    ///
    /// As [`TagRef::from_ptr`], plus: nothing else reaches the tag while the result
    /// lives, its repository's cache included.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_tag) -> Option<Self> {
        // SAFETY: the caller upholds `TagRef::from_ptr`'s contract and exclusivity.
        unsafe { TagRef::from_ptr(ptr) }.map(|r| TagMut(r, PhantomData))
    }

    /// Writable pointer to the C object.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_tag {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> TagRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> TagMut<'_, 'r> {
        TagMut(self.0, PhantomData)
    }
}

impl fmt::Debug for TagRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tag").finish_non_exhaustive()
    }
}

impl fmt::Debug for TagMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// One counted reference to a [`Tag`], given back by `git_tag_free` on drop; `clone`
/// takes another through `git_tag_dup`, on the same tag.
pub type TagArc<'r> = CArc<Tag<'r>, TagFree>;

impl<'r> Tag<'r> {
    /// `object`'s reference, as a tag's: the same object, still counted once, now owned
    /// as the tag it is; `object` back, untouched, when it is not of type tag.
    ///
    /// # Errors
    ///
    /// Gives `object` back, untouched, when it is not of type tag.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: an owner is never NULL"
    )]
    pub fn from_object(object: ObjectArc<'r>) -> Result<TagArc<'r>, ObjectArc<'r>> {
        if !is_tag(object.as_ref()) {
            return Err(object);
        }
        let raw = object.into_c();
        // SAFETY: the reference the object owner gave up, to a live object of type tag,
        // so a `git_tag`, of a repository borrowed for `'r`; `git_tag_free` releases it
        // as `git_object_free` would.
        let tag = unsafe { TagArc::from_c(raw.cast()) };
        Ok(tag.expect("an owner is never NULL"))
    }

    /// `tag`'s reference, as an object's: the same tag, still counted once, now owned
    /// as an object.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: an owner is never NULL"
    )]
    pub fn into_object(tag: TagArc<'r>) -> ObjectArc<'r> {
        let raw = tag.into_c();
        // SAFETY: the reference the tag owner gave up, to a live tag, which starts with
        // its `git_object`, of a repository borrowed for `'r`; `git_object_free`
        // releases it as `git_tag_free` would.
        let object = unsafe { ObjectArc::from_c(raw.cast()) };
        object.expect("an owner is never NULL")
    }
}

impl<'a, 'r> TagRef<'a, 'r> {
    /// The tag's name, as its `tag` header records it (`v1.0`, not `refs/tags/v1.0`),
    /// up to its first NUL. Borrowed in place for as long as the handle: the parse
    /// copies it into the tag, which frees it with itself, so it outlives no owner:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::tag::Tag;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let tag = Tag::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let name = tag.as_ref().name();
    /// drop(tag);
    /// let _ = name;
    /// ```
    ///
    /// Wraps: `git_tag_name`
    ///
    /// # Panics
    ///
    /// Panics, with `a parsed tag always has a name`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn name(&self) -> &'a CStr {
        // SAFETY: the handle addresses a live tag for `'a`, which libgit2 only reads.
        let name = unsafe { ffi::git_tag_name(self.as_ptr()) };
        assert!(!name.is_null(), "a parsed tag always has a name");
        // SAFETY: a NUL-terminated copy the tag owns, written by the parse before the
        // tag was handed out and freed with it, so live and unchanged for `'a`.
        unsafe { CStr::from_ptr(name) }
    }

    /// The tag's message: everything past the blank line ending its header, up to its
    /// first NUL; `None` when the content ends with the header. Borrowed in place for
    /// as long as the handle.
    ///
    /// Wraps: `git_tag_message`
    #[must_use]
    pub fn message(&self) -> Option<&'a CStr> {
        // SAFETY: the handle addresses a live tag for `'a`, which libgit2 only reads.
        let message = unsafe { ffi::git_tag_message(self.as_ptr()) };
        // SAFETY: NULL, or a NUL-terminated copy the tag owns, written by the parse
        // before the tag was handed out and freed with it, so live and unchanged for
        // `'a`.
        (!message.is_null()).then(|| unsafe { CStr::from_ptr(message) })
    }

    /// Who made the tag, and when, as its `tagger` header records it; `None` for a tag
    /// without one (git once wrote such tags). Borrowed in place for as long as the
    /// handle.
    ///
    /// The tagger's name or email reads as `None` only where libgit2 ran out of memory
    /// copying it while it parsed the tag (see [`SignatureRef::name`]).
    /// [`to_signature`](SignatureRef::to_signature) copies the tagger to keep it past
    /// the tag:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::tag::Tag;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let tag = Tag::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let tagger = tag.as_ref().tagger();
    /// drop(tag);
    /// let _ = tagger;
    /// ```
    ///
    /// Wraps: `git_tag_tagger`
    #[must_use]
    pub fn tagger(&self) -> Option<SignatureRef<'a>> {
        // SAFETY: the handle addresses a live tag for `'a`, which libgit2 only reads.
        let tagger = unsafe { ffi::git_tag_tagger(self.as_ptr()) };
        // SAFETY: NULL, or the signature the tag owns, which the parse filled before the
        // tag was handed out and which is freed with it: live and unchanged for `'a`.
        // Its name and email are each NULL (the parse ran out of memory) or a
        // NUL-terminated copy it owns, as the handles' invariant requires.
        unsafe { SignatureRef::from_ptr(tagger.cast_mut()) }
    }

    /// The id of the object the tag points at, as its `object` header records it, of
    /// the repository's id type. Borrowed in place for as long as the handle.
    ///
    /// Wraps: `git_tag_target_id`
    ///
    /// # Panics
    ///
    /// Panics, with `a live tag has a target id`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn target_id(&self) -> OidRef<'a> {
        // SAFETY: the handle addresses a live tag for `'a`; libgit2 only reads it and
        // returns the address of its `target` member.
        let id = unsafe { ffi::git_tag_target_id(self.as_ptr()) };
        // SAFETY: a member of the tag, written by the parse before the tag was handed
        // out, so non-NULL, live and unchanged for `'a`.
        unsafe { OidRef::from_ptr(id.cast_mut()) }.expect("a live tag has a target id")
    }

    /// The type of the object the tag points at, as its `type` header records it.
    ///
    /// Wraps: `git_tag_target_type`
    ///
    /// # Panics
    ///
    /// Panics, with `a parsed tag records a named target type`, only if libgit2 breaks its
    /// contract.
    #[must_use]
    pub fn target_kind(&self) -> ObjectType {
        // SAFETY: the handle addresses a live tag for `'a`, which libgit2 only reads.
        let raw = unsafe { ffi::git_tag_target_type(self.as_ptr()) };
        // The parse fails unless the header names one of the four types.
        ObjectType::from_raw(raw).expect("a parsed tag records a named target type")
    }

    /// The object the tag points at, looked up in the tag's repository by
    /// [`target_id`](Self::target_id) as of type [`target_kind`](Self::target_kind):
    /// one more counted reference of the repository's cache, or a fresh parse, living
    /// no longer than the repository's borrow `'r`, as the tag does.
    ///
    /// Wraps: `git_tag_target`
    ///
    /// # Errors
    ///
    /// Fails as [`Object::lookup`](crate::libgit2::src::libgit2::object::Object::lookup)
    /// does: with [`ErrorCode::NotFound`] when the object database lacks the target or
    /// holds it with another type than the tag records (class `GIT_ERROR_INVALID`, "the
    /// requested type does not match the type in the ODB"), and with libgit2's error
    /// when it cannot be read or parsed. The id needs no check: libgit2 parsed it with
    /// the repository's id type.
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without an object`, only if libgit2 breaks its contract.
    pub fn target(&self) -> Result<ObjectArc<'r>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; the handle addresses a
        // live tag for `'a`, whose repository, borrowed for `'r`, is live and used on
        // this thread only, as `RepositoryRef` allows: libgit2 looks the target up
        // through it, filling its cache, and only reads the tag, whose target id is of
        // the repository's type (the parse read it so).
        let rc = unsafe { ffi::git_tag_target(&raw mut out, self.as_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live object of the
        // same repository, borrowed for `'r`, given to the result and released by
        // `git_object_free`.
        let object = unsafe { ObjectArc::from_c(out) };
        Ok(object.expect("a lookup succeeded without an object"))
    }

    /// The first object of the tag's chain that is no tag: its target, or, for a tag
    /// of a tag, that tag's target, and so on. [`ObjectRef::peel`] of the tag's object
    /// with `None`; the result lives no longer than the repository's borrow `'r`.
    ///
    /// Wraps: `git_tag_peel`
    ///
    /// # Errors
    ///
    /// Fails, as that peel does, with libgit2's error when a target cannot be read
    /// from the object database or is not of the type its tag records.
    ///
    /// # Panics
    ///
    /// Panics, with `a peel succeeded without an object`, only if libgit2 breaks its contract.
    pub fn peel(&self) -> Result<ObjectArc<'r>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: as in `target`: `out` is a writable slot, the handle addresses a live
        // tag whose repository is live for `'r` and used on this thread only, where
        // libgit2 looks each target up (`git_object_peel` with `GIT_OBJECT_ANY`).
        let rc = unsafe { ffi::git_tag_peel(&raw mut out, self.as_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: as in `target`: one counted reference to a live object of the
        // repository, given to the result.
        let object = unsafe { ObjectArc::from_c(out) };
        Ok(object.expect("a peel succeeded without an object"))
    }
}

/// Creates the annotated tag whose tag object's raw content is `buffer`: writes the
/// object into `repo`'s object database, and points `refs/tags/<name>` at it, `<name>`
/// being the name the content records. Returns the tag object's id.
///
/// The content must parse as a tag of `repo`'s object id type ([`content_is_valid`]),
/// and its target exist in `repo` with the type the content records. The name is not
/// checked as [`create_lightweight`] checks one: only `refs/tags/<name>` must be a
/// valid reference name.
///
/// Wraps: `git_tag_create_from_buffer`
///
/// # Errors
///
/// Fails with [`ErrorCode::Exists`] (`GIT_ERROR_TAG`, "tag already exists") when the
/// reference exists and `force` is not set; `force` replaces it. Fails with `-1` when
/// the content does not parse (with the parser's error, such as `GIT_ERROR_TAG`
/// "failed to parse tag: …"), when its target is missing (`GIT_ERROR_ODB`) or of
/// another type (`GIT_ERROR_TAG`, "the type for the given target is invalid"), when
/// `refs/tags/<name>` is not a valid reference name or cannot be read
/// (`GIT_ERROR_REFERENCE`), or the object database cannot be loaded; and with
/// libgit2's error when the reference cannot be written. A zero target is refused, as a
/// missing one, with libgit2's `GIT_ERROR_ODB` error "odb: cannot read object: null OID
/// cannot exist", before libgit2 frees an uninitialised pointer for it (see the
/// [module docs](self)).
pub fn create_from_buffer(
    repo: RepositoryRef<'_>,
    buffer: &CStr,
    force: bool,
) -> Result<Oid, RepositoryError> {
    let failed = || RepositoryError::from_libgit2(ErrorCode::Generic.raw());
    // libgit2's order: the object database first, then the parse, then the target,
    // then the reference.
    let odb = repo.odb().map_err(|_| failed())?;
    let content = buffer.to_bytes();
    if content_is_valid(content, ObjectType::Tag, repo.oid_type()) != Ok(true) {
        return Err(failed());
    }
    let header = TagHeader::of(content);
    if header.target.iter().all(|&b| b == b'0') {
        set(
            ErrorClass::Odb,
            c"odb: cannot read object: null OID cannot exist",
        );
        return Err(failed());
    }
    if !normalizes(&header.refname()) {
        // libgit2 would leak the reference name on this failure: answer as it does,
        // after the target checks that precede it.
        header.check_target(repo, &odb)?;
        normalizes(&header.refname());
        return Err(failed());
    }
    drop(odb);
    let mut id = Oid::zeroed();
    // SAFETY: `id` is a writable id; `repo` a live repository on this thread, whose
    // object and reference databases libgit2 loads and writes through this shared
    // handle, as `RepositoryRef` allows; `buffer` a NUL-terminated string read for the
    // call. It parses (checked above, so the parse frees what it allocates) and names a
    // non-zero target, so `git_odb_read` writes the pointer libgit2 later frees.
    let rc = unsafe {
        ffi::git_tag_create_from_buffer(
            id.as_mut().as_mut_ptr(),
            repo.as_ptr().cast_mut(),
            buffer.as_ptr(),
            c_int::from(force),
        )
    };
    if rc < 0 {
        return Err(RepositoryError::from_libgit2(rc));
    }
    Ok(id)
}

/// The first three lines of tag content that parses as a tag: `object <target>`,
/// `type <kind>` and `tag <name>`, each without its keyword and newline.
struct TagHeader<'c> {
    target: &'c [u8],
    kind: &'c [u8],
    name: &'c [u8],
}

impl<'c> TagHeader<'c> {
    /// The header of `content`, which parses as a tag: the parser requires the three
    /// lines, in that order, each ended by a newline.
    fn of(content: &'c [u8]) -> Self {
        let mut lines = content.splitn(4, |&b| b == b'\n');
        let mut field = |keyword: &[u8]| {
            let line = lines.next().expect("a parsed tag has its header lines");
            line.strip_prefix(keyword)
                .expect("a parsed tag's header lines start with their keywords")
        };
        Self {
            target: field(b"object "),
            kind: field(b"type "),
            name: field(b"tag "),
        }
    }

    /// The reference libgit2 reads and writes for the tag: `refs/tags/` joined with
    /// the name, its leading `/`s dropped as `git_str_joinpath` drops them.
    fn refname(&self) -> CString {
        let start = self.name.iter().position(|&b| b != b'/');
        let name = &self.name[start.unwrap_or(self.name.len())..];
        CString::new([&b"refs/tags/"[..], name].concat()).expect("tag content holds no NUL")
    }

    /// `Ok` when the target exists in `odb` with the type the header records, as
    /// `git_tag_create_from_buffer` checks it (`git_odb_read`); otherwise its failure.
    fn check_target(
        &self,
        repo: RepositoryRef<'_>,
        odb: &OdbArc<'_>,
    ) -> Result<(), RepositoryError> {
        let failed = || RepositoryError::from_libgit2(ErrorCode::Generic.raw());
        let id = Oid::from_prefix(self.target, repo.oid_type()).map_err(|_| failed())?;
        let mut object = ptr::null_mut();
        // SAFETY: `object` is a writable slot, written on success (the zero id, for which
        // it is not, was refused before); `odb` is the repository's live object database,
        // which the counted reference keeps alive; `id` a live id of its type.
        let rc =
            unsafe { ffi::git_odb_read(&raw mut object, odb.as_c_ptr(), id.as_ref().as_ptr()) };
        if rc < 0 {
            return Err(failed());
        }
        let kind = {
            // SAFETY: `object` is the counted object `git_odb_read` handed over, read, then
            // released once.
            let kind = unsafe { ffi::git_odb_object_type(object) };
            // SAFETY: as above.
            unsafe { ffi::git_odb_object_free(object) };
            kind
        };
        let recorded = match self.kind {
            b"commit" => ObjectType::Commit,
            b"tree" => ObjectType::Tree,
            b"blob" => ObjectType::Blob,
            _ => ObjectType::Tag,
        };
        if kind != recorded.raw() {
            set(ErrorClass::Tag, c"the type for the given target is invalid");
            return Err(failed());
        }
        Ok(())
    }
}

/// Whether libgit2 can look `refname` up, which it first normalises as
/// `git_reference_lookup_resolved` does (one-level names allowed, at most
/// `GIT_REFNAME_MAX` bytes); when it cannot, the normalisation's error is set.
fn normalizes(refname: &CStr) -> bool {
    // `GIT_REFNAME_MAX`, `src/libgit2/refs.h`.
    let mut normalized = [0; 1024];
    // SAFETY: `normalized` is a writable buffer of the size passed; `refname` a
    // NUL-terminated string read for the call.
    let rc = unsafe {
        ffi::git_reference_normalize_name(
            normalized.as_mut_ptr(),
            normalized.len(),
            refname.as_ptr(),
            ffi::GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL,
        )
    };
    rc >= 0
}

/// Creates the lightweight tag `name`: points `refs/tags/<name>` at `target`, an
/// object of `repo`, of any type, and returns `target`'s id.
///
/// Wraps: `git_tag_create_lightweight`
///
/// # Errors
///
/// Refuses, with `-1`, a target looked up in another repository (`GIT_ERROR_INVALID`,
/// "the given target does not belong to this repository"), and a name starting with `-`
/// or equal to `HEAD` (`GIT_ERROR_TAG`, "'…' is not a valid tag name"). Fails with
/// [`ErrorCode::Exists`] (`GIT_ERROR_TAG`, "tag already exists") when the reference
/// exists and `force` is not set; `force` replaces it. Fails with libgit2's error when
/// `refs/tags/<name>` is not a valid reference name, or the reference cannot be read
/// or written.
pub fn create_lightweight(
    repo: RepositoryRef<'_>,
    name: &CStr,
    target: ObjectRef<'_, '_>,
    force: bool,
) -> Result<Oid, RepositoryError> {
    let mut id = Oid::zeroed();
    // SAFETY: `id` is a writable id; `repo` a live repository on this thread, whose
    // reference database libgit2 loads and writes through this shared handle; `name` a
    // NUL-terminated string read for the call; `target` a live object, borrowed for the
    // call, whose repository pointer libgit2 compares with `repo` before reading
    // anything else of it (its id), and which it never retains.
    let rc = unsafe {
        ffi::git_tag_create_lightweight(
            id.as_mut().as_mut_ptr(),
            repo.as_ptr().cast_mut(),
            name.as_ptr(),
            target.as_ptr(),
            c_int::from(force),
        )
    };
    if rc < 0 {
        return Err(RepositoryError::from_libgit2(rc));
    }
    Ok(id)
}

/// Creates the annotated tag `name`: writes a tag object recording `target`, `name`,
/// `tagger` and `message` into `repo`'s object database, and points `refs/tags/<name>`
/// at it. Returns the tag object's id.
///
/// The object is written as `object <id>`, `type <type>`, `tag <name>` and `tagger
/// <name> <<email>> <seconds> <zone>` lines, a blank line, then `message` as it is:
/// libgit2 checks none of them, so a name, tagger or message holding a newline in the
/// wrong place makes an object git cannot parse back (see
/// [`SignatureView`](crate::libgit2::src::libgit2::signature::SignatureView)).
///
/// A tagger without its name or its email (which only a signature libgit2 parsed out
/// of memory can be) is refused before libgit2 runs, with `-1` and a
/// `GIT_ERROR_INVALID` error: libgit2 would print the missing string through `%s`.
///
/// Wraps: `git_tag_create`
///
/// # Errors
///
/// Refuses, with `-1`, a target looked up in another repository
/// (`GIT_ERROR_INVALID`, "the given target does not belong to this repository"), and a
/// name starting with `-` or equal to `HEAD` (`GIT_ERROR_TAG`, "'…' is not a valid tag
/// name"). Fails with [`ErrorCode::Exists`] (`GIT_ERROR_TAG`, "tag already exists")
/// when the reference exists and `force` is not set; `force` replaces it. The tag
/// object is written before the reference, and stays when the reference cannot be
/// written. Fails with `-1` and
/// `GIT_ERROR_OBJECT` ("failed to create tag annotation") when the object cannot be
/// written, and with libgit2's error when `refs/tags/<name>` is not a valid reference
/// name, or the reference cannot be read or written.
pub fn create(
    repo: RepositoryRef<'_>,
    name: &CStr,
    target: ObjectRef<'_, '_>,
    tagger: SignatureRef<'_>,
    message: &CStr,
    force: bool,
) -> Result<Oid, RepositoryError> {
    printable(tagger)?;
    let mut id = Oid::zeroed();
    // SAFETY: a writable id; a live repository on this thread, whose databases libgit2
    // loads and writes through this shared handle; NUL-terminated strings read for the
    // call; a live object, compared with `repo` by owner before its id and type are
    // read, never retained; a tagger with both strings (checked above), only read.
    let rc = unsafe {
        ffi::git_tag_create(
            id.as_mut().as_mut_ptr(),
            repo.as_ptr().cast_mut(),
            name.as_ptr(),
            target.as_ptr(),
            tagger.as_ptr(),
            message.as_ptr(),
            c_int::from(force),
        )
    };
    if rc < 0 {
        return Err(RepositoryError::from_libgit2(rc));
    }
    Ok(id)
}

/// Writes the tag object [`create`] would write for `name`, `target`, `tagger` and
/// `message` into `repo`'s object database, without creating, checking or reading any
/// reference, and returns its id.
///
/// Unlike [`create`], nothing is checked but the tagger: any name is written, and a target of
/// another repository is recorded by its id and type, which `repo` may lack (or, for another object
/// id type, which its id type cannot name).
///
/// Wraps: `git_tag_annotation_create`
///
/// # Errors
///
/// Fails with `-1` and `GIT_ERROR_OBJECT` ("failed to create tag annotation") when the object
/// cannot be written or the object database loaded, and refuses a tagger without its name or its
/// email as [`create`] does.
pub fn create_annotation(
    repo: RepositoryRef<'_>,
    name: &CStr,
    target: ObjectRef<'_, '_>,
    tagger: SignatureRef<'_>,
    message: &CStr,
) -> Result<Oid, RepositoryError> {
    printable(tagger)?;
    let mut id = Oid::zeroed();
    // SAFETY: `id` is a writable id; `repo` a live repository on this thread, whose
    // object database libgit2 loads and writes through this shared handle; `name` and
    // `message` NUL-terminated strings read for the call; `target` a live object of a
    // live repository, whose id and type alone libgit2 reads, never retaining it;
    // `tagger` a live signature whose name and email are NUL-terminated (checked
    // above), only read.
    let rc = unsafe {
        ffi::git_tag_annotation_create(
            id.as_mut().as_mut_ptr(),
            repo.as_ptr().cast_mut(),
            name.as_ptr(),
            target.as_ptr(),
            tagger.as_ptr(),
            message.as_ptr(),
        )
    };
    if rc < 0 {
        return Err(RepositoryError::from_libgit2(rc));
    }
    Ok(id)
}

/// `Ok` when libgit2 can write `tagger` (`git_signature__writebuf` prints its name and
/// email with `%s`); otherwise a `GIT_ERROR_INVALID` failure.
fn printable(tagger: SignatureRef<'_>) -> Result<(), RepositoryError> {
    if tagger.name().is_some() && tagger.email().is_some() {
        return Ok(());
    }
    set(
        ErrorClass::Invalid,
        c"failed to create tag annotation - the tagger lacks its name or email",
    );
    Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()))
}

/// Deletes the tag `name`: removes the reference `refs/tags/<name>`, and its reflog.
/// The object it points at stays.
///
/// Wraps: `git_tag_delete`
///
/// # Errors
///
/// Fails with [`ErrorCode::NotFound`] when there is no such reference,
/// [`ErrorCode::InvalidSpec`] when `refs/tags/<name>` is not a valid reference name,
/// and with libgit2's error when the reference cannot be removed.
pub fn delete(repo: RepositoryRef<'_>, name: &CStr) -> Result<(), RepositoryError> {
    // SAFETY: `repo` is a live repository on this thread, whose reference database
    // libgit2 loads and writes through this shared handle; `name` a NUL-terminated
    // string read for the call.
    let rc = unsafe { ffi::git_tag_delete(repo.as_ptr().cast_mut(), name.as_ptr()) };
    if rc < 0 {
        return Err(RepositoryError::from_libgit2(rc));
    }
    Ok(())
}

/// The names of `repo`'s tags (`v1.0` for `refs/tags/v1.0`), in the order the
/// reference database lists them (by name, for the files backend).
///
/// Wraps: `git_tag_list`
///
/// # Errors
///
/// Fails when a tag's reference cannot be resolved to an id (a symbolic reference
/// whose target is missing), or the references cannot be listed: with code `-1` and
/// the thread's last error, which libgit2 sets but does not return (see the
/// [module docs](self)).
pub fn list(repo: RepositoryRef<'_>) -> Result<StrArrayVal, RepositoryError> {
    listed(|names| {
        // SAFETY: `names` is a live, empty array the routine fills with an array and
        // strings of libgit2's allocator, which it then owns; `repo` a live repository
        // on this thread, whose reference database libgit2 loads and reads.
        unsafe { ffi::git_tag_list(names, repo.as_ptr().cast_mut()) }
    })
}

/// The names of `repo`'s tags that match `pattern`, a glob as git's `wildmatch`
/// reads one (`*`, `?`, `[…]`, and `\` to escape), matched against the name below
/// `refs/tags/`, in the order [`list`] gives; every tag for an empty pattern.
///
/// Wraps: `git_tag_list_match`
///
/// # Errors
///
/// Fails as [`list`] does.
pub fn list_match(repo: RepositoryRef<'_>, pattern: &CStr) -> Result<StrArrayVal, RepositoryError> {
    listed(|names| {
        // SAFETY: as in `list`; `pattern` is a NUL-terminated string read for the call.
        unsafe { ffi::git_tag_list_match(names, pattern.as_ptr(), repo.as_ptr().cast_mut()) }
    })
}

/// The array `fill` (`git_tag_list` or `git_tag_list_match`) fills, or the failure it
/// returns or, returning 0 with a NULL array, only sets as the thread's last error.
fn listed(
    fill: impl FnOnce(*mut ffi::git_strarray) -> c_int,
) -> Result<StrArrayVal, RepositoryError> {
    let mut names = StrArray::new();
    let rc = fill(names.as_mut().as_mut_ptr());
    if rc < 0 {
        return Err(RepositoryError::from_libgit2(rc));
    }
    if !names.as_ref().has_buffer() {
        // libgit2 allocates the list before collecting into it, and hands it over
        // even when empty: a NULL array is the list it dropped on failure.
        return Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()));
    }
    Ok(names)
}

/// Calls `callback` with the full reference name (`refs/tags/v1.0`) of each of `repo`'s
/// tags and the id it resolves to: a tag object's for an annotated tag, the target's
/// for a lightweight one, in the order the reference database lists its references.
///
/// Returns [`ControlFlow::Continue`] once every tag was visited, and
/// [`ControlFlow::Break`] with the code the closure stopped the walk with; libgit2
/// then leaves a `GIT_ERROR_CALLBACK` error ("`git_tag_foreach` callback returned …") on
/// the thread, unless one is set already (by a libgit2 call the closure made, say). A
/// panic in the closure stops the walk and resumes here.
///
/// A break with `GIT_ITEROVER` (-31) is reported as that break, although libgit2
/// returns 0 for it: its reference walk takes the callback's code for the end of the
/// iteration, so C cannot tell that walk from a finished one.
///
/// The closure may use the repository, as libgit2 reads each reference afresh: a tag
/// it deletes before the walk reaches it fails the walk as not found.
///
/// Wraps: `git_tag_foreach`
///
/// # Errors
///
/// Fails with libgit2's error when the references cannot be listed, or a tag's
/// reference cannot be resolved to an id (a symbolic reference under `refs/tags/`
/// whose target is missing: [`ErrorCode::NotFound`]); the tags before it were visited.
pub fn foreach(
    repo: RepositoryRef<'_>,
    callback: &mut ForeachCallback<'_>,
) -> Result<ControlFlow<NonZero<c_int>>, RepositoryError> {
    let rc = callback.walk_with(|cb, payload| {
        // SAFETY: `repo` is a live repository on this thread, whose reference database
        // libgit2 loads and reads through this shared handle; `cb` is the callback's
        // trampoline and `payload` the callback, which `walk_with` lends for the walk
        // alone, and libgit2 calls it on this thread only, before returning.
        unsafe { ffi::git_tag_foreach(repo.as_ptr().cast_mut(), cb, payload) }
    });
    if let Some(code) = callback.stopped {
        // libgit2 stops at the first non-zero answer, and returns it, but for
        // `GIT_ITEROVER`, which `git_reference_foreach_name` turns into 0.
        debug_assert!(rc == code.get() || (rc == 0 && code.get() == ffi::GIT_ITEROVER));
        return Ok(ControlFlow::Break(code));
    }
    if rc != 0 {
        return Err(RepositoryError::from_libgit2(rc));
    }
    Ok(ControlFlow::Continue(()))
}

/// The closure a [`ForeachCallback`] calls: one tag's reference name and id, lent for
/// the call.
type ForeachFn<'f> = dyn FnMut(&CStr, OidRef<'_>) -> ControlFlow<NonZero<c_int>> + 'f;

/// A callback over a repository's tags (`git_tag_foreach_cb`): `git_tag_foreach` calls
/// it on the calling thread with the full reference name (`refs/tags/v1.0`) of each
/// reference under `refs/tags/` and the id it resolves to (a tag object's for an
/// annotated tag, the target's for a lightweight one), and stops at the first call that
/// does not return 0, returning that value.
///
/// Built over a closure with [`new`](Self::new). Its `extern "C"` trampoline lends the
/// closure each name as a `&CStr` and each id as an [`OidRef`], both valid for the call
/// only (libgit2 resolves the id into a local, and the name belongs to the reference
/// iterator), and returns [`ControlFlow::Continue`] as 0 and
/// [`ControlFlow::Break`]`(code)` as `code`, which a non-zero type guarantees stops the
/// walk. A panic in the closure does not cross into C: the trampoline stops the walk
/// with `GIT_EUSER` and the panic resumes once control is back in Rust, from
/// [`call`](Self::call) or from the wrapper that ran the walk.
///
/// ```
/// use std::ops::ControlFlow;
///
/// use libgit2::libgit2::src::libgit2::oid::{Oid, OidRef, OidType};
/// use libgit2::libgit2::src::libgit2::tag::ForeachCallback;
///
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// let mut seen = Vec::new();
/// let mut collect = |name: &std::ffi::CStr, id: OidRef<'_>| {
///     seen.push((name.to_owned(), id.to_oid()));
///     ControlFlow::Continue(())
/// };
/// let mut callback = ForeachCallback::new(&mut collect);
/// let id = Oid::from_string(c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750", OidType::Sha1).unwrap();
/// assert_eq!(
///     callback.call(c"refs/tags/v1", id.as_ref()),
///     ControlFlow::Continue(())
/// );
/// drop(callback);
/// assert_eq!(seen, [(c"refs/tags/v1".to_owned(), id)]);
/// ```
///
/// Neither the name nor the id can outlive the call:
///
/// ```compile_fail,E0521
/// use std::ffi::CStr;
/// use std::ops::ControlFlow;
/// use libgit2::libgit2::src::libgit2::tag::ForeachCallback;
/// use libgit2::libgit2::src::libgit2::oid::OidRef;
///
/// fn keep<'k>(kept: &mut Option<OidRef<'k>>) {
///     let mut f = |_: &CStr, id: OidRef<'_>| {
///         *kept = Some(id);
///         ControlFlow::Continue(())
///     };
///     let _ = ForeachCallback::new(&mut f);
/// }
/// ```
///
/// ```compile_fail,E0521
/// use std::ffi::CStr;
/// use std::ops::ControlFlow;
/// use libgit2::libgit2::src::libgit2::tag::ForeachCallback;
/// use libgit2::libgit2::src::libgit2::oid::OidRef;
///
/// fn keep<'k>(kept: &mut Option<&'k CStr>) {
///     let mut f = |name: &CStr, _: OidRef<'_>| {
///         *kept = Some(name);
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
/// use std::ffi::CStr;
/// use std::ops::ControlFlow;
/// use libgit2::libgit2::src::libgit2::tag::ForeachCallback;
/// use libgit2::libgit2::src::libgit2::oid::OidRef;
///
/// fn send<T: Send>(_: T) {}
/// let mut f = |_: &CStr, _: OidRef<'_>| ControlFlow::Continue(());
/// send(ForeachCallback::new(&mut f));
/// ```
///
/// Wraps: `git_tag_foreach_cb`
/// Handwritten
pub struct ForeachCallback<'f> {
    f: &'f mut ForeachFn<'f>,
    /// The code the closure stopped the last walk with: what it broke with, or
    /// `GIT_EUSER` for a panic. Tells the closure's codes from libgit2's own errors.
    stopped: Option<NonZero<c_int>>,
    /// A panic the closure raised inside a C walk, resumed once the walk returns.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> ForeachCallback<'f> {
    /// The callback that calls `f` with each tag's reference name and id, until it
    /// breaks.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(&CStr, OidRef<'_>) -> ControlFlow<NonZero<c_int>> + 'f,
    {
        Self {
            f,
            stopped: None,
            panic: None,
        }
    }

    /// Calls the callback with `name` and `id`, through the trampoline libgit2 calls,
    /// and returns what libgit2 would read from it. A panic of the closure resumes here.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: the trampoline is never NULL"
    )]
    pub fn call(&mut self, name: &CStr, id: OidRef<'_>) -> ControlFlow<NonZero<c_int>> {
        let rc = self.walk_with(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            // SAFETY: `payload` is this callback, which nothing else uses during the
            // walk; `name` a NUL-terminated string and `id` a live id, both lent for the
            // call, which only reads them: the trampoline's contract.
            unsafe { callback(name.as_ptr(), id.as_ptr().cast_mut(), payload) }
        });
        match NonZero::new(rc) {
            None => ControlFlow::Continue(()),
            Some(code) => ControlFlow::Break(code),
        }
    }

    /// Runs `walk` with the C callback and its payload, as the wrapper of a routine that
    /// calls the callback (`git_tag_foreach`) hands them to it, then resumes a panic the
    /// closure raised meanwhile. The payload is this callback, which nothing else
    /// touches until `walk` returns.
    pub(crate) fn walk_with<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_tag_foreach_cb, *mut c_void) -> R,
    ) -> R {
        self.stopped = None;
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
            .field("stopped", &self.stopped)
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_tag_foreach_cb` over a [`ForeachCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`ForeachCallback`] that nothing else uses during the
/// call; `name` be NULL or a NUL-terminated string, and `id` NULL or a `git_oid`, both
/// live and unchanged for the call.
unsafe extern "C" fn foreach_with(
    name: *const c_char,
    id: *mut ffi::git_oid,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: per this function's contract, `payload` is a live callback borrowed for
    // the call alone.
    let callback = unsafe { &mut *payload.cast::<ForeachCallback<'_>>() };
    if callback.panic.is_some() {
        return ErrorCode::User.raw();
    }
    if name.is_null() {
        // libgit2 never passes NULL; stop the walk rather than invent a name.
        return ErrorCode::User.raw();
    }
    // SAFETY: per this function's contract, a non-NULL `name` is a NUL-terminated string
    // live and unchanged for the call; the borrow given to the closure is for its call
    // only.
    let name = unsafe { CStr::from_ptr(name) };
    // SAFETY: as above, for a non-NULL `id`; the closure only reads it.
    let Some(id) = (unsafe { OidRef::from_ptr(id) }) else {
        return ErrorCode::User.raw();
    };
    let code = match panic::catch_unwind(AssertUnwindSafe(|| (callback.f)(name, id))) {
        Ok(ControlFlow::Continue(())) => return 0,
        Ok(ControlFlow::Break(code)) => code,
        Err(panic) => {
            callback.panic = Some(panic);
            NonZero::new(ErrorCode::User.raw()).expect("GIT_EUSER is not 0")
        }
    };
    callback.stopped = Some(code);
    code.get()
}

#[cfg(test)]
mod unit_tests {
    use core::ptr::NonNull;
    use std::process::Command;
    use std::sync::atomic::Ordering::SeqCst;
    use std::sync::atomic::{AtomicBool, AtomicIsize};

    use super::*;
    use crate::libgit2::src::util::alloc::{Alloc, Allocator};

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

    static REFUSING: AtomicBool = AtomicBool::new(false);
    static ALLOWED: AtomicIsize = AtomicIsize::new(0);
    /// Whether only the request `ALLOWED` counts down to is refused, the later ones
    /// granted again.
    static ONE_SHOT: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// Whether the next request is refused: while `REFUSING` is set, after `ALLOWED`
    /// more requests; with `ONE_SHOT`, that request alone.
    fn refuse() -> bool {
        if !REFUSING.load(SeqCst) {
            return false;
        }
        let left = ALLOWED.fetch_sub(1, SeqCst);
        if ONE_SHOT.load(SeqCst) {
            left == 0
        } else {
            left <= 0
        }
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
            NonNull::new(unsafe { malloc(len) }.cast())
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

    /// A refused allocation of the reference name is libgit2's out-of-memory error, not
    /// an invalid name; with the allocation granted the same name is valid.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn out_of_memory_is_an_error() {
        isolated(
            concat!(module_path!(), "::out_of_memory_is_an_error"),
            || {
                install_countdown();
                // The thread's error state, allocated now, so a refusal reports out of
                // memory rather than a thread state it could not allocate.
                set(ErrorClass::Invalid, c"error state allocated");
                ALLOWED.store(0, SeqCst);
                REFUSING.store(true, SeqCst);
                let refused = name_is_valid(c"v1.0");
                REFUSING.store(false, SeqCst);
                let error = refused.unwrap_err();
                assert_eq!(
                    (error.class(), error.message()),
                    (Some(ErrorClass::NoMemory), c"Out of memory")
                );
                assert_eq!(name_is_valid(c"v1.0"), Ok(true));
            },
        );
    }

    /// The reference name grows once for `refs/tags/` and again for a name longer than
    /// the first allocation's slack: a refusal of that second allocation is the same
    /// out-of-memory error, and nothing of the first is leaked.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn out_of_memory_growing_the_reference_name_is_an_error() {
        isolated(
            concat!(
                module_path!(),
                "::out_of_memory_growing_the_reference_name_is_an_error"
            ),
            || {
                install_countdown();
                set(ErrorClass::Invalid, c"error state allocated");
                // `refs/tags/` takes a 16-byte block; this name no longer fits it.
                let name = c"release/2024/q1";
                ALLOWED.store(1, SeqCst);
                REFUSING.store(true, SeqCst);
                let refused = name_is_valid(name);
                REFUSING.store(false, SeqCst);
                let error = refused.unwrap_err();
                assert_eq!(
                    (error.class(), error.message()),
                    (Some(ErrorClass::NoMemory), c"Out of memory")
                );
                assert_eq!(name_is_valid(name), Ok(true));
            },
        );
    }

    /// A listing whose list cannot be allocated is libgit2's out-of-memory error, which
    /// it returns (no names were collected, so nothing leaks); with the allocation
    /// granted the same listing succeeds.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn out_of_memory_listing_is_an_error() {
        isolated(
            concat!(module_path!(), "::out_of_memory_listing_is_an_error"),
            || {
                install_countdown();
                let sandbox = Sandbox::new("oom-list");
                let repo = sandbox.open();
                set(ErrorClass::Invalid, c"error state allocated");
                ALLOWED.store(0, SeqCst);
                REFUSING.store(true, SeqCst);
                let refused = list_match(repo.as_ref(), c"t*");
                REFUSING.store(false, SeqCst);
                let e = refused.unwrap_err();
                assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::NoMemory)));
                assert_eq!(list(repo.as_ref()).unwrap().as_ref().len(), 7);
            },
        );
    }

    /// When libgit2 cannot be initialised, the check reports it without calling
    /// `git_tag_name_is_valid`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn uninitialisable_library_is_reported() {
        isolated(
            concat!(module_path!(), "::uninitialisable_library_is_reported"),
            || {
                install_countdown();
                ALLOWED.store(0, SeqCst);
                REFUSING.store(true, SeqCst);
                let error = name_is_valid(c"v1.0").unwrap_err();
                assert_eq!(
                    (error.class(), error.message()),
                    (
                        Some(ErrorClass::Invalid),
                        c"library has not been initialized"
                    )
                );
            },
        );
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

    /// A private copy of `testrepo.git`, removed on drop.
    struct Sandbox(std::path::PathBuf);

    impl Sandbox {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("crustify-tag-unit-{}-{tag}", std::process::id()));
            drop(std::fs::remove_dir_all(&path));
            let source = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/c/tests/resources/testrepo.git"
            );
            copy_dir(std::path::Path::new(source), &path);
            Self(path)
        }

        fn open(&self) -> crate::libgit2::src::libgit2::repository::RepositoryBox {
            use crate::libgit2::src::libgit2::repository::Repository;
            assert!(runtime_pinned());
            let path = CString::new(self.0.to_str().unwrap()).unwrap();
            Repository::open(&path).unwrap()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    const COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
    /// `refs/tags/test`'s tag object.
    const TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";

    /// A tag of `target` (a commit) named `name`, with a tagger and a message.
    fn tag_buffer(target: &str, name: &str) -> CString {
        CString::new(format!(
            "object {target}\ntype commit\ntag {name}\n\
             tagger A U Thor <author@example.com> 1 +0000\n\nmessage\n"
        ))
        .unwrap()
    }

    /// A zero target is refused with the error `git_odb_read` sets for it, before
    /// libgit2 frees the pointer it left uninitialised (`ASan`: a use after free once the
    /// slot is stale); nothing is written. Corrects `git_tag_create_from_buffer`. In a
    /// child: writing a reference reads the time through glibc's `tzset`, which `TSan`
    /// reports against any concurrent caller.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn zero_targets_are_refused_before_c() {
        isolated(
            concat!(module_path!(), "::zero_targets_are_refused_before_c"),
            || {
                let sandbox = Sandbox::new("zero");
                let repo = sandbox.open();
                let zero = "0".repeat(40);
                for (target, name) in [(zero.as_str(), "zero"), ("0000000000", "short")] {
                    let buffer = tag_buffer(target, name);
                    let e = create_from_buffer(repo.as_ref(), &buffer, true).unwrap_err();
                    assert_eq!(e.raw_code(), -1, "{target}");
                    if target.len() == 40 {
                        assert_eq!(e.class(), Some(ErrorClass::Odb));
                        assert_eq!(
                            e.message(),
                            c"odb: cannot read object: null OID cannot exist"
                        );
                    } else {
                        assert_eq!(e.class(), Some(ErrorClass::Tag));
                    }
                    assert!(!sandbox.0.join("refs/tags").join(name).exists());
                }
                let id =
                    create_from_buffer(repo.as_ref(), &tag_buffer(COMMIT, "fine"), false).unwrap();
                assert!(sandbox.0.join("refs/tags/fine").exists());
                assert_eq!(
                    list_match(repo.as_ref(), c"fine").unwrap().as_ref().len(),
                    1
                );
                assert!(!id.as_ref().is_zero());
            },
        );
    }

    /// Content that libgit2 parses part of before failing (a malformed tagger, a header
    /// without a message) is refused by the leak-free parse first (`LSan`: the C parse
    /// would leak the name and tagger), with the parser's error and code `-1`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn malformed_buffers_are_refused_without_leaking() {
        let sandbox = Sandbox::new("malformed");
        let repo = sandbox.open();
        for (buffer, message) in [
            (
                format!("object {COMMIT}\ntype commit\ntag v9\ntagger bad\n\nm\n"),
                c"failed to parse signature - malformed e-mail",
            ),
            (
                format!("object {COMMIT}\ntype commit\ntag v9\ntagger A <a@b> 0 +0000\nx"),
                c"failed to parse tag: tag contains no message",
            ),
            (
                format!("object {COMMIT}\ntype commit\n"),
                c"failed to parse tag: object too short",
            ),
        ] {
            let buffer = CString::new(buffer).unwrap();
            let e = create_from_buffer(repo.as_ref(), &buffer, false).unwrap_err();
            assert_eq!(e.raw_code(), -1);
            assert_eq!(e.message(), message);
        }
        assert!(!sandbox.0.join("refs/tags/v9").exists());
    }

    /// A name that is no valid reference name is answered as libgit2 answers it, after
    /// its target checks, without libgit2 building (and leaking, `LSan`) the reference
    /// name; leading `/`s are dropped as libgit2 joins the name. In a child, as above.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn invalid_reference_names_are_answered_in_rust() {
        isolated(
            concat!(
                module_path!(),
                "::invalid_reference_names_are_answered_in_rust"
            ),
            || {
                let sandbox = Sandbox::new("names");
                let repo = sandbox.open();
                let buffer = |target: &str, kind: &str, name: &str| {
                    CString::new(format!(
                        "object {target}\ntype {kind}\ntag {name}\n\nmessage\n"
                    ))
                    .unwrap()
                };
                let cases = [
                    (buffer(COMMIT, "commit", "a..b"), ErrorClass::Reference),
                    (
                        buffer(COMMIT, "commit", &"n".repeat(1100)),
                        ErrorClass::Reference,
                    ),
                    (buffer(&"de".repeat(20), "commit", "a..b"), ErrorClass::Odb),
                    (buffer(COMMIT, "blob", "a..b"), ErrorClass::Tag),
                    (buffer(TAG, "tag", "a..b"), ErrorClass::Reference),
                    (buffer(TAG, "commit", "a..b"), ErrorClass::Tag),
                ];
                for (buffer, class) in cases {
                    let e = create_from_buffer(repo.as_ref(), &buffer, true).unwrap_err();
                    assert_eq!((e.raw_code(), e.class()), (-1, Some(class)), "{e:?}");
                }
                let e = create_from_buffer(repo.as_ref(), &buffer(COMMIT, "commit", "a..b"), false);
                assert_eq!(
                    e.unwrap_err().message(),
                    c"the given reference name 'refs/tags/a..b' is not valid"
                );
                create_from_buffer(repo.as_ref(), &buffer(COMMIT, "commit", "//lead"), false)
                    .unwrap();
                assert!(sandbox.0.join("refs/tags/lead").exists());
                let header = TagHeader::of(b"object x\ntype tag\ntag ///a/b\n");
                assert_eq!(header.refname().as_c_str(), c"refs/tags/a/b");
                assert_eq!((header.target, header.kind), (&b"x"[..], &b"tag"[..]));
                assert_eq!(
                    TagHeader::of(b"object \ntype \ntag /\n")
                        .refname()
                        .as_c_str(),
                    c"refs/tags/"
                );
            },
        );
    }

    /// Runs `body` alone in a child process, with `LeakSanitizer` off there: libgit2
    /// leaks the names it collected when a listing fails.
    fn isolated_tolerating_leaks(test: &str, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
        let separator = if inherited.is_empty() { "" } else { ":" };
        let out = Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .env(
                "ASAN_OPTIONS",
                format!("{inherited}{separator}detect_leaks=0"),
            )
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A tag whose symbolic reference dangles fails the listing: libgit2 returns 0 with
    /// a NULL array and only sets the error, which the wrappers report. Corrects
    /// `git_tag_list_match`'s silent empty list.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failed_listings_are_errors() {
        isolated_tolerating_leaks(
            concat!(module_path!(), "::failed_listings_are_errors"),
            || {
                let sandbox = Sandbox::new("dangling");
                let repo = sandbox.open();
                let all = list(repo.as_ref()).unwrap();
                assert_eq!(all.as_ref().len(), 7);
                std::fs::write(
                    sandbox.0.join("refs/tags/zz-dangling"),
                    "ref: refs/heads/nope\n",
                )
                .unwrap();
                for result in [list(repo.as_ref()), list_match(repo.as_ref(), c"*")] {
                    let e = result.unwrap_err();
                    assert_eq!(e.raw_code(), -1);
                    assert_eq!(e.class(), Some(ErrorClass::Reference));
                    assert_eq!(e.message(), c"reference 'refs/heads/nope' not found");
                }
            },
        );
    }

    /// `git_tag_foreach` through the trampoline: the closure sees each tag's full name
    /// and id; a break stops the walk with its code; a panic stops it with `GIT_EUSER`
    /// and resumes after.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn foreach_walks_through_the_trampoline() {
        use crate::libgit2::src::libgit2::repository::RepositoryRef;
        let sandbox = Sandbox::new("foreach");
        let repo = sandbox.open();
        let walk = |repo: RepositoryRef<'_>, callback: &mut ForeachCallback<'_>| {
            callback.walk_with(|cb, payload| {
                // SAFETY: a live repository on this thread; the callback and payload are
                // the handle's, live for the walk.
                unsafe { ffi::git_tag_foreach(repo.as_ptr().cast_mut(), cb, payload) }
            })
        };

        let mut seen = Vec::new();
        let mut collect = |name: &CStr, id: OidRef<'_>| {
            seen.push((
                name.to_str().unwrap().to_owned(),
                id.to_hex_string().unwrap(),
            ));
            ControlFlow::Continue(())
        };
        let mut callback = ForeachCallback::new(&mut collect);
        assert_eq!(walk(repo.as_ref(), &mut callback), 0);
        assert_eq!(
            format!("{callback:?}"),
            "ForeachCallback { stopped: None, panicked: false, .. }"
        );
        drop(callback);
        seen.sort();
        assert_eq!(seen.len(), 7);
        assert_eq!(
            seen[5],
            (
                "refs/tags/test".to_owned(),
                "b25fa35b38051e4ae45d4222e795f9df2e43f1d1".to_owned()
            )
        );

        let mut calls = 0;
        let mut stop = |_: &CStr, _: OidRef<'_>| {
            calls += 1;
            ControlFlow::Break(NonZero::new(42).unwrap())
        };
        let mut callback = ForeachCallback::new(&mut stop);
        assert_eq!(walk(repo.as_ref(), &mut callback), 42);
        drop(callback);
        assert_eq!(calls, 1);

        let mut boom =
            |_: &CStr, _: OidRef<'_>| -> ControlFlow<NonZero<c_int>> { panic!("walk panicked") };
        let mut callback = ForeachCallback::new(&mut boom);
        let caught = panic::catch_unwind(AssertUnwindSafe(|| walk(repo.as_ref(), &mut callback)));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"walk panicked")
        );
    }

    /// The trampoline stops the walk for the NULL arguments libgit2 never passes, and
    /// once a panic is recorded, without calling the closure.
    #[test]
    fn trampoline_guards_without_c() {
        let mut called = false;
        let mut f = |_: &CStr, _: OidRef<'_>| {
            called = true;
            ControlFlow::Continue(())
        };
        let mut callback = ForeachCallback::new(&mut f);
        let payload: *mut c_void = ptr::from_mut(&mut callback).cast();
        let mut id = ffi::git_oid::default();
        let user = ErrorCode::User.raw();
        // SAFETY: `payload` is the live callback, unused elsewhere; NULL is allowed for
        // either argument, and `id` is a live local.
        unsafe { assert_eq!(foreach_with(ptr::null(), &raw mut id, payload), user) };
        // SAFETY: as above.
        unsafe {
            assert_eq!(
                foreach_with(c"refs/tags/x".as_ptr(), ptr::null_mut(), payload),
                user
            );
        };
        callback.panic = Some(Box::new("recorded"));
        // The write above went through `callback`; borrow it afresh for the call.
        let payload: *mut c_void = ptr::from_mut(&mut callback).cast();
        // SAFETY: as above.
        let rc = unsafe { foreach_with(c"refs/tags/x".as_ptr(), &raw mut id, payload) };
        assert_eq!(rc, user);
        assert_eq!(
            format!("{callback:?}"),
            "ForeachCallback { stopped: None, panicked: true, .. }"
        );
        drop(callback);
        assert!(!called);
    }

    /// A tagger without its name or its email (what libgit2's parser leaves when it runs
    /// out of memory) never reaches `git_signature__writebuf`, which would print the
    /// NULL through `%s`: both writers refuse it before C, writing neither an object nor
    /// a reference.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn incomplete_taggers_never_reach_libgit2() {
        use crate::libgit2::src::libgit2::object::Object;
        use crate::libgit2::src::libgit2::oid::OidType;
        let sandbox = Sandbox::new("incomplete-tagger");
        let repo = sandbox.open();
        let id = Oid::from_string(&CString::new(COMMIT).unwrap(), OidType::Sha1).unwrap();
        let target = Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap();
        let objects = || {
            std::fs::read_dir(sandbox.0.join("objects"))
                .unwrap()
                .map(|dir| dir.unwrap().path())
                .filter(|dir| dir.is_dir())
                .map(|dir| std::fs::read_dir(dir).unwrap().count())
                .sum::<usize>()
        };
        let before = objects();
        let name = c"T".as_ptr().cast_mut();
        let email = c"t@example.com".as_ptr().cast_mut();
        for (name, email) in [
            (ptr::null_mut(), email),
            (name, ptr::null_mut()),
            (ptr::null_mut(), ptr::null_mut()),
        ] {
            let mut raw = ffi::git_signature {
                name,
                email,
                when: ffi::git_time {
                    time: 1,
                    offset: 0,
                    sign: b'+'.cast_signed(),
                },
            };
            // SAFETY: a live local signature whose strings are NULL or literals, only
            // read while the handle lives.
            let tagger = unsafe { SignatureRef::from_ptr(&raw mut raw) }.unwrap();
            for result in [
                create(repo.as_ref(), c"bad", target.as_ref(), tagger, c"m\n", true),
                create_annotation(repo.as_ref(), c"bad", target.as_ref(), tagger, c"m\n"),
            ] {
                let e = result.unwrap_err();
                assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
                assert_eq!(
                    e.message(),
                    c"failed to create tag annotation - the tagger lacks its name or email"
                );
            }
        }
        assert_eq!(objects(), before);
        assert!(!sandbox.0.join("refs/tags/bad").exists());
    }

    /// `foreach` tells the closure's breaks, whatever their sign, from libgit2's
    /// failures, through the code the trampoline records; a walk resets it, and a
    /// panic resumes with it set to `GIT_EUSER`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn foreach_tells_breaks_from_failures() {
        let sandbox = Sandbox::new("foreach-breaks");
        let repo = sandbox.open();
        // libgit2 returns 0 for a break with `GIT_ITEROVER`, which is still a break.
        for code in [
            -1,
            42,
            i32::MIN,
            ErrorCode::NotFound.raw(),
            ffi::GIT_ITEROVER,
        ] {
            let mut calls = 0;
            let mut stop = |_: &CStr, _: OidRef<'_>| {
                calls += 1;
                ControlFlow::Break(NonZero::new(code).unwrap())
            };
            let mut callback = ForeachCallback::new(&mut stop);
            let flow = foreach(repo.as_ref(), &mut callback).unwrap();
            assert_eq!(flow, ControlFlow::Break(NonZero::new(code).unwrap()));
            assert_eq!(callback.stopped, NonZero::new(code));
            drop(callback);
            assert_eq!(calls, 1);
        }

        let mut seen = 0;
        let mut count = |_: &CStr, _: OidRef<'_>| {
            seen += 1;
            ControlFlow::Continue(())
        };
        let mut callback = ForeachCallback::new(&mut count);
        callback.stopped = NonZero::new(5);
        assert_eq!(
            foreach(repo.as_ref(), &mut callback),
            Ok(ControlFlow::Continue(()))
        );
        assert_eq!(callback.stopped, None);
        drop(callback);
        assert_eq!(seen, 7);

        // A dangling symbolic reference fails the walk after the tags before it.
        std::fs::write(
            sandbox.0.join("refs/tags/zz-dangling"),
            "ref: refs/heads/nope\n",
        )
        .unwrap();
        let mut seen = 0;
        let mut count = |_: &CStr, _: OidRef<'_>| {
            seen += 1;
            ControlFlow::Continue(())
        };
        let mut callback = ForeachCallback::new(&mut count);
        let e = foreach(repo.as_ref(), &mut callback).unwrap_err();
        drop(callback);
        assert_eq!(e.code(), Some(ErrorCode::NotFound));
        assert_eq!(e.message(), c"reference 'refs/heads/nope' not found");
        assert_eq!(seen, 7);

        let mut boom =
            |_: &CStr, _: OidRef<'_>| -> ControlFlow<NonZero<c_int>> { panic!("foreach panicked") };
        let mut callback = ForeachCallback::new(&mut boom);
        let caught =
            panic::catch_unwind(AssertUnwindSafe(|| foreach(repo.as_ref(), &mut callback)));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"foreach panicked")
        );
        assert_eq!(callback.stopped, NonZero::new(ErrorCode::User.raw()));
        assert!(callback.panic.is_none());
    }

    /// libgit2's tag parser keeps a tag whose tagger's name or email it ran out of memory
    /// copying, and reports success: `tagger` lends that signature with the string
    /// missing, and both writers refuse it as their tagger before libgit2 would print
    /// the NULL. Each lookup refuses one allocation, at each position in turn; libgit2
    /// leaks on some of those failures (in its object database and cache), so the child
    /// runs with `LeakSanitizer` off.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn taggers_lost_to_out_of_memory_are_refused() {
        isolated_tolerating_leaks(
            concat!(
                module_path!(),
                "::taggers_lost_to_out_of_memory_are_refused"
            ),
            || {
                use crate::libgit2::src::libgit2::object::Object;
                use crate::libgit2::src::libgit2::oid::OidType;
                install_countdown();
                set(ErrorClass::Invalid, c"error state allocated");
                let sandbox = Sandbox::new("oom-tagger");
                let tag_id = Oid::from_string(&CString::new(TAG).unwrap(), OidType::Sha1).unwrap();
                let commit =
                    Oid::from_string(&CString::new(COMMIT).unwrap(), OidType::Sha1).unwrap();
                let (mut lost_name, mut lost_email) = (false, false);
                ONE_SHOT.store(true, SeqCst);
                for k in 0..10_000 {
                    let repo = sandbox.open();
                    ALLOWED.store(k, SeqCst);
                    REFUSING.store(true, SeqCst);
                    let tag = Tag::lookup(repo.as_ref(), tag_id.as_ref());
                    REFUSING.store(false, SeqCst);
                    let Ok(tag) = tag else { continue };
                    let tagger = tag.as_ref().tagger().expect("the tag has a tagger");
                    let (name, email) = (tagger.name(), tagger.email());
                    if name.is_some() && email.is_some() {
                        if ALLOWED.load(SeqCst) >= 0 {
                            // Every allocation of the lookup was granted: the sweep is over.
                            break;
                        }
                        continue;
                    }
                    lost_name |= name.is_none();
                    lost_email |= email.is_none();
                    let target = Object::lookup(repo.as_ref(), commit.as_ref(), None).unwrap();
                    for result in [
                        create(
                            repo.as_ref(),
                            c"lost",
                            target.as_ref(),
                            tagger,
                            c"m\n",
                            true,
                        ),
                        create_annotation(repo.as_ref(), c"lost", target.as_ref(), tagger, c"m\n"),
                    ] {
                        let e = result.unwrap_err();
                        assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
                    }
                    assert!(tagger.to_signature().is_err());
                }
                ONE_SHOT.store(false, SeqCst);
                assert!(lost_name && lost_email);
                assert!(!sandbox.0.join("refs/tags/lost").exists());
            },
        );
    }

    /// `git_tag`'s owner, handles, lifecycle policy and conversions.
    mod tag_object {
        use core::ffi::CStr;
        use core::mem::size_of;
        use core::ptr::{self, NonNull};

        use ffibox::{CDrop, CRefClone};

        use crate::ffi;
        use crate::libgit2::src::libgit2::object::{Object, ObjectArc, ObjectRef, ObjectType};
        use crate::libgit2::src::libgit2::object_api::*;
        use crate::libgit2::src::libgit2::oid::{Oid, OidRef, OidType};
        use crate::libgit2::src::libgit2::repository::{Repository, RepositoryBox};
        use crate::libgit2::src::libgit2::tag::*;
        use crate::libgit2::src::util::errors::{ErrorClass, ErrorCode, runtime_pinned};

        /// `testrepo.git`: tag `b25fa35` (`test`) of tag `7b43849`, of commit `e90810b`.
        const TESTREPO: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/c/tests/resources/testrepo.git\0"
        );
        const TAG: &CStr = c"b25fa35b38051e4ae45d4222e795f9df2e43f1d1";
        const INNER: &CStr = c"7b4384978d2493e851f9cca7858815fac9b10980";
        const COMMIT: &CStr = c"e90810b8df3e80c413d903f631643c716887138d";

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

        fn tag_view(hex: &CStr) -> (String, ffi::git_object_t) {
            (hex.to_str().unwrap().to_owned(), ffi::GIT_OBJECT_TAG)
        }

        #[test]
        fn layout_matches_c() {
            assert_eq!(size_of::<Tag<'_>>(), size_of::<ffi::git_tag>());
            assert_eq!(size_of::<TagArc<'_>>(), size_of::<*mut ffi::git_tag>());
            assert_eq!(
                size_of::<Option<TagArc<'_>>>(),
                size_of::<*mut ffi::git_tag>()
            );
            assert_eq!(
                size_of::<TagRef<'_, '_>>(),
                size_of::<*const ffi::git_tag>()
            );
            assert_eq!(
                size_of::<TagMut<'_, '_>>(),
                size_of::<*const ffi::git_tag>()
            );
            assert_eq!(size_of::<TagFree>(), 0);
        }

        /// The handles' seams, and the view as an object, over a pointer nothing reads,
        /// so Miri runs them: `git_tag` is opaque, of size zero, so a dangling aligned
        /// pointer addresses all of it.
        #[test]
        fn handles_without_c() {
            // SAFETY: NULL is refused without being read.
            assert!(unsafe { TagRef::from_ptr(ptr::null()) }.is_none());
            // SAFETY: as above.
            assert!(unsafe { TagMut::from_ptr(ptr::null_mut()) }.is_none());
            let p = NonNull::<ffi::git_tag>::dangling().as_ptr();
            // SAFETY: a zero-sized object at an aligned pointer, never read or written,
            // and reached by this handle alone.
            let mut m = unsafe { TagMut::from_ptr(p) }.unwrap();
            assert_eq!(m.as_mut_ptr(), p);
            assert_eq!(m.as_mut().as_mut_ptr(), p);
            let r = m.as_ref();
            assert_eq!(r.as_ptr(), p.cast_const());
            assert_eq!(r.as_object().as_ptr(), p.cast_const().cast());
            assert_eq!(format!("{r:?}"), "Tag { .. }");
            assert_eq!(format!("{m:?}"), "Tag { .. }");
        }

        /// The policy driven by hand: the up-ref counts one more reference on the same
        /// pointer, which an adopted owner and the free each settle; `to_tag` takes one
        /// through a borrowed handle, which outlives the owner it came from.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn policy_counts_references_on_one_tag() {
            let repo = open();
            let tag = Tag::lookup(repo.as_ref(), sha1(TAG).as_ref()).unwrap();
            let p = NonNull::new(tag.as_ptr()).unwrap();
            // SAFETY: `tag` holds a reference to the live tag at `p`.
            assert!(unsafe { TagFree.c_up_ref(p) });
            // SAFETY: the reference just counted, given to the owner.
            let adopted = unsafe { TagArc::from_raw(p.as_ptr()) }.unwrap();
            assert!(TagArc::ptr_eq(&adopted, &tag));
            let raw = adopted.into_c();
            // SAFETY: `raw` is the reference `into_c` gave up, released once here.
            unsafe { TagFree.c_drop(NonNull::new(raw).unwrap().cast()) };

            // SAFETY: `tag` keeps the tag alive while the handle lives.
            let borrowed = unsafe { TagRef::from_ptr(tag.as_c_ptr()) }.unwrap();
            let copy = borrowed.to_tag();
            let clone = copy.clone();
            assert!(TagArc::ptr_eq(&copy, &tag) && TagArc::ptr_eq(&clone, &tag));
            drop(tag);
            drop(copy);
            assert_eq!(view(clone.as_ref().as_object()), tag_view(TAG));
        }

        /// A tag lookup is the object lookup of type tag: the same cached object, and
        /// another type is not found; an id of another type than the repository's is
        /// refused before C, at every length.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn lookups_find_tags_only() {
            let repo = open();
            let r = repo.as_ref();
            let tag = Tag::lookup(r, sha1(TAG).as_ref()).unwrap();
            let short = Tag::lookup_prefix(r, sha1(TAG).as_ref(), 7).unwrap();
            let object = Object::lookup(r, sha1(TAG).as_ref(), Some(ObjectType::Tag)).unwrap();
            assert!(TagArc::ptr_eq(&tag, &short));
            assert_eq!(tag.as_c_ptr().cast(), object.as_c_ptr());
            for e in [
                Tag::lookup(r, sha1(COMMIT).as_ref()).unwrap_err(),
                Tag::lookup_prefix(r, sha1(COMMIT).as_ref(), 10).unwrap_err(),
            ] {
                assert_eq!(e.code(), Some(ErrorCode::NotFound));
                assert_eq!(e.class(), Some(ErrorClass::Invalid));
            }
            let e = Tag::lookup_prefix(r, sha1(TAG).as_ref(), 3).unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::Ambiguous));

            let mut wide = Oid::from_raw(&sha1(TAG).as_ref().id(), OidType::Sha256).unwrap();
            for kind in [u8::try_from(OidType::Sha256.raw()).unwrap(), 0, 9, u8::MAX] {
                wide.as_mut().set_raw_kind(kind);
                let mut refused = vec![Tag::lookup(r, wide.as_ref()).unwrap_err()];
                for len in [0, 4, 7, 40, 41, 64, usize::MAX] {
                    refused.push(Tag::lookup_prefix(r, wide.as_ref(), len).unwrap_err());
                }
                for e in refused {
                    assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
                    assert_eq!(
                        e.message(),
                        c"specified object ID type does not match repository object ID type"
                    );
                }
            }
        }

        /// Owners and handles convert between tag and object, keeping one count per
        /// owner; an object of another type stays an object, untouched.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn conversions_check_the_type_and_keep_the_count() {
            let repo = open();
            let r = repo.as_ref();
            let lookup = |hex: &CStr| Object::lookup(r, sha1(hex).as_ref(), None).unwrap();

            let commit = lookup(COMMIT);
            assert!(TagRef::from_object(commit.as_ref()).is_none());
            let commit = Tag::from_object(commit).unwrap_err();
            assert_eq!(view(commit.as_ref()).1, ffi::GIT_OBJECT_COMMIT);

            let object = lookup(INNER);
            let viewed = TagRef::from_object(object.as_ref()).unwrap();
            assert_eq!(viewed.as_ptr().cast(), object.as_ref().as_ptr());
            let from_view = viewed.to_tag();
            let tag = Tag::from_object(object.clone()).unwrap();
            assert!(TagArc::ptr_eq(&tag, &from_view));
            let back: ObjectArc<'_> = Tag::into_object(tag);
            assert!(ObjectArc::ptr_eq(&back, &object));
            drop((object, back));
            assert_eq!(view(from_view.as_ref().as_object()), tag_view(INNER));
        }

        /// The cache's reference is never visible, so an owner never proves itself
        /// sole: no exclusive handle, no box.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn an_owner_is_never_sole() {
            let repo = open();
            let mut tag = Tag::lookup(repo.as_ref(), sha1(TAG).as_ref()).unwrap();
            assert!(tag.get_mut().is_none());
            let tag = tag.try_into_box().unwrap_err();
            assert_eq!(format!("{:?}", tag.as_ref()), "Tag { .. }");
        }
    }
}
