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

use core::ptr::{self};

use crate::ffi;
use crate::libgit2::src::libgit2::diff::{Diff, DiffBox, DiffError, constructed};
use crate::libgit2::src::libgit2::index::IndexRef;
use crate::libgit2::src::libgit2::repository::RepositoryRef;
use crate::libgit2::src::libgit2::tree::TreeRef;

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
