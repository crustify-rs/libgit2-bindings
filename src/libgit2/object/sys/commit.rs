//! Safe wrappers over `include/git2/sys/commit.h`.
//!
//! [`create_from_ids`] and [`create_from_callback`] write a commit as
//! [`commit::create`](crate::libgit2::object::commit::create) does, of a tree and
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
//! [`object` module](crate::libgit2::object::object#looking-objects-up)). So the walk
//! a commit routine's wrapper runs asks the closure for every parent first, until it
//! returns `None`, refusing an id of another type than the repository's: only once the
//! closure gave them all does libgit2 run, through a trampoline handing out those ids,
//! which can neither fail nor panic. A refusal or a panic of the closure ends the walk
//! before libgit2 writes anything, whatever its options.

use core::any::Any;
use core::ffi::{CStr, c_void};
use core::{fmt, ptr};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use ffibox::CZeroable;

use crate::ffi;
use crate::libgit2::core::types::SignatureRef;
use crate::libgit2::object::commit::writable;
use crate::libgit2::object::object::id_fits;
use crate::libgit2::oid::oid::{Oid, OidRef, OidType};
use crate::libgit2::repository::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::util::errors::{ErrorClass, ErrorCode};
use crate::libgit2::util::sys::errors::set;

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
/// use libgit2::libgit2::object::sys::commit::ParentCallback;
/// use libgit2::libgit2::oid::oid::{Oid, OidType};
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
/// use libgit2::libgit2::object::sys::commit::ParentCallback;
/// use libgit2::libgit2::oid::oid::Oid;
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
/// use libgit2::libgit2::object::sys::commit::ParentCallback;
/// use libgit2::libgit2::oid::oid::Oid;
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
/// [`create`](crate::libgit2::object::commit::create) does, but with the tree and the
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
/// [`object` module](crate::libgit2::object::object#looking-objects-up)); and a
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
    use core::ffi::{CStr, c_int};
    use std::process::Command;

    use super::*;
    use crate::libgit2::util::errors::{ErrorBuf, last};
    use crate::libgit2::util::sys::errors::runtime_pinned;

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
            let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
            let (mut repo, mut builder, mut who) =
                (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            let mut tree = ffi::git_oid::default();
            {
                // SAFETY: writable slots, NUL-terminated strings and the live repository;
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
                        written = Some(w.create_updating(Some(branch), c"lenient\n", cb, payload));
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
                    drop(
                        ParentCallback::new(&mut boom).walk_with(OidType::Sha1, |cb, payload| {
                            w.create_updating(Some(branch), c"panicked\n", cb, payload)
                        }),
                    );
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

    use crate::libgit2::core::types::{SignatureView, Time};
    use crate::libgit2::object::commit::{Commit, create};
    use crate::libgit2::object::tree::Tree;
    use crate::libgit2::repository::repository::{Repository, RepositoryBox};
    use crate::libgit2::util::errors::ErrorClass;

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
            let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
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
        let me = SignatureView::new(c"A U Thor", c"a@example.com", Time::new(1_700_000_000, 60));
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
            Oid::from_string(c"dededededededededededededededededededede", OidType::Sha1).unwrap();
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
        let e =
            create_from_ids(repo, None, me, me, None, c"m\n", missing.as_ref(), &[]).unwrap_err();
        assert_eq!(e.raw_code(), -1);
    }
}
