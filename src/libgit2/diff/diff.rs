//! Safe views of diffs and their deltas.
//!
//! Generated diffs borrow their repository, but copy tree/index entries and paths.
//! Parsed diffs own patches with copied paths, hunk lines and binary data. The public API has no
//! diff up-ref or deep copy: fresh results are `DiffBox`, and a counted reference
//! shared with C is `DiffArc` (without `Clone` or an exclusive access path).
//! Retaining a generated patch must borrow the diff exclusively for the patch's
//! lifetime, or consume the box into a shared owner: C patches can update deltas.
//! The retained parse context still contains input cursors, but no public operation
//! or destructor dereferences them after parsing (`patch_parse.c`).
//! No wrapper here is `Send` or `Sync`; the atomic refcount does not protect deltas.
//!
//! Deltas are read-only output records, allocated/freed by their diff or patch;
//! their embedded file paths belong to the parent's pool or parsed patch. There is
//! no public delta constructor, disposer or cloner. Never free a delta separately.
//! File views are the minimal dependency needed to project those embedded records.

use core::ffi::CStr;
use core::marker::PhantomData;
use core::ptr::{self, NonNull, addr_of};
use std::ffi::CString;

use ffibox::{CArc, CBorrowedPtr, CBox, CCell, CDrop, define_ctype};

use crate::ffi;
use crate::libgit2::core::strarray::StrArrayRef;
use crate::libgit2::index::index::IndexRef;
use crate::libgit2::index::pathspec::patterns_are_parsable;
use crate::libgit2::object::tree::TreeRef;
use crate::libgit2::oid::oid::{OidRef, OidType};
use crate::libgit2::repository::repository::RepositoryRef;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use crate::libgit2::util::sys::errors::{runtime_pinned, set};

/// The exact nonzero C result and a snapshot of its thread-local error.
/// Positive progress-callback cancellation codes are preserved, not normalized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffError {
    code: i32,
    error: ErrorBuf,
}
impl DiffError {
    fn from_libgit2(code: i32) -> Self {
        Self {
            code,
            error: last(),
        }
    }

    /// libgit2's raw return code.
    #[must_use]
    pub fn raw_code(&self) -> i32 {
        self.code
    }

    /// The return code, as a named [`ErrorCode`] when libgit2 names it.
    #[must_use]
    pub fn code(&self) -> Option<ErrorCode> {
        ErrorCode::from_raw(self.code)
    }

    /// The thread's libgit2 error, copied when the call failed.
    #[must_use]
    pub fn error(&self) -> &ErrorBuf {
        &self.error
    }

    /// The error's class, when libgit2 names it.
    #[must_use]
    pub fn class(&self) -> Option<ErrorClass> {
        self.error.class()
    }

    /// The error's raw class.
    #[must_use]
    pub fn raw_class(&self) -> i32 {
        self.error.raw_class()
    }

    /// The error's message.
    #[must_use]
    pub fn message(&self) -> &CStr {
        self.error.message()
    }
}
impl core::fmt::Display for DiffError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "diff error {}: {}", self.code, self.error)
    }
}
impl std::error::Error for DiffError {}

/// Opaque C layout, carrying the repository's keepalive lifetime.
/// Parsed diffs have no repository and use `'static`.
///
/// Neither the layout nor its shared handle is `Sync`:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::diff::diff::Diff;
/// fn sync<T: Sync>() {}
/// sync::<Diff<'static>>();
/// ```
/// ```compile_fail,E0277
/// use libgit2::libgit2::diff::diff::DiffRef;
/// fn sync<T: Sync>() {}
/// sync::<DiffRef<'static, 'static>>();
/// ```
///
/// Wraps: `git_diff`
/// Handwritten
#[repr(transparent)]
pub struct Diff<'r>(ffi::git_diff, PhantomData<(*const (), &'r ())>);

/// A shared, copyable borrow. Getters never modify the diff.
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct DiffRef<'a, 'r>(CBorrowedPtr<'a, Diff<'r>>);

/// An exclusive borrow; reborrows cannot outlive this handle.
/// Handwritten
#[repr(transparent)]
pub struct DiffMut<'a, 'r>(DiffRef<'a, 'r>, PhantomData<&'a mut Diff<'r>>);

// SAFETY: layout and both handles are transparent over their specified C layout
// and CBorrowedPtr respectively, have no Drop, and Mut is invariant in Diff.
unsafe impl<'r> CCell for Diff<'r> {
    type C = ffi::git_diff;
    type Mut<'a>
        = DiffMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = DiffRef<'a, 'r>
    where
        Self: 'a;
}

impl<'a> DiffRef<'a, '_> {
    /// # Safety
    /// A non-null pointer must be a live, initialized diff for `'a`, with its
    /// repository and retained option referents live for `'r`. Nothing may mutate
    /// or release the diff or its deltas during `'a`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(p: *const ffi::git_diff) -> Option<Self> {
        NonNull::new(p.cast_mut()).map(|p| {
            // SAFETY: caller establishes the lifetime and shared access contract.
            Self(unsafe { CBorrowedPtr::new(p.cast()) })
        })
    }

    pub(crate) fn as_ptr(&self) -> *const ffi::git_diff {
        self.0.as_non_null().as_ptr().cast()
    }

    /// Number of deltas (including unmodified entries if requested).
    /// Wraps: `git_diff_num_deltas`
    #[must_use]
    pub fn len(&self) -> usize {
        // SAFETY: the handle keeps an initialized, unmodified diff alive.
        unsafe { ffi::git_diff_num_deltas(self.as_ptr()) }
    }

    /// Whether the diff holds no delta.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A delta borrowed from this diff, or None when the index is out of bounds.
    /// Wraps: `git_diff_get_delta`
    #[must_use]
    pub fn get(&self, index: usize) -> Option<DiffDeltaRef<'a>> {
        // SAFETY: C bounds-checks index; a returned delta and its paths are kept
        // alive and unmodified by the diff borrow for 'a.
        let diff_get_delta = unsafe { ffi::git_diff_get_delta(self.as_ptr(), index) };
        // SAFETY: as above.
        unsafe { DiffDeltaRef::from_ptr(diff_get_delta.cast_mut()) }
    }
}

impl<'r> DiffMut<'_, 'r> {
    /// # Safety
    /// Same validity/keepalive as `DiffRef::from_ptr`; additionally access must be
    /// exclusive for 'a, including every C-held reference and generated patch.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(p: *mut ffi::git_diff) -> Option<Self> {
        // SAFETY: the caller promises validity and exclusivity.
        unsafe { DiffRef::from_ptr(p) }.map(|p| Self(p, PhantomData))
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_diff {
        self.0.as_ptr().cast_mut()
    }

    /// A shared handle over the same object, borrowed from this one.
    #[must_use]
    pub fn as_ref(&self) -> DiffRef<'_, 'r> {
        self.0
    }

    /// An exclusive handle over the same object, reborrowed from this one.
    pub fn as_mut(&mut self) -> DiffMut<'_, 'r> {
        DiffMut(self.0, PhantomData)
    }
}

/// Releases a counted reference, dispatching to C's generated/parsed destructor.
/// No public routine can clone or retain a diff independently.
/// Wraps: `git_diff_free`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct DiffFree;
// SAFETY: git_diff_free decrements exactly one live reference and frees the
// concrete generated/parsed storage and its children only at count zero.
unsafe impl<'r> CDrop<Diff<'r>> for DiffFree {
    unsafe fn c_drop(&self, p: NonNull<Diff<'r>>) {
        // SAFETY: the caller surrenders one counted reference of the matching type.
        unsafe { ffi::git_diff_free(p.as_ptr().cast()) }
    }
}

/// A fresh diff's sole reference. Borrows exclude release or mutable reborrowing.
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::diff::diff::Diff;
/// let d = Diff::from_buffer(b"").unwrap();
/// let r = d.as_ref();
/// drop(d);
/// r.len();
/// ```
/// ```compile_fail,E0502
/// use libgit2::libgit2::diff::diff::Diff;
/// let mut d = Diff::from_buffer(b"").unwrap();
/// let r = d.as_ref();
/// let m = d.as_mut();
/// r.len();
/// ```
/// ```compile_fail,E0277
/// use libgit2::libgit2::diff::diff::Diff;
/// let d = Diff::from_buffer(b"").unwrap();
/// std::thread::spawn(move || drop(d));
/// ```
pub type DiffBox<'r> = CBox<Diff<'r>, DiffFree>;
/// One reference shared with C. No clone or exclusive access without a public
/// up-ref/count query; conversion from a sole owner is supported by ffibox.
pub type DiffArc<'r> = CArc<Diff<'r>, DiffFree>;

impl Diff<'static> {
    /// Parse Git patch bytes; paths, hunk lines and binary data are copied by C. Empty input is an
    /// empty diff.
    ///
    /// # Errors
    ///
    /// Non-Git unified diffs are rejected by C. Wraps: `git_diff_from_buffer`
    ///
    /// Fails, as well, with libgit2's error when it cannot be initialised.
    pub fn from_buffer(content: &[u8]) -> Result<DiffBox<'static>, DiffError> {
        if !runtime_pinned() {
            return Err(DiffError::from_libgit2(-1));
        }
        let (mut out, data) = (ptr::null_mut(), content.as_ptr().cast());
        // SAFETY: content is readable for its length, including the non-null empty
        // slice; out is writable. C copies the data used after parsing; retained
        // input cursors are never dereferenced again (see the module docs).
        let rc = unsafe { ffi::git_diff_from_buffer(&raw mut out, data, content.len()) };
        // SAFETY: successful parsing hands out a fresh sole reference with no repository,
        // released by DiffFree; every failure leaves `out` NULL (adopted as None).
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }

    /// Parse with an explicit object-id type (the entire parse-options surface).
    /// Wraps: `git_diff_from_buffer_ext`
    ///
    /// # Errors
    ///
    /// Fails as [`from_buffer`](Self::from_buffer) does.
    pub fn from_buffer_with_oid_type(
        content: &[u8],
        kind: OidType,
    ) -> Result<DiffBox<'static>, DiffError> {
        if !runtime_pinned() {
            return Err(DiffError::from_libgit2(-1));
        }
        let mut opts = ffi::git_diff_parse_options {
            version: ffi::GIT_DIFF_PARSE_OPTIONS_VERSION,
            oid_type: kind.raw(),
        };
        let mut out = ptr::null_mut();
        // SAFETY: options and bytes live through parsing. C copies paths/lines/data;
        // the retained parse cursor is never dereferenced after parsing finishes.
        let rc = unsafe {
            ffi::git_diff_from_buffer_ext(
                &raw mut out,
                content.as_ptr().cast(),
                content.len(),
                &raw mut opts,
            )
        };
        // SAFETY: successful parsing hands out a fresh sole reference with no repository,
        // released by DiffFree; every failure leaves `out` NULL (adopted as None).
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }
}

/// The diff a constructor returning `rc` handed out, or its error with the exact code.
/// Every constructor leaves its output NULL whenever it returns nonzero, including a
/// progress callback's positive code, so a failure adopts nothing.
fn constructed(rc: i32, diff: Option<DiffBox<'_>>) -> Result<DiffBox<'_>, DiffError> {
    if rc != 0 {
        return Err(DiffError::from_libgit2(rc));
    }
    Ok(diff.expect("successful diff construction returned NULL"))
}

impl<'r> Diff<'r> {
    /// Compare two trees; None denotes the empty tree on either side.
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: `git_diff_tree_to_tree`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when an object cannot be read from the object database,
    /// or when it runs out of memory.
    pub fn tree_to_tree(
        repo: RepositoryRef<'r>,
        old: Option<TreeRef<'_, '_>>,
        new: Option<TreeRef<'_, '_>>,
    ) -> Result<DiffBox<'r>, DiffError> {
        let mut out = ptr::null_mut();
        // SAFETY: initialized repository/input handles stay live during the call,
        // used only on this thread. C copies entries/paths, keeps repo uncounted,
        // and retains no input handle. NULL options selects callback-free defaults.
        let rc = unsafe {
            ffi::git_diff_tree_to_tree(
                &raw mut out,
                repo.as_ptr().cast_mut(),
                old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                new.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r,
        // released by DiffFree; every failure leaves `out` NULL (adopted as None).
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }

    /// Compare a tree (None is empty) to an index (None loads the repository index).
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: `git_diff_tree_to_index`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when the repository's index cannot be loaded, an object
    /// cannot be read, or it runs out of memory.
    pub fn tree_to_index(
        repo: RepositoryRef<'r>,
        old: Option<TreeRef<'_, '_>>,
        index: Option<IndexRef<'_>>,
    ) -> Result<DiffBox<'r>, DiffError> {
        let mut out = ptr::null_mut();
        // SAFETY: initialized repository/input handles stay live during the call,
        // used only on this thread. C copies entries/paths, keeps repo uncounted,
        // and retains no input handle. NULL options selects callback-free defaults.
        let rc = unsafe {
            ffi::git_diff_tree_to_index(
                &raw mut out,
                repo.as_ptr().cast_mut(),
                old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                index.map_or(ptr::null_mut(), |i| i.as_ptr().cast_mut()),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r,
        // released by DiffFree; every failure leaves `out` NULL (adopted as None).
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }

    /// Compare an index (None loads the repository index) to the working directory.
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: `git_diff_index_to_workdir`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for a bare repository, when the repository's index
    /// cannot be loaded, a file cannot be read or filtered, or it runs out of memory.
    pub fn index_to_workdir(
        repo: RepositoryRef<'r>,
        index: Option<IndexRef<'_>>,
    ) -> Result<DiffBox<'r>, DiffError> {
        let mut out = ptr::null_mut();
        // SAFETY: initialized repository/input handles stay live during the call,
        // used only on this thread. C copies entries/paths, keeps repo uncounted,
        // and retains no input handle. NULL options selects callback-free defaults.
        let rc = unsafe {
            ffi::git_diff_index_to_workdir(
                &raw mut out,
                repo.as_ptr().cast_mut(),
                index.map_or(ptr::null_mut(), |i| i.as_ptr().cast_mut()),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r,
        // released by DiffFree; every failure leaves `out` NULL (adopted as None).
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }

    /// Compare a tree (None is empty) directly to the working directory.
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: `git_diff_tree_to_workdir`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for a bare repository, when an object or a file cannot
    /// be read, or when it runs out of memory.
    pub fn tree_to_workdir(
        repo: RepositoryRef<'r>,
        old: Option<TreeRef<'_, '_>>,
    ) -> Result<DiffBox<'r>, DiffError> {
        let mut out = ptr::null_mut();
        // SAFETY: initialized repository/input handles stay live during the call,
        // used only on this thread. C copies entries/paths, keeps repo uncounted,
        // and retains no input handle. NULL options selects callback-free defaults.
        let rc = unsafe {
            ffi::git_diff_tree_to_workdir(
                &raw mut out,
                repo.as_ptr().cast_mut(),
                old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r,
        // released by DiffFree; every failure leaves `out` NULL (adopted as None).
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }

    /// Compare a tree to the working directory, accounting for staged changes.
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: `git_diff_tree_to_workdir_with_index`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for a bare repository, when the repository's index
    /// cannot be loaded, an object or a file cannot be read, or it runs out of memory.
    pub fn tree_to_workdir_with_index(
        repo: RepositoryRef<'r>,
        old: Option<TreeRef<'_, '_>>,
    ) -> Result<DiffBox<'r>, DiffError> {
        let mut out = ptr::null_mut();
        // SAFETY: initialized repository/input handles stay live during the call,
        // used only on this thread. C copies entries/paths, keeps repo uncounted,
        // and retains no input handle. NULL options selects callback-free defaults.
        let rc = unsafe {
            ffi::git_diff_tree_to_workdir_with_index(
                &raw mut out,
                repo.as_ptr().cast_mut(),
                old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r,
        // released by DiffFree; every failure leaves `out` NULL (adopted as None).
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }

    /// Compare two indices. A repository is required by this C revision.
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: `git_diff_index_to_index`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when an object cannot be read, or when it runs out of
    /// memory.
    pub fn index_to_index(
        repo: RepositoryRef<'r>,
        old: IndexRef<'_>,
        new: IndexRef<'_>,
    ) -> Result<DiffBox<'r>, DiffError> {
        let mut out = ptr::null_mut();
        // SAFETY: initialized repository/input handles stay live during the call,
        // used only on this thread. C copies entries/paths, keeps repo uncounted,
        // and retains no input handle. NULL options selects callback-free defaults.
        let rc = unsafe {
            ffi::git_diff_index_to_index(
                &raw mut out,
                repo.as_ptr().cast_mut(),
                old.as_ptr().cast_mut(),
                new.as_ptr().cast_mut(),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r,
        // released by DiffFree; every failure leaves `out` NULL (adopted as None).
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }
}

define_ctype!(
    /// A read-only delta borrowed from a diff, patch, or callback. All fields are
    /// initialized, but flags identify which id/size values C actually knows.
    /// Wraps: git_diff_delta
    DiffDelta,
    DiffDeltaRef,
    DiffDeltaMut,
    ffi::git_diff_delta
);

impl<'a> DiffDeltaRef<'a> {
    /// Field: `git_diff_delta.status`
    #[must_use]
    pub fn status(&self) -> ffi::git_delta_t {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let status_ptr = unsafe { &raw const (*self.as_ptr()).status };
        // SAFETY: initialized scalar projected from a live, unmodified delta.
        unsafe { status_ptr.read() }
    }

    /// Field: `git_diff_delta.flags`
    #[must_use]
    pub fn flags(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let flags_ptr = unsafe { &raw const (*self.as_ptr()).flags };
        // SAFETY: initialized scalar projected from a live, unmodified delta.
        unsafe { flags_ptr.read() }
    }

    /// Field: `git_diff_delta.similarity`
    #[must_use]
    pub fn similarity(&self) -> u16 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let similarity_ptr = unsafe { &raw const (*self.as_ptr()).similarity };
        // SAFETY: initialized scalar projected from a live, unmodified delta.
        unsafe { similarity_ptr.read() }
    }

    /// Field: `git_diff_delta.nfiles`
    #[must_use]
    pub fn nfiles(&self) -> u16 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let nfiles_ptr = unsafe { &raw const (*self.as_ptr()).nfiles };
        // SAFETY: initialized scalar projected from a live, unmodified delta.
        unsafe { nfiles_ptr.read() }
    }

    /// Shared projection of the embedded file, kept alive by the delta borrow.
    /// Field: `git_diff_delta.old_file`
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: an embedded field is non-null"
    )]
    pub fn old_file(&self) -> DiffFileRef<'a> {
        // SAFETY: the embedded initialized file and its path are live and
        // unmodified for the parent delta borrow; no reference covers C storage.
        let value = unsafe { addr_of!((*self.as_ptr()).old_file) };
        // SAFETY: as above.
        unsafe { DiffFileRef::from_ptr(value.cast_mut()) }.expect("an embedded field is non-null")
    }

    /// Shared projection of the embedded file, kept alive by the delta borrow.
    /// Field: `git_diff_delta.new_file`
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: an embedded field is non-null"
    )]
    pub fn new_file(&self) -> DiffFileRef<'a> {
        // SAFETY: the embedded initialized file and its path are live and
        // unmodified for the parent delta borrow; no reference covers C storage.
        let value = unsafe { addr_of!((*self.as_ptr()).new_file) };
        // SAFETY: as above.
        unsafe { DiffFileRef::from_ptr(value.cast_mut()) }.expect("an embedded field is non-null")
    }
}

// Required dependent projection: no separate allocation, ownership, or mutation
// API. C publishes files only as output through const deltas/callback pointers.
define_ctype!(
    /// One embedded side of a delta. Borrowed from its parent, never freed alone.
    /// Wraps: git_diff_file
    DiffFile,
    DiffFileRef,
    DiffFileMut,
    ffi::git_diff_file
);
impl<'a> DiffFileRef<'a> {
    /// The path, if present. It belongs to the parent's pool/patch, not this field.
    /// Field: `git_diff_file.path`
    #[must_use]
    pub fn path(&self) -> Option<&'a CStr> {
        {
            // SAFETY: a live initialized file; the path is null or a NUL-terminated
            // immutable string kept alive by the parent for 'a.
            let value = unsafe { addr_of!((*self.as_ptr()).path) };
            // SAFETY: as above.
            let path = unsafe { value.read() };
            // SAFETY: as above.
            unsafe {
                if path.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr(path))
                }
            }
        }
    }

    /// Field: `git_diff_file.id`
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: an embedded field is non-null"
    )]
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: initialized inline oid, live and unwritten for the parent's borrow.
        let value = unsafe { addr_of!((*self.as_ptr()).id) };
        // SAFETY: as above.
        unsafe { OidRef::from_ptr(value.cast_mut()) }.expect("an embedded field is non-null")
    }

    /// Field: `git_diff_file.size`
    #[must_use]
    pub fn size(&self) -> u64 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let size_ptr = unsafe { &raw const (*self.as_ptr()).size };
        // SAFETY: initialized scalar projected from a live, unmodified file.
        unsafe { size_ptr.read() }
    }

    /// Field: `git_diff_file.flags`
    #[must_use]
    pub fn flags(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let flags_ptr = unsafe { &raw const (*self.as_ptr()).flags };
        // SAFETY: initialized scalar projected from a live, unmodified file.
        unsafe { flags_ptr.read() }
    }

    /// Field: `git_diff_file.mode`
    #[must_use]
    pub fn mode(&self) -> u16 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let mode_ptr = unsafe { &raw const (*self.as_ptr()).mode };
        // SAFETY: initialized scalar projected from a live, unmodified file.
        unsafe { mode_ptr.read() }
    }

    /// Field: `git_diff_file.id_abbrev`
    #[must_use]
    pub fn id_abbrev(&self) -> u16 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let id_abbrev_ptr = unsafe { &raw const (*self.as_ptr()).id_abbrev };
        // SAFETY: initialized scalar projected from a live, unmodified file.
        unsafe { id_abbrev_ptr.read() }
    }
}

/// Inputs to the six C diff generators. These handles are borrowed only for the
/// construction call, even when raw options are used.
pub enum DiffInputs<'a, 't> {
    /// Two trees, as [`Diff::tree_to_tree`] compares them.
    Trees {
        /// The old tree; `None` is the empty tree.
        old: Option<TreeRef<'a, 't>>,
        /// The new tree; `None` is the empty tree.
        new: Option<TreeRef<'a, 't>>,
    },
    /// A tree and an index, as [`Diff::tree_to_index`] compares them.
    TreeToIndex {
        /// The old tree; `None` is the empty tree.
        old: Option<TreeRef<'a, 't>>,
        /// The index; `None` loads the repository's.
        index: Option<IndexRef<'a>>,
    },
    /// An index and the working directory, as [`Diff::index_to_workdir`] compares them.
    IndexToWorkdir {
        /// The index; `None` loads the repository's.
        index: Option<IndexRef<'a>>,
    },
    /// A tree and the working directory, as [`Diff::tree_to_workdir`] compares them.
    TreeToWorkdir {
        /// The old tree; `None` is the empty tree.
        old: Option<TreeRef<'a, 't>>,
    },
    /// A tree and the working directory with its staged changes, as
    /// [`Diff::tree_to_workdir_with_index`] compares them.
    TreeToWorkdirWithIndex {
        /// The old tree; `None` is the empty tree.
        old: Option<TreeRef<'a, 't>>,
    },
    /// Two indices, as [`Diff::index_to_index`] compares them.
    Indices {
        /// The old index.
        old: IndexRef<'a>,
        /// The new index.
        new: IndexRef<'a>,
    },
}

impl<'r> Diff<'r> {
    /// Generate with nondefault options, pending the higher-layer options/callback
    /// wrappers. Prefer the six safe default constructors when possible.
    ///
    /// # Errors
    ///
    /// Fails with code `-1` and a `GIT_ERROR_INVALID` error for the checked inputs the Safety
    /// section names, and otherwise as the safe constructors fail, with a callback's own code when
    /// a callback stops the diff.
    ///
    /// # Safety
    /// All nested pointers must satisfy `git_diff_options`: pathspec is a readable
    /// array of its stated count, every string is NUL-terminated, callbacks and
    /// payload match each other. Their referents must remain valid for `'r` (C copies
    /// the options into the diff). Callbacks must not unwind, mutate/free input
    /// objects or reenter access to the diff; callback argument pointers must not
    /// escape the callback. No other access may conflict with callback payload writes.
    ///
    /// The rest is checked, so it is no obligation: another `options.version`, a
    /// nonzero `options.oid_type` other than the repository's, or an empty pathspec
    /// pattern beside a non-empty one, which libgit2 would read past
    /// (`parse_optimized_patterns`, `src/libgit2/attr_file.c`), fails with code -1 and
    /// a `GIT_ERROR_INVALID` error. Inputs of the other object-id type are accepted,
    /// as by the safe constructors; their deltas keep each input id's own type.
    ///
    /// The version is checked here, with libgit2's own error: `git_diff_tree_to_workdir`
    /// and `git_diff_index_to_index` return its refusal as 1 instead of -1, a code a
    /// progress callback could also have returned (`(error = f(...) < 0)` in
    /// `diff_generate.c`).
    ///
    /// This is a temporary raw seam for the unavailable `git_diff_options` wrapper.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a formatted number has no NUL"
    )]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the helper consumes what it reports, as its callers hand it over"
    )]
    pub unsafe fn generate_with_options(
        repo: RepositoryRef<'r>,
        inputs: DiffInputs<'_, '_>,
        options: &'r ffi::git_diff_options,
    ) -> Result<DiffBox<'r>, DiffError> {
        if !(1..=ffi::GIT_DIFF_OPTIONS_VERSION).contains(&options.version) {
            // libgit2 formats the unsigned version with `%d`.
            let version = options.version.cast_signed();
            let message = format!("invalid version {version} on git_diff_options");
            set(
                ErrorClass::Invalid,
                &CString::new(message).expect("a formatted number has no NUL"),
            );
            return Err(DiffError::from_libgit2(-1));
        }
        // SAFETY: the caller guarantees the pathspec is a readable array of its stated
        // count of NUL-terminated strings, live beyond this call and unchanged by it.
        let pathspec = unsafe { StrArrayRef::from_ptr((&raw const options.pathspec).cast_mut()) };
        if !patterns_are_parsable(pathspec.expect("an embedded field is non-null")) {
            return Err(DiffError::from_libgit2(-1));
        }
        let mut out = ptr::null_mut();
        // Handles keep all input objects alive for the call and repo for 'r. The caller
        // guarantees nested option validity and callback discipline. Each generator copies
        // entries/paths and returns a sole owned reference.
        let raw_repo = repo.as_ptr().cast_mut();
        let rc = match inputs {
            DiffInputs::Trees { old, new } => {
                let old = old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut());
                let new = new.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut());
                // SAFETY: as above, for two trees or NULL.
                unsafe { ffi::git_diff_tree_to_tree(&raw mut out, raw_repo, old, new, options) }
            }
            DiffInputs::TreeToIndex { old, index } => {
                let old = old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut());
                let index = index.map_or(ptr::null_mut(), |i| i.as_ptr().cast_mut());
                // SAFETY: as above, for a tree or NULL and an index or NULL.
                unsafe { ffi::git_diff_tree_to_index(&raw mut out, raw_repo, old, index, options) }
            }
            DiffInputs::IndexToWorkdir { index } => {
                let index = index.map_or(ptr::null_mut(), |i| i.as_ptr().cast_mut());
                // SAFETY: as above, for an index or NULL.
                unsafe { ffi::git_diff_index_to_workdir(&raw mut out, raw_repo, index, options) }
            }
            DiffInputs::TreeToWorkdir { old } => {
                let old = old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut());
                // SAFETY: as above, for a tree or NULL.
                unsafe { ffi::git_diff_tree_to_workdir(&raw mut out, raw_repo, old, options) }
            }
            DiffInputs::TreeToWorkdirWithIndex { old } => {
                let old = old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut());
                // SAFETY: as above, for a tree or NULL.
                unsafe {
                    ffi::git_diff_tree_to_workdir_with_index(&raw mut out, raw_repo, old, options)
                }
            }
            DiffInputs::Indices { old, new } => {
                let (old, new) = (old.as_ptr().cast_mut(), new.as_ptr().cast_mut());
                // SAFETY: as above, for two indices.
                unsafe { ffi::git_diff_index_to_index(&raw mut out, raw_repo, old, new, options) }
            }
        };
        // SAFETY: success yields a sole reference with repo and option referents
        // valid for 'r, as required by this function's contract; every failure,
        // a callback's positive code included, leaves `out` NULL.
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn raw_owner_and_borrow_seams() {
        let owned = Diff::from_buffer(b"").unwrap();
        let raw = owned.into_c();
        // SAFETY: sole reference transferred from the safe owner immediately above.
        let mut owned = unsafe { DiffBox::from_c(raw) }.unwrap();
        {
            // SAFETY: the owner stays live and nothing mutates the diff in this scope.
            let r = unsafe { DiffRef::from_ptr(owned.as_c_ptr()) }.unwrap();
            assert!(r.is_empty());
        }
        {
            // SAFETY: the sole owner has no other active handle in this scope.
            let mut m = unsafe { DiffMut::from_ptr(owned.as_c_ptr()) }.unwrap();
            assert_eq!(m.as_mut_ptr(), raw);
            assert!(m.as_mut().as_ref().is_empty());
        }
        assert!(owned.as_mut().as_ref().is_empty());
        // SAFETY: null pointers are accepted by both optional borrowing seams.
        assert!(unsafe { DiffRef::from_ptr(ptr::null()) }.is_none());
        // SAFETY: null pointers are accepted by both optional borrowing seams.
        assert!(unsafe { DiffMut::from_ptr(ptr::null_mut()) }.is_none());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn error_snapshot_and_formatting() {
        let error = Diff::from_buffer(b"not a patch").err().unwrap();
        let copy = error.clone();
        assert_eq!(copy, error);
        assert_eq!(error.code().map(ErrorCode::raw), Some(error.raw_code()));
        assert_eq!(error.class(), error.error().class());
        assert!(error.to_string().contains("diff error"));
        assert_ne!(format!("{error:?}"), "");
        let boxed: Box<dyn std::error::Error> = Box::new(error);
        assert!(boxed.source().is_none());
    }

    /// `git_diff_tree_to_workdir` and `git_diff_index_to_index` return a refused options
    /// version as 1 (`(error = f(...) < 0)`); the wrapper refuses it as libgit2's other
    /// generators do: -1 with their `GIT_ERROR_INVALID` message, before any C call.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn refused_options_version_is_negative_for_every_generator() {
        use crate::libgit2::index::index::Index;
        use crate::libgit2::repository::repository::Repository;
        let dir =
            std::env::temp_dir().join(format!("crustify-diff-version-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        let repo = Repository::init(&CString::new(dir.to_str().unwrap()).unwrap(), false).unwrap();
        let index = Index::new().unwrap();
        let mut opts = ffi::git_diff_options::default();
        for (version, shown) in [(0, c"0"), (2, c"2"), (u32::MAX, c"-1")] {
            opts.version = version;
            let mut out = ptr::null_mut();
            // SAFETY: a live repository, NULL trees and a writable output slot; C checks
            // the version before reading any other field of the options.
            let rc = unsafe {
                ffi::git_diff_tree_to_tree(
                    &raw mut out,
                    repo.as_ref().as_ptr().cast_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &raw const opts,
                )
            };
            assert_eq!((rc, out), (-1, ptr::null_mut()));
            let expected = last();
            let message = format!(
                "invalid version {} on git_diff_options",
                shown.to_str().unwrap()
            );
            assert_eq!(expected.message().to_str(), Ok(message.as_str()));
            for inputs in [
                DiffInputs::TreeToWorkdir { old: None },
                DiffInputs::Indices {
                    old: index.as_ref(),
                    new: index.as_ref(),
                },
            ] {
                // SAFETY: no nested pointers or callbacks; the version is refused first.
                let error = unsafe { Diff::generate_with_options(repo.as_ref(), inputs, &opts) }
                    .unwrap_err();
                assert_eq!((error.raw_code(), error.error()), (-1, &expected));
            }
        }
        drop(repo);
        drop(std::fs::remove_dir_all(&dir));
    }

    /// Without a libgit2 runtime both parsers fail before calling C. The runtime is
    /// made to fail in a fresh process, whose first allocation request is refused.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2 and spawns a child")]
    #[expect(
        clippy::items_after_statements,
        reason = "each helper item sits beside the steps that use it"
    )]
    fn parsers_refuse_without_a_runtime() {
        use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
        const CHILD: &str = "CRUSTIFY_DIFF_NO_RUNTIME_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let name = concat!(module_path!(), "::parsers_refuse_without_a_runtime");
            let name = name.split_once("::").unwrap().1;
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{stdout}\n{stderr}");
            assert!(stdout.contains("1 passed"), "{stdout}");
            return;
        }
        struct Refusing;
        // SAFETY: it never hands out an allocation, which `Alloc` allows; `free` is
        // never reached, since nothing was allocated.
        unsafe impl Alloc for Refusing {
            fn malloc(_: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                None
            }

            unsafe fn realloc(
                _: Option<NonNull<u8>>,
                _: usize,
                _: Option<&CStr>,
                _: i32,
            ) -> Option<NonNull<u8>> {
                None
            }

            unsafe fn free(_: NonNull<u8>) {}
        }
        let allocator = Allocator::new::<Refusing>();
        let option = ffi::GIT_OPT_SET_ALLOCATOR.cast_signed();
        // SAFETY: this fresh process has never initialised libgit2, and no other
        // thread runs; C copies the valid allocator table.
        let rc = unsafe { ffi::git_libgit2_opts(option, allocator.as_ref().as_ptr()) };
        assert_eq!(rc, 0);
        assert_eq!(Diff::from_buffer(b"").err().unwrap().raw_code(), -1);
        let parsed = Diff::from_buffer_with_oid_type(b"", OidType::Sha256);
        assert_eq!(parsed.err().unwrap().raw_code(), -1);
        assert!(!runtime_pinned(), "a failed initialisation stays failed");
    }

    // Rust-only layout projections: run under Miri without calling the C library.
    #[test]
    fn delta_and_file_projection_layout() {
        assert_eq!(size_of::<DiffDelta>(), size_of::<ffi::git_diff_delta>());
        assert_eq!(align_of::<DiffDelta>(), align_of::<ffi::git_diff_delta>());
        assert_eq!(size_of::<DiffFile>(), size_of::<ffi::git_diff_file>());
        assert_eq!(
            size_of::<DiffBox<'static>>(),
            size_of::<*mut ffi::git_diff>()
        );
        let mut raw = ffi::git_diff_delta {
            status: ffi::GIT_DELTA_RENAMED,
            similarity: 99,
            nfiles: 2,
            flags: 7,
            ..Default::default()
        };
        raw.old_file.path = c"old".as_ptr();
        raw.old_file.size = u64::MAX;
        raw.old_file.mode = 0o100_644;
        raw.old_file.id_abbrev = 7;
        raw.old_file.flags = 5;
        // SAFETY: initialized stack record, with a static path. The exclusive
        // handle does not outlive this record or coexist with direct access.
        let mut delta = unsafe { DiffDeltaMut::from_ptr(&raw mut raw) }.unwrap();
        let reborrow = delta.as_mut();
        let d = reborrow.as_ref();
        assert_eq!(
            (d.status(), d.flags(), d.similarity(), d.nfiles()),
            (ffi::GIT_DELTA_RENAMED, 7, 99, 2)
        );
        let f = d.old_file();
        assert_eq!(f.path(), Some(c"old"));
        assert_eq!(f.size(), u64::MAX);
        assert_eq!((f.flags(), f.mode(), f.id_abbrev()), (5, 0o100_644, 7));
        assert_eq!(f.id().id(), [0; 32]);
        assert!(d.new_file().path().is_none());
        // SAFETY: null is expressly accepted by borrowed-handle construction.
        assert!(unsafe { DiffDeltaRef::from_ptr(ptr::null_mut()) }.is_none());
    }
}
