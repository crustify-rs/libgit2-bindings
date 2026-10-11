//! Safe wrappers over `include/git2/index.h`.
//!
//! An [`Index`] is a `git_index`: the staging area, a sorted list of entries plus the
//! conflict (`REUC`, `NAME`) and tree-cache extensions, optionally backed by an index
//! file on disk. The public headers declare it opaque, so Rust reaches it only behind a
//! pointer and only through libgit2's routines; it has no field accessors.
//!
//! # Ownership
//!
//! libgit2 counts references to an index (`GIT_REFCOUNT_*` over its private `rc`
//! field) and frees it when the last one is released by `git_index_free`, the down-ref.
//! It publishes no up_ref, no copy and no way to read the count, so Rust tells the two
//! kinds of reference apart by where they came from:
//!
//! | reference | from | owner | handles |
//! |---|---|---|---|
//! | the only one | [`Index::new`], [`Index::open`] and their `_with_oid_type` forms; later the merge, cherry-pick, revert and apply routines that build a fresh index | [`IndexBox`] | [`IndexRef`], [`IndexMut`] |
//! | one of several | an [`IndexBox`] converted with `.into()`; later `git_repository_index` and `git_rebase_inmemory_index`, which count one more reference to an index the repository or rebase keeps | [`IndexArc`] | [`IndexRef`] |
//!
//! Both owners release their reference through [`IndexFree`]. An [`IndexArc`] has no
//! `Clone` (there is no up_ref to count a copy with) and no exclusive handle: nothing
//! proves its reference is the only one, and another holder may change the index
//! behind it, invalidating whatever an [`IndexRef`] borrowed from it.
//!
//! An [`IndexBox`] stays the only reference only while every routine handed its pointer
//! returns without keeping one. libgit2's iterators, index writers and
//! `git_checkout_index` count a reference for the duration of a call or of a borrow
//! tied to the handle; `git_repository_set_index` keeps one for the repository. A
//! wrapper of such a retaining routine must take an [`IndexArc`] (into which the box
//! converts), never an [`IndexMut`] or [`IndexRef`] of a box.
//!
//! `git_index_owner` lends the repository that keeps a reference to the index, if any,
//! without counting it: libgit2 clears it when that repository releases the index.
//!
//! # Threads
//!
//! libgit2 synchronises only the reference count: two threads must not use one index
//! at once, and a shared index's other holders are not Rust values whose access this
//! crate controls. So neither the index nor its owners and handles are `Send` or
//! `Sync`:
//!
//! ```compile_fail,E0277
//! use libgit2::libgit2::index::index::Index;
//!
//! let index = Index::new().unwrap();
//! std::thread::spawn(move || drop(index));
//! ```

use core::ffi::{CStr, c_int};
use core::ptr::{self, NonNull};

use ffibox::{CArc, CBox, CNew, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::oid::oid::OidType;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, last};
use crate::libgit2::util::sys::errors::{runtime_pinned, set};

define_ctype!(
    /// A git index (`git_index`): the staging area, its entries and extensions,
    /// optionally backed by an index file.
    ///
    /// Opaque, and only ever behind a pointer (see the [module docs](self#ownership)):
    /// owned as an [`IndexBox`], which [`new`](Self::new), [`open`](Self::open) and
    /// their `_with_oid_type` forms build, or as an [`IndexArc`] when the reference is
    /// shared; borrowed as an [`IndexRef`] / [`IndexMut`] from either.
    ///
    /// There is no value to hold: the layout is libgit2's own.
    ///
    /// ```compile_fail,E0423
    /// use libgit2::libgit2::index::index::Index;
    ///
    /// let _ = Index(Default::default(), core::marker::PhantomData);
    /// ```
    ///
    /// Wraps: git_index
    Index, IndexRef, IndexMut, ffi::git_index
);

/// Releases one counted reference to an index through `git_index_free`, which frees the
/// index with its entries, extensions and path once no reference is left and no
/// repository keeps it; builds a fresh in-memory index through `git_index_new`.
///
/// `git_index_free` is the only releaser for every reference libgit2 hands out, so the
/// policy settles both [`IndexBox`] and [`IndexArc`]. It counts no copies: libgit2
/// publishes no up_ref.
///
/// Wraps: git_index_free
/// Wraps: git_index_new
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct IndexFree;

impl_cdrop!(IndexFree, Index, ffi::git_index_free);

// SAFETY: `Index::new` returns a fresh index from `git_index_new`, fully initialised,
// holding its only reference (count 1, no owning repository), which `git_index_free`
// releases; it fails only when libgit2 does (out of memory, or it cannot be
// initialised).
unsafe impl CNew<Index> for IndexFree {
    fn c_new(&self) -> Option<NonNull<Index>> {
        Index::new()
            .ok()
            .and_then(|index| NonNull::new(index.into_raw()))
    }
}

/// The only reference to an [`Index`], released by `git_index_free` on drop; the owner
/// with an [`IndexMut`]. Converts into an [`IndexArc`] with `.into()`.
///
/// Its handles borrow it, so neither outlives it:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::index::index::Index;
///
/// let index = Index::new().unwrap();
/// let handle = index.as_ref();
/// drop(index);
/// let _ = handle;
/// ```
///
/// and its exclusive handle is exclusive:
///
/// ```compile_fail,E0499
/// use libgit2::libgit2::index::index::Index;
///
/// let mut index = Index::new().unwrap();
/// let first = index.as_mut();
/// let second = index.as_mut();
/// let _ = (first, second);
/// ```
///
/// It has no `Clone`: libgit2 publishes no copy routine.
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::index::index::Index;
///
/// let _ = Index::new().unwrap().clone();
/// ```
pub type IndexBox = CBox<Index, IndexFree>;

/// One of several counted references to an [`Index`], released by `git_index_free` on
/// drop. Shared, so it hands out only [`IndexRef`]s, and has no `Clone`: libgit2
/// publishes no up_ref to count a copy with.
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::index::index::{Index, IndexArc};
///
/// let mut index: IndexArc = Index::new().unwrap().into();
/// let _ = index.get_mut();
/// ```
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::index::index::{Index, IndexArc};
///
/// let index: IndexArc = Index::new().unwrap().into();
/// let _ = index.clone();
/// ```
///
/// Nor does it convert back into an [`IndexBox`]: nothing proves its reference is the
/// only one.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::index::index::{Index, IndexArc, IndexBox};
///
/// let index: IndexArc = Index::new().unwrap().into();
/// let _ = IndexBox::try_from(index);
/// ```
pub type IndexArc = CArc<Index, IndexFree>;

/// The index a constructor returning `rc` handed out, or libgit2's error.
fn constructed(rc: c_int, index: Option<IndexBox>) -> Result<IndexBox, ErrorBuf> {
    index.filter(|_| rc >= 0).ok_or_else(last)
}

/// The options `git_index_new_ext` / `git_index_open_ext` read: this version's struct
/// with `oid_type`, the only setting it has.
fn options(oid_type: OidType) -> ffi::git_index_options {
    ffi::git_index_options {
        version: ffi::GIT_INDEX_OPTIONS_VERSION,
        oid_type: oid_type.raw(),
    }
}

/// Refuses an empty path, with a `GIT_ERROR_INDEX` error, before libgit2 sees it: see
/// [`Index::open`].
fn path_is_usable(path: &CStr) -> bool {
    if path.is_empty() {
        set(
            ErrorClass::Index,
            c"failed to open index: the path is empty",
        );
        return false;
    }
    true
}

impl Index {
    /// A fresh, empty in-memory index, backed by no file and no repository: libgit2
    /// refuses to read or write it, and fails the routines that need a repository's
    /// object database or working directory (adding by path, writing a tree), but every
    /// in-memory operation works on it. Its object ids are SHA1
    /// ([`with_oid_type`](Self::with_oid_type) chooses).
    ///
    /// Fails, with libgit2's error, only when it runs out of memory or cannot be
    /// initialised. [`IndexBox::new`](CBox::new) and [`IndexArc::new`](CArc::new) build
    /// the same index, reporting failure as `None`.
    ///
    /// Wraps: git_index_new
    pub fn new() -> Result<IndexBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, which libgit2 writes only on success. The
        // runtime is pinned, so the allocation reads a settled allocator and a failure is
        // reported on live thread-local state.
        let rc = unsafe { ffi::git_index_new(&mut out) };
        // SAFETY: `git_index_new` leaves `out` NULL on failure (adopted as `None`), or
        // writes a fresh index holding its only reference, which the caller owns, released
        // by `git_index_free` as `IndexFree` does.
        constructed(rc, unsafe { IndexBox::from_c(out) })
    }

    /// An index backed by the index file at `path`, without a repository; its contents
    /// are read from the file when it exists, and the index is empty when it does not
    /// (nor is the file created until the index is written). Its object ids are SHA1
    /// ([`open_with_oid_type`](Self::open_with_oid_type) chooses).
    ///
    /// The path is copied, not retained. The routines that need a repository fail on the
    /// index as on [`new`](Self::new)'s.
    ///
    /// An empty `path` is refused, with a `GIT_ERROR_INDEX` error, before libgit2 sees
    /// it. libgit2 accepts it, but writing the index it then builds hands `memcpy` a NULL
    /// source (`git_filebuf_open_withsize`, `src/util/filebuf.c`), which is undefined
    /// behaviour.
    ///
    /// Fails, with libgit2's error, when the file exists but cannot be read (a directory,
    /// say) or is not a valid index, and when libgit2 runs out of memory or cannot be
    /// initialised; libgit2 reports each of these as its generic error (`-1`).
    ///
    /// libgit2 leaks a few bytes when it refuses a version 4 file whose last entry runs
    /// past its end (a truncated file, say): its reader returns before freeing the path
    /// it decompressed for that entry.
    ///
    /// Wraps: git_index_open
    pub fn open(path: &CStr) -> Result<IndexBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if !path_is_usable(path) {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, which libgit2 writes only on success; `path` is
        // a non-NULL NUL-terminated string, read and copied for the call, never retained.
        // The runtime is pinned.
        let rc = unsafe { ffi::git_index_open(&mut out, path.as_ptr()) };
        // SAFETY: `git_index_open` leaves `out` NULL on failure (freeing the half-built
        // index itself; adopted as `None`), or writes a fresh index holding its only
        // reference, which the caller owns, released by `git_index_free` as `IndexFree`
        // does.
        constructed(rc, unsafe { IndexBox::from_c(out) })
    }

    /// As [`new`](Self::new), for an index whose object ids are of `oid_type`; the type
    /// is the only setting `git_index_options` holds. [`new`](Self::new) is this with
    /// [`OidType::Sha1`].
    ///
    /// libgit2 asserts the type is one it names, which [`OidType`] guarantees.
    ///
    /// Wraps: git_index_new_ext
    pub fn with_oid_type(oid_type: OidType) -> Result<IndexBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let opts = options(oid_type);
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, which libgit2 writes only on success; `opts`
        // is a live options struct of this version naming a valid type, which the
        // routine reads for the call and does not retain. The runtime is pinned.
        let rc = unsafe { ffi::git_index_new_ext(&mut out, &opts) };
        // SAFETY: as in `new`: `out` is NULL, or a fresh index holding its only
        // reference, released by `git_index_free` as `IndexFree` does.
        constructed(rc, unsafe { IndexBox::from_c(out) })
    }

    /// As [`open`](Self::open), for an index whose object ids are of `oid_type`: an
    /// existing file is read as an index of that type, and fails to read as one of
    /// another. [`open`](Self::open) is this with [`OidType::Sha1`]; the type is the
    /// only setting `git_index_options` holds, and libgit2 asserts it is one it names,
    /// which [`OidType`] guarantees.
    ///
    /// An empty `path` is refused as [`open`](Self::open) refuses it.
    ///
    /// Wraps: git_index_open_ext
    pub fn open_with_oid_type(path: &CStr, oid_type: OidType) -> Result<IndexBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if !path_is_usable(path) {
            return Err(last());
        }
        let opts = options(oid_type);
        let mut out = ptr::null_mut();
        // SAFETY: as in `open`; `opts` is a live options struct of this version naming a
        // valid type, which the routine reads for the call and does not retain.
        let rc = unsafe { ffi::git_index_open_ext(&mut out, path.as_ptr(), &opts) };
        // SAFETY: as in `open`.
        constructed(rc, unsafe { IndexBox::from_c(out) })
    }
}

/// Flags for adding files that match a pathspec.
///
/// These are copied integers with no foreign storage or lifecycle. Unknown bits
/// are retained, and combinations are passed to C unchanged. In particular, C's
/// workdir-diff implementation includes ignored files only when the entire flags
/// value equals `FORCE`, not when `FORCE` is combined with another flag.
///
/// Wraps: git_index_add_option_t
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AddFlags(ffi::git_index_add_option_t);

impl AddFlags {
    /// Normal addition, without any flags.
    pub const DEFAULT: Self = Self(ffi::GIT_INDEX_ADD_DEFAULT);
    /// Request adding ignored files.
    pub const FORCE: Self = Self(ffi::GIT_INDEX_ADD_FORCE);
    /// Request literal pathspec matching instead of wildcard matching.
    pub const DISABLE_PATHSPEC_MATCH: Self = Self(ffi::GIT_INDEX_ADD_DISABLE_PATHSPEC_MATCH);
    /// Check for explicitly named ignored files, unless `FORCE` is also set.
    pub const CHECK_PATHSPEC: Self = Self(ffi::GIT_INDEX_ADD_CHECK_PATHSPEC);

    /// An empty flag set, equivalent to `DEFAULT`.
    pub const fn empty() -> Self {
        Self::DEFAULT
    }

    /// Retains all bits, including those not named by this version of libgit2.
    pub const fn from_bits_retain(bits: core::ffi::c_uint) -> Self {
        Self(bits)
    }

    /// The unchanged unsigned flags for the C API.
    pub const fn bits(self) -> core::ffi::c_uint {
        self.0
    }

    /// Whether every bit in `other` is present (true for the empty set).
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Combines two sets without normalizing or discarding any bits.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Removes every bit present in `other`.
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl core::ops::BitOr for AddFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for AddFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Explicit filesystem capabilities affecting index actions.
///
/// Only the three published capability bits are accepted. The C value `-1` is
/// not a set of capabilities: use [`CapabilitySetting::FromOwner`] to request
/// reading the owning repository's configuration instead. Values own no foreign
/// resources and may be freely copied, shared or moved between threads.
///
/// Wraps: git_index_capability_t
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Capabilities(ffi::git_index_capability_t);

impl Capabilities {
    /// Match index paths case-insensitively.
    pub const IGNORE_CASE: Self = Self(ffi::GIT_INDEX_CAPABILITY_IGNORE_CASE);
    /// Do not trust filesystem executable-mode bits.
    pub const NO_FILEMODE: Self = Self(ffi::GIT_INDEX_CAPABILITY_NO_FILEMODE);
    /// The filesystem does not support symbolic links.
    pub const NO_SYMLINKS: Self = Self(ffi::GIT_INDEX_CAPABILITY_NO_SYMLINKS);

    const KNOWN: c_int = Self::IGNORE_CASE.0 | Self::NO_FILEMODE.0 | Self::NO_SYMLINKS.0;

    /// No capability overrides.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// All three explicit capabilities, not the `FROM_OWNER` command.
    pub const fn all() -> Self {
        Self(Self::KNOWN)
    }

    /// Checks that `bits` contains only explicit, published capability bits.
    /// Returns `None` for `FROM_OWNER` and for any unknown bit.
    pub const fn from_bits(bits: c_int) -> Option<Self> {
        if bits & !Self::KNOWN == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// The explicit capability bits, suitable for the C API.
    pub const fn bits(self) -> c_int {
        self.0
    }

    /// Whether every bit in `other` is present (true for the empty set).
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Combines two explicit sets; cannot produce the `FROM_OWNER` command.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Removes every bit present in `other`.
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl core::ops::BitOr for Capabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for Capabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Selects either an explicit capability set or the owning repository's config.
///
/// This Rust enum is not passed across FFI: [`to_raw`](Self::to_raw) supplies the
/// signed C value. `FromOwner` requires an index with an owning repository; C
/// reports an error otherwise. Unlike explicit sets, this command cannot be ORed:
///
/// ```compile_fail
/// use libgit2::libgit2::index::index::{Capabilities, CapabilitySetting};
/// let _ = CapabilitySetting::FromOwner | Capabilities::IGNORE_CASE;
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CapabilitySetting {
    /// Use exactly this explicit set.
    Explicit(Capabilities),
    /// Read `core.ignorecase`, `core.filemode` and `core.symlinks` from the owner.
    FromOwner,
}

impl CapabilitySetting {
    /// The signed C argument; only `FromOwner` produces the sentinel `-1`.
    pub const fn to_raw(self) -> c_int {
        match self {
            Self::Explicit(caps) => caps.bits(),
            Self::FromOwner => ffi::GIT_INDEX_CAPABILITY_FROM_OWNER,
        }
    }
}

impl From<Capabilities> for CapabilitySetting {
    fn from(caps: Capabilities) -> Self {
        Self::Explicit(caps)
    }
}

/// The result of a matched-path callback: zero applies the operation, a positive
/// code skips this path, and a negative code aborts the scan. Negative codes are
/// returned unchanged by libgit2; positive codes are consumed by the scan.
///
/// All C `int` values are valid, including application-specific abort codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MatchedPathResult(c_int);

impl MatchedPathResult {
    /// Apply the requested add, remove, or update to this path.
    pub const APPLY: Self = Self(0);
    /// Leave this path alone, and continue scanning.
    pub const SKIP: Self = Self(1);
    /// Abort with libgit2's user-callback error (`GIT_EUSER`).
    pub const ABORT: Self = Self(ffi::GIT_EUSER);

    /// Preserve an arbitrary C callback result without restricting its magnitude.
    pub const fn from_code(code: c_int) -> Self {
        Self(code)
    }

    /// The exact code returned to libgit2.
    pub const fn code(self) -> c_int {
        self.0
    }
}

type MatchedPathFn<'f> = dyn FnMut(&CStr, Option<&CStr>) -> MatchedPathResult + 'f;

/// A synchronous add/remove/update callback borrowing its closure exclusively.
///
/// `path` is the index/workdir path. `matched_pathspec` is the parsed pattern
/// matching it, or `None` when no patterns were supplied. Both are immutable,
/// potentially non-UTF-8 strings, lent only for this invocation. Copy a string
/// to keep it: C may delete its storage immediately after the callback returns.
/// Neither string nor the payload transfers ownership, in any of the C callers.
///
/// A panic aborts the scan with `GIT_EUSER` and resumes after C returns and
/// cleans up. Each new invocation/scan resets the stop state. The handle cannot
/// be sent or shared across threads; its exclusive borrow prevents same-handle
/// reentry, while calling a different callback is allowed.
///
/// Neither argument may escape the callback:
/// ```compile_fail
/// use std::ffi::CStr;
/// use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
/// let mut kept = None;
/// let mut f = |path: &CStr, _: Option<&CStr>| {
///     kept = Some(path);
///     MatchedPathResult::APPLY
/// };
/// let _ = MatchedPathCallback::new(&mut f);
/// ```
/// ```compile_fail
/// use std::ffi::CStr;
/// use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
/// let mut kept = None;
/// let mut f = |_: &CStr, pattern: Option<&CStr>| {
///     kept = pattern;
///     MatchedPathResult::APPLY
/// };
/// let _ = MatchedPathCallback::new(&mut f);
/// ```
/// The closure and its captured state must outlive the handle:
/// ```compile_fail
/// use std::ffi::CStr;
/// use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
/// let mut owned = Vec::new();
/// let mut f = move |_: &CStr, _: Option<&CStr>| {
///     owned.push(1);
///     MatchedPathResult::APPLY
/// };
/// let mut callback = MatchedPathCallback::new(&mut f);
/// drop(f);
/// callback.call(c"file", None);
/// ```
/// Exclusive calls cannot overlap:
/// ```compile_fail
/// use std::ffi::CStr;
/// use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
/// let mut f = |_: &CStr, _: Option<&CStr>| MatchedPathResult::APPLY;
/// let mut callback = MatchedPathCallback::new(&mut f);
/// let first = &mut callback;
/// callback.call(c"file", None);
/// first.call(c"file", None);
/// ```
/// ```compile_fail
/// use std::ffi::CStr;
/// use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
/// let mut f = |_: &CStr, _: Option<&CStr>| MatchedPathResult::APPLY;
/// let callback = MatchedPathCallback::new(&mut f);
/// std::thread::scope(|s| { s.spawn(move || drop(callback)); });
/// ```
///
/// Wraps: git_index_matched_path_cb
/// Handwritten
pub struct MatchedPathCallback<'f> {
    f: &'f mut MatchedPathFn<'f>,
    stopped: Option<c_int>,
    panic: Option<Box<dyn std::any::Any + Send>>,
}

impl<'f> MatchedPathCallback<'f> {
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(&CStr, Option<&CStr>) -> MatchedPathResult + 'f,
    {
        Self {
            f,
            stopped: None,
            panic: None,
        }
    }

    /// Invoke through the same trampoline C uses. Any panic resumes here, never
    /// across the C ABI. The input borrows last only for the closure's call.
    pub fn call(&mut self, path: &CStr, matched_pathspec: Option<&CStr>) -> MatchedPathResult {
        let code = self.scan_with(|callback, payload| {
            // SAFETY: scan_with supplies this live, exclusively borrowed handle;
            // both string pointers are immutable and valid through the call.
            unsafe {
                callback.expect("the trampoline is set")(
                    path.as_ptr(),
                    matched_pathspec.map_or(ptr::null(), CStr::as_ptr),
                    payload,
                )
            }
        });
        MatchedPathResult::from_code(code)
    }

    /// The negative callback code that stopped the last call/scan, including
    /// `GIT_EUSER` for a panic. A skipped path does not stop the scan.
    pub fn stopped(&self) -> Option<c_int> {
        self.stopped
    }

    /// Internal seam for the higher-layer add/remove/update wrappers. `scan`
    /// must use the pair synchronously on this thread, never retain it or invoke
    /// it recursively, and keep both input strings immutable for each call.
    /// No Rust reborrow of this handle occurs while C holds the payload pointer.
    pub(crate) fn scan_with<R>(
        &mut self,
        scan: impl FnOnce(ffi::git_index_matched_path_cb, *mut core::ffi::c_void) -> R,
    ) -> R {
        self.stopped = None;
        let result = scan(Some(matched_path_with), ptr::from_mut(self).cast());
        if let Some(panic) = self.panic.take() {
            std::panic::resume_unwind(panic);
        }
        result
    }
}

/// # Safety
/// `payload` is a live exclusively used MatchedPathCallback on this thread.
/// `path` is a non-null immutable NUL string; `pattern` is null or such a string.
/// Both strings live for the invocation; no recursive or concurrent call occurs.
unsafe extern "C" fn matched_path_with(
    path: *const core::ffi::c_char,
    pattern: *const core::ffi::c_char,
    payload: *mut core::ffi::c_void,
) -> c_int {
    // SAFETY: the internal seam guarantees the live exclusive handle and forbids
    // overlapping invocations or accesses to it during this call.
    let callback = unsafe { &mut *payload.cast::<MatchedPathCallback<'_>>() };
    if let Some(code) = callback.stopped {
        return code;
    }
    // SAFETY: the contract supplies a non-null immutable NUL string for this call.
    let path = unsafe { CStr::from_ptr(path) };
    let pattern = if pattern.is_null() {
        None
    } else {
        // SAFETY: non-null pattern is an immutable NUL string for this call.
        Some(unsafe { CStr::from_ptr(pattern) })
    };
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (callback.f)(path, pattern)));
    let code = match result {
        Ok(result) => result.code(),
        Err(panic) => {
            callback.panic = Some(panic);
            MatchedPathResult::ABORT.code()
        }
    };
    if code < 0 {
        callback.stopped = Some(code);
    }
    code
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use std::ffi::CString;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::SeqCst};

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process, where nothing has initialised libgit2.
    fn isolated(test: &str, body: fn()) {
        isolated_with(test, body, true);
    }

    /// As [`isolated`], with LeakSanitizer off in the child when `detect_leaks` is false:
    /// for bodies that drive libgit2 down a path on which it leaks.
    fn isolated_with(test: &str, body: fn(), detect_leaks: bool) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name);
        if !detect_leaks {
            let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
            let separator = if inherited.is_empty() { "" } else { ":" };
            command.env(
                "ASAN_OPTIONS",
                format!("{inherited}{separator}detect_leaks=0"),
            );
        }
        let out = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The policy's constructor hands out a fresh index, which `from_raw` adopts and
    /// `into_raw` gives back, released once.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn policy_constructor_round_trips_through_raw() {
        let fresh = IndexFree.c_new().unwrap();
        // SAFETY: a fresh index holding its only reference, which `IndexFree` releases.
        let index = unsafe { IndexBox::from_raw(fresh.as_ptr()) }.unwrap();
        let raw = index.into_raw();
        assert_eq!(raw, fresh.as_ptr());
        // SAFETY: the reference `into_raw` gave back, adopted once.
        let index: IndexArc = unsafe { IndexBox::from_raw(raw) }.unwrap().into();
        // SAFETY: the arc's live index, only read.
        assert_eq!(unsafe { ffi::git_index_entrycount(index.as_c_ptr()) }, 0);
    }

    #[test]
    fn options_carry_this_version_and_the_type() {
        for kind in [OidType::Sha1, OidType::Sha256] {
            let opts = options(kind);
            assert_eq!(opts.version, ffi::GIT_INDEX_OPTIONS_VERSION);
            assert_eq!(OidType::from_raw(opts.oid_type), Some(kind));
        }
    }

    /// A failure, or a success without an index, reports the thread's error and adopts
    /// nothing.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructed_reports_the_threads_error() {
        assert!(runtime_pinned());
        set(ErrorClass::Index, c"refused here");
        let error = constructed(-1, None).unwrap_err();
        assert_eq!(
            (error.class(), error.message()),
            (Some(ErrorClass::Index), c"refused here")
        );
        set(ErrorClass::Invalid, c"no index after all");
        let error = constructed(0, None).unwrap_err();
        assert_eq!(error.message(), c"no index after all");
    }

    /// A deliberate divergence: libgit2 opens an empty path, and writing that index is
    /// undefined behaviour (a NULL `memcpy` source in `git_filebuf_open_withsize`), so
    /// `open` refuses it. C's acceptance is checked without the write.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn empty_path_is_refused_where_c_accepts_it() {
        for built in [
            Index::open(c""),
            Index::open_with_oid_type(c"", OidType::Sha256),
        ] {
            let error = built.unwrap_err();
            assert_eq!(
                (error.class(), error.message()),
                (
                    Some(ErrorClass::Index),
                    c"failed to open index: the path is empty"
                )
            );
        }
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot and a NUL-terminated path, read for the call; the
        // runtime is pinned (by `open` above).
        let rc = unsafe { ffi::git_index_open(&mut out, c"".as_ptr()) };
        assert_eq!(rc, 0);
        // SAFETY: the fresh index C handed out, adopted once and never written.
        let c_index = unsafe { IndexBox::from_c(out) }.unwrap();
        // SAFETY: a live index, only read; the path is copied before the next call.
        let path = unsafe { CStr::from_ptr(ffi::git_index_path(c_index.as_c_ptr())) };
        assert_eq!(path, c"");
    }

    static REFUSING: AtomicBool = AtomicBool::new(false);
    static ALLOWED: AtomicIsize = AtomicIsize::new(0);
    /// Blocks [`Countdown`] handed out and has not had back.
    static LIVE: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut core::ffi::c_void;
        fn realloc(p: *mut core::ffi::c_void, n: usize) -> *mut core::ffi::c_void;
        fn free(p: *mut core::ffi::c_void);
    }

    /// Whether the next request is refused: while `REFUSING` is set, after `ALLOWED`
    /// more requests.
    fn refuse() -> bool {
        REFUSING.load(SeqCst) && ALLOWED.fetch_sub(1, SeqCst) <= 0
    }

    /// libc's allocator, refusing requests per [`refuse`].
    struct Countdown;

    // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
    unsafe impl Alloc for Countdown {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            if refuse() {
                return None;
            }
            // SAFETY: plain libc allocation.
            let block = NonNull::new(unsafe { malloc(len) }.cast());
            if block.is_some() {
                LIVE.fetch_add(1, SeqCst);
            }
            block
        }
        unsafe fn realloc(
            p: Option<NonNull<u8>>,
            len: usize,
            _: Option<&CStr>,
            _: i32,
        ) -> Option<NonNull<u8>> {
            if refuse() {
                return None;
            }
            let fresh = p.is_none();
            let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
            // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
            let block = NonNull::new(unsafe { realloc(p, len) }.cast());
            if fresh && block.is_some() {
                LIVE.fetch_add(1, SeqCst);
            }
            block
        }
        unsafe fn free(p: NonNull<u8>) {
            LIVE.fetch_sub(1, SeqCst);
            // SAFETY: `p` is a live libc block the caller gives up.
            unsafe { free(p.as_ptr().cast()) }
        }
    }

    /// Runs `build` with the `n`th allocation from now refused (and every later one),
    /// for `n` = 0, 1, ... until it succeeds; asserts each failure reports libgit2's
    /// out-of-memory error. Returns the number of failures.
    fn refuse_each_allocation(build: impl Fn() -> Result<IndexBox, ErrorBuf>) -> isize {
        // The thread's error state, allocated now, so a refusal reports out of memory
        // rather than a thread state it could not allocate.
        set(ErrorClass::Invalid, c"error state allocated");
        for n in 0.. {
            ALLOWED.store(n, SeqCst);
            REFUSING.store(true, SeqCst);
            let built = build();
            REFUSING.store(false, SeqCst);
            match built {
                Ok(index) => {
                    drop(index);
                    return n;
                }
                Err(error) => assert_eq!(
                    (error.class(), error.message()),
                    (Some(ErrorClass::NoMemory), c"Out of memory"),
                    "allocation {n}"
                ),
            }
        }
        unreachable!()
    }

    /// Installs [`Countdown`] as libgit2's allocator, before anything is allocated.
    fn install_countdown() {
        let allocator = Allocator::new::<Countdown>();
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

    /// When libgit2 cannot be initialised, neither constructor calls it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn uninitialisable_library_is_reported() {
        isolated(
            concat!(module_path!(), "::uninitialisable_library_is_reported"),
            || {
                install_countdown();
                ALLOWED.store(0, SeqCst);
                REFUSING.store(true, SeqCst);
                for built in [
                    Index::new(),
                    Index::open(c"index"),
                    Index::with_oid_type(OidType::Sha256),
                    Index::open_with_oid_type(c"index", OidType::Sha256),
                ] {
                    let error = built.unwrap_err();
                    assert_eq!(
                        (error.class(), error.message()),
                        (
                            Some(ErrorClass::Invalid),
                            c"library has not been initialized"
                        )
                    );
                }
                assert!(IndexBox::new().is_none());
                assert!(IndexArc::new().is_none());
            },
        );
    }

    /// Writes an index file of version `version` with `names` at `file`, through C.
    fn write_index_file(file: &CStr, version: u32, names: &[&CStr]) {
        let index = Index::open(file).unwrap();
        for name in names {
            let mut entry = ffi::git_index_entry {
                mode: 0o100644,
                path: name.as_ptr(),
                ..Default::default()
            };
            entry.id.id[0] = 1;
            entry.id.type_ = ffi::GIT_OID_SHA1 as _;
            // SAFETY: the box's live index; `entry` and its path are read and copied for
            // the call.
            let rc = unsafe { ffi::git_index_add(index.as_c_ptr(), &entry) };
            assert_eq!(rc, 0);
        }
        // SAFETY: the box's live index.
        let rc = unsafe { ffi::git_index_set_version(index.as_c_ptr(), version) };
        assert_eq!(rc, 0);
        // SAFETY: the box's live index, backed by `file`.
        let rc = unsafe { ffi::git_index_write(index.as_c_ptr()) };
        assert_eq!(rc, 0);
    }

    /// A C defect `open` inherits: libgit2 leaks the decompressed path of a version 4
    /// entry that runs past the end of the file (`read_entry` returns before freeing
    /// `tmp_path`, `src/libgit2/index.c`); a version 2 file cut the same way leaks
    /// nothing. Counted through the allocator, with LeakSanitizer off in the child.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn version_4_entry_cut_short_leaks_its_path_in_c() {
        isolated_with(
            concat!(
                module_path!(),
                "::version_4_entry_cut_short_leaks_its_path_in_c"
            ),
            || {
                install_countdown();
                assert!(runtime_pinned());
                let dir = std::env::temp_dir()
                    .join(format!("crustify-index-unit-leak-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                let file = CString::new(dir.join("index").to_str().unwrap()).unwrap();
                for (version, leaked) in [(2, 0), (4, 1)] {
                    let _ = std::fs::remove_file(dir.join("index"));
                    write_index_file(&file, version, &[c"dir/a", c"dir/b"]);
                    let bytes = std::fs::read(dir.join("index")).unwrap();
                    std::fs::write(dir.join("index"), &bytes[..bytes.len() - 1]).unwrap();
                    // The thread's error state settles on the first failure.
                    assert!(Index::open(&file).is_err());
                    let before = LIVE.load(SeqCst);
                    assert!(Index::open(&file).is_err());
                    assert_eq!(LIVE.load(SeqCst) - before, leaked, "version {version}");
                }
                let _ = std::fs::remove_dir_all(&dir);
            },
            false,
        );
    }

    /// Every allocation `Index::new` and `Index::open` make can fail: each failure is
    /// reported as out of memory, with the half-built index freed (LeakSanitizer).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn allocation_failures_are_reported_and_released() {
        isolated(
            concat!(
                module_path!(),
                "::allocation_failures_are_reported_and_released"
            ),
            || {
                install_countdown();
                assert!(runtime_pinned());

                assert!(refuse_each_allocation(Index::new) > 0);
                assert!(refuse_each_allocation(|| Index::with_oid_type(OidType::Sha256)) > 0);

                let dir = std::env::temp_dir()
                    .join(format!("crustify-index-unit-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                let file = CString::new(dir.join("index").to_str().unwrap()).unwrap();
                let missing = refuse_each_allocation(|| Index::open(&file));
                assert!(missing > 0);

                write_index_file(&file, 2, &[c"a", c"b/c", c"d"]);
                assert!(refuse_each_allocation(|| Index::open(&file)) > missing);
                assert!(
                    refuse_each_allocation(|| Index::open_with_oid_type(&file, OidType::Sha1))
                        > missing
                );
                let _ = std::fs::remove_dir_all(&dir);
            },
        );
    }

    mod option_unit_tests {
        use super::{AddFlags, Capabilities, CapabilitySetting};
        use crate::ffi;

        #[test]
        fn scalar_layouts_and_traits() {
            assert_eq!(
                size_of::<AddFlags>(),
                size_of::<ffi::git_index_add_option_t>()
            );
            assert_eq!(
                align_of::<AddFlags>(),
                align_of::<ffi::git_index_add_option_t>()
            );
            assert_eq!(
                size_of::<Capabilities>(),
                size_of::<ffi::git_index_capability_t>()
            );
            assert_eq!(
                align_of::<Capabilities>(),
                align_of::<ffi::git_index_capability_t>()
            );
            assert!(!core::mem::needs_drop::<AddFlags>());
            assert!(!core::mem::needs_drop::<Capabilities>());
            assert_eq!(AddFlags::default(), AddFlags::empty());
            assert_eq!(Capabilities::default(), Capabilities::empty());
            assert_eq!(format!("{:?}", AddFlags::FORCE), "AddFlags(1)");
            assert_eq!(
                format!("{:?}", Capabilities::NO_SYMLINKS),
                "Capabilities(4)"
            );
            assert_eq!(format!("{:?}", CapabilitySetting::FromOwner), "FromOwner");
            let setting = CapabilitySetting::from(Capabilities::all());
            assert_eq!(setting, CapabilitySetting::Explicit(Capabilities::all()));
            // Exercise Clone explicitly: these are independent, resource-free scalars.
            #[allow(clippy::clone_on_copy)]
            {
                assert_eq!(AddFlags::FORCE.clone(), AddFlags::FORCE);
                assert_eq!(Capabilities::all().clone(), Capabilities::all());
                assert_eq!(setting.clone(), setting);
            }
        }

        #[test]
        fn capability_validation_rejects_commands_and_unknown_bits() {
            for bits in [8, 9, 15, i32::MAX, i32::MIN, -1, -2, -8] {
                assert_eq!(Capabilities::from_bits(bits), None);
            }
            for bits in 0..8 {
                let caps = Capabilities::from_bits(bits).unwrap();
                assert_eq!(caps.bits(), bits);
                assert_eq!(CapabilitySetting::from(caps).to_raw(), bits);
            }
        }
    }

    mod matched_path {
        use super::*;
        use std::ffi::CString;
        use std::panic::{AssertUnwindSafe, catch_unwind};

        #[test]
        fn stopped_trampoline_is_not_reentered_and_scan_resets() {
            let mut calls = 0;
            let mut f = |_: &CStr, _: Option<&CStr>| {
                calls += 1;
                MatchedPathResult::from_code(-999)
            };
            let mut callback = MatchedPathCallback::new(&mut f);
            for _ in 0..2 {
                callback.scan_with(|cb, payload| {
                    for _ in 0..3 {
                        assert_eq!(
                            // SAFETY: live exclusive payload from scan_with and static
                            // immutable NUL strings, invoked synchronously without reentry.
                            unsafe { cb.unwrap()(c"a".as_ptr(), ptr::null(), payload) },
                            -999
                        );
                    }
                });
            }
            drop(callback);
            assert_eq!(calls, 2);
        }

        /// Exercises actual C -> Rust payload writes, both C invokers, and cleanup
        /// after a callback panic. BSan targets this exact test, not the workspace.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn foreign_scan_payload_and_panic_cleanup() {
            use crate::libgit2::repository::repository::Repository;
            let dir = std::env::temp_dir()
                .join(format!("crustify-matched-callback-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let repo = Repository::init(&path, false).unwrap();
            let index = repo.as_ref().index().unwrap();
            std::fs::write(dir.join("file"), "first").unwrap();
            let empty = ffi::git_strarray {
                strings: ptr::null_mut(),
                count: 0,
            };
            let mut seen = Vec::new();
            let mut f = |path: &CStr, pattern: Option<&CStr>| {
                seen.push((path.to_owned(), pattern.map(CStr::to_owned)));
                MatchedPathResult::APPLY
            };
            let mut cb = MatchedPathCallback::new(&mut f);
            assert_eq!(
                // SAFETY: live repository-backed index used exclusively during the call;
                // empty pathspec and callback payload are synchronous borrowed inputs.
                cb.scan_with(|callback, payload| unsafe {
                    ffi::git_index_add_all(index.as_c_ptr(), &empty, 0, callback, payload)
                }),
                0
            );
            std::fs::write(dir.join("file"), "a different length").unwrap();
            assert_eq!(
                // SAFETY: same as above; neither index nor callback is reentered.
                cb.scan_with(|callback, payload| unsafe {
                    ffi::git_index_update_all(index.as_c_ptr(), &empty, callback, payload)
                }),
                0
            );
            drop(cb);
            assert_eq!(
                seen,
                [(c"file".to_owned(), None), (c"file".to_owned(), None)]
            );

            let mut f =
                |_: &CStr, _: Option<&CStr>| -> MatchedPathResult { panic!("after C cleanup") };
            let mut cb = MatchedPathCallback::new(&mut f);
            let mut returned_from_c = false;
            let panic = catch_unwind(AssertUnwindSafe(|| {
                cb.scan_with(|callback, payload| {
                    // SAFETY: live exclusively used index and synchronous callback pair.
                    let code = unsafe {
                        ffi::git_index_remove_all(index.as_c_ptr(), &empty, callback, payload)
                    };
                    returned_from_c = true;
                    assert_eq!(code, MatchedPathResult::ABORT.code());
                })
            }));
            assert!(panic.is_err());
            assert!(returned_from_c);
            // SAFETY: the live index has not changed: the panic aborted before removal.
            assert_eq!(unsafe { ffi::git_index_entrycount(index.as_c_ptr()) }, 1);
            drop(index);
            drop(repo);
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn workdir_scan_skip_abort_and_removed_paths() {
            use crate::libgit2::repository::repository::Repository;
            use std::cell::Cell;
            let dir = std::env::temp_dir()
                .join(format!("crustify-matched-workdir-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let repo =
                Repository::init(&CString::new(dir.to_str().unwrap()).unwrap(), false).unwrap();
            let index = repo.as_ref().index().unwrap();
            std::fs::write(dir.join("file"), "contents").unwrap();
            std::fs::write(dir.join("outside"), "unmatched").unwrap();
            let mut pattern = c"file".as_ptr().cast_mut();
            let paths = ffi::git_strarray {
                strings: &mut pattern,
                count: 1,
            };
            let result = Cell::new(MatchedPathResult::SKIP);
            let mut f = |path: &CStr, pattern: Option<&CStr>| {
                assert_eq!((path, pattern), (c"file", Some(c"file")));
                result.get()
            };
            let mut cb = MatchedPathCallback::new(&mut f);
            for (answer, expected_code, count) in [
                (MatchedPathResult::SKIP, 0, 0),
                (MatchedPathResult::from_code(-999), -999, 0),
                (MatchedPathResult::APPLY, 0, 1),
            ] {
                result.set(answer);
                // SAFETY: live exclusively used index and borrowed pathspec; scan_with
                // supplies a synchronous callback pair that does not access the index.
                let code = cb.scan_with(|callback, payload| unsafe {
                    ffi::git_index_add_all(index.as_c_ptr(), &paths, 0, callback, payload)
                });
                assert_eq!(code, expected_code);
                assert_eq!(
                    // SAFETY: the live index is read only after the scan finishes.
                    unsafe { ffi::git_index_entrycount(index.as_c_ptr()) },
                    count
                );
            }
            std::fs::remove_file(dir.join("file")).unwrap();
            assert_eq!(
                // SAFETY: same synchronous exclusive-index contract; C removes the missing
                // workdir path only after the callback releases its borrowed path string.
                cb.scan_with(|callback, payload| unsafe {
                    ffi::git_index_update_all(index.as_c_ptr(), &paths, callback, payload)
                }),
                0
            );
            // SAFETY: the live index is only read after the callback and scan return.
            assert_eq!(unsafe { ffi::git_index_entrycount(index.as_c_ptr()) }, 0);
            drop(index);
            drop(repo);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
