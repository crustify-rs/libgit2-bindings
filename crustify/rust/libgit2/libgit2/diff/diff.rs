//! Safe views of diffs and their deltas.
//!
//! Generated diffs borrow their repository, but copy tree/index entries and paths.
//! Parsed diffs own patches with copied paths, hunk lines and binary data. The public API has no
//! diff up-ref or deep copy: fresh results are `DiffBox`, and a counted reference
//! shared with C is `DiffArc` (without `Clone` or an exclusive access path).
//! Retaining a generated patch must borrow the diff exclusively for the patch's
//! lifetime, or consume the box into a shared owner: C patches can update deltas.
//! The retained parse context still contains input cursors, but no public operation
//! or destructor dereferences them after parsing (patch_parse.c).
//! No wrapper here is `Send` or `Sync`; the atomic refcount does not protect deltas.
//!
//! Deltas are read-only output records, allocated/freed by their diff or patch;
//! their embedded file paths belong to the parent's pool or parsed patch. There is
//! no public delta constructor, disposer or cloner. Never free a delta separately.
//! File views are the minimal dependency needed to project those embedded records.

use crate::ffi;
use crate::libgit2::index::index::IndexRef;
use crate::libgit2::object::tree::TreeRef;
use crate::libgit2::oid::oid::{OidRef, OidType};
use crate::libgit2::repository::repository::RepositoryRef;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use crate::libgit2::util::sys::errors::runtime_pinned;
use core::ffi::CStr;
use core::marker::PhantomData;
use core::ptr::{self, NonNull, addr_of};
use ffibox::{CArc, CBorrowedPtr, CBox, CCell, CDrop, define_ctype};

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
    pub fn raw_code(&self) -> i32 {
        self.code
    }
    pub fn code(&self) -> Option<ErrorCode> {
        ErrorCode::from_raw(self.code)
    }
    pub fn error(&self) -> &ErrorBuf {
        &self.error
    }
    pub fn class(&self) -> Option<ErrorClass> {
        self.error.class()
    }
    pub fn raw_class(&self) -> i32 {
        self.error.raw_class()
    }
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
/// Wraps: git_diff
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
    type Ref<'a>
        = DiffRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = DiffMut<'a, 'r>
    where
        Self: 'a;
}

impl<'a, 'r> DiffRef<'a, 'r> {
    /// # Safety
    /// A non-null pointer must be a live, initialized diff for `'a`, with its
    /// repository and retained option referents live for `'r`. Nothing may mutate
    /// or release the diff or its deltas during `'a`.
    #[allow(dead_code)]
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
    /// Wraps: git_diff_num_deltas
    pub fn len(&self) -> usize {
        // SAFETY: the handle keeps an initialized, unmodified diff alive.
        unsafe { ffi::git_diff_num_deltas(self.as_ptr()) }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A delta borrowed from this diff, or None when the index is out of bounds.
    /// Wraps: git_diff_get_delta
    pub fn get(&self, index: usize) -> Option<DiffDeltaRef<'a>> {
        // SAFETY: C bounds-checks index; a returned delta and its paths are kept
        // alive and unmodified by the diff borrow for 'a.
        unsafe { DiffDeltaRef::from_ptr(ffi::git_diff_get_delta(self.as_ptr(), index).cast_mut()) }
    }
}

impl<'a, 'r> DiffMut<'a, 'r> {
    /// # Safety
    /// Same validity/keepalive as DiffRef::from_ptr; additionally access must be
    /// exclusive for 'a, including every C-held reference and generated patch.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(p: *mut ffi::git_diff) -> Option<Self> {
        // SAFETY: the caller promises validity and exclusivity.
        unsafe { DiffRef::from_ptr(p) }.map(|p| Self(p, PhantomData))
    }
    #[allow(dead_code)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_diff {
        self.0.as_ptr().cast_mut()
    }
    pub fn as_ref(&self) -> DiffRef<'_, 'r> {
        self.0
    }
    pub fn as_mut(&mut self) -> DiffMut<'_, 'r> {
        DiffMut(self.0, PhantomData)
    }
}

/// Releases a counted reference, dispatching to C's generated/parsed destructor.
/// No public routine can clone or retain a diff independently.
/// Wraps: git_diff_free
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
    /// Parse Git patch bytes; paths, hunk lines and binary data are copied by C.
    /// Empty input is an empty diff. Non-Git unified diffs are rejected by C.
    /// Wraps: git_diff_from_buffer
    pub fn from_buffer(content: &[u8]) -> Result<DiffBox<'static>, DiffError> {
        if !runtime_pinned() {
            return Err(DiffError::from_libgit2(-1));
        }
        let mut out = ptr::null_mut();
        // SAFETY: content is readable for its length, including the non-null empty
        // slice; out is writable. C copies the data used after parsing; retained
        // input cursors are never dereferenced again (see the module docs).
        let rc =
            unsafe { ffi::git_diff_from_buffer(&mut out, content.as_ptr().cast(), content.len()) };
        // SAFETY: successful parsing hands out a fresh reference with no repository.
        unsafe { from_raw_result(out, rc) }
    }

    /// Parse with an explicit object-id type (the entire parse-options surface).
    /// Wraps: git_diff_from_buffer_ext
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
                &mut out,
                content.as_ptr().cast(),
                content.len(),
                &mut opts,
            )
        };
        // SAFETY: successful parsing hands out a fresh reference with no repository.
        unsafe { from_raw_result(out, rc) }
    }
}

/// Adopts a constructor result, preserving the error code.
///
/// # Safety
/// On success out must be a fresh sole reference whose repository and retained
/// option referents outlive 'r; on failure out must transfer no allocation.
unsafe fn from_raw_result<'r>(out: *mut ffi::git_diff, rc: i32) -> Result<DiffBox<'r>, DiffError> {
    if rc != 0 {
        return Err(DiffError::from_libgit2(rc));
    }
    // SAFETY: the calling constructor guarantees the allocation, keepalive and
    // sole ownership; DiffFree calls its paired destructor.
    Ok(unsafe { DiffBox::from_c(out) }.expect("successful diff construction returned NULL"))
}

impl<'r> Diff<'r> {
    /// Compare two trees; None denotes the empty tree on either side.
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: git_diff_tree_to_tree
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
                &mut out,
                repo.as_ptr().cast_mut(),
                old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                new.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r.
        unsafe { from_raw_result(out, rc) }
    }

    /// Compare a tree (None is empty) to an index (None loads the repository index).
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: git_diff_tree_to_index
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
                &mut out,
                repo.as_ptr().cast_mut(),
                old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                index.map_or(ptr::null_mut(), |i| i.as_ptr().cast_mut()),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r.
        unsafe { from_raw_result(out, rc) }
    }

    /// Compare an index (None loads the repository index) to the working directory.
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: git_diff_index_to_workdir
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
                &mut out,
                repo.as_ptr().cast_mut(),
                index.map_or(ptr::null_mut(), |i| i.as_ptr().cast_mut()),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r.
        unsafe { from_raw_result(out, rc) }
    }

    /// Compare a tree (None is empty) directly to the working directory.
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: git_diff_tree_to_workdir
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
                &mut out,
                repo.as_ptr().cast_mut(),
                old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r.
        unsafe { from_raw_result(out, rc) }
    }

    /// Compare a tree to the working directory, accounting for staged changes.
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: git_diff_tree_to_workdir_with_index
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
                &mut out,
                repo.as_ptr().cast_mut(),
                old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r.
        unsafe { from_raw_result(out, rc) }
    }

    /// Compare two indices. A repository is required by this C revision.
    /// Uses default options. The result borrows only repo, not the input snapshots.
    /// Wraps: git_diff_index_to_index
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
                &mut out,
                repo.as_ptr().cast_mut(),
                old.as_ptr().cast_mut(),
                new.as_ptr().cast_mut(),
                ptr::null(),
            )
        };
        // SAFETY: success yields a sole reference whose retained repo lives for 'r.
        unsafe { from_raw_result(out, rc) }
    }
}

define_ctype!(
    /// A read-only delta borrowed from a diff, patch, or callback. All fields are
    /// initialized, but flags identify which id/size values C actually knows.
    /// Wraps: git_diff_delta
    DiffDelta, DiffDeltaRef, DiffDeltaMut, ffi::git_diff_delta
);

impl<'a> DiffDeltaRef<'a> {
    /// Field: git_diff_delta.status
    pub fn status(&self) -> ffi::git_delta_t {
        // SAFETY: initialized scalar projected from a live, unmodified delta.
        unsafe { addr_of!((*self.as_ptr()).status).read() }
    }
    /// Field: git_diff_delta.flags
    pub fn flags(&self) -> u32 {
        // SAFETY: initialized scalar projected from a live, unmodified delta.
        unsafe { addr_of!((*self.as_ptr()).flags).read() }
    }
    /// Field: git_diff_delta.similarity
    pub fn similarity(&self) -> u16 {
        // SAFETY: initialized scalar projected from a live, unmodified delta.
        unsafe { addr_of!((*self.as_ptr()).similarity).read() }
    }
    /// Field: git_diff_delta.nfiles
    pub fn nfiles(&self) -> u16 {
        // SAFETY: initialized scalar projected from a live, unmodified delta.
        unsafe { addr_of!((*self.as_ptr()).nfiles).read() }
    }
    /// Shared projection of the embedded file, kept alive by the delta borrow.
    /// Field: git_diff_delta.old_file
    pub fn old_file(&self) -> DiffFileRef<'a> {
        // SAFETY: the embedded initialized file and its path are live and
        // unmodified for the parent delta borrow; no reference covers C storage.
        unsafe { DiffFileRef::from_ptr(addr_of!((*self.as_ptr()).old_file).cast_mut()) }
            .expect("an embedded field is non-null")
    }
    /// Shared projection of the embedded file, kept alive by the delta borrow.
    /// Field: git_diff_delta.new_file
    pub fn new_file(&self) -> DiffFileRef<'a> {
        // SAFETY: the embedded initialized file and its path are live and
        // unmodified for the parent delta borrow; no reference covers C storage.
        unsafe { DiffFileRef::from_ptr(addr_of!((*self.as_ptr()).new_file).cast_mut()) }
            .expect("an embedded field is non-null")
    }
}

// Required dependent projection: no separate allocation, ownership, or mutation
// API. C publishes files only as output through const deltas/callback pointers.
define_ctype!(
    /// One embedded side of a delta. Borrowed from its parent, never freed alone.
    /// Wraps: git_diff_file
    DiffFile, DiffFileRef, DiffFileMut, ffi::git_diff_file
);
impl<'a> DiffFileRef<'a> {
    /// The path, if present. It belongs to the parent's pool/patch, not this field.
    /// Field: git_diff_file.path
    pub fn path(&self) -> Option<&'a CStr> {
        // SAFETY: a live initialized file; the path is null or a NUL-terminated
        // immutable string kept alive by the parent for 'a.
        unsafe {
            let path = addr_of!((*self.as_ptr()).path).read();
            if path.is_null() {
                None
            } else {
                Some(CStr::from_ptr(path))
            }
        }
    }
    /// Field: git_diff_file.id
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: initialized inline oid, live and unwritten for the parent's borrow.
        unsafe { OidRef::from_ptr(addr_of!((*self.as_ptr()).id).cast_mut()) }
            .expect("an embedded field is non-null")
    }
    /// Field: git_diff_file.size
    pub fn size(&self) -> u64 {
        // SAFETY: initialized scalar projected from a live, unmodified file.
        unsafe { addr_of!((*self.as_ptr()).size).read() }
    }
    /// Field: git_diff_file.flags
    pub fn flags(&self) -> u32 {
        // SAFETY: initialized scalar projected from a live, unmodified file.
        unsafe { addr_of!((*self.as_ptr()).flags).read() }
    }
    /// Field: git_diff_file.mode
    pub fn mode(&self) -> u16 {
        // SAFETY: initialized scalar projected from a live, unmodified file.
        unsafe { addr_of!((*self.as_ptr()).mode).read() }
    }
    /// Field: git_diff_file.id_abbrev
    pub fn id_abbrev(&self) -> u16 {
        // SAFETY: initialized scalar projected from a live, unmodified file.
        unsafe { addr_of!((*self.as_ptr()).id_abbrev).read() }
    }
}

/// Inputs to the six C diff generators. These handles are borrowed only for the
/// construction call, even when raw options are used.
pub enum DiffInputs<'a, 't> {
    Trees {
        old: Option<TreeRef<'a, 't>>,
        new: Option<TreeRef<'a, 't>>,
    },
    TreeToIndex {
        old: Option<TreeRef<'a, 't>>,
        index: Option<IndexRef<'a>>,
    },
    IndexToWorkdir {
        index: Option<IndexRef<'a>>,
    },
    TreeToWorkdir {
        old: Option<TreeRef<'a, 't>>,
    },
    TreeToWorkdirWithIndex {
        old: Option<TreeRef<'a, 't>>,
    },
    Indices {
        old: IndexRef<'a>,
        new: IndexRef<'a>,
    },
}

impl<'r> Diff<'r> {
    /// Generate with nondefault options, pending the higher-layer options/callback
    /// wrappers. Prefer the six safe default constructors when possible.
    ///
    /// # Safety
    /// `options.version` must equal `GIT_DIFF_OPTIONS_VERSION`. All nested pointers
    /// must satisfy git_diff_options: pathspec is a readable array of its stated
    /// count, every string is NUL-terminated, callbacks and payload match each other.
    /// Their referents must remain valid for `'r` (C copies the options into the
    /// diff). Callbacks must not unwind, mutate/free input objects or reenter access
    /// to the diff; callback argument pointers must not escape the callback.
    /// No other access may conflict with callback payload writes. Input trees and
    /// indices must be compatible with the repository's object-id type.
    ///
    /// This is a temporary raw seam for the unavailable git_diff_options wrapper.
    pub unsafe fn generate_with_options(
        repo: RepositoryRef<'r>,
        inputs: DiffInputs<'_, '_>,
        options: &'r ffi::git_diff_options,
    ) -> Result<DiffBox<'r>, DiffError> {
        let mut out = ptr::null_mut();
        // SAFETY: handles keep all input objects alive for the call and repo for
        // 'r. The caller guarantees nested option validity and callback discipline.
        // Each generator copies entries/paths and returns a sole owned reference.
        let rc = unsafe {
            match inputs {
                DiffInputs::Trees { old, new } => ffi::git_diff_tree_to_tree(
                    &mut out,
                    repo.as_ptr().cast_mut(),
                    old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                    new.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                    options,
                ),
                DiffInputs::TreeToIndex { old, index } => ffi::git_diff_tree_to_index(
                    &mut out,
                    repo.as_ptr().cast_mut(),
                    old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                    index.map_or(ptr::null_mut(), |i| i.as_ptr().cast_mut()),
                    options,
                ),
                DiffInputs::IndexToWorkdir { index } => ffi::git_diff_index_to_workdir(
                    &mut out,
                    repo.as_ptr().cast_mut(),
                    index.map_or(ptr::null_mut(), |i| i.as_ptr().cast_mut()),
                    options,
                ),
                DiffInputs::TreeToWorkdir { old } => ffi::git_diff_tree_to_workdir(
                    &mut out,
                    repo.as_ptr().cast_mut(),
                    old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                    options,
                ),
                DiffInputs::TreeToWorkdirWithIndex { old } => {
                    ffi::git_diff_tree_to_workdir_with_index(
                        &mut out,
                        repo.as_ptr().cast_mut(),
                        old.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut()),
                        options,
                    )
                }
                DiffInputs::Indices { old, new } => ffi::git_diff_index_to_index(
                    &mut out,
                    repo.as_ptr().cast_mut(),
                    old.as_ptr().cast_mut(),
                    new.as_ptr().cast_mut(),
                    options,
                ),
            }
        };
        // SAFETY: success yields a sole reference with repo and option referents
        // valid for 'r, as required by this function's contract.
        unsafe { from_raw_result(out, rc) }
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
        assert_eq!(error.code().map(|code| code.raw()), Some(error.raw_code()));
        assert_eq!(error.class(), error.error().class());
        assert!(error.to_string().contains("diff error"));
        assert!(!format!("{error:?}").is_empty());
        let boxed: Box<dyn std::error::Error> = Box::new(error);
        assert!(boxed.source().is_none());
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
        raw.old_file.mode = 0o100644;
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
        assert_eq!((f.flags(), f.mode(), f.id_abbrev()), (5, 0o100644, 7));
        assert_eq!(f.id().id(), [0; 32]);
        assert!(d.new_file().path().is_none());
        // SAFETY: null is expressly accepted by borrowed-handle construction.
        assert!(unsafe { DiffDeltaRef::from_ptr(ptr::null_mut()) }.is_none());
    }
}
