//! Safe handles for the opaque match lists published by `git2/pathspec.h`.
//!
//! Filename matches and failed patterns are copied into the list's pool. The list
//! also retains a counted reference to the compiled pathspec. Neither source paths,
//! the index, tree nor repository need to survive the returned filename list.
//! Diff matches instead borrow deltas: libgit2 does not retain the diff. `MatchList`
//! carries that borrow as `'diff`; filename lists use `'static`.
//!
//! All storage and destruction remain C-owned. There is no owning clone, in-place
//! initializer, field disposer, field access or list mutation. No `Send`/`Sync`
//! is asserted.

use core::ffi::CStr;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};
use ffibox::{CBorrowedPtr, CBox, CCell, CDrop};

use crate::ffi;
use crate::libgit2::diff::diff::{DiffDeltaRef, DiffRef};
use crate::libgit2::index::index::IndexMut;
use crate::libgit2::object::tree::TreeRef;
use crate::libgit2::repository::repository::{RepositoryError, RepositoryMut};

/// An opaque pathspec match list, owned by [`MatchListBox`].
///
/// `'diff` keeps borrowed diff deltas alive and unchanged. Filename lists own their
/// strings and have no source lifetime. Failed patterns always belong to the list.
/// The layout is never instantiated by value: only C constructs this opaque type.
///
/// A string cannot outlive its list:
/// ```compile_fail,E0505
/// use libgit2::libgit2::index::pathspec::MatchListBox;
/// fn invalid(list: MatchListBox<'static>) {
///     let entry = list.as_ref().entry(0).unwrap();
///     drop(list);
///     println!("{entry:?}");
/// }
/// ```
/// Exclusive reborrows exclude shared access:
/// ```compile_fail,E0502
/// use libgit2::libgit2::index::pathspec::MatchListBox;
/// fn invalid(mut list: MatchListBox<'static>) {
///     let exclusive = list.as_mut();
///     let shared = list.as_ref();
///     assert_eq!(exclusive.as_ref().len(), shared.len());
/// }
/// ```
/// Exclusive handles are invariant in the diff lifetime:
/// ```compile_fail
/// use libgit2::libgit2::index::pathspec::MatchListMut;
/// fn invalid<'a, 'd>(m: MatchListMut<'a, 'static>) -> MatchListMut<'a, 'd> { m }
/// ```
/// Owners remain thread-bound:
/// ```compile_fail,E0277
/// use libgit2::libgit2::index::pathspec::MatchListBox;
/// fn invalid(list: MatchListBox<'static>) {
///     std::thread::spawn(move || drop(list));
/// }
/// ```
///
/// Wraps: git_pathspec_match_list
/// Handwritten
#[repr(transparent)]
pub struct MatchList<'diff>(
    ffi::git_pathspec_match_list,
    PhantomData<(&'diff (), *const ())>,
);

/// Copyable shared handle; no Rust reference covers C-owned list storage.
///
/// Handwritten
#[repr(transparent)]
pub struct MatchListRef<'a, 'diff>(CBorrowedPtr<'a, MatchList<'diff>>);

impl Clone for MatchListRef<'_, '_> {
    fn clone(&self) -> Self {
        *self
    }
}
impl Copy for MatchListRef<'_, '_> {}

/// Exclusive handle, invariant in the retained diff lifetime.
///
/// Handwritten
#[repr(transparent)]
pub struct MatchListMut<'a, 'diff>(
    MatchListRef<'a, 'diff>,
    PhantomData<&'a mut MatchList<'diff>>,
);

// SAFETY: the layout and both no-Drop handles are transparent over the C type
// and CBorrowedPtr respectively; markers are zero-sized. Mut is invariant in
// MatchList, shared handles only read, and neither handle references C storage.
unsafe impl<'diff> CCell for MatchList<'diff> {
    type C = ffi::git_pathspec_match_list;
    type Ref<'a>
        = MatchListRef<'a, 'diff>
    where
        Self: 'a;
    type Mut<'a>
        = MatchListMut<'a, 'diff>
    where
        Self: 'a;
}

/// Releases the list, its arrays, string pool and retained pathspec reference.
/// Borrowed diff deltas are never freed. No C clone operation exists.
///
/// Wraps: git_pathspec_match_list_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct MatchListFree;

// SAFETY: every adopted list owns its C allocation, arrays and pool; the public
// destructor frees exactly those, and drops its retained pathspec reference once.
unsafe impl CDrop<MatchList<'_>> for MatchListFree {
    unsafe fn c_drop(&self, ptr: NonNull<MatchList<'_>>) {
        // SAFETY: CDrop's caller transfers the live, unique list allocation.
        unsafe { ffi::git_pathspec_match_list_free(ptr.as_ptr().cast()) }
    }
}

/// Sole owner, freed through C. No deep-copy or refcount clone is available.
pub type MatchListBox<'diff> = CBox<MatchList<'diff>, MatchListFree>;

impl<'a, 'diff> MatchListRef<'a, 'diff> {
    pub(crate) fn as_ptr(&self) -> *const ffi::git_pathspec_match_list {
        self.0.as_non_null().as_ptr().cast()
    }

    /// Number of filenames or diff deltas; zero with `FAILURES_ONLY`.
    ///
    /// Wraps: git_pathspec_match_list_entrycount
    pub fn len(&self) -> usize {
        // SAFETY: this handle keeps the initialized list alive and unwritten.
        unsafe { ffi::git_pathspec_match_list_entrycount(self.as_ptr()) }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Matched filename, or `None` out of bounds or for a diff-generated list.
    ///
    /// Wraps: git_pathspec_match_list_entry
    pub fn entry(&self, index: usize) -> Option<&'a CStr> {
        // SAFETY: C checks the datatype and index before reading its array. Strings
        // are NUL-terminated pool allocations, immutable until this list is freed.
        let p = unsafe { ffi::git_pathspec_match_list_entry(self.as_ptr(), index) };
        // SAFETY: non-null results belong to the list for the handle's lifetime.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) })
    }

    /// Number of unmatched patterns; zero unless `FIND_FAILURES` was requested.
    ///
    /// Wraps: git_pathspec_match_list_failed_entrycount
    pub fn failed_len(&self) -> usize {
        // SAFETY: the shared handle keeps the initialized list alive.
        unsafe { ffi::git_pathspec_match_list_failed_entrycount(self.as_ptr()) }
    }

    /// A copied unmatched pattern, or `None` out of bounds.
    ///
    /// Wraps: git_pathspec_match_list_failed_entry
    pub fn failed_entry(&self, index: usize) -> Option<&'a CStr> {
        // SAFETY: C bounds-checks before reading; failure strings are immutable
        // NUL-terminated allocations in this list's pool (not caller storage).
        let p = unsafe { ffi::git_pathspec_match_list_failed_entry(self.as_ptr(), index) };
        // SAFETY: a non-null result remains live for the list handle's lifetime.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) })
    }

    /// A delta borrowed from the source diff, or `None` out of bounds / for a
    /// filename list. Its lifetime is bounded by the list and the source diff.
    ///
    /// Wraps: git_pathspec_match_list_diff_entry
    pub fn diff_entry(&self, index: usize) -> Option<DiffDeltaRef<'a>> {
        // SAFETY: C checks datatype and bounds; the list and borrowed diff are live.
        unsafe {
            DiffDeltaRef::from_ptr(
                ffi::git_pathspec_match_list_diff_entry(self.as_ptr(), index).cast_mut(),
            )
        }
    }
}

impl<'diff> MatchListMut<'_, 'diff> {
    pub fn as_ref(&self) -> MatchListRef<'_, 'diff> {
        self.0
    }
    pub fn as_mut(&mut self) -> MatchListMut<'_, 'diff> {
        MatchListMut(self.0, PhantomData)
    }
}

impl MatchList<'static> {
    /// Match an index, copying filenames and failed patterns into the result.
    ///
    /// The index iterator may sort its source, so it receives an exclusive
    /// handle. The compiled pathspec is a temporary raw seam: its wrapper has
    /// not yet landed. Replace this argument with its shared owner when available.
    ///
    /// # Safety
    /// `pathspec` must be non-null, initialized by `git_pathspec_new`, and kept
    /// alive by a counted reference for this call. No concurrent access may
    /// mutate it. C retains another counted reference until the list is freed:
    /// any Rust owner of the original reference must permit shared ownership,
    /// not promise exclusive access to the compiled pathspec.
    ///
    /// Wraps: git_pathspec_match_index
    pub unsafe fn from_index(
        mut index: IndexMut<'_>,
        pathspec: *mut ffi::git_pathspec,
        flags: u32,
    ) -> Result<MatchListBox<'static>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: exclusive source index and pathspec valid per the caller; writable
        // out slot. C copies all names and releases its iterator before returning.
        let rc =
            unsafe { ffi::git_pathspec_match_index(&mut out, index.as_mut_ptr(), flags, pathspec) };
        // SAFETY: success transfers a fresh initialized list; failure frees and
        // nulls any list it allocated (or leaves our initial NULL unchanged).
        unsafe { finish(rc, out) }
    }

    /// Match a tree; the result copies filenames and need not retain the tree.
    ///
    /// # Safety
    /// `pathspec` must satisfy [`MatchList::from_index`]'s pathspec contract.
    ///
    /// Wraps: git_pathspec_match_tree
    pub unsafe fn from_tree(
        tree: TreeRef<'_, '_>,
        pathspec: *mut ffi::git_pathspec,
        flags: u32,
    ) -> Result<MatchListBox<'static>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: tree and repository kept alive by the handle; immutable tree
        // traversal, pathspec valid per the caller, and a writable output slot.
        let rc = unsafe {
            ffi::git_pathspec_match_tree(&mut out, tree.as_ptr().cast_mut(), flags, pathspec)
        };
        // SAFETY: fresh initialized list on success, NULL on failure.
        unsafe { finish(rc, out) }
    }

    /// Match the working directory, excluding ignored untracked files. A bare
    /// repository is an error. Filenames in the result are independent copies.
    ///
    /// # Safety
    /// `pathspec` must satisfy [`MatchList::from_index`]'s pathspec contract.
    ///
    /// Wraps: git_pathspec_match_workdir
    pub unsafe fn from_workdir(
        mut repo: RepositoryMut<'_>,
        pathspec: *mut ffi::git_pathspec,
        flags: u32,
    ) -> Result<MatchListBox<'static>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: exclusive repository handle, pathspec valid per the caller; C copies
        // names, retains no repository, and writes only our output slot.
        let rc = unsafe {
            ffi::git_pathspec_match_workdir(&mut out, repo.as_mut_ptr(), flags, pathspec)
        };
        // SAFETY: fresh initialized list on success, NULL on failure.
        unsafe { finish(rc, out) }
    }
}

impl<'diff> MatchList<'diff> {
    /// Match a diff, borrowing its deltas. Failed patterns are copied.
    ///
    /// The shared diff handle keeps its deltas alive and unchanged for the
    /// returned list's lifetime; the list retains its own pathspec reference.
    ///
    /// A source diff must outlive a returned list, including its destructor:
    /// ```compile_fail,E0597
    /// use libgit2::libgit2::diff::diff::Diff;
    /// use libgit2::libgit2::index::pathspec::MatchList;
    /// unsafe fn invalid(ps: *mut libgit2::ffi::git_pathspec) {
    ///     let list;
    ///     let diff = Diff::from_buffer(b"").unwrap();
    ///     // SAFETY: this example assumes ps meets the constructor contract.
    ///     list = unsafe { MatchList::from_diff(diff.as_ref(), ps, 0) };
    /// }
    /// ```
    ///
    /// # Safety
    /// `pathspec` must satisfy [`MatchList::from_index`]'s pathspec contract.
    ///
    /// Wraps: git_pathspec_match_diff
    pub unsafe fn from_diff(
        diff: DiffRef<'diff, '_>,
        pathspec: *mut ffi::git_pathspec,
        flags: u32,
    ) -> Result<MatchListBox<'diff>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: live diff handle retaining deltas for 'diff and pathspec per caller; writable
        // output slot. C retains the pathspec but merely borrows delta pointers.
        let rc = unsafe {
            ffi::git_pathspec_match_diff(&mut out, diff.as_ptr().cast_mut(), flags, pathspec)
        };
        // SAFETY: fresh initialized list on success, NULL on failure; its borrowed
        // deltas have the lifetime guaranteed by the shared diff handle.
        unsafe { finish(rc, out) }
    }
}

/// # Safety
/// `out` is NULL on error, or a freshly transferred initialized list on success.
/// Any borrowed deltas must remain alive and unchanged throughout `'diff`.
unsafe fn finish<'diff>(
    rc: i32,
    out: *mut ffi::git_pathspec_match_list,
) -> Result<MatchListBox<'diff>, RepositoryError> {
    if rc < 0 {
        return Err(RepositoryError::from_libgit2(rc));
    }
    // SAFETY: the successful constructor transferred one unique allocation,
    // released by MatchListFree; retained deltas satisfy the lifetime contract.
    unsafe { MatchListBox::from_c(out) }.ok_or_else(|| RepositoryError::from_libgit2(-1))
}

#[cfg(test)]
mod match_list_unit_tests {
    use super::*;

    #[test]
    fn handles_and_owner_preserve_pointer_layout() {
        assert_eq!(
            size_of::<MatchList<'static>>(),
            size_of::<ffi::git_pathspec_match_list>()
        );
        assert_eq!(
            align_of::<MatchList<'static>>(),
            align_of::<ffi::git_pathspec_match_list>()
        );
        assert_eq!(size_of::<MatchListRef<'_, '_>>(), size_of::<*const ()>());
        assert_eq!(size_of::<MatchListMut<'_, '_>>(), size_of::<*const ()>());
        assert_eq!(size_of::<MatchListBox<'_>>(), size_of::<*const ()>());
        assert_eq!(
            size_of::<Option<MatchListBox<'_>>>(),
            size_of::<*const ()>()
        );
        // SAFETY: NULL transfers no allocation or retained borrow.
        assert!(unsafe { MatchListBox::from_c(ptr::null_mut()) }.is_none());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn raw_adoption_transfers_one_list_and_release() {
        use crate::libgit2::index::index::Index;
        let mut index = Index::new().unwrap();
        let mut strings = [c"missing".as_ptr().cast_mut()];
        let patterns = ffi::git_strarray {
            strings: strings.as_mut_ptr(),
            count: 1,
        };
        let mut ps = ptr::null_mut();
        // SAFETY: initialized call-only string array and writable output. C copies
        // the input, and the safe index constructor has pinned the runtime.
        assert_eq!(unsafe { ffi::git_pathspec_new(&mut ps, &patterns) }, 0);
        // SAFETY: a live heap pathspec; its original counted reference remains
        // live for the call, and no owner promises exclusive access to it.
        let list =
            unsafe { MatchList::from_index(index.as_mut(), ps, ffi::GIT_PATHSPEC_FIND_FAILURES) }
                .unwrap();
        // SAFETY: release only the original reference; the list holds another.
        unsafe { ffi::git_pathspec_free(ps) };
        let raw = list.into_c();
        // SAFETY: the fresh list was just surrendered, with no remaining handles;
        // it owns copied strings and needs no external diff lifetime.
        let mut list: MatchListBox<'static> = unsafe { MatchListBox::from_c(raw) }.unwrap();
        drop(index);
        assert_eq!(list.as_ref().as_ptr(), raw.cast_const());
        // Intentionally exercise Clone, not only the equivalent Copy operation.
        #[allow(clippy::clone_on_copy)]
        let cloned = list.as_ref().clone();
        assert_eq!(cloned.failed_entry(0), Some(c"missing"));
        assert_eq!(
            list.as_mut().as_mut().as_ref().failed_entry(0),
            Some(c"missing")
        );
    }
}
