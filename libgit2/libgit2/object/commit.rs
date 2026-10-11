//! Safe wrappers over `include/git2/commit.h`.
//!
//! # Commits
//!
//! A [`Commit`] is a `git_commit`: a parsed commit object, whose first member is the
//! `git_object` every object starts with, of type `GIT_OBJECT_COMMIT`. It is opaque:
//! libgit2 publishes no field. It is counted and shared as every [`Object`](super::object::Object) is (see the
//! [`object` module](super::object#ownership)): the repository's object cache holds a
//! reference of its own and hands the same commit, one more reference counted on it, to
//! every lookup of its id; `git_commit_free` gives one reference back (it is
//! `git_object_free`) and `git_commit_dup` takes another, on the same pointer. So the
//! Rust owner is a [`CommitArc`], built by [`Commit::lookup`] and
//! [`Commit::lookup_prefix`], or derived from another commit by [`CommitRef::parent`]
//! and [`CommitRef::nth_gen_ancestor`], whose `Clone` is `git_commit_dup` and which
//! never proves itself the only reference: it lends the shared handle, [`CommitRef`],
//! and never [`CommitMut`]. [`CommitRef::tree`] looks a commit's tree up the same way,
//! as a [`TreeArc`].
//!
//! A commit keeps the repository it was looked up in (`object.repo`, uncounted), which
//! its owner, tree and parent lookups read and `git_repository_free` does not clear. So
//! a commit carries the lifetime `'r` of the repository borrow it was looked up
//! through, as an [`ObjectArc<'r>`](ObjectArc) does. Neither a commit nor its handles
//! are `Send` or `Sync`: what a commit reaches is its repository, which is neither, and
//! `git_commit_summary` and `git_commit_body` compute their result on first use and
//! store it in the shared commit, without synchronisation.
//!
//! A commit is an object, and libgit2's casts between the two never check the type:
//! [`CommitRef::as_object`] and [`Commit::into_object`] view or convert a commit as an
//! object, and [`CommitRef::from_object`] and [`Commit::from_object`] take an
//! object back as a commit only when it is of type `GIT_OBJECT_COMMIT`.
//!
//! # Commit arrays
//!
//! A [`CommitArray`] is a `git_commitarray`: a run of `count` commit pointers,
//! `commits`, in one buffer of libgit2's allocator, each pointer holding one reference
//! to a `git_commit`. libgit2 never allocates the struct itself: callers hold it by
//! value, `git_repository_commit_parents` fills one (overwriting both fields without
//! releasing what they held), and `git_commitarray_dispose` releases every commit with
//! `git_commit_free`, frees the buffer with `git__free` and zeroes the struct, leaving
//! it to its holder. Rust holds it the same way, as a [`CommitArrayVal`], which
//! disposes it on drop.
//!
//! Every handle upholds one invariant, which the safe API keeps and
//! [`CommitArray::from_raw`] or a crate-internal `from_ptr` over a C-owned struct must
//! vouch for: `commits` is NULL, or a live buffer of libgit2's allocator holding at
//! least `count` pointers, each to a live `git_commit` of which the array owns one
//! reference, looked up in a repository that outlives the array's `'r`. libgit2 leaves
//! `commits` NULL with `count == 0` for an empty result (an unborn `HEAD`, or a
//! failure). A NULL `commits` reads as the empty array whatever `count` says, and is
//! never handed to C.
//!
//! An empty array never reaches libgit2: nothing allocated it, so nothing has
//! initialised the library on its behalf, and even `git__free(NULL)` reads the
//! allocator a concurrent first `git_libgit2_init` writes. Every other array's buffer
//! came from libgit2's allocator, whose allocation initialised the library first.
//!
//! The array lends each commit as a [`CommitRef`] borrowed from it
//! ([`CommitArrayRef::get`], [`CommitArrayRef::iter`]), which
//! [`to_commit`](CommitRef::to_commit) turns into an owner of its own.
//! [`CommitArrayMut::set_commits`] moves owned commits in, in a fresh buffer of
//! libgit2's allocator ([`CommitArray::from_commits`] builds an array of them), and
//! [`CommitArrayMut::take`] moves them out. The array asserts neither `Send` nor
//! `Sync`, as its commits do not.
//!
//! An array may be dropped after the repository its commits came from:
//! `git_commit_free` touches only the commit. The commits may not be *used* past it,
//! since each keeps that repository: so the array carries the commits' `'r`, as a
//! [`CommitArc<'r>`](CommitArc) does, and lends them for no longer.
//!
//! # Extra headers
//!
//! A [`Header`] is a `git_commit_header`: a field name and a value that
//! `git_commit_create_ext` (and `git_commit_create_from_stage`, through the same
//! options) writes into the commit it creates, after `committer` and any `encoding`, as
//! `<field> <value>\n`, each newline of the value continued by a space. libgit2 never
//! allocates, keeps or frees one: callers build an array of them for the options'
//! `extra_headers`, which the call reads and forgets. So Rust holds a header by value,
//! borrowing both strings for `'h`, and a `[Header<'h>]` is the C array, the type being
//! `#[repr(transparent)]` over the struct.
//!
//! Neither string is ever NULL: libgit2 refuses a NULL one (`GIT_ERROR_INVALID`, from
//! `format_header_field`'s argument checks), so a header has no NULL state and no
//! `Default`. libgit2 writes the field as given, without checking it for a space or a
//! newline, either of which yields a commit git parses differently; a header keeps any
//! string, as the C struct does.
//!
//! # The commit builder
//!
//! A [`CommitBuilder`] is a `git_commitbuilder`: the commit a `git_commit_create_*`
//! routine is in the middle of writing, which it lends to the caller's signing callback
//! (`git_commit_signature_cb`) so that `git_commitbuilder_add_header` can add the
//! signature header to it. It is opaque: libgit2 publishes no field. libgit2 never
//! allocates, frees or copies one: the only builder is a local of the routine writing
//! the commit (`git_commit__create_internal`), pointing at that routine's commit
//! buffer, and lives only while the callback runs. So Rust never owns one, and has no
//! constructor, policy or owner for it: it is only ever reached through a
//! [`CommitBuilderMut`] the callback's wrapper builds from the pointer libgit2 passes,
//! for the call alone, and exclusively, since adding a header rewrites the buffer.
//! Neither handle is `Send` or `Sync`: the buffer belongs to the thread writing the
//! commit, which runs the callback.
//!
//! # Creation options
//!
//! A [`CreateExtOptions`] is a `git_commit_create_ext_options`: what
//! `git_commit_create_ext` does besides writing the commit — the reference to move to
//! it, the message's encoding, the [extra headers](#extra-headers), and the signing
//! callback with its payload. libgit2 never allocates one it hands out, nor keeps or
//! frees one it is given: the caller builds the struct (on the stack, in C) with
//! `git_commit_create_ext_options_init`, and the call reads it, and everything it
//! points at, through a `const` pointer, for the call only. libgit2's other creation
//! routines (`git_commit_create`, `_v`, `_from_ids`, `_from_callback`, `git_commit_amend`)
//! build one of their own the same way. So nothing it points at is owned: Rust holds
//! the struct by value, borrowing the strings and the headers for `'o`.
//!
//! A [`CreateOptions`] is a `git_commit_create_options`: the options of the routines
//! that commit the index or a tree on `HEAD` (`git_commit_create_from_stage`,
//! `git_commit_create_from_tree`, `git_commit_amend_from_stage`,
//! `git_commit_amend_from_tree`) — whether a commit that changes nothing is written,
//! the author and the committer (the repository's default signature when unset), and
//! the encoding, the extra headers and the signing callback, which those routines copy
//! into a `git_commit_create_ext_options` of their own. It is held the same way: the
//! caller builds it with `git_commit_create_options_init` and the routines read it,
//! copying it or not, for the call only, so Rust holds it by value, borrowing the
//! signatures, the strings and the headers for `'o`. `allow_empty_commit` is a one-bit
//! bitfield, read and written through its storage byte.
//!
//! The signing callback is a [`SignatureCallback`], which
//! [`CreateExtOptionsMut::set_signer`] and [`CreateOptionsMut::set_signer`] store with
//! the callback itself as the payload.
//!
//! # Signing
//!
//! A [`SignatureCallback`] is the closure a `git_commit_signature_cb` runs: it is given
//! the builder, the repository and the contents of the commit being written, and signs
//! them by adding a header ([`CommitBuilderMut::add_header`]). Adding a header frees the
//! buffer the contents were lent in (libgit2 swaps a rewritten copy in), so the closure
//! is given a copy of the contents, which stays valid whatever it adds.
//!
//! A commit can also be signed outside libgit2: [`create_with_signature`] writes
//! contents built elsewhere (`git_commit_create_buffer`) with a signature added as a
//! header, and [`extract_signature`] takes a written commit's signature header back
//! out, with the contents it signs.

use core::any::Any;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use ffibox::{
    CArc, CBorrowedPtr, CCell, CDispose, CDrop, CRefClone, CSlice, CVal, CZeroable, define_ctype,
};

use crate::ffi;
use crate::libgit2::core::buffer::{Buf, BufVal};
use crate::libgit2::core::types::{SignatureRef, StreamError};
use crate::libgit2::object::object::{ObjectArc, ObjectRef, id_fits};
use crate::libgit2::object::tree::TreeArc;
use crate::libgit2::odb::sys::odb_backend::{OdbDataArrayFree, OdbDataVec};
use crate::libgit2::oid::oid::{Oid, OidRef};
use crate::libgit2::repository::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use crate::libgit2::util::sys::errors::set;

/// A parsed commit (`git_commit`): a commit object read from a repository's object
/// database.
///
/// libgit2 allocates, counts and frees it, so it is only ever reached through a
/// [`CommitArc`] and its handles, built by [`lookup`](Self::lookup) and
/// [`lookup_prefix`](Self::lookup_prefix), derived from another commit by
/// [`CommitRef::parent`] and [`CommitRef::nth_gen_ancestor`], converted from an object by
/// [`from_object`](Self::from_object), or lent by a [`CommitArray`]. It is
/// opaque: no field is published.
///
/// `'r` is the borrow of the repository the commit was looked up in: routines that
/// look further objects up through it reach that repository, so it lives no longer
/// than the borrow (see the [module docs](self#commits)). The lifetime only shrinks:
///
/// ```compile_fail
/// use libgit2::libgit2::object::commit::CommitArc;
///
/// fn widen<'r>(commit: CommitArc<'r>) -> CommitArc<'static> {
///     commit
/// }
/// ```
///
/// and a commit stays on the thread that looked it up:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::commit::CommitArc;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(commit: CommitArc<'static>) {
///     send(commit)
/// }
/// ```
///
/// Wraps: git_commit
/// Handwritten
#[repr(transparent)]
pub struct Commit<'r>(ffi::git_commit, PhantomData<(*const (), &'r ())>);

/// Shared borrow of a [`Commit`]. `Copy`, like `&T`.
///
/// It stays on the thread of the commit's repository, as the owner does:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::commit::CommitRef;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(handle: CommitRef<'static, 'static>) {
///     send(handle)
/// }
/// ```
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct CommitRef<'a, 'r>(CBorrowedPtr<'a, Commit<'r>>);

/// Exclusive borrow of a [`Commit`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// No owner hands one out: every commit is shared with its repository's cache.
///
/// Handwritten
#[repr(transparent)]
pub struct CommitMut<'a, 'r>(
    CommitRef<'a, 'r>,
    // Invariant in the commit, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Commit<'r>>,
);

// SAFETY: `Commit` is `#[repr(transparent)]` over `git_commit` with a zero-sized
// marker; both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Commit<'r>>`
// (`CommitMut` through `CommitRef`) with zero-sized markers and no `Drop`, hand out no
// reference to the C object, and `CommitRef` has no write operation. `CommitMut` is
// invariant in the commit through its `PhantomData<&'a mut Commit<'r>>`.
unsafe impl<'r> CCell for Commit<'r> {
    type C = ffi::git_commit;
    type Ref<'a>
        = CommitRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = CommitMut<'a, 'r>
    where
        Self: 'a;
}

impl<'a, 'r> CommitRef<'a, 'r> {
    /// Borrows a raw pointer; `None` if null.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live commit looked up in a repository that
    /// outlives `'r`, which stays live during `'a` (a reference to it is held for at
    /// least that long).
    pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_commit) -> Option<Self> {
        // SAFETY: a non-null pointer the caller vouches for, per this function's
        // contract; `Commit` is transparent over `git_commit`.
        NonNull::new(ptr.cast_mut()).map(|p| CommitRef(unsafe { CBorrowedPtr::new(p.cast()) }))
    }

    /// Read-only pointer to the C object.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_commit {
        self.0.as_non_null().as_ptr().cast()
    }

    /// Another counted reference to the commit, as `git_commit_dup` takes one: the
    /// same commit, now also kept alive by the result, which may outlive this borrow
    /// but not the repository.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::object::commit::Commit;
    /// # use libgit2::libgit2::oid::oid::{Oid, OidType};
    /// # use libgit2::libgit2::repository::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let copy = commit.as_ref().to_commit();
    /// drop(commit);
    /// drop(repo);
    /// drop(copy);
    /// ```
    pub fn to_commit(&self) -> CommitArc<'r> {
        let commit = self.0.as_non_null();
        // SAFETY: the handle addresses a live commit for `'a`, whose reference its
        // holder keeps meanwhile; one more is counted on it here.
        if !unsafe { CommitFree.c_up_ref(commit) } {
            unreachable!("git_commit_dup cannot fail");
        }
        // SAFETY: the reference just counted, given to the result, which
        // `git_commit_free` releases; the commit's repository outlives `'r`, which the
        // result carries.
        unsafe { CommitArc::from_raw(commit.as_ptr()) }.expect("a handle is never NULL")
    }

    /// The commit, as the object it is: the same pointer, borrowed for as long.
    pub fn as_object(&self) -> ObjectRef<'a, 'r> {
        // SAFETY: a `git_commit` starts with its `git_object`, so the handle's pointer
        // addresses a live object of the same repository for `'a`, as this handle's
        // does; nothing is written through it while it is shared.
        let object = unsafe { ObjectRef::from_ptr(self.as_ptr().cast()) };
        object.expect("a handle is never NULL")
    }

    /// The object, as a commit, when it is one (of type `GIT_OBJECT_COMMIT`); `None`
    /// for a tree, a blob or a tag.
    pub fn from_object(object: ObjectRef<'a, 'r>) -> Option<Self> {
        if !is_commit(object) {
            return None;
        }
        // SAFETY: an object of type `GIT_OBJECT_COMMIT` is a `git_commit` (libgit2
        // allocates each object by its type), live for `'a` and of a repository that
        // outlives `'r`, as `object` vouches.
        unsafe { Self::from_ptr(object.as_ptr().cast()) }
    }

    /// The commit's `n`th parent, counting from 0 in the order the commit lists them
    /// (the first parent is the branch a merge was made on), looked up in the commit's
    /// repository by the id the commit records: one more reference of the repository's
    /// cache, or a fresh parse.
    ///
    /// Fails with [`ErrorCode::NotFound`] (class `GIT_ERROR_INVALID`, "parent `n` does
    /// not exist") when the commit has `n` parents or fewer; and as
    /// [`Commit::lookup`] does for the parent's id: [`ErrorCode::NotFound`] when the
    /// object database lacks the parent or it is not a commit, libgit2's error when it
    /// cannot be read or parsed.
    ///
    /// The parent lives no longer than the repository's borrow `'r`, as this commit
    /// does, but may outlive this handle and the commit's owner.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::object::commit::Commit;
    /// # use libgit2::libgit2::oid::oid::{Oid, OidType};
    /// # use libgit2::libgit2::repository::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let parent = commit.as_ref().parent(0).unwrap();
    /// drop(commit);
    /// drop(repo);
    /// drop(parent);
    /// ```
    ///
    /// Wraps: git_commit_parent
    pub fn parent(&self, n: u32) -> Result<CommitArc<'r>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; the handle addresses a
        // live commit for `'a`, whose recorded parent ids are of its repository's type
        // and whose repository, borrowed for `'r`, is live and used on this thread only,
        // as `RepositoryRef` allows: libgit2 looks the parent up through it, filling its
        // cache.
        let rc = unsafe { ffi::git_commit_parent(&mut out, self.as_ptr(), n) };
        if rc < 0 {
            return failed(rc);
        }
        // SAFETY: on success `out` holds one counted reference to a live commit of the
        // same repository, borrowed for `'r`, given to the result and released by
        // `git_commit_free`.
        let parent = unsafe { CommitArc::from_c(out) };
        Ok(parent.expect("a parent lookup succeeded without a commit"))
    }

    /// The commit's ancestor `n` generations back, following first parents only: this
    /// very commit for 0 (one more reference, as [`to_commit`](Self::to_commit) takes),
    /// its first parent for 1, that commit's first parent for 2, and so on, each looked
    /// up in the commit's repository as [`parent`](Self::parent) looks it up.
    ///
    /// Fails with [`ErrorCode::NotFound`] (class `GIT_ERROR_INVALID`, "parent 0 does
    /// not exist") when the first-parent chain reaches a root commit before `n`
    /// generations, and as [`parent`](Self::parent) does when an ancestor cannot be
    /// looked up. Every intermediate commit is released.
    ///
    /// The ancestor lives no longer than the repository's borrow `'r`, as this commit
    /// does.
    ///
    /// Wraps: git_commit_nth_gen_ancestor
    pub fn nth_gen_ancestor(&self, n: u32) -> Result<CommitArc<'r>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: as in `parent`. libgit2 also takes, and later releases or hands out,
        // one more reference on this commit (`git_commit_dup`, an atomic increment
        // through a cast of the `const` pointer), which leaves the holder's own
        // reference untouched.
        let rc = unsafe { ffi::git_commit_nth_gen_ancestor(&mut out, self.as_ptr(), n) };
        if rc < 0 {
            return failed(rc);
        }
        // SAFETY: on success `out` holds one counted reference to a live commit of the
        // same repository (this one for `n == 0`), borrowed for `'r`, given to the
        // result and released by `git_commit_free`.
        let ancestor = unsafe { CommitArc::from_c(out) };
        Ok(ancestor.expect("an ancestor lookup succeeded without a commit"))
    }

    /// The commit's tree, looked up in the commit's repository by the id the commit
    /// records: one more reference of the repository's cache, or a fresh parse.
    ///
    /// Fails as [`Tree::lookup`](crate::libgit2::object::tree::Tree::lookup) does for the
    /// tree's id: with [`ErrorCode::NotFound`]
    /// when the object database lacks the tree or it is not a tree, and with
    /// libgit2's error when it cannot be read or parsed.
    ///
    /// The tree lives no longer than the repository's borrow `'r`, as this commit
    /// does, but may outlive this handle and the commit's owner.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::object::commit::Commit;
    /// # use libgit2::libgit2::oid::oid::{Oid, OidType};
    /// # use libgit2::libgit2::repository::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let tree = commit.as_ref().tree().unwrap();
    /// drop(commit);
    /// drop(repo);
    /// drop(tree);
    /// ```
    ///
    /// Wraps: git_commit_tree
    pub fn tree(&self) -> Result<TreeArc<'r>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: as in `parent`: `out` is a writable slot, the handle addresses a live
        // commit whose recorded tree id is of its repository's type, and whose
        // repository is live for `'r` and used on this thread only, where libgit2
        // looks the tree up.
        let rc = unsafe { ffi::git_commit_tree(&mut out, self.as_ptr()) };
        if rc < 0 {
            return failed(rc);
        }
        // SAFETY: on success `out` holds one counted reference to a live tree of the
        // same repository, borrowed for `'r`, given to the result and released by
        // `git_tree_free`.
        let tree = unsafe { TreeArc::from_c(out) };
        Ok(tree.expect("a tree lookup succeeded without a tree"))
    }
}

impl<'a, 'r> CommitMut<'a, 'r> {
    /// Borrows a raw pointer exclusively; `None` if null.
    ///
    /// # Safety
    ///
    /// As [`CommitRef::from_ptr`], plus: nothing else reaches the commit while the
    /// result lives, its repository's cache included.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_commit) -> Option<Self> {
        // SAFETY: the caller upholds `CommitRef::from_ptr`'s contract and exclusivity.
        unsafe { CommitRef::from_ptr(ptr) }.map(|r| CommitMut(r, PhantomData))
    }

    /// Writable pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_commit {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> CommitRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> CommitMut<'_, 'r> {
        CommitMut(self.0, PhantomData)
    }
}

impl fmt::Debug for CommitRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Commit").finish_non_exhaustive()
    }
}

impl fmt::Debug for CommitMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Whether `object` is a commit: of type `GIT_OBJECT_COMMIT`, and so a `git_commit`.
fn is_commit(object: ObjectRef<'_, '_>) -> bool {
    // SAFETY: the handle addresses a live object; the call reads its type only.
    unsafe { ffi::git_object_type(object.as_ptr()) == ffi::GIT_OBJECT_COMMIT }
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
/// Wraps: git_commit_free
/// Wraps: git_commit_dup
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
        let rc = unsafe { ffi::git_commit_dup(&mut out, ptr.as_ptr().cast()) };
        debug_assert_eq!(out, ptr.as_ptr().cast());
        rc == 0
    }
}

/// One counted reference to a [`Commit`], given back by `git_commit_free` on drop;
/// `clone` takes another through `git_commit_dup`, on the same commit.
pub type CommitArc<'r> = CArc<Commit<'r>, CommitFree>;

impl<'r> Commit<'r> {
    /// The commit `id` names in `repo`.
    ///
    /// The repository's cache answers when it holds the commit, with the commit it
    /// holds; otherwise libgit2 reads it from the object database, parses it and,
    /// under the cache's size limit, caches it, so that every later lookup of the id
    /// yields the same commit while the cache keeps it.
    ///
    /// Fails with [`ErrorCode::NotFound`]
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
    /// The commit reads the repository it was looked up in, so it lives no longer than
    /// this borrow of it:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::object::commit::Commit;
    /// # use libgit2::libgit2::oid::oid::{Oid, OidType};
    /// # use libgit2::libgit2::repository::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// drop(repo);
    /// drop(commit);
    /// ```
    ///
    /// Wraps: git_commit_lookup
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
        let rc = unsafe { ffi::git_commit_lookup(&mut out, repo.as_ptr().cast_mut(), id.as_ptr()) };
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
    /// Fails with [`ErrorCode::Ambiguous`]
    /// when `len` is under 4 (`GIT_ERROR_OBJECT`, "ambiguous lookup - OID prefix is too
    /// short") or the prefix names several objects, with
    /// [`ErrorCode::NotFound`] when
    /// it names none or one that is not a commit, and as [`lookup`](Self::lookup)
    /// otherwise; an `id` of another type than `repo`'s is refused as it is, whatever
    /// `len`.
    ///
    /// Wraps: git_commit_lookup_prefix
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
            ffi::git_commit_lookup_prefix(&mut out, repo.as_ptr().cast_mut(), id.as_ptr(), len)
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: as in `lookup`: one counted reference to a live commit of `repo`.
        let commit = unsafe { CommitArc::from_c(out) };
        Ok(commit.expect("a lookup succeeded without a commit"))
    }

    /// The object, as a commit, when it is one (of type `GIT_OBJECT_COMMIT`): the same
    /// counted reference, moved. Otherwise the object, unchanged.
    pub fn from_object(object: ObjectArc<'r>) -> Result<CommitArc<'r>, ObjectArc<'r>> {
        if !is_commit(object.as_ref()) {
            return Err(object);
        }
        let raw = object.into_c();
        // SAFETY: `raw` is the counted reference the object owner gave up, to an object
        // of type `GIT_OBJECT_COMMIT`, so a `git_commit`, of a repository borrowed for
        // `'r`; `git_commit_free` releases it as `git_object_free` would.
        let commit = unsafe { CommitArc::from_c(raw.cast()) };
        Ok(commit.expect("an owner is never NULL"))
    }

    /// The commit, as the object it is: the same counted reference, moved.
    pub fn into_object(commit: CommitArc<'r>) -> ObjectArc<'r> {
        let raw = commit.into_c();
        // SAFETY: `raw` is the counted reference the commit owner gave up; a
        // `git_commit` starts with its `git_object`, of the same repository, borrowed
        // for `'r`, and `git_object_free` releases it as `git_commit_free` would.
        let object = unsafe { ObjectArc::from_c(raw.cast()) };
        object.expect("an owner is never NULL")
    }
}

// A `Commit` is a `git_commit`, never held by value; the owner is one pointer.
const _: () = assert!(
    size_of::<Commit<'static>>() == size_of::<ffi::git_commit>()
        && size_of::<CommitArc<'static>>() == size_of::<*mut ffi::git_commit>()
        && size_of::<Option<CommitArc<'static>>>() == size_of::<*mut ffi::git_commit>()
);

/// An array of commits (`git_commitarray`): `count` commit references in one buffer
/// of libgit2's allocator, which the struct owns, as `git_repository_commit_parents`
/// hands one out.
///
/// Held by value inside a [`CommitArrayVal`], which [`new`](Self::new) builds empty
/// for libgit2 to fill, [`from_commits`](Self::from_commits) builds from owned commits
/// and [`from_raw`](Self::from_raw) adopts from C; read it through the owner's
/// [`as_ref`](CVal::as_ref) handle. Not `Clone`: it owns its commits, and libgit2 has
/// no copy routine.
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::object::commit::{CommitArray, CommitArrayVal};
///
/// let array: CommitArrayVal<'static> = CommitArray::new();
/// let copy: CommitArrayVal<'static> = array.clone();
/// ```
///
/// Nor `Send`: the commits it owns are not.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::commit::{CommitArray, CommitArrayVal};
///
/// let array: CommitArrayVal<'static> = CommitArray::new();
/// std::thread::spawn(move || drop(array));
/// ```
///
/// `'r` is the borrow of the repository its commits were looked up in, which the
/// commits it lends carry; it only shrinks, and the exclusive handle, which moves
/// commits in, is invariant in it:
///
/// ```compile_fail
/// use libgit2::libgit2::object::commit::CommitArrayMut;
///
/// fn shrink<'a, 'r>(m: CommitArrayMut<'a, 'static>) -> CommitArrayMut<'a, 'r> {
///     m
/// }
/// ```
///
/// Wraps: git_commitarray
/// Handwritten
#[repr(transparent)]
pub struct CommitArray<'r>(ffi::git_commitarray, PhantomData<(*const (), &'r ())>);

/// Shared borrow of a [`CommitArray`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct CommitArrayRef<'a, 'r>(CBorrowedPtr<'a, CommitArray<'r>>);

/// Exclusive borrow of a [`CommitArray`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct CommitArrayMut<'a, 'r>(
    CommitArrayRef<'a, 'r>,
    // Invariant in the array, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut CommitArray<'r>>,
);

// SAFETY: `CommitArray` is `#[repr(transparent)]` over `git_commitarray` with a
// zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, CommitArray<'r>>` (`CommitArrayMut` through `CommitArrayRef`) with
// zero-sized markers and no `Drop`, hand out no reference to the array, and
// `CommitArrayRef` has no write operation. `CommitArrayMut` is invariant in the array
// through its `PhantomData<&'a mut CommitArray<'r>>`.
unsafe impl<'r> CCell for CommitArray<'r> {
    type C = ffi::git_commitarray;
    type Ref<'a>
        = CommitArrayRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = CommitArrayMut<'a, 'r>
    where
        Self: 'a;
}

/// Releases an array's commits through `git_commitarray_dispose`, which hands each to
/// `git_commit_free`, the buffer to `git__free`, and zeroes the struct, leaving the
/// struct itself, Rust's storage, alone.
///
/// An empty array (`commits` NULL) is not passed to C: it owns nothing, and the call
/// would be `git__free(NULL)`, which reads `git__allocator` without synchronisation
/// against the first `git_libgit2_init` another thread may be running; it would also
/// read a NULL buffer for a non-zero `count`. Nothing orders an empty array after that
/// initialisation, since nothing allocated for it.
///
/// Wraps: git_commitarray_dispose
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct CommitArrayDispose;

// SAFETY: `git_commitarray_dispose` releases each of the first `count` commits once,
// frees `commits` once and never the struct, which it zeroes. It accepts every state
// the safe API reaches: a buffer libgit2 filled (whose allocation initialised the
// library, so the allocator it reads is settled), adopted through `from_raw` under that
// function's contract, or one `set_commits` allocated through libgit2's allocator;
// `clear` and `take` leave the empty array, as `new` builds it, which owns nothing and
// is released without C.
unsafe impl<'r> CDispose<CommitArray<'r>> for CommitArrayDispose {
    #[inline]
    unsafe fn c_dispose(&self, ptr: NonNull<CommitArray<'r>>) {
        let raw: *mut ffi::git_commitarray = ptr.as_ptr().cast();
        // SAFETY: per `c_dispose`'s contract `raw` addresses a live `git_commitarray`.
        let commits = unsafe { addr_of!((*raw).commits).read() };
        if commits.is_null() {
            return;
        }
        // SAFETY: `raw` is live and, per the handles' invariant, owns `commits`, a
        // buffer from libgit2's allocator of at least `count` commit references; the
        // caller gives the value up, so each is released once.
        unsafe { ffi::git_commitarray_dispose(raw) }
    }
}

/// An owned [`CommitArray`], held inline; its commits and buffer are released by
/// `git_commitarray_dispose` on drop.
pub type CommitArrayVal<'r> = CVal<CommitArray<'r>, CommitArrayDispose>;

impl<'r> CommitArray<'r> {
    /// An empty array: no commits, nothing allocated, as `git_commitarray array = {0}`.
    /// The storage libgit2 fills a result into.
    pub fn new() -> CommitArrayVal<'r> {
        CVal::new(Self(
            ffi::git_commitarray {
                commits: ptr::null(),
                count: 0,
            },
            PhantomData,
        ))
    }

    /// An array of `commits`, in order, moved into a fresh buffer of libgit2's
    /// allocator; an empty `commits` allocates nothing. Released, commits and buffer,
    /// by `git_commitarray_dispose`, as an array libgit2 filled is.
    ///
    /// Fails, handing `commits` back, when the buffer cannot be allocated, or libgit2
    /// cannot be initialised (libgit2's error is then set on the calling thread).
    pub fn from_commits(
        commits: Vec<CommitArc<'r>>,
    ) -> Result<CommitArrayVal<'r>, Vec<CommitArc<'r>>> {
        let mut array = Self::new();
        array.as_mut().set_commits(commits)?;
        Ok(array)
    }

    /// Adopts a `git_commitarray` C code filled, as `git_repository_commit_parents`
    /// hands one out: its commits and buffer become the result's, released by
    /// `git_commitarray_dispose` on drop.
    ///
    /// An array whose `commits` is NULL owns nothing and reads as empty, whatever
    /// `count` holds.
    ///
    /// # Safety
    ///
    /// When `raw.commits` is not NULL, it must be a live allocation from libgit2's
    /// current allocator holding at least `raw.count` pointers, each to a live
    /// `git_commit` (an object of type `GIT_OBJECT_COMMIT`, never a tag or another
    /// object) of which `raw` owns one reference (one `git_commit_free` releases it),
    /// looked up in a repository that outlives `'r`, all of which the caller gives up:
    /// nothing else may free the buffer or release those references, nor write the
    /// buffer while the result lives. With `raw.commits` NULL there is no requirement.
    pub unsafe fn from_raw(raw: ffi::git_commitarray) -> CommitArrayVal<'r> {
        CVal::new(Self(raw, PhantomData))
    }

    /// The raw `git_commitarray`, ownership included: C code releases it with
    /// `git_commitarray_dispose`.
    pub fn into_raw(self) -> ffi::git_commitarray {
        self.0
    }

    /// Shared handle to the array — the getters.
    pub fn as_ref(&self) -> CommitArrayRef<'_, 'r> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        CommitArrayRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the array — the setters.
    pub fn as_mut(&mut self) -> CommitArrayMut<'_, 'r> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        CommitArrayMut(CommitArrayRef(ptr), PhantomData)
    }
}

impl<'a, 'r> CommitArrayRef<'a, 'r> {
    /// Borrows the array at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must address a live `git_commitarray` (or be NULL) that outlives `'a`,
    /// that nothing writes while the handle lives, and that keeps the handles'
    /// invariant (see the [module docs](self#commit-arrays)) for `'r`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_commitarray) -> Option<Self> {
        let ptr = NonNull::new(ptr.cast::<CommitArray<'r>>())?;
        // SAFETY: layout-preserving cast per `#[repr(transparent)]`; the caller upholds
        // liveness, the invariant and the absence of writers.
        Some(Self(unsafe { CBorrowedPtr::new(ptr) }))
    }

    /// Read-only pointer to the C struct.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_commitarray {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The number of commits; 0 without a buffer, whatever `count` says, as
    /// [`get`](Self::get) reads it.
    ///
    /// Field: git_commitarray.count
    pub fn len(&self) -> usize {
        let array = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to a live
        // `git_commitarray`; any `size_t` is a valid `usize`.
        let (commits, count) = unsafe {
            (
                addr_of!((*array).commits).read(),
                addr_of!((*array).count).read(),
            )
        };
        if commits.is_null() { 0 } else { count }
    }

    /// Whether the array holds no commits.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The commit at `index`, borrowed from the array for as long as the array is;
    /// `None` past the [`len`](Self::len) commits. The array owns a reference to it, so
    /// it lives at least that long; [`to_commit`](CommitRef::to_commit) takes one of
    /// its own.
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::object::commit::{CommitArray, CommitArrayVal};
    ///
    /// let array: CommitArrayVal<'static> = CommitArray::new();
    /// let first = array.as_ref().get(0);
    /// drop(array);
    /// let _ = first;
    /// ```
    ///
    /// Field: git_commitarray.commits
    pub fn get(&self, index: usize) -> Option<CommitRef<'a, 'r>> {
        if index >= self.len() {
            return None;
        }
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_commitarray`.
        let commits = unsafe { addr_of!((*self.as_ptr()).commits).read() };
        // SAFETY: `len` is non-zero, so `commits` is not NULL and, per the handles'
        // invariant, addresses at least `count > index` initialised commit pointers,
        // which nothing writes while the array is shared.
        let commit = unsafe { commits.add(index).read() };
        // SAFETY: per the handles' invariant, a live commit of which the array owns one
        // reference, so it lives as long as the array does, past `'a`, looked up in a
        // repository that outlives `'r`.
        unsafe { CommitRef::from_ptr(commit) }
    }

    /// The commits, in order, each borrowed from the array as [`get`](Self::get) lends
    /// it.
    ///
    /// Field: git_commitarray.commits
    pub fn iter(
        &self,
    ) -> impl ExactSizeIterator<Item = CommitRef<'a, 'r>> + DoubleEndedIterator + use<'a, 'r> {
        let array = *self;
        (0..self.len()).map(move |i| array.get(i).expect("an index under len"))
    }
}

impl fmt::Debug for CommitArrayRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl fmt::Debug for CommitArrayMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'r> CommitArrayMut<'a, 'r> {
    /// Borrows the array at `ptr` exclusively; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// As [`CommitArrayRef::from_ptr`], plus: nothing else reads or writes the array
    /// while the result lives, and whatever the setters store may stay there for as
    /// long as the array's owner expects commits of `'r`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_commitarray) -> Option<Self> {
        // SAFETY: the caller upholds `CommitArrayRef::from_ptr`'s contract, and
        // excludes every other access for `'a`.
        let shared = unsafe { CommitArrayRef::from_ptr(ptr) }?;
        Some(Self(shared, PhantomData))
    }

    /// Writable pointer to the C struct, for the setters and for libgit2 to fill.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_commitarray {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> CommitArrayRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> CommitArrayMut<'_, 'r> {
        CommitArrayMut(self.0, PhantomData)
    }

    /// Releases the commits and the buffer, leaving the empty array: what
    /// `git_commitarray_dispose` leaves, and the storage libgit2 fills again without
    /// leaking what it overwrites.
    ///
    /// Field: git_commitarray.commits
    pub fn clear(&mut self) {
        let array = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer is non-NULL and addresses a live
        // `git_commitarray` (`CommitArray` is transparent over it) that, per the
        // handles' invariant, owns what it holds, released once here; it is reset to
        // the empty array below, before anything reads it again.
        unsafe {
            CommitArrayDispose.c_dispose(NonNull::new_unchecked(array.cast::<CommitArray<'r>>()))
        };
        // SAFETY: writes the empty array through the exclusive handle's pointer to a
        // live `git_commitarray`; C has zeroed it already unless `commits` was NULL, in
        // which case only `count` may differ.
        unsafe {
            addr_of_mut!((*array).commits).write(ptr::null());
            addr_of_mut!((*array).count).write(0);
        }
    }

    /// Replaces the commits with `commits`, in order, moved into a fresh buffer of
    /// libgit2's allocator, releasing the ones held before as [`clear`](Self::clear)
    /// does; an empty `commits` leaves the empty array and allocates nothing.
    ///
    /// Fails, handing `commits` back and leaving the array as it was, when the buffer
    /// cannot be allocated, or libgit2 cannot be initialised (libgit2's error is then
    /// set on the calling thread).
    ///
    /// The commits must be of the array's own repository borrow `'r`:
    ///
    /// ```compile_fail,E0521
    /// use libgit2::libgit2::object::commit::{CommitArc, CommitArrayMut};
    ///
    /// fn store<'r>(mut array: CommitArrayMut<'_, 'static>, commit: CommitArc<'r>) {
    ///     let _ = array.set_commits(vec![commit]);
    /// }
    /// ```
    ///
    /// Field: git_commitarray.commits
    /// Field: git_commitarray.count
    pub fn set_commits(&mut self, commits: Vec<CommitArc<'r>>) -> Result<(), Vec<CommitArc<'r>>> {
        let Some(mut buffer) = OdbDataArrayFree::new_uninit::<*mut ffi::git_commit>(commits.len())
        else {
            return Err(commits);
        };
        for (slot, commit) in buffer.as_mut_slice().iter_mut().zip(commits) {
            slot.write(commit.into_c());
        }
        // SAFETY: `buffer` holds as many slots as `commits` had commits, and the loop
        // wrote each, the zip running to the end of both.
        let (buffer, count) = unsafe { buffer.assume_init() }.into_raw_parts();
        self.clear();
        let array = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to a live
        // `git_commitarray`, whose previous commits and buffer `clear` released.
        // `buffer` is NULL with `count == 0`, or `count` pointers in libgit2's
        // allocator, each a counted reference an owner of `'r` gave up to the array,
        // which keeps the handles' invariant.
        unsafe {
            addr_of_mut!((*array).commits).write(buffer.cast_const());
            addr_of_mut!((*array).count).write(count);
        }
        Ok(())
    }

    /// Takes the commits out, in order, leaving the empty array: what
    /// `git_commitarray_dispose` would release, owned. The buffer is released.
    ///
    /// Field: git_commitarray.commits
    pub fn take(&mut self) -> Vec<CommitArc<'r>> {
        let array = self.as_mut_ptr();
        // SAFETY: reads, then writes the empty array, through the exclusive handle's
        // pointer to a live `git_commitarray`.
        let (commits, count) = unsafe {
            let commits = addr_of!((*array).commits).read();
            let count = addr_of!((*array).count).read();
            addr_of_mut!((*array).commits).write(ptr::null());
            addr_of_mut!((*array).count).write(0);
            (commits, count)
        };
        if commits.is_null() {
            return Vec::new();
        }
        // SAFETY: per the handles' invariant `commits` is a live buffer of libgit2's
        // allocator holding at least `count` initialised commit pointers, owned by the
        // array and handed over whole, since the array no longer refers to it.
        // `OdbDataArrayFree` frees with `git__free`, which ignores the length, so spare
        // capacity beyond `count` is released with the buffer.
        let buffer = unsafe { OdbDataVec::from_raw_parts(commits.cast_mut(), count) };
        let buffer = buffer.expect("a non-NULL buffer");
        let owned = buffer
            .as_slice()
            .iter()
            // SAFETY: each pointer is a live commit of a repository that outlives `'r`,
            // whose one reference the array owned and now gives to the result, once:
            // the buffer is freed below without releasing it.
            .map(|&commit| unsafe { CommitArc::from_c(commit) }.expect("a commit is never NULL"))
            .collect();
        drop(buffer);
        owned
    }
}

// A `CommitArray` is a `git_commitarray`, so C reads and writes a value Rust holds in
// place, and the owner adds nothing to it.
const _: () = assert!(
    size_of::<CommitArray<'static>>() == size_of::<ffi::git_commitarray>()
        && align_of::<CommitArray<'static>>() == align_of::<ffi::git_commitarray>()
        && size_of::<CommitArrayVal<'static>>() == size_of::<ffi::git_commitarray>()
);

define_ctype!(
    /// A commit being written (`git_commitbuilder`), as libgit2 lends it to a commit
    /// signing callback: the commit's contents, to which a signature header can be added.
    ///
    /// libgit2 neither allocates nor frees one: the routine writing the commit keeps it
    /// on its stack while the callback runs, so it is never held by value, boxed or
    /// owned in Rust. It is reached only through the [`CommitBuilderMut`] the callback's
    /// wrapper lends for the call (see the [module docs](self#the-commit-builder)); there
    /// is no public way to make one.
    ///
    /// ```compile_fail,E0624
    /// use libgit2::libgit2::object::commit::CommitBuilderMut;
    ///
    /// let mut slot = 0u64;
    /// let _ = unsafe { CommitBuilderMut::from_ptr(core::ptr::from_mut(&mut slot).cast()) };
    /// ```
    ///
    /// The exclusive handle does not alias, as `&mut` does not:
    ///
    /// ```compile_fail,E0499
    /// use libgit2::libgit2::object::commit::CommitBuilderMut;
    ///
    /// fn alias(mut builder: CommitBuilderMut<'_>) {
    ///     let a = builder.as_mut();
    ///     let b = builder.as_mut();
    ///     drop((a, b));
    /// }
    /// ```
    ///
    /// and stays on the thread writing the commit:
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::object::commit::CommitBuilderMut;
    ///
    /// fn send<T: Send>(_: T) {}
    /// fn move_out(builder: CommitBuilderMut<'_>) {
    ///     send(builder)
    /// }
    /// ```
    ///
    /// Wraps: git_commitbuilder
    CommitBuilder, CommitBuilderRef, CommitBuilderMut, ffi::git_commitbuilder
);

impl fmt::Debug for CommitBuilderRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommitBuilder").finish_non_exhaustive()
    }
}

impl fmt::Debug for CommitBuilderMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// A `CommitBuilder` is a `git_commitbuilder`, and each handle one pointer to it.
const _: () = assert!(
    size_of::<CommitBuilder>() == size_of::<ffi::git_commitbuilder>()
        && size_of::<CommitBuilderMut<'static>>() == size_of::<*mut ffi::git_commitbuilder>()
);

impl CommitBuilderMut<'_> {
    /// Adds the header `<field> <value>` to the commit being written: after its last
    /// header, before the blank line that opens its message, each newline of `value`
    /// continued by a space, as a [`Header`] is written. A signing callback adds the
    /// signature of the contents it was given this way, under `gpgsig` for git's own
    /// signatures; it may add several headers, each after the previous one.
    ///
    /// libgit2 rewrites the commit into a new buffer with the header and frees the old
    /// one. It writes `field` as given: one holding a space or a newline yields a commit
    /// git parses differently, and one holding a blank line moves where the next header
    /// goes.
    ///
    /// Fails with `GIT_ERROR_INVALID`, "malformed commit contents", when the contents
    /// have no blank line (a commit libgit2 writes always has one), and with libgit2's
    /// out-of-memory error.
    ///
    /// Wraps: git_commitbuilder_add_header
    pub fn add_header(&mut self, field: &CStr, value: &CStr) -> Result<(), ErrorBuf> {
        let builder = self.as_mut_ptr();
        // SAFETY: the exclusive handle addresses the live builder libgit2 lent the
        // signing callback for its call, which nothing else uses meanwhile; libgit2
        // replaces the commit buffer it points at (only the contents the callback was
        // given, which the wrapper copied first, pointed into the old one). `field` and
        // `value` are NUL-terminated, read and copied during the call. libgit2 is
        // initialised: it is writing the commit.
        let rc =
            unsafe { ffi::git_commitbuilder_add_header(builder, field.as_ptr(), value.as_ptr()) };
        if rc < 0 { Err(last()) } else { Ok(()) }
    }
}

/// The closure a [`SignatureCallback`] calls: the builder, the repository and a copy of
/// the contents to sign, each lent for the call.
type SignFn<'f> =
    dyn FnMut(CommitBuilderMut<'_>, RepositoryRef<'_>, &CStr) -> Result<(), StreamError> + 'f;

/// A commit signing callback (`git_commit_signature_cb`): the routines writing a commit
/// with options (`git_commit_create_ext`, `git_commit_create_from_stage`) call it once,
/// on the calling thread, with the commit's builder, its repository and its contents,
/// before writing it, so that it can add a signature header.
///
/// Built over a closure with [`new`](Self::new). Its `extern "C"` trampoline lends the
/// closure a [`CommitBuilderMut`] and a [`RepositoryRef`] valid for the call only, and a
/// copy of the contents: adding a header frees the buffer libgit2 lends them in (see the
/// [module docs](self#signing)). `Ok` lets libgit2 write the commit, signed or not; an
/// error stops it before anything is written, and the commit routine returns the
/// error's code with its class and message as the thread's last libgit2 error. libgit2
/// reads every negative code as a failure, `GIT_PASSTHROUGH` included, whatever its
/// header says about that code. A panic in the closure does not cross into C: the
/// trampoline stops the commit with `GIT_EUSER` and the panic resumes once control is
/// back in Rust, in the wrapper that wrote the commit, or from
/// [`resume_panic`](Self::resume_panic) for a callback lent to options with
/// [`CreateExtOptionsMut::set_signer`].
///
/// The handles cannot outlive the call:
///
/// ```compile_fail,E0521
/// use libgit2::libgit2::core::types::StreamError;
/// use libgit2::libgit2::object::commit::{CommitBuilderMut, SignatureCallback};
/// use libgit2::libgit2::repository::repository::RepositoryRef;
///
/// fn keep<'k>(kept: &mut Option<CommitBuilderMut<'k>>) {
///     let mut f = |builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &core::ffi::CStr| {
///         *kept = Some(builder);
///         Ok::<(), StreamError>(())
///     };
///     let _ = SignatureCallback::new(&mut f);
/// }
/// ```
///
/// It borrows its closure exclusively and stays on its thread: libgit2 calls it
/// synchronously, and the closure need not be `Send`.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::core::types::StreamError;
/// use libgit2::libgit2::object::commit::{CommitBuilderMut, SignatureCallback};
/// use libgit2::libgit2::repository::repository::RepositoryRef;
///
/// fn send<T: Send>(_: T) {}
/// let mut f = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &core::ffi::CStr| Ok::<(), StreamError>(());
/// send(SignatureCallback::new(&mut f));
/// ```
///
/// Wraps: git_commit_signature_cb
/// Handwritten
pub struct SignatureCallback<'f> {
    f: &'f mut SignFn<'f>,
    /// A panic the closure raised inside a C call, resumed once that call returns.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> SignatureCallback<'f> {
    /// The callback that calls `f` to sign each commit written with it.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(CommitBuilderMut<'_>, RepositoryRef<'_>, &CStr) -> Result<(), StreamError> + 'f,
    {
        Self { f, panic: None }
    }

    /// Resumes a panic the closure raised inside a libgit2 call it was lent to through
    /// [`CreateExtOptionsMut::set_signer`], if any: that call failed with `GIT_EUSER`,
    /// and the callback refuses to sign until the panic is resumed. A wrapper that runs
    /// the call itself resumes it on its own.
    pub fn resume_panic(&mut self) {
        if let Some(panic) = self.panic.take() {
            resume_unwind(panic);
        }
    }

    /// Runs `write` with the C callback and its payload, as the wrapper of a routine that
    /// signs through it (`git_commit_create_ext`, through its options' `sign` and
    /// `payload`) hands them to it, then resumes a panic the closure raised meanwhile. The
    /// payload is this callback, which nothing else touches until `write` returns.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn sign_with<R>(
        &mut self,
        write: impl FnOnce(ffi::git_commit_signature_cb, *mut c_void) -> R,
    ) -> R {
        let result = write(Some(sign_with_closure), ptr::from_mut(self).cast());
        self.resume_panic();
        result
    }
}

impl fmt::Debug for SignatureCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignatureCallback")
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_commit_signature_cb` over a [`SignatureCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`SignatureCallback`] that nothing else uses during the
/// call; `builder` must be NULL or the live builder of the commit being written, lent
/// for the call and used by nothing else meanwhile; `repo` NULL or a live repository
/// used on this thread only; `content` NULL or a NUL-terminated string live for the call.
unsafe extern "C" fn sign_with_closure(
    builder: *mut ffi::git_commitbuilder,
    repo: *mut ffi::git_repository,
    content: *const c_char,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: per this function's contract, `payload` is a live callback borrowed for
    // the call alone.
    let callback = unsafe { &mut *payload.cast::<SignatureCallback<'_>>() };
    if callback.panic.is_some() {
        return ErrorCode::User.raw();
    }
    // SAFETY: per this function's contract, a non-NULL `builder` is lent exclusively for
    // the call, and the handle given to the closure is valid for its call only.
    let builder = unsafe { CommitBuilderMut::from_ptr(builder) };
    // SAFETY: per this function's contract, a non-NULL `repo` is live and used on this
    // thread only for the call; the closure's handle is shared, as the routine's caller
    // holds it.
    let repo = unsafe { RepositoryRef::from_ptr(repo) };
    let (Some(builder), Some(repo), false) = (builder, repo, content.is_null()) else {
        // libgit2 never passes NULL; stop the commit rather than invent an argument.
        return ErrorCode::User.raw();
    };
    // SAFETY: per this function's contract, `content` is NUL-terminated and live for the
    // call; it is copied at once, before anything can free it.
    let content = unsafe { CStr::from_ptr(content) }.to_owned();
    match catch_unwind(AssertUnwindSafe(|| (callback.f)(builder, repo, &content))) {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => error.raise(),
        Err(panic) => {
            callback.panic = Some(panic);
            ErrorCode::User.raw()
        }
    }
}

/// The failure of a routine that returned `rc`, with the thread's last error.
fn failed<T>(rc: c_int) -> Result<T, RepositoryError> {
    Err(RepositoryError::from_libgit2(rc))
}

/// Writes a commit into `repo`'s object database whose contents are `content`, as
/// `git_commit_create_buffer` formats them, signed with `signature` under the header
/// `field` (`gpgsig` for `None`), and returns its id. The signature is added as
/// [`CommitBuilderMut::add_header`] adds one: after the last header, each newline
/// continued by a space. Without a signature the contents are written as they are.
///
/// The contents must parse as a commit of the repository's id type (a `tree`, then any
/// `parent`s, an `author` and a `committer`), and, unless strict object creation is
/// turned off, the tree and every parent must exist in the repository with those
/// types; the signature is not checked. libgit2 writes `field` as given.
///
/// Fails with `GIT_ERROR_OBJECT`, "failed to parse bad commit object", or the
/// signature parser's error, for contents that do not parse; with `GIT_ERROR_INVALID`,
/// "malformed commit contents", for a signed one without a blank line after its headers;
/// and with libgit2's error when a tree or parent is missing or the object database
/// cannot be loaded or written.
///
/// Wraps: git_commit_create_with_signature
pub fn create_with_signature(
    repo: RepositoryRef<'_>,
    content: &CStr,
    signature: Option<&CStr>,
    field: Option<&CStr>,
) -> Result<Oid, RepositoryError> {
    let mut id = Oid::zeroed();
    let out = id.as_mut().as_mut_ptr();
    let signature = signature.map_or(ptr::null(), CStr::as_ptr);
    let field = field.map_or(ptr::null(), CStr::as_ptr);
    // SAFETY: `out` addresses `id`, written only on success; `repo` is a live repository
    // on this thread, whose object database libgit2 loads and writes through this shared
    // handle; `content`, `signature` and `field` are NUL-terminated or NULL (the last
    // two), read for the call only. The contents' ids are parsed with the repository's
    // id type, so every id libgit2 reads from the object database is of that type.
    let rc = unsafe {
        ffi::git_commit_create_with_signature(
            out,
            repo.as_ptr().cast_mut(),
            content.as_ptr(),
            signature,
            field,
        )
    };
    if rc < 0 { failed(rc) } else { Ok(id) }
}

/// The signature of the commit `id` names in `repo`, under the header `field` (`gpgsig`
/// for `None`), and the contents it signs: the commit's raw contents without that
/// header. A signature spread over several lines is joined with newlines, without the
/// spaces that continue them. Returned as `(signature, signed_data)`.
///
/// The first header whose line starts with `field` and a space is taken. libgit2 matches
/// `field` as a prefix: a header whose name only starts with `field` (`gpgsig-sha256`
/// for `gpgsig`), placed before the one taken, loses those bytes from the signed data.
///
/// Fails with [`ErrorCode::NotFound`] when the object is not a commit
/// (`GIT_ERROR_INVALID`) or has no such header (`GIT_ERROR_OBJECT`, "this commit is not
/// signed"), or when the object database has no such object; with `GIT_ERROR_OBJECT`,
/// "malformed header", when the header's last line does not end; and with libgit2's error
/// when the object database cannot be loaded or read.
///
/// Refuses, before libgit2 reads anything, an `id` of another type than `repo`'s, as
/// [`Object::lookup`](crate::libgit2::object::object::Object::lookup) does (libgit2 would
/// format it out of bounds), and an empty `field`, with `GIT_ERROR_INVALID`, "the
/// signature header field name is empty": libgit2 would never get past the commit's
/// first line.
///
/// Wraps: git_commit_extract_signature
pub fn extract_signature(
    repo: RepositoryRef<'_>,
    id: OidRef<'_>,
    field: Option<&CStr>,
) -> Result<(BufVal, BufVal), RepositoryError> {
    id_fits(repo, id)?;
    if field.is_some_and(|f| f.is_empty()) {
        set(
            ErrorClass::Invalid,
            c"the signature header field name is empty",
        );
        return failed(ErrorCode::Generic.raw());
    }
    let (mut signature, mut signed_data) = (Buf::new(), Buf::new());
    let field = field.map_or(ptr::null(), CStr::as_ptr);
    let (sig, data) = (
        signature.as_mut().as_mut_ptr(),
        signed_data.as_mut().as_mut_ptr(),
    );
    // SAFETY: `sig` and `data` address the fresh, empty buffers held above, which libgit2
    // fills (`git_buf_fromstr`) or leaves empty; `repo` is a live repository on this
    // thread, whose object database libgit2 loads through this shared handle; `id` is a
    // live id of the repository's type (checked above), only read despite the
    // signature's `git_oid *`; `field` is NULL or a non-empty NUL-terminated string read
    // for the call only.
    let rc = unsafe {
        ffi::git_commit_extract_signature(
            sig,
            data,
            repo.as_ptr().cast_mut(),
            id.as_ptr().cast_mut(),
            field,
        )
    };
    if rc < 0 {
        failed(rc)
    } else {
        Ok((signature, signed_data))
    }
}

/// One extra header of a commit being created (`git_commit_header`): a field name and
/// its value, both borrowed for `'h`, as the options of `git_commit_create_ext` take
/// them in an array.
///
/// Built by [`new`](Self::new); read it through [`as_ref`](Self::as_ref) and change it
/// through [`as_mut`](Self::as_mut):
///
/// ```
/// use libgit2::libgit2::object::commit::Header;
///
/// let mut header = Header::new(c"line_one", c"First extra header");
/// assert_eq!(header.as_ref().field(), c"line_one");
/// header.as_mut().set_value(c"Second\nline");
/// assert_eq!(header.as_ref().value(), c"Second\nline");
///
/// // A `const` array is the C array the options point at.
/// const HEADERS: [Header<'static>; 2] = [
///     Header::new(c"line_one", c"First extra header"),
///     Header::new(c"line_two", c"Second extra header"),
/// ];
/// assert_eq!(HEADERS[1].as_ref().field(), c"line_two");
/// ```
///
/// `#[repr(transparent)]` over `git_commit_header`, so a pointer to it, or to the first
/// of a `[Header]`, is a pointer to the C struct or array, which C reads in place. `'h`
/// is the lifetime of the strings it points at, so a header cannot outlive them:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::object::commit::Header;
/// use std::ffi::CString;
///
/// let header;
/// {
///     let value = CString::new("Extra").unwrap();
///     header = Header::new(c"field", &value);
/// }
/// let _ = header.as_ref().value();
/// ```
///
/// The exclusive handle is invariant in `'h`, so one cannot be shrunk to store a
/// shorter-lived string than its header was built for:
///
/// ```compile_fail
/// use libgit2::libgit2::object::commit::HeaderMut;
///
/// fn shrink<'a, 'h>(m: HeaderMut<'a, 'static>) -> HeaderMut<'a, 'h> {
///     m
/// }
/// ```
///
/// The shared handle only reads:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::object::commit::Header;
///
/// let header = Header::new(c"field", c"value");
/// header.as_ref().set_value(c"other");
/// ```
///
/// `Copy`, `Send` and `Sync`, as a pair of `&'h CStr`s is: libgit2 never writes
/// through the strings, and only the exclusive handle writes the fields. Equal when
/// both strings are.
///
/// Wraps: git_commit_header
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Header<'h>(ffi::git_commit_header, PhantomData<&'h CStr>);

/// Shared borrow of a [`Header`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct HeaderRef<'a, 'h>(CBorrowedPtr<'a, Header<'h>>);

/// Exclusive borrow of a [`Header`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct HeaderMut<'a, 'h>(
    HeaderRef<'a, 'h>,
    // Invariant in the header, and so in `'h`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Header<'h>>,
);

// SAFETY: `Header` is `#[repr(transparent)]` over `git_commit_header` with a zero-sized
// marker; both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Header<'h>>`
// (`HeaderMut` through `HeaderRef`) with zero-sized markers and no `Drop`, hand out no
// reference to the header, and `HeaderRef` has no write operation. `HeaderMut` is
// invariant in the header through its `PhantomData<&'a mut Header<'h>>`.
unsafe impl<'h> CCell for Header<'h> {
    type C = ffi::git_commit_header;
    type Ref<'a>
        = HeaderRef<'a, 'h>
    where
        Self: 'a;
    type Mut<'a>
        = HeaderMut<'a, 'h>
    where
        Self: 'a;
}

// SAFETY: a header is two string pointers that only the setters write, each a
// `&'h CStr`'s pointer: shared, immutable borrows of `CStr`, which is `Sync`, so they
// may be read from any thread. Nothing in the struct has thread affinity or interior
// mutability, and nothing is released on drop.
unsafe impl Send for Header<'_> {}
// SAFETY: as above; `HeaderMut` is an exclusive borrow of such a value, as
// `&mut Header`.
unsafe impl Send for HeaderMut<'_, '_> {}

// SAFETY: as above, and shared access only reads the fields (every write goes through
// `HeaderMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for Header<'_> {}
// SAFETY: as above; `HeaderRef` is a shared borrow, as `&Header`.
unsafe impl Send for HeaderRef<'_, '_> {}
// SAFETY: as above.
unsafe impl Sync for HeaderRef<'_, '_> {}
// SAFETY: `&HeaderMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for HeaderMut<'_, '_> {}

impl<'h> Header<'h> {
    /// The header `<field> <value>`. libgit2 writes both as given, each newline of
    /// `value` continued by a space; `field` should hold neither a space nor a newline.
    pub const fn new(field: &'h CStr, value: &'h CStr) -> Self {
        Self(
            ffi::git_commit_header {
                field: field.as_ptr(),
                value: value.as_ptr(),
            },
            PhantomData,
        )
    }

    /// Shared handle to the header — the getters.
    pub fn as_ref(&self) -> HeaderRef<'_, 'h> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        HeaderRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the header — the setters.
    pub fn as_mut(&mut self) -> HeaderMut<'_, 'h> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        HeaderMut(HeaderRef(ptr), PhantomData)
    }
}

impl PartialEq for Header<'_> {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.as_ref(), other.as_ref());
        a.field() == b.field() && a.value() == b.value()
    }
}

impl Eq for Header<'_> {}

impl fmt::Debug for Header<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'h> HeaderRef<'a, 'h> {
    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_commit_header *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_commit_header {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The field name, borrowed for `'h`, as it was set.
    ///
    /// Field: git_commit_header.field
    pub fn field(&self) -> &'h CStr {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to a live header. `new` and
        // every setter store a `&'h CStr`'s pointer, a NUL-terminated string that
        // outlives `'h` and that nothing writes, being shared-borrowed.
        unsafe { CStr::from_ptr(addr_of!((*p).field).read()) }
    }

    /// The value, borrowed for `'h`, as it was set.
    ///
    /// Field: git_commit_header.value
    pub fn value(&self) -> &'h CStr {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to a live header. `new` and
        // every setter store a `&'h CStr`'s pointer, a NUL-terminated string that
        // outlives `'h` and that nothing writes, being shared-borrowed.
        unsafe { CStr::from_ptr(addr_of!((*p).value).read()) }
    }

    /// A copy of the header, as a value of its own, borrowing the same strings.
    pub fn to_header(&self) -> Header<'h> {
        Header::new(self.field(), self.value())
    }
}

impl fmt::Debug for HeaderRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Header")
            .field("field", &self.field())
            .field("value", &self.value())
            .finish()
    }
}

impl<'h> HeaderMut<'_, 'h> {
    /// Writable pointer to the C struct, for the setters.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_commit_header {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> HeaderRef<'_, 'h> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> HeaderMut<'_, 'h> {
        HeaderMut(self.0, PhantomData)
    }

    /// Replaces the field name; the old string, borrowed, needs no release.
    ///
    /// Field: git_commit_header.field
    pub fn set_field(&mut self, field: &'h CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to a live header of a
        // `&'h CStr`'s pointer, which keeps the getters' guarantee; the old pointer is
        // borrowed, so nothing is released.
        unsafe { addr_of_mut!((*p).field).write(field.as_ptr()) }
    }

    /// Replaces the value; the old string, borrowed, needs no release.
    ///
    /// Field: git_commit_header.value
    pub fn set_value(&mut self, value: &'h CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to a live header of a
        // `&'h CStr`'s pointer, which keeps the getters' guarantee; the old pointer is
        // borrowed, so nothing is released.
        unsafe { addr_of_mut!((*p).value).write(value.as_ptr()) }
    }
}

impl fmt::Debug for HeaderMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// A `Header` is a `git_commit_header`, so a `[Header]` is the array C reads in place.
const _: () = assert!(
    size_of::<Header<'static>>() == size_of::<ffi::git_commit_header>()
        && align_of::<Header<'static>>() == align_of::<ffi::git_commit_header>()
        && size_of::<[Header<'static>; 3]>() == size_of::<[ffi::git_commit_header; 3]>()
);

/// Options for creating a commit with `git_commit_create_ext`
/// (`git_commit_create_ext_options`), held by value in Rust storage.
///
/// Built by [`new`](Self::new) (or [`Default`]) with libgit2's defaults: no reference
/// to update, no encoding header, no extra headers and no signing. Read it through
/// [`as_ref`](Self::as_ref) and change it through [`as_mut`](Self::as_mut):
///
/// ```
/// use libgit2::libgit2::object::commit::{CreateExtOptions, Header};
///
/// const HEADERS: [Header<'static>; 1] = [Header::new(c"x-tool", c"crustify")];
/// let mut options = CreateExtOptions::new();
/// let mut m = options.as_mut();
/// m.set_update_ref(Some(c"HEAD"));
/// m.set_extra_headers(&HEADERS);
/// assert_eq!(options.as_ref().update_ref(), Some(c"HEAD"));
/// assert_eq!(options.as_ref().extra_headers().len(), 1);
/// assert_eq!(options.as_ref().message_encoding(), None);
/// ```
///
/// `#[repr(transparent)]` over `git_commit_create_ext_options`, so a pointer to it is a
/// pointer to the C struct, which C reads in place. `'o` is the lifetime of what it
/// points at: the strings and headers its safe setters take are borrowed for `'o`, so
/// the options cannot outlive them:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::object::commit::CreateExtOptions;
/// use std::ffi::CString;
///
/// let mut options = CreateExtOptions::new();
/// {
///     let encoding = CString::new("ISO-8859-1").unwrap();
///     options.as_mut().set_message_encoding(Some(&encoding));
/// }
/// let _ = options.as_ref().message_encoding();
/// ```
///
/// The exclusive handle is invariant in `'o`, so one cannot be shrunk to accept a
/// shorter-lived borrow than its options were built for:
///
/// ```compile_fail
/// use libgit2::libgit2::object::commit::CreateExtOptionsMut;
///
/// fn shrink<'a, 'o>(m: CreateExtOptionsMut<'a, 'static>) -> CreateExtOptionsMut<'a, 'o> {
///     m
/// }
/// ```
///
/// Neither `Send` nor `Sync`: the payload of the signing callback
/// ([`set_signer`](CreateExtOptionsMut::set_signer)) is a [`SignatureCallback`] borrowed
/// exclusively, whose closure need not be `Send`, and which libgit2 calls on the thread
/// writing the commit.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::commit::CreateExtOptions;
///
/// let options = CreateExtOptions::new();
/// std::thread::spawn(move || drop(options));
/// ```
///
/// Not `Clone`: the payload its callback receives may be an exclusive borrow, which a
/// copy would alias. Build another with [`new`](Self::new) instead.
///
/// Wraps: git_commit_create_ext_options
/// Handwritten
#[repr(transparent)]
pub struct CreateExtOptions<'o>(
    ffi::git_commit_create_ext_options,
    PhantomData<(*const (), &'o CStr, &'o [Header<'o>])>,
);

/// Shared borrow of a [`CreateExtOptions`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct CreateExtOptionsRef<'a, 'o>(CBorrowedPtr<'a, CreateExtOptions<'o>>);

/// Exclusive borrow of a [`CreateExtOptions`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct CreateExtOptionsMut<'a, 'o>(
    CreateExtOptionsRef<'a, 'o>,
    // Invariant in the options, and so in `'o`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut CreateExtOptions<'o>>,
);

// SAFETY: `CreateExtOptions` is `#[repr(transparent)]` over
// `git_commit_create_ext_options` with a zero-sized marker; both handles are
// `#[repr(transparent)]` over `CBorrowedPtr<'a, CreateExtOptions<'o>>`
// (`CreateExtOptionsMut` through `CreateExtOptionsRef`) with zero-sized markers and no
// `Drop`, hand out no reference to the options, and `CreateExtOptionsRef` has no write
// operation. `CreateExtOptionsMut` is invariant in the options through its
// `PhantomData<&'a mut CreateExtOptions<'o>>`.
unsafe impl<'o> CCell for CreateExtOptions<'o> {
    type C = ffi::git_commit_create_ext_options;
    type Ref<'a>
        = CreateExtOptionsRef<'a, 'o>
    where
        Self: 'a;
    type Mut<'a>
        = CreateExtOptionsMut<'a, 'o>
    where
        Self: 'a;
}

impl<'o> CreateExtOptions<'o> {
    /// libgit2's default options, as `GIT_COMMIT_CREATE_EXT_OPTIONS_INIT` spells them:
    /// version 1, every other field NULL, zero or `None`.
    ///
    /// `git_commit_create_ext_options_init` only fails for a version other than
    /// `GIT_COMMIT_CREATE_EXT_OPTIONS_VERSION`, and with that version it reaches no
    /// global, no allocator and no thread state, so this needs no initialised library.
    ///
    /// Wraps: git_commit_create_ext_options_init
    pub fn new() -> Self {
        let mut raw = MaybeUninit::<ffi::git_commit_create_ext_options>::uninit();
        // SAFETY: `raw` is writable storage for one `git_commit_create_ext_options`,
        // which `git_commit_create_ext_options_init` overwrites whole with the template
        // for this version; nothing else reads it during the call.
        let rc = unsafe {
            ffi::git_commit_create_ext_options_init(
                raw.as_mut_ptr(),
                ffi::GIT_COMMIT_CREATE_EXT_OPTIONS_VERSION,
            )
        };
        assert_eq!(
            rc, 0,
            "git_commit_create_ext_options_init refused its own version"
        );
        // SAFETY: a 0 return means the template was copied in: every field is
        // initialised, every pointer NULL and the callback `None`.
        Self(unsafe { raw.assume_init() }, PhantomData)
    }

    /// Shared handle to the options — the getters, and the pointer for C calls that
    /// read them.
    pub fn as_ref(&self) -> CreateExtOptionsRef<'_, 'o> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        CreateExtOptionsRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the options — the setters.
    pub fn as_mut(&mut self) -> CreateExtOptionsMut<'_, 'o> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        CreateExtOptionsMut(CreateExtOptionsRef(ptr), PhantomData)
    }
}

impl Default for CreateExtOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CreateExtOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'o> CreateExtOptionsRef<'a, 'o> {
    /// Borrows the options at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must address live, initialised options (or be NULL) that outlive `'a` and
    /// that nothing writes while the handle lives, whose strings are NULL or
    /// NUL-terminated and outlive `'o`, and whose `extra_headers` is NULL or addresses
    /// `extra_headers_len` headers whose strings are non-NULL and NUL-terminated, all
    /// outliving `'o`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_commit_create_ext_options) -> Option<Self> {
        let ptr = NonNull::new(ptr.cast::<CreateExtOptions<'o>>())?;
        // SAFETY: layout-preserving cast per `#[repr(transparent)]`; the caller upholds
        // liveness, the lifetimes and the absence of writers.
        Some(Self(unsafe { CBorrowedPtr::new(ptr) }))
    }

    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_commit_create_ext_options *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_commit_create_ext_options {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The structure's version, `GIT_COMMIT_CREATE_EXT_OPTIONS_VERSION`; fixed by
    /// [`CreateExtOptions::new`].
    ///
    /// Field: git_commit_create_ext_options.version
    pub fn version(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).version).read() }
    }

    /// The name of the reference moved to the new commit; `None`, the default, moves
    /// none.
    ///
    /// Field: git_commit_create_ext_options.update_ref
    pub fn update_ref(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).update_ref).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The message's encoding, written as the commit's `encoding` header; `None`, the
    /// default, writes no such header (git reads the message as UTF-8).
    ///
    /// Field: git_commit_create_ext_options.message_encoding
    pub fn message_encoding(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).message_encoding).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The extra headers written after `committer` and any `encoding`, in order;
    /// empty, the default, writes none. Borrowed for `'o`, as they were set; each reads
    /// through its [`HeaderRef`].
    ///
    /// Field: git_commit_create_ext_options.extra_headers
    /// Field: git_commit_create_ext_options.extra_headers_len
    pub fn extra_headers(&self) -> CSlice<'o, Header<'o>> {
        let p = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to live options.
        let (headers, len) = unsafe {
            (
                addr_of!((*p).extra_headers).read(),
                addr_of!((*p).extra_headers_len).read(),
            )
        };
        match NonNull::new(headers.cast_mut()) {
            // SAFETY: every setter stores NULL with a length of 0, or the pointer and
            // length of a `&'o [Header<'o>]`: `len` initialised headers (`Header` is
            // transparent over `git_commit_header`) that outlive `'o` and that nothing
            // writes, being shared-borrowed.
            Some(headers) => unsafe { CSlice::from_raw_parts(headers.cast(), len) },
            // NULL is no headers; a run may not start at NULL.
            // SAFETY: zero elements at an aligned dangling pointer.
            None => unsafe { CSlice::from_raw_parts(NonNull::dangling(), 0) },
        }
    }

    /// Whether a signing callback is set
    /// ([`set_signer`](CreateExtOptionsMut::set_signer)); `false` by default.
    ///
    /// Field: git_commit_create_ext_options.sign
    /// Field: git_commit_create_ext_options.payload
    pub fn has_signer(&self) -> bool {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).sign).read() }.is_some()
    }

    /// The signing callback and its payload as libgit2 reads them: `None` and NULL by
    /// default, otherwise the trampoline of the [`SignatureCallback`] that
    /// [`set_signer`](CreateExtOptionsMut::set_signer) stored, and that callback. For the
    /// unit tests, which hand them to libgit2 themselves.
    #[cfg(test)]
    pub(crate) fn sign_callback(&self) -> (ffi::git_commit_signature_cb, *mut c_void) {
        let p = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to live options.
        unsafe { (addr_of!((*p).sign).read(), addr_of!((*p).payload).read()) }
    }
}

impl fmt::Debug for CreateExtOptionsRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateExtOptions")
            .field("version", &self.version())
            .field("update_ref", &self.update_ref())
            .field("message_encoding", &self.message_encoding())
            .field("extra_headers", &DebugHeaders(self.extra_headers()))
            .field("sign_callback", &self.has_signer())
            .finish()
    }
}

/// A run of headers, printed as a list.
struct DebugHeaders<'o>(CSlice<'o, Header<'o>>);

impl fmt::Debug for DebugHeaders<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.iter()).finish()
    }
}

impl<'a, 'o> CreateExtOptionsMut<'a, 'o> {
    /// Borrows the options at `ptr` exclusively; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// As [`CreateExtOptionsRef::from_ptr`], plus: nothing else reads or writes the
    /// options while the result lives, and whatever the setters store may stay there
    /// for as long as the options' owner expects `'o` borrows to.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_commit_create_ext_options) -> Option<Self> {
        // SAFETY: the caller upholds `CreateExtOptionsRef::from_ptr`'s contract, and
        // excludes every other access for `'a`.
        let shared = unsafe { CreateExtOptionsRef::from_ptr(ptr) }?;
        Some(Self(shared, PhantomData))
    }

    /// Writable pointer to the C struct, for the setters.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_commit_create_ext_options {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> CreateExtOptionsRef<'_, 'o> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> CreateExtOptionsMut<'_, 'o> {
        CreateExtOptionsMut(self.0, PhantomData)
    }

    /// Sets the reference moved to the new commit; `None` moves none. A symbolic
    /// reference (`HEAD`, say) is resolved to the reference it names, created when
    /// missing; an existing one must point at the commit's first parent, or the call
    /// fails with `GIT_EMODIFIED` without writing the reference.
    ///
    /// Field: git_commit_create_ext_options.update_ref
    pub fn set_update_ref(&mut self, name: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).update_ref).write(name.map_or(ptr::null(), CStr::as_ptr)) }
    }

    /// Sets the message's encoding, written as the commit's `encoding` header as given;
    /// `None` writes no such header.
    ///
    /// Field: git_commit_create_ext_options.message_encoding
    pub fn set_message_encoding(&mut self, encoding: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe {
            addr_of_mut!((*p).message_encoding).write(encoding.map_or(ptr::null(), CStr::as_ptr))
        }
    }

    /// Sets the extra headers, written in order after `committer` and any `encoding`,
    /// each as `<field> <value>`; an empty run writes none, and is stored as C stores
    /// it, NULL with a length of 0. Borrowed for `'o`, run and strings alike.
    ///
    /// Field: git_commit_create_ext_options.extra_headers
    /// Field: git_commit_create_ext_options.extra_headers_len
    pub fn set_extra_headers(&mut self, headers: &'o [Header<'o>]) {
        let run: *const ffi::git_commit_header = if headers.is_empty() {
            ptr::null()
        } else {
            headers.as_ptr().cast()
        };
        let p = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to live options; the
        // fields are plain data, so the old values need no drop. `run` is NULL with a
        // length of 0, or `headers`' own pointer and length, a run of initialised
        // `git_commit_header`s (`Header` is transparent over it) whose strings are
        // non-NULL, as `extra_headers()` requires.
        unsafe {
            addr_of_mut!((*p).extra_headers).write(run);
            addr_of_mut!((*p).extra_headers_len).write(headers.len());
        }
    }

    /// Sets the signing callback, borrowed exclusively for `'o`; `None` signs nothing.
    /// libgit2 calls it once per commit created with these options, after building the
    /// commit and before writing it, with the builder to add the signature header to
    /// (see [`SignatureCallback`]); an error fails the call, nothing written.
    ///
    /// The options store the callback's trampoline and, as its payload, the callback
    /// itself, which they borrow until they are dropped: it cannot be used, moved or
    /// dropped meanwhile, nor lent to other options, so each libgit2 call reaches it
    /// alone. A panic of its closure stops the commit with `GIT_EUSER` and is kept in the
    /// callback, which [`resume_panic`](SignatureCallback::resume_panic) resumes once the
    /// options are gone.
    ///
    /// ```compile_fail,E0499
    /// use libgit2::libgit2::core::types::StreamError;
    /// use libgit2::libgit2::object::commit::{CommitBuilderMut, CreateExtOptions, SignatureCallback};
    /// use libgit2::libgit2::repository::repository::RepositoryRef;
    ///
    /// let mut f = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &core::ffi::CStr| Ok::<(), StreamError>(());
    /// let mut signer = SignatureCallback::new(&mut f);
    /// let mut options = CreateExtOptions::new();
    /// options.as_mut().set_signer(Some(&mut signer));
    /// let mut other = CreateExtOptions::new();
    /// other.as_mut().set_signer(Some(&mut signer));
    /// drop((options, other));
    /// ```
    ///
    /// Field: git_commit_create_ext_options.sign
    /// Field: git_commit_create_ext_options.payload
    pub fn set_signer(&mut self, signer: Option<&'o mut SignatureCallback<'_>>) {
        let (callback, payload): (ffi::git_commit_signature_cb, *mut c_void) = match signer {
            Some(signer) => (Some(sign_with_closure), ptr::from_mut(signer).cast()),
            None => (None, ptr::null_mut()),
        };
        let p = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to live options; the
        // fields are plain data, so the old values need no drop. The payload is the
        // callback the options borrow exclusively for `'o`, for which the trampoline is
        // sound with libgit2's arguments.
        unsafe {
            addr_of_mut!((*p).sign).write(callback);
            addr_of_mut!((*p).payload).write(payload);
        }
    }
}

impl fmt::Debug for CreateExtOptionsMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// A `CreateExtOptions` is a `git_commit_create_ext_options`, so C reads a value Rust
// holds in place.
const _: () = assert!(
    size_of::<CreateExtOptions<'static>>() == size_of::<ffi::git_commit_create_ext_options>()
        && align_of::<CreateExtOptions<'static>>()
            == align_of::<ffi::git_commit_create_ext_options>()
);

/// Options for committing the index or a tree on `HEAD` with
/// `git_commit_create_from_stage`, `git_commit_create_from_tree` and their `amend`
/// counterparts (`git_commit_create_options`), held by value in Rust storage.
///
/// Built by [`new`](Self::new) (or [`Default`]) with libgit2's defaults: no empty
/// commits, the default author and committer, no encoding header, no extra headers and
/// no signing. Read it through [`as_ref`](Self::as_ref) and change it through
/// [`as_mut`](Self::as_mut):
///
/// ```
/// use libgit2::libgit2::core::types::{SignatureView, Time};
/// use libgit2::libgit2::object::commit::{CreateOptions, Header};
///
/// const HEADERS: [Header<'static>; 1] = [Header::new(c"x-tool", c"crustify")];
/// let author = SignatureView::new(c"A U Thor", c"author@example.com", Time::new(0, 0));
/// let mut options = CreateOptions::new();
/// let mut m = options.as_mut();
/// m.set_allow_empty_commit(true);
/// m.set_author(Some(author.as_ref()));
/// m.set_extra_headers(&HEADERS);
/// let r = options.as_ref();
/// assert!(r.allow_empty_commit());
/// assert_eq!(r.author().unwrap().name(), Some(c"A U Thor"));
/// assert!(r.committer().is_none());
/// assert_eq!(r.extra_headers().len(), 1);
/// ```
///
/// `#[repr(transparent)]` over `git_commit_create_options`, so a pointer to it is a
/// pointer to the C struct, which C reads in place. `'o` is the lifetime of what it
/// points at: the signatures, strings and headers its safe setters take are borrowed
/// for `'o`, so the options cannot outlive them:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::core::types::{SignatureView, Time};
/// use libgit2::libgit2::object::commit::CreateOptions;
///
/// let mut options = CreateOptions::new();
/// {
///     let committer = SignatureView::new(c"C", c"c@example.com", Time::new(0, 0));
///     options.as_mut().set_committer(Some(committer.as_ref()));
/// }
/// let _ = options.as_ref().committer();
/// ```
///
/// The exclusive handle is invariant in `'o`, so one cannot be shrunk to accept a
/// shorter-lived borrow than its options were built for:
///
/// ```compile_fail
/// use libgit2::libgit2::object::commit::CreateOptionsMut;
///
/// fn shrink<'a, 'o>(m: CreateOptionsMut<'a, 'static>) -> CreateOptionsMut<'a, 'o> {
///     m
/// }
/// ```
///
/// Neither `Send` nor `Sync`: the payload of the signing callback
/// ([`set_signer`](CreateOptionsMut::set_signer)) is a [`SignatureCallback`] borrowed
/// exclusively, whose closure need not be `Send`, and which libgit2 calls on the thread
/// writing the commit.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::commit::CreateOptions;
///
/// let options = CreateOptions::new();
/// std::thread::spawn(move || drop(options));
/// ```
///
/// Not `Clone`: the payload its callback receives may be an exclusive borrow, which a
/// copy would alias. Build another with [`new`](Self::new) instead.
///
/// Wraps: git_commit_create_options
/// Handwritten
#[repr(transparent)]
pub struct CreateOptions<'o>(
    ffi::git_commit_create_options,
    PhantomData<(*const (), &'o CStr, &'o [Header<'o>], SignatureRef<'o>)>,
);

/// Shared borrow of a [`CreateOptions`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct CreateOptionsRef<'a, 'o>(CBorrowedPtr<'a, CreateOptions<'o>>);

/// Exclusive borrow of a [`CreateOptions`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct CreateOptionsMut<'a, 'o>(
    CreateOptionsRef<'a, 'o>,
    // Invariant in the options, and so in `'o`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut CreateOptions<'o>>,
);

// SAFETY: `CreateOptions` is `#[repr(transparent)]` over `git_commit_create_options`
// with a zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, CreateOptions<'o>>` (`CreateOptionsMut` through `CreateOptionsRef`)
// with zero-sized markers and no `Drop`, hand out no reference to the options, and
// `CreateOptionsRef` has no write operation. `CreateOptionsMut` is invariant in the
// options through its `PhantomData<&'a mut CreateOptions<'o>>`.
unsafe impl<'o> CCell for CreateOptions<'o> {
    type C = ffi::git_commit_create_options;
    type Ref<'a>
        = CreateOptionsRef<'a, 'o>
    where
        Self: 'a;
    type Mut<'a>
        = CreateOptionsMut<'a, 'o>
    where
        Self: 'a;
}

impl<'o> CreateOptions<'o> {
    /// libgit2's default options, as `GIT_COMMIT_CREATE_OPTIONS_INIT` spells them:
    /// version 1, empty commits refused, every other field NULL, zero or `None`.
    ///
    /// `git_commit_create_options_init` only fails for a version other than
    /// `GIT_COMMIT_CREATE_OPTIONS_VERSION`, and with that version it reaches no global,
    /// no allocator and no thread state, so this needs no initialised library.
    ///
    /// Wraps: git_commit_create_options_init
    pub fn new() -> Self {
        // Zeroed rather than uninitialised: the bitfield's storage byte holds seven
        // unnamed bits besides `allow_empty_commit`, which C may treat as padding.
        let mut raw = MaybeUninit::<ffi::git_commit_create_options>::zeroed();
        // SAFETY: `raw` is writable storage for one `git_commit_create_options`, which
        // `git_commit_create_options_init` overwrites whole with the template for this
        // version; nothing else reads it during the call.
        let rc = unsafe {
            ffi::git_commit_create_options_init(
                raw.as_mut_ptr(),
                ffi::GIT_COMMIT_CREATE_OPTIONS_VERSION,
            )
        };
        assert_eq!(
            rc, 0,
            "git_commit_create_options_init refused its own version"
        );
        // SAFETY: a 0 return means the template was copied in: every field is
        // initialised, the bitfield's storage byte included, every pointer NULL and the
        // callback `None`.
        Self(unsafe { raw.assume_init() }, PhantomData)
    }

    /// Shared handle to the options — the getters, and the pointer for C calls that
    /// read them.
    pub fn as_ref(&self) -> CreateOptionsRef<'_, 'o> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        CreateOptionsRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the options — the setters.
    pub fn as_mut(&mut self) -> CreateOptionsMut<'_, 'o> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        CreateOptionsMut(CreateOptionsRef(ptr), PhantomData)
    }
}

impl Default for CreateOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CreateOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'o> CreateOptionsRef<'a, 'o> {
    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_commit_create_options *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_commit_create_options {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The structure's version, `GIT_COMMIT_CREATE_OPTIONS_VERSION`; fixed by
    /// [`CreateOptions::new`].
    ///
    /// Field: git_commit_create_options.version
    pub fn version(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).version).read() }
    }

    /// Whether a commit that changes nothing is written; `false`, the default, fails
    /// the commit (class `GIT_ERROR_REPOSITORY`, "no changes are staged for commit")
    /// when the tree is the one `HEAD` points at, or empty on an unborn `HEAD`:
    /// `git_commit_create_from_stage` returns `GIT_EUNCHANGED`, while
    /// `git_commit_create_from_tree` returns `-1` (`GIT_ERROR`) for the same check.
    /// Only the routines creating a commit read it; amending ignores it.
    ///
    /// Field: git_commit_create_options.allow_empty_commit
    pub fn allow_empty_commit(&self) -> bool {
        let p = self.as_ptr();
        // SAFETY: a read of the one-bit field's storage byte through the shared
        // handle's pointer to live options; bindgen's accessor projects it with
        // `addr_of!` and forms no reference.
        unsafe { ffi::git_commit_create_options::allow_empty_commit_raw(p) != 0 }
    }

    /// The commit's author; `None`, the default, is the repository's default signature
    /// (`user.name` and `user.email` from its configuration, at the current time) when
    /// creating, and the amended commit's author when amending. Borrowed for `'o`, as
    /// it was set.
    ///
    /// Field: git_commit_create_options.author
    pub fn author(&self) -> Option<SignatureRef<'o>> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `SignatureRef<'o>`'s pointer: a live signature that
        // outlives `'o` and that nothing writes, being shared-borrowed, whose strings
        // are NULL or NUL-terminated and outlive it.
        unsafe { SignatureRef::from_ptr(addr_of!((*p).author).read().cast_mut()) }
    }

    /// The commit's committer; `None`, the default, is the repository's default
    /// signature, when creating and amending alike. Borrowed for `'o`, as it was set.
    ///
    /// Field: git_commit_create_options.committer
    pub fn committer(&self) -> Option<SignatureRef<'o>> {
        let p = self.as_ptr();
        // SAFETY: as `author`: every setter stores NULL or a `SignatureRef<'o>`'s
        // pointer, live and unwritten for `'o`.
        unsafe { SignatureRef::from_ptr(addr_of!((*p).committer).read().cast_mut()) }
    }

    /// The message's encoding, written as the commit's `encoding` header; `None`, the
    /// default, writes no such header (git reads the message as UTF-8). Amending
    /// without a new message keeps the amended commit's encoding instead.
    ///
    /// Field: git_commit_create_options.message_encoding
    pub fn message_encoding(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).message_encoding).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The extra headers written after `committer` and any `encoding`, in order;
    /// empty, the default, writes none. Borrowed for `'o`, as they were set; each reads
    /// through its [`HeaderRef`].
    ///
    /// Field: git_commit_create_options.extra_headers
    /// Field: git_commit_create_options.extra_headers_len
    pub fn extra_headers(&self) -> CSlice<'o, Header<'o>> {
        let p = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to live options.
        let (headers, len) = unsafe {
            (
                addr_of!((*p).extra_headers).read(),
                addr_of!((*p).extra_headers_len).read(),
            )
        };
        match NonNull::new(headers.cast_mut()) {
            // SAFETY: every setter stores NULL with a length of 0, or the pointer and
            // length of a `&'o [Header<'o>]`: `len` initialised headers (`Header` is
            // transparent over `git_commit_header`) that outlive `'o` and that nothing
            // writes, being shared-borrowed.
            Some(headers) => unsafe { CSlice::from_raw_parts(headers.cast(), len) },
            // NULL is no headers; a run may not start at NULL.
            // SAFETY: zero elements at an aligned dangling pointer.
            None => unsafe { CSlice::from_raw_parts(NonNull::dangling(), 0) },
        }
    }

    /// Whether a signing callback is set
    /// ([`set_signer`](CreateOptionsMut::set_signer)); `false` by default.
    ///
    /// Field: git_commit_create_options.sign
    /// Field: git_commit_create_options.payload
    pub fn has_signer(&self) -> bool {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).sign).read() }.is_some()
    }

    /// The signing callback and its payload as libgit2 reads them: `None` and NULL by
    /// default, otherwise the trampoline of the [`SignatureCallback`] that
    /// [`set_signer`](CreateOptionsMut::set_signer) stored, and that callback. For the
    /// unit tests.
    #[cfg(test)]
    pub(crate) fn sign_callback(&self) -> (ffi::git_commit_signature_cb, *mut c_void) {
        let p = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to live options.
        unsafe { (addr_of!((*p).sign).read(), addr_of!((*p).payload).read()) }
    }
}

impl fmt::Debug for CreateOptionsRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateOptions")
            .field("version", &self.version())
            .field("allow_empty_commit", &self.allow_empty_commit())
            .field("author", &self.author())
            .field("committer", &self.committer())
            .field("message_encoding", &self.message_encoding())
            .field("extra_headers", &DebugHeaders(self.extra_headers()))
            .field("sign_callback", &self.has_signer())
            .finish()
    }
}

impl<'a, 'o> CreateOptionsMut<'a, 'o> {
    /// Writable pointer to the C struct, for the setters.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_commit_create_options {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> CreateOptionsRef<'_, 'o> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> CreateOptionsMut<'_, 'o> {
        CreateOptionsMut(self.0, PhantomData)
    }

    /// Sets whether a commit that changes nothing is written (see
    /// [`CreateOptionsRef::allow_empty_commit`]).
    ///
    /// Field: git_commit_create_options.allow_empty_commit
    pub fn set_allow_empty_commit(&mut self, allow: bool) {
        let p = self.as_mut_ptr();
        // SAFETY: a read-modify-write of the one-bit field's storage byte through the
        // exclusive handle's pointer to live options, leaving its other bits as they
        // were; bindgen's accessor projects it with `addr_of_mut!` and forms no
        // reference.
        unsafe { ffi::git_commit_create_options::set_allow_empty_commit_raw(p, allow.into()) }
    }

    /// Sets the commit's author; `None` takes the default (see
    /// [`CreateOptionsRef::author`]). Borrowed for `'o`.
    ///
    /// libgit2 writes the signature's strings as they are, and prints both: a wrapper
    /// handing these options to libgit2 refuses a signature that lost its name or its
    /// email (see the [signature docs](crate::libgit2::core::types#signatures)), which
    /// the options store as given.
    ///
    /// Field: git_commit_create_options.author
    pub fn set_author(&mut self, author: Option<SignatureRef<'o>>) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop. The pointer is NULL or a
        // `SignatureRef<'o>`'s, live and unwritten for `'o`, as `author()` requires.
        unsafe { addr_of_mut!((*p).author).write(author.map_or(ptr::null(), |s| s.as_ptr())) }
    }

    /// Sets the commit's committer; `None` takes the default (see
    /// [`CreateOptionsRef::committer`]). Borrowed for `'o`; refused by the writing
    /// wrappers when it lost a string, as [`set_author`](Self::set_author)'s is.
    ///
    /// Field: git_commit_create_options.committer
    pub fn set_committer(&mut self, committer: Option<SignatureRef<'o>>) {
        let p = self.as_mut_ptr();
        // SAFETY: as `set_author`.
        unsafe { addr_of_mut!((*p).committer).write(committer.map_or(ptr::null(), |s| s.as_ptr())) }
    }

    /// Sets the message's encoding, written as the commit's `encoding` header as given;
    /// `None` writes no such header.
    ///
    /// Field: git_commit_create_options.message_encoding
    pub fn set_message_encoding(&mut self, encoding: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe {
            addr_of_mut!((*p).message_encoding).write(encoding.map_or(ptr::null(), CStr::as_ptr))
        }
    }

    /// Sets the extra headers, written in order after `committer` and any `encoding`,
    /// each as `<field> <value>`; an empty run writes none, and is stored as C stores
    /// it, NULL with a length of 0. Borrowed for `'o`, run and strings alike.
    ///
    /// Field: git_commit_create_options.extra_headers
    /// Field: git_commit_create_options.extra_headers_len
    pub fn set_extra_headers(&mut self, headers: &'o [Header<'o>]) {
        let run: *const ffi::git_commit_header = if headers.is_empty() {
            ptr::null()
        } else {
            headers.as_ptr().cast()
        };
        let p = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to live options; the
        // fields are plain data, so the old values need no drop. `run` is NULL with a
        // length of 0, or `headers`' own pointer and length, a run of initialised
        // `git_commit_header`s (`Header` is transparent over it) whose strings are
        // non-NULL, as `extra_headers()` requires.
        unsafe {
            addr_of_mut!((*p).extra_headers).write(run);
            addr_of_mut!((*p).extra_headers_len).write(headers.len());
        }
    }

    /// Sets the signing callback, borrowed exclusively for `'o`; `None` signs nothing.
    /// libgit2 calls it once per commit created or amended with these options, after
    /// building the commit and before writing it, with the builder to add the
    /// signature header to (see [`SignatureCallback`]); an error fails the call,
    /// nothing written.
    ///
    /// The options store the callback's trampoline and, as its payload, the callback
    /// itself, which they borrow until they are dropped: it cannot be used, moved or
    /// dropped meanwhile, nor lent to other options, so each libgit2 call reaches it
    /// alone. A panic of its closure stops the commit with `GIT_EUSER` and is kept in the
    /// callback, which [`resume_panic`](SignatureCallback::resume_panic) resumes once the
    /// options are gone.
    ///
    /// ```compile_fail,E0499
    /// use libgit2::libgit2::core::types::StreamError;
    /// use libgit2::libgit2::object::commit::{CommitBuilderMut, CreateOptions, SignatureCallback};
    /// use libgit2::libgit2::repository::repository::RepositoryRef;
    ///
    /// let mut f = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &core::ffi::CStr| Ok::<(), StreamError>(());
    /// let mut signer = SignatureCallback::new(&mut f);
    /// let mut options = CreateOptions::new();
    /// options.as_mut().set_signer(Some(&mut signer));
    /// let mut other = CreateOptions::new();
    /// other.as_mut().set_signer(Some(&mut signer));
    /// drop((options, other));
    /// ```
    ///
    /// Field: git_commit_create_options.sign
    /// Field: git_commit_create_options.payload
    pub fn set_signer(&mut self, signer: Option<&'o mut SignatureCallback<'_>>) {
        let (callback, payload): (ffi::git_commit_signature_cb, *mut c_void) = match signer {
            Some(signer) => (Some(sign_with_closure), ptr::from_mut(signer).cast()),
            None => (None, ptr::null_mut()),
        };
        let p = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to live options; the
        // fields are plain data, so the old values need no drop. The payload is the
        // callback the options borrow exclusively for `'o`, for which the trampoline is
        // sound with libgit2's arguments.
        unsafe {
            addr_of_mut!((*p).sign).write(callback);
            addr_of_mut!((*p).payload).write(payload);
        }
    }
}

impl fmt::Debug for CreateOptionsMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// A `CreateOptions` is a `git_commit_create_options`, so C reads a value Rust holds in
// place.
const _: () = assert!(
    size_of::<CreateOptions<'static>>() == size_of::<ffi::git_commit_create_options>()
        && align_of::<CreateOptions<'static>>() == align_of::<ffi::git_commit_create_options>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::core::types::{ObjectType, SignatureView, Time};
    use crate::libgit2::object::object::Object;
    use crate::libgit2::oid::oid::{Oid, OidType};
    use crate::libgit2::repository::repository::{Repository, RepositoryBox};
    use crate::libgit2::util::errors::{ErrorClass, ErrorCode};
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use crate::libgit2::util::sys::errors::runtime_pinned;
    use core::ffi::{CStr, c_char, c_int, c_void};
    use core::mem::size_of;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

    /// The fixture repositories: `testrepo.git` (a branch at `a65fedf`), `mergedrepo`
    /// (merging `e280915` into `3a34580`) and `empty_bare.git` (an unborn `HEAD`).
    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git\0"
    );
    const MERGEDREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/mergedrepo/.gitted\0"
    );
    const EMPTY: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/empty_bare.git\0"
    );

    /// Opens the repository at the NUL-terminated `path`, read-only use only.
    fn open(path: &str) -> *mut ffi::git_repository {
        assert!(runtime_pinned());
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` receives the repository; `path` is NUL-terminated; no ceiling
        // directories.
        let rc = unsafe {
            ffi::git_repository_open_ext(
                &mut repo,
                path.as_ptr().cast(),
                ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                ptr::null(),
            )
        };
        assert_eq!(rc, 0);
        repo
    }

    /// The hex id of the live commit `commit`.
    fn hex(commit: CommitRef<'_, '_>) -> String {
        let mut out = [0u8; 41];
        // SAFETY: the handle's commit is live; its id is formatted into `out`, 41
        // writable bytes, NUL included.
        unsafe {
            ffi::git_oid_tostr(
                out.as_mut_ptr().cast(),
                out.len(),
                ffi::git_commit_id(commit.as_ptr()),
            )
        };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    fn ids(array: CommitArrayRef<'_, '_>) -> Vec<String> {
        array.iter().map(hex).collect()
    }

    /// `testrepo.git`, through the safe wrapper.
    fn open_testrepo() -> RepositoryBox {
        assert!(runtime_pinned());
        Repository::open(CStr::from_bytes_with_nul(TESTREPO.as_bytes()).unwrap()).unwrap()
    }

    fn sha1(hex: &CStr) -> Oid {
        Oid::from_string(hex, OidType::Sha1).unwrap()
    }

    /// `testrepo.git`'s `HEAD` commit, its parent and its tree.
    const HEAD: &CStr = c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
    const PARENT: &CStr = c"be3563ae3f795b2b4353bcce3a527ad0a4f7f644";
    const TREE: &CStr = c"944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";

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

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// libc's allocator, refusing everything while `REFUSING` is set.
    struct Refusing;

    // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
    unsafe impl Alloc for Refusing {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            if REFUSING.load(SeqCst) {
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
            if REFUSING.load(SeqCst) {
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
                ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
    }

    #[test]
    fn commit_layout_matches_c() {
        assert_eq!(size_of::<Commit<'_>>(), size_of::<ffi::git_commit>());
        assert_eq!(
            size_of::<CommitArc<'_>>(),
            size_of::<*mut ffi::git_commit>()
        );
        assert_eq!(
            size_of::<CommitRef<'_, '_>>(),
            size_of::<*const ffi::git_commit>()
        );
        assert_eq!(
            size_of::<CommitMut<'_, '_>>(),
            size_of::<*const ffi::git_commit>()
        );
        assert_eq!(size_of::<CommitFree>(), 0);
    }

    /// The commit handles' seams over a pointer nothing reads, so Miri runs them:
    /// `git_commit` is opaque, of size zero, so a dangling aligned pointer addresses all
    /// of it; the object view only casts it.
    #[test]
    fn commit_handles_without_c() {
        // SAFETY: NULL is refused without being read.
        assert!(unsafe { CommitRef::from_ptr(ptr::null()) }.is_none());
        // SAFETY: as above.
        assert!(unsafe { CommitMut::from_ptr(ptr::null_mut()) }.is_none());
        let p = NonNull::<ffi::git_commit>::dangling().as_ptr();
        // SAFETY: a zero-sized commit at an aligned pointer, never read or written, and
        // reached by this handle alone.
        let mut m = unsafe { CommitMut::from_ptr(p) }.unwrap();
        assert_eq!(m.as_mut_ptr(), p);
        assert_eq!(m.as_mut().as_mut_ptr(), p);
        let r = m.as_ref();
        assert_eq!(r.as_ptr(), p.cast_const());
        assert_eq!(r.as_object().as_ptr(), p.cast_const().cast());
        assert_eq!(format!("{r:?}"), "Commit { .. }");
        assert_eq!(format!("{m:?}"), "Commit { .. }");
    }

    /// An id of another type than the repository's is refused before libgit2 formats
    /// it out of bounds, at every length; the same bytes typed as the repository's find
    /// the commit. Corrects the C routines, which check nothing.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn commit_ids_of_another_type_are_refused_before_c() {
        let repo = open_testrepo();
        let repo = repo.as_ref();
        let head = sha1(HEAD);
        let mut wide = Oid::from_raw(&head.as_ref().id(), OidType::Sha256).unwrap();
        for kind in [OidType::Sha256.raw() as u8, 0, 9, u8::MAX] {
            wide.as_mut().set_raw_kind(kind);
            let mut results = vec![Commit::lookup(repo, wide.as_ref())];
            for len in [0, 3, 4, 7, 40, 64, usize::MAX] {
                results.push(Commit::lookup_prefix(repo, wide.as_ref(), len));
            }
            for result in results {
                let e = result.unwrap_err();
                assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
                assert_eq!(
                    e.message(),
                    c"specified object ID type does not match repository object ID type"
                );
            }
        }
        wide.as_mut().set_kind(OidType::Sha1);
        let found = Commit::lookup(repo, wide.as_ref()).unwrap();
        assert_eq!(hex(found.as_ref()), HEAD.to_str().unwrap());
        let found = Commit::lookup_prefix(repo, wide.as_ref(), 7).unwrap();
        assert_eq!(hex(found.as_ref()), HEAD.to_str().unwrap());
    }

    /// The policy driven by hand: the up-ref counts one more reference on the same
    /// pointer, which an adopted owner and the free each settle; `to_commit` takes one
    /// through a borrowed handle, and the object views share the commit's pointer.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn commit_policy_counts_references_on_one_commit() {
        let repo = open_testrepo();
        let commit = Commit::lookup(repo.as_ref(), sha1(HEAD).as_ref()).unwrap();
        let p = NonNull::new(commit.as_ptr()).unwrap();
        // SAFETY: `commit` holds a reference to the live commit at `p`.
        assert!(unsafe { CommitFree.c_up_ref(p) });
        // SAFETY: the reference just counted, given to the owner.
        let adopted = unsafe { CommitArc::from_raw(p.as_ptr()) }.unwrap();
        assert!(CommitArc::ptr_eq(&adopted, &commit));
        let raw = adopted.into_c();
        // SAFETY: `raw` is the reference `into_c` gave up, released once here.
        unsafe { CommitFree.c_drop(NonNull::new(raw).unwrap().cast()) };

        // SAFETY: `commit` keeps the commit alive while the handle lives.
        let borrowed = unsafe { CommitRef::from_ptr(commit.as_c_ptr()) }.unwrap();
        let copy = borrowed.to_commit();
        assert!(CommitArc::ptr_eq(&copy, &commit));
        assert_eq!(
            borrowed.as_object().as_ptr(),
            commit.as_c_ptr().cast_const().cast()
        );

        let object = Commit::into_object(copy);
        assert_eq!(object.as_c_ptr().cast(), commit.as_c_ptr());
        let viewed = CommitRef::from_object(object.as_ref()).unwrap();
        assert_eq!(viewed.as_ptr(), commit.as_c_ptr().cast_const());
        let back = Commit::from_object(object).unwrap();
        assert!(CommitArc::ptr_eq(&back, &commit));
        drop(commit);
        assert_eq!(hex(back.as_ref()), HEAD.to_str().unwrap());

        let tree = Object::lookup(repo.as_ref(), sha1(TREE).as_ref(), Some(ObjectType::Tree));
        let tree = tree.unwrap();
        assert!(CommitRef::from_object(tree.as_ref()).is_none());
        let tree = Commit::from_object(tree).unwrap_err();
        assert_eq!(
            Commit::lookup(repo.as_ref(), sha1(TREE).as_ref())
                .unwrap_err()
                .code(),
            Some(ErrorCode::NotFound)
        );
        drop(tree);
    }

    /// A commit's parents, first-parent ancestors and tree are owners of their own: one
    /// counted reference each (the very commit for generation 0), which outlives the
    /// commit it was derived from, and every refusal leaves the thread's error as libgit2
    /// sets it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn parents_ancestors_and_trees_derive_owned_references() {
        let repo = open_testrepo();
        let head = Commit::lookup(repo.as_ref(), sha1(HEAD).as_ref()).unwrap();
        let parent = head.as_ref().parent(0).unwrap();
        assert_eq!(hex(parent.as_ref()), PARENT.to_str().unwrap());
        let cached = Commit::lookup(repo.as_ref(), sha1(PARENT).as_ref()).unwrap();
        assert!(CommitArc::ptr_eq(&parent, &cached));
        let itself = head.as_ref().nth_gen_ancestor(0).unwrap();
        assert!(CommitArc::ptr_eq(&itself, &head));
        let first = head.as_ref().nth_gen_ancestor(1).unwrap();
        assert!(CommitArc::ptr_eq(&first, &parent));
        let tree = head.as_ref().tree().unwrap();
        assert_eq!(
            tree.as_ref().as_object().id().id()[..20],
            sha1(TREE).as_ref().id()[..20]
        );
        drop((head, itself, cached));
        // The merge parent's second parent, and the root five generations up.
        let second = parent.as_ref().parent(1).unwrap();
        assert_eq!(
            hex(second.as_ref()),
            "c47800c7266a2be04c571c04d5a6614691ea99bd"
        );
        let root = first.as_ref().nth_gen_ancestor(4).unwrap();
        assert_eq!(
            hex(root.as_ref()),
            "8496071c1b46c854b31185ea97743be6a8774479"
        );
        drop((parent, first));

        for (result, message) in [
            (second.as_ref().parent(1), "parent 1 does not exist"),
            (root.as_ref().parent(0), "parent 0 does not exist"),
            (root.as_ref().nth_gen_ancestor(1), "parent 0 does not exist"),
            (
                second.as_ref().nth_gen_ancestor(u32::MAX),
                "parent 0 does not exist",
            ),
            (
                root.as_ref().parent(u32::MAX),
                "parent 4294967295 does not exist",
            ),
        ] {
            let e = result.unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::NotFound));
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
            assert_eq!(e.message().to_str(), Ok(message));
        }
        assert!(root.as_ref().tree().is_ok());
        drop((root, second));
        assert!(!tree.as_ref().as_object().id().is_zero());
    }

    /// Commits moved into an array and taken out again: the same pointers, in order,
    /// each reference handed over once (LSan: a leak; ASan: a double release); the
    /// buffer is libgit2's, released by the dispose policy or by `take`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn commits_move_into_and_out_of_arrays() {
        let repo = open_testrepo();
        let head = Commit::lookup(repo.as_ref(), sha1(HEAD).as_ref()).unwrap();
        let parent = Commit::lookup(repo.as_ref(), sha1(PARENT).as_ref()).unwrap();
        let mut array =
            CommitArray::from_commits(vec![head.clone(), parent.clone(), head.clone()]).unwrap();
        let r = array.as_ref();
        let lent: Vec<_> = r.iter().map(|c| c.as_ptr().cast_mut()).collect();
        assert_eq!(lent, [head.as_c_ptr(), parent.as_c_ptr(), head.as_c_ptr()]);
        // SAFETY: a read of the array's own buffer pointer, through its shared handle.
        let buffer = unsafe { r.as_ptr().read() }.commits;
        assert!(!buffer.is_null());

        let taken = array.as_mut().take();
        assert!(array.as_ref().is_empty());
        assert_eq!(taken.len(), 3);
        assert!(CommitArc::ptr_eq(&taken[1], &parent));

        // Replacing releases what was held; an empty run leaves the empty array.
        let mut m = array.as_mut();
        m.set_commits(taken).unwrap();
        m.set_commits(vec![parent.clone()]).unwrap();
        assert_eq!(ids(m.as_ref()), [PARENT.to_str().unwrap()]);
        m.set_commits(Vec::new()).unwrap();
        // SAFETY: as above.
        assert!(unsafe { m.as_ref().as_ptr().read() }.commits.is_null());
        m.set_commits(vec![head]).unwrap();
        drop(parent);
        drop(array);
    }

    /// When the buffer cannot be allocated, the commits come back and the array keeps
    /// what it held; nothing is released twice or leaked.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_refused_buffer_hands_the_commits_back() {
        isolated(
            concat!(module_path!(), "::a_refused_buffer_hands_the_commits_back"),
            || {
                install_refusing();
                let repo = open_testrepo();
                let head = Commit::lookup(repo.as_ref(), sha1(HEAD).as_ref()).unwrap();
                let mut array = CommitArray::from_commits(vec![head.clone()]).unwrap();
                REFUSING.store(true, SeqCst);
                let refused = CommitArray::from_commits(vec![head.clone(), head.clone()]);
                let kept = array.as_mut().set_commits(vec![head.clone()]);
                let empty = CommitArray::from_commits(Vec::new());
                REFUSING.store(false, SeqCst);
                assert_eq!(refused.unwrap_err().len(), 2);
                assert_eq!(kept.unwrap_err().len(), 1);
                assert!(empty.unwrap().as_ref().is_empty());
                assert_eq!(ids(array.as_ref()), [HEAD.to_str().unwrap()]);
            },
        );
    }

    /// `GIT_COMMIT_CREATE_EXT_OPTIONS_INIT`, built in Rust so Miri can run the
    /// accessors; `options_new_matches_the_template` ties it to
    /// `git_commit_create_ext_options_init`.
    fn template<'o>() -> CreateExtOptions<'o> {
        CreateExtOptions(
            ffi::git_commit_create_ext_options {
                version: ffi::GIT_COMMIT_CREATE_EXT_OPTIONS_VERSION,
                ..Default::default()
            },
            PhantomData,
        )
    }

    fn raw(options: &CreateExtOptions<'_>) -> ffi::git_commit_create_ext_options {
        // SAFETY: a read of the live options through their shared handle's pointer.
        unsafe { options.as_ref().as_ptr().read() }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn options_new_matches_the_template() {
        let (new, template) = (raw(&CreateExtOptions::new()), raw(&template()));
        assert_eq!(new.version, template.version);
        assert_eq!(new.update_ref, template.update_ref);
        assert_eq!(new.message_encoding, template.message_encoding);
        assert_eq!(new.extra_headers, template.extra_headers);
        assert_eq!(new.extra_headers_len, template.extra_headers_len);
        assert!(new.sign.is_none() && template.sign.is_none());
        assert_eq!(new.payload, template.payload);
        assert_eq!(raw(&CreateExtOptions::default()).version, 1);
    }

    /// Every setter stores what its getter reads back, the strings' and the run's own
    /// pointers included, and an empty run is stored as C stores none.
    #[test]
    fn options_setters_round_trip() {
        let encoding = std::ffi::CString::new("ISO-8859-1").unwrap();
        let headers = [Header::new(c"a", c"1"), Header::new(c"b", c"2")];
        let mut never = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            unreachable!("only stored: the options reach no libgit2 call")
        };
        let mut signer = SignatureCallback::new(&mut never);
        let signer_at = ptr::from_mut(&mut signer).cast::<c_void>();
        let mut options = template();
        assert_eq!(
            format!("{options:?}"),
            "CreateExtOptions { version: 1, update_ref: None, message_encoding: None, \
             extra_headers: [], sign_callback: false }"
        );
        let mut m = options.as_mut();
        m.set_update_ref(Some(c"refs/heads/main"));
        m.set_message_encoding(Some(&encoding));
        m.set_extra_headers(&headers);
        m.set_signer(Some(&mut signer));
        {
            let mut inner = m.as_mut();
            inner.set_update_ref(Some(c"HEAD"));
            assert_eq!(inner.as_ref().update_ref(), Some(c"HEAD"));
        }
        let r = m.as_ref();
        assert_eq!(r.version(), 1);
        assert_eq!(r.message_encoding().unwrap().as_ptr(), encoding.as_ptr());
        let run = r.extra_headers();
        assert_eq!(run.len(), 2);
        assert_eq!(run.as_ptr(), headers.as_ptr().cast());
        assert_eq!(run.get(1).unwrap().value(), c"2");
        let (sign, data) = r.sign_callback();
        assert!(sign.is_some());
        assert_eq!(data, signer_at);
        assert_eq!(
            format!("{m:?}"),
            "CreateExtOptions { version: 1, update_ref: Some(\"HEAD\"), message_encoding: \
             Some(\"ISO-8859-1\"), extra_headers: [Header { field: \"a\", value: \"1\" }, \
             Header { field: \"b\", value: \"2\" }], sign_callback: true }"
        );

        m.set_extra_headers(&headers[..0]);
        m.set_update_ref(None);
        m.set_message_encoding(None);
        m.set_signer(None);
        let back = raw(&options);
        assert!(back.extra_headers.is_null() && back.extra_headers_len == 0);
        assert!(back.update_ref.is_null() && back.message_encoding.is_null());
        assert!(back.sign.is_none() && back.payload.is_null());
        assert!(options.as_ref().extra_headers().is_empty());
    }

    /// Options C code filled, borrowed through the crate-internal seams: a NULL run with
    /// a stale length reads as none, a C array of headers reads in place.
    #[test]
    fn options_handles_over_c_storage() {
        // SAFETY: NULL is refused.
        assert!(unsafe { CreateExtOptionsRef::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: as above.
        assert!(unsafe { CreateExtOptionsMut::from_ptr(ptr::null_mut()) }.is_none());
        let c_headers = [ffi::git_commit_header {
            field: c"f".as_ptr(),
            value: c"v".as_ptr(),
        }];
        let mut c = ffi::git_commit_create_ext_options {
            version: 1,
            extra_headers: ptr::null(),
            extra_headers_len: 3,
            ..Default::default()
        };
        // SAFETY: `c` is live, with no headers and no strings, for the handle's use.
        let r = unsafe { CreateExtOptionsRef::from_ptr(&mut c) }.unwrap();
        assert!(r.extra_headers().is_empty());
        c.extra_headers = c_headers.as_ptr();
        c.extra_headers_len = 1;
        // SAFETY: `c` now addresses one live header of `'static` strings, and nothing
        // else reaches `c` while the handle lives.
        let mut m = unsafe { CreateExtOptionsMut::<'_, 'static>::from_ptr(&mut c) }.unwrap();
        assert_eq!(m.as_ref().extra_headers().get(0).unwrap().field(), c"f");
        m.set_message_encoding(Some(c"UTF-8"));
        assert_eq!(c.message_encoding, c"UTF-8".as_ptr());
    }

    #[test]
    fn header_layout_matches_c() {
        assert_eq!(size_of::<Header<'_>>(), size_of::<ffi::git_commit_header>());
        assert_eq!(
            size_of::<HeaderRef<'_, '_>>(),
            size_of::<*const ffi::git_commit_header>()
        );
        assert_eq!(
            size_of::<HeaderMut<'_, '_>>(),
            size_of::<*const ffi::git_commit_header>()
        );
    }

    /// `new` stores the strings' own pointers, which the getters read back and C sees
    /// in place, through the header and through the first element of an array.
    #[test]
    fn header_stores_the_borrowed_pointers() {
        let (field, value) = (c"line_one", c"First extra header");
        let header = Header::new(field, value);
        let r = header.as_ref();
        assert_eq!(r.field(), field);
        assert_eq!(r.value(), value);
        assert_eq!(r.field().as_ptr(), field.as_ptr());
        assert_eq!(r.value().as_ptr(), value.as_ptr());
        // SAFETY: the handle's pointer addresses `header`, live and unwritten here.
        let raw = unsafe { r.as_ptr().read() };
        assert_eq!((raw.field, raw.value), (field.as_ptr(), value.as_ptr()));

        let headers = [header, Header::new(c"line_two", c"Second extra header")];
        let array: *const ffi::git_commit_header = headers.as_ptr().cast();
        // SAFETY: `Header` is transparent over `git_commit_header`, so `array` addresses
        // two live C structs, the array's elements, read only here.
        let second = unsafe { array.add(1).read() };
        // SAFETY: the second header stores a `&'static CStr`'s pointer.
        assert_eq!(unsafe { CStr::from_ptr(second.field) }, c"line_two");
    }

    /// The setters replace one string each, through a handle and a reborrow; a copy is
    /// independent of the header it came from.
    #[test]
    fn header_setters_reborrows_and_copies() {
        let owned_field = std::ffi::CString::new("owned").unwrap();
        let mut header = Header::new(c"a", c"b");
        {
            let mut m = header.as_mut();
            m.set_field(&owned_field);
            {
                let mut inner = m.as_mut();
                inner.set_value(c"multi\nline");
                assert_eq!(inner.as_ref().field(), c"owned");
            }
            assert_eq!(m.as_ref().value(), c"multi\nline");
            assert_eq!(
                format!("{m:?}"),
                r#"Header { field: "owned", value: "multi\nline" }"#
            );
        }
        let copy = header.as_ref().to_header();
        let mut second = header;
        second.as_mut().set_value(c"");
        assert_eq!(header.as_ref().value(), c"multi\nline");
        assert_eq!(second.as_ref().value(), c"");
        assert_eq!(copy, header);
        assert_ne!(copy, second);
        assert_eq!(format!("{header:?}"), format!("{:?}", header.as_ref()));
    }

    /// Equality compares the strings, not their addresses.
    #[test]
    fn header_equality_is_by_content() {
        let field = std::ffi::CString::new("line_one").unwrap();
        let value = std::ffi::CString::new("x").unwrap();
        assert_eq!(Header::new(&field, &value), Header::new(c"line_one", c"x"));
        assert_ne!(Header::new(&field, &value), Header::new(c"line_one", c"y"));
        assert_ne!(Header::new(&field, &value), Header::new(c"line_two", c"x"));
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(
            size_of::<CommitArray<'_>>(),
            size_of::<ffi::git_commitarray>()
        );
        assert_eq!(
            size_of::<CommitArrayVal<'_>>(),
            size_of::<ffi::git_commitarray>()
        );
        assert_eq!(
            size_of::<CommitArrayRef<'_, '_>>(),
            size_of::<*const ffi::git_commitarray>()
        );
        assert_eq!(
            size_of::<CommitArrayMut<'_, '_>>(),
            size_of::<*const ffi::git_commitarray>()
        );
        assert_eq!(size_of::<CommitArrayDispose>(), 0);
    }

    /// The empty array's accessors over a bare value and an owner, without libgit2, so
    /// Miri runs them; nothing here reaches C, `clear` and the drops included.
    #[test]
    fn empty_array_accessors_without_c() {
        let mut array = CommitArray::new();
        let mut m = array.as_mut();
        assert_eq!(m.as_ref().len(), 0);
        assert!(m.as_ref().is_empty());
        assert_eq!(m.as_ref().iter().len(), 0);
        assert!(m.as_ref().get(0).is_none());
        m.clear();
        assert!(m.take().is_empty());
        assert!(m.set_commits(Vec::new()).is_ok());
        assert_eq!(format!("{m:?}"), "[]");
        assert_eq!(format!("{:?}", array.as_ref()), "[]");
        let mut bare = array.into_inner();
        {
            let mut m = bare.as_mut();
            let mut inner = m.as_mut();
            inner.clear();
            assert!(inner.take().is_empty());
            assert!(m.as_ref().is_empty());
        }
        let raw = bare.into_raw();
        assert!(raw.commits.is_null() && raw.count == 0);
        // SAFETY: a NULL `commits` carries no requirement.
        let adopted = unsafe { CommitArray::from_raw(raw) };
        assert!(adopted.as_ref().is_empty());
        // SAFETY: NULL is refused without being read.
        assert!(unsafe { CommitArrayRef::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: as above.
        assert!(unsafe { CommitArrayMut::from_ptr(ptr::null_mut()) }.is_none());
    }

    /// Without a buffer the array is empty and owns nothing, whatever `count` says:
    /// every accessor agrees, `clear` resets the count, and neither it nor the dispose
    /// policy calls libgit2 (Miri aborts on any foreign call).
    #[test]
    fn null_buffer_is_empty_and_disposes_without_c() {
        let mut broken = ffi::git_commitarray {
            commits: ptr::null(),
            count: 3,
        };
        // SAFETY: `broken` is live and holds no buffer; nothing is released.
        unsafe {
            CommitArrayDispose.c_dispose(NonNull::from(&mut broken).cast::<CommitArray<'_>>())
        };
        assert_eq!((broken.commits, broken.count), (ptr::null(), 3));
        // SAFETY: `broken` is live for the handle's use; nothing else reaches it.
        let mut m = unsafe { CommitArrayMut::from_ptr(&mut broken) }.unwrap();
        assert_eq!(m.as_ref().len(), 0);
        assert!(m.as_ref().get(0).is_none());
        assert_eq!(m.as_ref().iter().count(), 0);
        assert_eq!(format!("{m:?}"), "[]");
        m.clear();
        assert_eq!((broken.commits, broken.count), (ptr::null(), 0));
        broken.count = 2;
        // SAFETY: as above.
        let mut m = unsafe { CommitArrayMut::from_ptr(&mut broken) }.unwrap();
        assert!(m.take().is_empty());
        assert_eq!((broken.commits, broken.count), (ptr::null(), 0));

        // SAFETY: a NULL `commits` carries no requirement.
        let adopted: CommitArrayVal<'_> = unsafe {
            CommitArray::from_raw(ffi::git_commitarray {
                commits: ptr::null(),
                count: usize::MAX,
            })
        };
        assert_eq!(adopted.as_ref().len(), 0);
        drop(adopted);
    }

    /// The handles' projections over a buffer Rust allocated, so Miri checks them: the
    /// commit handles only carry the pointers, never dereferenced, and the buffer leaves
    /// through `into_raw`, so nothing reaches libgit2's dispose.
    #[test]
    fn handles_over_rust_storage() {
        let mut slots = [0u64; 3];
        let fake: Vec<*mut ffi::git_commit> =
            slots.iter_mut().map(|s| ptr::from_mut(s).cast()).collect();
        let raw = ffi::git_commitarray {
            commits: fake.as_ptr(),
            count: 3,
        };
        // SAFETY: never disposed: the value leaves through `into_raw` below, so the
        // test's buffer and fake commits never reach `git_commitarray_dispose`.
        let array: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(raw) };
        let r = array.as_ref();
        assert_eq!(r.len(), 3);
        assert!(!r.is_empty());
        let lent: Vec<_> = r.iter().map(|c| c.as_ptr().cast_mut()).collect();
        assert_eq!(lent, fake);
        let back_to_front: Vec<_> = r.iter().rev().map(|c| c.as_ptr().cast_mut()).collect();
        assert_eq!(back_to_front, [fake[2], fake[1], fake[0]]);
        assert_eq!(r.iter().len(), 3);
        assert_eq!(r.get(2).unwrap().as_ptr(), fake[2].cast_const());
        assert!(r.get(3).is_none() && r.get(usize::MAX).is_none());
        assert_eq!(
            format!("{r:?}"),
            "[Commit { .. }, Commit { .. }, Commit { .. }]"
        );
        let back = array.into_inner().into_raw();
        assert_eq!((back.commits, back.count), (fake.as_ptr(), 3));
    }

    /// libgit2 fills an owner's storage in place, as the `git_repository_commit_parents`
    /// wrapper will through the crate-internal seam: one parent for a branch, two for
    /// a merge in progress, none for an unborn `HEAD`. Each result is read through the
    /// handles, cleared before the next fill (libgit2 overwrites without releasing),
    /// and released after its repositories are freed (LSan flags a leak, ASan a double
    /// release or a use after free).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn c_fills_an_owner_in_place() {
        let (testrepo, merged, empty) = (open(TESTREPO), open(MERGEDREPO), open(EMPTY));
        let mut array = CommitArray::new();
        let fill = |array: &mut CommitArrayVal<'_>, repo| {
            let out = array.as_mut().as_mut_ptr();
            // SAFETY: `out` is the owner's live storage, reached through its exclusive
            // handle and used for the call alone; it holds the empty array, so nothing
            // is overwritten unreleased; `repo` is open. libgit2 leaves `{NULL, 0}` or a
            // buffer of commit references it owns, which keeps the handles' invariant.
            let rc = unsafe { ffi::git_repository_commit_parents(out, repo) };
            assert_eq!(rc, 0);
        };

        fill(&mut array, testrepo);
        assert_eq!(
            ids(array.as_ref()),
            ["a65fedf39aefe402d3bb6e24df4d4f5fe4547750"]
        );
        array.as_mut().clear();
        assert!(array.as_ref().is_empty());

        fill(&mut array, merged);
        assert_eq!(
            ids(array.as_ref()),
            [
                "3a34580a35add43a4cf361e8e9a30060a905c876",
                "e2809157a7766f272e4cfe26e61ef2678a5357ff"
            ]
        );
        let mut second = CommitArray::new();
        fill(&mut second, testrepo);
        array.as_mut().clear();

        fill(&mut array, empty);
        assert!(array.as_ref().is_empty());
        assert!(array.as_ref().get(0).is_none());

        for repo in [testrepo, merged, empty] {
            // SAFETY: each repository is open and freed once; the arrays' commits
            // outlive it, `git_commit_free` reading only the commit.
            unsafe { ffi::git_repository_free(repo) };
        }
        assert_eq!(
            ids(second.as_ref()),
            ["a65fedf39aefe402d3bb6e24df4d4f5fe4547750"]
        );
        drop(second);
        drop(array);
    }

    /// The dispose policy driven by hand, and `into_raw` handing an array back to C's
    /// `git_commitarray_dispose`: each releases every commit and the buffer once.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn dispose_policy_and_c_release_the_commits() {
        let repo = open(MERGEDREPO);
        let mut raw = ffi::git_commitarray::default();
        // SAFETY: `raw` is a zeroed array, `repo` open.
        let rc = unsafe { ffi::git_repository_commit_parents(&mut raw, repo) };
        assert_eq!(rc, 0);
        // SAFETY: `raw` is what libgit2 filled: two commit references in a libgit2
        // buffer, given up here.
        let mut bare = unsafe { CommitArray::<'_>::from_raw(raw) }.into_inner();
        assert_eq!(bare.as_ref().len(), 2);
        // SAFETY: `bare` is a live array Rust owns, disposed once; C zeroes it.
        unsafe { CommitArrayDispose.c_dispose(NonNull::from(&mut bare)) };
        let zeroed = bare.into_raw();
        assert!(zeroed.commits.is_null() && zeroed.count == 0);

        let mut raw = ffi::git_commitarray::default();
        // SAFETY: as above.
        let rc = unsafe { ffi::git_repository_commit_parents(&mut raw, repo) };
        assert_eq!(rc, 0);
        // SAFETY: as above.
        let adopted: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(raw) };
        let mut back = adopted.into_inner().into_raw();
        assert_eq!(back.count, 2);
        // SAFETY: `back` owns its commits and buffer again, released once by C.
        unsafe { ffi::git_commitarray_dispose(&mut back) };
        // SAFETY: `repo` is open and freed once.
        unsafe { ffi::git_repository_free(repo) };
    }

    /// The builder's handles over Rust storage, so Miri checks them: the builder is
    /// opaque, so they never read through the pointer, only carry it, reborrow and print.
    #[test]
    fn commit_builder_handles_without_c() {
        assert_eq!(
            size_of::<CommitBuilder>(),
            size_of::<ffi::git_commitbuilder>()
        );
        assert_eq!(
            size_of::<CommitBuilderRef<'_>>(),
            size_of::<*const ffi::git_commitbuilder>()
        );
        assert_eq!(
            size_of::<Option<CommitBuilderMut<'_>>>(),
            size_of::<*mut ffi::git_commitbuilder>()
        );
        let mut storage = 0u64;
        let raw: *mut ffi::git_commitbuilder = ptr::from_mut(&mut storage).cast();
        // SAFETY: NULL is refused.
        assert!(unsafe { CommitBuilderMut::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: NULL is refused.
        assert!(unsafe { CommitBuilderRef::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: `raw` addresses live storage nothing else reaches while the handle
        // lives; the opaque builder is never read through it.
        let mut m = unsafe { CommitBuilderMut::from_ptr(raw) }.unwrap();
        assert_eq!(m.as_mut_ptr(), raw);
        assert_eq!(m.as_ref().as_ptr(), raw.cast_const());
        let mut inner = m.as_mut();
        assert_eq!(inner.as_mut_ptr(), raw);
        assert_eq!(format!("{inner:?}"), "CommitBuilder { .. }");
        let shared = m.as_ref();
        let copy = shared;
        assert_eq!(format!("{shared:?}"), format!("{copy:?}"));
        assert_eq!(format!("{m:?}"), "CommitBuilder { .. }");
        assert_eq!(storage, 0);
    }

    /// What the signing callback below saw and did, through its payload.
    #[derive(Default)]
    struct Signing {
        builder: usize,
        content: String,
        add_header: Option<c_int>,
    }

    /// A signing callback that reaches the builder only through a `CommitBuilderMut`,
    /// as the `git_commit_signature_cb` wrapper will, and adds the signature header
    /// through the handle's pointer.
    unsafe extern "C" fn sign(
        builder: *mut ffi::git_commitbuilder,
        _repo: *mut ffi::git_repository,
        content: *const c_char,
        payload: *mut c_void,
    ) -> c_int {
        // SAFETY: the payload is the test's live `Signing`, reached by nothing else
        // during the call.
        let state = unsafe { &mut *payload.cast::<Signing>() };
        // SAFETY: libgit2 lends the live builder for the call, exclusively; the handle
        // is dropped before the callback returns.
        let mut handle = unsafe { CommitBuilderMut::from_ptr(builder) }.unwrap();
        state.builder = handle.as_mut_ptr() as usize;
        // SAFETY: the commit's contents, NUL-terminated, live for the call.
        state.content = unsafe { CStr::from_ptr(content) }
            .to_str()
            .unwrap()
            .to_owned();
        let mut reborrowed = handle.as_mut();
        // SAFETY: the builder's pointer from its exclusive handle, and NUL-terminated
        // field and value, read for the call.
        let rc = unsafe {
            ffi::git_commitbuilder_add_header(
                reborrowed.as_mut_ptr(),
                c"gpgsig".as_ptr(),
                c"-----BEGIN SIGNATURE-----\nsigned\n-----END SIGNATURE-----".as_ptr(),
            )
        };
        state.add_header = Some(rc);
        0
    }

    /// libgit2 lends its builder to the signing callback, whose handle carries libgit2's
    /// own pointer back to `git_commitbuilder_add_header`: the signature lands in the
    /// written commit (ASan flags any access through a wrong pointer).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn commit_builder_lent_to_the_signing_callback() {
        assert!(runtime_pinned());
        let dir = std::env::temp_dir().join(format!(
            "crustify-commitbuilder-unit-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
        let mut repo = ptr::null_mut();
        // SAFETY: a writable slot and a NUL-terminated path; bare.
        let rc = unsafe { ffi::git_repository_init(&mut repo, path.as_ptr(), 1) };
        assert_eq!(rc, 0);

        // An empty tree, an identity and the signing options.
        let mut builder = ptr::null_mut();
        let mut tree_id = ffi::git_oid::default();
        let mut tree = ptr::null_mut();
        let mut who = ptr::null_mut();
        // SAFETY: writable slots, the live repository and NUL-terminated strings; each
        // object is freed once below.
        unsafe {
            assert_eq!(ffi::git_treebuilder_new(&mut builder, repo, ptr::null()), 0);
            assert_eq!(ffi::git_treebuilder_write(&mut tree_id, builder), 0);
            ffi::git_treebuilder_free(builder);
            assert_eq!(ffi::git_tree_lookup(&mut tree, repo, &tree_id), 0);
            assert_eq!(
                ffi::git_signature_new(
                    &mut who,
                    c"A U Thor".as_ptr(),
                    c"a@example.com".as_ptr(),
                    1_700_000_000,
                    0
                ),
                0
            );
        }
        let mut state = Signing::default();
        let opts = ffi::git_commit_create_ext_options {
            version: 1,
            sign: Some(sign),
            payload: ptr::from_mut(&mut state).cast(),
            ..Default::default()
        };
        let mut id = ffi::git_oid::default();
        // SAFETY: a writable id; the live repository, identity and tree (of that
        // repository); no parents; options whose callback and payload outlive the call.
        let rc = unsafe {
            ffi::git_commit_create_ext(
                &mut id,
                repo,
                who,
                who,
                c"signed commit\n".as_ptr(),
                tree,
                0,
                ptr::null(),
                &opts,
            )
        };
        assert_eq!(rc, 0);
        assert_ne!(state.builder, 0);
        assert_eq!(state.add_header, Some(0));
        assert!(state.content.starts_with("tree "));
        assert!(state.content.ends_with("\nsigned commit\n"));

        let mut commit = ptr::null_mut();
        let mut field = ffi::git_buf::default();
        // SAFETY: a writable slot, the live repository and the written id; then the live
        // commit, a zeroed buffer and a NUL-terminated name; all released once.
        unsafe {
            assert_eq!(ffi::git_commit_lookup(&mut commit, repo, &id), 0);
            assert_eq!(
                ffi::git_commit_header_field(&mut field, commit, c"gpgsig".as_ptr()),
                0
            );
            let value = CStr::from_ptr(field.ptr).to_str().unwrap().to_owned();
            assert_eq!(
                value,
                "-----BEGIN SIGNATURE-----\nsigned\n-----END SIGNATURE-----"
            );
            ffi::git_buf_dispose(&mut field);
            ffi::git_commit_free(commit);
            ffi::git_tree_free(tree);
            ffi::git_signature_free(who);
            ffi::git_repository_free(repo);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fresh bare repository holding the empty tree, and an identity, for writing
    /// commits; removed when dropped.
    struct Writer {
        dir: std::path::PathBuf,
        repo: *mut ffi::git_repository,
        tree: *mut ffi::git_tree,
        who: *mut ffi::git_signature,
    }

    impl Writer {
        fn new(tag: &str) -> Self {
            assert!(runtime_pinned());
            let dir = std::env::temp_dir()
                .join(format!("crustify-commit-sign-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
            let (mut repo, mut builder, mut tree, mut who) = (
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
            let mut tree_id = ffi::git_oid::default();
            // SAFETY: writable slots, NUL-terminated strings, and the live repository;
            // the builder is freed once, the rest when the writer drops.
            unsafe {
                assert_eq!(ffi::git_repository_init(&mut repo, path.as_ptr(), 1), 0);
                assert_eq!(ffi::git_treebuilder_new(&mut builder, repo, ptr::null()), 0);
                assert_eq!(ffi::git_treebuilder_write(&mut tree_id, builder), 0);
                ffi::git_treebuilder_free(builder);
                assert_eq!(ffi::git_tree_lookup(&mut tree, repo, &tree_id), 0);
                let rc = ffi::git_signature_new(
                    &mut who,
                    c"A U Thor".as_ptr(),
                    c"a@example.com".as_ptr(),
                    1_700_000_000,
                    0,
                );
                assert_eq!(rc, 0);
            }
            Self {
                dir,
                repo,
                tree,
                who,
            }
        }

        /// `git_commit_create_ext` with `message`, signing through `sign` and
        /// `payload`: the new commit's id, or the code and the thread's error.
        fn create(
            &self,
            message: &CStr,
            sign: ffi::git_commit_signature_cb,
            payload: *mut c_void,
        ) -> Result<ffi::git_oid, (c_int, ErrorBuf)> {
            let opts = ffi::git_commit_create_ext_options {
                version: 1,
                sign,
                payload,
                ..Default::default()
            };
            let mut id = ffi::git_oid::default();
            // SAFETY: a writable id; the live repository, identity and tree (of that
            // repository); no parents; options whose callback and payload are live for
            // the call.
            let rc = unsafe {
                ffi::git_commit_create_ext(
                    &mut id,
                    self.repo,
                    self.who,
                    self.who,
                    message.as_ptr(),
                    self.tree,
                    0,
                    ptr::null(),
                    &opts,
                )
            };
            if rc < 0 { Err((rc, last())) } else { Ok(id) }
        }

        /// Writes a commit signing through `callback`, as the wrapper of
        /// `git_commit_create_ext` will.
        fn create_signed(
            &self,
            message: &CStr,
            callback: &mut SignatureCallback<'_>,
        ) -> Result<ffi::git_oid, (c_int, ErrorBuf)> {
            callback.sign_with(|sign, payload| self.create(message, sign, payload))
        }

        /// The raw contents of the commit `id`.
        fn raw(&self, id: &ffi::git_oid) -> Vec<u8> {
            let (mut odb, mut obj) = (ptr::null_mut(), ptr::null_mut());
            // SAFETY: writable slots and the live repository and id; the object's data
            // is `size` bytes, copied before the object and the odb are freed once.
            unsafe {
                assert_eq!(ffi::git_repository_odb(&mut odb, self.repo), 0);
                assert_eq!(ffi::git_odb_read(&mut obj, odb, id), 0);
                let data = core::slice::from_raw_parts(
                    ffi::git_odb_object_data(obj).cast::<u8>(),
                    ffi::git_odb_object_size(obj),
                )
                .to_vec();
                ffi::git_odb_object_free(obj);
                ffi::git_odb_free(odb);
                data
            }
        }

        /// The loose objects written besides the empty tree: the commits.
        fn commits(&self) -> usize {
            let objects = self.dir.join("objects");
            let loose: usize = std::fs::read_dir(&objects)
                .unwrap()
                .map(|e| e.unwrap())
                .filter(|e| e.file_name().len() == 2)
                .map(|e| std::fs::read_dir(e.path()).unwrap().count())
                .sum();
            loose - 1
        }

        fn repository(&self) -> RepositoryRef<'_> {
            // SAFETY: the writer's live repository, used on this thread while the
            // handle borrows the writer.
            unsafe { RepositoryRef::from_ptr(self.repo) }.unwrap()
        }
    }

    impl Drop for Writer {
        fn drop(&mut self) {
            // SAFETY: each is live and freed once, the repository last.
            unsafe {
                ffi::git_signature_free(self.who);
                ffi::git_tree_free(self.tree);
                ffi::git_repository_free(self.repo);
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const SIGNATURE: &CStr = c"-----BEGIN SIGNATURE-----\nsigned\n-----END SIGNATURE-----";

    /// The closure is given the builder, the repository and the contents to sign, and
    /// adds the signature (then a second header) through the builder; reading the
    /// contents after the first header was added is sound, since they are a copy (ASan
    /// flags a read of the buffer libgit2 freed). The signature lands in the commit,
    /// where `extract_signature` finds it with the contents it signs.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signing_callback_adds_headers() {
        let w = Writer::new("adds");
        let mut seen = Vec::new();
        let mut sign =
            |mut builder: CommitBuilderMut<'_>, repo: RepositoryRef<'_>, content: &CStr| {
                assert_eq!(repo.as_ptr(), w.repo.cast_const());
                builder
                    .add_header(c"gpgsig", SIGNATURE)
                    .map_err(StreamError::from)?;
                seen.push(content.to_bytes().to_vec());
                builder
                    .as_mut()
                    .add_header(c"x-second", c"two")
                    .map_err(StreamError::from)?;
                seen.push(content.to_bytes().to_vec());
                Ok(())
            };
        let mut callback = SignatureCallback::new(&mut sign);
        assert!(format!("{callback:?}").starts_with("SignatureCallback { panicked: false"));
        let id = w.create_signed(c"signed\n", &mut callback).unwrap();
        drop(callback);
        let raw = String::from_utf8(w.raw(&id)).unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], seen[1]);
        let unsigned = String::from_utf8(seen.pop().unwrap()).unwrap();
        assert!(unsigned.starts_with("tree ") && unsigned.ends_with("\n\nsigned\n"));
        let (headers, message) = unsigned.split_once("\n\n").unwrap();
        assert_eq!(
            raw,
            format!(
                "{headers}\ngpgsig -----BEGIN SIGNATURE-----\n signed\n -----END SIGNATURE-----\nx-second two\n\n{message}"
            )
        );

        let oid = Oid::from_raw(&id.id[..20], OidType::Sha1).unwrap();
        let (signature, signed) = extract_signature(w.repository(), oid.as_ref(), None).unwrap();
        assert_eq!(signature.as_ref().as_bytes(), SIGNATURE.to_bytes());
        assert_eq!(
            signed.as_ref().as_bytes(),
            format!("{headers}\nx-second two\n\n{message}").as_bytes()
        );
    }

    /// The trampoline writes the same commit a C callback adding the same header does.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signing_callback_matches_a_c_callback() {
        unsafe extern "C" fn c_sign(
            builder: *mut ffi::git_commitbuilder,
            _repo: *mut ffi::git_repository,
            _content: *const c_char,
            _payload: *mut c_void,
        ) -> c_int {
            // SAFETY: the builder libgit2 lends for the call; NUL-terminated strings.
            unsafe {
                ffi::git_commitbuilder_add_header(builder, c"gpgsig".as_ptr(), SIGNATURE.as_ptr())
            }
        }
        let (c, rust) = (Writer::new("c-side"), Writer::new("rust-side"));
        let c_id = c.create(c"same\n", Some(c_sign), ptr::null_mut()).unwrap();
        let mut sign = |mut builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            builder
                .add_header(c"gpgsig", SIGNATURE)
                .map_err(StreamError::from)
        };
        let rust_id = rust
            .create_signed(c"same\n", &mut SignatureCallback::new(&mut sign))
            .unwrap();
        assert_eq!(c_id.id, rust_id.id);
        assert_eq!(c.raw(&c_id), rust.raw(&rust_id));
        // Not signing at all is `Ok` without a header: the unsigned commit.
        let unsigned = c.create(c"same\n", None, ptr::null_mut()).unwrap();
        let mut skip = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| Ok(());
        let skipped = rust
            .create_signed(c"same\n", &mut SignatureCallback::new(&mut skip))
            .unwrap();
        assert_eq!(unsigned.id, skipped.id);
    }

    /// An error stops the commit before it is written: the routine returns the error's
    /// code, with its class and message as the thread's error, whatever the code
    /// (`GIT_PASSTHROUGH` too, which libgit2 reads as a failure).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signing_errors_stop_the_commit() {
        let w = Writer::new("errors");
        for code in [ErrorCode::Generic, ErrorCode::User, ErrorCode::Passthrough] {
            let mut fail = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
                Err(StreamError::new(ErrorClass::Callback, c"no key").with_code(code))
            };
            let (rc, error) = w
                .create_signed(c"refused\n", &mut SignatureCallback::new(&mut fail))
                .unwrap_err();
            assert_eq!(rc, code.raw());
            assert_eq!(error.class(), Some(ErrorClass::Callback));
            assert_eq!(error.message(), c"no key");
        }
        assert_eq!(w.commits(), 0);
    }

    /// A panic in the closure stops the commit with `GIT_EUSER` and resumes once
    /// libgit2 has returned, with nothing written; the callback is usable again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signing_panics_resume_after_c() {
        let w = Writer::new("panics");
        let mut calls = 0;
        let mut boom = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            calls += 1;
            if calls == 1 {
                panic!("signing failed");
            }
            Ok(())
        };
        let mut callback = SignatureCallback::new(&mut boom);
        let mut returned = None;
        let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
            callback.sign_with(|sign, payload| {
                let result = w.create(c"panicked\n", sign, payload);
                returned = Some(result.as_ref().map_err(|(rc, _)| *rc).err());
                result
            })
        }));
        let panic = caught.unwrap_err();
        assert_eq!(panic.downcast_ref::<&str>(), Some(&"signing failed"));
        assert_eq!(returned, Some(Some(ErrorCode::User.raw())));
        assert_eq!(w.commits(), 0);
        assert!(w.create_signed(c"again\n", &mut callback).is_ok());
        assert_eq!(w.commits(), 1);
    }

    /// An empty header field name is refused before libgit2 reads anything: libgit2's
    /// scan for it never gets past the commit's first line. A `None` field is `gpgsig`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn empty_signature_fields_are_refused_before_c() {
        let w = Writer::new("empty-field");
        let mut sign = |mut builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            builder
                .add_header(c"gpgsig", c"sig")
                .map_err(StreamError::from)
        };
        let id = w
            .create_signed(c"m\n", &mut SignatureCallback::new(&mut sign))
            .unwrap();
        let oid = Oid::from_raw(&id.id[..20], OidType::Sha1).unwrap();
        let e = extract_signature(w.repository(), oid.as_ref(), Some(c"")).unwrap_err();
        assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
        assert_eq!(e.message(), c"the signature header field name is empty");
        let (signature, _) = extract_signature(w.repository(), oid.as_ref(), None).unwrap();
        assert_eq!(signature.as_ref().as_bytes(), b"sig");
    }

    /// A closure may write another commit, signed through its own callback, from inside
    /// the call: each trampoline reaches its own payload, and both commits are written.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signing_callbacks_reenter() {
        let w = Writer::new("reenter");
        let mut inner_calls = 0;
        let mut outer = |mut builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            let mut inner = |mut b: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
                inner_calls += 1;
                b.add_header(c"gpgsig", c"inner").map_err(StreamError::from)
            };
            let nested = w
                .create_signed(c"nested\n", &mut SignatureCallback::new(&mut inner))
                .map_err(|(_, e)| StreamError::from(e))?;
            let hex: String = nested.id[..20].iter().map(|b| format!("{b:02x}")).collect();
            let value = std::ffi::CString::new(hex).unwrap();
            builder
                .add_header(c"x-nested", &value)
                .map_err(StreamError::from)
        };
        let id = w
            .create_signed(c"outer\n", &mut SignatureCallback::new(&mut outer))
            .unwrap();
        assert_eq!(inner_calls, 1);
        assert_eq!(w.commits(), 2);
        let raw = String::from_utf8(w.raw(&id)).unwrap();
        assert!(raw.contains("\nx-nested "), "{raw}");
    }

    /// The trampoline driven by hand, as libgit2 never drives it: a NULL argument stops
    /// the commit without calling the closure, and so does a call after the closure
    /// panicked, until the panic resumes when the C call returns. The handles the
    /// closure would get are never read here (Miri runs it).
    #[test]
    fn trampoline_refuses_null_arguments_and_calls_after_a_panic() {
        let mut calls = 0;
        let mut boom = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            calls += 1;
            panic!("first call");
        };
        let mut callback = SignatureCallback::new(&mut boom);
        let (mut builder_slot, mut repo_slot) = (0u64, 0u64);
        let builder = ptr::from_mut(&mut builder_slot).cast::<ffi::git_commitbuilder>();
        let repo = ptr::from_mut(&mut repo_slot).cast::<ffi::git_repository>();
        let content = c"tree x\n\nmessage\n".as_ptr();
        let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
            callback.sign_with(|sign, payload| {
                let sign = sign.unwrap();
                // SAFETY: the payload is the callback; each pointer is NULL or live
                // storage the closure never reads, and `content` is NUL-terminated.
                unsafe {
                    let user = ErrorCode::User.raw();
                    assert_eq!(sign(ptr::null_mut(), repo, content, payload), user);
                    assert_eq!(sign(builder, ptr::null_mut(), content, payload), user);
                    assert_eq!(sign(builder, repo, ptr::null(), payload), user);
                    assert_eq!(sign(builder, repo, content, payload), user);
                    assert_eq!(sign(builder, repo, content, payload), user);
                }
            })
        }));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"first call")
        );
        assert!(format!("{callback:?}").contains("panicked: false"));
        drop(callback);
        assert_eq!(calls, 1);
    }

    /// A callback stored in options through `set_signer` signs each commit the options
    /// are passed to; a panic of its closure fails that commit with `GIT_EUSER`, is kept
    /// until `resume_panic` once the options are gone, and blocks signing until then.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signers_stored_in_options_sign_and_keep_panics() {
        let w = Writer::new("options-signer");
        let mut calls = 0;
        let mut sign = |mut builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            calls += 1;
            if calls == 2 {
                panic!("second commit");
            }
            builder
                .add_header(c"gpgsig", c"stored")
                .map_err(StreamError::from)
        };
        let mut signer = SignatureCallback::new(&mut sign);
        {
            let mut options = CreateExtOptions::new();
            options.as_mut().set_signer(Some(&mut signer));
            let (callback, payload) = options.as_ref().sign_callback();
            let id = w.create(c"one\n", callback, payload).unwrap();
            assert!(
                String::from_utf8(w.raw(&id))
                    .unwrap()
                    .contains("\ngpgsig stored\n")
            );
            let (rc, _) = w.create(c"two\n", callback, payload).unwrap_err();
            assert_eq!(rc, ErrorCode::User.raw());
            let (rc, _) = w.create(c"three\n", callback, payload).unwrap_err();
            assert_eq!(rc, ErrorCode::User.raw());
        }
        assert!(format!("{signer:?}").contains("panicked: true"));
        let caught = std::panic::catch_unwind(AssertUnwindSafe(|| signer.resume_panic()));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"second commit")
        );
        signer.resume_panic();
        let mut options = CreateExtOptions::new();
        options.as_mut().set_signer(Some(&mut signer));
        let (callback, payload) = options.as_ref().sign_callback();
        assert!(w.create(c"four\n", callback, payload).is_ok());
        drop(signer);
        assert_eq!((calls, w.commits()), (3, 2));
    }

    /// `GIT_COMMIT_CREATE_OPTIONS_INIT`, built in Rust so Miri can run the accessors;
    /// `create_options_new_matches_the_template` ties it to
    /// `git_commit_create_options_init`.
    fn create_template<'o>() -> CreateOptions<'o> {
        CreateOptions(
            ffi::git_commit_create_options {
                version: ffi::GIT_COMMIT_CREATE_OPTIONS_VERSION,
                ..Default::default()
            },
            PhantomData,
        )
    }

    fn create_raw(options: &CreateOptions<'_>) -> ffi::git_commit_create_options {
        // SAFETY: a read of the live options through their shared handle's pointer.
        unsafe { options.as_ref().as_ptr().read() }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn create_options_new_matches_the_template() {
        let (new, template) = (
            create_raw(&CreateOptions::new()),
            create_raw(&create_template()),
        );
        assert_eq!(new.version, template.version);
        assert_eq!(new._bitfield_1.get(0, 8), 0);
        assert_eq!(new.author, template.author);
        assert_eq!(new.committer, template.committer);
        assert_eq!(new.message_encoding, template.message_encoding);
        assert_eq!(new.extra_headers, template.extra_headers);
        assert_eq!(new.extra_headers_len, template.extra_headers_len);
        assert!(new.sign.is_none() && template.sign.is_none());
        assert_eq!(new.payload, template.payload);
        assert_eq!(create_raw(&CreateOptions::default()).version, 1);
    }

    /// The one-bit field reads and writes its own bit of the storage byte, leaving the
    /// seven others (and `version`, before it) as they were, as C's bitfield stores do.
    #[test]
    fn create_options_bitfield_keeps_its_neighbours() {
        let mut options = CreateOptions(
            ffi::git_commit_create_options {
                version: 7,
                _bitfield_1: ffi::__BindgenBitfieldUnit::new([0xfe]),
                ..Default::default()
            },
            PhantomData,
        );
        assert!(!options.as_ref().allow_empty_commit());
        options.as_mut().set_allow_empty_commit(true);
        let raw = create_raw(&options);
        assert_eq!((raw._bitfield_1.get(0, 8), raw.version), (0xff, 7));
        assert_eq!(raw.allow_empty_commit(), 1);
        assert!(options.as_ref().allow_empty_commit());
        options.as_mut().set_allow_empty_commit(false);
        let raw = create_raw(&options);
        assert_eq!(
            (raw._bitfield_1.get(0, 8), raw.allow_empty_commit()),
            (0xfe, 0)
        );
        assert!(!options.as_ref().allow_empty_commit());
    }

    /// Every setter stores what its getter reads back, the signatures', strings' and
    /// run's own pointers included, and clearing stores C's NULLs.
    #[test]
    fn create_options_setters_round_trip() {
        let encoding = std::ffi::CString::new("ISO-8859-1").unwrap();
        let headers = [Header::new(c"a", c"1"), Header::new(c"b", c"2")];
        let author = SignatureView::new(c"A", c"a@example.com", Time::new(1, 60));
        let committer = SignatureView::new(c"C", c"c@example.com", Time::new(2, -30));
        let mut never = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            unreachable!("only stored: the options reach no libgit2 call")
        };
        let mut signer = SignatureCallback::new(&mut never);
        let signer_at = ptr::from_mut(&mut signer).cast::<c_void>();
        let mut options = create_template();
        assert_eq!(
            format!("{options:?}"),
            "CreateOptions { version: 1, allow_empty_commit: false, author: None, \
             committer: None, message_encoding: None, extra_headers: [], \
             sign_callback: false }"
        );
        let mut m = options.as_mut();
        m.set_allow_empty_commit(true);
        m.set_author(Some(committer.as_ref()));
        m.set_committer(Some(committer.as_ref()));
        m.set_message_encoding(Some(&encoding));
        m.set_extra_headers(&headers);
        m.set_signer(Some(&mut signer));
        {
            let mut inner = m.as_mut();
            inner.set_author(Some(author.as_ref()));
            assert_eq!(inner.as_ref().author().unwrap().name(), Some(c"A"));
        }
        let r = m.as_ref();
        assert_eq!(r.version(), 1);
        assert!(r.allow_empty_commit());
        assert_eq!(r.author().unwrap().as_ptr(), author.as_ref().as_ptr());
        assert_eq!(r.committer().unwrap().as_ptr(), committer.as_ref().as_ptr());
        assert_eq!(r.committer().unwrap(), committer.as_ref());
        assert_eq!(r.message_encoding().unwrap().as_ptr(), encoding.as_ptr());
        let run = r.extra_headers();
        assert_eq!(run.len(), 2);
        assert_eq!(run.as_ptr(), headers.as_ptr().cast());
        assert_eq!(run.get(1).unwrap().value(), c"2");
        let (sign, data) = r.sign_callback();
        assert!(sign.is_some() && r.has_signer());
        assert_eq!(data, signer_at);
        assert_eq!(
            format!("{m:?}"),
            format!(
                "CreateOptions {{ version: 1, allow_empty_commit: true, author: Some({:?}), \
                 committer: Some({:?}), message_encoding: Some(\"ISO-8859-1\"), \
                 extra_headers: [Header {{ field: \"a\", value: \"1\" }}, \
                 Header {{ field: \"b\", value: \"2\" }}], sign_callback: true }}",
                author.as_ref(),
                committer.as_ref()
            )
        );

        m.set_allow_empty_commit(false);
        m.set_author(None);
        m.set_committer(None);
        m.set_message_encoding(None);
        m.set_extra_headers(&headers[..0]);
        m.set_signer(None);
        let back = create_raw(&options);
        assert_eq!(back._bitfield_1.get(0, 8), 0);
        assert!(back.author.is_null() && back.committer.is_null());
        assert!(back.extra_headers.is_null() && back.extra_headers_len == 0);
        assert!(back.message_encoding.is_null());
        assert!(back.sign.is_none() && back.payload.is_null());
        assert!(options.as_ref().extra_headers().is_empty());
        assert_eq!(format!("{options:?}"), format!("{:?}", create_template()));
    }

    /// `git_commit_create_from_tree` on the writer's `HEAD`, reading `options` in place:
    /// the new commit's id, or the code and the thread's error.
    fn create_from_tree(
        w: &Writer,
        options: &CreateOptions<'_>,
    ) -> Result<ffi::git_oid, (c_int, ErrorBuf)> {
        let mut id = ffi::git_oid::default();
        // SAFETY: a writable id; the live repository and its tree; a NUL-terminated
        // message; options Rust holds, whose borrows outlive the call by their `'o`.
        let rc = unsafe {
            ffi::git_commit_create_from_tree(
                &mut id,
                w.repo,
                w.tree,
                c"msg\n".as_ptr(),
                options.as_ref().as_ptr(),
            )
        };
        if rc < 0 { Err((rc, last())) } else { Ok(id) }
    }

    /// libgit2 reads every field in place: empty commits are refused by default and
    /// written when allowed; the signatures, encoding, headers and signer reach the
    /// commit; an unset author or committer is the configured default identity, whose
    /// failure fails the call; amending (which needs a `HEAD` commit) reads the same
    /// struct, keeping the amended commit's author and encoding where the options leave
    /// them unset, and ignoring `allow_empty_commit`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn create_options_read_in_place_by_libgit2() {
        let w = Writer::new("create-options");
        // Amending needs a `HEAD` commit.
        let options = CreateOptions::new();
        let mut amended = ffi::git_oid::default();
        // SAFETY: a writable id; the live repository, whose `HEAD` is unborn, and its
        // tree; no new message; options Rust holds.
        let rc = unsafe {
            ffi::git_commit_amend_from_tree(
                &mut amended,
                w.repo,
                w.tree,
                ptr::null(),
                options.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, ErrorCode::UnbornBranch.raw());

        // `git_commit_create_from_tree` reports the check's `GIT_EUNCHANGED` as -1.
        let (rc, error) = create_from_tree(&w, &CreateOptions::new()).unwrap_err();
        assert_eq!(rc, ErrorCode::Generic.raw());
        assert_eq!(error.class(), Some(ErrorClass::Repository));
        assert_eq!(error.message(), c"no changes are staged for commit");
        assert_eq!(w.commits(), 0);

        let headers = [Header::new(c"x-a", c"1")];
        let author = SignatureView::new(c"A", c"a@example.com", Time::new(1, 60));
        let committer = SignatureView::new(c"C", c"c@example.com", Time::new(2, -30));
        let mut sign = |mut builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            builder
                .add_header(c"gpgsig", c"sig")
                .map_err(StreamError::from)
        };
        let mut signer = SignatureCallback::new(&mut sign);
        let first = {
            let mut options = CreateOptions::new();
            let mut m = options.as_mut();
            m.set_allow_empty_commit(true);
            m.set_author(Some(author.as_ref()));
            m.set_committer(Some(committer.as_ref()));
            m.set_message_encoding(Some(c"ISO-8859-1"));
            m.set_extra_headers(&headers);
            m.set_signer(Some(&mut signer));
            create_from_tree(&w, &options).unwrap()
        };
        signer.resume_panic();
        let text = String::from_utf8(w.raw(&first)).unwrap();
        assert!(
            text.ends_with(
                "\nauthor A <a@example.com> 1 +0100\ncommitter C <c@example.com> 2 -0030\n\
                 encoding ISO-8859-1\nx-a 1\ngpgsig sig\n\nmsg\n"
            ),
            "{text}"
        );

        // An unset author or committer is the default identity, which libgit2 refuses
        // when the configuration names it with an empty name.
        let config = w.dir.join("config");
        let mut text = std::fs::read_to_string(&config).unwrap();
        text.push_str("[user]\n\tname = \n\temail = e@example.com\n");
        std::fs::write(&config, text).unwrap();
        let mut options = CreateOptions::new();
        options.as_mut().set_allow_empty_commit(true);
        options.as_mut().set_committer(Some(committer.as_ref()));
        let empty = c"failed to parse signature - Signature cannot have an empty name or email";
        let (rc, error) = create_from_tree(&w, &options).unwrap_err();
        assert_eq!(
            (rc, error.class(), error.message()),
            (-1, Some(ErrorClass::Invalid), empty)
        );
        let mut amended = ffi::git_oid::default();
        // SAFETY: as below, with options borrowing nothing.
        let rc = unsafe {
            ffi::git_commit_amend_from_tree(
                &mut amended,
                w.repo,
                w.tree,
                ptr::null(),
                CreateOptions::new().as_ref().as_ptr(),
            )
        };
        let error = last();
        assert_eq!(
            (rc, error.class(), error.message()),
            (-1, Some(ErrorClass::Invalid), empty)
        );
        assert_eq!(w.commits(), 1);

        // Amending: the author and the encoding come from the amended commit, the
        // committer from the options; empty commits are not checked.
        let mut options = CreateOptions::new();
        options.as_mut().set_committer(Some(author.as_ref()));
        let mut amended = ffi::git_oid::default();
        // SAFETY: a writable id; the live repository, whose `HEAD` is `first`, and its
        // tree; no new message; options Rust holds, borrowing `author` past the call.
        let rc = unsafe {
            ffi::git_commit_amend_from_tree(
                &mut amended,
                w.repo,
                w.tree,
                ptr::null(),
                options.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
        let text = String::from_utf8(w.raw(&amended)).unwrap();
        assert!(
            text.ends_with(
                "\nauthor A <a@example.com> 1 +0100\ncommitter A <a@example.com> 1 +0100\n\
                 encoding ISO-8859-1\n\nmsg\n"
            ),
            "{text}"
        );
        assert_eq!(w.commits(), 2);
    }
}
