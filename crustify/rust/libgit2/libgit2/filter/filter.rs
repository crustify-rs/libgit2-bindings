//! Safe wrappers over `include/git2/filter.h`.
//!
//! # Filter lists
//!
//! A [`FilterList`] is a `git_filter_list`: the filters (line-ending conversion,
//! `$Id$` expansion, or ones registered through `git2/sys/filter.h`) that apply to one
//! path in one direction, with the state each filter keeps for it. It is opaque:
//! libgit2 publishes no field.
//!
//! ## Ownership
//!
//! libgit2 allocates a list in one block, the path inline, and hands the caller sole
//! ownership; nothing counts or copies it. `git_filter_list_free` releases it: it hands
//! each filter's state (the payload its `check` made) to the filter's `cleanup`, frees
//! the entry array, then the block. So Rust owns one through a [`FilterListBox`],
//! released by [`FilterListFree`], and not `Clone`. The constructors are
//! [`FilterList::new`] (an empty list, `git2/sys/filter.h`) and [`FilterList::load`] /
//! [`FilterList::load_ext`] (the filters a path's attributes select), which give `None`
//! when no filter applies: libgit2 then succeeds with a NULL list, which its routines
//! read as the empty one.
//!
//! ## The repository
//!
//! A list keeps the repository it was built for (`source.repo`), without a reference:
//! applying it hands the repository to each filter, and libgit2's `crlf` filter reads
//! its configuration through it. So a list borrows its repository: `'r` is that
//! borrow, carried by every owner and handle, as `Refdb<'r>` carries its own, and the
//! repository cannot be dropped, nor reached exclusively, while the list lives.
//! Freeing a list does not touch the repository.
//!
//! The list also keeps a pointer to each filter it holds, which stays in libgit2's
//! filter registry until `git_filter_unregister`: the wrapper of that routine must not
//! let a filter go while a list holds it, since the list's free calls its `cleanup`.
//! The constructors pin libgit2's runtime first (`runtime_pinned`), so the registry and
//! its built-in filters outlive every list.
//!
//! ## Threads
//!
//! A list is neither `Send` nor `Sync`: applying it reaches its repository, whose
//! caches libgit2 fills unsynchronised, and its filters update their state in place.
//!
//! libgit2 marks a registered filter initialised the first time a list selects it
//! (`filter_initialize`, writing `fdef->initialized` under the registry's *read* lock,
//! or none from `git_filter_list_push`), so two threads loading their first lists at
//! once race on that flag, whatever repositories they use. The constructors here close
//! that race among themselves for libgit2's built-in filters, `crlf` and `ident`: the
//! first of them initialises both, once, before it loads anything, and the others wait
//! for it. libgit2's other routines that load a list internally
//! (`git_repository_hashfile`, `git_index_add_bypath`, checkout, ...) do not go through
//! that, so their first loads still race with each other and with this module's first
//! one: libgit2's defect, which only initialising the filters when the runtime is pinned
//! would close for the whole crate. A filter registered later through
//! `git2/sys/filter.h` is initialised by the first load that selects it: the wrapper of
//! `git_filter_register` must settle that.
//!
//! ## Raw seams
//!
//! The direction is a raw `git_filter_mode_t` and the flags a raw `git_filter_flag_t`
//! set, until those are wrapped: libgit2 only compares the one and tests bits of the
//! other, so every value is accepted. [`FilterList::load_ext`] takes the options as the
//! raw `git_filter_options`, which it only copies. The blob the forms ending in
//! `_with_blob` take is a raw `git_blob` pointer, until the `object` subsystem's run
//! wraps `git_blob` and replaces them.
//!
//! ## Paths
//!
//! A list is loaded for a path relative to the working directory. libgit2 checks that
//! with an assertion in its attribute lookup, which every load reaches (the built-in
//! filters select by attribute, and cannot be unregistered): a release build fails the
//! load with `GIT_ERROR_INTERNAL`, but a debug build (`_DEBUG`) aborts the process. The
//! loading constructors therefore refuse an absolute path themselves, with the error a
//! release build reports, without calling libgit2.
//!
//! The constructors can fail and set the calling thread's libgit2 error, which they
//! return, copied out, as an [`ErrorBuf`].

use core::ffi::{CStr, c_int};
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};
use std::sync::Once;

use ffibox::{CBorrowedPtr, CBox, CCell, CDrop};

use crate::ffi;
use crate::libgit2::repository::repository::RepositoryRef;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, last};
use crate::libgit2::util::sys::errors::{runtime_pinned, set};

/// A list of filters to apply to one path (`git_filter_list`), built for the
/// repository it borrows for `'r`.
///
/// libgit2 allocates and frees it, so it is only ever reached through a
/// [`FilterListBox`] and its handles. It is opaque: no field is published. See the
/// [module docs](self#filter-lists) for its ownership and its tie to the repository.
///
/// A list has no copy:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::filter::filter::FilterListBox;
///
/// fn copy(list: FilterListBox<'_>) {
///     let _copy = list.clone();
/// }
/// ```
///
/// The exclusive handle is invariant in `'r`, as `&mut` is in its pointee:
///
/// ```compile_fail
/// use libgit2::libgit2::filter::filter::FilterListMut;
///
/// fn shrink<'a, 'r>(m: FilterListMut<'a, 'static>) -> FilterListMut<'a, 'r> {
///     m
/// }
/// ```
///
/// An owner cannot outlive the repository it borrows:
///
/// ```compile_fail
/// use libgit2::libgit2::filter::filter::FilterListBox;
///
/// fn keep<'r>(list: FilterListBox<'r>) -> FilterListBox<'static> {
///     list
/// }
/// ```
///
/// A handle cannot outlive its owner, nor alias an exclusive one:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::filter::filter::{FilterListBox, FilterListRef};
///
/// fn escape(make: impl Fn() -> FilterListBox<'static>) {
///     let r: FilterListRef<'_, 'static>;
///     {
///         let list = make();
///         r = list.as_ref();
///     }
///     let _ = r;
/// }
/// ```
///
/// ```compile_fail,E0499
/// use libgit2::libgit2::filter::filter::FilterListBox;
///
/// fn alias(mut list: FilterListBox<'static>) {
///     let a = list.as_mut();
///     let b = list.as_mut();
///     drop((a, b));
/// }
/// ```
///
/// No list crosses threads:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::filter::filter::FilterListBox;
///
/// fn send<T: Send>(_: T) {}
/// fn check(list: FilterListBox<'static>) {
///     send(list);
/// }
/// ```
///
/// Wraps: git_filter_list
/// Handwritten
#[repr(transparent)]
pub struct FilterList<'r>(
    ffi::git_filter_list,
    PhantomData<(*const (), &'r ffi::git_repository)>,
);

/// Shared borrow of a [`FilterList`]. `Copy`, like `&T`.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct FilterListRef<'a, 'r>(CBorrowedPtr<'a, FilterList<'r>>);

/// Exclusive borrow of a [`FilterList`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// Handwritten
#[repr(transparent)]
pub struct FilterListMut<'a, 'r>(
    FilterListRef<'a, 'r>,
    // Invariant in the list, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut FilterList<'r>>,
);

// SAFETY: `FilterList` is `#[repr(transparent)]` over `git_filter_list` with a
// zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, FilterList<'r>>` (`FilterListMut` through `FilterListRef`) with
// zero-sized markers and no `Drop`, hand out no reference to the list, and
// `FilterListRef` has no write operation. `FilterListMut` is invariant in the list
// through its `PhantomData<&'a mut FilterList<'r>>`.
unsafe impl<'r> CCell for FilterList<'r> {
    type C = ffi::git_filter_list;
    type Ref<'a>
        = FilterListRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = FilterListMut<'a, 'r>
    where
        Self: 'a;
}

impl<'a, 'r> FilterListRef<'a, 'r> {
    /// Borrows the list at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live filter list, built for a repository that
    /// outlives `'r`, which stays live for `'a` and is not written through another
    /// handle meanwhile.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_filter_list) -> Option<Self> {
        // SAFETY: non-NULL per `NonNull::new`; live for `'a` per this function's
        // contract; `FilterList` is transparent over `git_filter_list`.
        NonNull::new(ptr.cast::<FilterList<'r>>()).map(|p| Self(unsafe { CBorrowedPtr::new(p) }))
    }

    /// Read-only pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_filter_list {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'a, 'r> FilterListMut<'a, 'r> {
    /// Borrows the list at `ptr` exclusively; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// As [`FilterListRef::from_ptr`], plus: nothing else reaches the list while the
    /// result lives.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_filter_list) -> Option<Self> {
        // SAFETY: the caller upholds `FilterListRef::from_ptr`'s contract and
        // exclusivity.
        unsafe { FilterListRef::from_ptr(ptr) }.map(|r| Self(r, PhantomData))
    }

    /// Writable pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_filter_list {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> FilterListRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> FilterListMut<'_, 'r> {
        FilterListMut(self.0, PhantomData)
    }
}

impl fmt::Debug for FilterListRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterList").finish_non_exhaustive()
    }
}

impl fmt::Debug for FilterListMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases a filter list through `git_filter_list_free`: each filter's `cleanup`
/// gets the state it keeps for the list, then the entry array and the list are freed.
///
/// The free reads neither the repository nor the path; it reaches each filter the list
/// holds, which the registry keeps (see the [module docs](self#the-repository)).
///
/// Wraps: git_filter_list_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct FilterListFree;

// SAFETY: `git_filter_list_free` releases a live list exactly once, with every state
// it holds: each entry's payload through its filter's `cleanup`, the entry array and
// the block, all from libgit2's allocator. It needs no state beyond the pointer, and no
// repository.
unsafe impl<'r> CDrop<FilterList<'r>> for FilterListFree {
    unsafe fn c_drop(&self, ptr: NonNull<FilterList<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live list whose ownership
        // the caller gives up; `FilterList` is transparent over `git_filter_list`. It
        // was built after the runtime was pinned, so its filters are still registered.
        unsafe { ffi::git_filter_list_free(ptr.as_ptr().cast()) }
    }
}

/// An owned [`FilterList`], released by `git_filter_list_free` on drop.
pub type FilterListBox<'r> = CBox<FilterList<'r>, FilterListFree>;

/// `Ok` for a relative `path`; for an absolute one, the error a release build of
/// libgit2 reports from the assertion in `collect_attr_files`, set as the thread's.
fn relative(path: &CStr) -> Result<(), ErrorBuf> {
    if path.to_bytes().first() == Some(&b'/') {
        set(
            ErrorClass::Internal,
            c"unrecoverable internal error: '!((path)[0] == '/')'",
        );
        return Err(last());
    }
    Ok(())
}

/// The list a loading constructor returning `rc` wrote, `None` for none, or libgit2's
/// error. libgit2 writes no list on failure.
fn loaded<'r>(
    rc: c_int,
    list: Option<FilterListBox<'r>>,
) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
    if rc < 0 {
        drop(list);
        return Err(last());
    }
    Ok(list)
}

/// A blob some caller promised is live for `'b`: what the `_with_blob` raw seams build
/// from their argument, under their `# Safety` contract, and nothing else does.
#[derive(Clone, Copy)]
struct LiveBlob<'b>(NonNull<ffi::git_blob>, PhantomData<&'b ffi::git_blob>);

impl LiveBlob<'_> {
    /// The pointer libgit2 takes: the blob, or NULL for none.
    fn as_ptr(blob: Option<Self>) -> *mut ffi::git_blob {
        blob.map_or(ptr::null_mut(), |b| b.0.as_ptr())
    }
}

impl<'r> FilterList<'r> {
    /// An empty list for `repo`, applying in direction `mode` with `flags`, to which
    /// filters are then added (`git_filter_list_push`, `git2/sys/filter.h`). The list
    /// has no path.
    ///
    /// Fails, with libgit2's out-of-memory error, when the list cannot be allocated.
    ///
    /// The list keeps `repo`, which its filters read when it is applied: it borrows
    /// `repo` for `'r`, and the repository cannot go first:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::ffi;
    /// use libgit2::libgit2::filter::filter::FilterList;
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::new().unwrap();
    /// let list = FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
    /// drop(repo);
    /// drop(list);
    /// ```
    ///
    /// Raw seam: `mode` is a raw `git_filter_mode_t` and `flags` a raw
    /// `git_filter_flag_t` set until those are wrapped; libgit2 accepts every value.
    ///
    /// Wraps: git_filter_list_new
    pub fn new(
        repo: RepositoryRef<'r>,
        mode: ffi::git_filter_mode_t,
        flags: u32,
    ) -> Result<FilterListBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        initialise_builtin_filters();
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, left NULL on failure; `repo` a live
        // repository, which the list keeps for `'r`, the handle's borrow, on this
        // thread, which neither leaves. The runtime is pinned, so a failure is reported
        // on live thread-local state.
        let rc =
            unsafe { ffi::git_filter_list_new(&mut out, repo.as_ptr().cast_mut(), mode, flags) };
        // SAFETY: on success `out` is a fresh list nothing else references, released by
        // `git_filter_list_free` as `FilterListFree` does; on failure it is NULL. Its
        // repository outlives `'r`.
        match loaded(rc, unsafe { FilterListBox::from_c(out) })? {
            Some(list) => Ok(list),
            // A slot left NULL on success would be a libgit2 bug, reported as the
            // thread's error.
            None => Err(last()),
        }
    }

    /// The filters that apply to `path` in `repo`, in direction `mode`, as its
    /// attributes select them; `None` when none does.
    ///
    /// `path` is relative to the working directory, and need not exist; an absolute one
    /// fails with `GIT_ERROR_INTERNAL`, as a release build of libgit2 fails it (see the
    /// [module docs](self#paths)). `flags` choose
    /// where attributes are read from (`GIT_FILTER_NO_SYSTEM_ATTRIBUTES`,
    /// `GIT_FILTER_ATTRIBUTES_FROM_HEAD`) and let a filter ignore unsafe conversions
    /// (`GIT_FILTER_ALLOW_UNSAFE`); `GIT_FILTER_ATTRIBUTES_FROM_COMMIT` reads the commit
    /// only [`load_ext`](Self::load_ext) takes, here the zero id, which libgit2 skips as
    /// missing.
    ///
    /// Fails, with libgit2's error, when the attributes cannot be read (with
    /// `GIT_FILTER_ATTRIBUTES_FROM_HEAD`, when `HEAD` is unborn), or a filter's `check`
    /// fails.
    ///
    /// The list keeps `repo`, as [`new`](Self::new) does:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::ffi;
    /// use libgit2::libgit2::filter::filter::FilterList;
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::open(c".").unwrap();
    /// let list = FilterList::load(repo.as_ref(), c"a.txt", ffi::GIT_FILTER_TO_ODB, 0);
    /// drop(repo);
    /// drop(list);
    /// ```
    ///
    /// Raw seam: `mode` and `flags` as for [`new`](Self::new).
    ///
    /// Wraps: git_filter_list_load
    pub fn load(
        repo: RepositoryRef<'r>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        flags: u32,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        Self::load_from(repo, None, path, mode, flags)
    }

    /// [`load`](Self::load) for the content of `blob`: each filter's `check` sees the
    /// blob's id (`git_filter_source_id`) while it decides whether to apply. The list
    /// keeps neither the blob nor its id, and libgit2's built-in filters do not read it.
    ///
    /// Raw seam: `git_blob` is not wrapped yet; the run wrapping it replaces `blob`
    /// with a blob handle.
    ///
    /// # Safety
    ///
    /// `blob` must address a live `git_blob` for the duration of the call; libgit2
    /// only reads its id, during the call.
    ///
    /// Wraps: git_filter_list_load
    pub unsafe fn load_with_blob(
        repo: RepositoryRef<'r>,
        blob: NonNull<ffi::git_blob>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        flags: u32,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        // The caller promises `blob` is live for the call, per this function's contract.
        let blob = LiveBlob(blob, PhantomData);
        Self::load_from(repo, Some(blob), path, mode, flags)
    }

    /// `git_filter_list_load`, with no blob or one live for the call.
    fn load_from(
        repo: RepositoryRef<'r>,
        blob: Option<LiveBlob<'_>>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        flags: u32,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        relative(path)?;
        initialise_builtin_filters();
        let mut out = ptr::null_mut();
        let (repo, path) = (repo.as_ptr().cast_mut(), path.as_ptr());
        let blob = LiveBlob::as_ptr(blob);
        // SAFETY: `out` is a writable slot, left NULL unless a list is built; `repo` a
        // live repository, whose attribute caches libgit2 fills through this shared
        // handle on this thread only, and which the list keeps for `'r`; `blob` NULL or
        // live for the call, as a `LiveBlob` is, its id read for the filters' checks;
        // `path` NUL-terminated, copied. The runtime is pinned: the filter registry and its lock exist, and a
        // failure is reported on live thread-local state.
        let rc = unsafe { ffi::git_filter_list_load(&mut out, repo, blob, path, mode, flags) };
        // SAFETY: `out` is NULL (no filter applies, or a failure), or a fresh list
        // nothing else references, released by `git_filter_list_free` as
        // `FilterListFree` does. Its repository outlives `'r`.
        loaded(rc, unsafe { FilterListBox::from_c(out) })
    }

    /// [`load`](Self::load) with options: their `flags`, and the commit whose
    /// `.gitattributes` `GIT_FILTER_ATTRIBUTES_FROM_COMMIT` reads (`attr_commit_id`);
    /// `None` uses the defaults, no flags. libgit2 copies the options, and reads neither
    /// their `version` nor their `reserved` pointer.
    ///
    /// Raw seam: `opts` is the raw `git_filter_options` until it is wrapped, and `mode`
    /// as for [`new`](Self::new).
    ///
    /// Wraps: git_filter_list_load_ext
    pub fn load_ext(
        repo: RepositoryRef<'r>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        opts: Option<&ffi::git_filter_options>,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        Self::load_ext_from(repo, None, path, mode, opts)
    }

    /// [`load_ext`](Self::load_ext) for the content of `blob`, as
    /// [`load_with_blob`](Self::load_with_blob) is for [`load`](Self::load).
    ///
    /// Raw seam: as for [`load_with_blob`](Self::load_with_blob).
    ///
    /// # Safety
    ///
    /// As for [`load_with_blob`](Self::load_with_blob): `blob` must address a live
    /// `git_blob` for the duration of the call.
    ///
    /// Wraps: git_filter_list_load_ext
    pub unsafe fn load_ext_with_blob(
        repo: RepositoryRef<'r>,
        blob: NonNull<ffi::git_blob>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        opts: Option<&ffi::git_filter_options>,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        // The caller promises `blob` is live for the call, per this function's contract.
        let blob = LiveBlob(blob, PhantomData);
        Self::load_ext_from(repo, Some(blob), path, mode, opts)
    }

    /// `git_filter_list_load_ext`, with no blob or one live for the call.
    fn load_ext_from(
        repo: RepositoryRef<'r>,
        blob: Option<LiveBlob<'_>>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        opts: Option<&ffi::git_filter_options>,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        relative(path)?;
        initialise_builtin_filters();
        let mut out = ptr::null_mut();
        let (repo, path) = (repo.as_ptr().cast_mut(), path.as_ptr());
        let blob = LiveBlob::as_ptr(blob);
        // libgit2 takes the options mutably but only copies them by value.
        let opts = opts.map_or(ptr::null_mut(), |o| ptr::from_ref(o).cast_mut());
        // SAFETY: as in `load_from`; `opts` is NULL or a live `git_filter_options`, only
        // read (copied into the session) for the call: libgit2 is built with
        // `GIT_DEPRECATE_HARD`, so the `reserved` pointer is never read.
        let rc = unsafe { ffi::git_filter_list_load_ext(&mut out, repo, blob, path, mode, opts) };
        // SAFETY: as in `load_from`.
        loaded(rc, unsafe { FilterListBox::from_c(out) })
    }
}

/// Marks libgit2's built-in filters (`crlf`, `ident`) initialised, once per process:
/// pushes each onto a scratch list, which runs `filter_initialize` for it, then frees the
/// list.
///
/// libgit2 initialises a filter, unsynchronised, the first time a list selects it (see
/// the [module docs](self#threads)). Every constructor calls this, with the runtime
/// pinned, before it loads a list: the first call does the work, and the others wait for
/// it, so the flag is written before any of their loads reads it. A failure (an
/// allocation) leaves the filters to their first load, as libgit2 would.
fn initialise_builtin_filters() {
    static DONE: Once = Once::new();
    DONE.call_once(|| {
        let mut list = ptr::null_mut();
        // SAFETY: `list` is a writable slot, left NULL on failure; no repository, which
        // the list only keeps, and it is never applied. The caller pinned the runtime.
        let rc = unsafe {
            ffi::git_filter_list_new(&mut list, ptr::null_mut(), ffi::GIT_FILTER_TO_ODB, 0)
        };
        if rc < 0 || list.is_null() {
            return;
        }
        for name in [c"crlf", c"ident"] {
            // SAFETY: a NUL-terminated name; libgit2 looks it up under its registry lock
            // and returns the registered filter, which the registry keeps, or NULL.
            let filter = unsafe { ffi::git_filter_lookup(name.as_ptr()) };
            if !filter.is_null() {
                // SAFETY: the live scratch list and a registered filter, with no payload;
                // libgit2 initialises the filter and appends an entry, or fails without
                // one. No constructor here loads meanwhile: they wait on `DONE`.
                unsafe { ffi::git_filter_list_push(list, filter, ptr::null_mut()) };
            }
        }
        // SAFETY: the scratch list, released once; its entries have no payload, which
        // the built-in filters' `cleanup` accepts.
        unsafe { ffi::git_filter_list_free(list) };
    });
}

// The layout type is the C type's size, so a list pointer converts both ways.
const _: () = assert!(size_of::<FilterList<'static>>() == size_of::<ffi::git_filter_list>());
const _: () =
    assert!(size_of::<FilterListBox<'static>>() == size_of::<*mut ffi::git_filter_list>());

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::repository::repository::{Repository, RepositoryBox};
    use core::mem::size_of;
    use std::ffi::CString;
    use std::path::PathBuf;

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process: libgit2 initialises a filter,
    /// unsynchronised, the first time any list selects it, and other modules' tests load
    /// lists through raw calls the crate cannot order (see the module docs).
    fn isolated(test: &str, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
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

    /// A fresh repository with a working directory whose `.gitattributes` gives `*.txt`
    /// the `crlf` filter and `*.id` the `ident` one; removed on drop.
    struct Workdir {
        path: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl Workdir {
        fn new(tag: &str) -> Self {
            assert!(runtime_pinned());
            let path = std::env::temp_dir()
                .join(format!("crustify-filter-unit-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            let c_path = CString::new(path.to_str().unwrap()).unwrap();
            let repo = Repository::init(&c_path, false).unwrap();
            std::fs::write(
                path.join(".gitattributes"),
                "*.txt text eol=crlf\n*.id ident\n",
            )
            .unwrap();
            Self {
                path,
                repo: Some(repo),
            }
        }

        fn repo(&self) -> RepositoryRef<'_> {
            self.repo.as_ref().unwrap().as_ref()
        }
    }

    impl Drop for Workdir {
        fn drop(&mut self) {
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn length(list: &FilterListBox<'_>) -> usize {
        // SAFETY: a live list; libgit2 only reads its entry count.
        unsafe { ffi::git_filter_list_length(list.as_ref().as_ptr()) }
    }

    #[test]
    fn layout_is_one_pointer() {
        assert_eq!(
            size_of::<FilterList<'_>>(),
            size_of::<ffi::git_filter_list>()
        );
        assert_eq!(
            size_of::<FilterListBox<'_>>(),
            size_of::<*mut ffi::git_filter_list>()
        );
        assert_eq!(
            size_of::<Option<FilterListBox<'_>>>(),
            size_of::<*mut ffi::git_filter_list>()
        );
        assert_eq!(
            size_of::<FilterListRef<'_, '_>>(),
            size_of::<*const ffi::git_filter_list>()
        );
        assert_eq!(
            size_of::<FilterListMut<'_, '_>>(),
            size_of::<*mut ffi::git_filter_list>()
        );
        assert_eq!(size_of::<FilterListFree>(), 0);
    }

    /// The handles over Rust storage, so Miri checks them: the list is opaque, so they
    /// only carry the pointer, reborrow and print; nothing is freed.
    #[test]
    fn handles_without_c() {
        let mut storage = 0u64;
        let raw: *mut ffi::git_filter_list = ptr::from_mut(&mut storage).cast();
        // SAFETY: NULL is refused.
        assert!(unsafe { FilterListRef::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: NULL is refused.
        assert!(unsafe { FilterListMut::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: `raw` addresses live storage nothing else reaches while the handle
        // lives; the opaque list is never read through it.
        let mut m = unsafe { FilterListMut::<'_, 'static>::from_ptr(raw) }.unwrap();
        assert_eq!(m.as_mut_ptr(), raw);
        assert_eq!(m.as_ref().as_ptr(), raw.cast_const());
        let mut inner = m.as_mut();
        assert_eq!(inner.as_mut_ptr(), raw);
        assert_eq!(format!("{inner:?}"), "FilterList { .. }");
        let r = m.as_ref();
        let copy = r;
        assert_eq!(format!("{r:?}"), format!("{copy:?}"));
        assert_eq!(format!("{m:?}"), "FilterList { .. }");
        // SAFETY: as above, shared.
        let shared = unsafe { FilterListRef::<'_, 'static>::from_ptr(raw) }.unwrap();
        assert_eq!(shared.as_ptr(), raw.cast_const());
        assert_eq!(storage, 0);
    }

    /// `loaded` reports the thread's error for a failing code, and passes a list or the
    /// lack of one through otherwise.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn loaded_reports_failures() {
        isolated(concat!(module_path!(), "::loaded_reports_failures"), || {
            assert!(runtime_pinned());
            set(ErrorClass::Filter, c"refused here");
            let error = loaded(-1, None).unwrap_err();
            assert_eq!(error.class(), Some(ErrorClass::Filter));
            assert_eq!(error.message(), c"refused here");
            assert!(loaded(0, None).unwrap().is_none());
            let wd = Workdir::new("loaded");
            let list = FilterList::new(wd.repo(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
            assert_eq!(length(&loaded(0, Some(list)).unwrap().unwrap()), 0);
        });
    }

    /// The corrected divergence: libgit2 asserts the path is relative, which aborts a
    /// debug build; the wrappers refuse an absolute path before calling it, with the
    /// release build's error, for every loading form, and still load relative ones.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn absolute_paths_are_refused_before_c() {
        isolated(
            concat!(module_path!(), "::absolute_paths_are_refused_before_c"),
            || {
                let wd = Workdir::new("absolute");
                let expect = |r: Result<Option<FilterListBox<'_>>, ErrorBuf>| {
                    let error = r.map(|_| ()).unwrap_err();
                    assert_eq!(error.class(), Some(ErrorClass::Internal));
                    assert_eq!(
                        error.message(),
                        c"unrecoverable internal error: '!((path)[0] == '/')'"
                    );
                };
                for path in [c"/", c"/a.txt", c"//a.id"] {
                    expect(FilterList::load(
                        wd.repo(),
                        path,
                        ffi::GIT_FILTER_TO_WORKTREE,
                        0,
                    ));
                    expect(FilterList::load_ext(
                        wd.repo(),
                        path,
                        ffi::GIT_FILTER_TO_ODB,
                        None,
                    ));
                }
                // Not absolute: a leading `.` or a `/` further in.
                for path in [c"./a.txt", c"sub/a.txt", c"a.txt/"] {
                    assert!(
                        FilterList::load(wd.repo(), path, ffi::GIT_FILTER_TO_WORKTREE, 0).is_ok()
                    );
                }
            },
        );
    }

    /// The built-in filters are initialised once, before the first load; calling it again
    /// does nothing, and lists still select them.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn builtin_filters_initialise_once() {
        isolated(
            concat!(module_path!(), "::builtin_filters_initialise_once"),
            || {
                assert!(runtime_pinned());
                initialise_builtin_filters();
                initialise_builtin_filters();
                let wd = Workdir::new("builtins");
                for path in [c"a.txt", c"a.id"] {
                    let list = FilterList::load(wd.repo(), path, ffi::GIT_FILTER_TO_WORKTREE, 0)
                        .unwrap()
                        .unwrap();
                    assert_eq!(length(&list), 1);
                }
            },
        );
    }

    /// The free policy, driven by hand and through the box, releases lists with filter
    /// state (`crlf` keeps a payload its `cleanup` frees) once each (LSan, ASan); a list
    /// given back to C with `into_c` is freed by `git_filter_list_free`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn free_policy_and_c_release_lists() {
        isolated(
            concat!(module_path!(), "::free_policy_and_c_release_lists"),
            || {
                let wd = Workdir::new("free");
                let list = FilterList::load(wd.repo(), c"a.txt", ffi::GIT_FILTER_TO_WORKTREE, 0)
                    .unwrap()
                    .unwrap();
                assert_eq!(length(&list), 1);
                let raw = list.into_c();
                // SAFETY: `raw` is the live list given up above, released once.
                unsafe {
                    FilterListFree.c_drop(NonNull::new(raw).unwrap().cast::<FilterList<'_>>())
                };

                let list = FilterList::load(wd.repo(), c"a.id", ffi::GIT_FILTER_TO_ODB, 0)
                    .unwrap()
                    .unwrap();
                let raw = list.into_c();
                // SAFETY: the live list given up above, released once by C.
                unsafe { ffi::git_filter_list_free(raw) };

                let mut out = ptr::null_mut();
                // SAFETY: a writable slot, the live repository and a NUL-terminated path.
                let rc = unsafe {
                    ffi::git_filter_list_load(
                        &mut out,
                        wd.repo().as_ptr().cast_mut(),
                        ptr::null_mut(),
                        c"b.txt".as_ptr(),
                        ffi::GIT_FILTER_TO_ODB,
                        0,
                    )
                };
                assert_eq!(rc, 0);
                // SAFETY: the fresh list C built for `wd`'s repository, which outlives it.
                let adopted = unsafe { FilterListBox::from_c(out) }.unwrap();
                assert_eq!(length(&adopted), 1);
                drop(adopted);
            },
        );
    }
}
