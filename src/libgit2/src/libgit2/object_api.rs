//! The typed object API of `src/libgit2/object_api.c`: the lookups, by id and by id
//! prefix, of blobs, commits, tags and trees, their ids, their repositories, the extra
//! counted references their handles hand out (`git_<type>_dup`), and the release
//! policies their owners free them through (`git_<type>_free`). The types themselves,
//! and the rest of their API, are their own modules': [`blob`](super::blob),
//! [`commit`](super::commit), [`tag`](super::tag) and [`tree`](super::tree).

use core::ffi::c_int;
use core::ptr::{self, NonNull};

use ffibox::{CDrop, CRefClone};

use crate::ffi;
use crate::libgit2::src::libgit2::blob::{Blob, BlobArc, BlobRef};
use crate::libgit2::src::libgit2::commit::{Commit, CommitArc, CommitRef};
use crate::libgit2::src::libgit2::object::id_fits;
use crate::libgit2::src::libgit2::oid::OidRef;
use crate::libgit2::src::libgit2::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::src::libgit2::signature::{SignatureBox, SignatureRef};
use crate::libgit2::src::libgit2::tag::{Tag, TagArc, TagRef};
use crate::libgit2::src::libgit2::tree::{Tree, TreeArc, TreeRef};
use crate::libgit2::src::util::errors::{ErrorBuf, ErrorClass, last, set};

impl<'a, 'r> BlobRef<'a, 'r> {
    /// Another counted reference to the blob, as `git_blob_dup` takes one: the same
    /// blob, now also kept alive by the result, which may outlive this borrow but not
    /// the repository.
    ///
    /// Wraps: `git_blob_dup`
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a handle is never NULL"
    )]
    pub fn to_blob(&self) -> BlobArc<'r> {
        let blob = self.as_non_null();
        // SAFETY: the handle addresses a live blob for `'a`, whose reference its holder
        // keeps meanwhile; one more is counted on it here.
        if !unsafe { BlobFree.c_up_ref(blob) } {
            unreachable!("git_blob_dup cannot fail");
        }
        // SAFETY: the reference just counted, given to the result, which `git_blob_free`
        // releases; the blob's repository outlives `'r`, which the result carries.
        unsafe { BlobArc::from_raw(blob.as_ptr()) }.expect("a handle is never NULL")
    }

    /// The blob's id, of its repository's id type: the id it was looked up by, or the
    /// full id a prefix found. Borrowed for as long as the handle: libgit2 keeps it in
    /// the blob, which writes it once, when it is read.
    ///
    /// Wraps: `git_blob_id`
    ///
    /// # Panics
    ///
    /// Panics, with `a live blob has an id`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: the handle addresses a live blob for `'a`; libgit2 only reads it, and
        // returns the address of the id cached in its object (`&obj->cached.oid`).
        let id = unsafe { ffi::git_blob_id(self.as_ptr()) };
        // SAFETY: the id is part of the blob, so it is non-NULL and lives as long as the
        // blob, past `'a`; nothing writes it once the blob is returned.
        unsafe { OidRef::from_ptr(id.cast_mut()) }.expect("a live blob has an id")
    }

    /// The repository the blob was looked up in, borrowed for `'r`, as the shared
    /// handle it was looked up through
    /// ([`ObjectRef::owner`](crate::libgit2::src::libgit2::object::ObjectRef::owner) for a blob).
    ///
    /// The result outlives the blob, but not the repository's borrow:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::blob::Blob;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let blob = Blob::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let owner = blob.as_ref().owner();
    /// drop(repo);
    /// let _ = owner.path();
    /// ```
    ///
    /// Wraps: `git_blob_owner`
    ///
    /// # Panics
    ///
    /// Panics, with `a blob always has a repository`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn owner(&self) -> RepositoryRef<'r> {
        // SAFETY: the handle addresses a live blob for `'a`, which libgit2 only reads.
        let repo = unsafe { ffi::git_blob_owner(self.as_ptr()) };
        // SAFETY: `obj->repo` is the repository the blob was looked up in, stored
        // without a count and never cleared. A `Blob<'r>` is only built from a
        // `RepositoryRef<'r>` (`lookup`, `lookup_prefix`) or from an `Object<'r>`, so
        // that repository is live and shared, not exclusively borrowed, for `'r`, on
        // this thread.
        unsafe { RepositoryRef::from_ptr(repo) }.expect("a blob always has a repository")
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
/// Wraps: `git_blob_free`
/// Wraps: `git_blob_dup`
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
        let rc = unsafe { ffi::git_blob_dup(&raw mut out, ptr.as_ptr().cast()) };
        debug_assert_eq!(out, ptr.as_ptr().cast());
        rc == 0
    }
}

impl<'r> Blob<'r> {
    /// The blob `id` names in `repo`: [`Object::lookup`] for type blob.
    ///
    /// libgit2 reads it from the object database and parses it afresh, or, when the
    /// cache keeps blobs and holds this one, answers with the blob it holds.
    ///
    /// The blob keeps the repository it was looked up in, so it lives no longer than
    /// this borrow of it:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::blob::Blob;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let blob = Blob::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// drop(repo);
    /// drop(blob);
    /// ```
    ///
    /// [`Object::lookup`]: crate::libgit2::src::libgit2::object::Object::lookup
    /// [`ErrorCode::NotFound`]: crate::libgit2::src::util::errors::ErrorCode::NotFound
    ///
    /// Wraps: `git_blob_lookup`
    ///
    /// # Errors
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
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without a blob`, only if libgit2 breaks its contract.
    pub fn lookup(repo: RepositoryRef<'r>, id: OidRef<'_>) -> Result<BlobArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `repo` is a live
        // repository on this thread, whose odb libgit2 loads and whose cache it reads
        // and fills through this shared handle, as `RepositoryRef` allows; `id` is a
        // live id of the repository's type (checked above), read for the call only.
        let rc =
            unsafe { ffi::git_blob_lookup(&raw mut out, repo.as_ptr().cast_mut(), id.as_ptr()) };
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
    /// [`Object::lookup_prefix`]: crate::libgit2::src::libgit2::object::Object::lookup_prefix
    /// [`ErrorCode::Ambiguous`]: crate::libgit2::src::util::errors::ErrorCode::Ambiguous
    /// [`ErrorCode::NotFound`]: crate::libgit2::src::util::errors::ErrorCode::NotFound
    ///
    /// Wraps: `git_blob_lookup_prefix`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::Ambiguous`] when `len` is under 4 or the prefix names
    /// several objects, with [`ErrorCode::NotFound`] when it names none or one that is
    /// not a blob, and as [`lookup`](Self::lookup) otherwise; an `id` of another type
    /// than `repo`'s is refused as it is, whatever `len`.
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without a blob`, only if libgit2 breaks its contract.
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
            ffi::git_blob_lookup_prefix(&raw mut out, repo.as_ptr().cast_mut(), id.as_ptr(), len)
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
}

// A `Blob` is a `git_blob`, never held by value; the owner is one pointer.
const _: () = assert!(
    size_of::<Blob<'static>>() == size_of::<ffi::git_blob>()
        && size_of::<BlobArc<'static>>() == size_of::<*mut ffi::git_blob>()
        && size_of::<Option<BlobArc<'static>>>() == size_of::<*mut ffi::git_blob>()
);

impl<'a, 'r> CommitRef<'a, 'r> {
    /// The commit's id: the id it was looked up by, or the full id a prefix found.
    /// Borrowed for as long as the handle: libgit2 keeps it in the commit, which writes
    /// it once, when it is parsed.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::commit::Commit;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let commit_id = commit.as_ref().id();
    /// drop(commit);
    /// let _ = commit_id.raw_kind();
    /// ```
    ///
    /// Wraps: `git_commit_id`
    ///
    /// # Panics
    ///
    /// Panics, with `a live commit has an id`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: the handle addresses a live commit for `'a`; libgit2 only reads it, and
        // returns the address of the id cached in its object (`&obj->cached.oid`).
        let id = unsafe { ffi::git_commit_id(self.as_ptr()) };
        // SAFETY: the id is part of the commit, so it is non-NULL and lives as long as
        // the commit, past `'a`; nothing writes it once the commit is cached or returned.
        unsafe { OidRef::from_ptr(id.cast_mut()) }.expect("a live commit has an id")
    }

    /// The repository the commit was looked up in, borrowed for `'r`, as the shared
    /// handle it was looked up through, as
    /// [`ObjectRef::owner`](crate::libgit2::src::libgit2::object::ObjectRef::owner) gives it.
    ///
    /// The result is that handle again, so it lets nothing happen that the commit's own
    /// lookups do not: it outlives the commit, but not the repository's borrow.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::commit::Commit;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let owner = commit.as_ref().owner();
    /// drop(repo);
    /// let _ = owner.path();
    /// ```
    ///
    /// Wraps: `git_commit_owner`
    ///
    /// # Panics
    ///
    /// Panics, with `a commit always has a repository`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn owner(&self) -> RepositoryRef<'r> {
        // SAFETY: the handle addresses a live commit for `'a`, which libgit2 only reads.
        let repo = unsafe { ffi::git_commit_owner(self.as_ptr()) };
        // SAFETY: `object.repo`, stored uncounted and never cleared, is the repository
        // the commit was looked up in. A `Commit<'r>` is only built from a
        // `RepositoryRef<'r>`, from a `Commit<'r>` or `Object<'r>` of it, or adopted under
        // a contract vouching for the same: the repository is live and shared on this
        // thread for `'r`, and the result is the handle the commit was looked up through.
        unsafe { RepositoryRef::from_ptr(repo) }.expect("a commit always has a repository")
    }
}

/// Releases one counted reference to a commit through `git_commit_free`, and takes
/// another through `git_commit_dup`.
///
/// They are `git_object_free` and `git_object_dup` on the commit's object: the free
/// decrements the object's atomic count and, at zero, frees the commit with its
/// parents, signatures and strings, never reading its repository; the copy
/// increments the same count and hands back the same pointer, so a copy is the same
/// commit, of the same repository, for the same `'r`. Neither can fail, and neither
/// allocates.
///
/// Wraps: `git_commit_free`
/// Wraps: `git_commit_dup`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct CommitFree;

// SAFETY: `git_commit_free` gives back exactly one counted reference of a live commit,
// in every state the owners reach (a NULL pointer, which it skips, never reaches it);
// the commit outlives the call through that reference, and is freed only once the
// count, which the cache's reference is part of, reaches zero.
unsafe impl<'r> CDrop<Commit<'r>> for CommitFree {
    unsafe fn c_drop(&self, ptr: NonNull<Commit<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live commit of which the
        // caller gives up one reference; `Commit` is transparent over `git_commit`.
        unsafe { ffi::git_commit_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `git_commit_dup` atomically increments the count of the live commit it is
// given and stores that same pointer in `out`, returning 0: the commit then owes one
// more `git_commit_free`, which this policy performs, and `ptr` stays valid. It never
// fails. `c_is_sole_owner` keeps its `false` default: the repository's cache counts a
// reference no owner sees, and a later lookup may take another.
unsafe impl<'r> CRefClone<Commit<'r>> for CommitFree {
    unsafe fn c_up_ref(&self, ptr: NonNull<Commit<'r>>) -> bool {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `ptr` addresses a live commit whose
        // reference the caller holds, per `c_up_ref`'s contract.
        let rc = unsafe { ffi::git_commit_dup(&raw mut out, ptr.as_ptr().cast()) };
        debug_assert_eq!(out, ptr.as_ptr().cast());
        rc == 0
    }
}

impl<'r> Commit<'r> {
    /// The commit `id` names in `repo`.
    ///
    /// The repository's cache answers when it holds the commit, with the commit it
    /// holds; otherwise libgit2 reads it from the object database, parses it and,
    /// under the cache's size limit, caches it, so that every later lookup of the id
    /// yields the same commit while the cache keeps it.
    ///
    /// The commit reads the repository it was looked up in, so it lives no longer than
    /// this borrow of it:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::commit::Commit;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// drop(repo);
    /// drop(commit);
    /// ```
    ///
    /// Wraps: `git_commit_lookup`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`](crate::libgit2::src::util::errors::ErrorCode::NotFound)
    /// when the object database has no such object (the zero id included), when the
    /// object is not a commit (class `GIT_ERROR_INVALID`, "the requested type does not
    /// match the type in the ODB"; a tag is not peeled, whatever libgit2's header
    /// says), and for a repository without a git directory, which has no object
    /// database to load (`GIT_ERROR_INVALID`, "path cannot exist in repository"); and
    /// with libgit2's error when the object database cannot be loaded or read, or the
    /// commit cannot be parsed.
    ///
    /// Refuses, with code `-1` and libgit2's `GIT_ERROR_INVALID` error "specified
    /// object ID type does not match repository object ID type", an `id` whose type is
    /// not `repo`'s, which libgit2 would format out of bounds (see the
    /// [`object` module](super::object#looking-objects-up)).
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without a commit`, only if libgit2 breaks its contract.
    pub fn lookup(
        repo: RepositoryRef<'r>,
        id: OidRef<'_>,
    ) -> Result<CommitArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `repo` is a live
        // repository on this thread, whose odb libgit2 loads and whose cache it fills
        // through this shared handle, as `RepositoryRef` allows; `id` is a live id of
        // the repository's type (checked above), read for the call only.
        let rc =
            unsafe { ffi::git_commit_lookup(&raw mut out, repo.as_ptr().cast_mut(), id.as_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live commit of
        // `repo` (an object of type `GIT_OBJECT_COMMIT`), borrowed for `'r`, given to
        // the result and released by `git_commit_free`.
        let commit = unsafe { CommitArc::from_c(out) };
        Ok(commit.expect("a lookup succeeded without a commit"))
    }

    /// The commit whose id starts with the first `len` hex digits of `id`, in `repo`.
    /// A `len` past the repository's full length (40 hex digits for SHA-1, 64 for
    /// SHA-256) means the full id, which is [`lookup`](Self::lookup).
    ///
    /// The digits past `len` are ignored, and every object in the database is a
    /// candidate, whatever its type: the prefix must name exactly one, which must then
    /// be a commit.
    ///
    /// Wraps: `git_commit_lookup_prefix`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::Ambiguous`](crate::libgit2::src::util::errors::ErrorCode::Ambiguous)
    /// when `len` is under 4 (`GIT_ERROR_OBJECT`, "ambiguous lookup - OID prefix is too
    /// short") or the prefix names several objects, with
    /// [`ErrorCode::NotFound`](crate::libgit2::src::util::errors::ErrorCode::NotFound) when
    /// it names none or one that is not a commit, and as [`lookup`](Self::lookup)
    /// otherwise; an `id` of another type than `repo`'s is refused as it is, whatever
    /// `len`.
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without a commit`, only if libgit2 breaks its contract.
    pub fn lookup_prefix(
        repo: RepositoryRef<'r>,
        id: OidRef<'_>,
        len: usize,
    ) -> Result<CommitArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: as in `lookup`; libgit2 clamps `len` to the repository's length, and
        // copies at most `(len + 1) / 2` of `id`'s `GIT_OID_MAX_SIZE` bytes, with its
        // type, into the key it searches with.
        let rc = unsafe {
            ffi::git_commit_lookup_prefix(&raw mut out, repo.as_ptr().cast_mut(), id.as_ptr(), len)
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: as in `lookup`: one counted reference to a live commit of `repo`.
        let commit = unsafe { CommitArc::from_c(out) };
        Ok(commit.expect("a lookup succeeded without a commit"))
    }
}

// A `Commit` is a `git_commit`, never held by value; the owner is one pointer.
const _: () = assert!(
    size_of::<Commit<'static>>() == size_of::<ffi::git_commit>()
        && size_of::<CommitArc<'static>>() == size_of::<*mut ffi::git_commit>()
        && size_of::<Option<CommitArc<'static>>>() == size_of::<*mut ffi::git_commit>()
);

/// `Ok` for a signature whose name and email libgit2's mailmap can resolve; for one
/// missing either, the error a release build of libgit2 reports from the assertion in
/// `git_mailmap_resolve`, set as the thread's.
pub(crate) fn resolvable(signature: SignatureRef<'_>) -> Result<(), ErrorBuf> {
    if signature.name().is_none() {
        set(
            ErrorClass::Internal,
            c"unrecoverable internal error: 'name'",
        );
        return Err(last());
    }
    if signature.email().is_none() {
        set(
            ErrorClass::Internal,
            c"unrecoverable internal error: 'email'",
        );
        return Err(last());
    }
    Ok(())
}

/// The signature a routine returning `rc` handed out, or libgit2's error.
pub(crate) fn signed(rc: c_int, signature: Option<SignatureBox>) -> Result<SignatureBox, ErrorBuf> {
    signature.filter(|_| rc >= 0).ok_or_else(last)
}

impl<'r> TagRef<'_, 'r> {
    /// Another counted reference to the tag, as `git_tag_dup` takes one: the same tag,
    /// now also kept alive by the result, which may outlive this borrow but not the
    /// repository.
    ///
    /// Wraps: `git_tag_dup`
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a handle is never NULL"
    )]
    pub fn to_tag(&self) -> TagArc<'r> {
        let tag = self.as_non_null();
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

/// Releases one counted reference to a tag through `git_tag_free`, and takes another
/// through `git_tag_dup`.
///
/// They are `git_object_free` and `git_object_dup` behind a cast: the free decrements
/// the object's atomic count and, at zero, frees the tag with its name, tagger and
/// message (`git_tag__free`), never reading its repository; the copy increments the same
/// count and hands back the same pointer. Neither can fail, and neither allocates.
///
/// Wraps: `git_tag_free`
/// Wraps: `git_tag_dup`
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
        let rc = unsafe { ffi::git_tag_dup(&raw mut out, ptr.as_ptr().cast()) };
        debug_assert_eq!(out, ptr.as_ptr().cast());
        rc == 0
    }
}

impl<'r> Tag<'r> {
    /// The tag `id` names in `repo`.
    ///
    /// The repository's cache answers when it holds the tag, with the tag it holds;
    /// otherwise libgit2 reads it from the object database, parses it and caches it,
    /// so that every later lookup of the id, as a tag or as an object, yields the same
    /// tag while the cache keeps it.
    ///
    /// Wraps: `git_tag_lookup`
    ///
    /// # Errors
    ///
    /// Fails as [`Object::lookup`](crate::libgit2::src::libgit2::object::Object::lookup) with
    /// type [`ObjectType::Tag`](crate::libgit2::src::libgit2::object::ObjectType::Tag) does:
    /// with [`ErrorCode::NotFound`](crate::libgit2::src::util::errors::ErrorCode::NotFound)
    /// when the object database has no such object, or the object is not a tag (class
    /// `GIT_ERROR_INVALID`, "the requested type does not match the type in the ODB"),
    /// and with libgit2's error when the tag cannot be read or parsed. An `id` of
    /// another type than `repo`'s is refused before the call, as there.
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without a tag`, only if libgit2 breaks its contract.
    pub fn lookup(repo: RepositoryRef<'r>, id: OidRef<'_>) -> Result<TagArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `repo` is a live
        // repository on this thread, whose odb libgit2 loads and whose cache it fills
        // through this shared handle, as `RepositoryRef` allows; `id` is a live id of
        // the repository's type (checked above), read for the call only.
        let rc =
            unsafe { ffi::git_tag_lookup(&raw mut out, repo.as_ptr().cast_mut(), id.as_ptr()) };
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
    /// [`Object::lookup_prefix`](crate::libgit2::src::libgit2::object::Object::lookup_prefix)
    /// with type tag does, and refuses an `id` of another type than `repo`'s as it does,
    /// whatever `len`.
    ///
    /// Wraps: `git_tag_lookup_prefix`
    ///
    /// # Errors
    ///
    /// Fails as [`Object::lookup_prefix`](crate::libgit2::src::libgit2::object::Object::lookup_prefix)
    /// with type tag does, and refuses an `id` of another type than `repo`'s as it does, whatever
    /// `len`.
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without a tag`, only if libgit2 breaks its contract.
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
            ffi::git_tag_lookup_prefix(&raw mut out, repo.as_ptr().cast_mut(), id.as_ptr(), len)
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live tag of `repo`,
        // borrowed for `'r`, given to the result and released by `git_tag_free`.
        let tag = unsafe { TagArc::from_c(out) };
        Ok(tag.expect("a lookup succeeded without a tag"))
    }
}

// A `Tag` is a `git_tag`, never held by value; the owner is one pointer.
const _: () = assert!(
    size_of::<Tag<'static>>() == size_of::<ffi::git_tag>()
        && size_of::<TagArc<'static>>() == size_of::<*mut ffi::git_tag>()
        && size_of::<Option<TagArc<'static>>>() == size_of::<*mut ffi::git_tag>()
);

impl<'a, 'r> TagRef<'a, 'r> {
    /// The tag object's id, of its repository's id type: the id it was looked up by, or
    /// the full id a prefix found. Borrowed for as long as the handle: libgit2 keeps it
    /// in the tag, which writes it once, when it is parsed.
    ///
    /// Wraps: `git_tag_id`
    ///
    /// # Panics
    ///
    /// Panics, with `a live tag has an id`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: the handle addresses a live tag for `'a`; `git_tag_id` is
        // `git_object_id` behind a cast, which only reads it and returns the address of
        // the id cached in it (`&obj->cached.oid`).
        let id = unsafe { ffi::git_tag_id(self.as_ptr()) };
        // SAFETY: the id is part of the tag, so it is non-NULL and lives as long as the
        // tag, past `'a`; nothing writes it once the tag is cached or returned.
        unsafe { OidRef::from_ptr(id.cast_mut()) }.expect("a live tag has an id")
    }

    /// The repository the tag was looked up in, borrowed for `'r`, as the shared handle
    /// it was looked up through, as
    /// [`ObjectRef::owner`](crate::libgit2::src::libgit2::object::ObjectRef::owner) gives it for
    /// the tag's object.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::tag::Tag;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let tag = Tag::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let owner = tag.as_ref().owner();
    /// drop(repo);
    /// let _ = owner.path();
    /// ```
    ///
    /// Wraps: `git_tag_owner`
    ///
    /// # Panics
    ///
    /// Panics, with `a tag always has a repository`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn owner(&self) -> RepositoryRef<'r> {
        // SAFETY: the handle addresses a live tag for `'a`; `git_tag_owner` is
        // `git_object_owner` behind a cast, which only reads it.
        let repo = unsafe { ffi::git_tag_owner(self.as_ptr()) };
        // SAFETY: `obj->repo` is the repository the tag was looked up in, stored without
        // a count and never cleared. A `Tag<'r>` is only built from a
        // `RepositoryRef<'r>` (`lookup`, `lookup_prefix`) or from an `Object<'r>` of
        // type tag, itself built that way, so that repository is live and shared, not
        // exclusively borrowed, for `'r`, on this thread: the handle returned is the one
        // the tag was looked up through.
        unsafe { RepositoryRef::from_ptr(repo) }.expect("a tag always has a repository")
    }
}

impl<'r> TreeRef<'_, 'r> {
    /// Another counted reference to the tree, as `git_tree_dup` takes one: the same
    /// tree, now also kept alive by the result, which may outlive this borrow but not
    /// the repository.
    ///
    /// Wraps: `git_tree_dup`
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a handle is never NULL"
    )]
    pub fn to_tree(&self) -> TreeArc<'r> {
        let tree = self.as_non_null();
        // SAFETY: the handle addresses a live tree for `'a`, whose reference its holder
        // keeps meanwhile; one more is counted on it here.
        if !unsafe { TreeFree.c_up_ref(tree) } {
            unreachable!("git_tree_dup cannot fail");
        }
        // SAFETY: the reference just counted, given to the result, which `git_tree_free`
        // releases; the tree's repository outlives `'r`, which the result carries.
        unsafe { TreeArc::from_raw(tree.as_ptr()) }.expect("a handle is never NULL")
    }
}

/// Releases one counted reference to a tree through `git_tree_free`, and takes another
/// through `git_tree_dup`.
///
/// They are `git_object_free` and `git_object_dup` behind a cast: the free decrements
/// the object's atomic count and, at zero, frees the tree with its entries and its
/// reference to the raw object data (`git_tree__free`), never reading its repository;
/// the copy increments the same count and hands back the same pointer. Neither can
/// fail, and neither allocates.
///
/// Wraps: `git_tree_free`
/// Wraps: `git_tree_dup`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct TreeFree;

// SAFETY: `git_tree_free` gives back exactly one counted reference of a live tree (a
// NULL pointer, which it skips, never reaches it); the tree outlives the call through
// that reference, and is freed only once the count, which the cache's reference is part
// of, reaches zero.
unsafe impl<'r> CDrop<Tree<'r>> for TreeFree {
    unsafe fn c_drop(&self, ptr: NonNull<Tree<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live tree of which the
        // caller gives up one reference; `Tree` is transparent over `git_tree`.
        unsafe { ffi::git_tree_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `git_tree_dup` atomically increments the count of the live tree it is given
// and stores that same pointer in `out`, returning 0: the tree then owes one more
// `git_tree_free`, which this policy performs, and `ptr` stays valid. It never fails.
// `c_is_sole_owner` keeps its `false` default: the repository's cache counts a reference
// no owner sees, and a later lookup may take another.
unsafe impl<'r> CRefClone<Tree<'r>> for TreeFree {
    unsafe fn c_up_ref(&self, ptr: NonNull<Tree<'r>>) -> bool {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `ptr` addresses a live tree whose reference
        // the caller holds, per `c_up_ref`'s contract.
        let rc = unsafe { ffi::git_tree_dup(&raw mut out, ptr.as_ptr().cast()) };
        debug_assert_eq!(out, ptr.as_ptr().cast());
        rc == 0
    }
}

impl<'r> Tree<'r> {
    /// The tree `id` names in `repo`.
    ///
    /// The repository's cache answers when it holds the tree, with the tree it holds;
    /// otherwise libgit2 reads it from the object database, parses it and, under the
    /// cache's size limit, caches it, so that every later lookup of the id, as a tree
    /// or as an object, yields the same tree while the cache keeps it.
    ///
    /// Wraps: `git_tree_lookup`
    ///
    /// # Errors
    ///
    /// Fails as [`Object::lookup`](crate::libgit2::src::libgit2::object::Object::lookup) with
    /// type [`ObjectType::Tree`](crate::libgit2::src::libgit2::object::ObjectType::Tree) does:
    /// with [`ErrorCode::NotFound`](crate::libgit2::src::util::errors::ErrorCode::NotFound)
    /// when the object database has no such object, or the object is not a tree (class
    /// `GIT_ERROR_INVALID`, "the requested type does not match the type in the ODB"),
    /// and with libgit2's error when the tree cannot be read or parsed. An `id` of
    /// another type than `repo`'s is refused before the call, as there.
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without a tree`, only if libgit2 breaks its contract.
    pub fn lookup(repo: RepositoryRef<'r>, id: OidRef<'_>) -> Result<TreeArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `repo` is a live
        // repository on this thread, whose odb libgit2 loads and whose cache it fills
        // through this shared handle, as `RepositoryRef` allows; `id` is a live id of
        // the repository's type (checked above), read for the call only.
        let rc =
            unsafe { ffi::git_tree_lookup(&raw mut out, repo.as_ptr().cast_mut(), id.as_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live tree of
        // `repo`, borrowed for `'r`, given to the result and released by
        // `git_tree_free`.
        let tree = unsafe { TreeArc::from_c(out) };
        Ok(tree.expect("a lookup succeeded without a tree"))
    }

    /// The tree whose id starts with the first `len` hex digits of `id`, in `repo`; a
    /// `len` past the repository's full length means the full id.
    ///
    /// The digits past `len` are ignored, and every object in the database is a
    /// candidate, whatever its type: the prefix must name exactly one, which must then
    /// be a tree. Fails as
    /// [`Object::lookup_prefix`](crate::libgit2::src::libgit2::object::Object::lookup_prefix)
    /// with type tree does, and refuses an `id` of another type than `repo`'s as it
    /// does, whatever `len`.
    ///
    /// Wraps: `git_tree_lookup_prefix`
    ///
    /// # Errors
    ///
    /// Fails as [`Object::lookup_prefix`](crate::libgit2::src::libgit2::object::Object::lookup_prefix)
    /// with type tree does, and refuses an `id` of another type than `repo`'s as it does, whatever
    /// `len`.
    ///
    /// # Panics
    ///
    /// Panics, with `a lookup succeeded without a tree`, only if libgit2 breaks its contract.
    pub fn lookup_prefix(
        repo: RepositoryRef<'r>,
        id: OidRef<'_>,
        len: usize,
    ) -> Result<TreeArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: as in `lookup`; libgit2 clamps `len` to the repository's length, and
        // copies at most `(len + 1) / 2` of `id`'s bytes, with its type, into its key.
        let rc = unsafe {
            ffi::git_tree_lookup_prefix(&raw mut out, repo.as_ptr().cast_mut(), id.as_ptr(), len)
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live tree of
        // `repo`, borrowed for `'r`, given to the result and released by
        // `git_tree_free`.
        let tree = unsafe { TreeArc::from_c(out) };
        Ok(tree.expect("a lookup succeeded without a tree"))
    }
}

// A `Tree` is a `git_tree`, never held by value; the owner is one pointer.
const _: () = assert!(
    size_of::<Tree<'static>>() == size_of::<ffi::git_tree>()
        && size_of::<TreeArc<'static>>() == size_of::<*mut ffi::git_tree>()
        && size_of::<Option<TreeArc<'static>>>() == size_of::<*mut ffi::git_tree>()
);

impl<'a, 'r> TreeRef<'a, 'r> {
    /// The tree's id, of its repository's id type: the id it was looked up by, or the
    /// full id a prefix found. Borrowed for as long as the handle: libgit2 keeps it in
    /// the tree, which writes it once, when it is parsed.
    ///
    /// Wraps: `git_tree_id`
    ///
    /// # Panics
    ///
    /// Panics, with `a live tree has an id`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: the handle addresses a live tree for `'a`; libgit2 only reads it, and
        // returns the address of the id cached in its object (`&obj->cached.oid`).
        let id = unsafe { ffi::git_tree_id(self.as_ptr()) };
        // SAFETY: the id is part of the tree, so it is non-NULL and lives as long as the
        // tree, past `'a`; nothing writes it once the tree is cached or returned.
        unsafe { OidRef::from_ptr(id.cast_mut()) }.expect("a live tree has an id")
    }

    /// The repository the tree was looked up in, borrowed for `'r`, as the shared handle
    /// it was looked up through: what looking its subtrees up reaches.
    ///
    /// The result is that handle again, so it lets nothing happen that the tree's own
    /// lookups do not: it outlives the tree, but not the repository's borrow.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::tree::Tree;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let owner = tree.as_ref().owner();
    /// drop(tree);
    /// drop(repo);
    /// let _ = owner.path();
    /// ```
    ///
    /// Wraps: `git_tree_owner`
    ///
    /// # Panics
    ///
    /// Panics, with `a tree always has a repository`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn owner(&self) -> RepositoryRef<'r> {
        // SAFETY: the handle addresses a live tree for `'a`, which libgit2 only reads.
        let repo = unsafe { ffi::git_tree_owner(self.as_ptr()) };
        // SAFETY: `object.repo`, stored uncounted and never cleared, is the repository
        // the tree was looked up in, through a `RepositoryRef<'r>` (`lookup*`, or an
        // object of it: `from_object`, a commit's tree), its `'r` only ever shrunk: it
        // is live and shared, not exclusively borrowed, for `'r`, on this thread, and the
        // result is the handle the tree was looked up through.
        unsafe { RepositoryRef::from_ptr(repo) }.expect("a tree always has a repository")
    }
}
