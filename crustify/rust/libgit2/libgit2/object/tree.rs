//! Safe wrappers over `include/git2/tree.h`.
//!
//! `git_tree_walk` takes its traversal order (`git_treewalk_mode`) by value and refuses
//! any value the header does not name with a `GIT_ERROR_INVALID` error before reading
//! the tree, so a raw value is converted fallibly ([`WalkMode::from_raw`]) and never
//! transmuted.
//!
//! # Tree updates
//!
//! A [`TreeUpdate`] is a `git_tree_update`: one change, an [`UpdateAction`] at a path
//! with the id and [`FileMode`] of the entry an upsert puts there, of the array
//! `git_tree_create_updated` applies to a tree. libgit2 neither allocates, copies nor
//! frees one: it reads the caller's array in place through a `const` pointer, for that
//! call only, and keeps none of it. So Rust holds an update by value, `Copy`, with no
//! teardown, `#[repr(transparent)]` over the C struct, so that a `[TreeUpdate]` is the
//! array C reads. Its path is a `const char *` libgit2 dereferences unchecked and never
//! frees: a `&'p CStr` the update borrows for `'p`, never NULL. Its action and mode are
//! only ever set from [`UpdateAction`] and [`FileMode`] variants.
//!
//! # Tree builders
//!
//! A [`TreeBuilder`] is a `git_treebuilder`: an in-memory set of tree entries, keyed by
//! file name, from which libgit2 writes a tree object into a repository's object
//! database. The public header only forward-declares the struct, so Rust never reads its
//! fields: it is reached through libgit2's routines alone.
//!
//! libgit2 allocates one in `git_treebuilder_new` and frees it, with every entry it
//! holds, in `git_treebuilder_free`; there is no copy routine and no count. The caller
//! owns the result alone, so Rust holds it as a [`TreeBuilderBox`], which frees it on
//! drop. `git_treebuilder_clear` frees the entries and keeps the builder, which stays
//! usable.
//!
//! The builder keeps an unowned pointer to the repository it was built for, which
//! inserting (to check that an entry's object exists) and writing (into the
//! repository's object database, which libgit2 may load then) follow; freeing never
//! reaches it. So a builder carries the lifetime `'r` of that repository's borrow, a
//! [`RepositoryRef`], and lives no longer than it. A repository is not `Sync`, and the
//! builder reaches it from whichever thread uses the builder, so a builder is neither
//! `Send` nor `Sync` either.
//!
//! The constructors can fail and set the calling thread's libgit2 error, so they pin
//! libgit2's runtime first (`runtime_pinned`) and return the error, copied out, as an
//! [`ErrorBuf`].
//!
//! # Tree entries
//!
//! A [`TreeEntry`] is a `git_tree_entry`: one name in a tree, with the id and file mode
//! of the object it names. It is opaque: libgit2 publishes no field. An entry comes in
//! two forms.
//!
//! - **Owned.** `git_tree_entry_dup` and `git_tree_entry_bypath` hand the caller an
//!   entry of its own: one block of libgit2's allocator holding the struct and, right
//!   past it, its NUL-terminated name, which `git_tree_entry_free` releases at once.
//!   It holds nothing else, no tree and no repository, so it may outlive both. Rust
//!   owns one through a [`TreeEntryBox`], whose `Clone` is `git_tree_entry_dup` and
//!   which [`TreeEntry::bypath`] builds from a tree object and a path.
//! - **Borrowed.** An entry of a tree (`git_tree_entry_byindex`, `_byname`, `_byid`)
//!   is an element of the tree's entries, its name pointing into the tree's raw data;
//!   an entry of a tree builder (`git_treebuilder_get`) is the builder's. Either lives
//!   as long as its parent does, and is never freed on its own: it is a
//!   [`TreeEntryRef`] borrowing the parent, which
//!   [`to_tree_entry`](TreeEntryRef::to_tree_entry) copies into an owned one.
//!
//! No routine writes an entry through a `const git_tree_entry *`, so a shared handle
//! reads an entry nobody changes; a builder rewrites the mode and id of one of its
//! entries in place only through an exclusive borrow of the builder. An entry is
//! `Send` and `Sync`.

use core::ffi::{CStr, c_int};
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};

use ffibox::{CBorrowedPtr, CBox, CCell, CDrop, CDupClone, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::core::types::FileMode;
use crate::libgit2::object::object::ObjectRef;
use crate::libgit2::oid::oid::{OidMut, OidRef};
use crate::libgit2::repository::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use crate::libgit2::util::sys::errors::{runtime_pinned, set};

/// The order in which `git_tree_walk` calls back for a tree's entries and those of the
/// subtrees it loads on the way: either way, once per entry it reaches, each tree's entries in
/// that tree's order, under the path of the tree that holds them.
///
/// Wraps: git_treewalk_mode
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum WalkMode {
    /// Visit each entry before the subtree it names: a subtree's entries follow its
    /// own, and a positive callback result skips the subtree.
    PreOrder = ffi::GIT_TREEWALK_PRE,
    /// Visit each entry after the subtree it names: a subtree's entries precede its
    /// own, so the subtree is already walked, and a positive callback result is
    /// ignored.
    PostOrder = ffi::GIT_TREEWALK_POST,
}

impl WalkMode {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name, which `git_tree_walk` refuses.
    pub const fn from_raw(raw: ffi::git_treewalk_mode) -> Option<Self> {
        match raw {
            ffi::GIT_TREEWALK_PRE => Some(Self::PreOrder),
            ffi::GIT_TREEWALK_POST => Some(Self::PostOrder),
            _ => None,
        }
    }

    /// The C value, as `git_tree_walk` takes it.
    pub const fn raw(self) -> ffi::git_treewalk_mode {
        self as ffi::git_treewalk_mode
    }
}

impl TryFrom<ffi::git_treewalk_mode> for WalkMode {
    type Error = UnnamedWalkMode;

    fn try_from(raw: ffi::git_treewalk_mode) -> Result<Self, UnnamedWalkMode> {
        Self::from_raw(raw).ok_or(UnnamedWalkMode(raw))
    }
}

impl From<WalkMode> for ffi::git_treewalk_mode {
    fn from(value: WalkMode) -> ffi::git_treewalk_mode {
        value.raw()
    }
}

/// A tree walk mode that this version of libgit2's header does not name, which
/// `git_tree_walk` refuses as an invalid walking mode; [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedWalkMode(ffi::git_treewalk_mode);

impl UnnamedWalkMode {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_treewalk_mode {
        self.0
    }
}

impl fmt::Display for UnnamedWalkMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 tree walk mode {} is not named by git2/tree.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedWalkMode {}

/// What one `git_tree_update` ([`TreeUpdate`]) asks `git_tree_create_updated` to do at
/// its path: insert or replace the entry there, or remove it.
///
/// `git_tree_create_updated` refuses any other value with `GIT_ERROR_TREE` ("unknown
/// action for update"), after building the trees above the path.
/// [`from_raw`](Self::from_raw) gives `None` for a value without a variant.
///
/// Wraps: git_tree_update_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum UpdateAction {
    /// Insert an entry at the path, or replace the one there with the update's id and
    /// file mode; replacing a blob with a tree, or a tree with a blob, fails.
    Upsert = ffi::GIT_TREE_UPDATE_UPSERT,
    /// Remove the entry at the path; only the path is read.
    Remove = ffi::GIT_TREE_UPDATE_REMOVE,
}

impl UpdateAction {
    /// The variant whose C value is `raw`; `None` for any value libgit2's header does not
    /// name.
    pub const fn from_raw(raw: ffi::git_tree_update_t) -> Option<Self> {
        match raw {
            ffi::GIT_TREE_UPDATE_UPSERT => Some(Self::Upsert),
            ffi::GIT_TREE_UPDATE_REMOVE => Some(Self::Remove),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take it.
    pub const fn raw(self) -> ffi::git_tree_update_t {
        self as ffi::git_tree_update_t
    }
}

impl TryFrom<ffi::git_tree_update_t> for UpdateAction {
    type Error = UnnamedUpdateAction;

    fn try_from(raw: ffi::git_tree_update_t) -> Result<Self, UnnamedUpdateAction> {
        Self::from_raw(raw).ok_or(UnnamedUpdateAction(raw))
    }
}

impl From<UpdateAction> for ffi::git_tree_update_t {
    fn from(value: UpdateAction) -> ffi::git_tree_update_t {
        value.raw()
    }
}

/// An update action that libgit2's header does not name, which `git_tree_create_updated`
/// refuses; [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedUpdateAction(ffi::git_tree_update_t);

impl UnnamedUpdateAction {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_tree_update_t {
        self.0
    }
}

impl fmt::Display for UnnamedUpdateAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 tree update action {} is not named by git2/tree.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedUpdateAction {}

// An `UpdateAction` passes where C takes a `git_tree_update_t`.
const _: () = assert!(
    size_of::<UpdateAction>() == size_of::<ffi::git_tree_update_t>()
        && align_of::<UpdateAction>() == align_of::<ffi::git_tree_update_t>()
);

/// One change for `git_tree_create_updated` to make to a tree (`git_tree_update`): an
/// [`UpdateAction`] at a path, with the id and file mode of the entry an upsert puts
/// there.
///
/// Built by [`upsert`](Self::upsert) or [`remove`](Self::remove); read it through
/// [`as_ref`](Self::as_ref) and change it through [`as_mut`](Self::as_mut):
///
/// ```
/// use ffibox::CZeroable;
/// use libgit2::libgit2::core::types::FileMode;
/// use libgit2::libgit2::object::tree::{TreeUpdate, UpdateAction};
/// use libgit2::libgit2::oid::oid::Oid;
///
/// let id = Oid::zeroed();
/// let mut update = TreeUpdate::upsert(c"dir/file.txt", id.as_ref(), FileMode::Blob);
/// assert_eq!(update.as_ref().path(), c"dir/file.txt");
/// update.as_mut().set_filemode(FileMode::BlobExecutable);
/// update.as_mut().set_action(UpdateAction::Remove);
/// assert_eq!(update.as_ref().action(), UpdateAction::Remove);
///
/// // A `const` array is the C array `git_tree_create_updated` reads.
/// const REMOVALS: [TreeUpdate<'static>; 2] = [
///     TreeUpdate::remove(c"README"),
///     TreeUpdate::remove(c"src/old.c"),
/// ];
/// assert_eq!(REMOVALS[1].as_ref().path(), c"src/old.c");
/// ```
///
/// `#[repr(transparent)]` over `git_tree_update`, so a pointer to it, or to the first of
/// a `[TreeUpdate]`, is a pointer to the C struct or array, which C reads in place. `'p`
/// is the lifetime of the path it points at, so an update cannot outlive it:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::object::tree::TreeUpdate;
/// use std::ffi::CString;
///
/// let update;
/// {
///     let path = CString::new("dir/file.txt").unwrap();
///     update = TreeUpdate::remove(&path);
/// }
/// let _ = update.as_ref().path();
/// ```
///
/// The exclusive handle is invariant in `'p`, so one cannot be shrunk to store a
/// shorter-lived path than its update was built for:
///
/// ```compile_fail
/// use libgit2::libgit2::object::tree::TreeUpdateMut;
///
/// fn shrink<'a, 'p>(m: TreeUpdateMut<'a, 'static>) -> TreeUpdateMut<'a, 'p> {
///     m
/// }
/// ```
///
/// The shared handle only reads:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::object::tree::TreeUpdate;
///
/// let update = TreeUpdate::remove(c"README");
/// update.as_ref().set_path(c"other");
/// ```
///
/// libgit2 never allocates, copies or frees one: it reads a caller's array through a
/// `const` pointer for one call, keeping nothing. So an update is a plain value, with no
/// teardown. Every field holds a value its setter accepts: a named action and file
/// mode, and a path that is never NULL. `Copy`, `Send` and `Sync`, as a `&'p CStr` and
/// plain values are: libgit2 never writes through the path, and only the exclusive
/// handle writes the fields. Equal when every field is, the path by its contents.
///
/// Wraps: git_tree_update
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct TreeUpdate<'p>(ffi::git_tree_update, PhantomData<&'p CStr>);

/// Shared borrow of a [`TreeUpdate`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct TreeUpdateRef<'a, 'p>(CBorrowedPtr<'a, TreeUpdate<'p>>);

/// Exclusive borrow of a [`TreeUpdate`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct TreeUpdateMut<'a, 'p>(
    TreeUpdateRef<'a, 'p>,
    // Invariant in the update, and so in `'p`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut TreeUpdate<'p>>,
);

// SAFETY: `TreeUpdate` is `#[repr(transparent)]` over `git_tree_update` with a zero-sized
// marker; both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, TreeUpdate<'p>>`
// (`TreeUpdateMut` through `TreeUpdateRef`) with zero-sized markers and no `Drop`, hand
// out no reference to the update, and `TreeUpdateRef` has no write operation.
// `TreeUpdateMut` is invariant in the update through its
// `PhantomData<&'a mut TreeUpdate<'p>>`.
unsafe impl<'p> CCell for TreeUpdate<'p> {
    type C = ffi::git_tree_update;
    type Ref<'a>
        = TreeUpdateRef<'a, 'p>
    where
        Self: 'a;
    type Mut<'a>
        = TreeUpdateMut<'a, 'p>
    where
        Self: 'a;
}

// SAFETY: an update is two enum values, 33 bytes of id and a string pointer, which only
// the setters write, a `&'p CStr`'s pointer: a shared, immutable borrow of `CStr`, which
// is `Sync`, so it may be read from any thread. Nothing in the struct has thread affinity
// or interior mutability, and nothing is released on drop.
unsafe impl Send for TreeUpdate<'_> {}
// SAFETY: as above; `TreeUpdateMut` is an exclusive borrow of such a value, as
// `&mut TreeUpdate`.
unsafe impl Send for TreeUpdateMut<'_, '_> {}

// SAFETY: as above, and shared access only reads the fields (every write goes through
// `TreeUpdateMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for TreeUpdate<'_> {}
// SAFETY: as above; `TreeUpdateRef` is a shared borrow, as `&TreeUpdate`.
unsafe impl Send for TreeUpdateRef<'_, '_> {}
// SAFETY: as above.
unsafe impl Sync for TreeUpdateRef<'_, '_> {}
// SAFETY: `&TreeUpdateMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for TreeUpdateMut<'_, '_> {}

impl<'p> TreeUpdate<'p> {
    /// Insert, or replace, the entry at `path` with one naming `id` with mode
    /// `filemode`.
    ///
    /// `path` is the entry's full path from the root tree, its components separated by
    /// `/`; the trees above it are created as needed. The update is only built here:
    /// `git_tree_create_updated` checks it when it runs, refusing for instance an
    /// [`Unreadable`](FileMode::Unreadable) mode, an `id` of a blob or tree the
    /// repository lacks, or the replacement of a tree with a blob or of a blob with a
    /// tree.
    pub fn upsert(path: &'p CStr, id: OidRef<'_>, filemode: FileMode) -> Self {
        Self::new(UpdateAction::Upsert, path, copy_id(id), filemode)
    }

    /// Remove the entry at `path`, the entry's full path from the root tree, its
    /// components separated by `/`. A tree left empty by the removal is removed too.
    ///
    /// Only the path is read for a removal: the id is libgit2's typeless, all-zero
    /// `GIT_OID_NONE` and the mode [`Unreadable`](FileMode::Unreadable), until a setter
    /// changes them.
    pub const fn remove(path: &'p CStr) -> Self {
        let none = ffi::git_oid {
            type_: 0,
            id: [0; ffi::GIT_OID_MAX_SIZE as usize],
        };
        Self::new(UpdateAction::Remove, path, none, FileMode::Unreadable)
    }

    const fn new(
        action: UpdateAction,
        path: &'p CStr,
        id: ffi::git_oid,
        filemode: FileMode,
    ) -> Self {
        Self(
            ffi::git_tree_update {
                action: action.raw(),
                id,
                filemode: filemode.raw(),
                path: path.as_ptr(),
            },
            PhantomData,
        )
    }

    /// Shared handle to the update — the getters.
    pub fn as_ref(&self) -> TreeUpdateRef<'_, 'p> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        TreeUpdateRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the update — the setters.
    pub fn as_mut(&mut self) -> TreeUpdateMut<'_, 'p> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        TreeUpdateMut(TreeUpdateRef(ptr), PhantomData)
    }
}

/// The `git_oid` `id` holds, copied field by field.
fn copy_id(id: OidRef<'_>) -> ffi::git_oid {
    ffi::git_oid {
        type_: id.raw_kind(),
        id: id.id(),
    }
}

impl PartialEq for TreeUpdate<'_> {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.as_ref(), other.as_ref());
        let (x, y) = (a.id(), b.id());
        a.action() == b.action()
            && (x.raw_kind(), x.id()) == (y.raw_kind(), y.id())
            && a.filemode() == b.filemode()
            && a.path() == b.path()
    }
}

impl Eq for TreeUpdate<'_> {}

impl fmt::Debug for TreeUpdate<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'p> TreeUpdateRef<'a, 'p> {
    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_tree_update *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_tree_update {
        self.0.as_non_null().as_ptr().cast()
    }

    /// What the update does at its path.
    ///
    /// Field: git_tree_update.action
    pub fn action(&self) -> UpdateAction {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to a live update; the field
        // is a plain integer.
        let raw = unsafe { addr_of!((*p).action).read() };
        // `new` and `set_action` only store a variant's value.
        UpdateAction::from_raw(raw).expect("an update holds a named action")
    }

    /// The id of the object an upsert's entry names, in place, for as long as the
    /// update is borrowed.
    ///
    /// Field: git_tree_update.id
    pub fn id(&self) -> OidRef<'a> {
        let p = self.as_ptr();
        // SAFETY: the field projected from the shared handle's pointer to a live update
        // is an initialised `git_oid`, which lives for `'a` with the update and which
        // nothing writes meanwhile (every write needs the exclusive handle); `OidRef`
        // only reads it.
        unsafe { OidRef::from_ptr(addr_of!((*p).id).cast_mut()) }.expect("a field is never NULL")
    }

    /// The mode of an upsert's entry.
    ///
    /// Field: git_tree_update.filemode
    pub fn filemode(&self) -> FileMode {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to a live update; the field
        // is a plain integer.
        let raw = unsafe { addr_of!((*p).filemode).read() };
        // `new` and `set_filemode` only store a variant's value.
        FileMode::from_raw(raw).expect("an update holds a named file mode")
    }

    /// The entry's full path from the root tree, borrowed for `'p`, as it was set.
    ///
    /// Field: git_tree_update.path
    pub fn path(&self) -> &'p CStr {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to a live update. `new` and
        // `set_path` store a `&'p CStr`'s pointer, a NUL-terminated string that outlives
        // `'p` and that nothing writes, being shared-borrowed.
        unsafe { CStr::from_ptr(addr_of!((*p).path).read()) }
    }

    /// A copy of the update, as a value of its own, borrowing the same path.
    pub fn to_tree_update(&self) -> TreeUpdate<'p> {
        TreeUpdate::new(
            self.action(),
            self.path(),
            copy_id(self.id()),
            self.filemode(),
        )
    }
}

impl fmt::Debug for TreeUpdateRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeUpdate")
            .field("action", &self.action())
            .field("id", &self.id())
            .field("filemode", &self.filemode())
            .field("path", &self.path())
            .finish()
    }
}

impl<'p> TreeUpdateMut<'_, 'p> {
    /// Writable pointer to the C struct, for the setters.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_tree_update {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> TreeUpdateRef<'_, 'p> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> TreeUpdateMut<'_, 'p> {
        TreeUpdateMut(self.0, PhantomData)
    }

    /// Replaces what the update does; the id and file mode stay, and are read again
    /// when it becomes an upsert.
    ///
    /// Field: git_tree_update.action
    pub fn set_action(&mut self, action: UpdateAction) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to a live update of a
        // variant's value, which keeps `action`'s guarantee.
        unsafe { addr_of_mut!((*p).action).write(action.raw()) }
    }

    /// The id of the object an upsert's entry names, in place, borrowed exclusively to
    /// change it.
    ///
    /// Field: git_tree_update.id
    pub fn id_mut(&mut self) -> OidMut<'_> {
        let p = self.as_mut_ptr();
        // SAFETY: the field projected from the exclusive handle's pointer to a live update
        // is an initialised `git_oid`, any of whose bytes may be written; the result
        // borrows this handle exclusively, so nothing else reaches the update meanwhile.
        unsafe { OidMut::from_ptr(addr_of_mut!((*p).id)) }.expect("a field is never NULL")
    }

    /// Replaces the id with a copy of `id`, type and all 32 bytes.
    ///
    /// Field: git_tree_update.id
    pub fn set_id(&mut self, id: OidRef<'_>) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to a live update; the
        // field is plain bytes, and `id` is copied out before the write.
        unsafe { addr_of_mut!((*p).id).write(copy_id(id)) }
    }

    /// Replaces the mode of an upsert's entry.
    ///
    /// Field: git_tree_update.filemode
    pub fn set_filemode(&mut self, filemode: FileMode) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to a live update of a
        // variant's value, which keeps `filemode`'s guarantee.
        unsafe { addr_of_mut!((*p).filemode).write(filemode.raw()) }
    }

    /// Replaces the path; the old string, borrowed, needs no release.
    ///
    /// Field: git_tree_update.path
    pub fn set_path(&mut self, path: &'p CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to a live update of a
        // `&'p CStr`'s pointer, which keeps `path`'s guarantee; the old pointer is
        // borrowed, so nothing is released.
        unsafe { addr_of_mut!((*p).path).write(path.as_ptr()) }
    }
}

impl fmt::Debug for TreeUpdateMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// A `TreeUpdate` is a `git_tree_update`, so a `[TreeUpdate]` is the array C reads in
// place.
const _: () = assert!(
    size_of::<TreeUpdate<'static>>() == size_of::<ffi::git_tree_update>()
        && align_of::<TreeUpdate<'static>>() == align_of::<ffi::git_tree_update>()
        && size_of::<[TreeUpdate<'static>; 3]>() == size_of::<[ffi::git_tree_update; 3]>()
        && size_of::<TreeUpdateRef<'static, 'static>>() == size_of::<*const ()>()
);

/// An in-memory tree being built (`git_treebuilder`): entries keyed by file name, which
/// libgit2 writes out as a tree object.
///
/// Only ever behind a pointer, owned as a [`TreeBuilderBox`], which [`new`](Self::new)
/// (empty) and [`from_tree`](Self::from_tree) (seeded with a tree's entries) build.
/// libgit2 publishes no fields: the handles carry no field accessors.
///
/// `'r` is the borrow of the repository the builder was built for, which it reaches
/// while inserting and writing (see the [module docs](self#tree-builders)); the owner
/// does not outlive it:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::object::tree::TreeBuilder;
/// use libgit2::libgit2::repository::repository::Repository;
///
/// let repo = Repository::new().unwrap();
/// let builder = TreeBuilder::new(repo.as_ref()).unwrap();
/// drop(repo);
/// drop(builder);
/// ```
///
/// nor does a handle outlive its owner:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::object::tree::TreeBuilderBox;
///
/// fn escape(builder: TreeBuilderBox<'_>) {
///     let handle = builder.as_ref();
///     drop(builder);
///     let _ = handle;
/// }
/// ```
///
/// The repository lends no exclusive handle while a builder borrows it:
///
/// ```compile_fail,E0502
/// use libgit2::libgit2::object::tree::TreeBuilder;
/// use libgit2::libgit2::repository::repository::Repository;
///
/// let mut repo = Repository::new().unwrap();
/// let builder = TreeBuilder::new(repo.as_ref()).unwrap();
/// let _ = repo.as_mut();
/// drop(builder);
/// ```
///
/// The builder reaches its repository, which is not `Sync`, so neither the owner nor a
/// handle crosses threads:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::tree::TreeBuilderBox;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(builder: TreeBuilderBox<'static>) {
///     send(builder);
/// }
/// ```
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::tree::TreeBuilderRef;
///
/// fn sync<T: Sync>() {}
/// sync::<TreeBuilderRef<'static, 'static>>();
/// ```
///
/// There is no copy routine, so the owner is not `Clone`:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::tree::TreeBuilderBox;
///
/// fn copyable<T: Clone>() {}
/// copyable::<TreeBuilderBox<'static>>();
/// ```
///
/// Wraps: git_treebuilder
/// Handwritten
#[repr(transparent)]
pub struct TreeBuilder<'r>(ffi::git_treebuilder, PhantomData<(*const (), &'r ())>);

/// Shared borrow of a [`TreeBuilder`]. `Copy`, like `&T`.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct TreeBuilderRef<'a, 'r>(CBorrowedPtr<'a, TreeBuilder<'r>>);

/// Exclusive borrow of a [`TreeBuilder`]: the routines that change its entries take it.
/// Reaches the shared handle with [`as_ref`](Self::as_ref).
///
/// Handwritten
#[repr(transparent)]
pub struct TreeBuilderMut<'a, 'r>(
    TreeBuilderRef<'a, 'r>,
    // Invariant in the builder, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut TreeBuilder<'r>>,
);

// SAFETY: `TreeBuilder` is `#[repr(transparent)]` over `git_treebuilder` with a zero-sized
// marker; both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, TreeBuilder<'r>>`
// (`TreeBuilderMut` through `TreeBuilderRef`) with zero-sized markers and no `Drop`, hand
// out no reference to the C object, and `TreeBuilderRef` has no write operation.
// `TreeBuilderMut` is invariant in the builder through its
// `PhantomData<&'a mut TreeBuilder<'r>>`.
unsafe impl<'r> CCell for TreeBuilder<'r> {
    type C = ffi::git_treebuilder;
    type Ref<'a>
        = TreeBuilderRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = TreeBuilderMut<'a, 'r>
    where
        Self: 'a;
}

impl<'a, 'r> TreeBuilderRef<'a, 'r> {
    /// Borrows a raw pointer; `None` if null.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live tree builder, which no one frees or writes
    /// during `'a`, built for a repository that outlives `'r`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_treebuilder) -> Option<Self> {
        NonNull::new(ptr.cast_mut()).map(|p| {
            // SAFETY: a non-null pointer the caller vouches for, per this function's
            // contract; `TreeBuilder` is transparent over `git_treebuilder`.
            TreeBuilderRef(unsafe { CBorrowedPtr::new(p.cast()) })
        })
    }

    /// Read-only pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_treebuilder {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'a, 'r> TreeBuilderMut<'a, 'r> {
    /// Borrows a raw pointer exclusively; `None` if null.
    ///
    /// # Safety
    ///
    /// As [`TreeBuilderRef::from_ptr`], plus: nothing else reaches the builder while the
    /// result lives.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_treebuilder) -> Option<Self> {
        // SAFETY: the caller upholds `TreeBuilderRef::from_ptr`'s contract and
        // exclusivity.
        unsafe { TreeBuilderRef::from_ptr(ptr) }.map(|r| TreeBuilderMut(r, PhantomData))
    }

    /// Writable pointer to the C object.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_treebuilder {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> TreeBuilderRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> TreeBuilderMut<'_, 'r> {
        TreeBuilderMut(self.0, PhantomData)
    }

    /// Removes every entry, freeing each; the builder stays usable, and still writes
    /// into the repository it was built for. Writing it now writes the empty tree.
    ///
    /// Wraps: git_treebuilder_clear
    pub fn clear(&mut self) {
        // SAFETY: the exclusive handle's pointer addresses a live builder that nothing
        // else reaches meanwhile; `git_treebuilder_clear` frees the entries it owns and
        // empties its map, without reaching the repository. It fails only on a NULL
        // builder, so its result is always 0 here.
        let rc = unsafe { ffi::git_treebuilder_clear(self.as_mut_ptr()) };
        debug_assert_eq!(rc, 0);
    }
}

impl fmt::Debug for TreeBuilderRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeBuilder").finish_non_exhaustive()
    }
}

impl fmt::Debug for TreeBuilderMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases a tree builder through `git_treebuilder_free`, which frees its write buffer,
/// every entry it holds and its map, then the struct, without reaching the repository.
/// libgit2 has no copy routine, so the policy does not clone.
///
/// Wraps: git_treebuilder_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct TreeBuilderFree;

// SAFETY: `git_treebuilder_free` releases a live builder exactly once, with everything it
// owns, whatever entries it holds. It needs no state beyond the pointer, and no
// repository.
unsafe impl<'r> CDrop<TreeBuilder<'r>> for TreeBuilderFree {
    unsafe fn c_drop(&self, ptr: NonNull<TreeBuilder<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live builder whose ownership
        // the caller gives up; `TreeBuilder` is transparent over `git_treebuilder`.
        unsafe { ffi::git_treebuilder_free(ptr.as_ptr().cast()) }
    }
}

/// An owned [`TreeBuilder`], freed by `git_treebuilder_free` on drop.
pub type TreeBuilderBox<'r> = CBox<TreeBuilder<'r>, TreeBuilderFree>;

const _: () =
    assert!(size_of::<TreeBuilderBox<'static>>() == size_of::<*mut ffi::git_treebuilder>());

impl<'r> TreeBuilder<'r> {
    /// An empty builder for `repo`: writing it straight away writes the empty tree.
    ///
    /// Fails, with libgit2's error, only when it runs out of memory.
    ///
    /// Wraps: git_treebuilder_new
    pub fn new(repo: RepositoryRef<'r>) -> Result<TreeBuilderBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `repo` a live repository on this thread, which
        // the builder only records; no source tree. The runtime is pinned, so a failure
        // is reported on live thread-local state.
        let rc =
            unsafe { ffi::git_treebuilder_new(&mut out, repo.as_ptr().cast_mut(), ptr::null()) };
        // SAFETY: on success `git_treebuilder_new` writes a fresh builder for `repo` that
        // the caller owns alone, released by `git_treebuilder_free` as `TreeBuilderFree`
        // does; it keeps `repo`, which outlives `'r`. On failure it leaves `out` NULL.
        constructed(rc, unsafe { TreeBuilderBox::from_c(out) })
    }

    /// A builder for `repo` holding a copy of each entry of `source`, an object of type
    /// tree: their names, ids and file modes as the tree spells them, unchecked. The
    /// builder keeps nothing of `source`, which may be dropped as soon as this returns.
    ///
    /// `source` may come from any repository. Its ids are copied as they are, so a tree
    /// of a repository of another object id type gives a builder that writes a malformed
    /// tree, as libgit2 does.
    ///
    /// Refuses, with a `GIT_ERROR_TREE` error "the object is not a tree", an object of
    /// another type, which libgit2 would read as a tree. Fails, with libgit2's error,
    /// when it runs out of memory, having freed the entries it copied.
    ///
    /// Wraps: git_treebuilder_new
    pub fn from_tree(
        repo: RepositoryRef<'r>,
        source: ObjectRef<'_, '_>,
    ) -> Result<TreeBuilderBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let Some(source) = as_tree(source) else {
            return Err(last());
        };
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `repo` a live repository on this thread, which
        // the builder only records (no entry is checked while seeding); `source` a live
        // tree for the call, which the handle it came from keeps alive, only read. The
        // runtime is pinned.
        let rc = unsafe {
            ffi::git_treebuilder_new(&mut out, repo.as_ptr().cast_mut(), source.as_ptr())
        };
        // SAFETY: as in `new`: a fresh builder for `repo` the caller owns alone, or NULL.
        constructed(rc, unsafe { TreeBuilderBox::from_c(out) })
    }
}

/// The `git_tree` an object of type tree is, live for as long as the handle; for an
/// object of any other type, `None`, with a `GIT_ERROR_TREE` error "the object is not a
/// tree" set as the thread's.
fn as_tree(object: ObjectRef<'_, '_>) -> Option<NonNull<ffi::git_tree>> {
    // SAFETY: the handle addresses a live object; `git_object_type` reads its cached
    // type.
    if unsafe { ffi::git_object_type(object.as_ptr()) } != ffi::GIT_OBJECT_TREE {
        set(ErrorClass::Tree, c"the object is not a tree");
        return None;
    }
    // A tree object is a `git_tree`, whose first member is the `git_object`.
    NonNull::new(object.as_ptr().cast_mut().cast())
}

/// The builder a `git_treebuilder_new` returning `rc` left in its out slot (adopted as
/// `builder`), or libgit2's error. On failure libgit2 leaves the slot untouched, so
/// NULL: it frees a half-seeded builder itself.
fn constructed(
    rc: c_int,
    builder: Option<TreeBuilderBox<'_>>,
) -> Result<TreeBuilderBox<'_>, ErrorBuf> {
    if rc < 0 {
        return Err(last());
    }
    builder.ok_or_else(last)
}

define_ctype!(
    /// One entry of a tree (`git_tree_entry`): a name, and the id and file mode of the
    /// object it names.
    ///
    /// libgit2 allocates and frees it, so it is only ever reached through a
    /// [`TreeEntryBox`], for an entry of its own ([`bypath`](Self::bypath), or a copy
    /// of a borrowed one), or a [`TreeEntryRef`] that borrows a tree or tree builder
    /// (see the [module docs](self#tree-entries)). It is opaque: no field is published.
    ///
    /// A borrowed entry lives no longer than its owner:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::object::tree::{TreeEntryBox, TreeEntryRef};
    ///
    /// fn escape(entry: TreeEntryBox) -> TreeEntryRef<'static> {
    ///     let handle = entry.as_ref();
    ///     drop(entry);
    ///     handle
    /// }
    /// ```
    ///
    /// Wraps: git_tree_entry
    TreeEntry,
    TreeEntryRef,
    TreeEntryMut,
    ffi::git_tree_entry
);

// SAFETY: an owned entry is one block of libgit2's process-wide allocator, which frees
// on any thread, holding the struct and its name and nothing shared with anything else;
// a borrowed one moves only as a handle, as `&T` does under `Sync` below. No C routine
// keeps thread-local state about an entry.
unsafe impl Send for TreeEntry {}
// SAFETY: libgit2 never writes an entry through a `const git_tree_entry *`, the only
// access a shared handle grants (`git_tree_entry_dup` and every accessor only read it),
// and builds its fields before handing it out; the one writer, a tree builder updating
// its own entry, needs an exclusive borrow of the builder, which a shared handle to the
// entry excludes.
unsafe impl Sync for TreeEntry {}

impl fmt::Debug for TreeEntryRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeEntry").finish_non_exhaustive()
    }
}

impl fmt::Debug for TreeEntryMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases an owned entry through `git_tree_entry_free`, which frees its one block,
/// name included; and copies an entry, owned or borrowed, through
/// `git_tree_entry_dup`, into a fresh block of its own.
///
/// The free only applies to an entry from `git_tree_entry_dup` or
/// `git_tree_entry_bypath`: an entry borrowed from a tree or builder is part of its
/// parent's storage, and is reached through a [`TreeEntryRef`] alone.
///
/// Wraps: git_tree_entry_free
/// Wraps: git_tree_entry_dup
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct TreeEntryFree;

impl_cdrop!(TreeEntryFree, TreeEntry, ffi::git_tree_entry_free);

// SAFETY: `dup` returns `None` or a fresh block from `git_tree_entry_dup` holding a
// copy of the entry, its name and id and mode, which `git_tree_entry_free` releases; it
// only reads `ptr`, leaving it valid.
unsafe impl CDupClone<TreeEntry> for TreeEntryFree {
    unsafe fn c_dup(&self, ptr: NonNull<TreeEntry>) -> Option<NonNull<TreeEntry>> {
        // SAFETY: per `c_dup`'s contract `ptr` addresses a live entry, which nothing
        // writes during the copy, the handle's only use; `TreeEntry` is transparent over
        // `git_tree_entry`.
        let source = unsafe { TreeEntryRef::from_ptr(ptr.as_ptr().cast()) }?;
        let copy = dup(source)?;
        NonNull::new(copy.into_c().cast())
    }
}

/// An owned [`TreeEntry`], freed by `git_tree_entry_free` on drop and copied by
/// `git_tree_entry_dup` on `clone` (`try_clone` reports a failed copy as `None`).
pub type TreeEntryBox = CBox<TreeEntry, TreeEntryFree>;

/// `git_tree_entry_dup(source)`: a fresh copy the caller owns, or `None` when libgit2
/// cannot allocate it (or be initialised), with the thread's error set.
fn dup(source: TreeEntryRef<'_>) -> Option<TreeEntryBox> {
    if !runtime_pinned() {
        return None;
    }
    let mut out = ptr::null_mut();
    // SAFETY: `out` is a writable slot, set only on success; `source` addresses a live
    // entry nothing writes while the shared handle lives, whose NUL-terminated name, id
    // and mode are read for the call only. The runtime is pinned, so a failure is
    // reported on live thread-local state.
    let rc = unsafe { ffi::git_tree_entry_dup(&mut out, source.as_ptr()) };
    if rc < 0 {
        return None;
    }
    // SAFETY: a fresh entry from `git_tree_entry_dup` the caller owns, released by
    // `git_tree_entry_free` as `TreeEntryFree` does.
    unsafe { TreeEntryBox::from_c(out) }
}

impl TreeEntry {
    /// The entry at `path` under `tree`, an object of type tree, as a copy of its own:
    /// each `/`-separated component of `path` is looked up in turn, the subtrees it
    /// names read from `tree`'s repository, and the last component's entry is copied
    /// out, as `git_tree_entry_dup` copies one. The result holds nothing of `tree` or
    /// its repository, and outlives both.
    ///
    /// The components are matched as the tree spells them: `.` and `..` name nothing,
    /// a trailing `/` is accepted only after a tree, and an empty component (an empty
    /// `path`, or a leading or doubled `/`) is an invalid path.
    ///
    /// Refuses, with [`ErrorCode::InvalidSpec`] and a `GIT_ERROR_TREE` error "the
    /// object is not a tree", an object of another type, which libgit2 would read as a
    /// tree (a commit or tag is peeled to its tree by
    /// [`ObjectRef::lookup_bypath`]). Fails with [`ErrorCode::NotFound`] (class
    /// `GIT_ERROR_TREE`) for an invalid path, a component the tree lacks, or a `/`
    /// after an entry that is not a tree; and with libgit2's error when a subtree
    /// cannot be read, or the copy cannot be allocated.
    ///
    /// Wraps: git_tree_entry_bypath
    pub fn bypath(tree: ObjectRef<'_, '_>, path: &CStr) -> Result<TreeEntryBox, RepositoryError> {
        let Some(tree) = as_tree(tree) else {
            return Err(RepositoryError::from_libgit2(ErrorCode::InvalidSpec.raw()));
        };
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `tree` is a live tree
        // for the call, which the handle it came from keeps alive, whose repository is
        // live for that handle's `'r` and used on this thread only, as `RepositoryRef`
        // allows, where libgit2 looks the subtrees up; `path` is NUL-terminated and
        // read for the call only.
        let rc = unsafe { ffi::git_tree_entry_bypath(&mut out, tree.as_ptr(), path.as_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` is a fresh entry from `git_tree_entry_dup` the
        // caller owns, released by `git_tree_entry_free` as `TreeEntryFree` does.
        let entry = unsafe { TreeEntryBox::from_c(out) };
        Ok(entry.expect("a lookup succeeded without an entry"))
    }
}

impl TreeEntryRef<'_> {
    /// A copy of the entry in a block of its own, as `git_tree_entry_dup` makes one:
    /// the same name, id and file mode, owned by the result and tied to no tree. An
    /// entry borrowed from a tree or builder becomes owned this way; an owner also
    /// copies with `Clone`.
    ///
    /// Fails with libgit2's error when it runs out of memory, or cannot be initialised.
    ///
    /// Wraps: git_tree_entry_dup
    pub fn to_tree_entry(&self) -> Result<TreeEntryBox, ErrorBuf> {
        dup(*self).ok_or_else(last)
    }
}

// A `TreeEntry` is a `git_tree_entry`, never held by value; the owner is one pointer.
const _: () = assert!(
    size_of::<TreeEntry>() == size_of::<ffi::git_tree_entry>()
        && size_of::<TreeEntryBox>() == size_of::<*mut ffi::git_tree_entry>()
        && size_of::<Option<TreeEntryBox>>() == size_of::<*mut ffi::git_tree_entry>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;

    const MODES: [WalkMode; 2] = [WalkMode::PreOrder, WalkMode::PostOrder];

    #[test]
    fn raw_round_trips() {
        for mode in MODES {
            assert_eq!(WalkMode::from_raw(mode.raw()), Some(mode));
            assert_eq!(WalkMode::try_from(mode.raw()), Ok(mode));
            assert_eq!(ffi::git_treewalk_mode::from(mode), mode.raw());
        }
        assert_eq!(WalkMode::PreOrder.raw(), ffi::GIT_TREEWALK_PRE);
        assert_eq!(WalkMode::PostOrder.raw(), ffi::GIT_TREEWALK_POST);
    }

    #[test]
    fn unnamed_values_are_refused_without_panicking() {
        for raw in [2, 3, 0x100, ffi::git_treewalk_mode::MAX] {
            assert_eq!(WalkMode::from_raw(raw), None);
            let err = WalkMode::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(
                err.to_string(),
                format!("libgit2 tree walk mode {raw} is not named by git2/tree.h")
            );
        }
    }

    #[test]
    fn layout_is_the_c_enum() {
        assert_eq!(size_of::<WalkMode>(), size_of::<ffi::git_treewalk_mode>());
        assert_eq!(size_of::<Option<WalkMode>>(), size_of::<WalkMode>());
    }

    /// Every variant converts to its C value and back; every other value is refused,
    /// keeping the value.
    #[test]
    fn update_actions_round_trip() {
        for (action, raw) in [
            (UpdateAction::Upsert, ffi::GIT_TREE_UPDATE_UPSERT),
            (UpdateAction::Remove, ffi::GIT_TREE_UPDATE_REMOVE),
        ] {
            assert_eq!(action.raw(), raw);
            assert_eq!(ffi::git_tree_update_t::from(action), raw);
            assert_eq!(UpdateAction::from_raw(raw), Some(action));
            assert_eq!(UpdateAction::try_from(raw), Ok(action));
        }
        for raw in [2, 3, 0x7fff_ffff, u32::MAX] {
            assert_eq!(UpdateAction::from_raw(raw), None);
            let err = UpdateAction::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(
                err.to_string(),
                format!("libgit2 tree update action {raw} is not named by git2/tree.h")
            );
        }
    }

    /// The builder handles are one pointer wide, the owner too, and `None` is NULL.
    #[test]
    fn layouts_are_pointer_sized() {
        assert_eq!(
            size_of::<TreeBuilder<'_>>(),
            size_of::<ffi::git_treebuilder>()
        );
        assert_eq!(
            size_of::<TreeBuilderRef<'_, '_>>(),
            size_of::<*const ffi::git_treebuilder>()
        );
        assert_eq!(
            size_of::<TreeBuilderMut<'_, '_>>(),
            size_of::<*mut ffi::git_treebuilder>()
        );
        assert_eq!(
            size_of::<Option<TreeBuilderBox<'_>>>(),
            size_of::<*mut ffi::git_treebuilder>()
        );
    }

    /// The handles' raw seams round-trip a pointer, reborrow, and format opaquely;
    /// nothing is dereferenced, so Miri runs it.
    #[test]
    fn handles_round_trip_their_pointer() {
        let mut raw = core::mem::MaybeUninit::<[usize; 8]>::uninit();
        let p: *mut ffi::git_treebuilder = raw.as_mut_ptr().cast();
        // SAFETY: a pointer into this test's live local storage, which no one else
        // reaches; the handles only carry it and nothing here dereferences it (the
        // opaque `git_treebuilder` is zero-sized to Rust).
        let mut m = unsafe { TreeBuilderMut::from_ptr(p) }.unwrap();
        assert_eq!(m.as_mut_ptr(), p);
        assert_eq!(m.as_mut().as_mut_ptr(), p);
        assert_eq!(m.as_ref().as_ptr(), p.cast_const());
        assert_eq!(format!("{m:?}"), "TreeBuilder { .. }");
        assert_eq!(format!("{:?}", m.as_ref()), "TreeBuilder { .. }");
        // SAFETY: NULL is refused before anything else.
        assert!(unsafe { TreeBuilderRef::from_ptr(ptr::null()) }.is_none());
        // SAFETY: as above.
        assert!(unsafe { TreeBuilderMut::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: as above; nothing is adopted.
        assert!(unsafe { TreeBuilderBox::from_c(ptr::null_mut()) }.is_none());
    }

    /// A builder `git_treebuilder_new` hands out, adopted through `from_c`, is borrowed
    /// back through `from_ptr`, cleared through the handle, released through
    /// `TreeBuilderFree` exactly once on drop (ASan: leak, double free), and given back
    /// through `into_c` without a release.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adopted_builders_release_once() {
        use crate::libgit2::repository::repository::Repository;

        let repo = Repository::new().unwrap();
        let raw = |repo: RepositoryRef<'_>| {
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot, a live repository, no source tree.
            let rc = unsafe {
                ffi::git_treebuilder_new(&mut out, repo.as_ptr().cast_mut(), ptr::null())
            };
            assert_eq!(rc, 0);
            out
        };
        let count = |p: *mut ffi::git_treebuilder| {
            // SAFETY: a live builder the test owns; the call only reads it.
            unsafe { ffi::git_treebuilder_entrycount(p) }
        };

        let p = raw(repo.as_ref());
        // SAFETY: a fresh builder of `repo`, owned by no one else, which
        // `git_treebuilder_free` releases.
        let mut builder = unsafe { TreeBuilderBox::from_c(p) }.unwrap();
        assert_eq!(builder.as_c_ptr(), p);
        let id = ffi::git_oid {
            type_: ffi::GIT_OID_SHA1 as u8,
            id: [0x11; 32],
        };
        // SAFETY: a live builder, a NUL-terminated name and a live id; a commit entry is
        // not checked against the object database.
        let rc = unsafe {
            ffi::git_treebuilder_insert(
                ptr::null_mut(),
                p,
                c"sub".as_ptr(),
                &id,
                ffi::GIT_FILEMODE_COMMIT,
            )
        };
        assert_eq!(rc, 0);
        assert_eq!(count(p), 1);
        {
            // SAFETY: the box's live builder; the box is not used while `m` lives.
            let mut m = unsafe { TreeBuilderMut::from_ptr(builder.as_c_ptr()) }.unwrap();
            m.clear();
            m.clear();
        }
        assert_eq!(count(builder.as_c_ptr()), 0);
        builder.as_mut().clear();
        drop(builder);

        let p = raw(repo.as_ref());
        // SAFETY: as above.
        let builder = unsafe { TreeBuilderBox::from_c(p) }.unwrap();
        let back = builder.into_c();
        assert_eq!(back, p);
        // SAFETY: the builder `into_c` gave up, released once here.
        unsafe { TreeBuilderFree.c_drop(NonNull::new(back.cast::<TreeBuilder<'_>>()).unwrap()) };
    }

    /// `git_tree_update`'s value, handles and accessors. Nothing calls libgit2, so Miri
    /// runs them all.
    mod tree_update {
        use crate::libgit2::core::types::FileMode;
        use crate::libgit2::object::tree::*;
        use crate::libgit2::oid::oid::{Oid, OidType};
        use core::ffi::CStr;
        use ffibox::CZeroable;
        use std::ffi::CString;

        /// A SHA1 id of `byte` repeated, built without libgit2.
        fn sha1(byte: u8) -> Oid {
            let mut id = Oid::zero(OidType::Sha1);
            let mut bytes = [0; 32];
            bytes[..20].fill(byte);
            id.as_mut().set_id(bytes);
            id
        }

        /// The C struct an update is, read whole through its shared handle.
        fn raw(update: &TreeUpdate<'_>) -> ffi::git_tree_update {
            // SAFETY: the handle addresses `update`, a live, initialised value.
            unsafe { update.as_ref().as_ptr().read() }
        }

        fn same_id(a: &ffi::git_oid, b: &ffi::git_oid) -> bool {
            (a.type_, a.id) == (b.type_, b.id)
        }

        #[test]
        fn layout_is_the_c_struct() {
            assert_eq!(
                size_of::<TreeUpdate<'_>>(),
                size_of::<ffi::git_tree_update>()
            );
            assert_eq!(
                align_of::<TreeUpdate<'_>>(),
                align_of::<ffi::git_tree_update>()
            );
            assert_eq!(size_of::<TreeUpdateRef<'_, '_>>(), size_of::<*const ()>());
            assert_eq!(size_of::<TreeUpdateMut<'_, '_>>(), size_of::<*mut ()>());

            // A `[TreeUpdate]` is the C array: element `i` is read where C reads it.
            let paths = [c"a", c"b/c", c"d/e/f"];
            let updates = paths.map(TreeUpdate::remove);
            let base = updates.as_ptr().cast::<ffi::git_tree_update>();
            for (i, path) in paths.iter().enumerate() {
                // SAFETY: `i` is within the live array, which is `[git_tree_update; 3]`.
                let c = unsafe { base.add(i).read() };
                assert_eq!(c.path, path.as_ptr());
                assert_eq!(c.action, ffi::GIT_TREE_UPDATE_REMOVE);
                assert_eq!(updates[i].as_ref().as_ptr(), base.wrapping_add(i));
            }
        }

        /// Each constructor fills every field of the C struct; the getters read them
        /// back.
        #[test]
        fn constructors_fill_every_field() {
            let id = sha1(0xab);
            let path = CString::new("dir/sub/file.txt").unwrap();
            let up = TreeUpdate::upsert(&path, id.as_ref(), FileMode::BlobExecutable);
            let c = raw(&up);
            assert_eq!(c.action, ffi::GIT_TREE_UPDATE_UPSERT);
            assert!(same_id(&c.id, &raw_oid(&id)));
            assert_eq!(c.filemode, ffi::GIT_FILEMODE_BLOB_EXECUTABLE);
            assert_eq!(c.path, path.as_ptr());
            let r = up.as_ref();
            assert_eq!(r.action(), UpdateAction::Upsert);
            assert_eq!(r.id().raw_kind(), OidType::Sha1.raw() as u8);
            assert_eq!(r.id().id(), id.as_ref().id());
            assert_eq!(r.filemode(), FileMode::BlobExecutable);
            assert_eq!(r.path(), path.as_c_str());

            const GONE: TreeUpdate<'static> = TreeUpdate::remove(c"README");
            let c = raw(&GONE);
            assert_eq!(c.action, ffi::GIT_TREE_UPDATE_REMOVE);
            assert!(same_id(&c.id, &raw_oid(&Oid::zeroed())));
            assert_eq!(c.filemode, ffi::GIT_FILEMODE_UNREADABLE);
            assert_eq!(GONE.as_ref().path(), c"README");
            assert_eq!(GONE.as_ref().filemode(), FileMode::Unreadable);
            assert_eq!(GONE.as_ref().id().kind(), None);
        }

        fn raw_oid(id: &Oid) -> ffi::git_oid {
            ffi::git_oid {
                type_: id.as_ref().raw_kind(),
                id: id.as_ref().id(),
            }
        }

        /// Every setter rewrites its field alone, through the C struct.
        #[test]
        fn setters_rewrite_their_field_alone() {
            let first = CString::new("first").unwrap();
            let second = CString::new("a/much/longer/second/path").unwrap();
            let mut update = TreeUpdate::upsert(&first, sha1(1).as_ref(), FileMode::Blob);
            let before = raw(&update);

            update.as_mut().set_action(UpdateAction::Remove);
            let c = raw(&update);
            assert_eq!(c.action, ffi::GIT_TREE_UPDATE_REMOVE);
            assert!(same_id(&c.id, &before.id));
            assert_eq!((c.filemode, c.path), (before.filemode, before.path));

            update.as_mut().set_filemode(FileMode::Link);
            assert_eq!(raw(&update).filemode, ffi::GIT_FILEMODE_LINK);
            assert_eq!(update.as_ref().filemode(), FileMode::Link);

            update.as_mut().set_path(&second);
            assert_eq!(raw(&update).path, second.as_ptr());
            assert_eq!(update.as_ref().path(), second.as_c_str());

            update.as_mut().set_id(sha1(2).as_ref());
            assert!(same_id(&raw(&update).id, &raw_oid(&sha1(2))));
            {
                let mut m = update.as_mut();
                let mut id = m.id_mut();
                id.set_kind(OidType::Sha256);
                assert!(id.id_mut().set_elem(31, 0xff));
            }
            let c = raw(&update);
            assert_eq!(c.id.type_, OidType::Sha256.raw() as u8);
            assert_eq!(c.id.id[..20], [2; 20]);
            assert_eq!(c.id.id[31], 0xff);
            assert_eq!(update.as_ref().id().kind(), Some(OidType::Sha256));

            update.as_mut().set_action(UpdateAction::Upsert);
            let c = raw(&update);
            assert_eq!(c.action, ffi::GIT_TREE_UPDATE_UPSERT);
            assert_eq!(c.filemode, ffi::GIT_FILEMODE_LINK);
            assert_eq!(c.path, second.as_ptr());
        }

        /// The handles reborrow onto the same struct, and the exclusive one reaches the
        /// getters; a copy, by `Copy` or through a handle, is a value of its own.
        #[test]
        fn handles_and_copies() {
            let mut update = TreeUpdate::remove(c"x");
            let at = core::ptr::from_ref(&update).cast::<ffi::git_tree_update>();
            {
                let mut m = update.as_mut();
                assert_eq!(m.as_mut_ptr().cast_const(), at);
                assert_eq!(m.as_mut().as_mut_ptr().cast_const(), at);
                assert_eq!(m.as_ref().as_ptr(), at);
                m.as_mut().set_path(c"y");
                assert_eq!(m.as_ref().path(), c"y");
            }
            assert_eq!(update.as_ref().as_ptr(), at);

            let copy = update;
            let through = update.as_ref().to_tree_update();
            assert_eq!(copy, update);
            assert_eq!(through, update);
            update.as_mut().set_action(UpdateAction::Upsert);
            assert_eq!(copy.as_ref().action(), UpdateAction::Remove);
            assert_ne!(copy, update);
            assert_eq!(through.as_ref().path(), c"y");
        }

        /// Equality compares every field, the path by its contents.
        #[test]
        fn equality_compares_every_field() {
            let owned = CString::new("p").unwrap();
            let base = TreeUpdate::upsert(c"p", sha1(1).as_ref(), FileMode::Blob);
            assert_eq!(
                base,
                TreeUpdate::upsert(&owned, sha1(1).as_ref(), FileMode::Blob)
            );
            let mut other = base;
            other.as_mut().set_path(c"q");
            assert_ne!(base, other);
            let mut other = base;
            other.as_mut().set_filemode(FileMode::Tree);
            assert_ne!(base, other);
            let mut other = base;
            other.as_mut().id_mut().set_raw_kind(0);
            assert_ne!(base, other);
            let mut other = base;
            assert!(other.as_mut().id_mut().id_mut().set_elem(0, 2));
            assert_ne!(base, other);
            let mut other = base;
            other.as_mut().set_action(UpdateAction::Remove);
            assert_ne!(base, other);
        }

        #[test]
        fn debug_shows_every_field() {
            let mut update = TreeUpdate::remove(c"README");
            let expected = format!(
                "TreeUpdate {{ action: Remove, id: Oid {{ kind: 0, id: {} }}, \
                 filemode: Unreadable, path: \"README\" }}",
                "0".repeat(64)
            );
            assert_eq!(format!("{update:?}"), expected);
            assert_eq!(format!("{:?}", update.as_ref()), expected);
            assert_eq!(format!("{:?}", update.as_mut()), expected);
            let path: &CStr = c"a/b";
            let up = TreeUpdate::upsert(path, sha1(0x0f).as_ref(), FileMode::Commit);
            let shown = format!("{up:?}");
            assert!(
                shown.starts_with("TreeUpdate { action: Upsert, id: Oid { kind: Sha1, id: 0f0f")
            );
            assert!(shown.ends_with(" }, filemode: Commit, path: \"a/b\" }"));
        }
    }

    /// `git_tree_entry`'s owner, handles and lifecycle policy.
    mod tree_entry {
        use crate::libgit2::object::tree::*;
        use crate::libgit2::oid::oid::{Oid, OidRef, OidType};
        use crate::libgit2::util::errors::ErrorClass;
        use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
        use core::ffi::{CStr, c_char, c_int, c_void};
        use core::mem::size_of;
        use ffibox::CDrop;
        use std::process::Command;
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// `testrepo.git`; commit `763d71a`'s tree holds `README` and the nested
        /// `ab/de/fgh/1.txt`.
        const TESTREPO: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/resources/testrepo.git\0"
        );
        const TREE: &CStr = c"ae90f12eea699729ed24555e40b9fd669da12a12";
        const README: &CStr = c"1385f264afb75a56a5bec74243be9b367ba4ca08";
        const NESTED: &CStr = c"1f67fc4386b2d171e0d21be1c447e12660561f9b";

        /// Runs `body` alone in a child process, where nothing has initialised libgit2.
        fn isolated(test: &str, body: fn()) {
            let name = test.split_once("::").map_or(test, |(_, rest)| rest);
            if std::env::var(CHILD).as_deref() == Ok(name) {
                body();
                return;
            }
            let out = Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, name)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "child `{name}` failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        /// Whether [`Refusing`] refuses every request.
        static REFUSE: AtomicBool = AtomicBool::new(false);

        unsafe extern "C" {
            fn malloc(n: usize) -> *mut c_void;
            fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
            fn free(p: *mut c_void);
        }

        /// libc's allocator, refusing every request while [`REFUSE`] is set.
        struct Refusing;

        impl Refusing {
            fn grant() -> bool {
                !REFUSE.load(SeqCst)
            }
        }

        // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
        unsafe impl Alloc for Refusing {
            fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                if !Self::grant() {
                    return None;
                }
                // SAFETY: plain libc allocation.
                NonNull::new(unsafe { malloc(len) }.cast())
            }
            unsafe fn realloc(
                p: Option<NonNull<u8>>,
                len: usize,
                _: Option<&CStr>,
                _: i32,
            ) -> Option<NonNull<u8>> {
                if !Self::grant() {
                    return None;
                }
                let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
                // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
                NonNull::new(unsafe { realloc(p, len) }.cast())
            }
            unsafe fn free(p: NonNull<u8>) {
                // SAFETY: `p` is a live libc block the caller gives up.
                unsafe { free(p.as_ptr().cast()) }
            }
        }

        /// Installs [`Refusing`] as libgit2's allocator, before anything is allocated.
        fn install_refusing() {
            let allocator = Allocator::new::<Refusing>();
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

        /// `testrepo.git` and the tree `TREE` of it, raw; both freed on drop. Stands in for
        /// the `git_tree` wrapper.
        struct Tree {
            repo: *mut ffi::git_repository,
            tree: *mut ffi::git_tree,
        }

        impl Tree {
            fn open() -> Self {
                assert!(runtime_pinned());
                let mut repo = ptr::null_mut();
                // SAFETY: a writable slot and a NUL-terminated path; no ceiling directories.
                let rc = unsafe {
                    ffi::git_repository_open_ext(
                        &mut repo,
                        TESTREPO.as_ptr().cast(),
                        ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                        ptr::null(),
                    )
                };
                assert_eq!(rc, 0);
                let id = Oid::from_string(TREE, OidType::Sha1).unwrap();
                let mut tree = ptr::null_mut();
                // SAFETY: a writable slot, the open repository and a live id of its type.
                let rc = unsafe { ffi::git_tree_lookup(&mut tree, repo, id.as_ref().as_ptr()) };
                assert_eq!(rc, 0);
                Self { repo, tree }
            }

            /// The tree's own entry `name`, borrowed for as long as the tree is.
            fn entry(&self, name: &CStr) -> TreeEntryRef<'_> {
                // SAFETY: the live tree, and a NUL-terminated name read for the call; the
                // entry is the tree's, unchanged until it is freed, after `'_`.
                unsafe {
                    let entry = ffi::git_tree_entry_byname(self.tree, name.as_ptr());
                    TreeEntryRef::from_ptr(entry.cast_mut()).unwrap()
                }
            }

            /// `git_tree_entry_bypath(path)`, adopted.
            fn bypath(&self, path: &CStr) -> TreeEntryBox {
                let mut out = ptr::null_mut();
                // SAFETY: a writable slot, the live tree and a NUL-terminated path.
                let rc = unsafe { ffi::git_tree_entry_bypath(&mut out, self.tree, path.as_ptr()) };
                assert_eq!(rc, 0);
                // SAFETY: a fresh entry the caller owns, freed by `git_tree_entry_free`.
                unsafe { TreeEntryBox::from_c(out) }.unwrap()
            }
        }

        impl Drop for Tree {
            fn drop(&mut self) {
                // SAFETY: both are live and freed once, the tree first; every entry
                // borrowed from the tree borrowed `self`.
                unsafe {
                    ffi::git_tree_free(self.tree);
                    ffi::git_repository_free(self.repo);
                }
            }
        }

        /// C's name, hex id and raw mode of a live entry.
        fn view(entry: TreeEntryRef<'_>) -> (String, String, u32) {
            let p = entry.as_ptr();
            // SAFETY: `p` is a live entry the handle borrows; its name and id live as long
            // as it does and are copied at once.
            unsafe {
                let name = CStr::from_ptr(ffi::git_tree_entry_name(p));
                let id = OidRef::from_ptr(ffi::git_tree_entry_id(p).cast_mut()).unwrap();
                (
                    name.to_str().unwrap().to_owned(),
                    id.to_hex_string().unwrap(),
                    ffi::git_tree_entry_filemode_raw(p),
                )
            }
        }

        fn expected(name: &str, id: &CStr) -> (String, String, u32) {
            (
                name.into(),
                id.to_str().unwrap().into(),
                ffi::GIT_FILEMODE_BLOB,
            )
        }

        /// `src/libgit2/tree.h`'s `struct git_tree_entry`, which the bindings keep opaque:
        /// an entry Rust builds, for the handles' paths that never reach libgit2.
        #[repr(C)]
        struct RawEntry {
            attr: u16,
            filename_len: u16,
            oid: ffi::git_oid,
            filename: *const c_char,
        }

        fn raw_entry() -> RawEntry {
            RawEntry {
                attr: ffi::GIT_FILEMODE_BLOB as u16,
                filename_len: 6,
                oid: ffi::git_oid::default(),
                filename: c"README".as_ptr(),
            }
        }

        #[test]
        fn layout_matches_c() {
            assert_eq!(size_of::<TreeEntry>(), size_of::<ffi::git_tree_entry>());
            assert_eq!(
                size_of::<TreeEntryBox>(),
                size_of::<*mut ffi::git_tree_entry>()
            );
            assert_eq!(
                size_of::<TreeEntryRef<'_>>(),
                size_of::<*const ffi::git_tree_entry>()
            );
            assert_eq!(
                size_of::<TreeEntryMut<'_>>(),
                size_of::<*const ffi::git_tree_entry>()
            );
            assert_eq!(size_of::<TreeEntryFree>(), 0);
        }

        /// The handles' seams over an entry Rust built, never read, so Miri runs them.
        #[test]
        fn handles_without_c() {
            // SAFETY: NULL is refused without being read.
            assert!(unsafe { TreeEntryRef::from_ptr(ptr::null_mut()) }.is_none());
            let mut raw = raw_entry();
            let p: *mut ffi::git_tree_entry = ptr::from_mut(&mut raw).cast();
            // SAFETY: `raw` is live for the handle's use and reached by it alone.
            let mut m = unsafe { TreeEntryMut::from_ptr(p) }.unwrap();
            assert_eq!(m.as_mut_ptr(), p);
            assert_eq!(m.as_mut().as_mut_ptr(), p);
            assert_eq!(m.as_ref().as_ptr(), p.cast_const());
            assert_eq!(format!("{:?}", m.as_ref()), "TreeEntry { .. }");
            assert_eq!(format!("{m:?}"), "TreeEntry { .. }");
        }

        /// An entry borrowed from a tree copies into one of its own, which outlives the
        /// tree and its repository, as does an entry `bypath` hands out; each owner copies
        /// again, and the policy releases a copy driven by hand.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn copies_outlive_the_tree() {
            let tree = Tree::open();
            let borrowed = tree.entry(c"README");
            assert_eq!(view(borrowed), expected("README", README));
            let copy = borrowed.to_tree_entry().unwrap();
            let nested = tree.bypath(c"ab/de/fgh/1.txt");
            let nested_copy = nested.as_ref().to_tree_entry().unwrap();
            drop(tree);

            assert_eq!(view(copy.as_ref()), expected("README", README));
            let clone = copy.clone();
            drop(copy);
            assert_eq!(view(clone.as_ref()), expected("README", README));
            drop(nested);
            assert_eq!(view(nested_copy.as_ref()), expected("1.txt", NESTED));
            let again = nested_copy.try_clone().unwrap();
            assert_eq!(view(again.as_ref()), expected("1.txt", NESTED));

            let p = NonNull::new(again.as_ptr()).unwrap();
            // SAFETY: `again` keeps the entry live; nothing writes it.
            let dup = unsafe { TreeEntryFree.c_dup(p) }.unwrap();
            // SAFETY: `dup` is a fresh copy this test owns, released once here.
            unsafe { TreeEntryFree.c_drop(dup) };
            assert_eq!(view(again.as_ref()), expected("1.txt", NESTED));
        }

        /// An allocation failure is reported as libgit2's out-of-memory error, and a failed
        /// copy leaves the original usable.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn allocation_failure_is_reported() {
            isolated(
                concat!(module_path!(), "::allocation_failure_is_reported"),
                || {
                    install_refusing();
                    let tree = Tree::open();
                    let owned = tree.bypath(c"README");
                    REFUSE.store(true, SeqCst);
                    let borrowed = tree.entry(c"README").to_tree_entry();
                    let from_owner = owned.as_ref().to_tree_entry();
                    let cloned = owned.try_clone();
                    REFUSE.store(false, SeqCst);
                    for e in [borrowed.unwrap_err(), from_owner.unwrap_err()] {
                        assert_eq!(e.class(), Some(ErrorClass::NoMemory));
                        assert_eq!(e.message(), c"Out of memory");
                    }
                    assert!(cloned.is_none());
                    assert_eq!(view(owned.clone().as_ref()), expected("README", README));
                },
            );
        }

        /// `bypath` looks into a tree object at every depth and hands out an owned copy;
        /// an object of any other type is refused in Rust, with the error set on the
        /// thread, before libgit2 reads it as a tree (the corrected divergence); and a
        /// refused allocation for the copy is libgit2's out-of-memory error.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn bypath_looks_only_into_trees() {
            isolated(
                concat!(module_path!(), "::bypath_looks_only_into_trees"),
                || {
                    use crate::libgit2::object::object::Object;
                    use crate::libgit2::repository::repository::Repository;
                    use crate::libgit2::util::errors::ErrorCode;
                    install_refusing();
                    let repo =
                        Repository::open(CStr::from_bytes_with_nul(TESTREPO.as_bytes()).unwrap())
                            .unwrap();
                    let lookup = |hex: &CStr| {
                        let id = Oid::from_string(hex, OidType::Sha1).unwrap();
                        Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap()
                    };
                    let tree = lookup(TREE);
                    let entry = TreeEntry::bypath(tree.as_ref(), c"ab/de/fgh/1.txt").unwrap();
                    assert_eq!(view(entry.as_ref()), expected("1.txt", NESTED));
                    assert_eq!(
                        view(
                            TreeEntry::bypath(tree.as_ref(), c"README")
                                .unwrap()
                                .as_ref()
                        ),
                        expected("README", README)
                    );
                    let e = TreeEntry::bypath(tree.as_ref(), c"ab/nope").unwrap_err();
                    assert_eq!(e.code(), Some(ErrorCode::NotFound));
                    assert_eq!(e.class(), Some(ErrorClass::Tree));

                    // Not trees: the commit of `TREE`, a blob and a tag.
                    for hex in [
                        c"763d71aadf09a7951596c9746c024e7eece7c7af",
                        README,
                        c"b25fa35b38051e4ae45d4222e795f9df2e43f1d1",
                    ] {
                        let other = lookup(hex);
                        let e = TreeEntry::bypath(other.as_ref(), c"README").unwrap_err();
                        assert_eq!(e.code(), Some(ErrorCode::InvalidSpec), "{hex:?}");
                        assert_eq!(e.class(), Some(ErrorClass::Tree));
                        assert_eq!(e.message(), c"the object is not a tree");
                        assert_eq!(last().message(), c"the object is not a tree");
                    }

                    REFUSE.store(true, SeqCst);
                    let refused = TreeEntry::bypath(tree.as_ref(), c"README");
                    REFUSE.store(false, SeqCst);
                    let e = refused.unwrap_err();
                    assert_eq!(e.code(), Some(ErrorCode::Generic));
                    assert_eq!(e.class(), Some(ErrorClass::NoMemory));
                    drop(tree);
                    drop(repo);
                    assert_eq!(view(entry.as_ref()), expected("1.txt", NESTED));
                },
            );
        }

        /// When libgit2 cannot be initialised, a copy reports C's static "not initialized"
        /// error without calling into it.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn copy_without_runtime_fails_cleanly() {
            isolated(
                concat!(module_path!(), "::copy_without_runtime_fails_cleanly"),
                || {
                    install_refusing();
                    REFUSE.store(true, SeqCst);
                    let mut raw = raw_entry();
                    // SAFETY: `raw` is live and unchanged for the handle's use.
                    let entry = unsafe { TreeEntryRef::from_ptr(ptr::from_mut(&mut raw).cast()) };
                    let e = entry.unwrap().to_tree_entry().unwrap_err();
                    REFUSE.store(false, SeqCst);
                    assert!(!runtime_pinned(), "libgit2 could not initialise");
                    assert_eq!(e.class(), Some(ErrorClass::Invalid));
                    assert_eq!(e.message(), c"library has not been initialized");
                },
            );
        }
    }
}
