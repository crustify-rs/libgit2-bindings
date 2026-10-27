//! Safe lifecycle, flags and match lists for `include/git2/pathspec.h`.
//!
//! The compiled pathspec is opaque: only C owns its layout and allocation.
//! Construction copies the patterns and prefix; no input string or array is
//! retained. A new pathspec has one reference. Match lists retain another reference
//! internally, released through the same down-ref, but C publishes no up-ref or
//! copy routine. Use [`PathspecBox`] while sole, and convert to [`PathspecArc`]
//! before passing it to a retaining matcher, which takes the shared owner. The
//! revwalk API instead stores an **uncounted** borrow: its eventual wrapper must
//! keep the pathspec alive until the walker releases it.
//!
//! Private `git_pathspec__init` / `__clear` support C stack storage, not the public
//! opaque type, and are deliberately not exposed. No pathspec fields are published.
//!
//! Filename matches and failed patterns are copied into the list's pool. Neither
//! source paths, the index, tree nor repository need to survive the returned
//! filename list. Diff matches instead borrow deltas: libgit2 does not retain the
//! diff. `MatchList` carries that borrow as `'diff`; filename lists use `'static`.
//!
//! All list storage and destruction remain C-owned. There is no owning clone,
//! in-place initializer, field disposer, field access or list mutation. No
//! `Send`/`Sync` is asserted for either type.

use core::ffi::CStr;
use core::marker::PhantomData;
use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};
use core::ptr::{self, NonNull};

use ffibox::{
    CArc, CBorrowedPtr, CBox, CCell, CDrop, CMutSeam, CRefSeam, define_ctype, impl_cdrop,
};

use crate::ffi;
use crate::libgit2::src::libgit2::diff::{DiffDeltaRef, DiffRef};
use crate::libgit2::src::libgit2::index::IndexMut;
use crate::libgit2::src::libgit2::repository::{RepositoryError, RepositoryMut};
use crate::libgit2::src::libgit2::strarray::StrArrayRef;
use crate::libgit2::src::libgit2::tree::TreeRef;
use crate::libgit2::src::util::errors::{ErrorBuf, ErrorClass, last, runtime_pinned, set};

define_ctype!(
    /// A compiled, C-owned pathspec, reached only through owners and handles.
    ///
    /// Wraps: git_pathspec
    Pathspec,
    PathspecRef,
    PathspecMut,
    ffi::git_pathspec
);

/// Releases one counted reference, freeing the patterns, prefix, pool and object
/// when the count reaches zero. No public clone or up-ref routine exists.
///
/// Wraps: `git_pathspec_free`
#[derive(Clone, Copy, Debug, Default)]
pub struct PathspecFree;

impl_cdrop!(PathspecFree, Pathspec, ffi::git_pathspec_free);

/// The sole reference to a compiled pathspec, with shared and exclusive handles.
///
/// Handles cannot survive their owner:
/// ```compile_fail,E0505
/// use libgit2::libgit2::src::libgit2::pathspec::PathspecBox;
/// fn invalid(owner: PathspecBox) {
///     let view = owner.as_ref();
///     drop(owner);
///     let _still_live = view;
/// }
/// ```
/// An exclusive reborrow freezes its parent:
/// ```compile_fail,E0499
/// use libgit2::libgit2::src::libgit2::pathspec::PathspecBox;
/// fn invalid(mut owner: PathspecBox) {
///     let mut parent = owner.as_mut();
///     let child = parent.as_mut();
///     let other = parent.as_mut();
///     let _both = (child, other);
/// }
/// ```
/// A shared reborrow also excludes a mutable one:
/// ```compile_fail,E0502
/// use libgit2::libgit2::src::libgit2::pathspec::PathspecBox;
/// fn invalid(mut owner: PathspecBox) {
///     let mut parent = owner.as_mut();
///     let shared = parent.as_ref();
///     let exclusive = parent.as_mut();
///     let _both = (shared, exclusive);
/// }
/// ```
pub type PathspecBox = CBox<Pathspec, PathspecFree>;

/// One counted reference, potentially shared with C match lists. Only shared
/// handles are available; no public routine can clone it or prove sole ownership.
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::src::libgit2::pathspec::PathspecArc;
/// fn invalid(mut owner: PathspecArc) { let _ = owner.get_mut(); }
/// ```
/// ```compile_fail,E0599
/// use libgit2::libgit2::src::libgit2::pathspec::PathspecArc;
/// fn invalid(owner: PathspecArc) { let _ = owner.clone(); }
/// ```
/// Owners and handles remain thread-bound:
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::pathspec::PathspecArc;
/// fn invalid(owner: PathspecArc) { std::thread::spawn(move || drop(owner)); }
/// ```
pub type PathspecArc = CArc<Pathspec, PathspecFree>;

impl Pathspec {
    /// Compiles an array of patterns, copying everything needed before returning. An empty array,
    /// or an array of empty strings, matches every path. Either a borrowed
    /// [`StrArrayView`](crate::libgit2::src::libgit2::strarray::StrArrayView) or an owned string
    /// array can provide the handle.
    ///
    /// Wraps: `git_pathspec_new`
    ///
    /// # Errors
    ///
    /// Reports C's error on failure.
    ///
    /// An array mixing empty and non-empty patterns is refused, with a
    /// `GIT_ERROR_INVALID` error, before libgit2 sees it: libgit2 would read past the
    /// end of each empty one (`parse_optimized_patterns`, `src/libgit2/attr_file.c`).
    pub fn new(patterns: StrArrayRef<'_>) -> Result<PathspecBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if !patterns_are_parsable(patterns) {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `patterns` supplies a live read-only array of non-null terminated
        // strings (or an empty array). C copies them and retains no input pointer.
        // `out` is a writable output slot; the pinned runtime outlives the result.
        let rc = unsafe { ffi::git_pathspec_new(&raw mut out, patterns.as_ptr()) };
        if rc < 0 {
            // C cleans its partial allocation and leaves the output untouched.
            return Err(last());
        }
        // SAFETY: success hands out one fresh, fully initialized allocation with
        // refcount 1, released exactly once by the paired `git_pathspec_free` policy.
        unsafe { PathspecBox::from_c(out) }.ok_or_else(last)
    }
}

impl PathspecRef<'_> {
    /// Whether `path` matches the compiled patterns. Only
    /// [`NO_GLOB`](PathspecFlags::NO_GLOB) and
    /// [`IGNORE_CASE`](PathspecFlags::IGNORE_CASE) are consulted: matching is
    /// case-sensitive unless `IGNORE_CASE` is set, whatever the platform, and every
    /// other bit is ignored. A pathspec with no effective pattern matches every path.
    /// Otherwise patterns are tried in order and the first that decides a path wins:
    /// `!pattern` excludes a path only ahead of any pattern that includes it. A
    /// literal pattern also matches the paths below it as a directory.
    ///
    /// libgit2 only reads the pathspec and `path`, retains neither, and reports no
    /// error: its sole failure path rejects a NULL argument, which cannot occur here.
    ///
    /// Wraps: `git_pathspec_matches_path`
    #[must_use]
    pub fn matches_path(&self, flags: PathspecFlags, path: &CStr) -> bool {
        let (pathspec, path) = (self.as_ptr(), path.as_ptr());
        // SAFETY: this handle keeps the initialized pathspec alive and unwritten for
        // the call, and `path` is NUL-terminated. C reads both without retaining them.
        let rc = unsafe { ffi::git_pathspec_matches_path(pathspec, flags.bits(), path) };
        rc > 0
    }
}

/// Whether libgit2 can parse `patterns` as a pathspec, setting a `GIT_ERROR_INVALID`
/// error when it cannot. Once one pattern is not empty, libgit2 parses every one, and
/// reads the byte after an empty pattern's terminator (`parse_optimized_patterns`,
/// `src/libgit2/attr_file.c`), which is undefined behaviour. An empty or all-empty
/// array is never parsed. Every wrapper handing libgit2 a pathspec array checks it.
pub(crate) fn patterns_are_parsable(patterns: StrArrayRef<'_>) -> bool {
    let mixed = patterns.iter().any(CStr::is_empty) && patterns.iter().any(|p| !p.is_empty());
    if mixed {
        set(
            ErrorClass::Invalid,
            c"invalid pathspec: an empty pattern beside a non-empty one",
        );
    }
    !mixed
}

/// Combinable path-matching options. Unknown bits can be retained for forward
/// compatibility; this C version ignores them. `IGNORE_CASE` takes precedence over
/// `USE_CASE` where case flags apply. These are bits, not a Rust enum: combinations
/// (including zero and both case flags) are valid.
///
/// Wraps: `git_pathspec_flag_t`
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PathspecFlags(u32);

impl PathspecFlags {
    /// Default matching, with no flags enabled.
    pub const DEFAULT: Self = Self(ffi::GIT_PATHSPEC_DEFAULT);
    /// Do not keep matching filenames; test for matches or collect failures only.
    pub const FAILURES_ONLY: Self = Self(ffi::GIT_PATHSPEC_FAILURES_ONLY);
    /// Record the patterns that had no matches.
    pub const FIND_FAILURES: Self = Self(ffi::GIT_PATHSPEC_FIND_FAILURES);
    /// Force case-insensitive matching.
    pub const IGNORE_CASE: Self = Self(ffi::GIT_PATHSPEC_IGNORE_CASE);
    /// Disable glob matching (literal directory-prefix matching still applies).
    pub const NO_GLOB: Self = Self(ffi::GIT_PATHSPEC_NO_GLOB);
    /// Report no matches as `GIT_ENOTFOUND` in list-producing operations.
    pub const NO_MATCH_ERROR: Self = Self(ffi::GIT_PATHSPEC_NO_MATCH_ERROR);
    /// Force case-sensitive matching where the operation supports it.
    pub const USE_CASE: Self = Self(ffi::GIT_PATHSPEC_USE_CASE);

    /// Preserve every supplied bit, including bits unknown to this C version.
    #[must_use]
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// The bitset passed to the C matching operations.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every bit in `other` is enabled.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no bits are enabled.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for PathspecFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for PathspecFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for PathspecFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for PathspecFlags {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

/// An opaque pathspec match list, owned by [`MatchListBox`].
///
/// `'diff` keeps borrowed diff deltas alive and unchanged. Filename lists own their
/// strings and have no source lifetime. Failed patterns always belong to the list.
/// The layout is never instantiated by value: only C constructs this opaque type.
///
/// A string cannot outlive its list:
/// ```compile_fail,E0505
/// use libgit2::libgit2::src::libgit2::pathspec::MatchListBox;
/// fn invalid(list: MatchListBox<'static>) {
///     let entry = list.as_ref().entry(0).unwrap();
///     drop(list);
///     println!("{entry:?}");
/// }
/// ```
/// Exclusive reborrows exclude shared access:
/// ```compile_fail,E0502
/// use libgit2::libgit2::src::libgit2::pathspec::MatchListBox;
/// fn invalid(mut list: MatchListBox<'static>) {
///     let exclusive = list.as_mut();
///     let shared = list.as_ref();
///     assert_eq!(exclusive.as_ref().len(), shared.len());
/// }
/// ```
/// Exclusive handles are invariant in the diff lifetime:
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::pathspec::MatchListMut;
/// fn invalid<'a, 'd>(m: MatchListMut<'a, 'static>) -> MatchListMut<'a, 'd> { m }
/// ```
/// Owners remain thread-bound:
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::pathspec::MatchListBox;
/// fn invalid(list: MatchListBox<'static>) {
///     std::thread::spawn(move || drop(list));
/// }
/// ```
///
/// Wraps: `git_pathspec_match_list`
/// Handwritten
#[repr(transparent)]
pub struct MatchList<'diff>(
    ffi::git_pathspec_match_list,
    PhantomData<(&'diff (), *const ())>,
);

/// Copyable shared handle; no Rust reference covers C-owned list storage.
///
/// Handwritten
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct MatchListRef<'a, 'diff>(CBorrowedPtr<'a, MatchList<'diff>>);

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
    type Mut<'a>
        = MatchListMut<'a, 'diff>
    where
        Self: 'a;
    type Ref<'a>
        = MatchListRef<'a, 'diff>
    where
        Self: 'a;
}

impl<'a, 'diff: 'a> CRefSeam<'a> for MatchListRef<'a, 'diff> {
    type Cell = MatchList<'diff>;
}

impl<'a, 'diff: 'a> CMutSeam<'a> for MatchListMut<'a, 'diff> {
    type Cell = MatchList<'diff>;
}

/// Releases the list, its arrays, string pool and retained pathspec reference.
/// Borrowed diff deltas are never freed. No C clone operation exists.
///
/// Wraps: `git_pathspec_match_list_free`
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

impl<'a> MatchListRef<'a, '_> {
    /// Number of filenames or diff deltas; zero with `FAILURES_ONLY`.
    ///
    /// Wraps: `git_pathspec_match_list_entrycount`
    #[must_use]
    pub fn len(&self) -> usize {
        // SAFETY: this handle keeps the initialized list alive and unwritten.
        unsafe { ffi::git_pathspec_match_list_entrycount(self.as_ptr()) }
    }

    /// Whether no path matched.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Matched filename, or `None` out of bounds or for a diff-generated list.
    ///
    /// Wraps: `git_pathspec_match_list_entry`
    #[must_use]
    pub fn entry(&self, index: usize) -> Option<&'a CStr> {
        // SAFETY: C checks the datatype and index before reading its array. Strings
        // are NUL-terminated pool allocations, immutable until this list is freed.
        let p = unsafe { ffi::git_pathspec_match_list_entry(self.as_ptr(), index) };
        // SAFETY: non-null results belong to the list for the handle's lifetime.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) })
    }

    /// Number of unmatched patterns; zero unless `FIND_FAILURES` was requested.
    ///
    /// Wraps: `git_pathspec_match_list_failed_entrycount`
    #[must_use]
    pub fn failed_len(&self) -> usize {
        // SAFETY: the shared handle keeps the initialized list alive.
        unsafe { ffi::git_pathspec_match_list_failed_entrycount(self.as_ptr()) }
    }

    /// A copied unmatched pattern, or `None` out of bounds.
    ///
    /// Wraps: `git_pathspec_match_list_failed_entry`
    #[must_use]
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
    /// Wraps: `git_pathspec_match_list_diff_entry`
    #[must_use]
    pub fn diff_entry(&self, index: usize) -> Option<DiffDeltaRef<'a>> {
        // SAFETY: C checks datatype and bounds; the list and borrowed diff are live.
        let pathspec_match_list_diff_entry =
            unsafe { ffi::git_pathspec_match_list_diff_entry(self.as_ptr(), index) };
        // SAFETY: as above.
        unsafe { DiffDeltaRef::from_ptr(pathspec_match_list_diff_entry.cast_mut()) }
    }
}

impl<'diff> MatchListMut<'_, 'diff> {
    /// A shared handle over the same object, borrowed from this one.
    #[must_use]
    pub fn as_ref(&self) -> MatchListRef<'_, 'diff> {
        self.0
    }

    /// An exclusive handle over the same object, reborrowed from this one.
    pub fn as_mut(&mut self) -> MatchListMut<'_, 'diff> {
        MatchListMut(self.0, PhantomData)
    }
}

impl MatchList<'static> {
    /// Match an index, copying filenames and failed patterns into the result.
    ///
    /// The index iterator may sort its source, so it receives an exclusive
    /// handle. The list takes its own counted reference to `pathspec`, which is
    /// why the argument is the shared owner: a [`PathspecBox`] converts into one.
    ///
    /// A sole owner is not accepted as is:
    /// ```compile_fail,E0308
    /// use libgit2::libgit2::src::libgit2::index::IndexMut;
    /// use libgit2::libgit2::src::libgit2::pathspec::{MatchList, PathspecBox, PathspecFlags};
    /// fn invalid(index: IndexMut<'_>, sole: PathspecBox) {
    ///     let _ = MatchList::from_index(index, &sole, PathspecFlags::DEFAULT);
    /// }
    /// ```
    ///
    /// Wraps: `git_pathspec_match_index`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when the index cannot be iterated, and with
    /// [`ErrorCode::NotFound`](crate::libgit2::src::util::errors::ErrorCode::NotFound) under
    /// [`PathspecFlags::NO_MATCH_ERROR`] when no path matches.
    pub fn from_index(
        mut index: IndexMut<'_>,
        pathspec: &PathspecArc,
        flags: PathspecFlags,
    ) -> Result<MatchListBox<'static>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: exclusive source index and a live shared pathspec owner, whose
        // patterns C only reads while atomically bumping their count; writable out
        // slot. C copies all names and releases its iterator before returning.
        let rc = unsafe {
            ffi::git_pathspec_match_index(
                &raw mut out,
                index.as_mut_ptr(),
                flags.bits(),
                pathspec.as_ref().as_ptr().cast_mut(),
            )
        };
        // SAFETY: success transfers a fresh initialized list, released by
        // MatchListFree; failure frees and nulls any list it allocated (or leaves
        // our initial NULL unchanged), adopted as None.
        constructed(rc, unsafe { MatchListBox::from_c(out) })
    }

    /// Match a tree; the result copies filenames and need not retain the tree.
    ///
    /// Wraps: `git_pathspec_match_tree`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when the tree cannot be iterated, and with
    /// [`ErrorCode::NotFound`](crate::libgit2::src::util::errors::ErrorCode::NotFound) under
    /// [`PathspecFlags::NO_MATCH_ERROR`] when no path matches.
    pub fn from_tree(
        tree: TreeRef<'_, '_>,
        pathspec: &PathspecArc,
        flags: PathspecFlags,
    ) -> Result<MatchListBox<'static>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: tree and repository kept alive by the handle; immutable tree
        // traversal, a live shared pathspec owner (read, and its count atomically
        // bumped, by C), and a writable output slot.
        let rc = unsafe {
            ffi::git_pathspec_match_tree(
                &raw mut out,
                tree.as_ptr().cast_mut(),
                flags.bits(),
                pathspec.as_ref().as_ptr().cast_mut(),
            )
        };
        // SAFETY: fresh initialized list on success, released by MatchListFree;
        // NULL on failure, adopted as None.
        constructed(rc, unsafe { MatchListBox::from_c(out) })
    }

    /// Match the working directory, excluding ignored untracked files. A bare
    /// repository is an error. Filenames in the result are independent copies.
    ///
    /// Wraps: `git_pathspec_match_workdir`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::BareRepo`](crate::libgit2::src::util::errors::ErrorCode::BareRepo)
    /// for a bare repository, with libgit2's code and error when the working directory cannot
    /// be read,
    /// and with [`ErrorCode::NotFound`](crate::libgit2::src::util::errors::ErrorCode::NotFound)
    /// under [`PathspecFlags::NO_MATCH_ERROR`] when no path matches.
    pub fn from_workdir(
        mut repo: RepositoryMut<'_>,
        pathspec: &PathspecArc,
        flags: PathspecFlags,
    ) -> Result<MatchListBox<'static>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: exclusive repository handle and a live shared pathspec owner (read,
        // and its count atomically bumped, by C); C copies names, retains no
        // repository, and writes only our output slot.
        let rc = unsafe {
            ffi::git_pathspec_match_workdir(
                &raw mut out,
                repo.as_mut_ptr(),
                flags.bits(),
                pathspec.as_ref().as_ptr().cast_mut(),
            )
        };
        // SAFETY: fresh initialized list on success, released by MatchListFree;
        // NULL on failure, adopted as None.
        constructed(rc, unsafe { MatchListBox::from_c(out) })
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
    /// use libgit2::libgit2::src::libgit2::strarray::StrArrayView;
    /// use libgit2::libgit2::src::libgit2::diff::Diff;
    /// use libgit2::libgit2::src::libgit2::pathspec::{MatchList, Pathspec, PathspecArc, PathspecFlags};
    /// let ps: PathspecArc = Pathspec::new(StrArrayView::default().as_ref()).unwrap().into();
    /// let list;
    /// {
    ///     let diff = Diff::from_buffer(b"").unwrap();
    ///     list = MatchList::from_diff(diff.as_ref(), &ps, PathspecFlags::DEFAULT);
    /// }
    /// drop(list);
    /// ```
    ///
    /// Wraps: `git_pathspec_match_diff`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when it runs out of memory, and with
    /// [`ErrorCode::NotFound`](crate::libgit2::src::util::errors::ErrorCode::NotFound) under
    /// [`PathspecFlags::NO_MATCH_ERROR`] when no path matches.
    pub fn from_diff(
        diff: DiffRef<'diff, '_>,
        pathspec: &PathspecArc,
        flags: PathspecFlags,
    ) -> Result<MatchListBox<'diff>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: live diff handle retaining deltas for 'diff, a live shared
        // pathspec owner, and a writable output slot. C reads the pathspec and
        // retains it via its atomic count, but merely borrows delta pointers.
        let rc = unsafe {
            ffi::git_pathspec_match_diff(
                &raw mut out,
                diff.as_ptr().cast_mut(),
                flags.bits(),
                pathspec.as_ref().as_ptr().cast_mut(),
            )
        };
        // SAFETY: fresh initialized list on success, NULL on failure (adopted as
        // None); its borrowed deltas stay alive and unchanged for 'diff, which the
        // shared diff handle guarantees.
        constructed(rc, unsafe { MatchListBox::from_c(out) })
    }
}

/// The list a matcher returning `rc` handed out, or its error. Every matcher leaves its
/// output NULL on failure, so a failure adopts nothing.
fn constructed(
    rc: i32,
    list: Option<MatchListBox<'_>>,
) -> Result<MatchListBox<'_>, RepositoryError> {
    if rc < 0 {
        return Err(RepositoryError::from_libgit2(rc));
    }
    list.ok_or_else(|| RepositoryError::from_libgit2(-1))
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
        use crate::libgit2::src::libgit2::index::Index;
        use crate::libgit2::src::libgit2::strarray::StrArrayView;
        let mut index = Index::new().unwrap();
        let patterns = StrArrayView::new(&[c"missing"]);
        let ps: PathspecArc = Pathspec::new(patterns.as_ref()).unwrap().into();
        let list =
            MatchList::from_index(index.as_mut(), &ps, PathspecFlags::FIND_FAILURES).unwrap();
        // Release the original reference; the list holds another.
        drop(ps);
        let raw = list.into_c();
        // SAFETY: the fresh list was just surrendered, with no remaining handles;
        // it owns copied strings and needs no external diff lifetime.
        let mut list: MatchListBox<'static> = unsafe { MatchListBox::from_c(raw) }.unwrap();
        drop(index);
        assert_eq!(list.as_ref().as_ptr(), raw.cast_const());
        // Intentionally exercise Clone, not only the equivalent Copy operation.
        #[expect(
            clippy::clone_on_copy,
            reason = "exercises `Clone` itself, not the `Copy` it shadows"
        )]
        let cloned = list.as_ref().clone();
        assert_eq!(cloned.failed_entry(0), Some(c"missing"));
        assert_eq!(
            list.as_mut().as_mut().as_ref().failed_entry(0),
            Some(c"missing")
        );
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::src::libgit2::strarray::StrArrayView;

    #[test]
    fn flag_algebra_and_layout() {
        assert_eq!(
            size_of::<PathspecFlags>(),
            size_of::<ffi::git_pathspec_flag_t>()
        );
        assert_eq!(
            align_of::<PathspecFlags>(),
            align_of::<ffi::git_pathspec_flag_t>()
        );
        assert_eq!(size_of::<Pathspec>(), size_of::<ffi::git_pathspec>());
        assert_eq!(align_of::<Pathspec>(), align_of::<ffi::git_pathspec>());
        assert_eq!(
            size_of::<PathspecBox>(),
            size_of::<*mut ffi::git_pathspec>()
        );
        assert_eq!(size_of::<PathspecArc>(), size_of::<PathspecBox>());
        assert_eq!(size_of::<PathspecRef<'_>>(), size_of::<PathspecBox>());
        assert_eq!(size_of::<PathspecMut<'_>>(), size_of::<PathspecBox>());
        assert!(PathspecFlags::default().is_empty());
        let mut flags = PathspecFlags::IGNORE_CASE | PathspecFlags::NO_GLOB;
        assert!(flags.contains(PathspecFlags::IGNORE_CASE));
        assert!(!flags.contains(PathspecFlags::USE_CASE));
        flags |= PathspecFlags::USE_CASE;
        flags &= PathspecFlags::IGNORE_CASE | PathspecFlags::USE_CASE;
        assert_eq!(flags.bits(), 3);
        assert_eq!((flags & PathspecFlags::NO_GLOB).bits(), 0);
        assert_eq!(PathspecFlags::from_bits_retain(u32::MAX).bits(), u32::MAX);
        assert!(format!("{flags:?}").contains("PathspecFlags"));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn raw_seams_preserve_single_release() {
        let patterns = StrArrayView::new(&[c"src/*.rs"]);
        let owner = Pathspec::new(patterns.as_ref()).unwrap();
        let raw = owner.into_c();
        // SAFETY: the sole initialized reference just surrendered above is adopted
        // exactly once, with the same release policy, without a concurrent borrower.
        let mut owner = unsafe { PathspecBox::from_c(raw) }.unwrap();
        assert_eq!(owner.as_ref().as_ptr(), raw.cast_const());
        assert_eq!(owner.as_mut().as_mut_ptr(), raw);
        // SAFETY: null transfers no ownership and yields None.
        assert!(unsafe { PathspecBox::from_c(ptr::null_mut()) }.is_none());
        let arc: PathspecArc = owner.into();
        assert_eq!(arc.as_ref().as_ptr(), raw.cast_const());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn counted_c_list_outlives_rust_owner() {
        let patterns = StrArrayView::new(&[c"missing/*"]);
        let owner: PathspecArc = Pathspec::new(patterns.as_ref()).unwrap().into();
        let mut index = crate::libgit2::src::libgit2::index::Index::new().unwrap();
        let list =
            MatchList::from_index(index.as_mut(), &owner, PathspecFlags::FIND_FAILURES).unwrap();
        drop(index);
        drop(owner); // C's list is now the only counted reference.
        drop(patterns);
        assert_eq!(list.as_ref().failed_entry(0), Some(c"missing/*"));
        // Dropping the list releases the last pathspec reference with its storage.
        drop(list);
    }

    /// Only an empty pattern beside a non-empty one is refused, before libgit2 parses
    /// the array, with an error naming the defect; the error is cleared otherwise.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mixed_empty_patterns_are_refused_before_libgit2() {
        for patterns in [
            &[c"a", c""][..],
            &[c"", c"src/*"][..],
            &[c"#x", c"", c"!"][..],
        ] {
            let view = StrArrayView::new(patterns);
            let error = Pathspec::new(view.as_ref()).err().unwrap();
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
            assert_eq!(
                error.message(),
                c"invalid pathspec: an empty pattern beside a non-empty one"
            );
        }
        for patterns in [&[][..], &[c""][..], &[c"", c""][..], &[c"a", c"#x"][..]] {
            let view = StrArrayView::new(patterns);
            assert!(patterns_are_parsable(view.as_ref()));
            assert!(Pathspec::new(view.as_ref()).is_ok());
        }
    }

    // Allocation fault injection lives in a fresh process: no other Rust test can
    // observe or race the process-global C allocator replacement.
    mod failures {
        use core::ffi::CStr;
        use core::ptr::NonNull;
        use std::sync::atomic::AtomicIsize;
        use std::sync::atomic::Ordering::SeqCst;

        use super::*;
        use crate::libgit2::src::util::alloc::{Alloc, Allocator};

        static FAIL_AT: AtomicIsize = AtomicIsize::new(-1);
        static LIVE: AtomicIsize = AtomicIsize::new(0);
        struct Failing;

        // SAFETY: these callbacks use one libc allocator family throughout and
        // reject allocations without changing ownership of existing allocations.
        unsafe impl Alloc for Failing {
            fn malloc(n: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                if FAIL_AT.load(SeqCst) == isize::MAX || FAIL_AT.fetch_sub(1, SeqCst) == 0 {
                    return None;
                }
                // SAFETY: libc allocates a fresh block of the requested size.
                let p = NonNull::new(unsafe { libc::malloc(n) }.cast());
                if p.is_some() {
                    LIVE.fetch_add(1, SeqCst);
                }
                p
            }

            unsafe fn realloc(
                p: Option<NonNull<u8>>,
                n: usize,
                _: Option<&CStr>,
                _: i32,
            ) -> Option<NonNull<u8>> {
                if FAIL_AT.load(SeqCst) == isize::MAX || FAIL_AT.fetch_sub(1, SeqCst) == 0 {
                    return None;
                }
                let old = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
                // SAFETY: caller transfers a live libc block (or null) on success;
                // libgit2 never requests realloc of a live block to zero bytes.
                let result = NonNull::new(unsafe { libc::realloc(old, n) }.cast());
                if p.is_none() && result.is_some() {
                    LIVE.fetch_add(1, SeqCst);
                }
                result
            }

            unsafe fn free(p: NonNull<u8>) {
                LIVE.fetch_sub(1, SeqCst);
                // SAFETY: caller surrenders one live block of this allocator.
                unsafe { libc::free(p.as_ptr().cast()) };
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2 and spawns child")]
        fn construction_failure_cleanup() {
            const CHILD: &str = "CRUSTIFY_PATHSPEC_FAILURE_CHILD";
            if std::env::var_os(CHILD).is_none() {
                let name = concat!(module_path!(), "::construction_failure_cleanup");
                let name = name.split_once("::").unwrap().1;
                for mode in ["init", "allocation"] {
                    let output = std::process::Command::new(std::env::current_exe().unwrap())
                        .args([name, "--exact", "--nocapture"])
                        .env(CHILD, mode)
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{}\n{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
                }
                return;
            }
            let allocator = Allocator::new::<Failing>();
            // SAFETY: this fresh process has never initialized libgit2; the valid
            // allocator table is copied by C before any allocations are made.
            let rc = unsafe {
                ffi::git_libgit2_opts(
                    ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                    allocator.as_ref().as_ptr(),
                )
            };
            assert_eq!(rc, 0);
            if std::env::var(CHILD).as_deref() == Ok("init") {
                FAIL_AT.store(isize::MAX, SeqCst);
                let view = StrArrayView::default();
                assert!(Pathspec::new(view.as_ref()).is_err());
                FAIL_AT.store(-1, SeqCst);
                assert!(!runtime_pinned(), "failed initialization stays failed");
                return;
            }
            assert!(runtime_pinned());
            // Warm up the thread's error storage before counting live allocations.
            set(ErrorClass::Invalid, c"warm up");
            let patterns = StrArrayView::new(&[c"src/*.rs", c"src/lib/*.c", c"src/other"]);
            let baseline = LIVE.load(SeqCst);
            let mut errors = 0;
            let mut successes = 0;
            for at in 0..20 {
                FAIL_AT.store(at, SeqCst);
                let result = Pathspec::new(patterns.as_ref());
                FAIL_AT.store(-1, SeqCst);
                if result.is_err() {
                    errors += 1;
                } else {
                    successes += 1;
                }
                drop(result);
                assert_eq!(
                    LIVE.load(SeqCst),
                    baseline,
                    "allocation failure {at} leaked"
                );
            }
            assert!(errors > 0 && successes > 0);
        }
    }
}
