//! Safe wrappers over `include/git2/sys/index.h`.
//!
//! NAME entries and their strings are allocated and freed by the index. The public
//! API lends them read-only; it publishes no standalone constructor, destructor or
//! clone. `git_index_name_add` duplicates its inputs, while clearing or releasing
//! the index destroys its entries. No public C operation transfers standalone
//! ownership of an entry or permits mutation of a returned entry.
//!
//! REUC (resolve-undo) entries follow the same model. The index allocates each one
//! with its path in the same block (`reuc_entry_alloc`, from `git_index_reuc_add` or
//! the index file reader) and frees it with that block (`index_entry_reuc_free`) when
//! a same-path add replaces it, on remove, clear, reload, and when the index is
//! released. The public API only lends const entries.

use core::ffi::{CStr, c_int};
use core::fmt;
use core::ptr::{self, NonNull, addr_of};

use ffibox::{CSlice, define_ctype};

use crate::ffi;
use crate::libgit2::core::types::FileMode;
use crate::libgit2::index::index::{IndexMut, IndexRef};
use crate::libgit2::oid::oid::{Oid, OidRef};
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, last};
use crate::libgit2::util::sys::errors::{clear, set};

define_ctype!(
    /// A rename-conflict record owned by an index. Obtain its read-only handle with
    /// [`IndexMut::name_entry`]; copy individual strings with `CStr::to_owned` if
    /// they must survive index mutation. The layout and exclusive handle are only
    /// for internal FFI interoperability, not independently constructed owners.
    ///
    /// Wraps: git_index_name_entry
    NameEntry,
    NameEntryRef,
    NameEntryMut,
    ffi::git_index_name_entry
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

define_ctype!(
    /// A resolve-undo (REUC) record owned by an index: the path of a conflict that
    /// was resolved, and the mode and blob id each side had before resolution.
    /// Obtain its read-only handle with [`IndexMut::reuc_entry`] or
    /// [`IndexMut::reuc_entry_by_path`]; copy what must survive index mutation. The
    /// layout and exclusive handle are only for internal FFI interoperability, not
    /// independently constructed owners: libgit2 publishes no constructor, clone or
    /// destructor for a standalone entry.
    ///
    /// Like the index, a record never crosses threads:
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::index::index::Index;
    /// let mut index = Index::new().unwrap();
    /// let handle = index.as_mut();
    /// let entry = handle.reuc_entry(0).unwrap();
    /// std::thread::scope(|s| {
    ///     s.spawn(move || entry.path());
    /// });
    /// ```
    ///
    /// Wraps: git_index_reuc_entry
    ReucEntry,
    ReucEntryRef,
    ReucEntryMut,
    ffi::git_index_reuc_entry
);

/// One side of a resolved conflict as a REUC record keeps it.
#[derive(Clone, Copy, Debug)]
pub struct ReucSide<'a> {
    /// The file mode the side had, as stored: any non-zero 32-bit value read from
    /// an index file or passed to `git_index_reuc_add`, not only the modes
    /// [`FileMode`](crate::libgit2::core::types::FileMode) names.
    pub mode: u32,
    /// The side's blob id, borrowed from the record.
    pub id: OidRef<'a>,
}

impl<'a> ReucSide<'a> {
    /// A side of mode `mode`, one of the modes libgit2 names, and id `id`, as
    /// [`IndexMut::add_reuc`] takes it.
    pub fn new(mode: FileMode, id: OidRef<'a>) -> Self {
        Self {
            mode: mode.raw(),
            id,
        }
    }
}

impl<'a> ReucEntryRef<'a> {
    /// The resolved path. Never NULL: libgit2 stores it in the record's own
    /// allocation, so it lives exactly as long as the record.
    ///
    /// Field: git_index_reuc_entry.path
    pub fn path(&self) -> &'a CStr {
        // SAFETY: the handle keeps this initialized, index-owned entry alive and
        // unmodified for 'a. Both allocators point `path` at the NUL-terminated tail
        // of the entry's own block, never NULL, and nothing writes through it.
        unsafe { CStr::from_ptr(addr_of!((*self.as_ptr()).path).read()) }
    }

    /// The ancestor's, our and their stored modes, in that order; 0 marks a side
    /// that did not exist.
    ///
    /// Field: git_index_reuc_entry.mode
    pub fn modes(&self) -> [u32; 3] {
        // SAFETY: a read through the shared handle's pointer to a live entry; any
        // three `u32`s are valid.
        unsafe { addr_of!((*self.as_ptr()).mode).read() }
    }

    /// The ancestor's, our and their ids, in that order, borrowed in place. A slot
    /// whose mode is 0 carries no id: libgit2 leaves it all zero (typeless).
    ///
    /// Field: git_index_reuc_entry.oid
    pub fn ids(&self) -> CSlice<'a, Oid> {
        // SAFETY: the field projected from the shared handle's pointer to a live entry
        // is three initialised `git_oid`s (`Oid` is transparent over one), non-NULL,
        // which live and stay unmodified for 'a with the entry; the run only reads.
        unsafe {
            let ids = addr_of!((*self.as_ptr()).oid).cast::<Oid>().cast_mut();
            CSlice::from_raw_parts(NonNull::new_unchecked(ids), 3)
        }
    }

    /// The common ancestor's side, or `None` when it had no such file.
    pub fn ancestor(&self) -> Option<ReucSide<'a>> {
        self.side(0)
    }

    /// Our side, or `None` when it had no such file.
    pub fn ours(&self) -> Option<ReucSide<'a>> {
        self.side(1)
    }

    /// Their side, or `None` when it had no such file.
    pub fn theirs(&self) -> Option<ReucSide<'a>> {
        self.side(2)
    }

    /// Slot `i`, present exactly when its mode is non-zero: libgit2 copies (and
    /// writes) an id only for those.
    fn side(&self, i: usize) -> Option<ReucSide<'a>> {
        let mode = self.modes()[i];
        if mode == 0 {
            return None;
        }
        Some(ReucSide {
            mode,
            id: self.ids().get(i)?,
        })
    }
}

impl fmt::Debug for ReucEntryRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReucEntry")
            .field("path", &self.path())
            .field("ancestor", &self.ancestor())
            .field("ours", &self.ours())
            .field("theirs", &self.theirs())
            .finish()
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

    /// Read the REUC record at `position`, in path order; out-of-range positions
    /// return `None`. Records stay valid while this handle is only shared, so several
    /// may be held at once, but none survives a mutation through it or the index's
    /// release.
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::index::index::Index;
    /// let mut index = Index::new().unwrap();
    /// let mut handle = index.as_mut();
    /// let entry = handle.reuc_entry(0).unwrap();
    /// handle.clear_names().unwrap();
    /// let _ = entry.path();
    /// ```
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::index::index::Index;
    /// let mut index = Index::new().unwrap();
    /// let handle = index.as_mut();
    /// let path = handle.reuc_entry(0).unwrap().path();
    /// drop(index);
    /// let _ = path.to_bytes();
    /// ```
    ///
    /// A shared index may change behind any of its holders, so its shared handle
    /// lends no records:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::index::index::{Index, IndexArc};
    /// let index: IndexArc = Index::new().unwrap().into();
    /// let _ = index.as_ref().reuc_entry(0);
    /// ```
    ///
    /// Wraps: git_index_reuc_get_byindex
    pub fn reuc_entry(&self, position: usize) -> Option<ReucEntryRef<'_>> {
        // SAFETY: the exclusive handle proves no other holder can change the index,
        // and the shared borrow of it excludes clearing, reloading, and freeing until
        // the returned borrow ends. C only reads the vector despite the non-const
        // parameter, returning null or one initialized, index-owned entry (whose
        // address no reordering changes); no Rust reference covers it.
        unsafe {
            ReucEntryRef::from_ptr(
                ffi::git_index_reuc_get_byindex(self.as_ref().as_ptr().cast_mut(), position)
                    .cast_mut(),
            )
        }
    }

    /// Find the REUC record for `path`, or `None`. The search folds case when the
    /// index ignores case. Borrowing is as for [`reuc_entry`](Self::reuc_entry).
    ///
    /// Wraps: git_index_reuc_get_bypath
    pub fn reuc_entry_by_path(&self, path: &CStr) -> Option<ReucEntryRef<'_>> {
        // SAFETY: as in `reuc_entry`; C returns before searching unless the vector is
        // flagged sorted, so it only reads. `path` is a NUL-terminated key read for the
        // binary search and never retained.
        unsafe {
            ReucEntryRef::from_ptr(
                ffi::git_index_reuc_get_bypath(self.as_ref().as_ptr().cast_mut(), path.as_ptr())
                    .cast_mut(),
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

/// The mode `git_index_reuc_add` takes for one side: 0 for an absent side. C stores
/// it in an `unsigned int`, so the wrapping cast keeps every `u32` as given.
fn reuc_mode(side: Option<ReucSide<'_>>) -> c_int {
    side.map_or(0, |side| side.mode as c_int)
}

impl IndexRef<'_> {
    /// Number of resolve-undo (REUC) records currently held by the index: the stages
    /// of conflicts resolved since the index was read.
    ///
    /// Wraps: git_index_reuc_entrycount
    pub fn reuc_entry_count(&self) -> usize {
        // SAFETY: the index is live; this C routine only reads its vector length
        // despite its non-const parameter, and returns no retained borrow.
        unsafe { ffi::git_index_reuc_entrycount(self.as_ptr().cast_mut()) }
    }
}

impl IndexMut<'_> {
    /// Records how the conflict at `path` was before it was resolved: the mode and id
    /// of its ancestor, our and their side, each `None` when the path was absent on
    /// that side. A record for `path` already there is replaced (and freed).
    ///
    /// The path and ids are copied; none is looked up. Any non-zero mode is stored as
    /// given, so the sides a [`ReucEntryRef`] lends add back unchanged; a side whose
    /// mode is 0 is recorded as absent, without its id. Empty and non-UTF-8 paths are
    /// kept as given.
    ///
    /// Fails, with libgit2's error, only when it runs out of memory.
    ///
    /// Wraps: git_index_reuc_add
    pub fn add_reuc(
        &mut self,
        path: &CStr,
        ancestor: Option<ReucSide<'_>>,
        ours: Option<ReucSide<'_>>,
        theirs: Option<ReucSide<'_>>,
    ) -> Result<(), ErrorBuf> {
        let id = |side: Option<ReucSide<'_>>| side.map_or(ptr::null(), |side| side.id.as_ptr());
        // SAFETY: the exclusive handle excludes every borrow of the record replaced;
        // `path` is NUL-terminated and each id live or NULL with mode 0 (never read);
        // C copies them before returning and frees the new record on failure.
        let rc = unsafe {
            ffi::git_index_reuc_add(
                self.as_mut_ptr(),
                path.as_ptr(),
                reuc_mode(ancestor),
                id(ancestor),
                reuc_mode(ours),
                id(ours),
                reuc_mode(theirs),
                id(theirs),
            )
        };
        if rc < 0 { Err(last()) } else { Ok(()) }
    }

    /// The position of the resolve-undo record for `path`, in the order the index
    /// keeps them (by path, case-insensitively when the index ignores case); `None`
    /// when there is none. Positions hold until the records next change.
    ///
    /// Requires exclusive index access because C sorts the REUC vector lazily.
    ///
    /// Wraps: git_index_reuc_find
    pub fn find_reuc(&mut self, path: &CStr) -> Option<usize> {
        let mut position = 0;
        let index = self.as_mut_ptr();
        // SAFETY: `at_pos` is a writable slot; the exclusive borrow permits the sort,
        // and `path` is NUL-terminated, read for the call.
        let rc = unsafe { ffi::git_index_reuc_find(&mut position, index, path.as_ptr()) };
        (rc == 0).then_some(position)
    }

    /// Removes and frees the resolve-undo record at `position`.
    ///
    /// Fails for a position past the last record. libgit2 then returns
    /// `GIT_ENOTFOUND` without an error of its own; the error is a `GIT_ERROR_INDEX`
    /// one describing the position.
    ///
    /// Wraps: git_index_reuc_remove
    pub fn remove_reuc(&mut self, position: usize) -> Result<(), ErrorBuf> {
        clear();
        // SAFETY: the exclusive handle excludes every borrow of the record freed; C
        // bounds-checks the position before removing anything.
        let rc = unsafe { ffi::git_index_reuc_remove(self.as_mut_ptr(), position) };
        if rc >= 0 {
            return Ok(());
        }
        if last().class() == Some(ErrorClass::None) {
            set(
                ErrorClass::Index,
                c"no resolve-undo record at that position",
            );
        }
        Err(last())
    }

    /// Destroy all resolve-undo records, leaving the index reusable. Existing record
    /// borrows must end before this call.
    ///
    /// Wraps: git_index_reuc_clear
    pub fn clear_reuc(&mut self) -> Result<(), ErrorBuf> {
        // SAFETY: exclusive access excludes every borrowed REUC record; C frees the
        // owned records and resets the vector, retaining the index itself.
        let rc = unsafe { ffi::git_index_reuc_clear(self.as_mut_ptr()) };
        if rc < 0 { Err(last()) } else { Ok(()) }
    }
}

#[cfg(test)]
mod unit_tests {
    use ffibox::CZeroable;

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

    fn raw_oid(kind: u32, first: u8) -> ffi::git_oid {
        let mut id = [0; 32];
        id[0] = first;
        id[31] = !first;
        ffi::git_oid {
            type_: kind as _,
            id,
        }
    }

    #[test]
    fn reuc_layout_and_side_projection() {
        assert_eq!(
            size_of::<ReucEntry>(),
            size_of::<ffi::git_index_reuc_entry>()
        );
        assert_eq!(
            align_of::<ReucEntry>(),
            align_of::<ffi::git_index_reuc_entry>()
        );
        let raw = ffi::git_index_reuc_entry {
            mode: [0, 0o100644, u32::MAX],
            oid: [
                raw_oid(0, 0),
                raw_oid(ffi::GIT_OID_SHA1, 0x11),
                raw_oid(ffi::GIT_OID_SHA256, 0x22),
            ],
            path: c"dir/\xff".as_ptr().cast_mut(),
        };
        // SAFETY: the raw entry is initialized local storage whose path is an
        // immutable static C string. Neither changes while the handle is used; it does
        // not assume ownership.
        let view = unsafe { ReucEntryRef::from_ptr(ptr::from_ref(&raw).cast_mut()) }.unwrap();
        let copy = view;
        assert_eq!(view.path(), c"dir/\xff");
        assert_eq!(copy.modes(), [0, 0o100644, u32::MAX]);
        let ids = view.ids();
        assert_eq!(ids.len(), 3);
        // Compare in Rust: `OidRef`'s `PartialEq` calls libgit2.
        for (id, raw) in ids.iter().zip(&raw.oid) {
            assert_eq!(id.raw_kind(), raw.type_);
            assert_eq!(id.id(), raw.id);
        }
        assert!(ids.get(3).is_none());
        assert!(view.ancestor().is_none());
        let ours = view.ours().unwrap();
        assert_eq!(ours.mode, 0o100644);
        assert_eq!(ours.id.raw_kind(), ffi::GIT_OID_SHA1 as u8);
        assert_eq!(ours.id.id()[0], 0x11);
        let theirs = view.theirs().unwrap();
        assert_eq!(theirs.mode, u32::MAX);
        assert_eq!(theirs.id.id()[31], !0x22);
        let debug = format!("{view:?}");
        assert!(debug.starts_with(
            "ReucEntry { path: \"dir/\\xff\", ancestor: None, ours: Some(ReucSide { mode: 33188,"
        ));
    }

    #[test]
    fn reuc_absent_sides_ignore_stale_ids() {
        // A zero mode hides whatever its slot holds; a non-zero one exposes even a
        // typeless id, as C stores and writes it.
        let raw = ffi::git_index_reuc_entry {
            mode: [0, 0, 1],
            oid: [
                raw_oid(ffi::GIT_OID_SHA1, 1),
                raw_oid(ffi::GIT_OID_SHA1, 2),
                raw_oid(0, 0),
            ],
            path: c"".as_ptr().cast_mut(),
        };
        // SAFETY: as above: initialized local storage and a static empty path.
        let view = unsafe { ReucEntryRef::from_ptr(ptr::from_ref(&raw).cast_mut()) }.unwrap();
        assert_eq!(view.path(), c"");
        assert!(view.ancestor().is_none());
        assert!(view.ours().is_none());
        let theirs = view.theirs().unwrap();
        assert_eq!((theirs.mode, theirs.id.raw_kind()), (1, 0));
        assert_eq!(view.ids().get(1).unwrap().id()[0], 2);
        // SAFETY: null pointers adopt as no handle.
        assert!(unsafe { ReucEntryRef::from_ptr(ptr::null_mut()) }.is_none());
    }

    #[test]
    fn absent_and_unreadable_reuc_sides() {
        let id = Oid::zeroed();
        let side = |mode| Some(ReucSide::new(mode, id.as_ref()));
        assert_eq!(reuc_mode(None), 0);
        assert_eq!(reuc_mode(side(FileMode::Unreadable)), 0);
        assert_eq!(reuc_mode(side(FileMode::Commit)), 0o160000);
        assert_eq!(reuc_mode(side(FileMode::BlobExecutable)), 0o100755);
        let any = ReucSide {
            mode: u32::MAX,
            id: id.as_ref(),
        };
        assert_eq!(reuc_mode(Some(any)), -1);
    }

    /// libgit2 returns `GIT_ENOTFOUND` without an error for a position past the end,
    /// leaving whatever error the thread had: the wrapper describes it instead.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn out_of_range_reuc_removal_is_described() {
        let mut index = crate::libgit2::index::index::Index::new().unwrap();
        set(ErrorClass::Odb, c"a stale error");
        // SAFETY: a live index with no records; C bounds-checks the position.
        let rc = unsafe { ffi::git_index_reuc_remove(index.as_c_ptr(), 0) };
        assert_eq!(rc, ffi::GIT_ENOTFOUND);
        assert_eq!(last().message(), c"a stale error");
        let error = index.as_mut().remove_reuc(0).unwrap_err();
        assert_eq!(error.class(), Some(ErrorClass::Index));
        assert_eq!(error.message(), c"no resolve-undo record at that position");
        let id = Oid::zeroed();
        let blob = ReucSide::new(FileMode::Blob, id.as_ref());
        index
            .as_mut()
            .add_reuc(c"p", Some(blob), None, None)
            .unwrap();
        let error = index.as_mut().remove_reuc(usize::MAX).unwrap_err();
        assert_eq!(error.class(), Some(ErrorClass::Index));
        assert_eq!(index.as_ref().reuc_entry_count(), 1);
        index.as_mut().remove_reuc(0).unwrap();
        assert_eq!(index.as_ref().reuc_entry_count(), 0);
    }
}
