//! Safe wrappers over `include/git2/tree.h`.
//!
//! `git_tree_walk` takes its traversal order (`git_treewalk_mode`) by value and refuses
//! any value the header does not name with a `GIT_ERROR_INVALID` error before reading
//! the tree, so a raw value is converted fallibly ([`WalkMode::from_raw`]) and never
//! transmuted.
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

use core::ffi::c_int;
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use ffibox::{CBorrowedPtr, CBox, CCell, CDrop};

use crate::ffi;
use crate::libgit2::repository::repository::RepositoryRef;
use crate::libgit2::util::errors::{ErrorBuf, last};
use crate::libgit2::util::sys::errors::runtime_pinned;

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

/// What one `git_tree_update` asks `git_tree_create_updated` to do at its path: insert
/// or replace the entry there, or remove it.
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

    /// A builder for `repo` holding a copy of each of `source`'s entries: their names,
    /// ids and file modes as the tree spells them, unchecked. The builder keeps nothing
    /// of `source`, which may be freed as soon as this returns.
    ///
    /// `source` may come from any repository. Its ids are copied as they are, so a tree
    /// of a repository of another object id type gives a builder that writes a malformed
    /// tree, as libgit2 does.
    ///
    /// Fails, with libgit2's error, only when it runs out of memory, having freed the
    /// entries it copied.
    ///
    /// Raw seam: this crate does not wrap `git_tree` yet.
    ///
    /// # Safety
    ///
    /// `source` must address a live `git_tree` (its caller holds a reference to it) that
    /// no one frees during the call.
    ///
    /// Wraps: git_treebuilder_new
    pub unsafe fn from_tree(
        repo: RepositoryRef<'r>,
        source: NonNull<ffi::git_tree>,
    ) -> Result<TreeBuilderBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `repo` a live repository on this thread, which
        // the builder only records (no entry is checked while seeding); `source` a live
        // tree for the call, per this function's contract, only read. The runtime is
        // pinned.
        let rc = unsafe {
            ffi::git_treebuilder_new(&mut out, repo.as_ptr().cast_mut(), source.as_ptr())
        };
        // SAFETY: as in `new`: a fresh builder for `repo` the caller owns alone, or NULL.
        constructed(rc, unsafe { TreeBuilderBox::from_c(out) })
    }
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
}
