//! The diff generators of `src/libgit2/diff_generate.c`: [`Diff`]'s
//! constructors over two trees, a tree and an index, an index and the working
//! directory, a tree and the working directory (with or without the index), and two
//! indices, with default options. The diffs and their deltas are
//! [`diff`](super::diff)'s.

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
