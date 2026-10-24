//! The patch parser of `src/libgit2/diff_parse.c`: [`Diff`]'s
//! constructors from the text of a Git patch, for SHA-1 or a given object id type.

use core::ptr::{self};

use crate::ffi;
use crate::libgit2::src::libgit2::diff::{Diff, DiffBox, DiffError, constructed};
use crate::libgit2::src::libgit2::oid::OidType;
use crate::libgit2::src::util::errors::runtime_pinned;

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
