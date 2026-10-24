//! Safe wrappers over `include/git2/repository.h`.
//!
//! # Layout items and open flags
//!
//! An [`Item`] is a `git_repository_item_t`: one file or directory of a repository's
//! layout, which `git_repository_item_path` resolves against the repository's git,
//! common or working directory. libgit2 indexes a static table of
//! `GIT_REPOSITORY_ITEM__LAST` entries with the value unchecked, so the sentinel and
//! every value past it read beyond that table: [`Item`] has no variant for them, and
//! [`Item::from_raw`] refuses them.
//!
//! An [`OpenFlag`] is a `git_repository_open_flag_t`, one bit of the `unsigned int`
//! that `git_repository_open_ext` takes; an [`OpenFlags`] set ORs them. libgit2 only
//! tests the bits it names and ignores the others, so a set keeps any bit it is given
//! ([`OpenFlags::from_bits_retain`]) and passes it through unchanged.
//!
//! # Initialisation options
//!
//! An [`InitOptions`] is a `git_repository_init_options`: the [`InitFlags`],
//! [`InitMode`], strings, object id type and reference storage that
//! `git_repository_init_ext` creates a repository with. libgit2 never allocates or
//! releases one: the caller fills it with `git_repository_init_options_init`
//! ([`InitOptions::new`]) and keeps it, and libgit2 only reads its strings during the
//! call, never keeping or freeing them. So Rust holds the options by value and borrows
//! the strings for `'o`, the getters on [`InitOptionsRef`] and the setters on
//! [`InitOptionsMut`], which is invariant in `'o`. The object id type and reference
//! storage are typed, `None` for the zero default: libgit2 does not check them, and
//! creates a repository it cannot open from a value it does not name.
//!
//! `git_repository_init_ext` ([`Repository::init_ext`]) writes to its options, `ORing`
//! internal bits into their flags, so it takes them exclusively ([`InitOptionsMut`]);
//! the wrapper clears those bits before each call, where libgit2 would read the ones a
//! previous call left as describing this one.
//!
//! # Repository state
//!
//! A [`RepositoryState`] is a `git_repository_state_t`: the multi-step operation a
//! repository is in the middle of, as `git_repository_state` reads it from the marker
//! files git leaves in the git directory. Its one producer returns it as an `int`, and
//! `-1` when it cannot build the path it probes, so a raw value is converted fallibly
//! ([`RepositoryState::from_raw`] and `TryFrom`) and never transmuted.
//!
//! # The repository
//!
//! A [`Repository`] is the opaque `git_repository`, which libgit2 allocates and frees
//! with no count: one [`RepositoryBox`] owns it, built by the constructors on
//! [`Repository`], which report a failure as a [`RepositoryError`].
//!
//! The routines over a repository go on its handles: [`RepositoryRef`] for those that
//! read it, or only fill its caches and write through the counted objects it shares
//! (its paths and object id type, the odb and refdb it lends, its state, its message,
//! its `HEAD`); [`RepositoryMut`] for those that replace a string it lends or a part it
//! owns (its identity, namespace and working directory, and the setters of
//! `git2/sys/repository.h`). A repository without a git directory ([`Repository::new`])
//! is refused by the routines that would build a path inside it, where libgit2 resolves
//! the path against the current directory or dereferences it. A
//! [`MergeheadCallback`] is the closure `git_repository_mergehead_foreach` calls with
//! each id of `MERGE_HEAD`.
//!
//! A string a getter lends ([`commondir`](RepositoryRef::commondir),
//! [`namespace`](RepositoryRef::namespace), [`ident`](RepositoryRef::ident)) lives as
//! long as the shared borrow. The counted index ([`IndexArc`]), a config snapshot and
//! the commits of [`commit_parents`](RepositoryRef::commit_parents) outlive the
//! repository; the counted config ([`ConfigArc`], whose file backends keep the
//! repository for their `includeIf` conditions) and references ([`ReferenceBox`])
//! borrow it. [`discover`] finds a repository without
//! opening it, and a [`FetchheadForeachCallback`] is the closure
//! [`for_each_fetchhead`](RepositoryRef::for_each_fetchhead) calls with each
//! [`FetchheadEntry`].
//!
//! A repository built without paths ([`Repository::new`], [`Repository::wrap_odb`]) has
//! no git directory, common directory or working directory. libgit2 resolves some
//! paths against those regardless, so against the process's working directory; the
//! wrappers whose routines would then write or remove files there refuse such a
//! repository instead ([`RepositoryMut::cleanup_state`], and
//! [`Worktree::lookup`](crate::libgit2::src::libgit2::worktree::Worktree::lookup), whose
//! routine would read a NULL path).

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
    /// the [`is_merge`](FetchheadEntry::is_merge) of the entry before it (`false` for
    /// the first), where an id alone is always for merge. Fails
    /// with [`ErrorCode::NotFound`] when there is no `FETCH_HEAD`, and for a repository
    /// without paths, whose `FETCH_HEAD` libgit2 would read from the process's working
    /// directory.
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
    /// entries before it were seen; with [`ErrorCode::NotFound`] when there is no `FETCH_HEAD`, and
    /// for a repository without paths.
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
