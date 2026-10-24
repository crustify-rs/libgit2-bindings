//! The `FETCH_HEAD` walk of `src/libgit2/fetchhead.c`:
//! [`RepositoryRef::for_each_fetchhead`](super::repository::RepositoryRef::for_each_fetchhead),
//! which hands each entry of the file to a callback. The entries and the callback are
//! [`repository`](super::repository)'s.

use core::ops::ControlFlow;

use crate::ffi;
use crate::libgit2::src::libgit2::repository::{
    FetchheadForeachCallback, RepositoryError, RepositoryRef, pathless, unpinned,
};
use crate::libgit2::src::util::errors::runtime_pinned;

impl RepositoryRef<'_> {
    /// Calls `callback` with each entry of the repository's `FETCH_HEAD`, in file
    /// order, until its closure returns `Break`: `Ok(Continue(()))` when every entry
    /// was seen, `Ok(Break(()))` when the closure stopped the walk. A panic of the
    /// closure stops the walk too, and is resumed here once libgit2 has returned.
    ///
    /// Each line is parsed before its call, so the entries before a malformed line
    /// are seen and the walk then fails with `GIT_ERROR_FETCHHEAD` and the line's
    /// number (an empty line, a bad id, a bad `not-for-merge` marker or description,
    /// a last line without its newline); ids are read as the repository's type. A line
    /// of an id and a tab alone has no marker to parse, and libgit2 gives its entry
    /// the [`is_merge`](super::repository::FetchheadEntry::is_merge) of the entry before it
    /// (`false` for the first), where an id alone is always for merge. Fails
    /// with [`ErrorCode::NotFound`](crate::libgit2::src::util::errors::ErrorCode::NotFound) when
    /// there is no `FETCH_HEAD`, and for a repository without paths, whose `FETCH_HEAD` libgit2
    /// would read from the process's working directory.
    ///
    /// libgit2 reads the file and calls back synchronously, on this thread, so the
    /// closure may use this repository's shared handles meanwhile, but not its
    /// exclusive one:
    ///
    /// ```compile_fail,E0502
    /// use std::ops::ControlFlow;
    /// use libgit2::libgit2::src::libgit2::repository::{
    ///     FetchheadEntry, FetchheadForeachCallback, RepositoryBox,
    /// };
    ///
    /// fn walk(mut repo: RepositoryBox) {
    ///     let mut rename = |_: FetchheadEntry<'_>| {
    ///         let _ = repo.as_mut().set_namespace(None);
    ///         ControlFlow::Continue(())
    ///     };
    ///     let mut callback = FetchheadForeachCallback::new(&mut rename);
    ///     let _ = repo.as_ref().for_each_fetchhead(&mut callback);
    /// }
    /// ```
    ///
    /// When the closure stops the walk, libgit2 leaves
    /// "`git_repository_fetchhead_foreach` callback returned -7" as the thread's error
    /// unless one is set already (by a libgit2 call the closure made, say).
    ///
    /// Wraps: `git_repository_fetchhead_foreach`
    ///
    /// # Errors
    ///
    /// Fails with `GIT_ERROR_FETCHHEAD` and the line's number for a malformed line, after the
    /// entries before it were seen; with
    /// [`ErrorCode::NotFound`](crate::libgit2::src::util::errors::ErrorCode::NotFound) when there
    /// is no `FETCH_HEAD`, and for a repository without paths.
    pub fn for_each_fetchhead(
        &self,
        callback: &mut FetchheadForeachCallback<'_>,
    ) -> Result<ControlFlow<()>, RepositoryError> {
        // A walk that makes no call (refused, or of an empty file) stopped nothing.
        callback.stopped = false;
        if !runtime_pinned() {
            return Err(unpinned());
        }
        if self.path().is_none() {
            return Err(pathless());
        }
        let repo = self.as_ptr().cast_mut();
        let (f, payload) = callback.raw();
        // SAFETY: a live repository with a git directory, used on this thread, which
        // the routine only reads (its `gitdir` and object id type); the callback's own
        // trampoline and payload, which nothing else touches until the call returns
        // (`callback` is exclusively borrowed). The runtime is pinned.
        let rc = unsafe { ffi::git_repository_fetchhead_foreach(repo, f, payload) };
        callback.resume_panic();
        match rc {
            0 => Ok(ControlFlow::Continue(())),
            ffi::GIT_EUSER if callback.stopped => Ok(ControlFlow::Break(())),
            rc => Err(RepositoryError::from_libgit2(rc)),
        }
    }
}
