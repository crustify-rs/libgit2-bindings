//! Safe wrappers over `include/git2/sys/commit.h`.
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
//! the list, after which the commit is written. It also reads each id from the object
//! database without checking that it is of the repository's id type, which the loose
//! backend then formats out of bounds (see the
//! [`object` module](crate::libgit2::object::object#looking-objects-up)). So once the
//! closure panics, or returns an id of another type than the repository's, the
//! trampoline hands libgit2 the repository's zero id, once, then NULL: strict object
//! creation finds no such commit and the routine fails before writing anything, and the
//! routine's wrapper reports the refusal or resumes the panic.

use core::any::Any;
use core::ffi::c_void;
use core::fmt;
use core::ptr;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use ffibox::CZeroable;

use crate::ffi;
use crate::libgit2::oid::oid::{Oid, OidRef, OidType};
use crate::libgit2::repository::repository::RepositoryError;
use crate::libgit2::util::errors::{ErrorClass, ErrorCode};
use crate::libgit2::util::sys::errors::set;

/// The closure a [`ParentCallback`] calls: the index of the parent asked for, from 0.
type ParentFn<'f> = dyn FnMut(usize) -> Option<Oid> + 'f;

/// How a walk through a [`ParentCallback`] failed.
enum Failure {
    /// The closure returned an id of another type than the repository's.
    Refused,
    /// The closure panicked; the panic resumes once the walk returns.
    Panicked(Box<dyn Any + Send + 'static>),
}

/// A callback giving the parents of a commit being written
/// (`git_commit_parent_callback`): `git_commit_create_from_callback` calls it with each
/// index from 0, and takes each id it returns as the next parent, until it returns
/// `None`.
///
/// Built over a closure with [`new`](Self::new), which returns each parent's id by
/// value: the callback keeps it, and its `extern "C"` trampoline returns a pointer to it,
/// valid until the next call, which libgit2 reads and copies before calling again. A
/// panic in the closure, or an id of another type than the repository's, cannot stop
/// libgit2 cleanly; the trampoline then fails the walk instead (see the
/// [module docs](self#parent-callbacks)), and the panic resumes once control is back in
/// Rust, from [`call`](Self::call) or from the wrapper that ran the walk.
///
/// ```
/// use libgit2::libgit2::object::sys::commit::ParentCallback;
/// use libgit2::libgit2::oid::oid::{Oid, OidType};
///
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// let parents = [
///     Oid::from_string(c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750", OidType::Sha1).unwrap(),
/// ];
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
/// Wraps: git_commit_parent_callback
/// Handwritten
pub struct ParentCallback<'f> {
    f: &'f mut ParentFn<'f>,
    /// The id last handed out, which libgit2 copies before it calls again.
    last: Oid,
    /// The repository's id type while a C routine walks the parents, `None` otherwise.
    kind: Option<OidType>,
    /// How the walk failed, if it did.
    failure: Option<Failure>,
    /// Whether the zero id was handed out after the failure; NULL follows it.
    stopped: bool,
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
            kind: None,
            failure: None,
            stopped: false,
        }
    }

    /// Calls the callback for the parent at `idx`, through the trampoline libgit2 calls,
    /// and returns what libgit2 would read from it: the parent's id, kept by the callback
    /// until its next call, or `None` at the end of the list. A panic of the closure
    /// resumes here.
    pub fn call(&mut self, idx: usize) -> Option<OidRef<'_>> {
        let id = self.with_payload(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            // SAFETY: `payload` is this callback, which nothing else uses during the
            // call: the trampoline's contract.
            unsafe { callback(idx, payload) }
        });
        if let Some(Failure::Panicked(panic)) = self.failure.take() {
            resume_unwind(panic);
        }
        // SAFETY: the trampoline returns NULL or a pointer to `self.last`, live and
        // unwritten while the result borrows `self`.
        unsafe { OidRef::from_ptr(id.cast_mut()) }
    }

    /// Runs `walk` with the C callback and its payload, as the wrapper of the routine
    /// that calls the callback (`git_commit_create_from_callback`) hands them to it, for
    /// a repository whose ids are of type `kind`. The payload is this callback, which
    /// nothing else touches until `walk` returns.
    ///
    /// Resumes a panic the closure raised meanwhile. When the closure returned an id of
    /// another type than `kind`, which was never handed to libgit2, the walk fails with
    /// libgit2's wording for the mismatch (`GIT_ERROR_INVALID`), set as the thread's
    /// error, whatever `walk` returned.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn walk_with<R>(
        &mut self,
        kind: OidType,
        walk: impl FnOnce(ffi::git_commit_parent_callback, *mut c_void) -> R,
    ) -> Result<R, RepositoryError> {
        self.kind = Some(kind);
        let result = self.with_payload(walk);
        self.kind = None;
        match self.failure.take() {
            None => Ok(result),
            Some(Failure::Panicked(panic)) => resume_unwind(panic),
            Some(Failure::Refused) => {
                set(
                    ErrorClass::Invalid,
                    c"specified object ID type does not match repository object ID type",
                );
                Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()))
            }
        }
    }

    /// Runs `walk` with the trampoline and this callback as its payload, from a clean
    /// state.
    fn with_payload<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_commit_parent_callback, *mut c_void) -> R,
    ) -> R {
        self.failure = None;
        self.stopped = false;
        walk(Some(parent_with), ptr::from_mut(self).cast())
    }

    /// What the trampoline hands out once the walk failed: inside a C walk the
    /// repository's zero id, once, then nothing (NULL); outside one, nothing.
    fn stop(&mut self) -> Option<&Oid> {
        match self.kind {
            Some(kind) if !self.stopped => {
                self.stopped = true;
                self.last = Oid::zero(kind);
                Some(&self.last)
            }
            _ => None,
        }
    }
}

impl fmt::Debug for ParentCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParentCallback")
            .field("failed", &self.failure.is_some())
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
    let id = if callback.failure.is_some() {
        callback.stop()
    } else {
        match catch_unwind(AssertUnwindSafe(|| (callback.f)(idx))) {
            Ok(None) => None,
            Ok(Some(id)) => {
                let fits = callback
                    .kind
                    .is_none_or(|kind| u32::from(id.as_ref().raw_kind()) == kind.raw());
                if fits {
                    callback.last = id;
                    Some(&callback.last)
                } else {
                    callback.failure = Some(Failure::Refused);
                    callback.stop()
                }
            }
            Err(panic) => {
                callback.failure = Some(Failure::Panicked(panic));
                callback.stop()
            }
        }
    };
    id.map_or(ptr::null(), |id| id.as_ref().as_ptr())
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::util::errors::{ErrorBuf, last};
    use crate::libgit2::util::sys::errors::runtime_pinned;
    use core::ffi::{CStr, c_int};
    use std::process::Command;

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
        assert!(format!("{callback:?}").starts_with("ParentCallback { failed: false"));
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
            if calls == 1 {
                panic!("no parent");
            }
            Some(sha1(3))
        };
        let mut callback = ParentCallback::new(&mut next);
        let caught = std::panic::catch_unwind(AssertUnwindSafe(|| view(callback.call(0))));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"no parent")
        );
        assert_eq!(view(callback.call(0)), view(Some(sha1(3).as_ref())));
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
            let _ = std::fs::remove_dir_all(&dir);
            let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
            let (mut repo, mut builder, mut who) =
                (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            let mut tree = ffi::git_oid::default();
            // SAFETY: writable slots, NUL-terminated strings and the live repository;
            // the builder is freed once, the rest when the writer drops.
            unsafe {
                assert_eq!(ffi::git_repository_init(&mut repo, path.as_ptr(), 1), 0);
                assert_eq!(ffi::git_treebuilder_new(&mut builder, repo, ptr::null()), 0);
                assert_eq!(ffi::git_treebuilder_write(&mut tree, builder), 0);
                ffi::git_treebuilder_free(builder);
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
                    &self.tree,
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
            // SAFETY: writable slots, the live repository and id; the object's data is
            // `size` bytes, copied before the object and the odb are freed once.
            let raw = unsafe {
                assert_eq!(ffi::git_repository_odb(&mut odb, self.repo), 0);
                assert_eq!(ffi::git_odb_read(&mut obj, odb, id.as_ref().as_ptr()), 0);
                let data = core::slice::from_raw_parts(
                    ffi::git_odb_object_data(obj).cast::<u8>(),
                    ffi::git_odb_object_size(obj),
                )
                .to_vec();
                ffi::git_odb_object_free(obj);
                ffi::git_odb_free(odb);
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
            unsafe {
                ffi::git_signature_free(self.who);
                ffi::git_repository_free(self.repo);
            }
            let _ = std::fs::remove_dir_all(&self.dir);
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

    /// An id of another type than the repository's never reaches libgit2: the zero id
    /// of the repository's type goes instead, which strict object creation finds
    /// missing, so nothing is written; the walk fails with libgit2's wording.
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
        assert!(format!("{callback:?}").contains("failed: false"));
        drop(callback);
        assert_eq!(asked, [0]);
        assert_eq!(w.commits(), 0);
    }

    /// A panic in the closure stops the walk the same way, and resumes once libgit2 has
    /// returned, with nothing written.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn walks_resume_panics_after_c() {
        let w = Writer::new("panics");
        let mut boom = |_: usize| -> Option<Oid> { panic!("no parent") };
        let mut callback = ParentCallback::new(&mut boom);
        let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = w.create_with(c"panicked\n", &mut callback);
        }));
        assert_eq!(
            caught.unwrap_err().downcast_ref::<&str>(),
            Some(&"no parent")
        );
        assert_eq!(w.commits(), 0);
    }

    /// Without strict object creation libgit2 reads no parent: the zero id goes in once,
    /// then NULL ends the list, so the walk terminates, and still fails. Global option:
    /// run alone in a child.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn refusals_end_the_list_without_strict_creation() {
        isolated(
            concat!(
                module_path!(),
                "::refusals_end_the_list_without_strict_creation"
            ),
            || {
                assert!(runtime_pinned());
                // SAFETY: an option taking an `int`; this child runs nothing else.
                let rc = unsafe {
                    ffi::git_libgit2_opts(ffi::GIT_OPT_ENABLE_STRICT_OBJECT_CREATION as c_int, 0)
                };
                assert_eq!(rc, 0);
                let w = Writer::new("lenient");
                let mut asked = Vec::new();
                let mut wide = |i: usize| {
                    asked.push(i);
                    Some(Oid::zero(OidType::Sha256))
                };
                let mut written = None;
                let e = ParentCallback::new(&mut wide)
                    .walk_with(OidType::Sha1, |cb, payload| {
                        written = Some(w.create(c"lenient\n", cb, payload).unwrap());
                    })
                    .unwrap_err();
                assert_eq!(e.class(), Some(ErrorClass::Invalid));
                assert_eq!(asked, [0]);
                let zero = Oid::zero(OidType::Sha1).as_ref().to_hex_string().unwrap();
                assert_eq!(w.parents(&written.unwrap()), [zero]);
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
}
