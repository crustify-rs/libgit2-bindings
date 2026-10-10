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
//! database. It is an [`Object`](crate::libgit2::object::object::Object) of type tag:
//! the struct starts with the `git_object`, and `git_tag_lookup`, `git_tag_free` and
//! `git_tag_dup` are `git_object_lookup` (of type tag), `git_object_free` and
//! `git_object_dup` behind a cast. It is opaque: libgit2 publishes no field.
//!
//! So a tag is owned as an object is (see [`object`](crate::libgit2::object::object)'s
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
//! A tag is an object, and an object of type tag is a tag: [`TagRef::as_object`] views
//! a tag as its object, [`TagRef::from_object`] views an object of type tag as a tag,
//! and [`Tag::from_object`] and [`Tag::into_object`] hand one counted reference from one
//! owner to the other, checking the object's type on the way in. libgit2 itself never
//! checks the type of what it is given as a tag.

use core::ffi::{CStr, c_int};
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use ffibox::{CArc, CBorrowedPtr, CCell, CDrop, CRefClone};

use crate::ffi;
use crate::libgit2::object::object::{ObjectArc, ObjectRef, id_fits};
use crate::libgit2::oid::oid::OidRef;
use crate::libgit2::repository::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::util::errors::{ErrorBuf, last};
use crate::libgit2::util::sys::errors::runtime_pinned;

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
/// Fails, with libgit2's error, only when libgit2 runs out of memory building the
/// reference name, or cannot be initialised.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::object::tag::name_is_valid;
///
/// assert_eq!(name_is_valid(c"v1.0"), Ok(true));
/// assert_eq!(name_is_valid(c"release/2024"), Ok(true));
/// assert_eq!(name_is_valid(c"-v1.0"), Ok(false));
/// assert_eq!(name_is_valid(c"HEAD"), Ok(false));
/// assert_eq!(name_is_valid(c"v1..0"), Ok(false));
/// ```
///
/// Wraps: git_tag_name_is_valid
pub fn name_is_valid(name: &CStr) -> Result<bool, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut valid: c_int = 0;
    // SAFETY: `valid` is a writable `int`, which libgit2 writes before returning; `name`
    // is a non-NULL NUL-terminated string, read and copied for the call, never retained.
    // The runtime is pinned, so the allocation reads a settled allocator and a failure is
    // reported on live thread-local state.
    let rc = unsafe { ffi::git_tag_name_is_valid(&mut valid, name.as_ptr()) };
    if rc < 0 {
        return Err(last());
    }
    Ok(valid != 0)
}

/// An annotated tag (`git_tag`), parsed from a repository's object database: an
/// [`Object`](crate::libgit2::object::object::Object) of type tag.
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
/// use libgit2::libgit2::object::tag::TagArc;
///
/// fn widen<'r>(tag: TagArc<'r>) -> TagArc<'static> {
///     tag
/// }
/// ```
///
/// and a tag stays on the thread that looked it up:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::tag::TagArc;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(tag: TagArc<'static>) {
///     send(tag)
/// }
/// ```
///
/// Wraps: git_tag
/// Handwritten
#[repr(transparent)]
pub struct Tag<'r>(ffi::git_tag, PhantomData<(*const (), &'r ())>);

/// Shared borrow of a [`Tag`]. `Copy`, like `&T`.
///
/// It stays on the thread of the tag's repository, as the owner does:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::tag::TagRef;
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
    type Ref<'a>
        = TagRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = TagMut<'a, 'r>
    where
        Self: 'a;
}

impl<'a, 'r> TagRef<'a, 'r> {
    /// Borrows a raw pointer; `None` if null.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live tag looked up in a repository that outlives
    /// `'r`, which stays live during `'a` (a reference to it is held for at least that
    /// long) and which no one writes during `'a`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_tag) -> Option<Self> {
        // SAFETY: a non-null pointer the caller vouches for, per this function's
        // contract; `Tag` is transparent over `git_tag`.
        NonNull::new(ptr.cast_mut()).map(|p| TagRef(unsafe { CBorrowedPtr::new(p.cast()) }))
    }

    /// Read-only pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_tag {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The tag `object` is, borrowed as long as `object` is; `None` for an object of
    /// another type.
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
    pub fn as_object(&self) -> ObjectRef<'a, 'r> {
        // SAFETY: a `git_tag` starts with its `git_object`, so the tag's pointer is its
        // object's, live and unwritten for `'a` as the tag is, of a repository that
        // outlives `'r`.
        let object = unsafe { ObjectRef::from_ptr(self.as_ptr().cast()) };
        object.expect("a handle is never NULL")
    }

    /// Another counted reference to the tag, as `git_tag_dup` takes one: the same tag,
    /// now also kept alive by the result, which may outlive this borrow but not the
    /// repository.
    ///
    /// Wraps: git_tag_dup
    pub fn to_tag(&self) -> TagArc<'r> {
        let tag = self.0.as_non_null();
        // SAFETY: the handle addresses a live tag for `'a`, whose reference its holder
        // keeps meanwhile; one more is counted on it here.
        if !unsafe { TagFree.c_up_ref(tag) } {
            unreachable!("git_tag_dup cannot fail");
        }
        // SAFETY: the reference just counted, given to the result, which `git_tag_free`
        // releases; the tag's repository outlives `'r`, which the result carries.
        unsafe { TagArc::from_raw(tag.as_ptr()) }.expect("a handle is never NULL")
    }
}

/// Whether `object` is of type tag, as libgit2 recorded it when parsing it.
fn is_tag(object: ObjectRef<'_, '_>) -> bool {
    // SAFETY: the handle addresses a live object; `git_object_type` reads its cached
    // type.
    unsafe { ffi::git_object_type(object.as_ptr()) == ffi::GIT_OBJECT_TAG }
}

impl<'a, 'r> TagMut<'a, 'r> {
    /// Borrows a raw pointer exclusively; `None` if null.
    ///
    /// # Safety
    ///
    /// As [`TagRef::from_ptr`], plus: nothing else reaches the tag while the result
    /// lives, its repository's cache included.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_tag) -> Option<Self> {
        // SAFETY: the caller upholds `TagRef::from_ptr`'s contract and exclusivity.
        unsafe { TagRef::from_ptr(ptr) }.map(|r| TagMut(r, PhantomData))
    }

    /// Writable pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_tag {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
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

/// Releases one counted reference to a tag through `git_tag_free`, and takes another
/// through `git_tag_dup`.
///
/// They are `git_object_free` and `git_object_dup` behind a cast: the free decrements
/// the object's atomic count and, at zero, frees the tag with its name, tagger and
/// message (`git_tag__free`), never reading its repository; the copy increments the same
/// count and hands back the same pointer. Neither can fail, and neither allocates.
///
/// Wraps: git_tag_free
/// Wraps: git_tag_dup
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct TagFree;

// SAFETY: `git_tag_free` gives back exactly one counted reference of a live tag (a NULL
// pointer, which it skips, never reaches it); the tag outlives the call through that
// reference, and is freed only once the count, which the cache's reference is part of,
// reaches zero.
unsafe impl<'r> CDrop<Tag<'r>> for TagFree {
    unsafe fn c_drop(&self, ptr: NonNull<Tag<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live tag of which the
        // caller gives up one reference; `Tag` is transparent over `git_tag`.
        unsafe { ffi::git_tag_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `git_tag_dup` atomically increments the count of the live tag it is given and
// stores that same pointer in `out`, returning 0: the tag then owes one more
// `git_tag_free`, which this policy performs, and `ptr` stays valid. It never fails.
// `c_is_sole_owner` keeps its `false` default: the repository's cache counts a reference
// no owner sees, and a later lookup may take another.
unsafe impl<'r> CRefClone<Tag<'r>> for TagFree {
    unsafe fn c_up_ref(&self, ptr: NonNull<Tag<'r>>) -> bool {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `ptr` addresses a live tag whose reference
        // the caller holds, per `c_up_ref`'s contract.
        let rc = unsafe { ffi::git_tag_dup(&mut out, ptr.as_ptr().cast()) };
        debug_assert_eq!(out, ptr.as_ptr().cast());
        rc == 0
    }
}

/// One counted reference to a [`Tag`], given back by `git_tag_free` on drop; `clone`
/// takes another through `git_tag_dup`, on the same tag.
pub type TagArc<'r> = CArc<Tag<'r>, TagFree>;

impl<'r> Tag<'r> {
    /// The tag `id` names in `repo`.
    ///
    /// The repository's cache answers when it holds the tag, with the tag it holds;
    /// otherwise libgit2 reads it from the object database, parses it and caches it,
    /// so that every later lookup of the id, as a tag or as an object, yields the same
    /// tag while the cache keeps it.
    ///
    /// Fails as [`Object::lookup`](crate::libgit2::object::object::Object::lookup) with
    /// type [`ObjectType::Tag`](crate::libgit2::core::types::ObjectType::Tag) does:
    /// with [`ErrorCode::NotFound`](crate::libgit2::util::errors::ErrorCode::NotFound)
    /// when the object database has no such object, or the object is not a tag (class
    /// `GIT_ERROR_INVALID`, "the requested type does not match the type in the ODB"),
    /// and with libgit2's error when the tag cannot be read or parsed. An `id` of
    /// another type than `repo`'s is refused before the call, as there.
    ///
    /// Wraps: git_tag_lookup
    pub fn lookup(repo: RepositoryRef<'r>, id: OidRef<'_>) -> Result<TagArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `repo` is a live
        // repository on this thread, whose odb libgit2 loads and whose cache it fills
        // through this shared handle, as `RepositoryRef` allows; `id` is a live id of
        // the repository's type (checked above), read for the call only.
        let rc = unsafe { ffi::git_tag_lookup(&mut out, repo.as_ptr().cast_mut(), id.as_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live tag of `repo`,
        // borrowed for `'r`, given to the result and released by `git_tag_free`.
        let tag = unsafe { TagArc::from_c(out) };
        Ok(tag.expect("a lookup succeeded without a tag"))
    }

    /// The tag whose id starts with the first `len` hex digits of `id`, in `repo`; a
    /// `len` past the repository's full length means the full id.
    ///
    /// The digits past `len` are ignored, and every object in the database is a
    /// candidate, whatever its type: the prefix must name exactly one, which must then
    /// be a tag. Fails as
    /// [`Object::lookup_prefix`](crate::libgit2::object::object::Object::lookup_prefix)
    /// with type tag does, and refuses an `id` of another type than `repo`'s as it does,
    /// whatever `len`.
    ///
    /// Wraps: git_tag_lookup_prefix
    pub fn lookup_prefix(
        repo: RepositoryRef<'r>,
        id: OidRef<'_>,
        len: usize,
    ) -> Result<TagArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: as in `lookup`; libgit2 clamps `len` to the repository's length, and
        // copies at most `(len + 1) / 2` of `id`'s bytes, with its type, into its key.
        let rc = unsafe {
            ffi::git_tag_lookup_prefix(&mut out, repo.as_ptr().cast_mut(), id.as_ptr(), len)
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live tag of `repo`,
        // borrowed for `'r`, given to the result and released by `git_tag_free`.
        let tag = unsafe { TagArc::from_c(out) };
        Ok(tag.expect("a lookup succeeded without a tag"))
    }

    /// `object`'s reference, as a tag's: the same object, still counted once, now owned
    /// as the tag it is; `object` back, untouched, when it is not of type tag.
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
    pub fn into_object(tag: TagArc<'r>) -> ObjectArc<'r> {
        let raw = tag.into_c();
        // SAFETY: the reference the tag owner gave up, to a live tag, which starts with
        // its `git_object`, of a repository borrowed for `'r`; `git_object_free`
        // releases it as `git_tag_free` would.
        let object = unsafe { ObjectArc::from_c(raw.cast()) };
        object.expect("an owner is never NULL")
    }
}

// A `Tag` is a `git_tag`, never held by value; the owner is one pointer.
const _: () = assert!(
    size_of::<Tag<'static>>() == size_of::<ffi::git_tag>()
        && size_of::<TagArc<'static>>() == size_of::<*mut ffi::git_tag>()
        && size_of::<Option<TagArc<'static>>>() == size_of::<*mut ffi::git_tag>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::util::errors::ErrorClass;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use crate::libgit2::util::sys::errors::set;
    use core::ffi::c_void;
    use core::ptr::NonNull;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::SeqCst};

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

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
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
            let p = p.map_or(core::ptr::null_mut(), |p| p.as_ptr().cast());
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
                ffi::GIT_OPT_SET_ALLOCATOR as c_int,
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

    /// `git_tag`'s owner, handles, lifecycle policy and conversions.
    mod tag_object {
        use crate::ffi;
        use crate::libgit2::core::types::ObjectType;
        use crate::libgit2::object::object::{Object, ObjectArc, ObjectRef};
        use crate::libgit2::object::tag::*;
        use crate::libgit2::oid::oid::{Oid, OidRef, OidType};
        use crate::libgit2::repository::repository::{Repository, RepositoryBox};
        use crate::libgit2::util::errors::{ErrorClass, ErrorCode};
        use crate::libgit2::util::sys::errors::runtime_pinned;
        use core::ffi::CStr;
        use core::mem::size_of;
        use core::ptr::{self, NonNull};
        use ffibox::{CDrop, CRefClone};

        /// `testrepo.git`: tag `b25fa35` (`test`) of tag `7b43849`, of commit `e90810b`.
        const TESTREPO: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/resources/testrepo.git\0"
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
            // SAFETY: `p` is a live object the handle borrows; its id lives as long as it
            // and is formatted at once.
            unsafe {
                let id = OidRef::from_ptr(ffi::git_object_id(p).cast_mut()).unwrap();
                (id.to_hex_string().unwrap(), ffi::git_object_type(p))
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
            for kind in [OidType::Sha256.raw() as u8, 0, 9, u8::MAX] {
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
