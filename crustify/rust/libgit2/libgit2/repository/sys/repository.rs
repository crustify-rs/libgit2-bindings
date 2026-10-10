//! Safe wrappers over `include/git2/sys/repository.h`.
//!
//! # Options for a repository without backends
//!
//! `git_repository_new_ext` takes a `git_repository_new_options` ([`NewOptions`]): a
//! structure version and the `git_oid_t` of the objects the new repository holds. It
//! only reads them, for the call: it refuses a version of 0 or above
//! [`NewOptions::VERSION`] with `GIT_ERROR_INVALID`, reads an `oid_type` of `0` as
//! `GIT_OID_DEFAULT`, and asserts that any other value is a type it names, which
//! aborts a debug build of libgit2 and fails with `-1` otherwise. So the version keeps
//! any value, and the object id type is an `Option<OidType>`: `None` is the zero
//! default, and no safe path stores a value [`OidType`] does not name.
//!
//! The options are a plain value, never allocated or released by libgit2.
//! [`NewOptions::new`] fills one through `git_repository_new_options_init`, as
//! `GIT_REPOSITORY_NEW_OPTIONS_INIT` does: the current version and the default type.
//! All-zero is not that default (libgit2 refuses version 0), so there is no `zeroed`.
//!
//! # The submodule cache
//!
//! A repository can hold a cache of its submodules, which `git_submodule_lookup` (and
//! a diff, for the submodules it meets) answers from instead of reading
//! `.gitmodules`, the index, `HEAD` and the working directory again: it reflects the
//! repository as it was when the cache was loaded, until it is cleared. The cache
//! holds a counted reference to each submodule, so a submodule looked up through it
//! outlives a clear.
//!
//! Both routines take the exclusive [`RepositoryMut`]: loading replaces the cache and
//! clearing frees it, and libgit2 routines that read it keep a bare pointer to it while
//! they run, so no other handle of the repository may be in use meanwhile.
//!
//! # Installed parts
//!
//! `git_repository_new` is [`Repository::new`] (and [`RepositoryFree`]'s constructor),
//! next to the repository's other constructors.
//!
//! The setters below hand the repository one counted reference of its own to a config,
//! index, object database or reference database, and make the repository that object's
//! owner (`rc.owner`); the object the repository held before is disowned and released.
//! The caller's reference stays the caller's. They write the repository's fields, so
//! they take the exclusive [`RepositoryMut`]: no handle that borrowed the repository's
//! previous part (an [`OdbArc`] from [`RepositoryRef::odb`]) is live across the swap.
//! The reference database is the exception (below).
//!
//! Each part's own contract decides how it is passed:
//!
//! - a config and an index are shared once installed, so the setters take a
//!   [`ConfigArc`] / [`IndexArc`] by reference (a sole [`ConfigBox`] or [`IndexBox`]
//!   converts with `.into()` first);
//! - an object database reads its owning repository's object cache, so only one no
//!   repository owns yet, an [`OdbArc<'static>`](OdbArc), is installed, and it is
//!   consumed: the repository keeps it, and [`RepositoryRef::odb`] lends it back,
//!   borrowing the repository;
//! - a reference database points back at the repository it was opened for, which
//!   libgit2 does not update when another repository installs it, so
//!   [`set_refdb`](RepositoryRef::set_refdb) is `unsafe`; and a refdb built for the
//!   repository itself borrows it, so it takes the shared [`RepositoryRef`].
//!
//! # Cleaning up
//!
//! [`RepositoryMut::cleanup`] releases what a [`Repository`] has loaded, as freeing it
//! does first. Like the submodule cache's routines, it takes the exclusive handle.
//!
//! [`Repository`]: crate::libgit2::repository::repository::Repository
//! [`Repository::new`]: crate::libgit2::repository::repository::Repository::new
//! [`RepositoryFree`]: crate::libgit2::repository::repository::RepositoryFree
//! [`ConfigBox`]: crate::libgit2::config::config::ConfigBox
//! [`IndexBox`]: crate::libgit2::index::index::IndexBox

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::ptr::{self, addr_of, addr_of_mut};

use ffibox::define_ctype;

use crate::ffi;
use crate::libgit2::config::config::ConfigArc;
use crate::libgit2::index::index::IndexArc;
use crate::libgit2::odb::odb::OdbArc;
use crate::libgit2::oid::oid::{OidType, UnnamedOidType};
use crate::libgit2::refs::refdb::RefdbArc;
use crate::libgit2::repository::repository::{
    RepositoryBox, RepositoryError, RepositoryMut, RepositoryRef, pathless, status,
};
use crate::libgit2::util::errors::{ErrorClass, ErrorCode};
use crate::libgit2::util::sys::errors::set;
define_ctype!(
    /// Options for a repository without backends (`git_repository_new_options`): the
    /// structure version, and the type of the object ids the repository holds.
    ///
    /// A plain value of 8 bytes, resource-free: `Copy`, held inline, never released.
    /// Build one with [`new`](Self::new) (or `Default`), then read and write its fields
    /// through [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut):
    ///
    /// ```
    /// use libgit2::libgit2::oid::oid::OidType;
    /// use libgit2::libgit2::repository::sys::repository::NewOptions;
    ///
    /// # if cfg!(miri) { return; }
    /// let mut opts = NewOptions::new();
    /// assert_eq!(opts.as_ref().oid_type(), None);
    /// opts.as_mut().set_oid_type(Some(OidType::Sha256));
    /// assert_eq!(opts.as_ref().version(), NewOptions::VERSION);
    /// assert_eq!(opts.as_ref().oid_type(), Some(OidType::Sha256));
    /// ```
    ///
    /// A handle borrows the value, so it cannot outlive it:
    ///
    /// ```compile_fail,E0597
    /// use libgit2::libgit2::repository::sys::repository::NewOptions;
    ///
    /// let handle = {
    ///     let opts = NewOptions::new();
    ///     opts.as_ref()
    /// };
    /// let _ = handle.oid_type();
    /// ```
    ///
    /// nor coexist with an exclusive one:
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::repository::sys::repository::NewOptions;
    ///
    /// let mut opts = NewOptions::new();
    /// let mut m = opts.as_mut();
    /// let r = opts.as_ref();
    /// m.set_oid_type(None);
    /// let _ = r.oid_type();
    /// ```
    ///
    /// and the shared handle only reads:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::repository::sys::repository::NewOptions;
    ///
    /// let opts = NewOptions::new();
    /// opts.as_ref().set_oid_type(None);
    /// ```
    ///
    /// Wraps: git_repository_new_options
    #[derive(Clone, Copy)]
    NewOptions, NewOptionsRef, NewOptionsMut, ffi::git_repository_new_options
);

// SAFETY: a `NewOptions` is 8 plain bytes: no pointer, no interior mutability, no
// thread affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for NewOptions {}
// SAFETY: `NewOptionsMut` is an exclusive borrow of such a value, as `&mut NewOptions`.
unsafe impl Send for NewOptionsMut<'_> {}

// SAFETY: shared access only reads the fields (every write goes through
// `NewOptionsMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for NewOptions {}
// SAFETY: as above; `NewOptionsRef` is a shared borrow, as `&NewOptions`.
unsafe impl Send for NewOptionsRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for NewOptionsRef<'_> {}
// SAFETY: `&NewOptionsMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for NewOptionsMut<'_> {}

impl NewOptions {
    /// The structure version this crate's header declares
    /// (`GIT_REPOSITORY_NEW_OPTIONS_VERSION`), the only one libgit2 accepts.
    pub const VERSION: u32 = ffi::GIT_REPOSITORY_NEW_OPTIONS_VERSION;

    /// The default options, as `git_repository_new_options_init` fills them for
    /// [`VERSION`](Self::VERSION) and `GIT_REPOSITORY_NEW_OPTIONS_INIT` spells them:
    /// the current version and the default object id type ([`OidType::default`]).
    /// Needs no initialised libgit2.
    ///
    /// Wraps: git_repository_new_options_init
    pub fn new() -> NewOptions {
        let mut raw = ffi::git_repository_new_options::default();
        // SAFETY: `raw` is a live `git_repository_new_options` the call may write, so
        // the routine's non-NULL assertion holds. With the header's own version its
        // version check passes, so it only copies the 8-byte
        // `GIT_REPOSITORY_NEW_OPTIONS_INIT` template into `raw`: no allocation and no
        // error state, so no initialised runtime is needed.
        let rc = unsafe { ffi::git_repository_new_options_init(&mut raw, Self::VERSION) };
        assert_eq!(
            rc, 0,
            "git_repository_new_options_init refused its own version"
        );
        NewOptions(raw, PhantomData)
    }
}

impl Default for NewOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The `git_repository_new_options` value, for code that already holds one by value.
/// Any version is kept; an `oid_type` that is neither `0` nor a type [`OidType`] names
/// is refused, as libgit2 would assert on it.
impl TryFrom<ffi::git_repository_new_options> for NewOptions {
    type Error = UnnamedOidType;

    fn try_from(raw: ffi::git_repository_new_options) -> Result<NewOptions, UnnamedOidType> {
        if raw.oid_type != 0 {
            OidType::try_from(raw.oid_type)?;
        }
        Ok(NewOptions(raw, PhantomData))
    }
}

/// The `git_repository_new_options` value, for code that passes one by value.
impl From<NewOptions> for ffi::git_repository_new_options {
    fn from(opts: NewOptions) -> ffi::git_repository_new_options {
        opts.0
    }
}

impl NewOptionsRef<'_> {
    /// The structure version. [`NewOptions::new`] sets [`NewOptions::VERSION`];
    /// libgit2 refuses the options with `GIT_ERROR_INVALID` if it is 0 or above that.
    ///
    /// Field: git_repository_new_options.version
    pub fn version(&self) -> u32 {
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_repository_new_options`; any value is a valid `u32`.
        unsafe { addr_of!((*self.as_ptr()).version).read() }
    }

    /// The type of the repository's object ids; `None`, the default, means
    /// [`OidType::default`]. Every safe path stores `0` or a named type, so `None` is
    /// always the zero default.
    ///
    /// Field: git_repository_new_options.oid_type
    pub fn oid_type(&self) -> Option<OidType> {
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_repository_new_options`; the field is a plain `u32`, any value valid.
        let raw = unsafe { addr_of!((*self.as_ptr()).oid_type).read() };
        OidType::from_raw(raw)
    }

    /// A copy of the options, as a value of its own.
    pub fn to_options(&self) -> NewOptions {
        // SAFETY: a read of the whole live `git_repository_new_options` through the
        // shared handle's pointer; it is `Copy` plain data.
        NewOptions(unsafe { self.as_ptr().read() }, PhantomData)
    }
}

impl NewOptionsMut<'_> {
    /// Replaces the structure version. Any value is accepted; libgit2 refuses options
    /// whose version is 0 or above [`NewOptions::VERSION`].
    ///
    /// Field: git_repository_new_options.version
    pub fn set_version(&mut self, version: u32) {
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_repository_new_options`; the field is a plain integer.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).version).write(version) }
    }

    /// Replaces the type of the repository's object ids; `None` stores `0`, which
    /// libgit2 reads as [`OidType::default`].
    ///
    /// Field: git_repository_new_options.oid_type
    pub fn set_oid_type(&mut self, oid_type: Option<OidType>) {
        let raw = oid_type.map_or(0, OidType::raw);
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_repository_new_options`; the field is a plain integer, and `raw` is 0 or
        // a type libgit2 names.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).oid_type).write(raw) }
    }
}

impl fmt::Debug for NewOptionsRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewOptions")
            .field("version", &self.version())
            .field("oid_type", &self.oid_type())
            .finish()
    }
}

impl fmt::Debug for NewOptionsMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for NewOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'b> PartialEq<NewOptionsRef<'b>> for NewOptionsRef<'_> {
    /// Field by field.
    fn eq(&self, other: &NewOptionsRef<'b>) -> bool {
        self.version() == other.version() && self.oid_type() == other.oid_type()
    }
}

impl Eq for NewOptionsRef<'_> {}

impl Hash for NewOptionsRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.version().hash(state);
        self.oid_type().hash(state);
    }
}

impl PartialEq for NewOptions {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for NewOptions {}

impl Hash for NewOptions {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

// A `NewOptions` is a `git_repository_new_options`, so C reads a value Rust holds in
// place.
const _: () = assert!(
    size_of::<NewOptions>() == size_of::<ffi::git_repository_new_options>()
        && align_of::<NewOptions>() == align_of::<ffi::git_repository_new_options>()
);

impl RepositoryMut<'_> {
    /// Loads every submodule of the repository into its submodule cache: those that
    /// `.gitmodules` configures, then those the index and `HEAD`'s tree record, then
    /// what the working directory holds of each. The index and `HEAD` add only
    /// submodules `.gitmodules` names, so a repository without a working directory
    /// (bare, or without paths) gets an empty cache.
    ///
    /// A cache already loaded is released first, so this reloads it; libgit2 alone
    /// would leak it. On failure the repository is left without a cache.
    ///
    /// Fails with libgit2's error when the submodules cannot be read (a malformed
    /// `.gitmodules`, say), or memory runs out.
    ///
    /// A shared handle can neither load nor clear the cache:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::repository::repository::RepositoryBox;
    ///
    /// fn reload(repo: &RepositoryBox) {
    ///     let _ = repo.as_ref().cache_submodules();
    /// }
    /// ```
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::repository::repository::RepositoryBox;
    ///
    /// fn clear(repo: &RepositoryBox) {
    ///     repo.as_ref().clear_submodule_cache();
    /// }
    /// ```
    ///
    /// Wraps: git_repository_submodule_cache_all
    pub fn cache_submodules(&mut self) -> Result<(), RepositoryError> {
        self.clear_submodule_cache();
        // SAFETY: a live repository on this thread, exclusively borrowed, whose cache
        // slot is NULL (cleared above), so the routine stores a fresh cache there, owned
        // by the repository, without overwriting one. A repository exists only once its
        // constructor pinned the runtime.
        let rc = unsafe { ffi::git_repository_submodule_cache_all(self.as_mut_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        Ok(())
    }

    /// Releases the repository's submodule cache, if it has one: lookups read the
    /// repository again. Submodules looked up through the cache stay valid; each holds
    /// its own reference.
    ///
    /// Wraps: git_repository_submodule_cache_clear
    pub fn clear_submodule_cache(&mut self) {
        // SAFETY: a live repository on this thread, exclusively borrowed, so no libgit2
        // routine holds its cache while the routine frees it and sets the slot to NULL.
        let rc = unsafe { ffi::git_repository_submodule_cache_clear(self.as_mut_ptr()) };
        // `git_submodule_cache_free` returns 0 whatever it frees.
        debug_assert_eq!(rc, 0);
    }
}

/// The object id type `odb` hashes with, as `git_repository_wrap_odb` reads it into a
/// scratch repository; libgit2 publishes no accessor of its own.
///
/// The scratch repository owns the odb while it lives and disowns it when it is freed,
/// before this returns: `odb` is a reference no repository owns, which no other Rust
/// handle reaches (it was moved here), so nothing observes the transient owner.
fn odb_oid_type(odb: &OdbArc<'static>) -> Result<OidType, RepositoryError> {
    let mut out = ptr::null_mut();
    // SAFETY: `out` is a writable slot; `odb` a live odb no repository owns, which the
    // scratch repository takes a counted reference to. Every repository's runtime was
    // pinned when it was built, and so was the odb's.
    let rc = unsafe { ffi::git_repository_wrap_odb(&mut out, odb.as_c_ptr()) };
    // SAFETY: `git_repository_wrap_odb` leaves `out` NULL on failure, or writes a fresh
    // repository the caller owns alone; it is freed at the end of this function, which
    // releases its reference to the odb and clears the odb's owner.
    let scratch = unsafe { RepositoryBox::from_c(out) };
    match scratch {
        Some(scratch) if rc >= 0 => Ok(scratch.as_ref().oid_type()),
        scratch => {
            let error = RepositoryError::from_libgit2(rc);
            drop(scratch);
            Err(error)
        }
    }
}

impl RepositoryMut<'_> {
    /// Makes the repository bare: records `core.bare = true` and removes
    /// `core.worktree` in its configuration, then forgets its working directory. A
    /// bare repository is left as it is.
    ///
    /// Fails with libgit2's error when the configuration cannot be loaded or written
    /// (`GIT_EREADONLY` for a repository without a configuration file of its own),
    /// leaving the working directory in place.
    ///
    /// Wraps: git_repository_set_bare
    pub fn set_bare(&mut self) -> Result<(), RepositoryError> {
        // SAFETY: a live repository, written through the exclusive handle: libgit2
        // frees its working directory string, which no handle lends past this borrow.
        let rc = unsafe { ffi::git_repository_set_bare(self.as_mut_ptr()) };
        status(rc)
    }

    /// Installs `config` as the repository's configuration, in place of the one it
    /// loaded or was given; the repository keeps a reference of its own, becomes the
    /// config's owner and writes through it, and its cached configuration values are
    /// dropped.
    ///
    /// Wraps: git_repository_set_config
    pub fn set_config(&mut self, config: &ConfigArc) {
        // SAFETY: a live repository, written through the exclusive handle; `config` a
        // live config, which libgit2 up-refs for the repository: the arc's reference
        // stays the arc's, and a shared owner is what the config now has.
        let rc = unsafe { ffi::git_repository_set_config(self.as_mut_ptr(), config.as_c_ptr()) };
        debug_assert_eq!(rc, 0, "git_repository_set_config refused a config");
    }

    /// Installs `index` as the repository's index, in place of the one it loaded or
    /// was given; the repository keeps a reference of its own and becomes the index's
    /// owner. `None` removes the repository's index, which is then loaded from its
    /// index file on next use.
    ///
    /// libgit2 does not compare the index's object id type with the repository's.
    ///
    /// Wraps: git_repository_set_index
    pub fn set_index(&mut self, index: Option<&IndexArc>) {
        let index = index.map_or(ptr::null_mut(), IndexArc::as_c_ptr);
        // SAFETY: a live repository, written through the exclusive handle; `index` NULL
        // or a live index, which libgit2 up-refs for the repository: the arc's reference
        // stays the arc's.
        let rc = unsafe { ffi::git_repository_set_index(self.as_mut_ptr(), index) };
        debug_assert_eq!(rc, 0, "git_repository_set_index refused an index");
    }

    /// Installs `odb` as the repository's object database, in place of the one it
    /// loaded or was given; the repository keeps it, becomes its owner and from then
    /// on caches its objects. [`RepositoryRef::odb`] lends it back.
    ///
    /// The odb must be one no repository owns, which is what `OdbArc<'static>` is; an
    /// arc a repository lent borrows that repository and is refused at compile time:
    ///
    /// ```compile_fail,E0597
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let mut target = Repository::new().unwrap();
    /// let source = Repository::new().unwrap();
    /// let odb = source.as_ref().odb().unwrap();
    /// target.as_mut().set_odb(odb).unwrap();
    /// ```
    ///
    /// Fails, leaving the repository unchanged, when the odb's object id type is not
    /// the repository's (GIT_ERROR_INVALID, "specified object ID type does not match
    /// repository object ID type"): libgit2 does not check it, and the repository's
    /// ids would then be formatted into paths of the odb's length. Reading the odb's
    /// type allocates a scratch repository, which can fail with `-1` when libgit2 runs
    /// out of memory.
    ///
    /// Wraps: git_repository_set_odb
    pub fn set_odb(&mut self, odb: OdbArc<'static>) -> Result<(), RepositoryError> {
        if odb_oid_type(&odb)? != self.as_ref().oid_type() {
            set(
                ErrorClass::Invalid,
                c"specified object ID type does not match repository object ID type",
            );
            return Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()));
        }
        // SAFETY: a live repository, written through the exclusive handle; `odb` a live
        // odb of the repository's type that no repository owns, which libgit2 up-refs
        // and makes this repository's.
        let rc = unsafe { ffi::git_repository_set_odb(self.as_mut_ptr(), odb.as_c_ptr()) };
        debug_assert_eq!(rc, 0, "git_repository_set_odb refused an odb");
        // The repository holds its own reference now.
        drop(odb);
        Ok(())
    }

    /// Rewrites the file system settings `git_repository_init` records in the
    /// repository's configuration (`core.filemode`, `core.symlinks`,
    /// `core.ignorecase`, `core.precomposeunicode`...), probing the file system again,
    /// as after the repository was copied elsewhere; with `recurse`, does the same in
    /// its submodules, whose failures are ignored.
    ///
    /// Fails with libgit2's error when the configuration cannot be read or written. A
    /// repository without a git directory is refused with [`ErrorCode::NotFound`]
    /// (GIT_ERROR_INVALID, "path cannot exist in repository"): libgit2 would write a
    /// `config` file in the current directory.
    ///
    /// Wraps: git_repository_reinit_filesystem
    pub fn reinit_filesystem(&mut self, recurse: bool) -> Result<(), RepositoryError> {
        if self.as_ref().path().is_none() {
            return Err(pathless());
        }
        let repo = self.as_mut_ptr();
        // SAFETY: a live repository with a git directory, written through the exclusive
        // handle (libgit2 clears its cached configuration values).
        let rc = unsafe { ffi::git_repository_reinit_filesystem(repo, recurse.into()) };
        status(rc)
    }
}

impl RepositoryRef<'_> {
    /// Installs `refdb` as the repository's reference database, in place of the one
    /// it opened or was given; the repository keeps a reference of its own and becomes
    /// the refdb's owner. The arc's reference stays the arc's.
    ///
    /// A repository opens its own reference database on first use; this replaces it,
    /// typically with one over a custom backend
    /// ([`Refdb::new`](crate::libgit2::refs::refdb::Refdb::new)). Such a refdb borrows
    /// the repository it is built for, so this takes the shared handle: the repository
    /// cannot be borrowed exclusively while the refdb's arc lives.
    ///
    /// # Safety
    ///
    /// - `refdb` must have been opened for a repository that outlives every reference
    ///   operation this repository runs through it, and whose object id type is this
    ///   repository's: this repository itself always qualifies. libgit2 keeps the
    ///   refdb's pointer to the repository it was opened for, uncounted, and its
    ///   backends read that repository on every operation; object ids the refdb parses
    ///   with that repository's type are then looked up in this one, where an id of
    ///   another type is formatted into a loose object's path of the wrong length.
    ///   Neither follows from `'r`, which bounds only the arc: the repository keeps its
    ///   reference until it is freed or given another refdb.
    /// - No libgit2 routine may be running on this repository, as when this is called
    ///   from one of its callbacks: such a routine may hold the previous refdb without a
    ///   reference of its own, which the swap releases.
    ///
    /// Wraps: git_repository_set_refdb
    pub unsafe fn set_refdb(&self, refdb: &RefdbArc<'_>) {
        let repo = self.as_ptr().cast_mut();
        // SAFETY: a live repository, on this thread, which no libgit2 routine is using
        // per this function's contract, so swapping its refdb is unobserved; `refdb` a
        // live refdb, which libgit2 up-refs for the repository, of a repository that
        // per this function's contract outlives this one's reference operations and
        // shares its object id type.
        let rc = unsafe { ffi::git_repository_set_refdb(repo, refdb.as_c_ptr()) };
        debug_assert_eq!(rc, 0, "git_repository_set_refdb refused a refdb");
    }
}

impl RepositoryMut<'_> {
    /// Releases what the repository has loaded: its object cache, attribute and
    /// submodule caches, grafts, and its references to its configuration, index,
    /// object database and reference database, leaving it as if just opened. Counted
    /// references handed out earlier ([`config`](RepositoryRef::config),
    /// [`index`](RepositoryRef::index)...) stay valid; the repository loads fresh
    /// ones on next use. Freeing a repository cleans it up first.
    ///
    /// libgit2 does not reload the grafts it releases: commits parsed afterwards
    /// ignore `info/grafts` and the shallow boundary, and each parse leaves an
    /// "invalid argument" error on the thread.
    ///
    /// It takes the repository exclusively, so nothing borrowed from it is in use:
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let mut repo = Repository::new().unwrap();
    /// let commondir = repo.as_ref().commondir();
    /// repo.as_mut().cleanup();
    /// let _ = commondir;
    /// ```
    ///
    /// Wraps: git_repository__cleanup
    pub fn cleanup(&mut self) {
        // SAFETY: a live repository, borrowed exclusively, so no handle reads what this
        // releases; counted sub-objects handed out keep their own references.
        let rc = unsafe { ffi::git_repository__cleanup(self.as_mut_ptr()) };
        debug_assert_eq!(rc, 0, "git_repository__cleanup cannot fail");
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    /// `GIT_REPOSITORY_NEW_OPTIONS_INIT`, built in Rust so Miri can run the accessors;
    /// `new_matches_the_template` ties it to `git_repository_new_options_init`.
    fn template() -> NewOptions {
        NewOptions(
            ffi::git_repository_new_options {
                version: ffi::GIT_REPOSITORY_NEW_OPTIONS_VERSION,
                oid_type: 0,
            },
            PhantomData,
        )
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_the_template() {
        let options = NewOptions::new();
        assert_eq!((options.0.version, options.0.oid_type), (1, 0));
        assert_eq!(options, template());
        assert_eq!(NewOptions::default(), options);
    }

    #[test]
    fn defaults_read_as_documented() {
        let options = template();
        assert_eq!(options.as_ref().version(), NewOptions::VERSION);
        assert_eq!(options.as_ref().oid_type(), None);
        assert_eq!(
            format!("{options:?}"),
            "NewOptions { version: 1, oid_type: None }"
        );
    }

    #[test]
    fn each_setter_writes_its_field_only() {
        let mut options = template();
        options.as_mut().set_oid_type(Some(OidType::Sha256));
        assert_eq!(
            (options.0.version, options.0.oid_type),
            (1, ffi::GIT_OID_SHA256)
        );
        options.as_mut().set_version(0);
        assert_eq!(
            (options.0.version, options.0.oid_type),
            (0, ffi::GIT_OID_SHA256)
        );
        options.as_mut().set_oid_type(None);
        assert_eq!((options.0.version, options.0.oid_type), (0, 0));
    }

    #[test]
    fn try_from_keeps_named_types_and_refuses_the_rest() {
        for (raw, ok) in [
            (0, true),
            (ffi::GIT_OID_SHA1, true),
            (ffi::GIT_OID_SHA256, true),
            (3, false),
            (u32::MAX, false),
        ] {
            let value = ffi::git_repository_new_options {
                version: 9,
                oid_type: raw,
            };
            match NewOptions::try_from(value) {
                Ok(o) => {
                    assert!(ok, "{raw}");
                    assert_eq!((o.0.version, o.0.oid_type), (9, raw));
                    let back = ffi::git_repository_new_options::from(o);
                    assert_eq!((back.version, back.oid_type), (9, raw));
                }
                Err(e) => {
                    assert!(!ok, "{raw}");
                    assert_eq!(e.raw(), raw);
                }
            }
        }
    }

    #[test]
    fn getters_read_what_c_may_store() {
        // A struct C filled may hold an unnamed type, which reads as `None`.
        let mut options = template();
        options.0.oid_type = 3;
        assert_eq!(options.as_ref().oid_type(), None);
        assert_eq!(options.as_ref().to_options().0.oid_type, 3);
    }

    #[test]
    fn handles_reborrow_and_compare() {
        let mut options = template();
        let mut m = options.as_mut();
        m.as_mut().set_oid_type(Some(OidType::Sha1));
        assert_eq!(format!("{m:?}"), format!("{:?}", m.as_ref()));
        let other = template();
        assert_ne!(m.as_ref(), other.as_ref());
        assert_eq!(m.as_ref(), m.as_ref().to_options().as_ref());
        assert_eq!(options.as_ref().oid_type(), Some(OidType::Sha1));
    }

    mod submodule_cache {
        use super::super::*;
        use std::ffi::CString;
        use std::fs;
        use std::os::unix::ffi::OsStrExt;

        use crate::libgit2::repository::repository::Repository;

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn reloading_releases_the_previous_cache() {
            let dir = std::env::temp_dir().join(format!(
                "crustify-sys-repository-unit-{}-reload",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            let mut repo =
                Repository::init(&CString::new(dir.as_os_str().as_bytes()).unwrap(), false)
                    .unwrap();
            fs::write(
                dir.join(".gitmodules"),
                "[submodule \"sub\"]\n\tpath = sub\n\turl = https://example.com/sub\n",
            )
            .unwrap();
            // `git_repository_submodule_cache_all` alone would leak every cache but the
            // last, with the submodules in it (LSan, under the asan-ubsan variant).
            for _ in 0..3 {
                repo.as_mut().cache_submodules().unwrap();
            }
            repo.as_mut().clear_submodule_cache();
            repo.as_mut().clear_submodule_cache();
            repo.as_mut().cache_submodules().unwrap();
            // The repository releases the cache it holds when freed.
            drop(repo);
            fs::remove_dir_all(&dir).unwrap();
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn a_failed_load_leaves_no_cache() {
            let dir = std::env::temp_dir().join(format!(
                "crustify-sys-repository-unit-{}-failed",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            let mut repo =
                Repository::init(&CString::new(dir.as_os_str().as_bytes()).unwrap(), false)
                    .unwrap();
            repo.as_mut().cache_submodules().unwrap();
            fs::write(dir.join(".gitmodules"), "[submodule \"sub\"\n").unwrap();
            let e = repo.as_mut().cache_submodules().unwrap_err();
            assert_eq!(e.raw_code(), -1);
            assert_eq!(
                e.class(),
                Some(crate::libgit2::util::errors::ErrorClass::Config)
            );
            // Lookups read the repository again: the broken `.gitmodules` fails them.
            let (mut sm, raw) = (core::ptr::null_mut(), repo.as_c_ptr());
            // SAFETY: a writable slot, the box's live repository and a NUL-terminated name.
            let rc = unsafe { ffi::git_submodule_lookup(&mut sm, raw, c"sub".as_ptr()) };
            assert!(rc < 0);
            drop(repo);
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    mod installed_parts {
        use super::super::*;

        use crate::libgit2::odb::odb::Odb;
        use crate::libgit2::repository::repository::Repository;

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn odb_oid_type_reads_each_type_and_leaves_the_odb_unowned() {
            for kind in [OidType::Sha1, OidType::Sha256] {
                let odb = Odb::with_oid_type(kind).unwrap();
                assert_eq!(odb_oid_type(&odb).unwrap(), kind);
                // Read again: the scratch repository released its reference.
                assert_eq!(odb_oid_type(&odb).unwrap(), kind);
                // Unowned still, so a repository of the type takes it, and the odb
                // outlives the scratch repositories.
                let raw = odb.as_c_ptr();
                let mut repo = Repository::with_oid_type(kind).unwrap();
                repo.as_mut().set_odb(odb).unwrap();
                assert_eq!(repo.as_ref().odb().unwrap().as_c_ptr(), raw);
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn a_mismatched_odb_is_refused_and_released() {
            let mut repo = Repository::new().unwrap();
            let odb = Odb::with_oid_type(OidType::Sha256).unwrap();
            let error = repo.as_mut().set_odb(odb).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::Generic));
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
            assert_eq!(
                error.message().to_str(),
                Ok("specified object ID type does not match repository object ID type")
            );
            // The repository kept no odb: it has no path to load one from either.
            assert_eq!(repo.as_ref().odb().unwrap_err().raw_code(), -1);
        }
    }
}
