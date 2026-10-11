//! Safe wrappers over `include/git2/sys/index.h`.
//!
//! NAME entries and their strings are allocated and freed by the index. The public
//! API lends them read-only; it publishes no standalone constructor, destructor or
//! clone. `git_index_name_add` duplicates its inputs, while clearing or releasing
//! the index destroys its entries. No public C operation transfers standalone
//! ownership of an entry or permits mutation of a returned entry.

use core::ffi::CStr;
use core::ptr::{self, addr_of};

use ffibox::define_ctype;

use crate::ffi;
use crate::libgit2::index::index::{IndexMut, IndexRef};
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, last};
use crate::libgit2::util::sys::errors::set;

define_ctype!(
    /// A rename-conflict record owned by an index. Obtain its read-only handle with
    /// [`IndexMut::name_entry`]; copy individual strings with `CStr::to_owned` if
    /// they must survive index mutation. The layout and exclusive handle are only
    /// for internal FFI interoperability, not independently constructed owners.
    ///
    /// Wraps: git_index_name_entry
    NameEntry, NameEntryRef, NameEntryMut, ffi::git_index_name_entry
);

impl<'a> NameEntryRef<'a> {
    /// The original path, absent for a conflict without an ancestor name.
    ///
    /// Field: git_index_name_entry.ancestor
    pub fn ancestor(&self) -> Option<&'a CStr> {
        // SAFETY: the handle keeps this initialized entry and its immutable,
        // nullable NUL-terminated strings alive for 'a; projection forms no reference
        // to the C struct. Non-null strings remain unmodified for the same borrow.
        unsafe {
            let p = addr_of!((*self.as_ptr()).ancestor).read();
            if p.is_null() {
                None
            } else {
                Some(CStr::from_ptr(p))
            }
        }
    }

    /// Our path, or `None` when this side has no name.
    ///
    /// Field: git_index_name_entry.ours
    pub fn ours(&self) -> Option<&'a CStr> {
        // SAFETY: as for ancestor: this field is null or a live immutable C string
        // owned by the index, which cannot change during the handle's borrow.
        unsafe {
            let p = addr_of!((*self.as_ptr()).ours).read();
            if p.is_null() {
                None
            } else {
                Some(CStr::from_ptr(p))
            }
        }
    }

    /// Their path, or `None` when this side has no name.
    ///
    /// Field: git_index_name_entry.theirs
    pub fn theirs(&self) -> Option<&'a CStr> {
        // SAFETY: as for ancestor: the initialized field points to an immutable
        // NUL-terminated string kept alive for 'a, or is null.
        unsafe {
            let p = addr_of!((*self.as_ptr()).theirs).read();
            if p.is_null() {
                None
            } else {
                Some(CStr::from_ptr(p))
            }
        }
    }
}

impl IndexRef<'_> {
    /// Number of rename-conflict records currently held by the index.
    ///
    /// Wraps: git_index_name_entrycount
    pub fn name_entry_count(&self) -> usize {
        // SAFETY: the index is live; this C routine only reads its vector length
        // despite its non-const parameter, and returns no retained borrow.
        unsafe { ffi::git_index_name_entrycount(self.as_ptr().cast_mut()) }
    }
}

impl IndexMut<'_> {
    /// Duplicate names into a new index-owned rename-conflict record. At least two
    /// names must be present, as required by C. Empty strings and non-UTF-8 names
    /// are preserved in memory; no pointer into the inputs is retained.
    ///
    /// Wraps: git_index_name_add
    pub fn add_name(
        &mut self,
        ancestor: Option<&CStr>,
        ours: Option<&CStr>,
        theirs: Option<&CStr>,
    ) -> Result<(), ErrorBuf> {
        if [ancestor, ours, theirs].iter().flatten().count() < 2 {
            set(
                ErrorClass::Invalid,
                c"a rename conflict requires at least two names",
            );
            return Err(last());
        }
        // SAFETY: the index is exclusive and initialized; at least two inputs are
        // non-null C strings. C copies them before returning and cleans up partial
        // allocations on failure. The owning index has pinned the runtime.
        let rc = unsafe {
            ffi::git_index_name_add(
                self.as_mut_ptr(),
                ancestor.map_or(ptr::null(), CStr::as_ptr),
                ours.map_or(ptr::null(), CStr::as_ptr),
                theirs.map_or(ptr::null(), CStr::as_ptr),
            )
        };
        if rc < 0 { Err(last()) } else { Ok(()) }
    }

    /// Read a record in C's sort order; out-of-range positions return `None`.
    /// Requires exclusive index access because C sorts the NAME vector lazily.
    /// Neither the record nor its strings can outlive this borrow or survive a
    /// mutation through another index handle.
    ///
    /// ```compile_fail,E0499
    /// use libgit2::libgit2::index::index::Index;
    /// let mut index = Index::new().unwrap();
    /// let mut handle = index.as_mut();
    /// let entry = handle.name_entry(0).unwrap();
    /// handle.clear_names().unwrap();
    /// let _ = entry.ancestor();
    /// ```
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::index::index::Index;
    /// let mut index = Index::new().unwrap();
    /// let mut handle = index.as_mut();
    /// let name = handle.name_entry(0).unwrap().ours().unwrap();
    /// drop(index);
    /// let _ = name.to_bytes();
    /// ```
    ///
    /// Wraps: git_index_name_get_byindex
    pub fn name_entry(&mut self, position: usize) -> Option<NameEntryRef<'_>> {
        // SAFETY: the exclusive index borrow permits sorting and excludes clearing,
        // reloading, and freeing until the returned borrow ends. C returns null or
        // one initialized, index-owned entry; no Rust reference covers it.
        unsafe {
            NameEntryRef::from_ptr(
                ffi::git_index_name_get_byindex(self.as_mut_ptr(), position).cast_mut(),
            )
        }
    }

    /// Destroy all rename-conflict records and their strings, leaving the index
    /// reusable. Existing record/string borrows must end before this call.
    ///
    /// Wraps: git_index_name_clear
    pub fn clear_names(&mut self) -> Result<(), ErrorBuf> {
        // SAFETY: exclusive access excludes every borrowed NAME entry; C disposes
        // the owned entries and resets the vector, retaining the index itself.
        let rc = unsafe { ffi::git_index_name_clear(self.as_mut_ptr()) };
        if rc < 0 { Err(last()) } else { Ok(()) }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn layout_and_nullable_projection() {
        assert_eq!(
            size_of::<NameEntry>(),
            size_of::<ffi::git_index_name_entry>()
        );
        assert_eq!(
            align_of::<NameEntry>(),
            align_of::<ffi::git_index_name_entry>()
        );
        let raw = ffi::git_index_name_entry {
            ancestor: ptr::null_mut(),
            ours: c"".as_ptr().cast_mut(),
            theirs: c"\xff".as_ptr().cast_mut(),
        };
        // SAFETY: the raw entry is initialized local storage; all non-null pointers
        // are immutable static C strings. Neither storage nor strings change while
        // the handle is used; it does not assume ownership.
        let view = unsafe { NameEntryRef::from_ptr(ptr::from_ref(&raw).cast_mut()) }.unwrap();
        assert_eq!(view.ancestor(), None);
        assert_eq!(view.ours(), Some(c""));
        assert_eq!(view.theirs(), Some(c"\xff"));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn missing_names_rejected_before_ffi() {
        let mut index = crate::libgit2::index::index::Index::new().unwrap();
        for args in [
            [None, None, None],
            [Some(c"a"), None, None],
            [None, Some(c"o"), None],
            [None, None, Some(c"t")],
        ] {
            let err = index
                .as_mut()
                .add_name(args[0], args[1], args[2])
                .unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Invalid));
            assert_eq!(index.as_ref().name_entry_count(), 0);
        }
    }
}
