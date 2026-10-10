//! Safe wrappers over `include/git2/annotated_commit.h`.
//!
//! An [`AnnotatedCommit`] is a `git_annotated_commit`: a commit together with how the
//! caller looked it up (a reference name, a revspec, a fetch head's branch and remote),
//! which merge and rebase record in their messages and files. The public header only
//! forward-declares the struct, so Rust never reads its fields: it is reached through
//! libgit2's routines alone.
//!
//! libgit2 allocates one in each of its four public constructors and frees it in
//! `git_annotated_commit_free`; there is no copy routine. The caller owns the result
//! alone, so Rust holds it as an [`AnnotatedCommitBox`], which frees it on drop.
//!
//! The struct holds a counted reference to its commit, shared with the repository's
//! object cache, and copies of the strings it was looked up by. Freeing it releases
//! that reference with an atomic decrement and frees the strings, without touching
//! the repository. But the commit keeps an unowned pointer back to the repository it
//! was looked up in, which the routines that consume annotated commits (merge,
//! rebase, reset, branch creation) follow, so an annotated commit carries the lifetime
//! `'r` of that repository's borrow, a [`RepositoryRef`], and lives no longer than it.
//!
//! An object id names an object only in a repository of its type: libgit2 formats the
//! id into a loose object's path sized for the repository's type, so a SHA256 id in a
//! SHA1 repository overflows that buffer, and a SHA1 id in a SHA256 repository, or an
//! id of a type libgit2 does not know, leaves part or all of the path unwritten. The
//! constructors taking an id refuse one whose type is not the repository's before the
//! call, as [`AnnotatedCommit::from_ref`] refuses a reference of a repository of
//! another type (whose peeled id is of that type), with libgit2's `GIT_ERROR_INVALID`
//! "specified object ID type does not match repository object ID type".
//!
//! The constructors can fail and set the calling thread's libgit2 error, so they pin
//! libgit2's runtime first (`runtime_pinned`) and return the error, copied out, as an
//! [`ErrorBuf`]. libgit2's return code (`GIT_ENOTFOUND`, `GIT_EAMBIGUOUS`, ...) is not
//! kept: [`ErrorBuf`] carries the class and the message.

use core::ffi::{CStr, c_int};
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use ffibox::{CBorrowedPtr, CBox, CCell, CDrop};

use crate::ffi;
use crate::libgit2::oid::oid::OidRef;
use crate::libgit2::refs::refs::ReferenceRef;
use crate::libgit2::repository::repository::RepositoryRef;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, last};
use crate::libgit2::util::sys::errors::{runtime_pinned, set};

/// A commit and how it was looked up (`git_annotated_commit`): the input merge and
/// rebase take to record the caller's intent ("merge `refs/heads/feature`", not just
/// its commit id).
///
/// Only ever behind a pointer, owned as an [`AnnotatedCommitBox`], which the
/// constructors ([`lookup`](Self::lookup), [`from_ref`](Self::from_ref),
/// [`from_revspec`](Self::from_revspec), [`from_fetchhead`](Self::from_fetchhead))
/// build. libgit2 publishes no fields: the handles carry no field accessors.
///
/// `'r` is the borrow of the repository the commit was looked up in, which its
/// consumers reach through it; the owner does not outlive it:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::refs::annotated_commit::AnnotatedCommit;
/// use libgit2::libgit2::repository::repository::Repository;
///
/// let repo = Repository::open(c".").unwrap();
/// let commit = AnnotatedCommit::from_revspec(repo.as_ref(), c"HEAD").unwrap();
/// drop(repo);
/// drop(commit);
/// ```
///
/// and a handle does not outlive its owner:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::refs::annotated_commit::AnnotatedCommitBox;
///
/// fn escape(commit: AnnotatedCommitBox<'_>) {
///     let handle = commit.as_ref();
///     drop(commit);
///     let _ = handle;
/// }
/// ```
///
/// The owner is `Send` but not `Sync`: libgit2's merge routines take
/// `const git_annotated_commit **` heads and still cache each head's tree in it, so no
/// shared handle may cross threads.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::refs::annotated_commit::AnnotatedCommitBox;
///
/// fn shared(commit: &AnnotatedCommitBox<'_>) {
///     std::thread::scope(|s| {
///         s.spawn(|| commit.as_ref());
///     });
/// }
/// ```
///
/// There is no copy routine, so the owner is not `Clone`:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::refs::annotated_commit::AnnotatedCommitBox;
///
/// fn copyable<T: Clone>() {}
/// copyable::<AnnotatedCommitBox<'static>>();
/// ```
///
/// Wraps: git_annotated_commit
/// Handwritten
#[repr(transparent)]
pub struct AnnotatedCommit<'r>(ffi::git_annotated_commit, PhantomData<(*const (), &'r ())>);

/// Shared borrow of an [`AnnotatedCommit`]. `Copy`, like `&T`.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct AnnotatedCommitRef<'a, 'r>(CBorrowedPtr<'a, AnnotatedCommit<'r>>);

/// Exclusive borrow of an [`AnnotatedCommit`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// Handwritten
#[repr(transparent)]
pub struct AnnotatedCommitMut<'a, 'r>(
    AnnotatedCommitRef<'a, 'r>,
    // Invariant in the commit, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut AnnotatedCommit<'r>>,
);

// SAFETY: `AnnotatedCommit` is `#[repr(transparent)]` over `git_annotated_commit` with a
// zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, AnnotatedCommit<'r>>` (`AnnotatedCommitMut` through
// `AnnotatedCommitRef`) with zero-sized markers and no `Drop`, hand out no reference to
// the C object, and `AnnotatedCommitRef` has no write operation. `AnnotatedCommitMut` is
// invariant in the commit through its `PhantomData<&'a mut AnnotatedCommit<'r>>`.
unsafe impl<'r> CCell for AnnotatedCommit<'r> {
    type C = ffi::git_annotated_commit;
    type Ref<'a>
        = AnnotatedCommitRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = AnnotatedCommitMut<'a, 'r>
    where
        Self: 'a;
}

// SAFETY: an owned annotated commit is its strings (libgit2's allocator) and a counted
// reference to a commit (and the tree merge may cache), whose count libgit2 changes
// atomically and whose other holders (the repository's object cache) never write the
// rest. Freeing it is `git__free` and that decrement, on any thread, never reaching the
// repository; no libgit2 routine keeps thread-local state about it.
unsafe impl Send for AnnotatedCommit<'_> {}
// SAFETY: `AnnotatedCommitMut` is an exclusive borrow of such an object, as
// `&mut AnnotatedCommit`.
unsafe impl Send for AnnotatedCommitMut<'_, '_> {}

impl<'a, 'r> AnnotatedCommitRef<'a, 'r> {
    /// Borrows a raw pointer; `None` if null.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live annotated commit, which no one frees or
    /// writes during `'a`, looked up in a repository that outlives `'r`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_annotated_commit) -> Option<Self> {
        NonNull::new(ptr.cast_mut()).map(|p| {
            // SAFETY: a non-null pointer the caller vouches for, per this function's
            // contract; `AnnotatedCommit` is transparent over `git_annotated_commit`.
            AnnotatedCommitRef(unsafe { CBorrowedPtr::new(p.cast()) })
        })
    }

    /// Read-only pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_annotated_commit {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'a, 'r> AnnotatedCommitMut<'a, 'r> {
    /// Borrows a raw pointer exclusively; `None` if null.
    ///
    /// # Safety
    ///
    /// As [`AnnotatedCommitRef::from_ptr`], plus: nothing else reaches the commit while
    /// the result lives.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_annotated_commit) -> Option<Self> {
        // SAFETY: the caller upholds `AnnotatedCommitRef::from_ptr`'s contract and
        // exclusivity.
        unsafe { AnnotatedCommitRef::from_ptr(ptr) }.map(|r| AnnotatedCommitMut(r, PhantomData))
    }

    /// Writable pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_annotated_commit {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> AnnotatedCommitRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> AnnotatedCommitMut<'_, 'r> {
        AnnotatedCommitMut(self.0, PhantomData)
    }
}

impl fmt::Debug for AnnotatedCommitRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnnotatedCommit").finish_non_exhaustive()
    }
}

impl fmt::Debug for AnnotatedCommitMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases an annotated commit through `git_annotated_commit_free`, which drops its
/// commit reference (and the tree merge may have cached), hands its strings to
/// `git__free`, then frees the struct. libgit2 has no copy routine, so the policy
/// does not clone.
///
/// Wraps: git_annotated_commit_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct AnnotatedCommitFree;

// SAFETY: `git_annotated_commit_free` releases a live annotated commit exactly once,
// whatever constructor built it (every public one builds a "real" commit, whose
// branch frees the commit reference, the tree and the three strings, each NULL or
// owned). It needs no state beyond the pointer, and no repository.
unsafe impl<'r> CDrop<AnnotatedCommit<'r>> for AnnotatedCommitFree {
    unsafe fn c_drop(&self, ptr: NonNull<AnnotatedCommit<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live annotated commit whose
        // ownership the caller gives up; `AnnotatedCommit` is transparent over
        // `git_annotated_commit`.
        unsafe { ffi::git_annotated_commit_free(ptr.as_ptr().cast()) }
    }
}

/// An owned [`AnnotatedCommit`], freed by `git_annotated_commit_free` on drop.
pub type AnnotatedCommitBox<'r> = CBox<AnnotatedCommit<'r>, AnnotatedCommitFree>;

/// The annotated commit a constructor returning `rc` left in `out` (adopted as
/// `commit`), or libgit2's error.
///
/// `git_annotated_commit_from_ref` and `git_annotated_commit_from_fetchhead` copy the
/// reference name (and the remote URL) after building the commit, and return `-1`
/// when that copy runs out of memory with the commit still in `out`: it is freed here,
/// after the error is copied out.
fn constructed<'r>(
    rc: c_int,
    commit: Option<AnnotatedCommitBox<'r>>,
) -> Result<AnnotatedCommitBox<'r>, ErrorBuf> {
    if rc < 0 {
        let error = last();
        drop(commit);
        return Err(error);
    }
    commit.ok_or_else(last)
}

/// Whether `id` is of `repo`'s object id type; sets libgit2's error when it is not.
fn id_fits(repo: RepositoryRef<'_>, id: OidRef<'_>) -> bool {
    type_fits(repo, u32::from(id.raw_kind()))
}

/// Whether the object id type `kind` is `repo`'s; sets libgit2's error when it is not.
fn type_fits(repo: RepositoryRef<'_>, kind: ffi::git_oid_t) -> bool {
    // SAFETY: `repo` is a live repository on this thread, whose `oid_type` libgit2 only
    // reads.
    if unsafe { ffi::git_repository_oid_type(repo.as_ptr().cast_mut()) } == kind {
        return true;
    }
    set(
        ErrorClass::Invalid,
        c"specified object ID type does not match repository object ID type",
    );
    false
}

impl<'r> AnnotatedCommit<'r> {
    /// The commit `id` names in `repo`, annotated with nothing but its id: its
    /// description is the id's hex form, and it carries no reference name.
    ///
    /// Fails, with libgit2's error, when `repo` holds no object `id` names, or one that
    /// is not a commit. An id whose type is not the repository's is refused before the
    /// call (see the [module docs](self)).
    ///
    /// Wraps: git_annotated_commit_lookup
    pub fn lookup(
        repo: RepositoryRef<'r>,
        id: OidRef<'_>,
    ) -> Result<AnnotatedCommitBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if !id_fits(repo, id) {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `repo` a live repository on this thread,
        // whose object cache libgit2 may fill through it, as `RepositoryRef` allows;
        // `id` a live `git_oid` of the repository's type, only read. The runtime is
        // pinned, so a failure is reported on live thread-local state.
        let rc = unsafe {
            ffi::git_annotated_commit_lookup(&mut out, repo.as_ptr().cast_mut(), id.as_ptr())
        };
        // SAFETY: `git_annotated_commit_lookup` leaves `out` NULL on failure, or writes a
        // fresh annotated commit of `repo` the caller owns alone, released by
        // `git_annotated_commit_free` as `AnnotatedCommitFree` does.
        constructed(rc, unsafe { AnnotatedCommitBox::from_c(out) })
    }

    /// The commit `reference` peels to, looked up in `repo`, annotated with the
    /// reference's name: both its description and its reference name (the input merge
    /// writes as "branch 'feature'").
    ///
    /// A symbolic reference is resolved and an annotated tag peeled, but the name kept
    /// is `reference`'s own (`HEAD`, `refs/tags/v1`). `reference` is peeled in its own
    /// repository, which its handle keeps alive, and the commit then looked up in
    /// `repo`, usually the same one. Fails, with libgit2's error, when it does not peel
    /// to a commit (a tree or a blob), or when the commit it peels to is not in `repo`.
    /// A reference of a repository whose object id type is not `repo`'s is refused
    /// before the call (see the [module docs](self)): libgit2 would look its peeled id
    /// up in `repo` as it does an id of that type.
    ///
    /// When libgit2 runs out of memory copying the reference name, it reports the
    /// failure with the commit already built, which is freed here, but returns before
    /// releasing the object `reference` peeled to, whose reference leaks; when it runs
    /// out of memory copying the description, it leaks the half-built commit before
    /// reporting the error.
    ///
    /// Wraps: git_annotated_commit_from_ref
    pub fn from_ref(
        repo: RepositoryRef<'r>,
        reference: ReferenceRef<'_, '_>,
    ) -> Result<AnnotatedCommitBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let kind =
            // SAFETY: `reference` is a live attached reference, whose database and
            // repository its borrow keeps alive on this thread; libgit2 reads the
            // database's repository pointer, then that repository's `oid_type`.
            unsafe { ffi::git_repository_oid_type(ffi::git_reference_owner(reference.as_ptr())) };
        if !type_fits(repo, kind) {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `repo` a live repository on this thread, as
        // in `lookup`; `reference` a live attached reference of a repository of `repo`'s
        // object id type whose repository its borrow
        // keeps alive, on this thread (it is not `Send`), only read (its name is
        // copied). The runtime is pinned.
        let rc = unsafe {
            ffi::git_annotated_commit_from_ref(
                &mut out,
                repo.as_ptr().cast_mut(),
                reference.as_ptr(),
            )
        };
        // SAFETY: `git_annotated_commit_from_ref` leaves `out` NULL, or a fresh annotated
        // commit of `repo` the caller owns alone, whatever it returns, released by
        // `git_annotated_commit_free` as `AnnotatedCommitFree` does.
        constructed(rc, unsafe { AnnotatedCommitBox::from_c(out) })
    }

    /// The commit `id` names in `repo`, annotated as a fetch head: `branch_name` (the
    /// remote branch, as `FETCH_HEAD` names it) is its description and reference name,
    /// and `remote_url` the remote it came from.
    ///
    /// Fails, with libgit2's error, as [`lookup`](Self::lookup) does, and refuses an id
    /// as it does; neither string is checked.
    ///
    /// When libgit2 runs out of memory copying either string, it reports the failure
    /// with the commit already built, which is freed here; when it runs out of memory
    /// copying the description, it leaks the half-built commit before reporting the
    /// error.
    ///
    /// Wraps: git_annotated_commit_from_fetchhead
    pub fn from_fetchhead(
        repo: RepositoryRef<'r>,
        branch_name: &CStr,
        remote_url: &CStr,
        id: OidRef<'_>,
    ) -> Result<AnnotatedCommitBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if !id_fits(repo, id) {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `repo` a live repository on this thread, as
        // in `lookup`; both strings are NUL-terminated and `id` a live `git_oid` of the
        // repository's type, all only read (the strings are copied). The runtime is
        // pinned.
        let rc = unsafe {
            ffi::git_annotated_commit_from_fetchhead(
                &mut out,
                repo.as_ptr().cast_mut(),
                branch_name.as_ptr(),
                remote_url.as_ptr(),
                id.as_ptr(),
            )
        };
        // SAFETY: `git_annotated_commit_from_fetchhead` leaves `out` NULL, or a fresh
        // annotated commit of `repo` the caller owns alone, whatever it returns,
        // released by `git_annotated_commit_free` as `AnnotatedCommitFree` does.
        constructed(rc, unsafe { AnnotatedCommitBox::from_c(out) })
    }

    /// The commit the revision `revspec` names in `repo` (see `man gitrevisions`),
    /// peeled to a commit and annotated with `revspec` as its description; it carries
    /// no reference name.
    ///
    /// Fails, with libgit2's error, when `revspec` does not parse, names nothing, or
    /// names an object that does not peel to a commit.
    ///
    /// Wraps: git_annotated_commit_from_revspec
    pub fn from_revspec(
        repo: RepositoryRef<'r>,
        revspec: &CStr,
    ) -> Result<AnnotatedCommitBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `repo` a live repository on this thread, as
        // in `lookup`; `revspec` is NUL-terminated and only read (it is copied as the
        // description). The runtime is pinned.
        let rc = unsafe {
            ffi::git_annotated_commit_from_revspec(
                &mut out,
                repo.as_ptr().cast_mut(),
                revspec.as_ptr(),
            )
        };
        // SAFETY: `out` starts NULL, which `git_annotated_commit_from_revspec` leaves
        // when it fails, or holds a fresh annotated commit of `repo` the caller owns
        // alone, released by `git_annotated_commit_free` as `AnnotatedCommitFree` does.
        constructed(rc, unsafe { AnnotatedCommitBox::from_c(out) })
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::oid::oid::{Oid, OidType};
    use crate::libgit2::refs::refs::ReferenceBox;
    use crate::libgit2::repository::repository::RepositoryBox;
    use core::mem::size_of;
    use ffibox::CZeroable;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    #[test]
    fn layout_is_one_pointer() {
        assert_eq!(
            size_of::<AnnotatedCommit<'_>>(),
            size_of::<ffi::git_annotated_commit>()
        );
        assert_eq!(
            size_of::<AnnotatedCommitBox<'_>>(),
            size_of::<*mut ffi::git_annotated_commit>()
        );
        assert_eq!(
            size_of::<Option<AnnotatedCommitBox<'_>>>(),
            size_of::<*mut ffi::git_annotated_commit>()
        );
        assert_eq!(
            size_of::<AnnotatedCommitRef<'_, '_>>(),
            size_of::<*const ffi::git_annotated_commit>()
        );
        assert_eq!(
            size_of::<AnnotatedCommitMut<'_, '_>>(),
            size_of::<*mut ffi::git_annotated_commit>()
        );
    }

    /// The handles' raw seams round-trip a pointer, reborrow, and format opaquely;
    /// nothing is dereferenced, so Miri runs it.
    #[test]
    fn handles_round_trip_their_pointer() {
        let mut raw = core::mem::MaybeUninit::<[usize; 4]>::uninit();
        let p: *mut ffi::git_annotated_commit = raw.as_mut_ptr().cast();
        // SAFETY: a pointer into this test's live local storage, which no one else
        // reaches; the handles only carry it and nothing here dereferences it (the
        // opaque `git_annotated_commit` is zero-sized to Rust).
        let mut m = unsafe { AnnotatedCommitMut::from_ptr(p) }.unwrap();
        assert_eq!(m.as_mut_ptr(), p);
        assert_eq!(m.as_mut().as_ref().as_ptr(), p.cast_const());
        assert_eq!(format!("{:?}", m), "AnnotatedCommit { .. }");
        // SAFETY: NULL is refused before anything else.
        assert!(unsafe { AnnotatedCommitRef::from_ptr(ptr::null()) }.is_none());
    }

    /// A fresh bare repository of object id type `kind` holding one commit; removed on
    /// drop, after the repository it holds.
    struct Repo {
        path: PathBuf,
        repo: Option<RepositoryBox>,
        commit: ffi::git_oid,
    }

    impl Repo {
        fn new(tag: &str, kind: OidType) -> Self {
            assert!(runtime_pinned());
            let path = std::env::temp_dir().join(format!(
                "crustify-annotated-commit-unit-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let mut opts = ffi::git_repository_init_options::default();
            // SAFETY: `opts` is a live options struct, filled with the version's
            // defaults.
            let rc = unsafe {
                ffi::git_repository_init_options_init(
                    &mut opts,
                    ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
                )
            };
            assert_eq!(rc, 0);
            opts.flags = ffi::GIT_REPOSITORY_INIT_BARE | ffi::GIT_REPOSITORY_INIT_MKDIR;
            opts.oid_type = kind.into();
            let mut raw = ptr::null_mut();
            // SAFETY: a live slot, a NUL-terminated path and live options, all read for
            // the call.
            let rc = unsafe { ffi::git_repository_init_ext(&mut raw, c_path.as_ptr(), &mut opts) };
            assert_eq!(rc, 0);
            // SAFETY: a fresh repository this fixture owns alone, released by
            // `git_repository_free` as `RepositoryFree` does.
            let repo = unsafe { RepositoryBox::from_c(raw) }.unwrap();
            let mut odb = ptr::null_mut();
            // SAFETY: a live slot and the live repository; released below.
            assert_eq!(unsafe { ffi::git_repository_odb(&mut odb, raw) }, 0);
            let mut tree = ffi::git_oid::default();
            // SAFETY: a live slot and the live odb; no data, at a non-NULL address
            // (libgit2 hands it to `memcpy`).
            let rc = unsafe {
                ffi::git_odb_write(&mut tree, odb, b"".as_ptr().cast(), 0, ffi::GIT_OBJECT_TREE)
            };
            assert_eq!(rc, 0);
            let mut hex = [0u8; 65];
            // SAFETY: `hex` holds 65 bytes, which bounds the write; `tree` is live.
            unsafe { ffi::git_oid_tostr(hex.as_mut_ptr().cast(), hex.len(), &tree) };
            let hex = CStr::from_bytes_until_nul(&hex).unwrap().to_str().unwrap();
            let data =
                format!("tree {hex}\nauthor A <a@b> 0 +0000\ncommitter A <a@b> 0 +0000\n\nm\n");
            let mut commit = ffi::git_oid::default();
            // SAFETY: a live slot, the live odb and `data` readable for its length.
            let rc = unsafe {
                ffi::git_odb_write(
                    &mut commit,
                    odb,
                    data.as_ptr().cast(),
                    data.len(),
                    ffi::GIT_OBJECT_COMMIT,
                )
            };
            assert_eq!(rc, 0);
            // SAFETY: the odb reference taken above, released once.
            unsafe { ffi::git_odb_free(odb) };
            Self {
                path,
                repo: Some(repo),
                commit,
            }
        }

        fn repo(&self) -> RepositoryRef<'_> {
            self.repo.as_ref().unwrap().as_ref()
        }

        /// The commit's id under the type byte `kind`.
        fn id(&self, kind: u8) -> Oid {
            let mut id = Oid::zeroed();
            id.as_mut().set_raw_kind(kind);
            id.as_mut().set_id(self.commit.id);
            id
        }

        /// `refs/heads/main`, pointed at the commit and adopted.
        fn main(&self) -> ReferenceBox<'_> {
            let mut hex = [0u8; 65];
            // SAFETY: `hex` holds 65 bytes, which bounds the write; the id is live.
            unsafe { ffi::git_oid_tostr(hex.as_mut_ptr().cast(), hex.len(), &self.commit) };
            let hex = CStr::from_bytes_until_nul(&hex).unwrap().to_str().unwrap();
            let file = self.path.join("refs/heads/main");
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, format!("{hex}\n")).unwrap();
            let mut out = ptr::null_mut();
            let raw = self.repo().as_ptr().cast_mut();
            let rc =
                // SAFETY: a live slot, the live repository and a NUL-terminated name.
                unsafe { ffi::git_reference_lookup(&mut out, raw, c"refs/heads/main".as_ptr()) };
            assert_eq!(rc, 0);
            // SAFETY: a fresh attached reference the caller owns, released by
            // `git_reference_free`; it borrows `self`, which frees the repository last.
            unsafe { ReferenceBox::from_c(out) }.unwrap()
        }

        /// The commit, built by libgit2 and adopted.
        fn raw_commit(&self) -> Option<AnnotatedCommitBox<'_>> {
            let mut out = ptr::null_mut();
            // SAFETY: a live slot, the live repository used by this thread alone and a
            // live id of its type.
            let rc = unsafe {
                ffi::git_annotated_commit_lookup(
                    &mut out,
                    self.repo().as_ptr().cast_mut(),
                    &self.commit,
                )
            };
            assert_eq!(rc, 0);
            // SAFETY: a fresh annotated commit of the repository, which this test owns
            // alone, released by `git_annotated_commit_free` as `AnnotatedCommitFree` does.
            unsafe { AnnotatedCommitBox::from_c(out) }
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// `constructed` adopts what C built on success, frees what it left behind a
    /// failure (LSan checks), and reports the thread's error either way.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructed_adopts_or_frees_what_c_left() {
        let repo = Repo::new("constructed", OidType::Sha1);
        let built = constructed(0, repo.raw_commit()).unwrap();
        assert!(!built.as_c_ptr().is_null());
        set(ErrorClass::NoMemory, c"probe: left behind");
        let e = constructed(-1, repo.raw_commit()).unwrap_err();
        assert_eq!(
            (e.class(), e.message()),
            (Some(ErrorClass::NoMemory), c"probe: left behind")
        );
        set(ErrorClass::Odb, c"probe: nothing");
        let e = constructed(-3, None).unwrap_err();
        assert_eq!(e.message(), c"probe: nothing");
        // Success without a result is reported as the thread's error.
        let e = constructed(0, None).unwrap_err();
        assert_eq!(e.message(), c"probe: nothing");
        drop(built);
    }

    /// Deliberate correction: a reference of a repository of another object id type
    /// never reaches libgit2, which would look its peeled id up in `repo` as an id of
    /// that type (a heap overflow for a SHA256 reference and a SHA1 repository).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn references_of_another_id_type_are_refused_before_c() {
        const REFUSED: &CStr = c"specified object ID type does not match repository object ID type";
        let narrow = Repo::new("ref-sha1", OidType::Sha1);
        let wide = Repo::new("ref-sha256", OidType::Sha256);
        for (into, from) in [(&narrow, &wide), (&wide, &narrow), (&narrow, &narrow)] {
            let reference = from.main();
            let result = AnnotatedCommit::from_ref(into.repo(), reference.as_ref());
            if ptr::eq(into, from) {
                assert!(result.is_ok());
            } else {
                let e = result.unwrap_err();
                assert_eq!(
                    (e.class(), e.message()),
                    (Some(ErrorClass::Invalid), REFUSED)
                );
            }
        }
    }

    /// Deliberate correction: an id of another type than the repository's never
    /// reaches libgit2, which would format it into an object path sized for the
    /// repository's type (overflowing it for a SHA256 id in a SHA1 repository).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn ids_of_another_type_are_refused_before_c() {
        const REFUSED: &CStr = c"specified object ID type does not match repository object ID type";
        for (tag, own) in [("sha1", OidType::Sha1), ("sha256", OidType::Sha256)] {
            let repo = Repo::new(&format!("refused-{tag}"), own);
            assert_eq!(repo.commit.type_, own.raw() as u8);
            for kind in 0..=u8::MAX {
                let id = repo.id(kind);
                set(ErrorClass::Odb, c"probe");
                let fits = id_fits(repo.repo(), id.as_ref());
                assert_eq!(fits, kind == own.raw() as u8, "{tag} {kind}");
                let message = last();
                assert_eq!(message.message(), if fits { c"probe" } else { REFUSED });
                let looked_up = AnnotatedCommit::lookup(repo.repo(), id.as_ref());
                let fetched = AnnotatedCommit::from_fetchhead(repo.repo(), c"b", c"u", id.as_ref());
                for result in [looked_up, fetched] {
                    match result {
                        Ok(_) => assert!(fits),
                        Err(e) => {
                            assert!(!fits);
                            assert_eq!(
                                (e.class(), e.message()),
                                (Some(ErrorClass::Invalid), REFUSED)
                            );
                        }
                    }
                }
            }
        }
    }
}
