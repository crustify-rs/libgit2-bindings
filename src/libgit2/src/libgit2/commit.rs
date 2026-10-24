//! Safe wrappers over `include/git2/commit.h`.
//!
//! # Commits
//!
//! A [`Commit`] is a `git_commit`: a parsed commit object, whose first member is the
//! `git_object` every object starts with, of type `GIT_OBJECT_COMMIT`. It is opaque:
//! libgit2 publishes no field. It is counted and shared as every [`Object`](super::object::Object)
//! is (see the [`object` module](super::object#ownership)): the repository's object cache holds a
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
//! What the commit records is read through the shared handle, in place, borrowed for as
//! long as the handle: its id, its tree's and its parents' ids ([`CommitRef::id`],
//! [`CommitRef::tree_id`], [`CommitRef::parent_id`]), its message and headers as written
//! ([`CommitRef::message_raw`], [`CommitRef::raw_header`]; [`CommitRef::header_field`]
//! copies one header's value out), and its committer's time. libgit2 parses every commit
//! it hands out in full (only the revision walker parses commits partially, into
//! structs of its own that it never publishes), so each of these is set, the encoding
//! alone being optional, and none is written again before the commit is freed.
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
//! contents built elsewhere ([`create_buffer`]) with a signature added as a header, and
//! [`extract_signature`] takes a written commit's signature header back out, with the
//! contents it signs.
//!
//! # Reading a commit
//!
//! A commit lends what it records for as long as it is borrowed: its author and
//! committer ([`CommitRef::author`], [`CommitRef::committer`]), as [`SignatureRef`]s
//! into the commit, and the body of its message ([`CommitRef::body`]), which libgit2
//! computes on first use and keeps in the commit. [`CommitRef::author_with_mailmap`]
//! and [`CommitRef::committer_with_mailmap`] copy a signature out as a mailmap maps it.
//!
//! # Creating commits
//!
//! [`create`] writes a commit of a tree and parents looked up in the repository, as
//! [`create_from_array`] does for a number of parents fixed at compile time (the
//! variadic `git_commit_create_v`), and [`create_ext`] with [creation
//! options](#creation-options); [`create_buffer`] formats the same contents without
//! writing them, and [`CommitRef::amend`] writes a copy of a commit with some of its
//! parts replaced. The routines of [`sys::commit`](super::sys::commit) take the tree and
//! the parents by id. A [`CreateCallback`] (`git_commit_create_cb`) writes the commits a
//! routine (a rebase) is about to write, in its place.
//!
//! [`create_from_stage`] and [`create_from_tree`] commit the index or a tree on `HEAD`,
//! as `git commit` does, with [creation options](#creation-options) whose unset
//! signatures are the repository's default identity, and [`amend_from_stage`] and
//! [`amend_from_tree`] amend `HEAD`'s commit the same way. The two creating routines are
//! composed from the routines libgit2 composes them from, but for the parents, which
//! come from [`RepositoryRef::commit_parents`]: libgit2's own collection frees commits
//! the object cache still holds when `MERGE_HEAD` cannot be read through.
//!
//! libgit2 asserts that the tree is the repository's (a release build fails, a debug
//! one aborts), and ends the parent list silently at the first commit of another
//! repository: the wrappers refuse both before it runs. They also refuse a signature
//! libgit2 cannot write: one whose name or email its parser lost, which it would print
//! as a NULL string, and one whose offset is `i32::MIN` minutes, which it would negate
//! out of range. A time outside `u32` seconds is written wrapped, as libgit2 writes it.
//!
//! Safe wrappers over `include/git2/sys/commit.h`.
//!
//! [`create_from_ids`] and [`create_from_callback`] write a commit as
//! [`commit::create`](crate::libgit2::src::libgit2::commit::create) does, of a tree and
//! parents given by id: libgit2 then checks that they exist, with their types, unless
//! strict object creation is turned off. An id of another type than the repository's
//! is refused before libgit2 reads it.
//!
//! # Parent callbacks
//!
//! A [`ParentCallback`] is the closure a `git_commit_parent_callback` runs:
//! `git_commit_create_from_callback` calls it with 0, 1, 2... for the commit's parents in
//! order, until it returns NULL, on the calling thread. Each id is read through the
//! pointer the callback returns, checked against the repository when strict object
//! creation is on (the default), and copied before the next call, so the callback keeps
//! the last id it handed out until it is called again; the closure returns ids by
//! value.
//!
//! libgit2 has no way to stop at a parent: anything but NULL is a parent, and NULL ends
//! the list, after which the commit is written (with strict object creation turned off,
//! whatever the parents are, and the reference to update moved to it). It also reads
//! each id from the object database without checking that it is of the repository's id
//! type, which the loose backend then formats out of bounds (see the
//! [`object` module](crate::libgit2::src::libgit2::object#looking-objects-up)). So the walk
//! a commit routine's wrapper runs asks the closure for every parent first, until it
//! returns `None`, refusing an id of another type than the repository's: only once the
//! closure gave them all does libgit2 run, through a trampoline handing out those ids,
//! which can neither fail nor panic. A refusal or a panic of the closure ends the walk
//! before libgit2 writes anything, whatever its options.

use core::any::Any;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull};
use std::ffi::CString;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use ffibox::{
    CArc, CBorrowedPtr, CCell, CDispose, CRefClone, CSlice, CVal, CZeroable, define_ctype,
};

use crate::ffi;
use crate::libgit2::src::libgit2::buf::{Buf, BufVal};
use crate::libgit2::src::libgit2::diff::Diff;
use crate::libgit2::src::libgit2::filter::StreamError;
use crate::libgit2::src::libgit2::index::IndexRef;
use crate::libgit2::src::libgit2::mailmap::MailmapRef;
use crate::libgit2::src::libgit2::object::{ObjectArc, ObjectRef, id_fits};
use crate::libgit2::src::libgit2::object_api::{CommitFree, resolvable, signed};
use crate::libgit2::src::libgit2::odb::{OdbDataArrayFree, OdbDataVec};
use crate::libgit2::src::libgit2::oid::{Oid, OidRef, OidType};
use crate::libgit2::src::libgit2::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::src::libgit2::signature::{Signature, SignatureBox, SignatureRef};
use crate::libgit2::src::libgit2::tree::{Tree, TreeArc, TreeRef};
use crate::libgit2::src::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last, set};

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
/// use libgit2::libgit2::src::libgit2::commit::CommitArc;
///
/// fn widen<'r>(commit: CommitArc<'r>) -> CommitArc<'static> {
///     commit
/// }
/// ```
///
/// and a commit stays on the thread that looked it up:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::commit::CommitArc;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(commit: CommitArc<'static>) {
///     send(commit)
/// }
/// ```
///
/// Wraps: `git_commit`
/// Handwritten
#[repr(transparent)]
pub struct Commit<'r>(ffi::git_commit, PhantomData<(*const (), &'r ())>);

/// Shared borrow of a [`Commit`]. `Copy`, like `&T`.
///
/// It stays on the thread of the commit's repository, as the owner does:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::commit::CommitRef;
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
    type Mut<'a>
        = CommitMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = CommitRef<'a, 'r>
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
        // SAFETY: this function's contract is `CCell::ref_from_ptr`'s: `ptr` is NULL or a
        // live `git_commit` that outlives `'a` and is not written meanwhile.
        unsafe { <Commit<'r> as CCell>::ref_from_ptr(ptr.cast_mut()) }
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
    /// # use libgit2::libgit2::src::libgit2::commit::Commit;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let copy = commit.as_ref().to_commit();
    /// drop(commit);
    /// drop(repo);
    /// drop(copy);
    /// ```
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a handle is never NULL"
    )]
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
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a handle is never NULL"
    )]
    pub fn as_object(&self) -> ObjectRef<'a, 'r> {
        // SAFETY: a `git_commit` starts with its `git_object`, so the handle's pointer
        // addresses a live object of the same repository for `'a`, as this handle's
        // does; nothing is written through it while it is shared.
        let object = unsafe { ObjectRef::from_ptr(self.as_ptr().cast()) };
        object.expect("a handle is never NULL")
    }

    /// The object, as a commit, when it is one (of type `GIT_OBJECT_COMMIT`); `None`
    /// for a tree, a blob or a tag.
    #[must_use]
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
    /// The parent lives no longer than the repository's borrow `'r`, as this commit
    /// does, but may outlive this handle and the commit's owner.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::commit::Commit;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let parent = commit.as_ref().parent(0).unwrap();
    /// drop(commit);
    /// drop(repo);
    /// drop(parent);
    /// ```
    ///
    /// Wraps: `git_commit_parent`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] (class `GIT_ERROR_INVALID`, "parent `n` does
    /// not exist") when the commit has `n` parents or fewer; and as
    /// [`Commit::lookup`] does for the parent's id: [`ErrorCode::NotFound`] when the
    /// object database lacks the parent or it is not a commit, libgit2's error when it
    /// cannot be read or parsed.
    ///
    /// # Panics
    ///
    /// Panics, with `a parent lookup succeeded without a commit`, only if libgit2 breaks its
    /// contract.
    pub fn parent(&self, n: u32) -> Result<CommitArc<'r>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; the handle addresses a
        // live commit for `'a`, whose recorded parent ids are of its repository's type
        // and whose repository, borrowed for `'r`, is live and used on this thread only,
        // as `RepositoryRef` allows: libgit2 looks the parent up through it, filling its
        // cache.
        let rc = unsafe { ffi::git_commit_parent(&raw mut out, self.as_ptr(), n) };
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
    /// The ancestor lives no longer than the repository's borrow `'r`, as this commit
    /// does.
    ///
    /// Wraps: `git_commit_nth_gen_ancestor`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] (class `GIT_ERROR_INVALID`, "parent 0 does
    /// not exist") when the first-parent chain reaches a root commit before `n`
    /// generations, and as [`parent`](Self::parent) does when an ancestor cannot be
    /// looked up. Every intermediate commit is released.
    ///
    /// # Panics
    ///
    /// Panics, with `an ancestor lookup succeeded without a commit`, only if libgit2 breaks its
    /// contract.
    pub fn nth_gen_ancestor(&self, n: u32) -> Result<CommitArc<'r>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: as in `parent`. libgit2 also takes, and later releases or hands out,
        // one more reference on this commit (`git_commit_dup`, an atomic increment
        // through a cast of the `const` pointer), which leaves the holder's own
        // reference untouched.
        let rc = unsafe { ffi::git_commit_nth_gen_ancestor(&raw mut out, self.as_ptr(), n) };
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
    /// The tree lives no longer than the repository's borrow `'r`, as this commit
    /// does, but may outlive this handle and the commit's owner.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::commit::Commit;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let tree = commit.as_ref().tree().unwrap();
    /// drop(commit);
    /// drop(repo);
    /// drop(tree);
    /// ```
    ///
    /// Wraps: `git_commit_tree`
    ///
    /// # Errors
    ///
    /// Fails as [`Tree::lookup`](crate::libgit2::src::libgit2::tree::Tree::lookup) does for the
    /// tree's id: with [`ErrorCode::NotFound`]
    /// when the object database lacks the tree or it is not a tree, and with
    /// libgit2's error when it cannot be read or parsed.
    ///
    /// # Panics
    ///
    /// Panics, with `a tree lookup succeeded without a tree`, only if libgit2 breaks its contract.
    pub fn tree(&self) -> Result<TreeArc<'r>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: as in `parent`: `out` is a writable slot, the handle addresses a live
        // commit whose recorded tree id is of its repository's type, and whose
        // repository is live for `'r` and used on this thread only, where libgit2
        // looks the tree up.
        let rc = unsafe { ffi::git_commit_tree(&raw mut out, self.as_ptr()) };
        if rc < 0 {
            return failed(rc);
        }
        // SAFETY: on success `out` holds one counted reference to a live tree of the
        // same repository, borrowed for `'r`, given to the result and released by
        // `git_tree_free`.
        let tree = unsafe { TreeArc::from_c(out) };
        Ok(tree.expect("a tree lookup succeeded without a tree"))
    }

    /// The commit's message, as git shows it: the raw message without its leading
    /// newlines. Borrowed in place for as long as the handle, so it cannot outlive the
    /// commit, as no string a getter returns can:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::commit::Commit;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let message = commit.as_ref().message();
    /// drop(commit);
    /// let _ = message.to_bytes();
    /// ```
    ///
    /// Wraps: `git_commit_message`
    #[must_use]
    pub fn message(&self) -> &'a CStr {
        // SAFETY: the handle addresses a live, fully parsed commit for `'a`, whose raw
        // message libgit2 only reads, returning a pointer into it.
        let message = unsafe { ffi::git_commit_message(self.as_ptr()) };
        // SAFETY: a non-NULL pointer into the commit's raw message, NUL-terminated,
        // which libgit2 never writes after parsing it and frees only with the commit,
        // which lives for `'a`.
        unsafe { CStr::from_ptr(message) }
    }

    /// The commit's message exactly as the commit records it: everything after the
    /// blank line that ends its headers (empty without one), leading newlines
    /// included. Borrowed in place for as long as the handle.
    ///
    /// Wraps: `git_commit_message_raw`
    #[must_use]
    pub fn message_raw(&self) -> &'a CStr {
        // SAFETY: the handle addresses a live commit for `'a`; libgit2 only reads its
        // raw message pointer.
        let message = unsafe { ffi::git_commit_message_raw(self.as_ptr()) };
        // SAFETY: the commit's own raw message, non-NULL in a fully parsed commit and
        // NUL-terminated, never written after parsing and freed only with the commit,
        // which lives for `'a`.
        unsafe { CStr::from_ptr(message) }
    }

    /// The encoding the commit's `encoding` header names for its message, borrowed in
    /// place for as long as the handle; `None` without that header, where git assumes
    /// UTF-8. Only the last `encoding` header counts, and libgit2 leaks the others.
    ///
    /// Wraps: `git_commit_message_encoding`
    #[must_use]
    pub fn message_encoding(&self) -> Option<&'a CStr> {
        // SAFETY: the handle addresses a live commit for `'a`; libgit2 only reads its
        // encoding pointer, NULL without the header.
        let encoding = unsafe { ffi::git_commit_message_encoding(self.as_ptr()) };
        // SAFETY: a non-NULL encoding is the commit's own NUL-terminated copy of the
        // header's value, never written after parsing and freed only with the commit,
        // which lives for `'a`.
        (!encoding.is_null()).then(|| unsafe { CStr::from_ptr(encoding) })
    }

    /// The commit's headers, exactly as the commit records them: every line before the
    /// blank line that ends them (`tree`, the `parent`s, `author`, `committer`, then
    /// any others), each with its newline. Borrowed in place for as long as the handle.
    ///
    /// Wraps: `git_commit_raw_header`
    #[must_use]
    pub fn raw_header(&self) -> &'a CStr {
        // SAFETY: the handle addresses a live commit for `'a`; libgit2 only reads its
        // raw header pointer.
        let header = unsafe { ffi::git_commit_raw_header(self.as_ptr()) };
        // SAFETY: the commit's own raw header, non-NULL in a fully parsed commit and
        // NUL-terminated, never written after parsing and freed only with the commit,
        // which lives for `'a`.
        unsafe { CStr::from_ptr(header) }
    }

    /// The value of the commit's header `field`: the first header line (`tree`
    /// included) whose name is exactly `field`, without the name and its space; each line that
    /// continues it (a line starting with a space) adds a newline and the line without
    /// its space, as a `gpgsig` signature spans several.
    ///
    /// Wraps: `git_commit_header_field`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] (class `GIT_ERROR_OBJECT`, "no such field
    /// '`field`'") when no header has that name, the empty name included; and with
    /// `GIT_ERROR_OBJECT`, "malformed header", when the header's last continuation line
    /// does not end, or libgit2's error when it runs out of memory.
    ///
    /// Refuses, as a field it does not find, a `field` holding a newline, which no
    /// header's name holds: libgit2 matches the field against the rest of the headers,
    /// across line ends, and on such a match copies a negative length, overflowing a
    /// pointer or requesting an allocation of nearly `usize::MAX` bytes.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a C string holds no NUL"
    )]
    pub fn header_field(&self, field: &CStr) -> Result<BufVal, RepositoryError> {
        if field.to_bytes().contains(&b'\n') {
            let mut message = b"no such field '".to_vec();
            message.extend_from_slice(field.to_bytes());
            message.push(b'\'');
            let message = CString::new(message).expect("a C string holds no NUL");
            set(ErrorClass::Object, &message);
            return failed(ErrorCode::NotFound.raw());
        }
        let mut out = Buf::new();
        let buf = out.as_mut().as_mut_ptr();
        // SAFETY: `buf` addresses the fresh, empty buffer held above, which libgit2
        // fills (`git_buf_fromstr`) or leaves empty; the handle addresses a live, fully
        // parsed commit for `'a`, whose raw header (newline-terminated, as the parser
        // cuts it) libgit2 only reads; `field` is NUL-terminated, holds no newline, so
        // every match ends within its line, and is read for the call only. The commit's
        // allocation pinned the runtime, so errors reach live thread state.
        let rc = unsafe { ffi::git_commit_header_field(buf, self.as_ptr(), field.as_ptr()) };
        if rc < 0 { failed(rc) } else { Ok(out) }
    }

    /// The commit's summary: the first paragraph of its [`message`](Self::message) (up
    /// to a blank line, or a line of whitespace alone), each run of whitespace that
    /// holds a newline collapsed to one space and every other run kept, trailing
    /// whitespace dropped. Empty for an empty message.
    ///
    /// libgit2 computes it on first use and keeps it in the commit, shared by every
    /// owner and handle of the commit (so a commit is not `Sync`); it lives as long as
    /// the commit, and the result is borrowed for as long as the handle.
    ///
    /// Wraps: `git_commit_summary`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's out-of-memory error, only when it cannot allocate even an
    /// empty summary; a summary that runs out of memory part-way comes out empty.
    pub fn summary(&self) -> Result<&'a CStr, ErrorBuf> {
        // SAFETY: the handle addresses a live, fully parsed commit for `'a`. libgit2
        // writes its `summary` field once, when NULL, through this pointer: nothing else
        // reads or writes the commit meanwhile, since a commit and its handles stay on
        // one thread and hold no Rust reference into it, and no earlier result is
        // invalidated, since a summary, once set, is never replaced.
        let summary = unsafe { ffi::git_commit_summary(self.as_ptr().cast_mut()) };
        if summary.is_null() {
            return Err(last());
        }
        // SAFETY: the NUL-terminated summary the commit now keeps, which libgit2 never
        // replaces and frees only with the commit, which lives for `'a`.
        Ok(unsafe { CStr::from_ptr(summary) })
    }

    /// When the commit was made, as its committer's time: seconds since the epoch.
    ///
    /// Wraps: `git_commit_time`
    #[must_use]
    pub fn time(&self) -> i64 {
        // SAFETY: the handle addresses a live, fully parsed commit for `'a`, whose
        // committer libgit2 always parses (a commit without one fails to parse); the
        // call only reads it.
        unsafe { ffi::git_commit_time(self.as_ptr()) }
    }

    /// The timezone the commit was made in, as its committer's: minutes east of UTC.
    ///
    /// Wraps: `git_commit_time_offset`
    #[must_use]
    pub fn time_offset(&self) -> i32 {
        // SAFETY: as in `time`.
        unsafe { ffi::git_commit_time_offset(self.as_ptr()) }
    }

    /// The id of the commit's tree, as the commit records it, borrowed in place for as
    /// long as the handle. [`tree`](Self::tree) looks the tree up.
    ///
    /// Wraps: `git_commit_tree_id`
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a field's address is not NULL"
    )]
    pub fn tree_id(&self) -> OidRef<'a> {
        // SAFETY: the handle addresses a live commit for `'a`; libgit2 returns the
        // address of its `tree_id` field, only reading the commit.
        let id = unsafe { ffi::git_commit_tree_id(self.as_ptr()) };
        // SAFETY: the id is a field of the commit, so it is non-NULL and lives as long
        // as the commit, past `'a`; nothing writes it after the commit is parsed.
        unsafe { OidRef::from_ptr(id.cast_mut()) }.expect("a field's address is not NULL")
    }

    /// How many parents the commit records: 0 for a root commit, 2 or more for a
    /// merge.
    ///
    /// Wraps: `git_commit_parentcount`
    #[must_use]
    pub fn parent_count(&self) -> u32 {
        // SAFETY: the handle addresses a live commit for `'a`; the call only reads the
        // size of its parent id array.
        unsafe { ffi::git_commit_parentcount(self.as_ptr()) }
    }

    /// The id of the commit's `n`th parent, counting from 0 as
    /// [`parent`](Self::parent) does, as the commit records it, borrowed in place for
    /// as long as the handle; `None` when the commit has `n` parents or fewer.
    ///
    /// Wraps: `git_commit_parent_id`
    #[must_use]
    pub fn parent_id(&self, n: u32) -> Option<OidRef<'a>> {
        // SAFETY: the handle addresses a live commit for `'a`; libgit2 bounds-checks `n`
        // against its parent id array (`git_array_get`) and returns NULL past it, or the
        // address of that element, only reading the commit.
        let id = unsafe { ffi::git_commit_parent_id(self.as_ptr(), n) };
        // SAFETY: NULL, or an element of the commit's parent id array, which lives as
        // long as the commit, past `'a`, and is not written after the commit is parsed.
        unsafe { OidRef::from_ptr(id.cast_mut()) }
    }
}

impl<'r> CommitMut<'_, 'r> {
    /// Borrows a raw pointer exclusively; `None` if null.
    ///
    /// # Safety
    ///
    /// As [`CommitRef::from_ptr`], plus: nothing else reaches the commit while the
    /// result lives, its repository's cache included.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_commit) -> Option<Self> {
        // SAFETY: the caller upholds `CommitRef::from_ptr`'s contract and exclusivity.
        unsafe { CommitRef::from_ptr(ptr) }.map(|r| CommitMut(r, PhantomData))
    }

    /// Writable pointer to the C object.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_commit {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    #[must_use]
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

/// One counted reference to a [`Commit`], given back by `git_commit_free` on drop;
/// `clone` takes another through `git_commit_dup`, on the same commit.
pub type CommitArc<'r> = CArc<Commit<'r>, CommitFree>;

impl<'r> Commit<'r> {
    /// The object, as a commit, when it is one (of type `GIT_OBJECT_COMMIT`): the same
    /// counted reference, moved. Otherwise the object, unchanged.
    ///
    /// # Errors
    ///
    /// Gives `object` back, unchanged, when it is not a commit.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: an owner is never NULL"
    )]
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
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: an owner is never NULL"
    )]
    pub fn into_object(commit: CommitArc<'r>) -> ObjectArc<'r> {
        let raw = commit.into_c();
        // SAFETY: `raw` is the counted reference the commit owner gave up; a
        // `git_commit` starts with its `git_object`, of the same repository, borrowed
        // for `'r`, and `git_object_free` releases it as `git_commit_free` would.
        let object = unsafe { ObjectArc::from_c(raw.cast()) };
        object.expect("an owner is never NULL")
    }
}

impl<'a> CommitRef<'a, '_> {
    /// The commit's author: who wrote the change, and when, as the commit records it.
    /// Borrowed in place for as long as the handle: the commit holds it, parsed once.
    ///
    /// Its name or email reads as `None` where libgit2 ran out of memory copying it
    /// while parsing the commit (see [`SignatureRef::name`]).
    ///
    /// The signature lives no longer than the commit's borrow:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::src::libgit2::commit::Commit;
    /// # use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    /// # use libgit2::libgit2::src::libgit2::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let author = commit.as_ref().author();
    /// drop(commit);
    /// let _ = author.name();
    /// ```
    ///
    /// Wraps: `git_commit_author`
    ///
    /// # Panics
    ///
    /// Panics, with `a parsed commit has an author`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn author(&self) -> SignatureRef<'a> {
        // SAFETY: the handle addresses a live commit for `'a`, which libgit2 only reads.
        let author = unsafe { ffi::git_commit_author(self.as_ptr()) };
        // SAFETY: `commit->author`, which a commit parsed in full (every commit a lookup
        // returns) always has: a signature the commit owns and frees with itself, so it
        // lives past `'a`, and which nothing writes once parsed. Its strings are NULL or
        // NUL-terminated strings of the commit's, as the handles require.
        unsafe { SignatureRef::from_ptr(author.cast_mut()) }.expect("a parsed commit has an author")
    }

    /// The commit's committer: who recorded the commit, and when (the commit's time).
    /// Borrowed in place for as long as the handle, as [`author`](Self::author) is.
    ///
    /// Wraps: `git_commit_committer`
    ///
    /// # Panics
    ///
    /// Panics, with `a parsed commit has a committer`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn committer(&self) -> SignatureRef<'a> {
        // SAFETY: the handle addresses a live commit for `'a`, which libgit2 only reads.
        let committer = unsafe { ffi::git_commit_committer(self.as_ptr()) };
        // SAFETY: `commit->committer`, which every parsed commit has (libgit2 parses it
        // even for its quick parses), owned by the commit, as in `author`.
        let committer = unsafe { SignatureRef::from_ptr(committer.cast_mut()) };
        committer.expect("a parsed commit has a committer")
    }

    /// The commit's author as `mailmap` maps them (unchanged for `None`, or when no
    /// entry matches), in a signature of its own, at the author's time.
    ///
    /// The mapped name and email are trimmed and checked as [`Signature::new`] checks
    /// them: fails with libgit2's error (`GIT_ERROR_INVALID`) when either holds an
    /// angle bracket or is empty once trimmed, and when it runs out of memory.
    ///
    /// [`Signature::new`]: crate::libgit2::src::libgit2::signature::Signature::new
    ///
    /// Wraps: `git_commit_author_with_mailmap`
    ///
    /// # Errors
    ///
    /// Refuses an author whose name or email libgit2 lost while parsing the commit,
    /// with the `GIT_ERROR_INTERNAL` error a release build of libgit2 reports from its
    /// assertion ("unrecoverable internal error: 'name'", or `'email'`): a debug build
    /// would abort.
    pub fn author_with_mailmap(
        &self,
        mailmap: Option<MailmapRef<'_>>,
    ) -> Result<SignatureBox, ErrorBuf> {
        resolvable(self.author())?;
        let mut out = ptr::null_mut();
        let mailmap = mailmap.map_or(ptr::null(), |m| m.as_ptr());
        // SAFETY: `out` is a writable slot, set only on success; the handle addresses a
        // live commit, whose author has both strings (checked above), only read;
        // `mailmap` is NULL or a live mailmap, only read. libgit2 is initialised: the
        // commit was allocated by it.
        let rc =
            unsafe { ffi::git_commit_author_with_mailmap(&raw mut out, self.as_ptr(), mailmap) };
        // SAFETY: on success `out` is a fresh signature from libgit2's allocator that
        // the caller owns alone, released by `git_signature_free`; it is NULL on failure.
        signed(rc, unsafe { SignatureBox::from_c(out) })
    }

    /// The commit's committer as `mailmap` maps them, as
    /// [`author_with_mailmap`](Self::author_with_mailmap) maps the author.
    ///
    /// Wraps: `git_commit_committer_with_mailmap`
    ///
    /// # Errors
    ///
    /// Fails as [`author_with_mailmap`](Self::author_with_mailmap) does, for the committer.
    pub fn committer_with_mailmap(
        &self,
        mailmap: Option<MailmapRef<'_>>,
    ) -> Result<SignatureBox, ErrorBuf> {
        resolvable(self.committer())?;
        let mut out = ptr::null_mut();
        let mailmap = mailmap.map_or(ptr::null(), |m| m.as_ptr());
        let commit = self.as_ptr();
        // SAFETY: as in `author_with_mailmap`, for the committer.
        let rc = unsafe { ffi::git_commit_committer_with_mailmap(&raw mut out, commit, mailmap) };
        // SAFETY: as in `author_with_mailmap`.
        signed(rc, unsafe { SignatureBox::from_c(out) })
    }

    /// The commit message's body: what follows its summary paragraph, without the
    /// whitespace (spaces, tabs, newlines) that surrounds it. `None` when the message
    /// has no body (one paragraph only, or only whitespace after it), and when libgit2
    /// runs out of memory copying it.
    ///
    /// libgit2 computes the body on first use and keeps it in the commit, shared by
    /// every holder: it is borrowed for as long as the handle, and later calls return
    /// the same string.
    ///
    /// Wraps: `git_commit_body`
    #[must_use]
    pub fn body(&self) -> Option<&'a CStr> {
        // SAFETY: the handle addresses a live commit for `'a`. libgit2 writes its
        // `body` cache through the pointer when it is NULL (and only then), as a `Cell`
        // is written through a shared reference: the commit is reached only from the
        // thread of its repository (no handle or owner is `Send`), so no other access
        // runs meanwhile, and no Rust reference to the commit exists.
        let body = unsafe { ffi::git_commit_body(self.as_ptr().cast_mut()) };
        // SAFETY: a non-NULL result is `commit->body`, a NUL-terminated string the
        // commit owns and frees with itself, so it lives past `'a`, and which libgit2
        // never writes or replaces once set.
        (!body.is_null()).then(|| unsafe { CStr::from_ptr(body) })
    }
}

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
/// use {libgit2::libgit2::src::libgit2::commit::CommitArray, libgit2::libgit2::src::libgit2::commit::CommitArrayVal};
///
/// let array: CommitArrayVal<'static> = CommitArray::new();
/// let copy: CommitArrayVal<'static> = array.clone();
/// ```
///
/// Nor `Send`: the commits it owns are not.
///
/// ```compile_fail,E0277
/// use {libgit2::libgit2::src::libgit2::commit::CommitArray, libgit2::libgit2::src::libgit2::commit::CommitArrayVal};
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
/// use libgit2::libgit2::src::libgit2::commit::CommitArrayMut;
///
/// fn shrink<'a, 'r>(m: CommitArrayMut<'a, 'static>) -> CommitArrayMut<'a, 'r> {
///     m
/// }
/// ```
///
/// Wraps: `git_commitarray`
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
    type Mut<'a>
        = CommitArrayMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = CommitArrayRef<'a, 'r>
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
/// Wraps: `git_commitarray_dispose`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct CommitArrayDispose;

// SAFETY: `git_commitarray_dispose` releases each of the first `count` commits once,
// frees `commits` once and never the struct, which it zeroes. It accepts every state the
// safe API reaches: a buffer libgit2 filled (so the allocator it reads is settled), one
// adopted through `from_raw` under that function's contract, or one `set_commits`
// allocated through libgit2's allocator; `clear` and `take` leave the empty array `new`
// builds, which owns nothing and is released without C.
unsafe impl<'r> CDispose<CommitArray<'r>> for CommitArrayDispose {
    #[inline]
    unsafe fn c_dispose(&self, ptr: NonNull<CommitArray<'r>>) {
        let raw: *mut ffi::git_commitarray = ptr.as_ptr().cast();
        // SAFETY: per `c_dispose`'s contract `raw` addresses a live `git_commitarray`; the
        // projection forms no reference to it.
        let commits_ptr = unsafe { &raw const (*raw).commits };
        // SAFETY: as above, the field is initialised.
        let commits = unsafe { commits_ptr.read() };
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
        // SAFETY: an empty array owns nothing, which `git_commitarray_dispose` accepts.
        unsafe {
            CVal::new(Self(
                ffi::git_commitarray {
                    commits: ptr::null(),
                    count: 0,
                },
                PhantomData,
            ))
        }
    }

    /// An array of `commits`, in order, moved into a fresh buffer of libgit2's
    /// allocator; an empty `commits` allocates nothing. Released, commits and buffer,
    /// by `git_commitarray_dispose`, as an array libgit2 filled is.
    ///
    /// # Errors
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
        // SAFETY: the caller gives up `raw`, which owns what it points at (this
        // function's contract).
        unsafe { CVal::new(Self(raw, PhantomData)) }
    }

    /// The raw `git_commitarray`, ownership included: C code releases it with
    /// `git_commitarray_dispose`.
    #[must_use]
    pub fn into_raw(self) -> ffi::git_commitarray {
        self.0
    }

    /// Shared handle to the array — the getters.
    #[must_use]
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
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
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
    /// Field: `git_commitarray.count`
    #[must_use]
    pub fn len(&self) -> usize {
        let array = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses a live `git_commitarray`; the projection
        // forms no reference to it.
        let commits_ptr = unsafe { &raw const (*array).commits };
        // SAFETY: as above.
        let count_ptr = unsafe { &raw const (*array).count };
        // SAFETY: reads the initialised field; nothing writes it while the array is shared.
        let commits = unsafe { commits_ptr.read() };
        // SAFETY: as above; any `size_t` is a valid `usize`.
        let count = unsafe { count_ptr.read() };
        if commits.is_null() { 0 } else { count }
    }

    /// Whether the array holds no commits.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The commit at `index`, borrowed from the array for as long as the array is;
    /// `None` past the [`len`](Self::len) commits. The array owns a reference to it, so
    /// it lives at least that long; [`to_commit`](CommitRef::to_commit) takes one of
    /// its own.
    ///
    /// ```compile_fail,E0505
    /// use {libgit2::libgit2::src::libgit2::commit::CommitArray, libgit2::libgit2::src::libgit2::commit::CommitArrayVal};
    ///
    /// let array: CommitArrayVal<'static> = CommitArray::new();
    /// let first = array.as_ref().get(0);
    /// drop(array);
    /// let _ = first;
    /// ```
    ///
    /// Field: `git_commitarray.commits`
    #[must_use]
    pub fn get(&self, index: usize) -> Option<CommitRef<'a, 'r>> {
        if index >= self.len() {
            return None;
        }
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let commits_ptr = unsafe { &raw const (*self.as_ptr()).commits };
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_commitarray`.
        let commits = unsafe { commits_ptr.read() };
        // SAFETY: `len` is non-zero, so `commits` is not NULL and, per the handles' invariant,
        // addresses at least `count > index` initialised commit pointers: the offset stays within
        // that run.
        let slot = unsafe { commits.add(index) };
        // SAFETY: `slot` addresses one of those initialised commit pointers, which nothing writes
        // while the array is shared.
        let commit = unsafe { slot.read() };
        // SAFETY: per the handles' invariant, a live commit of which the array owns one
        // reference, so it lives as long as the array does, past `'a`, looked up in a
        // repository that outlives `'r`.
        unsafe { CommitRef::from_ptr(commit) }
    }

    /// The commits, in order, each borrowed from the array as [`get`](Self::get) lends
    /// it.
    ///
    /// Field: `git_commitarray.commits`
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: an index under len"
    )]
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

impl<'r> CommitArrayMut<'_, 'r> {
    /// Borrows the array at `ptr` exclusively; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// As [`CommitArrayRef::from_ptr`], plus: nothing else reads or writes the array
    /// while the result lives, and whatever the setters store may stay there for as
    /// long as the array's owner expects commits of `'r`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
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
    #[must_use]
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
    /// Field: `git_commitarray.commits`
    pub fn clear(&mut self) {
        let array = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer is non-NULL, as every handle's is.
        let array_ptr = unsafe { NonNull::new_unchecked(array.cast::<CommitArray<'r>>()) };
        // SAFETY: `array_ptr` addresses a live `git_commitarray` (`CommitArray` is transparent over
        // it) that, per the handles' invariant, owns what it holds, released once here; it is reset
        // to the empty array below, before anything reads it again.
        unsafe { CommitArrayDispose.c_dispose(array_ptr) };
        // SAFETY: the exclusive handle's pointer addresses a live `git_commitarray`; the projection
        // forms no reference to it.
        let commits_ptr = unsafe { &raw mut (*array).commits };
        // SAFETY: as above.
        let count_ptr = unsafe { &raw mut (*array).count };
        // SAFETY: writes the empty array: C has zeroed it already unless `commits` was NULL, in
        // which case only `count` may differ.
        unsafe { commits_ptr.write(ptr::null()) };
        // SAFETY: as above.
        unsafe { count_ptr.write(0) };
    }

    /// Replaces the commits with `commits`, in order, moved into a fresh buffer of
    /// libgit2's allocator, releasing the ones held before as [`clear`](Self::clear)
    /// does; an empty `commits` leaves the empty array and allocates nothing.
    ///
    /// The commits must be of the array's own repository borrow `'r`:
    ///
    /// ```compile_fail,E0521
    /// use {libgit2::libgit2::src::libgit2::commit::CommitArc, libgit2::libgit2::src::libgit2::commit::CommitArrayMut};
    ///
    /// fn store<'r>(mut array: CommitArrayMut<'_, 'static>, commit: CommitArc<'r>) {
    ///     let _ = array.set_commits(vec![commit]);
    /// }
    /// ```
    ///
    /// Field: `git_commitarray.commits`
    /// Field: `git_commitarray.count`
    ///
    /// # Errors
    ///
    /// Fails, handing `commits` back and leaving the array as it was, when the buffer
    /// cannot be allocated, or libgit2 cannot be initialised (libgit2's error is then
    /// set on the calling thread).
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
        // SAFETY: the exclusive handle's pointer addresses a live `git_commitarray`; the projection
        // forms no reference to it.
        let commits_ptr = unsafe { &raw mut (*array).commits };
        // SAFETY: as above.
        let count_ptr = unsafe { &raw mut (*array).count };
        // SAFETY: `clear` released the previous commits and buffer. `buffer` is NULL with `count ==
        // 0`, or `count` pointers in libgit2's allocator, each a counted reference an owner of `'r`
        // gave up to the array, which keeps the handles' invariant.
        unsafe { commits_ptr.write(buffer.cast_const()) };
        // SAFETY: as above, the count of those pointers.
        unsafe { count_ptr.write(count) };
        Ok(())
    }

    /// Takes the commits out, in order, leaving the empty array: what
    /// `git_commitarray_dispose` would release, owned. The buffer is released.
    ///
    /// Field: `git_commitarray.commits`
    ///
    /// # Panics
    ///
    /// Panics, with `a non-NULL buffer`, only if libgit2 breaks its contract.
    pub fn take(&mut self) -> Vec<CommitArc<'r>> {
        let array = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses a live `git_commitarray`; the projection
        // forms no reference to it.
        let commits_ptr = unsafe { &raw mut (*array).commits };
        // SAFETY: as above.
        let count_ptr = unsafe { &raw mut (*array).count };
        // SAFETY: reads the initialised field through the exclusive handle.
        let commits = unsafe { commits_ptr.read() };
        // SAFETY: as above.
        let count = unsafe { count_ptr.read() };
        // SAFETY: writes the empty array, whose contents the caller now holds.
        unsafe { commits_ptr.write(ptr::null()) };
        // SAFETY: as above.
        unsafe { count_ptr.write(0) };
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
    /// use libgit2::libgit2::src::libgit2::commit::CommitBuilderMut;
    ///
    /// let mut slot = 0u64;
    /// let _ = unsafe { CommitBuilderMut::from_ptr(core::ptr::from_mut(&mut slot).cast()) };
    /// ```
    ///
    /// The exclusive handle does not alias, as `&mut` does not:
    ///
    /// ```compile_fail,E0499
    /// use libgit2::libgit2::src::libgit2::commit::CommitBuilderMut;
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
    /// use libgit2::libgit2::src::libgit2::commit::CommitBuilderMut;
    ///
    /// fn send<T: Send>(_: T) {}
    /// fn move_out(builder: CommitBuilderMut<'_>) {
    ///     send(builder)
    /// }
    /// ```
    ///
    /// Wraps: git_commitbuilder
    CommitBuilder,
    CommitBuilderRef,
    CommitBuilderMut,
    ffi::git_commitbuilder
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
    /// Wraps: `git_commitbuilder_add_header`
    ///
    /// # Errors
    ///
    /// Fails with `GIT_ERROR_INVALID`, "malformed commit contents", when the contents
    /// have no blank line (a commit libgit2 writes always has one), and with libgit2's
    /// out-of-memory error.
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
/// with options (`git_commit_create_ext`, and the `git_commit_create_from_*` and
/// `git_commit_amend_from_*` routines) call it once, on the calling thread, with the
/// commit's builder, its repository and its contents, before writing it, so that it can
/// add a signature header.
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
/// back in Rust, in the wrapper that wrote the commit ([`create_ext`] for a callback
/// lent to options with [`CreateExtOptionsMut::set_signer`], [`create_from_stage`] and
/// the other routines committing on `HEAD` for one lent with
/// [`CreateOptionsMut::set_signer`]), or from [`resume_panic`](Self::resume_panic).
///
/// The handles cannot outlive the call:
///
/// ```compile_fail,E0521
/// use libgit2::libgit2::src::libgit2::filter::StreamError;
/// use {libgit2::libgit2::src::libgit2::commit::CommitBuilderMut, libgit2::libgit2::src::libgit2::commit::SignatureCallback};
/// use libgit2::libgit2::src::libgit2::repository::RepositoryRef;
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
/// use libgit2::libgit2::src::libgit2::filter::StreamError;
/// use {libgit2::libgit2::src::libgit2::commit::CommitBuilderMut, libgit2::libgit2::src::libgit2::commit::SignatureCallback};
/// use libgit2::libgit2::src::libgit2::repository::RepositoryRef;
///
/// fn send<T: Send>(_: T) {}
/// let mut f = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &core::ffi::CStr| Ok::<(), StreamError>(());
/// send(SignatureCallback::new(&mut f));
/// ```
///
/// Wraps: `git_commit_signature_cb`
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
    /// [`CreateExtOptionsMut::set_signer`] or [`CreateOptionsMut::set_signer`], if any:
    /// that call failed with `GIT_EUSER`, and the callback refuses to sign until the
    /// panic is resumed. A wrapper that runs the call itself ([`create_ext`],
    /// [`create_from_stage`], [`create_from_tree`], [`amend_from_stage`],
    /// [`amend_from_tree`]) resumes it on its own.
    pub fn resume_panic(&mut self) {
        if let Some(panic) = self.panic.take() {
            resume_unwind(panic);
        }
    }

    /// Runs `write` with the C callback and its payload, as the wrapper of a routine that
    /// signs through it (`git_commit_create_ext`, through its options' `sign` and
    /// `payload`) hands them to it, then resumes a panic the closure raised meanwhile. The
    /// payload is this callback, which nothing else touches until `write` returns.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
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
/// Wraps: `git_commit_create_with_signature`
///
/// # Errors
///
/// Fails with `GIT_ERROR_OBJECT`, "failed to parse bad commit object", or the
/// signature parser's error, for contents that do not parse; with `GIT_ERROR_INVALID`,
/// "malformed commit contents", for a signed one without a blank line after its headers;
/// and with libgit2's error when a tree or parent is missing or the object database
/// cannot be loaded or written.
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
/// Wraps: `git_commit_extract_signature`
///
/// # Errors
///
/// Fails with [`ErrorCode::NotFound`] when the object is not a commit
/// (`GIT_ERROR_INVALID`) or has no such header (`GIT_ERROR_OBJECT`, "this commit is not
/// signed"), or when the object database has no such object; with `GIT_ERROR_OBJECT`,
/// "malformed header", when the header's last line does not end; and with libgit2's error
/// when the object database cannot be loaded or read.
///
/// Refuses, before libgit2 reads anything, an `id` of another type than `repo`'s, as
/// [`Object::lookup`](crate::libgit2::src::libgit2::object::Object::lookup) does (libgit2 would
/// format it out of bounds), and an empty `field`, with `GIT_ERROR_INVALID`, "the
/// signature header field name is empty": libgit2 would never get past the commit's
/// first line.
pub fn extract_signature(
    repo: RepositoryRef<'_>,
    id: OidRef<'_>,
    field: Option<&CStr>,
) -> Result<(BufVal, BufVal), RepositoryError> {
    id_fits(repo, id)?;
    if field.is_some_and(CStr::is_empty) {
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

// --- Creating commits ------------------------------------------------------------------

/// `Ok` when libgit2 can write `signature` into a commit's contents; the refusal
/// otherwise, set as the thread's error. `git_signature__writebuf` prints the name and
/// the email with `%s`, which a signature libgit2's parser lost one of (NULL) would hand
/// a NULL string, and negates a negative offset as an `int`, which overflows (undefined
/// behaviour) for `i32::MIN` minutes. Every other time is written, the seconds through a
/// 32-bit unsigned cast.
pub(crate) fn writable(signature: SignatureRef<'_>) -> Result<(), RepositoryError> {
    if signature.name().is_none() || signature.email().is_none() {
        set(
            ErrorClass::Invalid,
            c"failed to create commit: signature is missing its name or email",
        );
        return failed(ErrorCode::Generic.raw());
    }
    if signature.when().offset_minutes() == i32::MIN {
        set(
            ErrorClass::Invalid,
            c"failed to create commit: signature timezone offset is out of range",
        );
        return failed(ErrorCode::Generic.raw());
    }
    Ok(())
}

/// `Ok` when `tree` was looked up in `repo`, as libgit2 asserts; otherwise the error a
/// release build reports from that assertion (a debug build aborts), set as the
/// thread's.
fn tree_in(repo: RepositoryRef<'_>, tree: TreeRef<'_, '_>) -> Result<(), RepositoryError> {
    if tree.owner().as_ptr() == repo.as_ptr() {
        return Ok(());
    }
    set(
        ErrorClass::Invalid,
        c"invalid argument: 'git_tree_owner(tree) == repo'",
    );
    failed(ErrorCode::Generic.raw())
}

/// `Ok` when every commit of `parents` was looked up in `repo`; otherwise a
/// `GIT_ERROR_INVALID` refusal, set as the thread's. libgit2 itself ends the parent
/// list silently at the first commit of another repository (`git_commit_create`,
/// `git_commit_create_ext`, `git_commit_create_buffer`) or writes that commit's id as
/// a parent the repository may not have (`git_commit_create_v`).
fn parents_in(
    repo: RepositoryRef<'_>,
    parents: &[CommitRef<'_, '_>],
) -> Result<(), RepositoryError> {
    if parents.iter().all(|p| p.owner().as_ptr() == repo.as_ptr()) {
        return Ok(());
    }
    set(
        ErrorClass::Invalid,
        c"failed to create commit: a parent commit is from another repository",
    );
    failed(ErrorCode::Generic.raw())
}

/// The checks every commit-writing wrapper makes before libgit2 runs: `tree` and
/// `parents` of `repo`, and both signatures writable.
fn creatable(
    repo: RepositoryRef<'_>,
    author: SignatureRef<'_>,
    committer: SignatureRef<'_>,
    tree: TreeRef<'_, '_>,
    parents: &[CommitRef<'_, '_>],
) -> Result<(), RepositoryError> {
    tree_in(repo, tree)?;
    parents_in(repo, parents)?;
    writable(author)?;
    writable(committer)
}

/// Writes a commit into `repo`'s object database and returns its id: `tree` as its
/// tree, `parents` as its parents in order (none for a root commit), `author` and
/// `committer` as its signatures, then an `encoding` header for `message_encoding`
/// (none for `None`: git reads the message as UTF-8), a blank line and `message`, as
/// given (see [`prettify`](crate::libgit2::src::libgit2::message::prettify) to clean one up).
///
/// With an `update_ref`, the reference of that name (resolved through symbolic
/// references, so `HEAD` moves the current branch) is then moved to the commit, with a
/// reflog entry, or created when missing. An existing one must point at the first
/// parent: otherwise the call fails with [`ErrorCode::Modified`] (`GIT_ERROR_OBJECT`,
/// "failed to create commit: current tip is not the first parent") before writing
/// anything.
///
/// The tree and the parents are objects of `repo`, so libgit2 does not look them up
/// again. A signature's time is written as libgit2 writes every time: its seconds
/// through a 32-bit unsigned cast, so a time before the epoch or after 2106 wraps.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::src::libgit2::commit::{Commit, create};
/// use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
/// use libgit2::libgit2::src::libgit2::repository::Repository;
/// use libgit2::libgit2::src::libgit2::signature::{SignatureView, Time};
/// use libgit2::libgit2::src::libgit2::tree::Tree;
///
/// let dir = std::env::temp_dir().join(format!("commit-create-doc-{}", std::process::id()));
/// let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
/// let repo = Repository::init(&path, true).unwrap();
/// // The empty tree, which libgit2 knows without storing it.
/// let tree_id =
///     Oid::from_string(c"4b825dc642cb6eb9a060e54bf8d69288fbee4904", OidType::Sha1).unwrap();
/// let tree = Tree::lookup(repo.as_ref(), tree_id.as_ref()).unwrap();
/// let me = SignatureView::new(
///     c"A U Thor",
///     c"author@example.com",
///     Time::new(1_700_000_000, 60),
/// );
/// let id = create(
///     repo.as_ref(),
///     Some(c"HEAD"),
///     me.as_ref(),
///     me.as_ref(),
///     None,
///     c"Initial\n",
///     tree.as_ref(),
///     &[],
/// )
/// .unwrap();
/// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
/// assert_eq!(commit.as_ref().author().name(), Some(c"A U Thor"));
/// # drop(commit);
/// # drop(tree);
/// # drop(repo);
/// # std::fs::remove_dir_all(&dir).unwrap();
/// ```
///
/// Wraps: `git_commit_create`
///
/// # Errors
///
/// Refuses, before libgit2 runs and with `GIT_ERROR_INVALID`: a `tree` looked up in
/// another repository, with the error a release build of libgit2 reports from its
/// assertion ("invalid argument: '`git_tree_owner(tree)` == repo'"; a debug build
/// aborts); a parent looked up in another repository, which libgit2 would silently
/// drop, with every parent after it; and a signature missing its name or email (lost
/// by libgit2's parser) or whose offset is `i32::MIN` minutes, which libgit2 cannot
/// write. Fails with libgit2's error when the reference cannot be read or written, or
/// the object database cannot be loaded or written.
#[expect(
    clippy::too_many_arguments,
    reason = "the C routine's parameters, in its order"
)]
pub fn create<'r>(
    repo: RepositoryRef<'r>,
    update_ref: Option<&CStr>,
    author: SignatureRef<'_>,
    committer: SignatureRef<'_>,
    message_encoding: Option<&CStr>,
    message: &CStr,
    tree: TreeRef<'_, 'r>,
    parents: &[CommitRef<'_, 'r>],
) -> Result<Oid, RepositoryError> {
    creatable(repo, author, committer, tree, parents)?;
    let mut id = Oid::zeroed();
    let out = id.as_mut().as_mut_ptr();
    let mut ptrs: Vec<_> = parents.iter().map(CommitRef::as_ptr).collect();
    let update_ref = update_ref.map_or(ptr::null(), CStr::as_ptr);
    let encoding = message_encoding.map_or(ptr::null(), CStr::as_ptr);
    // SAFETY: `out` addresses `id`, written on success; `repo` is live on this thread,
    // its references, odb and caches written through this shared handle as `RepositoryRef`
    // allows; the signatures are live and writable (checked above); the strings are
    // NUL-terminated or NULL; `tree` and the `ptrs.len()` commits at `ptrs` are live objects
    // of `repo` (checked above), and `ptrs` is only read (dangling, unread, when empty).
    let rc = unsafe {
        ffi::git_commit_create(
            out,
            repo.as_ptr().cast_mut(),
            update_ref,
            author.as_ptr(),
            committer.as_ptr(),
            encoding,
            message.as_ptr(),
            tree.as_ptr(),
            ptrs.len(),
            ptrs.as_mut_ptr(),
        )
    };
    if rc < 0 { failed(rc) } else { Ok(id) }
}

/// The most parents [`create_from_array`] passes: each count is its own call of the
/// variadic C routine.
pub const MAX_ARRAY_PARENTS: usize = 16;

/// [`create`], for a number of parents fixed at compile time, as C code passes them to
/// the variadic `git_commit_create_v`: the same commit, written the same way, with the
/// same refusals. Up to [`MAX_ARRAY_PARENTS`] parents; [`create`] takes any number.
///
/// More parents than that do not compile:
///
/// ```compile_fail,E0080
/// use libgit2::libgit2::src::libgit2::commit::create_from_array;
///
/// let octopus = create_from_array::<17>;
/// # let _ = octopus as fn(_, _, _, _, _, _, _, _) -> _;
/// ```
///
/// Wraps: `git_commit_create_v`
///
/// # Errors
///
/// Fails as [`create`] does.
#[expect(
    clippy::too_many_arguments,
    reason = "the C routine's parameters, in its order"
)]
pub fn create_from_array<'r, const N: usize>(
    repo: RepositoryRef<'r>,
    update_ref: Option<&CStr>,
    author: SignatureRef<'_>,
    committer: SignatureRef<'_>,
    message_encoding: Option<&CStr>,
    message: &CStr,
    tree: TreeRef<'_, 'r>,
    parents: [CommitRef<'_, 'r>; N],
) -> Result<Oid, RepositoryError> {
    const {
        assert!(
            N <= MAX_ARRAY_PARENTS,
            "too many parents for create_from_array"
        );
    };
    creatable(repo, author, committer, tree, &parents)?;
    let mut id = Oid::zeroed();
    let out = id.as_mut().as_mut_ptr();
    let mut p = [ptr::null::<ffi::git_commit>(); MAX_ARRAY_PARENTS];
    for (slot, parent) in p.iter_mut().zip(&parents) {
        *slot = parent.as_ptr();
    }
    let (repo, update_ref) = (
        repo.as_ptr().cast_mut(),
        update_ref.map_or(ptr::null(), CStr::as_ptr),
    );
    let (author, committer) = (author.as_ptr(), committer.as_ptr());
    let encoding = message_encoding.map_or(ptr::null(), CStr::as_ptr);
    let (message, tree) = (message.as_ptr(), tree.as_ptr());
    /// `git_commit_create_v` with the fixed arguments, `N` and the first `N` of `p`.
    macro_rules! create_v {
        ($($n:literal => [$($i:literal)*]),* $(,)?) => {
            match N {
                $(
                    // SAFETY: as in `create`; the `N` variadic arguments are the parents'
                    // pointers, each a live `const git_commit *` of `repo` (checked
                    // above), as many as the count says, which libgit2 reads with
                    // `va_arg` and ends at; none is NULL.
                    $n => unsafe {
                        ffi::git_commit_create_v(
                            out, repo, update_ref, author, committer, encoding, message,
                            tree, N, $(p[$i]),*
                        )
                    },
                )*
                _ => unreachable!("N is at most MAX_ARRAY_PARENTS"),
            }
        };
    }
    let rc = create_v!(
        0 => [],
        1 => [0],
        2 => [0 1],
        3 => [0 1 2],
        4 => [0 1 2 3],
        5 => [0 1 2 3 4],
        6 => [0 1 2 3 4 5],
        7 => [0 1 2 3 4 5 6],
        8 => [0 1 2 3 4 5 6 7],
        9 => [0 1 2 3 4 5 6 7 8],
        10 => [0 1 2 3 4 5 6 7 8 9],
        11 => [0 1 2 3 4 5 6 7 8 9 10],
        12 => [0 1 2 3 4 5 6 7 8 9 10 11],
        13 => [0 1 2 3 4 5 6 7 8 9 10 11 12],
        14 => [0 1 2 3 4 5 6 7 8 9 10 11 12 13],
        15 => [0 1 2 3 4 5 6 7 8 9 10 11 12 13 14],
        16 => [0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15],
    );
    if rc < 0 { failed(rc) } else { Ok(id) }
}

/// The contents of the commit [`create`] would write, without writing it, nor moving a
/// reference: the raw text to sign and pass to [`create_with_signature`], or to hash.
///
/// Unlike [`create`], libgit2 checks that the tree and every parent exist in the object database
/// with their types, unless strict object creation is turned off; they are objects of `repo`, so
/// they do.
///
/// Wraps: `git_commit_create_buffer`
///
/// # Errors
///
/// Refuses what [`create`] refuses, and fails with libgit2's error when the object database cannot
/// be read.
pub fn create_buffer<'r>(
    repo: RepositoryRef<'r>,
    author: SignatureRef<'_>,
    committer: SignatureRef<'_>,
    message_encoding: Option<&CStr>,
    message: &CStr,
    tree: TreeRef<'_, 'r>,
    parents: &[CommitRef<'_, 'r>],
) -> Result<BufVal, RepositoryError> {
    creatable(repo, author, committer, tree, parents)?;
    let mut buf = Buf::new();
    let mut ptrs: Vec<_> = parents.iter().map(CommitRef::as_ptr).collect();
    let encoding = message_encoding.map_or(ptr::null(), CStr::as_ptr);
    // SAFETY: `buf` is the fresh, empty buffer held above, which libgit2 fills
    // (`git_buf_fromstr`) or leaves empty; the rest as in `create`: libgit2 reads
    // `repo`'s object database through this shared handle to check the tree and the
    // parents, whose ids are of its type.
    let rc = unsafe {
        ffi::git_commit_create_buffer(
            buf.as_mut().as_mut_ptr(),
            repo.as_ptr().cast_mut(),
            author.as_ptr(),
            committer.as_ptr(),
            encoding,
            message.as_ptr(),
            tree.as_ptr(),
            ptrs.len(),
            ptrs.as_mut_ptr(),
        )
    };
    if rc < 0 { failed(rc) } else { Ok(buf) }
}

/// [`create`], with what the commit gets besides its tree, parents, signatures and
/// message taken from `options` (the defaults of [`CreateExtOptions::new`] for
/// `None`): the reference to move to it, the `encoding` header, the extra headers
/// (after `committer` and `encoding`, in order), and the signing callback, which
/// libgit2 calls once with the contents before writing them.
///
/// The options are borrowed exclusively for the call, as their signing callback is:
/// libgit2 lends it the builder of the commit being written. A panic of its closure
/// stops the commit, which fails with `GIT_EUSER`, and resumes here once libgit2 has
/// returned. An error the closure returns fails the call with its code and message,
/// before anything is written.
///
/// Wraps: `git_commit_create_ext`
///
/// # Errors
///
/// Refuses what [`create`] refuses, and fails as it does, and with the signing
/// callback's error.
pub fn create_ext<'r>(
    repo: RepositoryRef<'r>,
    author: SignatureRef<'_>,
    committer: SignatureRef<'_>,
    message: &CStr,
    tree: TreeRef<'_, 'r>,
    parents: &[CommitRef<'_, 'r>],
    mut options: Option<CreateExtOptionsMut<'_, '_>>,
) -> Result<Oid, RepositoryError> {
    creatable(repo, author, committer, tree, parents)?;
    let mut id = Oid::zeroed();
    let out = id.as_mut().as_mut_ptr();
    let ptrs: Vec<_> = parents.iter().map(CommitRef::as_ptr).collect();
    let opts = options
        .as_ref()
        .map_or(ptr::null(), |o| o.as_ref().as_ptr());
    // SAFETY: as in `create`; `opts` is NULL or the live options this function borrows
    // exclusively, only read by libgit2 (with the strings and headers they borrow for
    // their `'o`), whose signing callback, if any, is the trampoline over the
    // `SignatureCallback` they borrow exclusively, which libgit2 calls on this thread
    // with the builder, the repository and the contents. `ptrs` is only read.
    let rc = unsafe {
        ffi::git_commit_create_ext(
            out,
            repo.as_ptr().cast_mut(),
            author.as_ptr(),
            committer.as_ptr(),
            message.as_ptr(),
            tree.as_ptr(),
            ptrs.len(),
            ptrs.as_ptr().cast(),
            opts,
        )
    };
    if let Some(options) = options.as_mut() {
        options.resume_signer_panic();
    }
    if rc < 0 { failed(rc) } else { Ok(id) }
}

impl<'r> CommitRef<'_, 'r> {
    /// Writes a copy of this commit into its repository's object database, with each
    /// given value in place of its own, and returns the copy's id: the same parents,
    /// and the commit's author, committer, `encoding` header, message and tree for each
    /// `None`. The commit itself is unchanged; a message encoding cannot be dropped.
    ///
    /// With an `update_ref`, the reference of that name (resolved through up to 5
    /// symbolic references) must point at this commit, and is moved to the copy, with a
    /// reflog entry; otherwise the call fails before writing anything, with
    /// `GIT_ERROR_REFERENCE`, "commit to amend is not the tip of the given branch", or
    /// libgit2's error when the reference cannot be read (a missing one included).
    ///
    /// Wraps: `git_commit_amend`
    ///
    /// # Errors
    ///
    /// Refuses what [`create`] refuses, for the signatures and the tree the copy gets
    /// (this commit's own included). Fails with libgit2's error when this commit's tree
    /// cannot be read, a reference cannot be written, or the object database cannot be
    /// written; libgit2 then leaks the reference it read for `update_ref`.
    pub fn amend(
        &self,
        update_ref: Option<&CStr>,
        author: Option<SignatureRef<'_>>,
        committer: Option<SignatureRef<'_>>,
        message_encoding: Option<&CStr>,
        message: Option<&CStr>,
        tree: Option<TreeRef<'_, 'r>>,
    ) -> Result<Oid, RepositoryError> {
        if let Some(tree) = tree {
            tree_in(self.owner(), tree)?;
        }
        writable(author.unwrap_or_else(|| self.author()))?;
        writable(committer.unwrap_or_else(|| self.committer()))?;
        let mut id = Oid::zeroed();
        let out = id.as_mut().as_mut_ptr();
        let sig = |s: Option<SignatureRef<'_>>| s.map_or(ptr::null(), |s| s.as_ptr());
        let string = |s: Option<&CStr>| s.map_or(ptr::null(), CStr::as_ptr);
        // SAFETY: `out` addresses `id`, written on success; the commit is live and only read,
        // its repository live for `'r` on this thread, written through as `RepositoryRef`
        // allows; the signatures are NULL or live, and those written (these or the
        // commit's) are writable (checked above); the strings are NULL or NUL-terminated;
        // `tree` is NULL or a live tree of the commit's repository (checked above).
        let rc = unsafe {
            ffi::git_commit_amend(
                out,
                self.as_ptr(),
                string(update_ref),
                sig(author),
                sig(committer),
                string(message_encoding),
                string(message),
                tree.map_or(ptr::null(), |t| t.as_ptr()),
            )
        };
        if rc < 0 { failed(rc) } else { Ok(id) }
    }
}

// --- Committing on `HEAD` --------------------------------------------------------------

/// `Ok` when libgit2 can write the signatures `options` set (see [`writable`]); the
/// refusal otherwise, set as the thread's error. An unset one is the repository's
/// default identity, which libgit2 builds whole, or the amended commit's author, which
/// [`head_commit`] checks.
fn options_writable(options: Option<CreateOptionsRef<'_, '_>>) -> Result<(), RepositoryError> {
    let Some(options) = options else {
        return Ok(());
    };
    if let Some(author) = options.author() {
        writable(author)?;
    }
    if let Some(committer) = options.committer() {
        writable(committer)?;
    }
    Ok(())
}

/// What a routine committing on `HEAD` commits: a tree, or the repository's index.
#[derive(Clone, Copy)]
enum Committed<'a, 'r> {
    Tree(TreeRef<'a, 'r>),
    Index(IndexRef<'a>),
}

/// `check_for_empty_commit` (static in libgit2's `commit.c`): `Ok` when what is committed
/// differs from the tree `HEAD` points at (from nothing, on an unborn `HEAD`); when it
/// does not, the failure `GIT_EUNCHANGED`, class `GIT_ERROR_REPOSITORY`, "no changes are
/// staged for commit", set as the thread's error; libgit2's error when `HEAD` cannot be
/// read or peeled to a tree, or the diff fails.
///
/// Composed from the routines it calls, in its order: `git_repository_head_tree`
/// (private to libgit2) as [`RepositoryRef::head`] and `git_reference_peel`, then
/// `git_diff_tree_to_tree` or `git_diff_tree_to_index` with the default options, whose
/// deltas are counted through the safe diff owner.
fn changes(repo: RepositoryRef<'_>, committed: Committed<'_, '_>) -> Result<(), RepositoryError> {
    let head = match repo.head() {
        Ok(head) => Some(head),
        Err(e) if e.code() == Some(ErrorCode::UnbornBranch) => None,
        Err(e) => return Err(e),
    };
    let head_tree = match head {
        Some(head) => {
            let mut tree = ptr::null_mut();
            // SAFETY: a writable slot; `head` is the live reference `git_repository_head`
            // handed out, of `repo`, used on this thread only and only read;
            // `GIT_OBJECT_TREE` is a target libgit2 accepts.
            let rc = unsafe {
                ffi::git_reference_peel(&raw mut tree, head.as_ref().as_ptr(), ffi::GIT_OBJECT_TREE)
            };
            if rc < 0 {
                return failed(rc);
            }
            // SAFETY: on success one counted reference to a live object of type
            // `GIT_OBJECT_TREE`, so a `git_tree`, of `repo`; the owner releases it.
            Some(unsafe { TreeArc::from_c(tree.cast()) }.expect("a peel succeeded without a tree"))
        }
        None => None,
    };
    let old = head_tree.as_ref().map(CArc::as_ref);
    let diff = match committed {
        Committed::Tree(tree) => Diff::tree_to_tree(repo, old, Some(tree)),
        Committed::Index(index) => Diff::tree_to_index(repo, old, Some(index)),
    }
    .map_err(|error| RepositoryError::from_libgit2(error.raw_code()))?;
    let deltas = diff.as_ref().len();
    drop(diff);
    if deltas == 0 {
        set(ErrorClass::Repository, c"no changes are staged for commit");
        return failed(ErrorCode::Unchanged.raw());
    }
    Ok(())
}

/// `create_from_tree` (static in libgit2's `commit.c`): writes a commit of `tree` and
/// moves `HEAD` to it, with the author and the committer `options` set (the repository's
/// default signature for each unset one), the parents a commit made now would have
/// ([`RepositoryRef::commit_parents`]), and the options' encoding, extra headers and
/// signer, through [`create_ext`].
///
/// libgit2's routine takes the parents from `git_repository_commit_parents`, whose
/// defects [`RepositoryRef::commit_parents`] avoids: it frees the commits it collected
/// while the object cache holds them when `MERGE_HEAD` cannot be read through (a use
/// after free), and takes whatever object `HEAD` resolves to as a commit. So `HEAD` at a
/// tag is peeled to its commit, which then fails as not the tip of `HEAD` (libgit2 writes
/// the tag as the first parent), and `HEAD` at another object fails (libgit2 writes it as
/// a parent).
///
/// Fails, as libgit2 does, with `-1` and the thread's error when the default signature
/// or the parents cannot be had, and otherwise as [`create_ext`] fails.
fn commit_on_head<'r>(
    repo: RepositoryRef<'r>,
    tree: TreeRef<'_, 'r>,
    message: &CStr,
    mut options: Option<CreateOptionsMut<'_, '_>>,
) -> Result<Oid, RepositoryError> {
    let opts = options.as_ref().map(|o| o.as_ref());
    let (author, committer) = (
        opts.and_then(|o| o.author()),
        opts.and_then(|o| o.committer()),
    );
    let default = if author.is_none() || committer.is_none() {
        let default = Signature::default(repo).or_else(|_| failed(ErrorCode::Generic.raw()))?;
        Some(default)
    } else {
        None
    };
    let default = default.as_ref().map(SignatureBox::as_ref);
    let (Some(author), Some(committer)) = (author.or(default), committer.or(default)) else {
        unreachable!("the default signature fills every unset one")
    };
    let parents = repo
        .commit_parents()
        .or_else(|_| failed(ErrorCode::Generic.raw()))?;
    let parents: Vec<_> = parents.as_ref().iter().collect();
    let write = |mut ext: CreateExtOptionsMut<'_, '_>| {
        ext.set_update_ref(Some(c"HEAD"));
        create_ext(repo, author, committer, message, tree, &parents, Some(ext))
    };
    match options.as_mut() {
        Some(options) => options.with_ext_options(write),
        None => write(CreateExtOptions::new().as_mut()),
    }
}

/// Commits the repository's index on `HEAD`, as `git commit -m <message>` does, and
/// returns the new commit's id: the index is written as a tree into the object database,
/// and a commit of that tree is written, whose parents are the commit `HEAD` resolves to
/// (none on an unborn `HEAD`) and, during a merge, each commit `MERGE_HEAD` names; `HEAD`
/// (its branch) is then moved to it, with a reflog entry.
///
/// The commit gets what `options` set (the defaults of [`CreateOptions::new`] for
/// `None`): the author and the committer, the repository's default signature
/// ([`Signature::default`]) for each unset one; the `encoding` header and the extra
/// headers; and the signing callback, which libgit2 calls once with the contents before
/// writing them. Unless the options allow it, a commit that changes nothing is refused:
/// when the index holds the tree `HEAD` points at (no entries, on an unborn `HEAD`), the
/// call fails with [`ErrorCode::Unchanged`] (class `GIT_ERROR_REPOSITORY`, "no changes are
/// staged for commit") before anything is written.
///
/// The options are borrowed exclusively for the call, as their signing callback is. A
/// panic of its closure stops the commit, which fails with `GIT_EUSER`, and resumes here
/// once libgit2 has returned; an error it returns fails the call with its code and
/// message. Either way the index's tree, written before signing, stays in the object
/// database, and `HEAD` does not move.
///
/// This is `git_commit_create_from_stage` composed from the routines it calls, the same
/// way, but for the parents, which come from [`RepositoryRef::commit_parents`]: libgit2's
/// own collection frees `HEAD`'s commit while the object cache still holds it when
/// `MERGE_HEAD` holds a line it cannot parse or that names no commit, and writes any
/// object `HEAD` resolves to as a parent. Here `HEAD` at a tag fails with
/// [`ErrorCode::Modified`] (its commit is not the tip `HEAD` names), and `HEAD` at a tree
/// or a blob with `-1`.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::src::libgit2::commit::{Commit, CreateOptions, create_from_stage};
/// use libgit2::libgit2::src::libgit2::repository::Repository;
/// use libgit2::libgit2::src::libgit2::signature::{SignatureView, Time};
/// use libgit2::libgit2::src::util::errors::ErrorCode;
///
/// let dir = std::env::temp_dir().join(format!("commit-stage-doc-{}", std::process::id()));
/// let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
/// let repo = Repository::init(&path, true).unwrap();
/// let me = SignatureView::new(
///     c"A U Thor",
///     c"author@example.com",
///     Time::new(1_700_000_000, 60),
/// );
/// let mut options = CreateOptions::new();
/// options.as_mut().set_author(Some(me.as_ref()));
/// options.as_mut().set_committer(Some(me.as_ref()));
/// // The index is empty, as the unborn `HEAD` is: nothing to commit.
/// let e = create_from_stage(repo.as_ref(), c"Initial\n", Some(options.as_mut())).unwrap_err();
/// assert_eq!(e.code(), Some(ErrorCode::Unchanged));
/// options.as_mut().set_allow_empty_commit(true);
/// let id = create_from_stage(repo.as_ref(), c"Initial\n", Some(options.as_mut())).unwrap();
/// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
/// assert_eq!(commit.as_ref().parent_count(), 0);
/// # drop(commit);
/// # drop(repo);
/// # std::fs::remove_dir_all(&dir).unwrap();
/// ```
///
/// Replaces: `git_commit_create_from_stage`
///
/// # Errors
///
/// Refuses, before libgit2 runs and with `GIT_ERROR_INVALID`, an author or committer
/// libgit2 cannot write (see [`create`]). Fails with `-1` and libgit2's error when the
/// index cannot be read or written as a tree, the default signature is needed and
/// missing (an unset or empty `user.name` or `user.email`), or the parents cannot be
/// read (`HEAD` at an object that is not a commit or a tag included); with libgit2's
/// error when the diff against `HEAD` fails, `HEAD` has moved meanwhile
/// ([`ErrorCode::Modified`]), or the reference or the object database cannot be written.
pub fn create_from_stage(
    repo: RepositoryRef<'_>,
    message: &CStr,
    options: Option<CreateOptionsMut<'_, '_>>,
) -> Result<Oid, RepositoryError> {
    let opts = options.as_ref().map(|o| o.as_ref());
    options_writable(opts)?;
    let index = repo.index().or_else(|_| failed(ErrorCode::Generic.raw()))?;
    if !opts.is_some_and(|o| o.allow_empty_commit()) {
        changes(repo, Committed::Index(index.as_ref()))?;
    }
    let mut id = Oid::zeroed();
    // SAFETY: `id` is a writable id, set on success; the index is the repository's, live
    // through the reference held above, written (sorted, its tree cache refreshed) on this
    // thread only, as `git_commit_create_from_stage` writes it: an index is neither `Send`
    // nor `Sync`, and no handle to it lends any of its contents. libgit2 writes the trees
    // into the object database of the repository that keeps the index.
    let rc = unsafe { ffi::git_index_write_tree(id.as_mut().as_mut_ptr(), index.as_c_ptr()) };
    if rc < 0 {
        return failed(ErrorCode::Generic.raw());
    }
    let tree = Tree::lookup(repo, id.as_ref()).or_else(|_| failed(ErrorCode::Generic.raw()))?;
    commit_on_head(repo, tree.as_ref(), message, options)
}

/// Commits `tree` on `HEAD` and returns the new commit's id: [`create_from_stage`], with
/// the given tree in place of the index's, written the same way and with the same
/// options, refusals and failures.
///
/// A commit that changes nothing (`tree` is the tree `HEAD` points at, or empty on an
/// unborn `HEAD`), unless the options allow it, fails with `-1` (`GIT_ERROR`), class
/// `GIT_ERROR_REPOSITORY`, "no changes are staged for commit", as libgit2 reports the
/// check here, rather than [`ErrorCode::Unchanged`]; so does a failure of the diff
/// against `HEAD`.
///
/// This is `git_commit_create_from_tree` composed from the routines it calls, as
/// [`create_from_stage`] is, for the same defects.
///
/// Replaces: `git_commit_create_from_tree`
///
/// # Errors
///
/// Refuses besides, before libgit2 runs, a `tree` looked up in another repository, with
/// the error a release build of libgit2 reports from its assertion (`GIT_ERROR_INVALID`,
/// "invalid argument: '`git_tree_owner(tree)` == repo'"; a debug build aborts), which it
/// reaches only after diffing that tree against `HEAD`.
pub fn create_from_tree<'r>(
    repo: RepositoryRef<'r>,
    tree: TreeRef<'_, 'r>,
    message: &CStr,
    options: Option<CreateOptionsMut<'_, '_>>,
) -> Result<Oid, RepositoryError> {
    tree_in(repo, tree)?;
    let opts = options.as_ref().map(|o| o.as_ref());
    options_writable(opts)?;
    if !opts.is_some_and(|o| o.allow_empty_commit()) {
        changes(repo, Committed::Tree(tree)).or_else(|_| failed(ErrorCode::Generic.raw()))?;
    }
    commit_on_head(repo, tree, message, options)
}

/// The commit `HEAD` resolves to, as `git_commit_amend_from_tree` peels it, when the
/// options leave the author unset and libgit2 writes that commit's author: `Ok(None)`
/// when the commit cannot be had (libgit2 then fails the same way, setting its own
/// error) or the options set an author, the refusal of [`writable`] for an author it
/// cannot write. Held for the call, the commit lends libgit2 the author checked here
/// ([`amend_on_head`]).
fn head_commit<'r>(
    repo: RepositoryRef<'r>,
    options: Option<CreateOptionsRef<'_, '_>>,
) -> Result<Option<CommitArc<'r>>, RepositoryError> {
    if options.and_then(|o| o.author()).is_some() {
        return Ok(None);
    }
    let Ok(head) = repo.head() else {
        return Ok(None);
    };
    let mut commit = ptr::null_mut();
    // SAFETY: a writable slot; `head` is the live reference `git_repository_head` handed
    // out, of `repo`, used on this thread only and only read; `GIT_OBJECT_COMMIT` is a
    // target libgit2 accepts.
    let rc = unsafe {
        ffi::git_reference_peel(
            &raw mut commit,
            head.as_ref().as_ptr(),
            ffi::GIT_OBJECT_COMMIT,
        )
    };
    if rc < 0 {
        return Ok(None);
    }
    // SAFETY: on success one counted reference to a live object of type
    // `GIT_OBJECT_COMMIT`, so a `git_commit`, of `repo`, borrowed for `'r`; the owner
    // releases it.
    let commit = unsafe { CommitArc::from_c(commit.cast()) };
    let commit = commit.expect("a peel succeeded without a commit");
    writable(commit.as_ref().author())?;
    Ok(Some(commit))
}

/// `git_commit_amend_from_tree` with `tree`, or `git_commit_amend_from_stage` for `None`,
/// after the refusals both wrappers make; resumes the signer's panic once libgit2 returns.
fn amend_on_head<'r>(
    repo: RepositoryRef<'r>,
    tree: Option<TreeRef<'_, 'r>>,
    message: Option<&CStr>,
    mut options: Option<CreateOptionsMut<'_, '_>>,
) -> Result<Oid, RepositoryError> {
    let opts = options.as_ref().map(|o| o.as_ref());
    options_writable(opts)?;
    let held = head_commit(repo, opts)?;
    // libgit2 writes `HEAD`'s commit's author from a parse of its own, which shares the
    // checked one only through the object cache, and that keeps no commit over 4096
    // bytes: the checked author is set in a copy of the options (C's defaults for none).
    let with_author = held.as_ref().map(|commit| {
        let mut copy = match opts {
            // SAFETY: a read of the live options through the shared handle's pointer.
            Some(o) => unsafe { o.as_ptr().read() },
            None => CreateOptions::new().0,
        };
        copy.author = commit.as_ref().author().as_ptr();
        copy
    });
    let mut id = Oid::zeroed();
    let (out, raw) = (id.as_mut().as_mut_ptr(), repo.as_ptr().cast_mut());
    let message = message.map_or(ptr::null(), CStr::as_ptr);
    let opts = match &with_author {
        Some(copy) => ptr::from_ref(copy),
        None => opts.map_or(ptr::null(), |o| o.as_ptr()),
    };
    // `opts` is NULL, the live options this function borrows exclusively, or their copy
    // with the author of `held`, released after the call; only read by libgit2, with the
    // signatures (writable, checked above), strings and headers they borrow for `'o`.
    // Their signing callback, if any, is the trampoline over the `SignatureCallback` they
    // borrow exclusively, which libgit2 calls on this thread, through the copy if made.
    let rc = match tree {
        // SAFETY: `out` addresses `id`, written on success; `repo` is live on this thread,
        // its index, references, odb and caches written through this shared handle as
        // `RepositoryRef` allows; `tree` is a live tree of `repo` (checked by the caller);
        // `message` is NULL or NUL-terminated; `opts` as above.
        Some(tree) => unsafe {
            ffi::git_commit_amend_from_tree(out, raw, tree.as_ptr(), message, opts)
        },
        // SAFETY: as above, without a tree: libgit2 commits the index's.
        None => unsafe { ffi::git_commit_amend_from_stage(out, raw, message, opts) },
    };
    drop(held);
    if let Some(options) = options.as_mut() {
        options.resume_signer_panic();
    }
    if rc < 0 { failed(rc) } else { Ok(id) }
}

/// Amends the commit `HEAD` resolves to with the repository's index, as `git commit
/// --amend` does, and returns the new commit's id: the index is written as a tree into
/// the object database, and a commit of that tree is written with the amended commit's
/// parents; `HEAD` (its branch) is then moved to it, with a reflog entry.
///
/// The commit gets `message`, or the amended commit's message *and its encoding*, which
/// then override the options' encoding, for `None`; the author `options` set, or the
/// amended commit's author; the committer they set, or the repository's default
/// signature ([`Signature::default`]); and their encoding, extra headers (the amended
/// commit's own are not carried over) and signing callback, as [`create_from_stage`]
/// takes them. A commit that changes nothing is written whatever the options say.
///
/// The options are borrowed exclusively for the call, as their signing callback is: a
/// panic of its closure stops the commit, which fails with `GIT_EUSER`, and resumes here
/// once libgit2 has returned; an error it returns fails the call with its code and
/// message. Either way the index's tree, written before signing, stays in the object
/// database, and `HEAD` does not move.
///
/// Wraps: `git_commit_amend_from_stage`
///
/// # Errors
///
/// Refuses, before libgit2 runs and with `GIT_ERROR_INVALID`, an author or committer
/// libgit2 cannot write (see [`create`]), the amended commit's author included when the
/// options set none. Fails with `-1` and libgit2's error when the index cannot be read or
/// written as a tree; with [`ErrorCode::UnbornBranch`] on an unborn `HEAD`, and with
/// libgit2's error when `HEAD` cannot be peeled to a commit, the default signature is
/// needed and missing, or the reference or the object database cannot be written.
pub fn amend_from_stage(
    repo: RepositoryRef<'_>,
    message: Option<&CStr>,
    options: Option<CreateOptionsMut<'_, '_>>,
) -> Result<Oid, RepositoryError> {
    amend_on_head(repo, None, message, options)
}

/// Amends the commit `HEAD` resolves to with `tree` and returns the new commit's id:
/// [`amend_from_stage`], with the given tree in place of the index's, written the same
/// way and with the same options, refusals and failures.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::src::libgit2::commit::{
///     Commit, CreateOptions, amend_from_tree, create_from_tree,
/// };
/// use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
/// use libgit2::libgit2::src::libgit2::repository::Repository;
/// use libgit2::libgit2::src::libgit2::signature::{SignatureView, Time};
/// use libgit2::libgit2::src::libgit2::tree::Tree;
///
/// let dir = std::env::temp_dir().join(format!("commit-amend-doc-{}", std::process::id()));
/// let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
/// let repo = Repository::init(&path, true).unwrap();
/// let tree_id =
///     Oid::from_string(c"4b825dc642cb6eb9a060e54bf8d69288fbee4904", OidType::Sha1).unwrap();
/// let tree = Tree::lookup(repo.as_ref(), tree_id.as_ref()).unwrap();
/// let me = SignatureView::new(
///     c"A U Thor",
///     c"author@example.com",
///     Time::new(1_700_000_000, 60),
/// );
/// let mut options = CreateOptions::new();
/// let mut m = options.as_mut();
/// m.set_allow_empty_commit(true);
/// m.set_author(Some(me.as_ref()));
/// m.set_committer(Some(me.as_ref()));
/// m.set_message_encoding(Some(c"ISO-8859-1"));
/// create_from_tree(
///     repo.as_ref(),
///     tree.as_ref(),
///     c"Initial\n",
///     Some(options.as_mut()),
/// )
/// .unwrap();
/// // The amended commit's message and encoding, the options' author.
/// let mut options = CreateOptions::new();
/// options.as_mut().set_committer(Some(me.as_ref()));
/// let id = amend_from_tree(repo.as_ref(), tree.as_ref(), None, Some(options.as_mut())).unwrap();
/// let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
/// assert_eq!(commit.as_ref().message(), c"Initial\n");
/// assert_eq!(commit.as_ref().message_encoding(), Some(c"ISO-8859-1"));
/// # drop(commit);
/// # drop(tree);
/// # drop(repo);
/// # std::fs::remove_dir_all(&dir).unwrap();
/// ```
///
/// Wraps: `git_commit_amend_from_tree`
///
/// # Errors
///
/// Refuses besides, before libgit2 runs, a `tree` looked up in another repository, with
/// the error [`create_from_tree`] reports for one: libgit2 does not check it here, and
/// writes its id, of whatever type, unchecked.
pub fn amend_from_tree<'r>(
    repo: RepositoryRef<'r>,
    tree: TreeRef<'_, 'r>,
    message: Option<&CStr>,
    options: Option<CreateOptionsMut<'_, '_>>,
) -> Result<Oid, RepositoryError> {
    tree_in(repo, tree)?;
    amend_on_head(repo, Some(tree), message, options)
}

/// The closure a [`CreateCallback`] calls: the commit's author, committer, message
/// encoding, message, tree and parents, each lent for the call.
type CreateFn<'f> = dyn for<'c> FnMut(
        SignatureRef<'c>,
        SignatureRef<'c>,
        Option<&'c CStr>,
        &'c CStr,
        TreeRef<'c, 'c>,
        &'c [CommitRef<'c, 'c>],
    ) -> Result<Option<Oid>, StreamError>
    + 'f;

/// A commit creation callback (`git_commit_create_cb`): a routine about to write a
/// commit (`git_rebase_commit`, for each commit it replays) calls it instead, on the
/// calling thread, with what the commit is made of, so that it can write the commit
/// another way (sign it with [`create_buffer`] and [`create_with_signature`], say).
///
/// Built over a closure with [`new`](Self::new), which is lent the author, the
/// committer, the message encoding (`None` for UTF-8), the message, the tree and the
/// parents for the call only, and returns:
///
/// - `Ok(Some(id))`: it wrote the commit, whose id the routine goes on with. The id must be of the
///   tree's repository's id type, which the routine looks it up in: another is refused, failing the
///   routine with `GIT_ERROR_INVALID`, "specified object ID type does not match repository object
///   ID type".
/// - `Ok(None)`: it wrote nothing (`GIT_PASSTHROUGH`), and the routine writes the commit itself, as
///   [`create`] does.
/// - `Err(error)`: the routine stops and fails with the error's code, class and message. An error
///   whose code is [`ErrorCode::Passthrough`] reads as `Ok(None)`.
///
/// A panic in the closure does not cross into C: the trampoline stops the routine with
/// `GIT_EUSER`, refuses every later call, and the panic resumes once control is back in
/// Rust, in the wrapper that ran the routine, or [`call`](Self::call).
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::src::libgit2::commit::{CommitRef, CreateCallback};
/// use libgit2::libgit2::src::libgit2::filter::StreamError;
/// use libgit2::libgit2::src::libgit2::oid::Oid;
/// use libgit2::libgit2::src::libgit2::signature::SignatureRef;
/// use libgit2::libgit2::src::libgit2::tree::TreeRef;
///
/// let mut seen = 0;
/// let mut f = |_: SignatureRef<'_>,
///              _: SignatureRef<'_>,
///              _: Option<&core::ffi::CStr>,
///              _: &core::ffi::CStr,
///              _: TreeRef<'_, '_>,
///              parents: &[CommitRef<'_, '_>]| {
///     seen += parents.len();
///     Ok::<Option<Oid>, StreamError>(None)
/// };
/// let callback = CreateCallback::new(&mut f);
/// # drop(callback);
/// ```
///
/// The handles cannot outlive the call:
///
/// ```compile_fail
/// use {libgit2::libgit2::src::libgit2::signature::SignatureRef, libgit2::libgit2::src::libgit2::filter::StreamError};
/// use {libgit2::libgit2::src::libgit2::commit::CommitRef, libgit2::libgit2::src::libgit2::commit::CreateCallback};
/// use libgit2::libgit2::src::libgit2::tree::TreeRef;
/// use libgit2::libgit2::src::libgit2::oid::Oid;
///
/// fn keep<'k>(kept: &mut Option<TreeRef<'k, 'k>>) {
///     let mut f = |_: SignatureRef<'_>, _: SignatureRef<'_>, _: Option<&core::ffi::CStr>,
///                  _: &core::ffi::CStr, tree: TreeRef<'_, '_>, _: &[CommitRef<'_, '_>]| {
///         *kept = Some(tree);
///         Ok::<Option<Oid>, StreamError>(None)
///     };
///     let _ = CreateCallback::new(&mut f);
/// }
/// ```
///
/// It borrows its closure exclusively and stays on its thread: libgit2 calls it
/// synchronously, and the closure need not be `Send`.
///
/// ```compile_fail,E0277
/// use {libgit2::libgit2::src::libgit2::signature::SignatureRef, libgit2::libgit2::src::libgit2::filter::StreamError};
/// use {libgit2::libgit2::src::libgit2::commit::CommitRef, libgit2::libgit2::src::libgit2::commit::CreateCallback};
/// use libgit2::libgit2::src::libgit2::tree::TreeRef;
/// use libgit2::libgit2::src::libgit2::oid::Oid;
///
/// fn send<T: Send>(_: T) {}
/// let mut f = |_: SignatureRef<'_>, _: SignatureRef<'_>, _: Option<&core::ffi::CStr>,
///              _: &core::ffi::CStr, _: TreeRef<'_, '_>, _: &[CommitRef<'_, '_>]| {
///     Ok::<Option<Oid>, StreamError>(None)
/// };
/// send(CreateCallback::new(&mut f));
/// ```
///
/// Wraps: `git_commit_create_cb`
/// Handwritten
pub struct CreateCallback<'f> {
    f: &'f mut CreateFn<'f>,
    /// A panic the closure raised inside a C call, resumed once that call returns.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> CreateCallback<'f> {
    /// The callback that calls `f` for each commit a routine is about to write.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: for<'c> FnMut(
                SignatureRef<'c>,
                SignatureRef<'c>,
                Option<&'c CStr>,
                &'c CStr,
                TreeRef<'c, 'c>,
                &'c [CommitRef<'c, 'c>],
            ) -> Result<Option<Oid>, StreamError>
            + 'f,
    {
        Self { f, panic: None }
    }

    /// Calls the callback for a commit of `tree` and `parents`, through the trampoline
    /// libgit2 calls, and returns what libgit2 would read from it: `Some` of the id it
    /// wrote, `None` for a passthrough, or the error it failed with (its code, class and
    /// message), the refusal of an id of another type than the tree's repository's
    /// included. A panic of the closure resumes here.
    ///
    /// # Errors
    ///
    /// Fails with the error libgit2 would read from the callback: the closure's own, or the refusal
    /// of an id of another type than the tree's repository's.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: the trampoline is never NULL"
    )]
    pub fn call(
        &mut self,
        author: SignatureRef<'_>,
        committer: SignatureRef<'_>,
        message_encoding: Option<&CStr>,
        message: &CStr,
        tree: TreeRef<'_, '_>,
        parents: &[CommitRef<'_, '_>],
    ) -> Result<Option<Oid>, RepositoryError> {
        let mut id = Oid::zeroed();
        let out = id.as_mut().as_mut_ptr();
        let mut ptrs: Vec<_> = parents.iter().map(CommitRef::as_ptr).collect();
        let encoding = message_encoding.map_or(ptr::null(), CStr::as_ptr);
        let rc = self.create_with(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            // SAFETY: the trampoline's contract: `payload` is this callback, used by
            // nothing else during the call; `out` addresses `id`; the signatures, the
            // tree and the `ptrs.len()` commits `ptrs` addresses are live for the call,
            // as their handles vouch; the strings are NULL or NUL-terminated.
            unsafe {
                callback(
                    out,
                    author.as_ptr(),
                    committer.as_ptr(),
                    encoding,
                    message.as_ptr(),
                    tree.as_ptr(),
                    ptrs.len(),
                    ptrs.as_mut_ptr(),
                    payload,
                )
            }
        });
        match rc {
            0 => Ok(Some(id)),
            rc if rc == ErrorCode::Passthrough.raw() => Ok(None),
            rc => failed(rc),
        }
    }

    /// Runs `create` with the C callback and its payload, as the wrapper of a routine
    /// that calls it (`git_rebase_commit`, through its options' `commit_create_cb` and
    /// `payload`) hands them to it, then resumes a panic the closure raised meanwhile.
    /// The payload is this callback, which nothing else touches until `create` returns.
    pub(crate) fn create_with<R>(
        &mut self,
        create: impl FnOnce(ffi::git_commit_create_cb, *mut c_void) -> R,
    ) -> R {
        let result = create(Some(create_with_closure), ptr::from_mut(self).cast());
        if let Some(panic) = self.panic.take() {
            resume_unwind(panic);
        }
        result
    }
}

impl fmt::Debug for CreateCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateCallback")
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_commit_create_cb` over a [`CreateCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`CreateCallback`] that nothing else uses during the
/// call; `out` must be NULL or writable; `author` and `committer` NULL or live
/// signatures whose strings are NULL or NUL-terminated; `message_encoding` and
/// `message` NULL or NUL-terminated; `tree` NULL or a live tree whose repository is
/// live and used on this thread only; `parents` NULL or addressing `parent_count`
/// pointers, each NULL or a live commit; all of them live and unwritten for the call.
unsafe extern "C" fn create_with_closure(
    out: *mut ffi::git_oid,
    author: *const ffi::git_signature,
    committer: *const ffi::git_signature,
    message_encoding: *const c_char,
    message: *const c_char,
    tree: *const ffi::git_tree,
    parent_count: usize,
    parents: *mut *const ffi::git_commit,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: per this function's contract, `payload` is a live callback borrowed for
    // the call alone.
    let callback = unsafe { &mut *payload.cast::<CreateCallback<'_>>() };
    if callback.panic.is_some() {
        return ErrorCode::User.raw();
    }
    // SAFETY: per this function's contract, a non-NULL signature is live and unwritten for the
    // call, with NULL or NUL-terminated strings; the handle lives for the call.
    let author = unsafe { SignatureRef::from_ptr(author.cast_mut()) };
    // SAFETY: as above, for the committer.
    let committer = unsafe { SignatureRef::from_ptr(committer.cast_mut()) };
    // SAFETY: per this function's contract, a non-NULL tree is live for the call, of a
    // repository live and used on this thread only meanwhile.
    let tree = unsafe { TreeRef::from_ptr(tree) };
    let (Some(author), Some(committer), Some(tree), false, false) =
        (author, committer, tree, message.is_null(), out.is_null())
    else {
        // libgit2 never passes NULL; stop the commit rather than invent an argument.
        return ErrorCode::User.raw();
    };
    // SAFETY: per this function's contract, the message is NUL-terminated and live for the call.
    let message = unsafe { CStr::from_ptr(message) };
    // SAFETY: as above, for a non-NULL encoding.
    let encoding =
        (!message_encoding.is_null()).then(|| unsafe { CStr::from_ptr(message_encoding) });
    let raw: &[*const ffi::git_commit] = match (parent_count, parents.is_null()) {
        (0, _) => &[],
        (_, true) => return ErrorCode::User.raw(),
        // SAFETY: per this function's contract, `parents` addresses `parent_count`
        // pointers, live and unwritten for the call.
        (n, false) => unsafe { core::slice::from_raw_parts(parents, n) },
    };
    let parents: Option<Vec<_>> = raw
        .iter()
        // SAFETY: per this function's contract, each non-NULL pointer is a live commit
        // for the call; the handles live no longer.
        .map(|&p| unsafe { CommitRef::from_ptr(p) })
        .collect();
    let Some(parents) = parents else {
        return ErrorCode::User.raw();
    };
    let kind = tree.owner().oid_type();
    let result = catch_unwind(AssertUnwindSafe(|| {
        (callback.f)(author, committer, encoding, message, tree, &parents)
    }));
    match result {
        Ok(Ok(Some(id))) if u32::from(id.as_ref().raw_kind()) != kind.raw() => {
            set(
                ErrorClass::Invalid,
                c"specified object ID type does not match repository object ID type",
            );
            ErrorCode::Generic.raw()
        }
        Ok(Ok(Some(id))) => {
            // SAFETY: the id's own pointer addresses a whole, initialised `git_oid`.
            let raw_id = unsafe { id.as_ref().as_ptr().read() };
            // SAFETY: `out` is writable, per this function's contract.
            unsafe { out.write(raw_id) };
            0
        }
        Ok(Ok(None)) => ErrorCode::Passthrough.raw(),
        Ok(Err(error)) => error.raise(),
        Err(panic) => {
            callback.panic = Some(panic);
            ErrorCode::User.raw()
        }
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
/// use libgit2::libgit2::src::libgit2::commit::Header;
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
/// use libgit2::libgit2::src::libgit2::commit::Header;
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
/// use libgit2::libgit2::src::libgit2::commit::HeaderMut;
///
/// fn shrink<'a, 'h>(m: HeaderMut<'a, 'static>) -> HeaderMut<'a, 'h> {
///     m
/// }
/// ```
///
/// The shared handle only reads:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::src::libgit2::commit::Header;
///
/// let header = Header::new(c"field", c"value");
/// header.as_ref().set_value(c"other");
/// ```
///
/// `Copy`, `Send` and `Sync`, as a pair of `&'h CStr`s is: libgit2 never writes
/// through the strings, and only the exclusive handle writes the fields. Equal when
/// both strings are.
///
/// Wraps: `git_commit_header`
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
    type Mut<'a>
        = HeaderMut<'a, 'h>
    where
        Self: 'a;
    type Ref<'a>
        = HeaderRef<'a, 'h>
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
    #[must_use]
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
    #[must_use]
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

impl<'h> HeaderRef<'_, 'h> {
    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_commit_header *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_commit_header {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The field name, borrowed for `'h`, as it was set.
    ///
    /// Field: `git_commit_header.field`
    #[must_use]
    pub fn field(&self) -> &'h CStr {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses a live header; the projection forms no
        // reference to it.
        let field_ptr = unsafe { &raw const (*p).field };
        // SAFETY: reads the initialised field; nothing writes it while the header is shared.
        let field = unsafe { field_ptr.read() };
        // SAFETY: `new` and every setter store a `&'h CStr`'s pointer, a NUL-terminated string that
        // outlives `'h` and that nothing writes, being shared-borrowed.
        unsafe { CStr::from_ptr(field) }
    }

    /// The value, borrowed for `'h`, as it was set.
    ///
    /// Field: `git_commit_header.value`
    #[must_use]
    pub fn value(&self) -> &'h CStr {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses a live header; the projection forms no
        // reference to it.
        let value_ptr = unsafe { &raw const (*p).value };
        // SAFETY: reads the initialised field; nothing writes it while the header is shared.
        let value = unsafe { value_ptr.read() };
        // SAFETY: `new` and every setter store a `&'h CStr`'s pointer, a NUL-terminated string that
        // outlives `'h` and that nothing writes, being shared-borrowed.
        unsafe { CStr::from_ptr(value) }
    }

    /// A copy of the header, as a value of its own, borrowing the same strings.
    #[must_use]
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
    #[must_use]
    pub fn as_ref(&self) -> HeaderRef<'_, 'h> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> HeaderMut<'_, 'h> {
        HeaderMut(self.0, PhantomData)
    }

    /// Replaces the field name; the old string, borrowed, needs no release.
    ///
    /// Field: `git_commit_header.field`
    pub fn set_field(&mut self, field: &'h CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let field_ptr = unsafe { &raw mut (*p).field };
        // SAFETY: a write through the exclusive handle's pointer to a live header of a
        // `&'h CStr`'s pointer, which keeps the getters' guarantee; the old pointer is
        // borrowed, so nothing is released.
        unsafe { field_ptr.write(field.as_ptr()) }
    }

    /// Replaces the value; the old string, borrowed, needs no release.
    ///
    /// Field: `git_commit_header.value`
    pub fn set_value(&mut self, value: &'h CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let value_ptr = unsafe { &raw mut (*p).value };
        // SAFETY: a write through the exclusive handle's pointer to a live header of a
        // `&'h CStr`'s pointer, which keeps the getters' guarantee; the old pointer is
        // borrowed, so nothing is released.
        unsafe { value_ptr.write(value.as_ptr()) }
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
/// use libgit2::libgit2::src::libgit2::commit::{CreateExtOptions, Header};
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
/// use libgit2::libgit2::src::libgit2::commit::CreateExtOptions;
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
/// use libgit2::libgit2::src::libgit2::commit::CreateExtOptionsMut;
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
/// use libgit2::libgit2::src::libgit2::commit::CreateExtOptions;
///
/// let options = CreateExtOptions::new();
/// std::thread::spawn(move || drop(options));
/// ```
///
/// Not `Clone`: the payload its callback receives may be an exclusive borrow, which a
/// copy would alias. Build another with [`new`](Self::new) instead.
///
/// Wraps: `git_commit_create_ext_options`
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
    type Mut<'a>
        = CreateExtOptionsMut<'a, 'o>
    where
        Self: 'a;
    type Ref<'a>
        = CreateExtOptionsRef<'a, 'o>
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
    /// Wraps: `git_commit_create_ext_options_init`
    ///
    /// # Panics
    ///
    /// Panics only if libgit2 refuses the version of the options it defines itself, which it never
    /// does.
    #[must_use]
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
    #[must_use]
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

impl<'o> CreateExtOptionsRef<'_, 'o> {
    /// Borrows the options at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must address live, initialised options (or be NULL) that outlive `'a` and
    /// that nothing writes while the handle lives, whose strings are NULL or
    /// NUL-terminated and outlive `'o`, and whose `extra_headers` is NULL or addresses
    /// `extra_headers_len` headers whose strings are non-NULL and NUL-terminated, all
    /// outliving `'o`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
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
    /// Field: `git_commit_create_ext_options.version`
    #[must_use]
    pub fn version(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let version_ptr = unsafe { &raw const (*p).version };
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { version_ptr.read() }
    }

    /// The name of the reference moved to the new commit; `None`, the default, moves
    /// none.
    ///
    /// Field: `git_commit_create_ext_options.update_ref`
    #[must_use]
    pub fn update_ref(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let update_ref_ptr = unsafe { &raw const (*p).update_ref };
        // SAFETY: reads the initialised field; nothing writes it while the options are shared.
        let s = unsafe { update_ref_ptr.read() };
        // SAFETY: every setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) })
    }

    /// The message's encoding, written as the commit's `encoding` header; `None`, the
    /// default, writes no such header (git reads the message as UTF-8).
    ///
    /// Field: `git_commit_create_ext_options.message_encoding`
    #[must_use]
    pub fn message_encoding(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let message_encoding_ptr = unsafe { &raw const (*p).message_encoding };
        // SAFETY: reads the initialised field; nothing writes it while the options are shared.
        let s = unsafe { message_encoding_ptr.read() };
        // SAFETY: every setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) })
    }

    /// The extra headers written after `committer` and any `encoding`, in order;
    /// empty, the default, writes none. Borrowed for `'o`, as they were set; each reads
    /// through its [`HeaderRef`].
    ///
    /// Field: `git_commit_create_ext_options.extra_headers`
    /// Field: `git_commit_create_ext_options.extra_headers_len`
    #[must_use]
    pub fn extra_headers(&self) -> CSlice<'o, Header<'o>> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let headers_ptr = unsafe { &raw const (*p).extra_headers };
        // SAFETY: as above.
        let len_ptr = unsafe { &raw const (*p).extra_headers_len };
        // SAFETY: reads the initialised field; nothing writes it while the options are shared.
        let headers = unsafe { headers_ptr.read() };
        // SAFETY: as above.
        let len = unsafe { len_ptr.read() };
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
    /// Field: `git_commit_create_ext_options.sign`
    /// Field: `git_commit_create_ext_options.payload`
    #[must_use]
    pub fn has_signer(&self) -> bool {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let sign_ptr = unsafe { &raw const (*p).sign };
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { sign_ptr.read() }.is_some()
    }

    /// The signing callback and its payload as libgit2 reads them: `None` and NULL by
    /// default, otherwise the trampoline of the [`SignatureCallback`] that
    /// [`set_signer`](CreateExtOptionsMut::set_signer) stored, and that callback. For the
    /// unit tests, which hand them to libgit2 themselves.
    #[cfg(test)]
    pub(crate) fn sign_callback(&self) -> (ffi::git_commit_signature_cb, *mut c_void) {
        let p = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to live options.
        let sign_ptr2 = unsafe { &raw const (*p).sign };
        // SAFETY: as above.
        let read2 = unsafe { sign_ptr2.read() };
        // SAFETY: as above.
        let payload_ptr = unsafe { &raw const (*p).payload };
        // SAFETY: as above.
        unsafe { (read2, payload_ptr.read()) }
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

impl<'o> CreateExtOptionsMut<'_, 'o> {
    /// Borrows the options at `ptr` exclusively; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// As [`CreateExtOptionsRef::from_ptr`], plus: nothing else reads or writes the
    /// options while the result lives, whatever the setters store may stay there for
    /// as long as the options' owner expects `'o` borrows to, and a non-NULL `payload`
    /// addresses a [`SignatureCallback`] the options borrow exclusively for `'o`, as
    /// [`set_signer`](Self::set_signer) stores one.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
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
    #[must_use]
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
    /// Field: `git_commit_create_ext_options.update_ref`
    pub fn set_update_ref(&mut self, name: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let update_ref_ptr = unsafe { &raw mut (*p).update_ref };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { update_ref_ptr.write(name.map_or(ptr::null(), CStr::as_ptr)) }
    }

    /// Sets the message's encoding, written as the commit's `encoding` header as given;
    /// `None` writes no such header.
    ///
    /// Field: `git_commit_create_ext_options.message_encoding`
    pub fn set_message_encoding(&mut self, encoding: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let encoding_ptr = unsafe { &raw mut (*p).message_encoding };
        // SAFETY: the field is plain data, so the old value needs no drop.
        unsafe { encoding_ptr.write(encoding.map_or(ptr::null(), CStr::as_ptr)) };
    }

    /// Sets the extra headers, written in order after `committer` and any `encoding`,
    /// each as `<field> <value>`; an empty run writes none, and is stored as C stores
    /// it, NULL with a length of 0. Borrowed for `'o`, run and strings alike.
    ///
    /// Field: `git_commit_create_ext_options.extra_headers`
    /// Field: `git_commit_create_ext_options.extra_headers_len`
    pub fn set_extra_headers(&mut self, headers: &'o [Header<'o>]) {
        let run: *const ffi::git_commit_header = if headers.is_empty() {
            ptr::null()
        } else {
            headers.as_ptr().cast()
        };
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let headers_ptr = unsafe { &raw mut (*p).extra_headers };
        // SAFETY: as above.
        let len_ptr = unsafe { &raw mut (*p).extra_headers_len };
        // SAFETY: the fields are plain data, so the old values need no drop. `run` is NULL with a
        // length of 0, or `headers`' own pointer and length, a run of initialised
        // `git_commit_header`s (`Header` is transparent over it) whose strings are non-NULL, as
        // `extra_headers()` requires.
        unsafe { headers_ptr.write(run) };
        // SAFETY: as above, the length of that run.
        unsafe { len_ptr.write(headers.len()) };
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
    /// callback, which [`create_ext`] resumes once libgit2 returns (and
    /// [`resume_panic`](SignatureCallback::resume_panic) resumes for options lent to C
    /// another way).
    ///
    /// ```compile_fail,E0499
    /// use libgit2::libgit2::src::libgit2::filter::StreamError;
    /// use {libgit2::libgit2::src::libgit2::commit::CommitBuilderMut, libgit2::libgit2::src::libgit2::commit::CreateExtOptions, libgit2::libgit2::src::libgit2::commit::SignatureCallback};
    /// use libgit2::libgit2::src::libgit2::repository::RepositoryRef;
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
    /// Field: `git_commit_create_ext_options.sign`
    /// Field: `git_commit_create_ext_options.payload`
    pub fn set_signer(&mut self, signer: Option<&'o mut SignatureCallback<'_>>) {
        let (callback, payload): (ffi::git_commit_signature_cb, *mut c_void) = match signer {
            Some(signer) => (Some(sign_with_closure), ptr::from_mut(signer).cast()),
            None => (None, ptr::null_mut()),
        };
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let sign_ptr = unsafe { &raw mut (*p).sign };
        // SAFETY: as above.
        let payload_ptr = unsafe { &raw mut (*p).payload };
        // SAFETY: the field is plain data, so the old value needs no drop; the trampoline is sound
        // with libgit2's arguments and the payload written next.
        unsafe { sign_ptr.write(callback) };
        // SAFETY: as above; the payload is the callback the options borrow exclusively for `'o`.
        unsafe { payload_ptr.write(payload) };
    }
}

impl CreateExtOptionsMut<'_, '_> {
    /// Resumes a panic the signing callback's closure raised during the libgit2 call
    /// these options were just lent to, if any: what [`create_ext`] does once libgit2
    /// returns.
    fn resume_signer_panic(&mut self) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let payload_ptr = unsafe { &raw const (*p).payload };
        // SAFETY: a read through the exclusive handle's pointer to live options.
        let payload = unsafe { payload_ptr.read() };
        if payload.is_null() {
            return;
        }
        // SAFETY: a non-NULL payload is the `SignatureCallback` `set_signer` stored (or
        // `from_ptr`'s caller vouched for), which the options borrow exclusively for
        // `'o`; this handle borrows the options exclusively, and the libgit2 call that
        // used the callback has returned, so nothing else reaches it.
        let signer = unsafe { &mut *payload.cast::<SignatureCallback<'_>>() };
        signer.resume_panic();
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

/// Options for committing the index or a tree on `HEAD` with [`create_from_stage`],
/// [`create_from_tree`] and their `amend` counterparts, [`amend_from_stage`] and
/// [`amend_from_tree`] (`git_commit_create_options`), held by value in Rust storage.
///
/// Built by [`new`](Self::new) (or [`Default`]) with libgit2's defaults: no empty
/// commits, the default author and committer, no encoding header, no extra headers and
/// no signing. Read it through [`as_ref`](Self::as_ref) and change it through
/// [`as_mut`](Self::as_mut):
///
/// ```
/// use libgit2::libgit2::src::libgit2::commit::{CreateOptions, Header};
/// use libgit2::libgit2::src::libgit2::signature::{SignatureView, Time};
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
/// use {libgit2::libgit2::src::libgit2::signature::SignatureView, libgit2::libgit2::src::libgit2::signature::Time};
/// use libgit2::libgit2::src::libgit2::commit::CreateOptions;
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
/// use libgit2::libgit2::src::libgit2::commit::CreateOptionsMut;
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
/// use libgit2::libgit2::src::libgit2::commit::CreateOptions;
///
/// let options = CreateOptions::new();
/// std::thread::spawn(move || drop(options));
/// ```
///
/// Not `Clone`: the payload its callback receives may be an exclusive borrow, which a
/// copy would alias. Build another with [`new`](Self::new) instead.
///
/// Wraps: `git_commit_create_options`
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
    type Mut<'a>
        = CreateOptionsMut<'a, 'o>
    where
        Self: 'a;
    type Ref<'a>
        = CreateOptionsRef<'a, 'o>
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
    /// Wraps: `git_commit_create_options_init`
    ///
    /// # Panics
    ///
    /// Panics only if libgit2 refuses the version of the options it defines itself, which it never
    /// does.
    #[must_use]
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
    #[must_use]
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

impl<'o> CreateOptionsRef<'_, 'o> {
    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_commit_create_options *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_commit_create_options {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The structure's version, `GIT_COMMIT_CREATE_OPTIONS_VERSION`; fixed by
    /// [`CreateOptions::new`].
    ///
    /// Field: `git_commit_create_options.version`
    #[must_use]
    pub fn version(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let version_ptr = unsafe { &raw const (*p).version };
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { version_ptr.read() }
    }

    /// Whether a commit that changes nothing is written; `false`, the default, fails
    /// the commit (class `GIT_ERROR_REPOSITORY`, "no changes are staged for commit")
    /// when the tree committed (the index's, for `git_commit_create_from_stage`) is the
    /// one `HEAD` points at, or empty on an unborn `HEAD`, before anything is written:
    /// `git_commit_create_from_stage` returns `GIT_EUNCHANGED`, while
    /// `git_commit_create_from_tree` returns `-1` (`GIT_ERROR`) for the same check.
    /// Only the routines creating a commit read it; amending ignores it.
    ///
    /// Field: `git_commit_create_options.allow_empty_commit`
    #[must_use]
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
    /// Field: `git_commit_create_options.author`
    #[must_use]
    pub fn author(&self) -> Option<SignatureRef<'o>> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let author_ptr = unsafe { &raw const (*p).author };
        // SAFETY: reads the initialised field; nothing writes it while the options are shared.
        let author = unsafe { author_ptr.read() };
        // SAFETY: every setter stores NULL or a `SignatureRef<'o>`'s pointer: a live signature that
        // outlives `'o` and that nothing writes, being shared-borrowed, whose strings are NULL or
        // NUL-terminated and outlive it.
        unsafe { SignatureRef::from_ptr(author.cast_mut()) }
    }

    /// The commit's committer; `None`, the default, is the repository's default
    /// signature, when creating and amending alike. Borrowed for `'o`, as it was set.
    ///
    /// Field: `git_commit_create_options.committer`
    #[must_use]
    pub fn committer(&self) -> Option<SignatureRef<'o>> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let committer_ptr = unsafe { &raw const (*p).committer };
        // SAFETY: reads the initialised field; nothing writes it while the options are shared.
        let committer = unsafe { committer_ptr.read() };
        // SAFETY: as `author`: every setter stores NULL or a `SignatureRef<'o>`'s pointer, live and
        // unwritten for `'o`.
        unsafe { SignatureRef::from_ptr(committer.cast_mut()) }
    }

    /// The message's encoding, written as the commit's `encoding` header; `None`, the
    /// default, writes no such header (git reads the message as UTF-8). Amending
    /// without a new message keeps the amended commit's encoding instead.
    ///
    /// Field: `git_commit_create_options.message_encoding`
    #[must_use]
    pub fn message_encoding(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let message_encoding_ptr = unsafe { &raw const (*p).message_encoding };
        // SAFETY: reads the initialised field; nothing writes it while the options are shared.
        let s = unsafe { message_encoding_ptr.read() };
        // SAFETY: every setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) })
    }

    /// The extra headers written after `committer` and any `encoding`, in order;
    /// empty, the default, writes none. Borrowed for `'o`, as they were set; each reads
    /// through its [`HeaderRef`].
    ///
    /// Field: `git_commit_create_options.extra_headers`
    /// Field: `git_commit_create_options.extra_headers_len`
    #[must_use]
    pub fn extra_headers(&self) -> CSlice<'o, Header<'o>> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let headers_ptr = unsafe { &raw const (*p).extra_headers };
        // SAFETY: as above.
        let len_ptr = unsafe { &raw const (*p).extra_headers_len };
        // SAFETY: reads the initialised field; nothing writes it while the options are shared.
        let headers = unsafe { headers_ptr.read() };
        // SAFETY: as above.
        let len = unsafe { len_ptr.read() };
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
    /// Field: `git_commit_create_options.sign`
    /// Field: `git_commit_create_options.payload`
    #[must_use]
    pub fn has_signer(&self) -> bool {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let sign_ptr = unsafe { &raw const (*p).sign };
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { sign_ptr.read() }.is_some()
    }

    /// The signing callback and its payload as libgit2 reads them: `None` and NULL by
    /// default, otherwise the trampoline of the [`SignatureCallback`] that
    /// [`set_signer`](CreateOptionsMut::set_signer) stored, and that callback. For the
    /// unit tests.
    #[cfg(test)]
    pub(crate) fn sign_callback(&self) -> (ffi::git_commit_signature_cb, *mut c_void) {
        let p = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to live options.
        let sign_ptr2 = unsafe { &raw const (*p).sign };
        // SAFETY: as above.
        let read2 = unsafe { sign_ptr2.read() };
        // SAFETY: as above.
        let payload_ptr = unsafe { &raw const (*p).payload };
        // SAFETY: as above.
        unsafe { (read2, payload_ptr.read()) }
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

impl<'o> CreateOptionsMut<'_, 'o> {
    /// Writable pointer to the C struct, for the setters.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_commit_create_options {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    #[must_use]
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
    /// Field: `git_commit_create_options.allow_empty_commit`
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
    /// email (see the [signature docs](crate::libgit2::src::libgit2::signature#signatures)), which
    /// the options store as given.
    ///
    /// Field: `git_commit_create_options.author`
    pub fn set_author(&mut self, author: Option<SignatureRef<'o>>) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let author_ptr = unsafe { &raw mut (*p).author };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop. The pointer is NULL or a
        // `SignatureRef<'o>`'s, live and unwritten for `'o`, as `author()` requires.
        unsafe { author_ptr.write(author.map_or(ptr::null(), |s| s.as_ptr())) }
    }

    /// Sets the commit's committer; `None` takes the default (see
    /// [`CreateOptionsRef::committer`]). Borrowed for `'o`; refused by the writing
    /// wrappers when it lost a string, as [`set_author`](Self::set_author)'s is.
    ///
    /// Field: `git_commit_create_options.committer`
    pub fn set_committer(&mut self, committer: Option<SignatureRef<'o>>) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let committer_ptr = unsafe { &raw mut (*p).committer };
        // SAFETY: as `set_author`.
        unsafe { committer_ptr.write(committer.map_or(ptr::null(), |s| s.as_ptr())) }
    }

    /// Sets the message's encoding, written as the commit's `encoding` header as given;
    /// `None` writes no such header.
    ///
    /// Field: `git_commit_create_options.message_encoding`
    pub fn set_message_encoding(&mut self, encoding: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let encoding_ptr = unsafe { &raw mut (*p).message_encoding };
        // SAFETY: the field is plain data, so the old value needs no drop.
        unsafe { encoding_ptr.write(encoding.map_or(ptr::null(), CStr::as_ptr)) };
    }

    /// Sets the extra headers, written in order after `committer` and any `encoding`,
    /// each as `<field> <value>`; an empty run writes none, and is stored as C stores
    /// it, NULL with a length of 0. Borrowed for `'o`, run and strings alike.
    ///
    /// Field: `git_commit_create_options.extra_headers`
    /// Field: `git_commit_create_options.extra_headers_len`
    pub fn set_extra_headers(&mut self, headers: &'o [Header<'o>]) {
        let run: *const ffi::git_commit_header = if headers.is_empty() {
            ptr::null()
        } else {
            headers.as_ptr().cast()
        };
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let headers_ptr = unsafe { &raw mut (*p).extra_headers };
        // SAFETY: as above.
        let len_ptr = unsafe { &raw mut (*p).extra_headers_len };
        // SAFETY: the fields are plain data, so the old values need no drop. `run` is NULL with a
        // length of 0, or `headers`' own pointer and length, a run of initialised
        // `git_commit_header`s (`Header` is transparent over it) whose strings are non-NULL, as
        // `extra_headers()` requires.
        unsafe { headers_ptr.write(run) };
        // SAFETY: as above, the length of that run.
        unsafe { len_ptr.write(headers.len()) };
    }

    /// Sets the signing callback, borrowed exclusively for `'o`; `None` signs nothing.
    /// libgit2 calls it once per commit created or amended with these options, after
    /// building the commit and before writing it, with the builder to add the
    /// signature header to (see [`SignatureCallback`]); an error fails the call before
    /// the commit is written or `HEAD` moves. The routines committing the index
    /// (`git_commit_create_from_stage`, `git_commit_amend_from_stage`) have written its
    /// tree into the object database by then, and leave it there.
    ///
    /// The options store the callback's trampoline and, as its payload, the callback
    /// itself, which they borrow until they are dropped: it cannot be used, moved or
    /// dropped meanwhile, nor lent to other options, so each libgit2 call reaches it
    /// alone. A panic of its closure stops the commit with `GIT_EUSER` and is kept in the
    /// callback, which the routines committing on `HEAD` ([`create_from_stage`],
    /// [`create_from_tree`], [`amend_from_stage`], [`amend_from_tree`]) resume once
    /// libgit2 returns (and [`resume_panic`](SignatureCallback::resume_panic) resumes for
    /// options lent to C another way).
    ///
    /// ```compile_fail,E0499
    /// use libgit2::libgit2::src::libgit2::filter::StreamError;
    /// use {libgit2::libgit2::src::libgit2::commit::CommitBuilderMut, libgit2::libgit2::src::libgit2::commit::CreateOptions, libgit2::libgit2::src::libgit2::commit::SignatureCallback};
    /// use libgit2::libgit2::src::libgit2::repository::RepositoryRef;
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
    /// Field: `git_commit_create_options.sign`
    /// Field: `git_commit_create_options.payload`
    pub fn set_signer(&mut self, signer: Option<&'o mut SignatureCallback<'_>>) {
        let (callback, payload): (ffi::git_commit_signature_cb, *mut c_void) = match signer {
            Some(signer) => (Some(sign_with_closure), ptr::from_mut(signer).cast()),
            None => (None, ptr::null_mut()),
        };
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let sign_ptr = unsafe { &raw mut (*p).sign };
        // SAFETY: as above.
        let payload_ptr = unsafe { &raw mut (*p).payload };
        // SAFETY: the field is plain data, so the old value needs no drop; the trampoline is sound
        // with libgit2's arguments and the payload written next.
        unsafe { sign_ptr.write(callback) };
        // SAFETY: as above; the payload is the callback the options borrow exclusively for `'o`.
        unsafe { payload_ptr.write(payload) };
    }
}

impl<'o> CreateOptionsMut<'_, 'o> {
    /// Runs `f` with the `git_commit_create_ext_options` libgit2's routines committing on
    /// `HEAD` build from these options (`init_ext_options`): their encoding, extra headers
    /// and signing callback with its payload, and nothing else set. The copy lives for
    /// `f` alone, during which this handle, which borrows the options exclusively, is
    /// frozen: the callback the options borrow exclusively is reached through the copy
    /// only.
    fn with_ext_options<R>(&mut self, f: impl FnOnce(CreateExtOptionsMut<'_, 'o>) -> R) -> R {
        let mut ext = CreateExtOptions::new();
        // Each value copied is one `CreateOptionsMut`'s setters stored and
        // `CreateExtOptionsMut`'s store under the same invariant, for the same `'o`: NULL
        // or a `&'o CStr`; NULL with 0 or a `&'o [Header<'o>]`; `None` with NULL or the
        // trampoline with a `SignatureCallback` borrowed exclusively for `'o`, which
        // nothing but the copy reaches while `f` runs, as this handle stays borrowed.
        let (from, to) = (self.as_mut_ptr(), ext.as_mut().as_mut_ptr());
        // SAFETY: `from` addresses this exclusive handle's live options; the projection forms no
        // reference to them.
        let from_message_encoding = unsafe { &raw const (*from).message_encoding };
        // SAFETY: `to` addresses the live options of `ext`, held above; the projection forms no
        // reference to them.
        let to_message_encoding = unsafe { &raw mut (*to).message_encoding };
        // SAFETY: reads the initialised field through this exclusive handle.
        let value = unsafe { from_message_encoding.read() };
        // SAFETY: writes plain data, valid as read; the old value needs no drop.
        unsafe { to_message_encoding.write(value) };
        // SAFETY: as above.
        let from_extra_headers = unsafe { &raw const (*from).extra_headers };
        // SAFETY: as above.
        let to_extra_headers = unsafe { &raw mut (*to).extra_headers };
        // SAFETY: as above.
        let value = unsafe { from_extra_headers.read() };
        // SAFETY: as above.
        unsafe { to_extra_headers.write(value) };
        // SAFETY: as above.
        let from_extra_headers_len = unsafe { &raw const (*from).extra_headers_len };
        // SAFETY: as above.
        let to_extra_headers_len = unsafe { &raw mut (*to).extra_headers_len };
        // SAFETY: as above.
        let value = unsafe { from_extra_headers_len.read() };
        // SAFETY: as above.
        unsafe { to_extra_headers_len.write(value) };
        // SAFETY: as above.
        let from_sign = unsafe { &raw const (*from).sign };
        // SAFETY: as above.
        let to_sign = unsafe { &raw mut (*to).sign };
        // SAFETY: as above.
        let value = unsafe { from_sign.read() };
        // SAFETY: as above.
        unsafe { to_sign.write(value) };
        // SAFETY: as above.
        let from_payload = unsafe { &raw const (*from).payload };
        // SAFETY: as above.
        let to_payload = unsafe { &raw mut (*to).payload };
        // SAFETY: as above.
        let value = unsafe { from_payload.read() };
        // SAFETY: as above.
        unsafe { to_payload.write(value) };
        f(ext.as_mut())
    }

    /// Resumes a panic the signing callback's closure raised during the libgit2 call
    /// these options were just lent to, if any: what [`amend_from_stage`] and
    /// [`amend_from_tree`] do once libgit2 returns.
    fn resume_signer_panic(&mut self) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let payload_ptr = unsafe { &raw const (*p).payload };
        // SAFETY: a read through the exclusive handle's pointer to live options.
        let payload = unsafe { payload_ptr.read() };
        if payload.is_null() {
            return;
        }
        // SAFETY: a non-NULL payload is the `SignatureCallback` `set_signer` stored, which
        // the options borrow exclusively for `'o`; this handle borrows the options
        // exclusively, and the libgit2 call that used the callback has returned, so
        // nothing else reaches it.
        let signer = unsafe { &mut *payload.cast::<SignatureCallback<'_>>() };
        signer.resume_panic();
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

/// The closure a [`ParentCallback`] calls: the index of the parent asked for, from 0.
type ParentFn<'f> = dyn FnMut(usize) -> Option<Oid> + 'f;

/// A callback giving the parents of a commit being written
/// (`git_commit_parent_callback`): `git_commit_create_from_callback` calls it with each
/// index from 0, and takes each id it returns as the next parent, until it returns
/// `None`.
///
/// Built over a closure with [`new`](Self::new), which returns each parent's id by
/// value: the callback keeps it, and its `extern "C"` trampoline returns a pointer to it,
/// valid until the next call, which libgit2 reads and copies before calling again. A
/// panic in the closure, or an id of another type than the repository's, cannot stop
/// libgit2 cleanly, so the wrapper of a commit routine collects every parent from the
/// closure before libgit2 runs (see the [module docs](self#parent-callbacks)). Through
/// [`call`](Self::call), a panic does not cross the trampoline, which returns NULL, and
/// resumes once `call` is back in Rust.
///
/// ```
/// use libgit2::libgit2::src::libgit2::commit::ParentCallback;
/// use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
///
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// let parents =
///     [Oid::from_string(c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750", OidType::Sha1).unwrap()];
/// let mut next = |i: usize| parents.get(i).map(|id| id.as_ref().to_oid());
/// let mut callback = ParentCallback::new(&mut next);
/// assert_eq!(callback.call(0), Some(parents[0].as_ref()));
/// assert_eq!(callback.call(1), None);
/// ```
///
/// An id it returns lives until it is called again:
///
/// ```compile_fail,E0499
/// use libgit2::libgit2::src::libgit2::commit::ParentCallback;
/// use libgit2::libgit2::src::libgit2::oid::Oid;
///
/// fn twice(callback: &mut ParentCallback<'_>) {
///     let first = callback.call(0);
///     let second = callback.call(1);
///     let _ = (first, second);
/// }
/// ```
///
/// It borrows its closure exclusively and stays on its thread: libgit2 calls it
/// synchronously, and the closure need not be `Send`.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::commit::ParentCallback;
/// use libgit2::libgit2::src::libgit2::oid::Oid;
///
/// fn send<T: Send>(_: T) {}
/// let mut f = |_: usize| None::<Oid>;
/// send(ParentCallback::new(&mut f));
/// ```
///
/// Wraps: `git_commit_parent_callback`
/// Handwritten
pub struct ParentCallback<'f> {
    f: &'f mut ParentFn<'f>,
    /// The id last handed out, which libgit2 copies before it calls again.
    last: Oid,
    /// A panic the closure raised inside the trampoline, resumed once it returns.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> ParentCallback<'f> {
    /// The callback that calls `f` with each parent's index, until it returns `None`.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(usize) -> Option<Oid> + 'f,
    {
        Self {
            f,
            last: Oid::zeroed(),
            panic: None,
        }
    }

    /// Calls the callback for the parent at `idx`, through the trampoline libgit2 calls,
    /// and returns what libgit2 would read from it: the parent's id, kept by the callback
    /// until its next call, or `None` at the end of the list. A panic of the closure
    /// resumes here.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: the trampoline is never NULL"
    )]
    pub fn call(&mut self, idx: usize) -> Option<OidRef<'_>> {
        let id = self.with_payload(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            // SAFETY: `payload` is this callback, which nothing else uses during the
            // call: the trampoline's contract.
            unsafe { callback(idx, payload) }
        });
        if let Some(panic) = self.panic.take() {
            resume_unwind(panic);
        }
        // SAFETY: the trampoline returns NULL or a pointer to `self.last`, live and
        // unwritten while the result borrows `self`.
        unsafe { OidRef::from_ptr(id.cast_mut()) }
    }

    /// Runs `walk` with a C callback and its payload, as the wrapper of the routine that
    /// calls the callback (`git_commit_create_from_callback`) hands them to it, for a
    /// repository whose ids are of type `kind`.
    ///
    /// The closure is asked for every parent first, from index 0 until it returns `None`;
    /// a panic propagates from here, before `walk` runs. An id of another type than
    /// `kind` ends the walk there too, with libgit2's wording for the mismatch
    /// (`GIT_ERROR_INVALID`), set as the thread's error. Otherwise `walk` gets a
    /// trampoline over the collected ids, which hands out each in turn, then NULL, and
    /// can neither fail nor panic, so libgit2 writes the commit with exactly those
    /// parents, or fails on its own.
    pub(crate) fn walk_with<R>(
        &mut self,
        kind: OidType,
        walk: impl FnOnce(ffi::git_commit_parent_callback, *mut c_void) -> R,
    ) -> Result<R, RepositoryError> {
        let mut parents = Vec::new();
        while let Some(id) = (self.f)(parents.len()) {
            if u32::from(id.as_ref().raw_kind()) != kind.raw() {
                set(
                    ErrorClass::Invalid,
                    c"specified object ID type does not match repository object ID type",
                );
                return Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()));
            }
            parents.push(id);
        }
        let mut collected = |idx: usize| parents.get(idx).copied();
        Ok(ParentCallback::new(&mut collected).with_payload(walk))
    }

    /// Runs `walk` with the trampoline and this callback as its payload.
    fn with_payload<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_commit_parent_callback, *mut c_void) -> R,
    ) -> R {
        walk(Some(parent_with), ptr::from_mut(self).cast())
    }
}

impl fmt::Debug for ParentCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParentCallback")
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_commit_parent_callback` over a [`ParentCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`ParentCallback`] that nothing else uses during the
/// call. The pointer returned is NULL or addresses the callback's last id, valid until
/// the callback is next called, moved or dropped.
unsafe extern "C" fn parent_with(idx: usize, payload: *mut c_void) -> *const ffi::git_oid {
    // SAFETY: per this function's contract, `payload` is a live callback borrowed for
    // the call alone.
    let callback = unsafe { &mut *payload.cast::<ParentCallback<'_>>() };
    if callback.panic.is_some() {
        return ptr::null();
    }
    let id = match catch_unwind(AssertUnwindSafe(|| (callback.f)(idx))) {
        Ok(None) => None,
        Ok(Some(id)) => {
            callback.last = id;
            Some(&callback.last)
        }
        Err(panic) => {
            callback.panic = Some(panic);
            None
        }
    };
    id.map_or(ptr::null(), |id| id.as_ref().as_ptr())
}

/// Writes a commit into `repo`'s object database and returns its id, as
/// [`create`](crate::libgit2::src::libgit2::commit::create) does, but with the tree and the
/// parents given by id, and the parents asked from `parents` in order, until it gives
/// `None` (see the [module docs](self#parent-callbacks)).
///
/// Unless strict object creation is turned off, libgit2 checks that the tree and every
/// parent exist in the object database with their types, failing with libgit2's error
/// otherwise (`GIT_ERROR_INVALID`, "object to set as tree does not exist", say). With an
/// `update_ref`, the reference is moved as `create` moves it, an existing one only from
/// the first parent.
///
/// Wraps: `git_commit_create_from_callback`
///
/// # Errors
///
/// Refuses, before libgit2 runs and with `GIT_ERROR_INVALID`, "specified object ID type
/// does not match repository object ID type", a tree id or a parent id whose type is not
/// `repo`'s, which libgit2 would format out of bounds (see the
/// [`object` module](crate::libgit2::src::libgit2::object#looking-objects-up)); and a
/// signature `create` refuses. A panic of the callback's closure propagates before
/// libgit2 runs.
#[expect(
    clippy::too_many_arguments,
    reason = "the C routine's parameters, in its order"
)]
pub fn create_from_callback(
    repo: RepositoryRef<'_>,
    update_ref: Option<&CStr>,
    author: SignatureRef<'_>,
    committer: SignatureRef<'_>,
    message_encoding: Option<&CStr>,
    message: &CStr,
    tree: OidRef<'_>,
    parents: &mut ParentCallback<'_>,
) -> Result<Oid, RepositoryError> {
    id_fits(repo, tree)?;
    writable(author)?;
    writable(committer)?;
    let mut id = Oid::zeroed();
    let out = id.as_mut().as_mut_ptr();
    let update_ref = update_ref.map_or(ptr::null(), CStr::as_ptr);
    let encoding = message_encoding.map_or(ptr::null(), CStr::as_ptr);
    let rc = parents.walk_with(repo.oid_type(), |callback, payload| {
        // SAFETY: `out` addresses `id`, written on success; `repo` is live on this thread,
        // written through as `RepositoryRef` allows; the signatures are live and writable
        // and `tree` an id of the repository's type (checked above); the strings are
        // NUL-terminated or NULL; the callback and payload are the walk's, handing out ids
        // of that type, each live until the next call, which libgit2 copies first.
        unsafe {
            ffi::git_commit_create_from_callback(
                out,
                repo.as_ptr().cast_mut(),
                update_ref,
                author.as_ptr(),
                committer.as_ptr(),
                encoding,
                message.as_ptr(),
                tree.as_ptr(),
                callback,
                payload,
            )
        }
    })?;
    if rc < 0 {
        Err(RepositoryError::from_libgit2(rc))
    } else {
        Ok(id)
    }
}

/// [`create_from_callback`], with the parents given by id in order, as a slice.
///
/// Wraps: `git_commit_create_from_ids`
///
/// # Errors
///
/// Refuses what [`create_from_callback`] refuses, a parent id of another type than
/// `repo`'s included, and fails as it does.
#[expect(
    clippy::too_many_arguments,
    reason = "the C routine's parameters, in its order"
)]
pub fn create_from_ids(
    repo: RepositoryRef<'_>,
    update_ref: Option<&CStr>,
    author: SignatureRef<'_>,
    committer: SignatureRef<'_>,
    message_encoding: Option<&CStr>,
    message: &CStr,
    tree: OidRef<'_>,
    parents: &[OidRef<'_>],
) -> Result<Oid, RepositoryError> {
    id_fits(repo, tree)?;
    for parent in parents {
        id_fits(repo, *parent)?;
    }
    writable(author)?;
    writable(committer)?;
    let mut id = Oid::zeroed();
    let out = id.as_mut().as_mut_ptr();
    let mut ptrs: Vec<*const ffi::git_oid> = parents.iter().map(OidRef::as_ptr).collect();
    let update_ref = update_ref.map_or(ptr::null(), CStr::as_ptr);
    let encoding = message_encoding.map_or(ptr::null(), CStr::as_ptr);
    // SAFETY: as in `create_from_callback`; `ptrs` addresses `ptrs.len()` live ids of the
    // repository's type (checked above), only read despite the `git_oid **` (an empty
    // one is dangling and never read).
    let rc = unsafe {
        ffi::git_commit_create_from_ids(
            out,
            repo.as_ptr().cast_mut(),
            update_ref,
            author.as_ptr(),
            committer.as_ptr(),
            encoding,
            message.as_ptr(),
            tree.as_ptr(),
            ptrs.len(),
            ptrs.as_mut_ptr(),
        )
    };
    if rc < 0 {
        Err(RepositoryError::from_libgit2(rc))
    } else {
        Ok(id)
    }
}

#[cfg(test)]
mod unit_tests {
    use core::ffi::{CStr, c_char, c_int, c_void};
    use core::mem::size_of;
    use std::process::Command;
    use std::sync::atomic::Ordering::SeqCst;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    use ffibox::CDrop;

    use super::*;
    use crate::libgit2::src::libgit2::object::{Object, ObjectType};
    use crate::libgit2::src::libgit2::oid::{Oid, OidType};
    use crate::libgit2::src::libgit2::repository::{Repository, RepositoryBox};
    use crate::libgit2::src::libgit2::signature::{SignatureView, Time};
    use crate::libgit2::src::util::alloc::{Alloc, Allocator};
    use crate::libgit2::src::util::errors::{ErrorClass, ErrorCode, runtime_pinned};

    /// The fixture repositories: `testrepo.git` (a branch at `a65fedf`), `mergedrepo`
    /// (merging `e280915` into `3a34580`) and `empty_bare.git` (an unborn `HEAD`).
    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/testrepo.git\0"
    );
    const MERGEDREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/mergedrepo/.gitted\0"
    );
    const EMPTY: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/empty_bare.git\0"
    );

    /// Opens the repository at the NUL-terminated `path`, read-only use only.
    fn open(path: &str) -> *mut ffi::git_repository {
        assert!(runtime_pinned());
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` receives the repository; `path` is NUL-terminated; no ceiling
        // directories.
        let rc = unsafe {
            ffi::git_repository_open_ext(
                &raw mut repo,
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
        let commit_id = unsafe { ffi::git_commit_id(commit.as_ptr()) };
        // SAFETY: as above.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), commit_id) };
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
    /// An allocation length [`Refusing`] refuses besides; 0 for none.
    static REFUSED_LEN: AtomicUsize = AtomicUsize::new(0);
    /// How many fresh blocks of `REFUSED_LEN` bytes [`Refusing`] grants before it
    /// refuses them.
    static REFUSED_LEN_GRANTS: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// libc's allocator, refusing everything while `REFUSING` is set, and fresh blocks of
    /// `REFUSED_LEN` bytes once `REFUSED_LEN_GRANTS` of them are granted.
    struct Refusing;

    // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
    unsafe impl Alloc for Refusing {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            let granted = || {
                REFUSED_LEN_GRANTS
                    .try_update(SeqCst, SeqCst, |n| n.checked_sub(1))
                    .is_ok()
            };
            if REFUSING.load(SeqCst) || (REFUSED_LEN.load(SeqCst) == len && !granted()) {
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
                ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
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
        for kind in [u8::try_from(OidType::Sha256.raw()).unwrap(), 0, 9, u8::MAX] {
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
    /// each reference handed over once (`LSan`: a leak; `ASan`: a double release); the
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
        let encoding = CString::new("ISO-8859-1").unwrap();
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
        let r = unsafe { CreateExtOptionsRef::from_ptr(&raw mut c) }.unwrap();
        assert!(r.extra_headers().is_empty());
        c.extra_headers = c_headers.as_ptr();
        c.extra_headers_len = 1;
        // SAFETY: `c` now addresses one live header of `'static` strings, and nothing
        // else reaches `c` while the handle lives.
        let mut m = unsafe { CreateExtOptionsMut::<'_, 'static>::from_ptr(&raw mut c) }.unwrap();
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
        let add = unsafe { array.add(1) };
        // SAFETY: as above.
        let second = unsafe { add.read() };
        // SAFETY: the second header stores a `&'static CStr`'s pointer.
        assert_eq!(unsafe { CStr::from_ptr(second.field) }, c"line_two");
    }

    /// The setters replace one string each, through a handle and a reborrow; a copy is
    /// independent of the header it came from.
    #[test]
    fn header_setters_reborrows_and_copies() {
        let owned_field = CString::new("owned").unwrap();
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
        let field = CString::new("line_one").unwrap();
        let value = CString::new("x").unwrap();
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
            CommitArrayDispose.c_dispose(NonNull::from(&mut broken).cast::<CommitArray<'_>>());
        };
        assert_eq!((broken.commits, broken.count), (ptr::null(), 3));
        // SAFETY: `broken` is live for the handle's use; nothing else reaches it.
        let mut m = unsafe { CommitArrayMut::from_ptr(&raw mut broken) }.unwrap();
        assert_eq!(m.as_ref().len(), 0);
        assert!(m.as_ref().get(0).is_none());
        assert_eq!(m.as_ref().iter().count(), 0);
        assert_eq!(format!("{m:?}"), "[]");
        m.clear();
        assert_eq!((broken.commits, broken.count), (ptr::null(), 0));
        broken.count = 2;
        // SAFETY: as above.
        let mut m = unsafe { CommitArrayMut::from_ptr(&raw mut broken) }.unwrap();
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
    /// and released after its repositories are freed (`LSan` flags a leak, `ASan` a double
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
        let rc = unsafe { ffi::git_repository_commit_parents(&raw mut raw, repo) };
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
        let rc = unsafe { ffi::git_repository_commit_parents(&raw mut raw, repo) };
        assert_eq!(rc, 0);
        // SAFETY: as above.
        let adopted: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(raw) };
        let mut back = adopted.into_inner().into_raw();
        assert_eq!(back.count, 2);
        // SAFETY: `back` owns its commits and buffer again, released once by C.
        unsafe { ffi::git_commitarray_dispose(&raw mut back) };
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
    /// written commit (`ASan` flags any access through a wrong pointer).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn commit_builder_lent_to_the_signing_callback() {
        assert!(runtime_pinned());
        let dir = std::env::temp_dir().join(format!(
            "crustify-commitbuilder-unit-{}",
            std::process::id()
        ));
        drop(std::fs::remove_dir_all(&dir));
        let path = CString::new(dir.to_str().unwrap()).unwrap();
        let mut repo = ptr::null_mut();
        // SAFETY: a writable slot and a NUL-terminated path; bare.
        let rc = unsafe { ffi::git_repository_init(&raw mut repo, path.as_ptr(), 1) };
        assert_eq!(rc, 0);

        // An empty tree, an identity and the signing options.
        let mut builder = ptr::null_mut();
        let mut tree_id = ffi::git_oid::default();
        let mut tree = ptr::null_mut();
        let mut who = ptr::null_mut();
        // SAFETY: writable slots, the live repository and NUL-terminated strings; each
        // object is freed once below.
        unsafe {
            assert_eq!(
                ffi::git_treebuilder_new(&raw mut builder, repo, ptr::null()),
                0
            );
        };
        // SAFETY: as above.
        unsafe { assert_eq!(ffi::git_treebuilder_write(&raw mut tree_id, builder), 0) };
        // SAFETY: as above.
        unsafe { ffi::git_treebuilder_free(builder) };
        // SAFETY: as above.
        unsafe {
            assert_eq!(
                ffi::git_tree_lookup(&raw mut tree, repo, &raw const tree_id),
                0
            );
        };
        // SAFETY: as above.
        unsafe {
            assert_eq!(
                ffi::git_signature_new(
                    &raw mut who,
                    c"A U Thor".as_ptr(),
                    c"a@example.com".as_ptr(),
                    1_700_000_000,
                    0
                ),
                0
            );
        };
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
                &raw mut id,
                repo,
                who,
                who,
                c"signed commit\n".as_ptr(),
                tree,
                0,
                ptr::null(),
                &raw const opts,
            )
        };
        assert_eq!(rc, 0);
        assert_ne!(state.builder, 0);
        assert_eq!(state.add_header, Some(0));
        assert!(state.content.starts_with("tree "));
        assert!(state.content.ends_with("\nsigned commit\n"));

        let mut commit = ptr::null_mut();
        let mut field = ffi::git_buf::default();
        {
            // SAFETY: a writable slot, the live repository and the written id; then the live
            // commit, a zeroed buffer and a NUL-terminated name; all released once.
            unsafe {
                assert_eq!(
                    ffi::git_commit_lookup(&raw mut commit, repo, &raw const id),
                    0
                );
            };
            // SAFETY: as above.
            unsafe {
                assert_eq!(
                    ffi::git_commit_header_field(&raw mut field, commit, c"gpgsig".as_ptr()),
                    0
                );
            };
            // SAFETY: as above.
            let value = unsafe { CStr::from_ptr(field.ptr).to_str().unwrap().to_owned() };
            assert_eq!(
                value,
                "-----BEGIN SIGNATURE-----\nsigned\n-----END SIGNATURE-----"
            );
            // SAFETY: as above.
            unsafe { ffi::git_buf_dispose(&raw mut field) };
            // SAFETY: as above.
            unsafe { ffi::git_commit_free(commit) };
            // SAFETY: as above.
            unsafe { ffi::git_tree_free(tree) };
            // SAFETY: as above.
            unsafe { ffi::git_signature_free(who) };
            // SAFETY: as above.
            unsafe { ffi::git_repository_free(repo) };
        }
        drop(std::fs::remove_dir_all(&dir));
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
            drop(std::fs::remove_dir_all(&dir));
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let (mut repo, mut builder, mut tree, mut who) = (
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
            let mut tree_id = ffi::git_oid::default();
            {
                // SAFETY: writable slots, NUL-terminated strings, and the live repository;
                // the builder is freed once, the rest when the writer drops.
                unsafe { assert_eq!(ffi::git_repository_init(&raw mut repo, path.as_ptr(), 1), 0) };
                // SAFETY: as above.
                unsafe {
                    assert_eq!(
                        ffi::git_treebuilder_new(&raw mut builder, repo, ptr::null()),
                        0
                    );
                };
                // SAFETY: as above.
                unsafe { assert_eq!(ffi::git_treebuilder_write(&raw mut tree_id, builder), 0) };
                // SAFETY: as above.
                unsafe { ffi::git_treebuilder_free(builder) };
                // SAFETY: as above.
                unsafe {
                    assert_eq!(
                        ffi::git_tree_lookup(&raw mut tree, repo, &raw const tree_id),
                        0
                    );
                };
                // SAFETY: as above.
                let rc = unsafe {
                    ffi::git_signature_new(
                        &raw mut who,
                        c"A U Thor".as_ptr(),
                        c"a@example.com".as_ptr(),
                        1_700_000_000,
                        0,
                    )
                };
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
                    &raw mut id,
                    self.repo,
                    self.who,
                    self.who,
                    message.as_ptr(),
                    self.tree,
                    0,
                    ptr::null(),
                    &raw const opts,
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
            {
                // SAFETY: writable slots and the live repository and id; the object's data
                // is `size` bytes, copied before the object and the odb are freed once.
                unsafe { assert_eq!(ffi::git_repository_odb(&raw mut odb, self.repo), 0) };
                // SAFETY: as above.
                unsafe { assert_eq!(ffi::git_odb_read(&raw mut obj, odb, id), 0) };
                // SAFETY: as above.
                let odb_object_data = unsafe { ffi::git_odb_object_data(obj) };
                // SAFETY: as above.
                let odb_object_size = unsafe { ffi::git_odb_object_size(obj) };
                // SAFETY: as above.
                let data = unsafe {
                    core::slice::from_raw_parts(odb_object_data.cast::<u8>(), odb_object_size)
                        .to_vec()
                };
                // SAFETY: as above.
                unsafe { ffi::git_odb_object_free(obj) };
                // SAFETY: as above.
                unsafe { ffi::git_odb_free(odb) };
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
            unsafe { ffi::git_signature_free(self.who) };
            // SAFETY: as above.
            unsafe { ffi::git_tree_free(self.tree) };
            // SAFETY: as above.
            unsafe { ffi::git_repository_free(self.repo) };
            drop(std::fs::remove_dir_all(&self.dir));
        }
    }

    const SIGNATURE: &CStr = c"-----BEGIN SIGNATURE-----\nsigned\n-----END SIGNATURE-----";

    /// The closure is given the builder, the repository and the contents to sign, and
    /// adds the signature (then a second header) through the builder; reading the
    /// contents after the first header was added is sound, since they are a copy (`ASan`
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
            assert!(calls != 1, "signing failed");
            Ok(())
        };
        let mut callback = SignatureCallback::new(&mut boom);
        let mut returned = None;
        let caught = catch_unwind(AssertUnwindSafe(|| {
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
        use std::fmt::Write as _;
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
            let hex = nested.id[..20].iter().fold(String::new(), |mut hex, b| {
                let _ = write!(hex, "{b:02x}");
                hex
            });
            let value = CString::new(hex).unwrap();
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
        let caught = catch_unwind(AssertUnwindSafe(|| {
            callback.sign_with(|sign, payload| {
                let sign = sign.unwrap();
                {
                    let user = ErrorCode::User.raw();
                    // SAFETY: the payload is the callback; each pointer is NULL or live
                    // storage the closure never reads, and `content` is NUL-terminated.
                    unsafe { assert_eq!(sign(ptr::null_mut(), repo, content, payload), user) };
                    // SAFETY: as above.
                    unsafe { assert_eq!(sign(builder, ptr::null_mut(), content, payload), user) };
                    // SAFETY: as above.
                    unsafe { assert_eq!(sign(builder, repo, ptr::null(), payload), user) };
                    // SAFETY: as above.
                    unsafe { assert_eq!(sign(builder, repo, content, payload), user) };
                    // SAFETY: as above.
                    unsafe { assert_eq!(sign(builder, repo, content, payload), user) };
                }
            });
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
            assert!(calls != 2, "second commit");
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
        let caught = catch_unwind(AssertUnwindSafe(|| signer.resume_panic()));
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
                _bitfield_1: ffi::__BindgenBitfieldUnit::new([0xFE]),
                ..Default::default()
            },
            PhantomData,
        );
        assert!(!options.as_ref().allow_empty_commit());
        options.as_mut().set_allow_empty_commit(true);
        let raw = create_raw(&options);
        assert_eq!((raw._bitfield_1.get(0, 8), raw.version), (0xFF, 7));
        assert_eq!(raw.allow_empty_commit(), 1);
        assert!(options.as_ref().allow_empty_commit());
        options.as_mut().set_allow_empty_commit(false);
        let raw = create_raw(&options);
        assert_eq!(
            (raw._bitfield_1.get(0, 8), raw.allow_empty_commit()),
            (0xFE, 0)
        );
        assert!(!options.as_ref().allow_empty_commit());
    }

    /// Every setter stores what its getter reads back, the signatures', strings' and
    /// run's own pointers included, and clearing stores C's NULLs.
    #[test]
    fn create_options_setters_round_trip() {
        let encoding = CString::new("ISO-8859-1").unwrap();
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
                &raw mut id,
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
                &raw mut amended,
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
                &raw mut amended,
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
                &raw mut amended,
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

    /// `testrepo.git`'s merge `be3563a` of `9fd738e` and `c47800c`.
    const MERGE: &CStr = c"be3563ae3f795b2b4353bcce3a527ad0a4f7f644";
    const MERGE_PARENTS: [&str; 2] = [
        "9fd738e8f7967c078dceed8190330fc8648ee56a",
        "c47800c7266a2be04c571c04d5a6614691ea99bd",
    ];

    fn oid_hex(id: OidRef<'_>) -> String {
        id.to_hex_string().unwrap()
    }

    /// The getters over two commits whose contents `git cat-file` shows: `HEAD`
    /// (`a65fedf`, one parent, a message of one newline) and a merge.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_read_what_the_commit_records() {
        let repo = open_testrepo();
        let head = Commit::lookup(repo.as_ref(), sha1(HEAD).as_ref()).unwrap();
        let h = head.as_ref();
        assert_eq!(oid_hex(h.id()), HEAD.to_str().unwrap());
        assert_eq!(oid_hex(h.tree_id()), TREE.to_str().unwrap());
        assert_eq!(h.parent_count(), 1);
        assert_eq!(oid_hex(h.parent_id(0).unwrap()), PARENT.to_str().unwrap());
        assert!(h.parent_id(1).is_none() && h.parent_id(u32::MAX).is_none());
        assert_eq!((h.message_raw(), h.message()), (c"\n", c""));
        assert_eq!(h.summary(), Ok(c""));
        assert_eq!(h.message_encoding(), None);
        assert_eq!((h.time(), h.time_offset()), (1_312_943_626, -420));
        let header = h.raw_header().to_str().unwrap();
        assert!(header.starts_with("tree 944c0f6e4dfa41595e6eb3ceecdb14f50fe18162\nparent "));
        assert!(header.ends_with("1312943626 -0700\n"));
        assert_eq!(h.owner().path(), repo.as_ref().path());

        let merge = Commit::lookup(repo.as_ref(), sha1(MERGE).as_ref()).unwrap();
        let m = merge.as_ref();
        assert_eq!(m.parent_count(), 2);
        let parents: Vec<_> = (0..3).map(|n| m.parent_id(n).map(oid_hex)).collect();
        assert_eq!(
            parents,
            [
                Some(MERGE_PARENTS[0].into()),
                Some(MERGE_PARENTS[1].into()),
                None
            ]
        );
        assert_eq!(m.message(), c"Merge branch 'br2'\n");
        assert_eq!(m.message(), m.message_raw());
        assert_eq!(m.summary(), Ok(c"Merge branch 'br2'"));
        let parent = m.header_field(c"parent").unwrap();
        assert_eq!(parent.as_ref().as_bytes(), MERGE_PARENTS[0].as_bytes());
        let tree = m.header_field(c"tree").unwrap();
        assert_eq!(tree.as_ref().as_bytes(), oid_hex(m.tree_id()).as_bytes());
        let missing = m.header_field(c"encoding").unwrap_err();
        assert_eq!(missing.code(), Some(ErrorCode::NotFound));
        assert_eq!(missing.message(), c"no such field 'encoding'");
    }

    /// libgit2 matches a header field against the rest of the header, across line
    /// ends, and on a match past the line copies `eol - buf`, negative, as a length:
    /// for these first two fields, `UBSan`'s pointer overflow in `git_str_put` and
    /// `ASan`'s allocation of `0xffffffffffffffd0` bytes. The wrapper refuses every field
    /// holding a newline as one it does not find, which is what libgit2 reports for
    /// those of them it reads safely.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn header_fields_holding_a_newline_are_refused_before_c() {
        let repo = open_testrepo();
        let head = Commit::lookup(repo.as_ref(), sha1(HEAD).as_ref()).unwrap();
        let fields: [&CStr; 5] = [
            c"tree 944c0f6e4dfa41595e6eb3ceecdb14f50fe18162\nparent",
            c"tree 944c0f6e4dfa41595e6eb3ceecdb14f50fe18162\nparent be3563ae3f795b2b4353bcce3a527ad0a4f7f644\nauthor",
            c"\n",
            c"parent\n",
            c"zz\nzz",
        ];
        for field in fields {
            let e = head.as_ref().header_field(field).unwrap_err();
            assert_eq!(e.raw_code(), ErrorCode::NotFound.raw(), "{field:?}");
            assert_eq!(e.class(), Some(ErrorClass::Object));
            let expected = format!("no such field '{}'", field.to_str().unwrap());
            assert_eq!(e.message().to_str().unwrap(), expected);
            assert_eq!(last().message(), e.message());
        }
        // The same commit's header still reads, and a field without a newline is found.
        let parent = head.as_ref().header_field(c"parent").unwrap();
        assert_eq!(parent.as_ref().as_bytes(), PARENT.to_bytes());
    }

    /// The summary is computed once, into the commit, and every handle of the commit
    /// reads that one string, which lives as long as the commit does.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn summary_is_computed_once_into_the_shared_commit() {
        let repo = open_testrepo();
        let merge = Commit::lookup(repo.as_ref(), sha1(MERGE).as_ref()).unwrap();
        let copy = merge.as_ref().to_commit();
        let first = merge.as_ref().summary().unwrap().as_ptr();
        let second = copy.as_ref().summary().unwrap();
        assert!(ptr::eq(first, second.as_ptr()));
        drop(merge);
        assert_eq!(copy.as_ref().summary(), Ok(c"Merge branch 'br2'"));
    }

    /// Out of memory: a summary whose string and empty fallback cannot be allocated
    /// fails with libgit2's error, and is computed on a later call; a header field
    /// fails, leaving the commit readable.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn summaries_and_header_fields_out_of_memory() {
        isolated(
            concat!(
                module_path!(),
                "::summaries_and_header_fields_out_of_memory"
            ),
            || {
                install_refusing();
                let repo = open_testrepo();
                let merge = Commit::lookup(repo.as_ref(), sha1(MERGE).as_ref()).unwrap();
                REFUSING.store(true, SeqCst);
                let summary = merge.as_ref().summary();
                let field = merge.as_ref().header_field(c"author");
                REFUSING.store(false, SeqCst);
                assert_eq!(summary.unwrap_err().class(), Some(ErrorClass::NoMemory));
                assert_eq!(field.unwrap_err().class(), Some(ErrorClass::NoMemory));
                assert_eq!(merge.as_ref().summary(), Ok(c"Merge branch 'br2'"));
                assert!(merge.as_ref().header_field(c"author").is_ok());
            },
        );
    }
    // --- Reading and creating commits ------------------------------------------------

    use crate::libgit2::src::libgit2::mailmap::Mailmap;
    use crate::libgit2::src::libgit2::tree::Tree;

    /// The empty tree's id, which libgit2 knows without storing it.
    const EMPTY_TREE: &CStr = c"4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    /// A fresh bare repository, through the safe wrappers, removed when dropped.
    struct Scratch {
        dir: std::path::PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl Scratch {
        fn new(tag: &str) -> Self {
            assert!(runtime_pinned());
            let dir = std::env::temp_dir().join(format!(
                "crustify-commit-create-{}-{tag}",
                std::process::id()
            ));
            drop(std::fs::remove_dir_all(&dir));
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let repo = Repository::init(&path, true).unwrap();
            Self {
                dir,
                repo: Some(repo),
            }
        }

        fn repo(&self) -> RepositoryRef<'_> {
            self.repo.as_ref().unwrap().as_ref()
        }

        fn empty_tree(&self) -> TreeArc<'_> {
            Tree::lookup(self.repo(), sha1(EMPTY_TREE).as_ref()).unwrap()
        }

        /// The loose objects written so far.
        fn objects(&self) -> usize {
            std::fs::read_dir(self.dir.join("objects"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.file_name().len() == 2)
                .map(|e| std::fs::read_dir(e.path()).unwrap().count())
                .sum()
        }

        /// The raw contents of the object `id`.
        fn raw(&self, id: &Oid) -> Vec<u8> {
            let (mut odb, mut obj) = (ptr::null_mut(), ptr::null_mut());
            {
                // SAFETY: writable slots, the live repository and id; the object's data is
                // `size` bytes, copied before the object and the odb are freed once.
                unsafe {
                    assert_eq!(
                        ffi::git_repository_odb(&raw mut odb, self.repo().as_ptr().cast_mut()),
                        0
                    );
                };
                // SAFETY: as above.
                unsafe {
                    assert_eq!(
                        ffi::git_odb_read(&raw mut obj, odb, id.as_ref().as_ptr()),
                        0
                    );
                };
                // SAFETY: as above.
                let odb_object_data = unsafe { ffi::git_odb_object_data(obj) };
                // SAFETY: as above.
                let odb_object_size = unsafe { ffi::git_odb_object_size(obj) };
                // SAFETY: as above.
                let data = unsafe {
                    core::slice::from_raw_parts(odb_object_data.cast::<u8>(), odb_object_size)
                        .to_vec()
                };
                // SAFETY: as above.
                unsafe { ffi::git_odb_object_free(obj) };
                // SAFETY: as above.
                unsafe { ffi::git_odb_free(odb) };
                data
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            drop(self.repo.take());
            drop(std::fs::remove_dir_all(&self.dir));
        }
    }

    /// A signature over static strings, at a fixed time in the zone `offset` minutes
    /// east of UTC.
    fn who(offset: i32) -> SignatureView<'static> {
        SignatureView::new(
            c"A U Thor",
            c"a@example.com",
            Time::new(1_700_000_000, offset),
        )
    }

    /// The code, class and message of a refusal.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the helper consumes what it reports, as its callers hand it over"
    )]
    fn refusal(e: RepositoryError) -> (c_int, Option<ErrorClass>, String) {
        (
            e.raw_code(),
            e.class(),
            e.message().to_string_lossy().into_owned(),
        )
    }

    /// A signature missing its name or email (as libgit2's parser leaves one when it
    /// runs out of memory), or whose offset libgit2 cannot negate, is refused before
    /// it reaches a writer; every other one is writable. The mailmap refuses the first
    /// kind with a release build's assertion error.
    #[test]
    #[cfg_attr(miri, ignore = "sets libgit2's error")]
    fn signatures_libgit2_cannot_write_are_refused() {
        assert!(runtime_pinned());
        for offset in [i32::MIN + 1, -1, 0, 1, i32::MAX] {
            let sig = who(offset);
            assert!(writable(sig.as_ref()).is_ok(), "{offset}");
            assert!(resolvable(sig.as_ref()).is_ok());
        }
        let e = writable(who(i32::MIN).as_ref()).unwrap_err();
        assert_eq!(
            refusal(e),
            (
                -1,
                Some(ErrorClass::Invalid),
                "failed to create commit: signature timezone offset is out of range".into()
            )
        );
        let (name, email) = (c"N".as_ptr().cast_mut(), c"e@x".as_ptr().cast_mut());
        for (n, e, missing) in [
            (ptr::null_mut(), email, "name"),
            (name, ptr::null_mut(), "email"),
            (ptr::null_mut(), ptr::null_mut(), "name"),
        ] {
            let mut raw = ffi::git_signature {
                name: n,
                email: e,
                when: Time::new(0, 0).into(),
            };
            // SAFETY: a live signature on the stack, whose strings are NULL or static,
            // read only while it lives.
            let sig = unsafe { SignatureRef::from_ptr(&raw mut raw) }.unwrap();
            let err = writable(sig).unwrap_err();
            assert_eq!(
                refusal(err),
                (
                    -1,
                    Some(ErrorClass::Invalid),
                    "failed to create commit: signature is missing its name or email".into()
                )
            );
            let err = resolvable(sig).unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Internal));
            assert_eq!(
                err.message().to_str().unwrap(),
                format!("unrecoverable internal error: '{missing}'")
            );
        }
    }

    /// A tree or a parent of another repository is refused by every creation wrapper
    /// before libgit2 runs: nothing is written.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn trees_and_parents_of_other_repositories_are_refused_before_c() {
        let (a, b) = (Scratch::new("own-a"), Scratch::new("own-b"));
        let me = who(0);
        let me = me.as_ref();
        let (tree_a, tree_b) = (a.empty_tree(), b.empty_tree());
        let id = create(a.repo(), None, me, me, None, c"a\n", tree_a.as_ref(), &[]).unwrap();
        let root_a = Commit::lookup(a.repo(), id.as_ref()).unwrap();
        let id = create(b.repo(), None, me, me, None, c"b\n", tree_b.as_ref(), &[]).unwrap();
        let root_b = Commit::lookup(b.repo(), id.as_ref()).unwrap();
        let before = a.objects();

        let tree = (
            -1,
            Some(ErrorClass::Invalid),
            "invalid argument: 'git_tree_owner(tree) == repo'".to_owned(),
        );
        let parent = (
            -1,
            Some(ErrorClass::Invalid),
            "failed to create commit: a parent commit is from another repository".to_owned(),
        );
        let (ra, ta, tb) = (a.repo(), tree_a.as_ref(), tree_b.as_ref());
        let (pa, pb) = (root_a.as_ref(), root_b.as_ref());
        for (t, parents, expected) in [(tb, [pa], &tree), (ta, [pb], &parent)] {
            let e = create(ra, None, me, me, None, c"x\n", t, &parents).unwrap_err();
            assert_eq!(&refusal(e), expected);
            let e = create_from_array(ra, None, me, me, None, c"x\n", t, parents).unwrap_err();
            assert_eq!(&refusal(e), expected);
            let e = create_buffer(ra, me, me, None, c"x\n", t, &parents).unwrap_err();
            assert_eq!(&refusal(e), expected);
            let mut options = CreateExtOptions::new();
            let e =
                create_ext(ra, me, me, c"x\n", t, &parents, Some(options.as_mut())).unwrap_err();
            assert_eq!(&refusal(e), expected);
        }
        // A foreign parent after an own one is refused too: libgit2 would drop it.
        let e = create(ra, None, me, me, None, c"x\n", ta, &[pa, pb]).unwrap_err();
        assert_eq!(refusal(e), parent);
        let e = pa
            .amend(None, None, None, None, None, Some(tb))
            .unwrap_err();
        assert_eq!(refusal(e), tree);
        let min = who(i32::MIN);
        let e = pa
            .amend(None, Some(min.as_ref()), None, None, None, None)
            .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        let e = create(ra, None, me, min.as_ref(), None, c"x\n", ta, &[]).unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        assert_eq!(a.objects(), before);
    }

    /// The creation wrappers write the commit `create_buffer` formats, whatever the
    /// number of parents, and the array form writes the same commit as the slice form.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "the closure lets each handle reborrow for its own lifetime, which the method path cannot"
    )]
    fn creation_forms_write_the_formatted_commit() {
        let s = Scratch::new("forms");
        let (repo, tree) = (s.repo(), s.empty_tree());
        let (me, them) = (who(90), who(-150));
        let (me, them) = (me.as_ref(), them.as_ref());
        let mut commits = Vec::new();
        for i in 0..3 {
            let message = CString::new(format!("root {i}\n")).unwrap();
            let id = create(repo, None, me, them, None, &message, tree.as_ref(), &[]).unwrap();
            commits.push(Commit::lookup(repo, id.as_ref()).unwrap());
        }
        let refs: Vec<_> = commits.iter().map(|x| x.as_ref()).collect();
        for n in 0..=refs.len() {
            let parents = &refs[..n];
            let encoding = (n % 2 == 1).then_some(c"ISO-8859-1");
            let buf =
                create_buffer(repo, me, them, encoding, c"msg\n", tree.as_ref(), parents).unwrap();
            let id = create(
                repo,
                None,
                me,
                them,
                encoding,
                c"msg\n",
                tree.as_ref(),
                parents,
            )
            .unwrap();
            assert_eq!(s.raw(&id), buf.as_ref().as_bytes(), "{n} parents");
            let commit = Commit::lookup(repo, id.as_ref()).unwrap();
            assert_eq!(commit.as_ref().author(), me);
            assert_eq!(commit.as_ref().committer(), them);
            let array = match n {
                0 => create_from_array(repo, None, me, them, encoding, c"msg\n", tree.as_ref(), []),
                1 => create_from_array(
                    repo,
                    None,
                    me,
                    them,
                    encoding,
                    c"msg\n",
                    tree.as_ref(),
                    [refs[0]],
                ),
                2 => create_from_array(
                    repo,
                    None,
                    me,
                    them,
                    encoding,
                    c"msg\n",
                    tree.as_ref(),
                    [refs[0], refs[1]],
                ),
                _ => create_from_array(
                    repo,
                    None,
                    me,
                    them,
                    encoding,
                    c"msg\n",
                    tree.as_ref(),
                    [refs[0], refs[1], refs[2]],
                ),
            };
            assert_eq!(array.unwrap(), id, "{n} parents");
        }
        // The largest array the variadic call takes.
        let many = [refs[0]; MAX_ARRAY_PARENTS];
        let id =
            create_from_array(repo, None, me, me, None, c"octopus\n", tree.as_ref(), many).unwrap();
        let text = String::from_utf8(s.raw(&id)).unwrap();
        assert_eq!(text.matches("\nparent ").count(), MAX_ARRAY_PARENTS);
        let slice = create(repo, None, me, me, None, c"octopus\n", tree.as_ref(), &many).unwrap();
        assert_eq!(slice, id);
    }

    /// `create` and `amend` move the reference they are given, from its first parent
    /// only, and the extended form takes its reference, encoding and headers from the
    /// options. Reference writes reach glibc's timezone state (see the `TSan` note on the
    /// reflog), so the test runs alone.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn references_move_from_the_first_parent() {
        isolated(
            concat!(module_path!(), "::references_move_from_the_first_parent"),
            references_move_from_the_first_parent_body,
        );
    }

    #[expect(
        clippy::items_after_statements,
        reason = "each helper item sits beside the steps that use it"
    )]
    fn references_move_from_the_first_parent_body() {
        let s = Scratch::new("refs");
        let (repo, tree) = (s.repo(), s.empty_tree());
        let me = who(0);
        let me = me.as_ref();
        // The branch `HEAD` names (whatever the default branch is called), read back.
        let head = |s: &Scratch| -> Option<String> {
            let target = std::fs::read_to_string(s.dir.join("HEAD")).unwrap();
            let name = target.trim().strip_prefix("ref: ").unwrap().to_owned();
            std::fs::read_to_string(s.dir.join(name))
                .ok()
                .map(|t| t.trim().to_owned())
        };
        let root = create(
            repo,
            Some(c"HEAD"),
            me,
            me,
            None,
            c"root\n",
            tree.as_ref(),
            &[],
        )
        .unwrap();
        assert_eq!(head(&s), Some(root.as_ref().to_hex_string().unwrap()));
        let root = Commit::lookup(repo, root.as_ref()).unwrap();
        // A root commit cannot move a reference that already exists.
        let e = create(
            repo,
            Some(c"HEAD"),
            me,
            me,
            None,
            c"r2\n",
            tree.as_ref(),
            &[],
        )
        .unwrap_err();
        assert_eq!(e.code(), Some(ErrorCode::Modified));
        assert_eq!(
            e.message(),
            c"failed to create commit: current tip is not the first parent"
        );
        // The extended form, from the options.
        const HEADERS: [Header<'static>; 1] = [Header::new(c"x-tool", c"crustify\nunit")];
        let mut options = CreateExtOptions::new();
        let mut m = options.as_mut();
        m.set_update_ref(Some(c"HEAD"));
        m.set_message_encoding(Some(c"ISO-8859-1"));
        m.set_extra_headers(&HEADERS);
        let child = create_ext(
            repo,
            me,
            me,
            c"child\n",
            tree.as_ref(),
            &[root.as_ref()],
            Some(options.as_mut()),
        )
        .unwrap();
        assert_eq!(head(&s), Some(child.as_ref().to_hex_string().unwrap()));
        let text = String::from_utf8(s.raw(&child)).unwrap();
        assert!(text.contains("\nencoding ISO-8859-1\nx-tool crustify\n unit\n\nchild\n"));
        let child = Commit::lookup(repo, child.as_ref()).unwrap();

        // Amending the root is refused while the branch points elsewhere; amending the
        // tip moves the branch to the copy, which keeps the parents.
        let e = root
            .as_ref()
            .amend(Some(c"HEAD"), None, None, None, None, None)
            .unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Reference));
        assert_eq!(
            e.message(),
            c"commit to amend is not the tip of the given branch"
        );
        let other = who(-60);
        let amended = child
            .as_ref()
            .amend(
                Some(c"HEAD"),
                None,
                Some(other.as_ref()),
                None,
                Some(c"amended\n"),
                None,
            )
            .unwrap();
        assert_eq!(head(&s), Some(amended.as_ref().to_hex_string().unwrap()));
        let text = String::from_utf8(s.raw(&amended)).unwrap();
        let expected = format!(
            "tree {}\nparent {}\nauthor A U Thor <a@example.com> 1700000000 +0000\n\
             committer A U Thor <a@example.com> 1700000000 -0100\nencoding ISO-8859-1\n\n\
             amended\n",
            EMPTY_TREE.to_str().unwrap(),
            root.as_ref().id().to_hex_string().unwrap(),
        );
        assert_eq!(text, expected);
    }

    /// A panic of the signing callback lent to the options resumes from `create_ext`,
    /// after libgit2 returned without writing; the callback signs again afterwards, and
    /// an error it returns fails the call with its code and message.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn create_ext_resumes_the_signers_panic() {
        let s = Scratch::new("signer-panic");
        let (repo, tree) = (s.repo(), s.empty_tree());
        let me = who(0);
        let me = me.as_ref();
        let mut calls = 0;
        let mut sign =
            |mut b: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
                calls += 1;
                match calls {
                    1 => panic!("signing failed"),
                    2 => Err(StreamError::new(ErrorClass::Callback, c"no key")
                        .with_code(ErrorCode::User)),
                    _ => b.add_header(c"gpgsig", c"sig").map_err(StreamError::from),
                }
            };
        let mut signer = SignatureCallback::new(&mut sign);
        let mut options = CreateExtOptions::new();
        options.as_mut().set_signer(Some(&mut signer));
        let before = s.objects();
        let caught = catch_unwind(AssertUnwindSafe(|| {
            create_ext(
                repo,
                me,
                me,
                c"m\n",
                tree.as_ref(),
                &[],
                Some(options.as_mut()),
            )
        }));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"signing failed")
        );
        assert_eq!(s.objects(), before);
        let e = create_ext(
            repo,
            me,
            me,
            c"m\n",
            tree.as_ref(),
            &[],
            Some(options.as_mut()),
        )
        .unwrap_err();
        assert_eq!(
            refusal(e),
            (
                ErrorCode::User.raw(),
                Some(ErrorClass::Callback),
                "no key".into()
            )
        );
        let id = create_ext(
            repo,
            me,
            me,
            c"m\n",
            tree.as_ref(),
            &[],
            Some(options.as_mut()),
        )
        .unwrap();
        drop(signer);
        assert_eq!(calls, 3);
        let text = String::from_utf8(s.raw(&id)).unwrap();
        assert!(text.contains("\ngpgsig sig\n\nm\n"));
        // Without options, nothing signs.
        let plain = create_ext(repo, me, me, c"m\n", tree.as_ref(), &[], None).unwrap();
        assert!(!String::from_utf8(s.raw(&plain)).unwrap().contains("gpgsig"));
    }

    /// The signatures and the body are borrowed from the commit, at the addresses C
    /// returns; the body is computed once and kept; a mailmap resolves copies.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signatures_and_body_are_read_in_place() {
        let s = Scratch::new("read");
        let (repo, tree) = (s.repo(), s.empty_tree());
        let (me, them) = (who(30), who(-30));
        let messages: [(&CStr, Option<&CStr>); 4] = [
            (
                c"summary\n\n  body line\n second\n\n\n",
                Some(c"body line\n second"),
            ),
            (c"only a summary\n", None),
            (c"\n\nsummary\n\n \t\n", None),
            (c"s\n\nb", Some(c"b")),
        ];
        for (message, body) in messages {
            let id = create(
                repo,
                None,
                me.as_ref(),
                them.as_ref(),
                None,
                message,
                tree.as_ref(),
                &[],
            )
            .unwrap();
            let commit = Commit::lookup(repo, id.as_ref()).unwrap();
            let r = commit.as_ref();
            // SAFETY: the live commit the owner holds; the getters only read it.
            let commit_author = unsafe { ffi::git_commit_author(r.as_ptr()) };
            // SAFETY: as above.
            let (author, committer) =
                unsafe { (commit_author, ffi::git_commit_committer(r.as_ptr())) };
            assert_eq!(r.author().as_ptr(), author);
            assert_eq!(r.committer().as_ptr(), committer);
            assert_eq!(r.author(), me.as_ref());
            assert_eq!(r.committer(), them.as_ref());
            let first = r.body();
            assert_eq!(first, body, "{message:?}");
            let again = r.body();
            assert_eq!(first.map(CStr::as_ptr), again.map(CStr::as_ptr));
            // A copy of the owner shares the commit, and its cached body.
            let copy = commit.clone();
            assert_eq!(
                copy.as_ref().body().map(CStr::as_ptr),
                first.map(CStr::as_ptr)
            );
            drop(commit);
            assert_eq!(copy.as_ref().body(), body);
        }

        let id = create(
            repo,
            None,
            me.as_ref(),
            them.as_ref(),
            None,
            c"m\n",
            tree.as_ref(),
            &[],
        )
        .unwrap();
        let commit = Commit::lookup(repo, id.as_ref()).unwrap();
        let mailmap =
            Mailmap::from_buffer(b"Real Name <real@example.com> <a@example.com>\n").unwrap();
        let mapped = commit
            .as_ref()
            .author_with_mailmap(Some(mailmap.as_ref()))
            .unwrap();
        assert_eq!(mapped.as_ref().name(), Some(c"Real Name"));
        assert_eq!(mapped.as_ref().email(), Some(c"real@example.com"));
        assert_eq!(mapped.as_ref().when(), me.as_ref().when());
        let mapped = commit
            .as_ref()
            .committer_with_mailmap(Some(mailmap.as_ref()))
            .unwrap();
        assert_eq!(mapped.as_ref().when(), them.as_ref().when());
        let unmapped = commit.as_ref().author_with_mailmap(None).unwrap();
        assert_eq!(unmapped.as_ref(), me.as_ref());
        assert_ne!(
            unmapped.as_ref().as_ptr(),
            commit.as_ref().author().as_ptr()
        );

        // A name libgit2's signature constructor refuses fails the resolution.
        let empty = SignatureView::new(c"", c"e@example.com", Time::new(0, 0));
        let id = create(
            repo,
            None,
            empty.as_ref(),
            me.as_ref(),
            None,
            c"m\n",
            tree.as_ref(),
            &[],
        )
        .unwrap();
        let commit = Commit::lookup(repo, id.as_ref()).unwrap();
        let e = commit.as_ref().author_with_mailmap(None).unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        assert!(commit.as_ref().committer_with_mailmap(None).is_ok());
    }

    /// Every parent count the variadic call takes writes the commit the slice form
    /// writes for the same parents.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[cfg_attr(
        bsan,
        ignore = "BorrowSanitizer reports git_commit_create_v's later variadic parents as freed"
    )]
    #[expect(
        clippy::items_after_statements,
        reason = "each helper item sits beside the steps that use it"
    )]
    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "the closure lets each handle reborrow for its own lifetime, which the method path cannot"
    )]
    fn every_array_length_writes_the_slice_commit() {
        let s = Scratch::new("array-lengths");
        let (repo, tree) = (s.repo(), s.empty_tree());
        let me = who(0);
        let me = me.as_ref();
        let parents: Vec<_> = (0..MAX_ARRAY_PARENTS)
            .map(|i| {
                let message = CString::new(format!("p{i}\n")).unwrap();
                let id = create(repo, None, me, me, None, &message, tree.as_ref(), &[]).unwrap();
                Commit::lookup(repo, id.as_ref()).unwrap()
            })
            .collect();
        let refs: Vec<_> = parents.iter().map(|x| x.as_ref()).collect();
        fn both<const N: usize>(
            repo: RepositoryRef<'_>,
            me: SignatureRef<'_>,
            tree: TreeRef<'_, '_>,
            refs: &[CommitRef<'_, '_>],
        ) {
            let array: [CommitRef<'_, '_>; N] = core::array::from_fn(|i| refs[i]);
            let a = create_from_array(repo, None, me, me, None, c"n\n", tree, array).unwrap();
            let s = create(repo, None, me, me, None, c"n\n", tree, &refs[..N]).unwrap();
            assert_eq!(a, s, "{N} parents");
        }
        macro_rules! lengths {
            ($($n:literal)*) => { $(both::<$n>(repo, me, tree.as_ref(), &refs);)* };
        }
        lengths!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16);
    }

    /// The trampoline refuses a parent run that is NULL for a non-zero count, or that
    /// holds a NULL commit, before the closure runs.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn create_callback_refuses_null_parents() {
        let s = Scratch::new("create-cb-null");
        let tree = s.empty_tree();
        let me = who(0);
        let mut calls = 0;
        let mut f = |_: SignatureRef<'_>,
                     _: SignatureRef<'_>,
                     _: Option<&CStr>,
                     _: &CStr,
                     _: TreeRef<'_, '_>,
                     _: &[CommitRef<'_, '_>]| {
            calls += 1;
            Ok::<_, StreamError>(None)
        };
        let mut callback = CreateCallback::new(&mut f);
        let mut out = ffi::git_oid::default();
        let mut holes = [ptr::null::<ffi::git_commit>(); 2];
        for (count, parents) in [(2, ptr::null_mut()), (2, holes.as_mut_ptr())] {
            let payload: *mut c_void = ptr::from_mut(&mut callback).cast();
            // SAFETY: the payload is the live callback, used by nothing else; the
            // signatures, message and tree are live for the call; the parents are NULL,
            // or a run of `count` NULL pointers, which the trampoline refuses unread.
            let rc = unsafe {
                create_with_closure(
                    &raw mut out,
                    me.as_ref().as_ptr(),
                    me.as_ref().as_ptr(),
                    ptr::null(),
                    c"m".as_ptr(),
                    tree.as_ref().as_ptr(),
                    count,
                    parents,
                    payload,
                )
            };
            assert_eq!(rc, ErrorCode::User.raw());
        }
        drop(callback);
        assert_eq!(calls, 0);
    }

    /// The creation callback's trampoline refuses NULL arguments and a pending panic
    /// before reading anything else (Miri runs it: no libgit2 call is reached).
    #[test]
    fn create_callback_refuses_null_arguments() {
        let mut calls = 0;
        let mut f = |_: SignatureRef<'_>,
                     _: SignatureRef<'_>,
                     _: Option<&CStr>,
                     _: &CStr,
                     _: TreeRef<'_, '_>,
                     _: &[CommitRef<'_, '_>]| {
            calls += 1;
            Ok::<_, StreamError>(None)
        };
        let mut callback = CreateCallback::new(&mut f);
        let payload: *mut c_void = ptr::from_mut(&mut callback).cast();
        let mut out = ffi::git_oid::default();
        // SAFETY: the payload is the live callback, used by nothing else; every other
        // pointer is NULL, which the trampoline refuses before reading anything.
        let rc = unsafe {
            create_with_closure(
                &raw mut out,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                0,
                ptr::null_mut(),
                payload,
            )
        };
        assert_eq!(rc, ErrorCode::User.raw());
        callback.panic = Some(Box::new("pending"));
        // Writing the callback invalidated the pointer taken before: take it again.
        let payload: *mut c_void = ptr::from_mut(&mut callback).cast();
        // SAFETY: as above; the pending panic stops the trampoline first.
        let rc = unsafe {
            create_with_closure(
                &raw mut out,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                0,
                ptr::null_mut(),
                payload,
            )
        };
        assert_eq!(rc, ErrorCode::User.raw());
        assert!(format!("{callback:?}").starts_with("CreateCallback { panicked: true"));
        drop(callback);
        assert_eq!(calls, 0);
    }

    /// `call` drives the trampoline as libgit2 would: each outcome of the closure comes
    /// back as libgit2 reads it, an id of another type is refused, and a panic resumes
    /// from `call`, after which the callback works again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn create_callback_call_maps_each_outcome() {
        let s = Scratch::new("create-cb");
        let (repo, tree) = (s.repo(), s.empty_tree());
        let me = who(0);
        let id = create(
            repo,
            None,
            me.as_ref(),
            me.as_ref(),
            None,
            c"p\n",
            tree.as_ref(),
            &[],
        )
        .unwrap();
        let parent = Commit::lookup(repo, id.as_ref()).unwrap();
        let mut step = 0;
        let mut seen = Vec::new();
        let mut f = |author: SignatureRef<'_>,
                     _: SignatureRef<'_>,
                     encoding: Option<&CStr>,
                     message: &CStr,
                     _: TreeRef<'_, '_>,
                     parents: &[CommitRef<'_, '_>]| {
            step += 1;
            seen.push((
                author.name().map(CStr::to_owned),
                encoding.map(CStr::to_owned),
                message.to_owned(),
                parents.len(),
            ));
            match step {
                1 => Ok(Some(sha1(HEAD))),
                2 => Ok(None),
                3 => Err(StreamError::new(ErrorClass::Callback, c"refused")),
                4 => Ok(Some(Oid::zero(OidType::Sha256))),
                5 => panic!("callback panicked"),
                _ => Err(StreamError::new(ErrorClass::Callback, c"late")
                    .with_code(ErrorCode::Passthrough)),
            }
        };
        let mut callback = CreateCallback::new(&mut f);
        let call = |cb: &mut CreateCallback<'_>| {
            cb.call(
                me.as_ref(),
                me.as_ref(),
                Some(c"UTF-8"),
                c"msg\n",
                tree.as_ref(),
                &[parent.as_ref()],
            )
        };
        assert_eq!(call(&mut callback).unwrap().unwrap(), sha1(HEAD));
        assert!(call(&mut callback).unwrap().is_none());
        let e = call(&mut callback).unwrap_err();
        assert_eq!(
            refusal(e),
            (-1, Some(ErrorClass::Callback), "refused".into())
        );
        let e = call(&mut callback).unwrap_err();
        assert_eq!(
            refusal(e),
            (
                -1,
                Some(ErrorClass::Invalid),
                "specified object ID type does not match repository object ID type".into()
            )
        );
        let caught = catch_unwind(AssertUnwindSafe(|| call(&mut callback)));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"callback panicked")
        );
        // A passthrough code reads as a passthrough.
        assert!(call(&mut callback).unwrap().is_none());
        drop(callback);
        assert_eq!(seen.len(), 6);
        assert!(seen.iter().all(|s| s
            == &(
                Some(c"A U Thor".to_owned()),
                Some(c"UTF-8".to_owned()),
                c"msg\n".to_owned(),
                1
            )));
    }

    /// The creation callback under its C caller, `git_rebase_commit`: an in-memory
    /// rebase of one commit onto another, whose replayed commit the callback writes
    /// itself, leaves to libgit2, refuses, or panics on.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[expect(
        clippy::too_many_lines,
        reason = "one scenario, its steps run and compared in order"
    )]
    fn create_callback_writes_rebased_commits() {
        let s = Scratch::new("rebase");
        let repo = s.repo();
        let me = who(0);
        let me = me.as_ref();
        let raw_repo = repo.as_ptr().cast_mut();
        // A tree holding `file` with `content`.
        let tree_of = |content: &[u8]| -> Oid {
            let blob =
                crate::libgit2::src::libgit2::blob::create_from_buffer(repo, content).unwrap();
            let (mut builder, mut id) = (ptr::null_mut(), Oid::zeroed());
            {
                // SAFETY: writable slots, the live repository, a NUL-terminated name and a
                // live blob id; the builder is freed once.
                unsafe {
                    assert_eq!(
                        ffi::git_treebuilder_new(&raw mut builder, raw_repo, ptr::null()),
                        0
                    );
                };
                // SAFETY: as above.
                let rc = unsafe {
                    ffi::git_treebuilder_insert(
                        ptr::null_mut(),
                        builder,
                        c"file".as_ptr(),
                        blob.as_ref().as_ptr(),
                        ffi::GIT_FILEMODE_BLOB,
                    )
                };
                assert_eq!(rc, 0);
                // SAFETY: as above.
                unsafe {
                    assert_eq!(
                        ffi::git_treebuilder_write(id.as_mut().as_mut_ptr(), builder),
                        0
                    );
                };
                // SAFETY: as above.
                unsafe { ffi::git_treebuilder_free(builder) };
            }
            id
        };
        let commit_of = |content: &[u8], parents: &[CommitRef<'_, '_>], message: &CStr| {
            let tree = Tree::lookup(repo, tree_of(content).as_ref()).unwrap();
            let id = create(repo, None, me, me, None, message, tree.as_ref(), parents).unwrap();
            Commit::lookup(repo, id.as_ref()).unwrap()
        };
        let base = commit_of(b"base\n", &[], c"base\n");
        let upstream = commit_of(b"base\nupstream\n", &[base.as_ref()], c"upstream\n");
        let branch = commit_of(b"branch\nbase\n", &[base.as_ref()], c"branch\n");

        let annotated = |c: &CommitArc<'_>| {
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot, the live repository and a commit id of it.
            let rc = unsafe {
                ffi::git_annotated_commit_lookup(&raw mut out, raw_repo, c.as_ref().id().as_ptr())
            };
            assert_eq!(rc, 0);
            out
        };
        // Replays `branch` onto `upstream` in memory, through `cb`: the code of
        // `git_rebase_commit`, and the id it wrote.
        let rebase = |cb: &mut CreateCallback<'_>| -> (c_int, Oid) {
            let mut id = Oid::zeroed();
            let out = id.as_mut().as_mut_ptr();
            let rc = cb.create_with(|callback, payload| {
                let (b, u) = (annotated(&branch), annotated(&upstream));
                // SAFETY: a zeroed `git_rebase_options` is filled by its init routine.
                let mut opts: ffi::git_rebase_options = unsafe { core::mem::zeroed() };
                let (mut rebase, mut op) = (ptr::null_mut(), ptr::null_mut());
                {
                    // `GIT_REBASE_OPTIONS_VERSION`.
                    // SAFETY: writable slots; the live repository and annotated commits;
                    // options whose callback and payload are `create_with`'s, live for the
                    // calls; the rebase and the annotated commits are freed once, before
                    // `create_with` resumes a panic.
                    unsafe { assert_eq!(ffi::git_rebase_options_init(&raw mut opts, 1), 0) };
                    opts.inmemory = 1;
                    opts.commit_create_cb = callback;
                    opts.payload = payload;
                    // SAFETY: as above.
                    unsafe {
                        assert_eq!(
                            ffi::git_rebase_init(
                                &raw mut rebase,
                                raw_repo,
                                b,
                                u,
                                ptr::null(),
                                &raw const opts
                            ),
                            0
                        );
                    };
                    // SAFETY: as above.
                    unsafe { assert_eq!(ffi::git_rebase_next(&raw mut op, rebase), 0) };
                    // SAFETY: as above.
                    let rc = unsafe {
                        ffi::git_rebase_commit(
                            out,
                            rebase,
                            ptr::null(),
                            me.as_ptr(),
                            ptr::null(),
                            ptr::null(),
                        )
                    };
                    // SAFETY: as above.
                    unsafe { ffi::git_rebase_free(rebase) };
                    // SAFETY: as above.
                    unsafe { ffi::git_annotated_commit_free(b) };
                    // SAFETY: as above.
                    unsafe { ffi::git_annotated_commit_free(u) };
                    rc
                }
            });
            (rc, id)
        };

        let outcome = core::cell::Cell::new(0);
        let mut seen = Vec::new();
        let mut f =
            |author: SignatureRef<'_>,
             committer: SignatureRef<'_>,
             encoding: Option<&CStr>,
             message: &CStr,
             tree: TreeRef<'_, '_>,
             parents: &[CommitRef<'_, '_>]| {
                let parent_ids: Vec<_> = parents
                    .iter()
                    .map(|p| p.id().to_hex_string().unwrap())
                    .collect();
                seen.push((
                    author.name().map(CStr::to_owned),
                    encoding.is_none(),
                    message.to_owned(),
                    parent_ids,
                ));
                match outcome.get() {
                    0 => {
                        let owner = tree.owner();
                        let id = create(
                            owner,
                            None,
                            author,
                            committer,
                            encoding,
                            c"written by the callback\n",
                            tree,
                            parents,
                        )
                        .map_err(|e| StreamError::from(ErrorBuf::from(e)))?;
                        Ok(Some(id))
                    }
                    1 => Ok(None),
                    2 => Err(StreamError::new(ErrorClass::Callback, c"not now")
                        .with_code(ErrorCode::User)),
                    _ => panic!("rebase callback panicked"),
                }
            };
        let mut callback = CreateCallback::new(&mut f);
        let upstream_hex = upstream.as_ref().id().to_hex_string().unwrap();

        let (rc, written) = rebase(&mut callback);
        assert_eq!(rc, 0);
        let commit = Commit::lookup(repo, written.as_ref()).unwrap();
        assert_eq!(commit.as_ref().committer(), me);
        assert!(
            String::from_utf8(s.raw(&written))
                .unwrap()
                .ends_with("\n\nwritten by the callback\n")
        );

        outcome.set(1);
        let (rc, passthrough) = rebase(&mut callback);
        assert_eq!(rc, 0);
        assert!(
            String::from_utf8(s.raw(&passthrough))
                .unwrap()
                .ends_with("\n\nbranch\n")
        );
        assert_ne!(passthrough, written);

        outcome.set(2);
        let (rc, _) = rebase(&mut callback);
        assert_eq!(rc, ErrorCode::User.raw());
        assert_eq!(last().message(), c"not now");

        outcome.set(3);
        let caught = catch_unwind(AssertUnwindSafe(|| rebase(&mut callback)));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"rebase callback panicked")
        );
        drop(callback);
        assert_eq!(seen.len(), 4);
        for s in &seen {
            assert_eq!(
                s,
                &(
                    Some(c"A U Thor".to_owned()),
                    true,
                    c"branch\n".to_owned(),
                    vec![upstream_hex.clone()]
                )
            );
        }
    }

    // --- Committing on `HEAD` --------------------------------------------------------

    /// The copy the creating routines hand `git_commit_create_ext` carries the options'
    /// encoding, headers, signer and payload, and nothing else: no reference to update,
    /// no signatures (`init_ext_options`).
    #[test]
    #[cfg_attr(miri, ignore = "fills the copy through libgit2")]
    fn ext_options_copy_what_libgit2_copies() {
        let headers = [Header::new(c"x-a", c"1"), Header::new(c"x-b", c"2")];
        let who = SignatureView::new(c"W", c"w@example.com", Time::new(0, 0));
        let mut f = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| Ok(());
        let mut signer = SignatureCallback::new(&mut f);
        let mut options = create_template();
        let mut m = options.as_mut();
        m.set_allow_empty_commit(true);
        m.set_author(Some(who.as_ref()));
        m.set_committer(Some(who.as_ref()));
        m.set_message_encoding(Some(c"UTF-16"));
        m.set_extra_headers(&headers);
        m.set_signer(Some(&mut signer));
        let (sign, payload) = m.as_ref().sign_callback();
        let seen = m.with_ext_options(|ext| {
            let r = ext.as_ref();
            assert_eq!(r.version(), ffi::GIT_COMMIT_CREATE_EXT_OPTIONS_VERSION);
            assert_eq!(r.update_ref(), None);
            assert_eq!(r.message_encoding(), Some(c"UTF-16"));
            assert_eq!(r.extra_headers().len(), 2);
            assert_eq!(r.extra_headers().as_ptr(), headers.as_ptr().cast());
            // SAFETY: a read of the live copy through its shared handle's pointer.
            let raw = unsafe { r.as_ptr().read() };
            assert_eq!(raw.sign.map(|f| f as usize), sign.map(|f| f as usize));
            assert_eq!(raw.payload, payload);
            7
        });
        assert_eq!(seen, 7);
        // Options without a signer, encoding or headers copy none.
        let mut bare = create_template();
        bare.as_mut().with_ext_options(|ext| {
            let r = ext.as_ref();
            assert!(!r.has_signer() && r.message_encoding().is_none());
            assert!(r.extra_headers().is_empty());
        });
        // Nothing to resume on either.
        options.as_mut().resume_signer_panic();
        bare.as_mut().resume_signer_panic();
    }

    /// The wrappers refuse, before libgit2 runs, a signature that lost a string (as
    /// libgit2's parser leaves one when out of memory), set as the author or the
    /// committer, and a tree of another repository: nothing is written, the index's tree
    /// included, and `HEAD` stays unborn.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn head_routines_refuse_what_libgit2_cannot_write() {
        let (s, other) = (
            Scratch::new("head-refusals"),
            Scratch::new("head-refusals-other"),
        );
        let (repo, tree, foreign) = (s.repo(), s.empty_tree(), other.empty_tree());
        let mut raw = ffi::git_signature {
            name: ptr::null_mut(),
            email: c"e@x".as_ptr().cast_mut(),
            when: Time::new(0, 0).into(),
        };
        // SAFETY: a live signature on the stack, whose strings are NULL or static, read
        // only while it lives.
        let lost = unsafe { SignatureRef::from_ptr(&raw mut raw) }.unwrap();
        let missing = (
            -1,
            Some(ErrorClass::Invalid),
            "failed to create commit: signature is missing its name or email".to_owned(),
        );
        let before = s.objects();
        for as_author in [true, false] {
            let mut options = CreateOptions::new();
            let mut m = options.as_mut();
            m.set_allow_empty_commit(true);
            if as_author {
                m.set_author(Some(lost));
            } else {
                m.set_committer(Some(lost));
            }
            let out = [
                super::create_from_tree(repo, tree.as_ref(), c"m\n", Some(m.as_mut())),
                create_from_stage(repo, c"m\n", Some(m.as_mut())),
                amend_from_tree(repo, tree.as_ref(), None, Some(m.as_mut())),
                amend_from_stage(repo, None, Some(m.as_mut())),
            ];
            for e in out {
                assert_eq!(refusal(e.unwrap_err()), missing);
            }
        }
        let owner = (
            -1,
            Some(ErrorClass::Invalid),
            "invalid argument: 'git_tree_owner(tree) == repo'".to_owned(),
        );
        let e = super::create_from_tree(repo, foreign.as_ref(), c"m\n", None).unwrap_err();
        assert_eq!(refusal(e), owner);
        let e = amend_from_tree(repo, foreign.as_ref(), None, None).unwrap_err();
        assert_eq!(refusal(e), owner);
        assert_eq!(s.objects(), before);
        assert!(repo.head().is_err());
    }

    /// An amend that leaves the author unset writes `HEAD`'s commit's author: when
    /// libgit2's parser lost its name (out of memory, reporting success), the wrappers
    /// refuse before libgit2 runs, and the commit they hold keeps libgit2 reading the
    /// same parse.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn amended_authors_libgit2_lost_are_refused() {
        isolated(
            concat!(module_path!(), "::amended_authors_libgit2_lost_are_refused"),
            || {
                install_refusing();
                let s = Scratch::new("lost-author");
                let (repo, tree) = (s.repo(), s.empty_tree());
                let name = CString::new("N".repeat(200)).unwrap();
                let author = SignatureView::new(&name, c"a@example.com", Time::new(5, 0));
                let me = who(0);
                let id = create(
                    repo,
                    Some(c"HEAD"),
                    author.as_ref(),
                    me.as_ref(),
                    None,
                    c"root\n",
                    tree.as_ref(),
                    &[],
                )
                .unwrap();
                // Writing `HEAD` looked the commit up for its reflog message: a fresh
                // instance of the repository, with an empty object cache, parses it anew.
                let path = CString::new(s.dir.to_str().unwrap()).unwrap();
                let fresh = Repository::open(&path).unwrap();
                let (repo, tree) = (
                    fresh.as_ref(),
                    Tree::lookup(fresh.as_ref(), tree.as_ref().id()).unwrap(),
                );
                let before = s.objects();
                let mut options = CreateOptions::new();
                options.as_mut().set_committer(Some(me.as_ref()));
                // The name's copy is the parse's only block of its length.
                REFUSED_LEN.store(201, SeqCst);
                let tree_amend = amend_from_tree(repo, tree.as_ref(), None, Some(options.as_mut()));
                let stage_amend = amend_from_stage(repo, None, Some(options.as_mut()));
                REFUSED_LEN.store(0, SeqCst);
                for e in [tree_amend, stage_amend] {
                    assert_eq!(
                        refusal(e.unwrap_err()).2,
                        "failed to create commit: signature is missing its name or email"
                    );
                }
                let cached = Commit::lookup(repo, id.as_ref()).unwrap();
                assert_eq!(cached.as_ref().author().name(), None);
                // `amend_from_stage` wrote the index's tree, the empty one libgit2 knows.
                assert!(s.objects() <= before + 1);
                // With an author set, `HEAD`'s is not written, nor checked.
                options.as_mut().set_author(Some(me.as_ref()));
                let amended = amend_from_tree(repo, tree.as_ref(), None, Some(options.as_mut()));
                let amended = Commit::lookup(repo, amended.unwrap().as_ref()).unwrap();
                assert_eq!(amended.as_ref().author().name(), Some(c"A U Thor"));
                assert_eq!(amended.as_ref().message(), c"root\n");
            },
        );
    }

    /// The amended commit's author the wrappers check is the one libgit2 writes, also
    /// when the object cache does not keep the commit (libgit2 caches none over 4096
    /// bytes), so that every lookup parses it anew: a parse of libgit2's own losing the
    /// author's name after the wrapper's kept it writes no `(null)` author.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn uncached_amended_authors_are_the_ones_checked() {
        isolated(
            concat!(
                module_path!(),
                "::uncached_amended_authors_are_the_ones_checked"
            ),
            || {
                install_refusing();
                let s = Scratch::new("uncached-author");
                let (repo, tree) = (s.repo(), s.empty_tree());
                let name = CString::new("N".repeat(200)).unwrap();
                let author = SignatureView::new(&name, c"a@example.com", Time::new(5, 0));
                let me = who(0);
                let message = CString::new(format!("{}\n", "m".repeat(5000))).unwrap();
                let root = (author.as_ref(), me.as_ref(), message.as_c_str());
                create(
                    repo,
                    Some(c"HEAD"),
                    root.0,
                    root.1,
                    None,
                    root.2,
                    tree.as_ref(),
                    &[],
                )
                .unwrap();
                let mut options = CreateOptions::new();
                options.as_mut().set_committer(Some(me.as_ref()));
                for stage in [false, true] {
                    // The name's copy is the parse's only block of its length: the first
                    // parse of `HEAD`'s commit keeps it, every later one loses it.
                    REFUSED_LEN_GRANTS.store(1, SeqCst);
                    REFUSED_LEN.store(201, SeqCst);
                    let amended = if stage {
                        amend_from_stage(repo, None, Some(options.as_mut()))
                    } else {
                        amend_from_tree(repo, tree.as_ref(), None, Some(options.as_mut()))
                    };
                    REFUSED_LEN.store(0, SeqCst);
                    let amended = Commit::lookup(repo, amended.unwrap().as_ref()).unwrap();
                    assert_eq!(amended.as_ref().author().name(), Some(name.as_c_str()));
                    assert_eq!(amended.as_ref().message(), message.as_c_str());
                }
            },
        );
    }

    /// What `git_commit_create_from_tree` or `git_commit_create_from_stage` (for `None`)
    /// gives on `repo`'s `HEAD` with an empty commit allowed: the code, class and message,
    /// or 0.
    fn c_commit_on_head(
        repo: RepositoryRef<'_>,
        tree: Option<TreeRef<'_, '_>>,
    ) -> (c_int, Option<ErrorClass>, String) {
        let mut options = CreateOptions::new();
        options.as_mut().set_allow_empty_commit(false);
        let mut id = ffi::git_oid::default();
        let (raw, opts) = (repo.as_ptr().cast_mut(), options.as_ref().as_ptr());
        // Only called where libgit2 fails before collecting the parents.
        let rc = match tree {
            // SAFETY: a writable id; the live repository and a live tree of it; a
            // NUL-terminated message; options Rust holds, borrowing nothing.
            Some(tree) => unsafe {
                ffi::git_commit_create_from_tree(
                    &raw mut id,
                    raw,
                    tree.as_ptr(),
                    c"m\n".as_ptr(),
                    opts,
                )
            },
            // SAFETY: as above, without a tree.
            None => unsafe {
                ffi::git_commit_create_from_stage(&raw mut id, raw, c"m\n".as_ptr(), opts)
            },
        };
        if rc == 0 {
            return (0, None, String::new());
        }
        let e = last();
        (rc, e.class(), e.message().to_string_lossy().into_owned())
    }

    /// With empty commits allowed, the composed creating routines go on to the parents,
    /// and fail as libgit2's do where `HEAD` cannot be read: libgit2 fails collecting them
    /// before it holds any commit, so its routines are safe to compare with here.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn unreadable_heads_fail_collecting_parents_as_libgit2_does() {
        let s = Scratch::new("unreadable-head");
        let (repo, tree) = (s.repo(), s.empty_tree());
        let me = who(0);
        let mut options = CreateOptions::new();
        let mut m = options.as_mut();
        m.set_allow_empty_commit(true);
        m.set_author(Some(me.as_ref()));
        m.set_committer(Some(me.as_ref()));
        std::fs::write(s.dir.join("HEAD"), "not a reference\n").unwrap();
        let before = s.objects();
        for staged in [false, true] {
            let mut id = ffi::git_oid::default();
            let (raw, opts) = (repo.as_ptr().cast_mut(), m.as_ref().as_ptr());
            // libgit2 fails reading `HEAD`, before it collects a parent.
            let rc = if staged {
                // SAFETY: a writable id; the live repository; a NUL-terminated message;
                // options Rust holds, borrowing signatures that outlive the call.
                unsafe {
                    ffi::git_commit_create_from_stage(&raw mut id, raw, c"m\n".as_ptr(), opts)
                }
            } else {
                let t = tree.as_ref().as_ptr();
                // SAFETY: as above, with a live tree of the repository.
                unsafe {
                    ffi::git_commit_create_from_tree(&raw mut id, raw, t, c"m\n".as_ptr(), opts)
                }
            };
            let theirs = (
                rc,
                last().class(),
                last().message().to_string_lossy().into_owned(),
            );
            let ours = if staged {
                create_from_stage(repo, c"m\n", Some(m.as_mut()))
            } else {
                super::create_from_tree(repo, tree.as_ref(), c"m\n", Some(m.as_mut()))
            };
            assert_eq!(refusal(ours.unwrap_err()), theirs);
            assert_eq!(theirs.0, -1);
        }
        assert!(s.objects() <= before + 1, "at most the index's tree");
    }

    /// The composed creating routines fail as libgit2's do where `HEAD` cannot be
    /// peeled to a tree or is missing, where the index cannot be written as a tree (a
    /// conflict), where the diff against `HEAD` fails (a missing subtree), and where the
    /// repository has no index (no paths); amending the index fails alike.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[expect(
        clippy::too_many_lines,
        reason = "one scenario, its steps run and compared in order"
    )]
    fn composed_routines_fail_as_libgit2_does() {
        let s = Scratch::new("composed-failures");
        let (repo, tree) = (s.repo(), s.empty_tree());
        let tree_of = |r: RepositoryRef<'_>, t: &TreeArc<'_>| {
            let mut opts = CreateOptions::new();
            opts.as_mut().set_allow_empty_commit(false);
            let ours = super::create_from_tree(r, t.as_ref(), c"m\n", Some(opts.as_mut()));
            (
                ours.map(|_| ()).map_err(refusal),
                c_commit_on_head(r, Some(t.as_ref())),
            )
        };
        let stage_of = |r: RepositoryRef<'_>| {
            let ours = create_from_stage(r, c"m\n", None);
            (ours.map(|_| ()).map_err(refusal), c_commit_on_head(r, None))
        };
        // `HEAD` at a blob: no tree to diff against.
        let blob = crate::libgit2::src::libgit2::blob::create_from_buffer(repo, b"blob\n").unwrap();
        let head = s.dir.join("HEAD");
        std::fs::write(
            &head,
            format!("{}\n", blob.as_ref().to_hex_string().unwrap()),
        )
        .unwrap();
        let (ours, theirs) = tree_of(repo, &tree);
        assert_eq!(ours.unwrap_err(), theirs);
        let (ours, theirs) = stage_of(repo);
        assert_eq!(ours.unwrap_err(), theirs);
        assert!(theirs.0 < 0);
        // No `HEAD` at all.
        std::fs::remove_file(&head).unwrap();
        let (ours, theirs) = tree_of(repo, &tree);
        assert_eq!(ours.unwrap_err(), theirs);
        let (ours, theirs) = stage_of(repo);
        assert_eq!(ours.unwrap_err(), theirs);
        assert_eq!(theirs.0, ErrorCode::NotFound.raw());
        std::fs::write(&head, "ref: refs/heads/master\n").unwrap();

        // A conflicted index changes something, and cannot be written as a tree.
        let index = repo.index().unwrap();
        // SAFETY: a zeroed entry is a valid one with no stat data.
        let mut entry: ffi::git_index_entry = unsafe { core::mem::zeroed() };
        entry.mode = ffi::GIT_FILEMODE_BLOB;
        entry.id = ffi::git_oid {
            type_: blob.as_ref().raw_kind(),
            id: blob.as_ref().id(),
        };
        entry.path = c"conflicted".as_ptr();
        // Stage 1 (`GIT_INDEX_ENTRY_STAGESHIFT` is 12): the common ancestor's side.
        entry.flags = 1 << 12;
        // SAFETY: the live index; libgit2 copies the entry and its path.
        let ret = unsafe { ffi::git_index_add(index.as_c_ptr(), &raw const entry) };
        assert_eq!(ret, 0);
        let before = s.objects();
        let (ours, theirs) = stage_of(repo);
        assert_eq!(ours.unwrap_err(), theirs);
        assert_eq!(theirs.0, -1);
        // Amending writes the index first too.
        let e = amend_from_stage(repo, None, None).unwrap_err();
        assert_eq!(refusal(e), (-1, theirs.1, theirs.2.clone()));
        assert_eq!(s.objects(), before);
        drop(index);

        // `HEAD` at a commit whose tree names a missing subtree: the diff against it
        // fails while descending into it.
        let mut raw = b"40000 sub\0".to_vec();
        raw.extend_from_slice(&[0xAB; 20]);
        let (mut odb, mut broken) = (ptr::null_mut(), ffi::git_oid::default());
        {
            // SAFETY: writable slots, the live repository and `raw.len()` bytes; the odb
            // reference is released once.
            unsafe {
                assert_eq!(
                    ffi::git_repository_odb(&raw mut odb, repo.as_ptr().cast_mut()),
                    0
                );
            };
            // SAFETY: as above.
            let rc = unsafe {
                ffi::git_odb_write(
                    &raw mut broken,
                    odb,
                    raw.as_ptr().cast(),
                    raw.len(),
                    ffi::GIT_OBJECT_TREE,
                )
            };
            assert_eq!(rc, 0);
            // SAFETY: as above.
            unsafe { ffi::git_odb_free(odb) };
        }
        let broken = Oid::from_raw(&broken.id[..20], OidType::Sha1).unwrap();
        let broken = Tree::lookup(repo, broken.as_ref()).unwrap();
        let me = who(0);
        let tip = create(
            repo,
            None,
            me.as_ref(),
            me.as_ref(),
            None,
            c"broken\n",
            broken.as_ref(),
            &[],
        )
        .unwrap();
        std::fs::write(
            &head,
            format!("{}\n", tip.as_ref().to_hex_string().unwrap()),
        )
        .unwrap();
        let (ours, theirs) = tree_of(repo, &tree);
        assert_eq!(ours.unwrap_err(), theirs);
        assert_eq!(theirs.0, -1);
        let (ours, theirs) = stage_of(repo);
        assert_eq!(ours.unwrap_err(), theirs);
        assert!(theirs.0 < 0 && theirs.2 != "no changes are staged for commit");

        // A repository without paths has no index.
        let pathless = Repository::new().unwrap();
        let (ours, theirs) = stage_of(pathless.as_ref());
        assert_eq!(ours.unwrap_err(), theirs);
        assert_eq!(theirs.0, -1);
        let e = amend_from_stage(pathless.as_ref(), None, None).unwrap_err();
        assert_eq!(refusal(e), theirs);
    }

    mod sys_commit {
        use core::ffi::{CStr, c_int};
        use std::process::Command;

        use super::super::*;
        use crate::libgit2::src::util::errors::{ErrorBuf, last, runtime_pinned};

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// Runs `body` alone in a child process, so that the global options it sets reach
        /// no other test.
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

        /// A SHA-1 id of `byte`s, built without libgit2 (Miri runs the tests using it).
        fn sha1(byte: u8) -> Oid {
            let mut id = Oid::zero(OidType::Sha1);
            id.as_mut().set_id([byte; 32]);
            id
        }

        /// An id's bytes and type byte, read without libgit2.
        fn view(id: Option<OidRef<'_>>) -> Option<([u8; 32], u8)> {
            id.map(|id| (id.id(), id.raw_kind()))
        }

        /// The trampoline driven from Rust, without libgit2 (Miri runs it): each id comes
        /// back as the closure returned it, kept by the callback until its next call, and
        /// `None` ends the list. No type is checked outside a walk.
        #[test]
        fn call_hands_back_each_id() {
            let ids = [sha1(1), sha1(2)];
            let mut asked = Vec::new();
            let mut next = |i: usize| {
                asked.push(i);
                ids.get(i).copied()
            };
            let mut callback = ParentCallback::new(&mut next);
            assert_eq!(view(callback.call(0)), view(Some(ids[0].as_ref())));
            let second = view(callback.call(1));
            assert_eq!(second, view(Some(ids[1].as_ref())));
            assert!(callback.call(2).is_none());
            assert_eq!(view(callback.call(0)), view(Some(ids[0].as_ref())));
            assert!(format!("{callback:?}").starts_with("ParentCallback { panicked: false"));
            drop(callback);
            assert_eq!(asked, [0, 1, 2, 0]);

            let mut wide = |_: usize| Some(Oid::zero(OidType::Sha256));
            let mut callback = ParentCallback::new(&mut wide);
            assert_eq!(callback.call(0).unwrap().kind(), Some(OidType::Sha256));
        }

        /// A panic of the closure resumes from `call`, after the trampoline returned NULL;
        /// the callback works again afterwards.
        #[test]
        fn call_resumes_panics() {
            let mut calls = 0;
            let mut next = |_: usize| {
                calls += 1;
                assert!(calls != 1, "no parent");
                Some(sha1(3))
            };
            let mut callback = ParentCallback::new(&mut next);
            let caught = catch_unwind(AssertUnwindSafe(|| view(callback.call(0))));
            assert_eq!(
                caught.unwrap_err().downcast_ref::<&str>(),
                Some(&"no parent")
            );
            assert_eq!(view(callback.call(0)), view(Some(sha1(3).as_ref())));
        }

        /// `walk_with` asks the closure for every parent before the walk runs, then lends the
        /// walk a trampoline over them, which gives each id in turn, kept until the next
        /// call, then NULL; a panic propagates before the walk runs (Miri runs it: the walk
        /// here is Rust).
        #[test]
        fn walks_collect_every_parent_first() {
            let ids = [sha1(1), sha1(2), sha1(3)];
            let calls = core::cell::Cell::new(0);
            let mut next = |i: usize| {
                calls.set(calls.get() + 1);
                ids.get(i).copied()
            };
            let mut callback = ParentCallback::new(&mut next);
            let seen = callback
                .walk_with(OidType::Sha1, |cb, payload| {
                    assert_eq!(calls.get(), 4);
                    let cb = cb.expect("a trampoline");
                    let mut seen = Vec::new();
                    for i in 0..8 {
                        // SAFETY: the trampoline and payload `walk_with` lent, for this walk.
                        let id = unsafe { cb(i, payload) };
                        // SAFETY: NULL or the trampoline's last id, read before its next call.
                        seen.push(view(unsafe { OidRef::from_ptr(id.cast_mut()) }));
                    }
                    seen
                })
                .unwrap();
            let expected: Vec<_> = (0..8).map(|i| view(ids.get(i).map(Oid::as_ref))).collect();
            assert_eq!(seen, expected);
            drop(callback);
            assert_eq!(calls.get(), 4);

            let mut boom = |i: usize| -> Option<Oid> {
                assert!(i != 1, "parent 1");
                Some(sha1(9))
            };
            let mut walked = false;
            let caught = catch_unwind(AssertUnwindSafe(|| {
                drop(ParentCallback::new(&mut boom).walk_with(OidType::Sha1, |_, _| walked = true));
            }));
            assert_eq!(
                caught.unwrap_err().downcast_ref::<&str>(),
                Some(&"parent 1")
            );
            assert!(!walked);
        }

        /// A bare repository with the empty tree and an identity, writing commits through
        /// `git_commit_create_from_callback`.
        struct Writer {
            dir: std::path::PathBuf,
            repo: *mut ffi::git_repository,
            tree: ffi::git_oid,
            who: *mut ffi::git_signature,
        }

        impl Writer {
            fn new(tag: &str) -> Self {
                assert!(runtime_pinned());
                let dir = std::env::temp_dir()
                    .join(format!("crustify-parent-cb-{}-{tag}", std::process::id()));
                drop(std::fs::remove_dir_all(&dir));
                let path = CString::new(dir.to_str().unwrap()).unwrap();
                let (mut repo, mut builder, mut who) =
                    (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
                let mut tree = ffi::git_oid::default();
                {
                    // SAFETY: writable slots, NUL-terminated strings and the live repository;
                    // the builder is freed once, the rest when the writer drops.
                    unsafe {
                        assert_eq!(ffi::git_repository_init(&raw mut repo, path.as_ptr(), 1), 0);
                    };
                    // SAFETY: as above.
                    unsafe {
                        assert_eq!(
                            ffi::git_treebuilder_new(&raw mut builder, repo, ptr::null()),
                            0
                        );
                    };
                    // SAFETY: as above.
                    unsafe { assert_eq!(ffi::git_treebuilder_write(&raw mut tree, builder), 0) };
                    // SAFETY: as above.
                    unsafe { ffi::git_treebuilder_free(builder) };
                    // SAFETY: as above.
                    let rc = unsafe {
                        ffi::git_signature_new(
                            &raw mut who,
                            c"A U Thor".as_ptr(),
                            c"a@example.com".as_ptr(),
                            1_700_000_000,
                            0,
                        )
                    };
                    assert_eq!(rc, 0);
                }
                Self {
                    dir,
                    repo,
                    tree,
                    who,
                }
            }

            /// `git_commit_create_from_callback` with `message` and the parents `cb` and
            /// `payload` give: the new commit's id, or the code and the thread's error.
            fn create(
                &self,
                message: &CStr,
                cb: ffi::git_commit_parent_callback,
                payload: *mut c_void,
            ) -> Result<Oid, (c_int, ErrorBuf)> {
                self.create_updating(None, message, cb, payload)
            }

            /// As [`create`](Self::create), updating `update_ref` to the new commit.
            fn create_updating(
                &self,
                update_ref: Option<&CStr>,
                message: &CStr,
                cb: ffi::git_commit_parent_callback,
                payload: *mut c_void,
            ) -> Result<Oid, (c_int, ErrorBuf)> {
                let mut id = Oid::zeroed();
                // SAFETY: a writable id; the live repository, identity and tree id; a NULL or
                // NUL-terminated reference name; the callback and its payload are live for
                // the call.
                let rc = unsafe {
                    ffi::git_commit_create_from_callback(
                        id.as_mut().as_mut_ptr(),
                        self.repo,
                        update_ref.map_or(ptr::null(), CStr::as_ptr),
                        self.who,
                        self.who,
                        ptr::null(),
                        message.as_ptr(),
                        &raw const self.tree,
                        cb,
                        payload,
                    )
                };
                if rc < 0 { Err((rc, last())) } else { Ok(id) }
            }

            /// Writes a commit whose parents `callback` gives, as the wrapper of
            /// `git_commit_create_from_callback` will.
            fn create_with(
                &self,
                message: &CStr,
                callback: &mut ParentCallback<'_>,
            ) -> Result<Result<Oid, (c_int, ErrorBuf)>, RepositoryError> {
                callback.walk_with(OidType::Sha1, |cb, payload| {
                    self.create(message, cb, payload)
                })
            }

            /// The ids on the `parent` lines of the commit `id`, read back from the object
            /// database.
            fn parents(&self, id: &Oid) -> Vec<String> {
                let (mut odb, mut obj) = (ptr::null_mut(), ptr::null_mut());
                let raw = {
                    // SAFETY: writable slots, the live repository and id; the object's data is
                    // `size` bytes, copied before the object and the odb are freed once.
                    unsafe { assert_eq!(ffi::git_repository_odb(&raw mut odb, self.repo), 0) };
                    // SAFETY: as above.
                    unsafe {
                        assert_eq!(
                            ffi::git_odb_read(&raw mut obj, odb, id.as_ref().as_ptr()),
                            0
                        );
                    };
                    // SAFETY: as above.
                    let odb_object_data = unsafe { ffi::git_odb_object_data(obj) };
                    // SAFETY: as above.
                    let odb_object_size = unsafe { ffi::git_odb_object_size(obj) };
                    // SAFETY: as above.
                    let data = unsafe {
                        core::slice::from_raw_parts(odb_object_data.cast::<u8>(), odb_object_size)
                            .to_vec()
                    };
                    // SAFETY: as above.
                    unsafe { ffi::git_odb_object_free(obj) };
                    // SAFETY: as above.
                    unsafe { ffi::git_odb_free(odb) };
                    String::from_utf8(data).unwrap()
                };
                raw.lines()
                    .filter_map(|l| l.strip_prefix("parent ").map(str::to_owned))
                    .collect()
            }

            /// The loose objects written besides the empty tree: the commits.
            fn commits(&self) -> usize {
                let loose: usize = std::fs::read_dir(self.dir.join("objects"))
                    .unwrap()
                    .map(|e| e.unwrap())
                    .filter(|e| e.file_name().len() == 2)
                    .map(|e| std::fs::read_dir(e.path()).unwrap().count())
                    .sum();
                loose - 1
            }
        }

        impl Drop for Writer {
            fn drop(&mut self) {
                // SAFETY: each is live and freed once, the repository last.
                unsafe { ffi::git_signature_free(self.who) };
                // SAFETY: as above.
                unsafe { ffi::git_repository_free(self.repo) };
                drop(std::fs::remove_dir_all(&self.dir));
            }
        }

        /// A C parent callback over a `Vec<ffi::git_oid>`.
        unsafe extern "C" fn c_parents(idx: usize, payload: *mut c_void) -> *const ffi::git_oid {
            // SAFETY: the payload is the test's live vector, only read.
            let ids = unsafe { &*payload.cast::<Vec<ffi::git_oid>>() };
            ids.get(idx).map_or(ptr::null(), ptr::from_ref)
        }

        /// libgit2 takes each id the trampoline hands it as the next parent, in order, and
        /// writes the commit a C callback over the same ids writes.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn walks_give_libgit2_each_parent() {
            let w = Writer::new("walk");
            let mut none = |_: usize| None;
            let root = w
                .create_with(c"root\n", &mut ParentCallback::new(&mut none))
                .unwrap()
                .unwrap();
            let other = w
                .create_with(c"other\n", &mut ParentCallback::new(&mut none))
                .unwrap()
                .unwrap();
            let ids = [root, other, root];
            let mut asked = Vec::new();
            let mut next = |i: usize| {
                asked.push(i);
                ids.get(i).copied()
            };
            let merge = w
                .create_with(c"merge\n", &mut ParentCallback::new(&mut next))
                .unwrap()
                .unwrap();
            assert_eq!(asked, [0, 1, 2, 3]);
            let hex: Vec<_> = ids
                .iter()
                .map(|id| id.as_ref().to_hex_string().unwrap())
                .collect();
            assert_eq!(w.parents(&merge), hex);

            let mut raw: Vec<ffi::git_oid> = ids
                .iter()
                // SAFETY: each id is live and only read.
                .map(|id| unsafe { id.as_ref().as_ptr().read() })
                .collect();
            let c = w
                .create(c"merge\n", Some(c_parents), ptr::from_mut(&mut raw).cast())
                .unwrap();
            assert_eq!(c, merge);
        }

        /// An id of another type than the repository's never reaches libgit2: the walk ends
        /// at it, before libgit2 runs, so nothing is written; it fails with libgit2's
        /// wording.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn ids_of_another_type_are_refused() {
            let w = Writer::new("refused");
            let mut asked = Vec::new();
            let mut wide = |i: usize| {
                asked.push(i);
                Some(Oid::zero(OidType::Sha256))
            };
            let mut callback = ParentCallback::new(&mut wide);
            let e = w.create_with(c"refused\n", &mut callback).unwrap_err();
            assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
            assert_eq!(
                e.message(),
                c"specified object ID type does not match repository object ID type"
            );
            assert!(format!("{callback:?}").contains("panicked: false"));
            drop(callback);
            assert_eq!(asked, [0]);
            assert_eq!(w.commits(), 0);
        }

        /// A panic in the closure stops the walk the same way: it propagates from the walk,
        /// before libgit2 runs, with nothing written.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn walks_propagate_panics_before_c() {
            let w = Writer::new("panics");
            let mut boom = |_: usize| -> Option<Oid> { panic!("no parent") };
            let mut callback = ParentCallback::new(&mut boom);
            let caught = catch_unwind(AssertUnwindSafe(|| {
                drop(w.create_with(c"panicked\n", &mut callback));
            }));
            assert_eq!(
                caught.unwrap_err().downcast_ref::<&str>(),
                Some(&"no parent")
            );
            assert_eq!(w.commits(), 0);
        }

        /// Without strict object creation libgit2 checks no parent, so a refused id or a
        /// panic must stop the walk before libgit2 runs: nothing is written, and an unborn
        /// branch to update stays unborn. Global option: run alone in a child.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn failed_walks_write_nothing_without_strict_creation() {
            isolated(
                concat!(
                    module_path!(),
                    "::failed_walks_write_nothing_without_strict_creation"
                ),
                || {
                    assert!(runtime_pinned());
                    // SAFETY: an option taking an `int`; this child runs nothing else.
                    let rc = unsafe {
                        ffi::git_libgit2_opts(
                            ffi::GIT_OPT_ENABLE_STRICT_OBJECT_CREATION.cast_signed(),
                            0,
                        )
                    };
                    assert_eq!(rc, 0);
                    let w = Writer::new("lenient");
                    let branch = c"refs/heads/main";
                    let mut asked = Vec::new();
                    let mut wide = |i: usize| {
                        asked.push(i);
                        Some(Oid::zero(OidType::Sha256))
                    };
                    let mut written = None;
                    let e = ParentCallback::new(&mut wide)
                        .walk_with(OidType::Sha1, |cb, payload| {
                            written =
                                Some(w.create_updating(Some(branch), c"lenient\n", cb, payload));
                        })
                        .unwrap_err();
                    assert_eq!(e.class(), Some(ErrorClass::Invalid));
                    assert_eq!(asked, [0]);
                    assert!(written.is_none(), "libgit2 ran: {written:?}");

                    let mut boom = |i: usize| -> Option<Oid> {
                        assert_eq!(i, 0);
                        panic!("no parent")
                    };
                    let caught = catch_unwind(AssertUnwindSafe(|| {
                        drop(ParentCallback::new(&mut boom).walk_with(
                            OidType::Sha1,
                            |cb, payload| {
                                w.create_updating(Some(branch), c"panicked\n", cb, payload)
                            },
                        ));
                    }));
                    assert_eq!(
                        caught.unwrap_err().downcast_ref::<&str>(),
                        Some(&"no parent")
                    );
                    assert_eq!(w.commits(), 0);
                    assert!(!w.dir.join("refs/heads/main").exists());
                },
            );
        }

        /// With a reference to update, libgit2 checks the first parent the walk gives
        /// against the reference's tip: a commit on another tip fails with `GIT_EMODIFIED`
        /// and writes nothing; one on the tip moves the reference.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn walks_updating_a_reference_check_its_tip() {
            let w = Writer::new("update-ref");
            let branch = c"refs/heads/main";
            let mut none = |_: usize| None;
            let root = callback_walk(&w, Some(branch), c"root\n", &mut none).unwrap();
            let mut none = |_: usize| None;
            let other = callback_walk(&w, None, c"other\n", &mut none).unwrap();
            let mut off_tip = |i: usize| (i == 0).then_some(other);
            let (rc, e) = callback_walk(&w, Some(branch), c"off\n", &mut off_tip).unwrap_err();
            assert_eq!(rc, ErrorCode::Modified.raw());
            assert_eq!(
                e.message(),
                c"failed to create commit: current tip is not the first parent"
            );
            assert_eq!(w.commits(), 2);
            let mut on_tip = |i: usize| (i == 0).then_some(root);
            let child = callback_walk(&w, Some(branch), c"child\n", &mut on_tip).unwrap();
            let tip = std::fs::read_to_string(w.dir.join("refs/heads/main")).unwrap();
            assert_eq!(tip.trim(), child.as_ref().to_hex_string().unwrap());
            assert_eq!(w.parents(&child), [root.as_ref().to_hex_string().unwrap()]);
        }

        /// A walk of `f`'s parents writing `message`, updating `update_ref`.
        fn callback_walk(
            w: &Writer,
            update_ref: Option<&CStr>,
            message: &CStr,
            f: &mut dyn FnMut(usize) -> Option<Oid>,
        ) -> Result<Oid, (c_int, ErrorBuf)> {
            let mut f = f;
            ParentCallback::new(&mut f)
                .walk_with(OidType::Sha1, |cb, payload| {
                    w.create_updating(update_ref, message, cb, payload)
                })
                .unwrap()
        }

        // --- Creating commits by id ------------------------------------------------------

        use crate::libgit2::src::libgit2::commit::{Commit, create};
        use crate::libgit2::src::libgit2::repository::{Repository, RepositoryBox};
        use crate::libgit2::src::libgit2::signature::{SignatureView, Time};
        use crate::libgit2::src::libgit2::tree::Tree;
        use crate::libgit2::src::util::errors::ErrorClass;

        /// The empty tree's id, which libgit2 knows without storing it.
        const EMPTY_TREE: &CStr = c"4b825dc642cb6eb9a060e54bf8d69288fbee4904";

        /// A fresh bare repository, through the safe wrappers, removed when dropped.
        struct Scratch {
            dir: std::path::PathBuf,
            repo: Option<RepositoryBox>,
        }

        impl Scratch {
            fn new(tag: &str) -> Self {
                assert!(runtime_pinned());
                let dir = std::env::temp_dir().join(format!(
                    "crustify-commit-by-id-{}-{tag}",
                    std::process::id()
                ));
                drop(std::fs::remove_dir_all(&dir));
                let path = CString::new(dir.to_str().unwrap()).unwrap();
                let repo = Repository::init(&path, true).unwrap();
                Self {
                    dir,
                    repo: Some(repo),
                }
            }

            fn repo(&self) -> RepositoryRef<'_> {
                self.repo.as_ref().unwrap().as_ref()
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                drop(self.repo.take());
                drop(std::fs::remove_dir_all(&self.dir));
            }
        }

        fn empty_tree() -> Oid {
            Oid::from_string(EMPTY_TREE, OidType::Sha1).unwrap()
        }

        /// An id of another type than the repository's, the tree's or a parent's, is
        /// refused before libgit2 reads it, and so is a signature libgit2 cannot write: the
        /// closure is not asked for parents then, and nothing is written.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn ids_of_another_type_are_refused_before_c() {
            let s = Scratch::new("refused");
            let repo = s.repo();
            let me = SignatureView::new(c"A", c"a@example.com", Time::new(0, 0));
            let me = me.as_ref();
            let (tree, wide) = (empty_tree(), Oid::zero(OidType::Sha256));
            let mismatch = c"specified object ID type does not match repository object ID type";
            let check = |r: Result<Oid, RepositoryError>, message: &CStr| {
                let e = r.unwrap_err();
                assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
                assert_eq!(e.message(), message);
            };
            let ids = |tree: &Oid, parents: &[OidRef<'_>]| {
                create_from_ids(repo, None, me, me, None, c"m\n", tree.as_ref(), parents)
            };
            check(ids(&wide, &[]), mismatch);
            check(ids(&tree, &[tree.as_ref(), wide.as_ref()]), mismatch);

            let mut asked = 0;
            let mut next = |_: usize| {
                asked += 1;
                None
            };
            let mut callback = ParentCallback::new(&mut next);
            let r = create_from_callback(
                repo,
                None,
                me,
                me,
                None,
                c"m\n",
                wide.as_ref(),
                &mut callback,
            );
            check(r, mismatch);
            let min = SignatureView::new(c"A", c"a@example.com", Time::new(0, i32::MIN));
            let r = create_from_callback(
                repo,
                None,
                min.as_ref(),
                me,
                None,
                c"m\n",
                tree.as_ref(),
                &mut callback,
            );
            check(
                r,
                c"failed to create commit: signature timezone offset is out of range",
            );
            drop(callback);
            assert_eq!(asked, 0);
            let mut wide_parent = |i: usize| (i == 0).then(|| Oid::zero(OidType::Sha256));
            let mut callback = ParentCallback::new(&mut wide_parent);
            let r = create_from_callback(
                repo,
                None,
                me,
                me,
                None,
                c"m\n",
                tree.as_ref(),
                &mut callback,
            );
            check(r, mismatch);
            assert!(
                std::fs::read_dir(s.dir.join("objects"))
                    .unwrap()
                    .all(|e| { e.unwrap().file_name().len() != 2 })
            );
        }

        /// The id-taking forms write the commit `create` writes for the same tree and
        /// parents; strict object creation refuses a parent the object database lacks.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn id_forms_write_what_create_writes() {
            let s = Scratch::new("forms");
            let repo = s.repo();
            let me =
                SignatureView::new(c"A U Thor", c"a@example.com", Time::new(1_700_000_000, 60));
            let me = me.as_ref();
            let tree_id = empty_tree();
            let tree = Tree::lookup(repo, tree_id.as_ref()).unwrap();
            let root = create(repo, None, me, me, None, c"root\n", tree.as_ref(), &[]).unwrap();
            let root_commit = Commit::lookup(repo, root.as_ref()).unwrap();
            let expected = create(
                repo,
                None,
                me,
                me,
                Some(c"UTF-8"),
                c"child\n",
                tree.as_ref(),
                &[root_commit.as_ref(), root_commit.as_ref()],
            )
            .unwrap();
            let by_ids = create_from_ids(
                repo,
                None,
                me,
                me,
                Some(c"UTF-8"),
                c"child\n",
                tree_id.as_ref(),
                &[root.as_ref(), root.as_ref()],
            )
            .unwrap();
            assert_eq!(by_ids, expected);
            let mut next = |i: usize| (i < 2).then_some(root);
            let mut callback = ParentCallback::new(&mut next);
            let by_callback = create_from_callback(
                repo,
                None,
                me,
                me,
                Some(c"UTF-8"),
                c"child\n",
                tree_id.as_ref(),
                &mut callback,
            )
            .unwrap();
            assert_eq!(by_callback, expected);

            let missing =
                Oid::from_string(c"dededededededededededededededededededede", OidType::Sha1)
                    .unwrap();
            let e = create_from_ids(
                repo,
                None,
                me,
                me,
                None,
                c"m\n",
                tree_id.as_ref(),
                &[missing.as_ref()],
            )
            .unwrap_err();
            assert_eq!(e.raw_code(), -1);
            let e = create_from_ids(repo, None, me, me, None, c"m\n", missing.as_ref(), &[])
                .unwrap_err();
            assert_eq!(e.raw_code(), -1);
        }
    }
}
