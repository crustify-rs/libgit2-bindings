//! Safe wrappers over `include/git2/tree.h`.
//!
//! `git_tree_walk` takes its traversal order (`git_treewalk_mode`) by value and refuses
//! any value the header does not name with a `GIT_ERROR_INVALID` error before reading
//! the tree, so a raw value is converted fallibly ([`WalkMode::from_raw`]) and never
//! transmuted. Its callback (`git_treewalk_cb`) is a [`WalkCallback`] over a Rust
//! closure, lent each entry and the path of the tree holding it for the call only, and
//! answering with a [`WalkAction`].
//!
//! # Tree updates
//!
//! A [`TreeUpdate`] is a `git_tree_update`: one change, an [`UpdateAction`] at a path
//! with the id and [`FileMode`] of the entry an upsert puts there, of the array
//! `git_tree_create_updated` ([`Tree::create_updated`]) applies to a tree. libgit2
//! neither allocates, copies nor frees one: it reads the caller's array in place
//! through a `const` pointer, for that call only, and keeps none of it. So Rust holds an
//! update by value, `Copy`, with no teardown, `#[repr(transparent)]` over the C struct,
//! so that a `[TreeUpdate]` is the array C reads. Its path is a `const char *` libgit2
//! dereferences unchecked and never frees: a `&'p CStr` the update borrows for `'p`,
//! never NULL. Its action and mode are only ever set from [`UpdateAction`] and
//! [`FileMode`] variants.
//!
//! # Trees
//!
//! A [`Tree`] is a `git_tree`: a tree object, parsed from a repository's object
//! database. It is an [`Object`](crate::libgit2::object::object::Object) of type tree:
//! the struct starts with the `git_object`, and `git_tree_lookup`, `git_tree_free` and
//! `git_tree_dup` are `git_object_lookup` (of type tree), `git_object_free` and
//! `git_object_dup` behind a cast. It is opaque: libgit2 publishes no field.
//!
//! So a tree is owned as an object is (see [`object`](crate::libgit2::object::object)'s
//! module docs): a [`TreeArc`] is one counted reference, given back by `git_tree_free`,
//! and its `Clone` takes another on the same object through `git_tree_dup`; the
//! repository's cache counts one more no owner sees, so no owner proves itself the only
//! one, and none lends [`TreeMut`]. The parse fills the tree's entries, whose names
//! point into the raw object data the tree keeps a reference to, before libgit2 hands
//! the object out, and nothing writes them until the last reference frees them with the
//! tree (`git_tree__free`), so the shared [`TreeRef`] reads an object nobody changes,
//! and an entry borrowed from it lives as long as that borrow. A tree carries the
//! lifetime `'r` of the repository borrow it was looked up through, whose cache it
//! shares and which looking up its subtrees reaches, and is neither `Send` nor `Sync`,
//! as an object is.
//!
//! A tree is an object, and an object of type tree is a tree: [`TreeRef::as_object`]
//! views a tree as its object, [`TreeRef::from_object`] views an object of type tree
//! as a tree, and [`Tree::from_object`] and [`Tree::into_object`] hand one counted
//! reference from one owner to the other, checking the object's type on the way in.
//! libgit2 itself never checks the type of what it is given as a tree, so every wrapper
//! taking one takes a [`TreeRef`].
//!
//! The shared handle reads the tree: its id ([`TreeRef::id`]) and repository
//! ([`TreeRef::owner`]), and its entries, by index ([`TreeRef::get`]), name or id, or in
//! order ([`TreeRef::iter`]), each lent for as long as the handle's borrow.
//! [`TreeRef::walk`] walks the tree and the subtrees libgit2 looks up in its repository
//! on the way, calling a [`WalkCallback`] with each entry.
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
//! Each entry is the builder's own allocation, keyed by its name. A shared handle looks
//! one up ([`get`](TreeBuilderRef::get)) and lends it for as long as the handle's borrow;
//! the exclusive handle inserts ([`insert`](TreeBuilderMut::insert), which replaces the
//! id and mode of an entry of the same name in place), removes entries (by name, or
//! those a filter picks: [`remove_if`](TreeBuilderMut::remove_if)), clears them, and
//! writes them out ([`write`](TreeBuilderMut::write), which refills the builder's own
//! write buffer). No entry handle outlives the borrow that lent it, so none sees an entry
//! changed or freed.
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
//!
//! An entry's name and id live inside it (an owned entry's block, a tree's raw data, a
//! builder's entry), so the getters lend them for the handle's lifetime. An entry
//! keeps no repository: [`to_object`](TreeEntryRef::to_object) takes the one to look
//! its object up in, and refuses one of another object id type.
//!
//! A [`TreeBuilderFilter`] is the closure `git_treebuilder_filter`
//! ([`TreeBuilderMut::remove_if`]) calls with each of a builder's entries, to tell it
//! which to remove.

use core::any::Any;
use core::cmp::Ordering;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::fmt;
use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::ops::{ControlFlow, Range};
use core::ptr::{self, NonNull, addr_of, addr_of_mut};
use std::panic::{self, AssertUnwindSafe};

use ffibox::{
    CArc, CBorrowedPtr, CBox, CCell, CDrop, CDupClone, CRefClone, CZeroable, define_ctype,
    impl_cdrop,
};

use crate::ffi;
use crate::libgit2::core::types::{FileMode, ObjectType};
use crate::libgit2::object::object::{ObjectArc, ObjectRef, id_fits};
use crate::libgit2::oid::oid::{Oid, OidMut, OidRef};
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

/// What a [`WalkCallback`] answers for one entry of a tree walk.
///
/// libgit2 reads the C callback's result as a sign: 0 goes on, a positive value skips,
/// a negative one stops the walk, which then returns it. A closure answers with a
/// variant, which the trampoline returns as 0, 1 and `GIT_EUSER`.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WalkAction {
    /// Go on with the walk: in pre-order, into the subtree the entry names, if it names
    /// one.
    Continue,
    /// In pre-order, do not walk the subtree the entry names, and go on with the entry
    /// after it; libgit2 does not even load that subtree. A post-order walk has already
    /// walked the subtree when it calls back, and goes on as for `Continue`.
    Skip,
    /// Stop the walk at once: the walk returns `GIT_EUSER`. libgit2 sets a
    /// `GIT_ERROR_CALLBACK` error "git_tree_walk callback returned -7" only when the
    /// thread has no libgit2 error yet: one the closure set, or one an earlier call left,
    /// stays the thread's.
    Stop,
}

impl WalkAction {
    /// The value the trampoline returns to libgit2 for this answer.
    const fn raw(self) -> c_int {
        match self {
            Self::Continue => 0,
            Self::Skip => 1,
            Self::Stop => ErrorCode::User.raw(),
        }
    }

    /// How libgit2 reads a callback's result `rc`.
    const fn from_raw(rc: c_int) -> Self {
        match rc {
            0 => Self::Continue,
            1.. => Self::Skip,
            _ => Self::Stop,
        }
    }
}

/// The closure a [`WalkCallback`] calls: the path of the tree holding the entry, and the
/// entry, both lent for the call.
type WalkFn<'f> = dyn FnMut(&CStr, TreeEntryRef<'_>) -> WalkAction + 'f;

/// A tree walk callback (`git_treewalk_cb`): a Rust closure `git_tree_walk` calls with
/// each entry it reaches, through a C function and a payload pointing at this handle,
/// on the calling thread.
///
/// Built over a closure with [`new`](Self::new). Its `extern "C"` trampoline lends the
/// closure, for the call only:
///
/// - `root`, the path from the walked tree to the tree holding the entry, each subtree's
///   name followed by `/`: empty for the walked tree's own entries, `dir/sub/` for the
///   entries of `dir/sub`. Names are bytes, as git stores them, not necessarily UTF-8.
/// - the entry, a [`TreeEntryRef`]: libgit2 frees a subtree, with its entries, once it
///   has walked it. [`to_tree_entry`](TreeEntryRef::to_tree_entry) copies one to keep.
///
/// and returns the closure's [`WalkAction`]. A panic in the closure does not cross into
/// C: the trampoline stops the walk with `GIT_EUSER`, and the panic resumes once control
/// is back in Rust, from [`call`](Self::call) or from [`TreeRef::walk`].
///
/// ```
/// use core::ffi::CStr;
/// use libgit2::libgit2::core::types::FileMode;
/// use libgit2::libgit2::object::tree::{TreeBuilder, TreeEntryRef, WalkAction, WalkCallback};
/// use libgit2::libgit2::oid::oid::{Oid, OidType};
/// use libgit2::libgit2::repository::repository::Repository;
///
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// // A builder's entry stands in for one of a walked tree.
/// let repo = Repository::new().unwrap();
/// let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
/// let id = Oid::from_string(c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750", OidType::Sha1).unwrap();
/// builder.as_mut().insert(c"sub", id.as_ref(), FileMode::Commit).unwrap();
///
/// let mut seen = Vec::new();
/// let mut collect = |root: &CStr, _: TreeEntryRef<'_>| {
///     seen.push(root.to_owned());
///     WalkAction::Skip
/// };
/// let mut callback = WalkCallback::new(&mut collect);
/// let entry = builder.as_ref().get(c"sub").unwrap();
/// assert_eq!(callback.call(c"dir/", entry), WalkAction::Skip);
/// drop(callback);
/// assert_eq!(seen, [c"dir/"]);
/// ```
///
/// Neither the root nor the entry outlives the call:
///
/// ```compile_fail,E0521
/// use core::ffi::CStr;
/// use libgit2::libgit2::object::tree::{TreeEntryRef, WalkAction, WalkCallback};
///
/// fn keep<'k>(kept: &mut Option<TreeEntryRef<'k>>) {
///     let mut f = |_: &CStr, entry: TreeEntryRef<'_>| {
///         *kept = Some(entry);
///         WalkAction::Continue
///     };
///     let _ = WalkCallback::new(&mut f);
/// }
/// ```
///
/// ```compile_fail,E0521
/// use core::ffi::CStr;
/// use libgit2::libgit2::object::tree::{TreeEntryRef, WalkAction, WalkCallback};
///
/// fn keep<'k>(kept: &mut Option<&'k CStr>) {
///     let mut f = |root: &CStr, _: TreeEntryRef<'_>| {
///         *kept = Some(root);
///         WalkAction::Continue
///     };
///     let _ = WalkCallback::new(&mut f);
/// }
/// ```
///
/// It borrows its closure exclusively and stays on its thread: libgit2 calls it
/// synchronously, and the closure need not be `Send`.
///
/// ```compile_fail,E0277
/// use core::ffi::CStr;
/// use libgit2::libgit2::object::tree::{TreeEntryRef, WalkAction, WalkCallback};
///
/// fn send<T: Send>(_: T) {}
/// let mut f = |_: &CStr, _: TreeEntryRef<'_>| WalkAction::Continue;
/// send(WalkCallback::new(&mut f));
/// ```
///
/// Wraps: git_treewalk_cb
/// Handwritten
pub struct WalkCallback<'f> {
    f: &'f mut WalkFn<'f>,
    /// Whether the last call stopped the walk: the closure answered `Stop`, or panicked.
    stopped: bool,
    /// A panic the closure raised inside a C walk, resumed once the walk returns.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> WalkCallback<'f> {
    /// The callback that calls `f` with each entry, until it answers `Stop`.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(&CStr, TreeEntryRef<'_>) -> WalkAction + 'f,
    {
        Self {
            f,
            stopped: false,
            panic: None,
        }
    }

    /// Calls the callback with `root` and `entry`, through the trampoline libgit2 calls,
    /// and returns the answer libgit2 would read from it. A panic of the closure resumes
    /// here.
    pub fn call(&mut self, root: &CStr, entry: TreeEntryRef<'_>) -> WalkAction {
        let rc = self.walk_with(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            // SAFETY: `payload` is this callback, which nothing else uses during the
            // walk; `root` is NUL-terminated and `entry` a live entry nothing writes,
            // both lent for the call: the trampoline's contract.
            unsafe { callback(root.as_ptr(), entry.as_ptr(), payload) }
        });
        WalkAction::from_raw(rc)
    }

    /// Whether the last call stopped the walk: the closure answered
    /// [`Stop`](WalkAction::Stop), or panicked. Tells the callback's `GIT_EUSER` from
    /// libgit2's own errors.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Runs `walk` with the C callback and its payload, as the wrapper of a routine that
    /// calls the callback (`git_tree_walk`) hands them to it, then resumes a panic the
    /// closure raised meanwhile. The payload is this callback, which nothing else
    /// touches until `walk` returns, and only for calls from this thread.
    pub(crate) fn walk_with<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_treewalk_cb, *mut c_void) -> R,
    ) -> R {
        self.stopped = false;
        let result = walk(Some(walk_with), ptr::from_mut(self).cast());
        if let Some(panic) = self.panic.take() {
            std::panic::resume_unwind(panic);
        }
        result
    }
}

impl fmt::Debug for WalkCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WalkCallback")
            .field("stopped", &self.stopped)
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_treewalk_cb` over a [`WalkCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`WalkCallback`] that nothing else uses during the
/// call; `root` be NULL or NUL-terminated, and `entry` NULL or a live entry nothing
/// writes, both live for the call.
unsafe extern "C" fn walk_with(
    root: *const c_char,
    entry: *const ffi::git_tree_entry,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: per this function's contract, `payload` is a live callback borrowed for
    // the call alone.
    let callback = unsafe { &mut *payload.cast::<WalkCallback<'_>>() };
    if callback.panic.is_some() {
        callback.stopped = true;
        return WalkAction::Stop.raw();
    }
    // SAFETY: per this function's contract, a non-NULL `entry` is live and unchanged for
    // the call; the handle given to the closure is valid for its call only.
    let entry = unsafe { TreeEntryRef::from_ptr(entry.cast_mut()) };
    let (false, Some(entry)) = (root.is_null(), entry) else {
        // libgit2 always passes both; stop the walk rather than invent them.
        callback.stopped = true;
        return WalkAction::Stop.raw();
    };
    // SAFETY: `root` is non-NULL, so NUL-terminated and live for the call per this
    // function's contract; the closure borrows it for its call only.
    let root = unsafe { CStr::from_ptr(root) };
    let answer = std::panic::catch_unwind(AssertUnwindSafe(|| (callback.f)(root, entry)));
    let action = answer.unwrap_or_else(|panic| {
        callback.panic = Some(panic);
        WalkAction::Stop
    });
    callback.stopped = action == WalkAction::Stop;
    action.raw()
}

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

/// A tree object (`git_tree`), parsed from a repository's object database: an
/// [`Object`](crate::libgit2::object::object::Object) of type tree, listing the
/// entries of one directory.
///
/// libgit2 allocates, counts and frees it, so it is only ever reached through a
/// [`TreeArc`] and its handles, built by [`lookup`](Self::lookup) and
/// [`lookup_prefix`](Self::lookup_prefix), or adopted from an object owner by
/// [`from_object`](Self::from_object). It is opaque: no field is published.
///
/// `'r` is the borrow of the repository the tree was looked up in, as for an object
/// (see the [module docs](self#trees)). The lifetime only shrinks:
///
/// ```compile_fail
/// use libgit2::libgit2::object::tree::TreeArc;
///
/// fn widen<'r>(tree: TreeArc<'r>) -> TreeArc<'static> {
///     tree
/// }
/// ```
///
/// and a tree stays on the thread that looked it up:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::tree::TreeArc;
///
/// fn send<T: Send>(_: T) {}
/// fn move_out(tree: TreeArc<'static>) {
///     send(tree)
/// }
/// ```
///
/// Wraps: git_tree
/// Handwritten
#[repr(transparent)]
pub struct Tree<'r>(ffi::git_tree, PhantomData<(*const (), &'r ())>);

/// Shared borrow of a [`Tree`]. `Copy`, like `&T`.
///
/// It stays on the thread of the tree's repository, as the owner does:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::tree::TreeRef;
///
/// fn sync<T: Sync>() {}
/// sync::<TreeRef<'static, 'static>>();
/// ```
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct TreeRef<'a, 'r>(CBorrowedPtr<'a, Tree<'r>>);

/// Exclusive borrow of a [`Tree`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// No owner hands one out: every tree is shared with its repository's cache.
///
/// Handwritten
#[repr(transparent)]
pub struct TreeMut<'a, 'r>(
    TreeRef<'a, 'r>,
    // Invariant in the tree, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Tree<'r>>,
);

// SAFETY: `Tree` is `#[repr(transparent)]` over `git_tree` with a zero-sized marker;
// both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Tree<'r>>` (`TreeMut`
// through `TreeRef`) with zero-sized markers and no `Drop`, hand out no reference to the
// C object, and `TreeRef` has no write operation. `TreeMut` is invariant in the tree
// through its `PhantomData<&'a mut Tree<'r>>`.
unsafe impl<'r> CCell for Tree<'r> {
    type C = ffi::git_tree;
    type Ref<'a>
        = TreeRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = TreeMut<'a, 'r>
    where
        Self: 'a;
}

impl<'a, 'r> TreeRef<'a, 'r> {
    /// Borrows a raw pointer; `None` if null.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live tree looked up in a repository that
    /// outlives `'r`, which stays live during `'a` (a reference to it is held for at
    /// least that long) and which no one writes during `'a`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_tree) -> Option<Self> {
        // SAFETY: this function's contract is `CCell::ref_from_ptr`'s: `ptr` is NULL or a
        // live `git_tree` that outlives `'a` and is not written meanwhile.
        unsafe { <Tree<'r> as CCell>::ref_from_ptr(ptr.cast_mut()) }
    }

    /// Read-only pointer to the C object.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_tree {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The tree `object` is, borrowed as long as `object` is; `None` for an object of
    /// another type.
    pub fn from_object(object: ObjectRef<'a, 'r>) -> Option<Self> {
        if !is_tree(object) {
            return None;
        }
        // SAFETY: an object of type tree was allocated and parsed as a `git_tree`,
        // whose first member is that `git_object`, so the pointer addresses the whole
        // tree; the object handle keeps it live and unwritten for `'a`, and its
        // repository outlives `'r`.
        unsafe { Self::from_ptr(object.as_ptr().cast()) }
    }

    /// The tree as the object it is, borrowed as long as this handle: what every
    /// `git_object` routine takes.
    pub fn as_object(&self) -> ObjectRef<'a, 'r> {
        // SAFETY: a `git_tree` starts with its `git_object`, so the tree's pointer is
        // its object's, live and unwritten for `'a` as the tree is, of a repository that
        // outlives `'r`.
        let object = unsafe { ObjectRef::from_ptr(self.as_ptr().cast()) };
        object.expect("a handle is never NULL")
    }

    /// Another counted reference to the tree, as `git_tree_dup` takes one: the same
    /// tree, now also kept alive by the result, which may outlive this borrow but not
    /// the repository.
    ///
    /// Wraps: git_tree_dup
    pub fn to_tree(&self) -> TreeArc<'r> {
        let tree = self.0.as_non_null();
        // SAFETY: the handle addresses a live tree for `'a`, whose reference its holder
        // keeps meanwhile; one more is counted on it here.
        if !unsafe { TreeFree.c_up_ref(tree) } {
            unreachable!("git_tree_dup cannot fail");
        }
        // SAFETY: the reference just counted, given to the result, which `git_tree_free`
        // releases; the tree's repository outlives `'r`, which the result carries.
        unsafe { TreeArc::from_raw(tree.as_ptr()) }.expect("a handle is never NULL")
    }
}

/// Whether `object` is of type tree, as libgit2 recorded it when parsing it.
fn is_tree(object: ObjectRef<'_, '_>) -> bool {
    object.kind() == ObjectType::Tree
}

impl<'a, 'r> TreeMut<'a, 'r> {
    /// Borrows a raw pointer exclusively; `None` if null.
    ///
    /// # Safety
    ///
    /// As [`TreeRef::from_ptr`], plus: nothing else reaches the tree while the result
    /// lives, its repository's cache included.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_tree) -> Option<Self> {
        // SAFETY: the caller upholds `TreeRef::from_ptr`'s contract and exclusivity.
        unsafe { TreeRef::from_ptr(ptr) }.map(|r| TreeMut(r, PhantomData))
    }

    /// Writable pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_tree {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> TreeRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> TreeMut<'_, 'r> {
        TreeMut(self.0, PhantomData)
    }
}

impl fmt::Debug for TreeRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tree").finish_non_exhaustive()
    }
}

impl fmt::Debug for TreeMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases one counted reference to a tree through `git_tree_free`, and takes another
/// through `git_tree_dup`.
///
/// They are `git_object_free` and `git_object_dup` behind a cast: the free decrements
/// the object's atomic count and, at zero, frees the tree with its entries and its
/// reference to the raw object data (`git_tree__free`), never reading its repository;
/// the copy increments the same count and hands back the same pointer. Neither can
/// fail, and neither allocates.
///
/// Wraps: git_tree_free
/// Wraps: git_tree_dup
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct TreeFree;

// SAFETY: `git_tree_free` gives back exactly one counted reference of a live tree (a
// NULL pointer, which it skips, never reaches it); the tree outlives the call through
// that reference, and is freed only once the count, which the cache's reference is part
// of, reaches zero.
unsafe impl<'r> CDrop<Tree<'r>> for TreeFree {
    unsafe fn c_drop(&self, ptr: NonNull<Tree<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live tree of which the
        // caller gives up one reference; `Tree` is transparent over `git_tree`.
        unsafe { ffi::git_tree_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `git_tree_dup` atomically increments the count of the live tree it is given
// and stores that same pointer in `out`, returning 0: the tree then owes one more
// `git_tree_free`, which this policy performs, and `ptr` stays valid. It never fails.
// `c_is_sole_owner` keeps its `false` default: the repository's cache counts a reference
// no owner sees, and a later lookup may take another.
unsafe impl<'r> CRefClone<Tree<'r>> for TreeFree {
    unsafe fn c_up_ref(&self, ptr: NonNull<Tree<'r>>) -> bool {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `ptr` addresses a live tree whose reference
        // the caller holds, per `c_up_ref`'s contract.
        let rc = unsafe { ffi::git_tree_dup(&mut out, ptr.as_ptr().cast()) };
        debug_assert_eq!(out, ptr.as_ptr().cast());
        rc == 0
    }
}

/// One counted reference to a [`Tree`], given back by `git_tree_free` on drop; `clone`
/// takes another through `git_tree_dup`, on the same tree.
pub type TreeArc<'r> = CArc<Tree<'r>, TreeFree>;

impl<'r> Tree<'r> {
    /// The tree `id` names in `repo`.
    ///
    /// The repository's cache answers when it holds the tree, with the tree it holds;
    /// otherwise libgit2 reads it from the object database, parses it and, under the
    /// cache's size limit, caches it, so that every later lookup of the id, as a tree
    /// or as an object, yields the same tree while the cache keeps it.
    ///
    /// Fails as [`Object::lookup`](crate::libgit2::object::object::Object::lookup) with
    /// type [`ObjectType::Tree`] does:
    /// with [`ErrorCode::NotFound`]
    /// when the object database has no such object, or the object is not a tree (class
    /// `GIT_ERROR_INVALID`, "the requested type does not match the type in the ODB"),
    /// and with libgit2's error when the tree cannot be read or parsed. An `id` of
    /// another type than `repo`'s is refused before the call, as there.
    ///
    /// Wraps: git_tree_lookup
    pub fn lookup(repo: RepositoryRef<'r>, id: OidRef<'_>) -> Result<TreeArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `repo` is a live
        // repository on this thread, whose odb libgit2 loads and whose cache it fills
        // through this shared handle, as `RepositoryRef` allows; `id` is a live id of
        // the repository's type (checked above), read for the call only.
        let rc = unsafe { ffi::git_tree_lookup(&mut out, repo.as_ptr().cast_mut(), id.as_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live tree of
        // `repo`, borrowed for `'r`, given to the result and released by
        // `git_tree_free`.
        let tree = unsafe { TreeArc::from_c(out) };
        Ok(tree.expect("a lookup succeeded without a tree"))
    }

    /// The tree whose id starts with the first `len` hex digits of `id`, in `repo`; a
    /// `len` past the repository's full length means the full id.
    ///
    /// The digits past `len` are ignored, and every object in the database is a
    /// candidate, whatever its type: the prefix must name exactly one, which must then
    /// be a tree. Fails as
    /// [`Object::lookup_prefix`](crate::libgit2::object::object::Object::lookup_prefix)
    /// with type tree does, and refuses an `id` of another type than `repo`'s as it
    /// does, whatever `len`.
    ///
    /// Wraps: git_tree_lookup_prefix
    pub fn lookup_prefix(
        repo: RepositoryRef<'r>,
        id: OidRef<'_>,
        len: usize,
    ) -> Result<TreeArc<'r>, RepositoryError> {
        id_fits(repo, id)?;
        let mut out = ptr::null_mut();
        // SAFETY: as in `lookup`; libgit2 clamps `len` to the repository's length, and
        // copies at most `(len + 1) / 2` of `id`'s bytes, with its type, into its key.
        let rc = unsafe {
            ffi::git_tree_lookup_prefix(&mut out, repo.as_ptr().cast_mut(), id.as_ptr(), len)
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live tree of
        // `repo`, borrowed for `'r`, given to the result and released by
        // `git_tree_free`.
        let tree = unsafe { TreeArc::from_c(out) };
        Ok(tree.expect("a lookup succeeded without a tree"))
    }

    /// `object`'s reference, as a tree's: the same object, still counted once, now
    /// owned as the tree it is; `object` back, untouched, when it is not of type tree.
    pub fn from_object(object: ObjectArc<'r>) -> Result<TreeArc<'r>, ObjectArc<'r>> {
        if !is_tree(object.as_ref()) {
            return Err(object);
        }
        let raw = object.into_c();
        // SAFETY: the reference the object owner gave up, to a live object of type
        // tree, so a `git_tree`, of a repository borrowed for `'r`; `git_tree_free`
        // releases it as `git_object_free` would.
        let tree = unsafe { TreeArc::from_c(raw.cast()) };
        Ok(tree.expect("an owner is never NULL"))
    }

    /// `tree`'s reference, as an object's: the same tree, still counted once, now owned
    /// as an object.
    pub fn into_object(tree: TreeArc<'r>) -> ObjectArc<'r> {
        let raw = tree.into_c();
        // SAFETY: the reference the tree owner gave up, to a live tree, which starts
        // with its `git_object`, of a repository borrowed for `'r`; `git_object_free`
        // releases it as `git_tree_free` would.
        let object = unsafe { ObjectArc::from_c(raw.cast()) };
        object.expect("an owner is never NULL")
    }

    /// Writes the tree `baseline` becomes once `updates` are applied to it (or an empty
    /// tree, for `None`) into `repo`'s object database, with every tree above a changed
    /// path, and returns the new root tree's id. `baseline` itself is left as it is.
    ///
    /// libgit2 applies the updates in the order of their paths' bytes, not the slice's:
    /// it descends into each path's trees, creating the missing ones, upserts or
    /// removes its last component, and writes each changed tree on the way back up,
    /// dropping a tree the updates left empty. With no update it writes `baseline`
    /// again. The ids of the trees, commits and blobs it writes are of `repo`'s type.
    ///
    /// Fails with `-1` and a `GIT_ERROR_TREE` error for two updates of one path
    /// ("duplicate entries given for update"), a path crossing an entry that is not a
    /// tree ("D/F conflict when updating tree"), and an upsert replacing an entry with
    /// one of another type ("cannot replace 'blob' with 'tree' at '…'"); with
    /// libgit2's error for an upsert the tree builder refuses (an invalid name or mode,
    /// a zero id, or, under strict object creation, an object `repo` lacks with the
    /// mode's type), a removal of an entry the tree lacks ("failed to remove entry:
    /// file isn't in the tree - …"), a tree that cannot be read, or an object database
    /// that cannot be loaded or written. Trees written before the failure stay in the
    /// object database.
    ///
    /// Three inputs are refused before libgit2 runs, with `-1`, whatever the updates'
    /// order: a `baseline` of another object id type than `repo`'s, and an upsert of
    /// an id of another type that libgit2 would look up (any mode but a commit's, a
    /// non-zero id), each with `GIT_ERROR_INVALID` ("specified object ID type does not
    /// match repository object ID type"), as libgit2 would format them out of bounds of
    /// the loose object path; and an upsert with a path component of more than 65535
    /// bytes, with `GIT_ERROR_INVALID` ("tree entry path too long"), which libgit2
    /// would write into a corrupt tree, its length truncated (see
    /// [`TreeBuilderMut::insert`]).
    ///
    /// Wraps: git_tree_create_updated
    pub fn create_updated(
        repo: RepositoryRef<'r>,
        baseline: Option<TreeRef<'_, '_>>,
        updates: &[TreeUpdate<'_>],
    ) -> Result<Oid, RepositoryError> {
        if let Some(baseline) = baseline {
            // libgit2 looks the baseline's subtrees up in `repo` by their ids.
            id_fits(repo, baseline.as_object().id())?;
        }
        for update in updates.iter().map(TreeUpdate::as_ref) {
            if update.action() != UpdateAction::Upsert {
                continue;
            }
            if update
                .path()
                .to_bytes()
                .split(|&b| b == b'/')
                .any(|c| c.len() > NAME_MAX)
            {
                set(ErrorClass::Invalid, c"tree entry path too long");
                return Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()));
            }
            let id = update.id();
            if !matches!(update.filemode(), FileMode::Commit | FileMode::Unreadable)
                && !id.is_zero()
            {
                // The tree builder looks the object up: its id must be of `repo`'s type.
                id_fits(repo, id)?;
            }
        }
        let mut id = Oid::zeroed();
        let baseline = baseline.map_or(ptr::null_mut(), |tree| tree.as_ptr().cast_mut());
        // SAFETY: a writable id; a live repository on this thread, used through this shared
        // handle; NULL or a live tree of `repo`'s id type, counted and released in the
        // call; `updates.len()` live updates (transparent over `git_tree_update`), with
        // borrowed NUL-terminated paths and named actions and modes, read in place. Ids
        // looked up fit `repo`, and no upserted component exceeds 65535 bytes (above).
        let rc = unsafe {
            ffi::git_tree_create_updated(
                id.as_mut().as_mut_ptr(),
                repo.as_ptr().cast_mut(),
                baseline,
                updates.len(),
                updates.as_ptr().cast(),
            )
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        Ok(id)
    }
}

// A `Tree` is a `git_tree`, never held by value; the owner is one pointer.
const _: () = assert!(
    size_of::<Tree<'static>>() == size_of::<ffi::git_tree>()
        && size_of::<TreeArc<'static>>() == size_of::<*mut ffi::git_tree>()
        && size_of::<Option<TreeArc<'static>>>() == size_of::<*mut ffi::git_tree>()
);

impl<'a, 'r> TreeRef<'a, 'r> {
    /// The tree's id, of its repository's id type: the id it was looked up by, or the
    /// full id a prefix found. Borrowed for as long as the handle: libgit2 keeps it in
    /// the tree, which writes it once, when it is parsed.
    ///
    /// Wraps: git_tree_id
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: the handle addresses a live tree for `'a`; libgit2 only reads it, and
        // returns the address of the id cached in its object (`&obj->cached.oid`).
        let id = unsafe { ffi::git_tree_id(self.as_ptr()) };
        // SAFETY: the id is part of the tree, so it is non-NULL and lives as long as the
        // tree, past `'a`; nothing writes it once the tree is cached or returned.
        unsafe { OidRef::from_ptr(id.cast_mut()) }.expect("a live tree has an id")
    }

    /// The repository the tree was looked up in, borrowed for `'r`, as the shared handle
    /// it was looked up through: what looking its subtrees up reaches.
    ///
    /// The result is that handle again, so it lets nothing happen that the tree's own
    /// lookups do not: it outlives the tree, but not the repository's borrow.
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::object::tree::Tree;
    /// # use libgit2::libgit2::oid::oid::{Oid, OidType};
    /// # use libgit2::libgit2::repository::repository::Repository;
    /// let repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let owner = tree.as_ref().owner();
    /// drop(tree);
    /// drop(repo);
    /// let _ = owner.path();
    /// ```
    ///
    /// Wraps: git_tree_owner
    pub fn owner(&self) -> RepositoryRef<'r> {
        // SAFETY: the handle addresses a live tree for `'a`, which libgit2 only reads.
        let repo = unsafe { ffi::git_tree_owner(self.as_ptr()) };
        // SAFETY: `object.repo`, stored uncounted and never cleared, is the repository
        // the tree was looked up in, through a `RepositoryRef<'r>` (`lookup*`, or an
        // object of it: `from_object`, a commit's tree), its `'r` only ever shrunk: it
        // is live and shared, not exclusively borrowed, for `'r`, on this thread, and the
        // result is the handle the tree was looked up through.
        unsafe { RepositoryRef::from_ptr(repo) }.expect("a tree always has a repository")
    }

    /// The number of entries the tree lists: one per name.
    ///
    /// Wraps: git_tree_entrycount
    pub fn len(&self) -> usize {
        // SAFETY: the handle addresses a live tree for `'a`, whose entry array libgit2
        // filled when parsing it and only reads the size of here.
        unsafe { ffi::git_tree_entrycount(self.as_ptr()) }
    }

    /// Whether the tree lists no entry: the empty tree.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The entry at `index`, in the tree's order (git's: by name, byte by byte, a tree's
    /// name compared as if it ended in `/`); `None` from [`len`](Self::len) on.
    ///
    /// The entry is the tree's own, its name pointing into the tree's raw data: it lives
    /// as long as this borrow of the tree, and nothing changes it meanwhile;
    /// [`to_tree_entry`](TreeEntryRef::to_tree_entry) copies it to keep.
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::object::tree::TreeArc;
    ///
    /// fn escape(tree: TreeArc<'_>) {
    ///     let handle = tree.as_ref();
    ///     let entry = handle.get(0);
    ///     drop(tree);
    ///     let _ = entry.map(|entry| entry.name());
    /// }
    /// ```
    ///
    /// Wraps: git_tree_entry_byindex
    pub fn get(&self, index: usize) -> Option<TreeEntryRef<'a>> {
        // SAFETY: the handle addresses a live tree for `'a`; libgit2 checks `index`
        // against the entry count and returns NULL past it, or the address of the entry
        // in the tree's array, which the tree keeps, unwritten, until it is freed after
        // `'a`.
        unsafe {
            let entry = ffi::git_tree_entry_byindex(self.as_ptr(), index);
            TreeEntryRef::from_ptr(entry.cast_mut())
        }
    }

    /// The tree's entry named `name`, the very bytes of the tree's own name for it, with
    /// no `/`; `None` when the tree has none. Lent as [`get`](Self::get) lends one.
    ///
    /// No tree holds a name longer than 65535 bytes, so a longer `name` names nothing;
    /// libgit2 then leaves a `GIT_ERROR_INVALID` error "tree entry path too long" as the
    /// thread's error. A miss sets no error otherwise. [`TreeEntry::bypath`] looks a
    /// path up through the subtrees.
    ///
    /// Wraps: git_tree_entry_byname
    pub fn get_by_name(&self, name: &CStr) -> Option<TreeEntryRef<'a>> {
        // SAFETY: the handle addresses a live tree for `'a`, only read; `name` is
        // NUL-terminated and read for the call only, within its length (a name longer
        // than 65535 bytes is compared on its first `len % 65536` bytes and its full
        // length, and matches nothing). The result is NULL or an entry of the tree's
        // array, live and unwritten until the tree is freed after `'a`.
        unsafe {
            let entry = ffi::git_tree_entry_byname(self.as_ptr(), name.as_ptr());
            TreeEntryRef::from_ptr(entry.cast_mut())
        }
    }

    /// The first entry, in the tree's order, naming the object `id`; `None` when no
    /// entry does. Lent as [`get`](Self::get) lends one. Every entry is compared, so
    /// this takes time in proportion to the tree's length.
    ///
    /// The ids compare as whole `git_oid`s: an id of another type than the tree's
    /// matches nothing, and neither does a SHA-1 id whose unused tail bytes are not zero
    /// (libgit2 zeroes them in every id it parses).
    ///
    /// Wraps: git_tree_entry_byid
    pub fn get_by_id(&self, id: OidRef<'_>) -> Option<TreeEntryRef<'a>> {
        // SAFETY: the handle addresses a live tree for `'a`, only read; `id` is a live,
        // whole `git_oid`, compared bytewise for the call only. The result is NULL or an
        // entry of the tree's array, live and unwritten until the tree is freed after
        // `'a`.
        unsafe {
            let entry = ffi::git_tree_entry_byid(self.as_ptr(), id.as_ptr());
            TreeEntryRef::from_ptr(entry.cast_mut())
        }
    }

    /// The tree's entries, in its order, each lent as [`get`](Self::get) lends one.
    pub fn iter(&self) -> TreeEntries<'a, 'r> {
        TreeEntries {
            tree: *self,
            range: 0..self.len(),
        }
    }

    /// Walks the tree, calling `callback` with each entry under the path of the tree
    /// holding it, and with those of every subtree it reaches, in the order `mode`
    /// names: `Ok(Continue(()))` when the walk went through, `Ok(Break(()))` when the
    /// closure stopped it ([`WalkAction::Stop`]). A panic of the closure stops the walk
    /// too, and resumes here once libgit2 has returned.
    ///
    /// libgit2 looks each subtree up in the tree's repository, through its cache, and
    /// releases it once walked, so a subtree's entries are lent for the call alone (see
    /// [`WalkCallback`]). A pre-order [`WalkAction::Skip`] keeps the walk out of the
    /// subtree the entry names, which is then not even looked up.
    ///
    /// Fails with the error of the subtree lookup that failed, after the calls before
    /// it: [`ErrorCode::NotFound`] when the object database lacks a subtree, or holds an
    /// object of another type under its id, and libgit2's error when one cannot be read
    /// or parsed, or memory runs out. When the closure stops the walk, libgit2 leaves
    /// "`git_tree_walk` callback returned -7" as the thread's error unless one is set
    /// already (by a libgit2 call the closure made, say).
    ///
    /// libgit2 calls back synchronously, on this thread, so the closure may use this
    /// tree and its repository's shared handles meanwhile, walking the tree again
    /// included, but not the repository's exclusive handle:
    ///
    /// ```compile_fail,E0502
    /// use core::ffi::CStr;
    /// use libgit2::libgit2::object::tree::{
    ///     Tree, TreeEntryRef, WalkAction, WalkCallback, WalkMode,
    /// };
    /// use libgit2::libgit2::oid::oid::{Oid, OidType};
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let mut repo = Repository::open(c".").unwrap();
    /// let id = Oid::zero(OidType::Sha1);
    /// let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
    /// let mut f = |_: &CStr, _: TreeEntryRef<'_>| {
    ///     let _ = repo.as_mut();
    ///     WalkAction::Continue
    /// };
    /// let mut callback = WalkCallback::new(&mut f);
    /// let _ = tree.as_ref().walk(WalkMode::PreOrder, &mut callback);
    /// ```
    ///
    /// ```
    /// # if cfg!(miri) { return; } // Miri cannot call libgit2.
    /// use core::ffi::CStr;
    /// use std::ops::ControlFlow;
    /// use libgit2::libgit2::object::tree::{
    ///     Tree, TreeEntryRef, WalkAction, WalkCallback, WalkMode,
    /// };
    /// use libgit2::libgit2::oid::oid::{Oid, OidType};
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::open(c"c/tests/resources/testrepo.git").unwrap();
    /// let id = Oid::from_string(c"ae90f12eea699729ed24555e40b9fd669da12a12", OidType::Sha1);
    /// let tree = Tree::lookup(repo.as_ref(), id.unwrap().as_ref()).unwrap();
    ///
    /// let mut paths = Vec::new();
    /// let mut collect = |root: &CStr, entry: TreeEntryRef<'_>| {
    ///     let mut path = root.to_bytes().to_vec();
    ///     path.extend_from_slice(entry.name().to_bytes());
    ///     paths.push(String::from_utf8(path).unwrap());
    ///     WalkAction::Continue
    /// };
    /// let mut callback = WalkCallback::new(&mut collect);
    /// let walked = tree.as_ref().walk(WalkMode::PreOrder, &mut callback);
    /// assert_eq!(walked.unwrap(), ControlFlow::Continue(()));
    /// drop(callback);
    /// assert!(paths.contains(&"ab/de/fgh/1.txt".to_owned()));
    /// ```
    ///
    /// Wraps: git_tree_walk
    pub fn walk(
        &self,
        mode: WalkMode,
        callback: &mut WalkCallback<'_>,
    ) -> Result<ControlFlow<()>, RepositoryError> {
        let tree = self.as_ptr();
        let rc = callback.walk_with(|f, payload| {
            // SAFETY: the handle addresses a live tree for `'a`, only read, whose
            // repository is live for `'r` and used on this thread only, as
            // `RepositoryRef` allows, where libgit2 looks the subtrees up and releases
            // each it walked; `mode` is a named mode; `f` and `payload` are the
            // callback's own trampoline and payload, which nothing else touches until
            // the call returns (`callback` is exclusively borrowed).
            unsafe { ffi::git_tree_walk(tree, mode.raw(), f, payload) }
        });
        match rc {
            0 => Ok(ControlFlow::Continue(())),
            ffi::GIT_EUSER if callback.stopped() => Ok(ControlFlow::Break(())),
            rc => Err(RepositoryError::from_libgit2(rc)),
        }
    }
}

impl<'a, 'r> IntoIterator for TreeRef<'a, 'r> {
    type Item = TreeEntryRef<'a>;
    type IntoIter = TreeEntries<'a, 'r>;

    fn into_iter(self) -> TreeEntries<'a, 'r> {
        self.iter()
    }
}

/// The entries of a tree, in its order, from either end: [`TreeRef::iter`]. Each is lent
/// for as long as the tree's handle, which the iterator holds.
#[derive(Clone, Debug)]
pub struct TreeEntries<'a, 'r> {
    tree: TreeRef<'a, 'r>,
    /// The indices not yet handed out; a tree's entries never change.
    range: Range<usize>,
}

impl<'a> Iterator for TreeEntries<'a, '_> {
    type Item = TreeEntryRef<'a>;

    fn next(&mut self) -> Option<TreeEntryRef<'a>> {
        self.tree.get(self.range.next()?)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }

    fn nth(&mut self, n: usize) -> Option<TreeEntryRef<'a>> {
        self.tree.get(self.range.nth(n)?)
    }
}

impl DoubleEndedIterator for TreeEntries<'_, '_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.tree.get(self.range.next_back()?)
    }
}

impl ExactSizeIterator for TreeEntries<'_, '_> {}

impl FusedIterator for TreeEntries<'_, '_> {}

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

    /// The number of entries the builder holds: one per distinct file name.
    ///
    /// Wraps: git_treebuilder_entrycount
    pub fn len(&self) -> usize {
        // SAFETY: the handle addresses a live builder that no one writes while it is
        // borrowed; `git_treebuilder_entrycount` takes it mutably but only reads the size
        // of its map.
        unsafe { ffi::git_treebuilder_entrycount(self.as_ptr().cast_mut()) }
    }

    /// Whether the builder holds no entry, so that writing it writes the empty tree.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The repository the builder was built for, where it looks objects up and writes
    /// its trees. libgit2 publishes no accessor: this reads `git_treebuilder`'s first
    /// member, `git_repository *repo` (`src/libgit2/tree.h`), which `git_treebuilder_new`
    /// sets and nothing changes afterwards.
    pub(crate) fn repository(&self) -> RepositoryRef<'r> {
        let repo = self.as_ptr().cast::<*mut ffi::git_repository>();
        // SAFETY: the handle addresses a live builder, whose private struct starts with
        // its `repo` pointer (`struct git_treebuilder` in `src/libgit2/tree.h`, the
        // revision this crate links), aligned as a pointer; libgit2 writes it only when
        // building the builder, so the read races with nothing.
        let repo = unsafe { repo.read() };
        // SAFETY: a `TreeBuilder<'r>` is only built from a `RepositoryRef<'r>`
        // (`TreeBuilder::new`, `TreeBuilder::from_tree`), whose repository libgit2 stores
        // uncounted (`git_treebuilder_new` refuses NULL): it is live, shared and used on
        // this thread for `'r`, so the handle returned is the one the builder was built
        // from.
        unsafe { RepositoryRef::from_ptr(repo) }.expect("a builder always has a repository")
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
        let (mut out, repo) = (ptr::null_mut(), repo.as_ptr().cast_mut());
        // SAFETY: `out` is a writable slot; `repo` a live repository on this thread, which
        // the builder only records; no source tree. The runtime is pinned, so a failure
        // is reported on live thread-local state.
        let rc = unsafe { ffi::git_treebuilder_new(&mut out, repo, ptr::null()) };
        // SAFETY: on success `git_treebuilder_new` writes a fresh builder for `repo` that
        // the caller owns alone, released by `git_treebuilder_free` as `TreeBuilderFree`
        // does; it keeps `repo`, which outlives `'r`. On failure it leaves `out` NULL.
        constructed(rc, unsafe { TreeBuilderBox::from_c(out) })
    }

    /// A builder for `repo` holding a copy of each entry of `source`: their names, ids
    /// and file modes as the tree spells them, unchecked. The builder keeps nothing of
    /// `source`, which may be dropped as soon as this returns.
    ///
    /// `source` may come from any repository. Its ids are copied as they are, so a tree
    /// of a repository of another object id type gives a builder that writes a malformed
    /// tree, as libgit2 does. An object of another type than tree has no [`TreeRef`]
    /// ([`TreeRef::from_object`] refuses it), so libgit2 never reads one as a tree.
    ///
    /// Fails, with libgit2's error, when it runs out of memory, having freed the entries
    /// it copied.
    ///
    /// Wraps: git_treebuilder_new
    pub fn from_tree(
        repo: RepositoryRef<'r>,
        source: TreeRef<'_, '_>,
    ) -> Result<TreeBuilderBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
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

/// The longest entry name a tree stores: its entries record a name's length in 16 bits.
const NAME_MAX: usize = u16::MAX as usize;

impl<'a, 'r> TreeBuilderRef<'a, 'r> {
    /// The builder's entry named `name`, the very bytes it was inserted or seeded under;
    /// `None` when the builder holds none.
    ///
    /// The entry is the builder's own: it lives as long as this borrow of the builder,
    /// which keeps it from being removed, replaced or cleared meanwhile;
    /// [`to_tree_entry`](TreeEntryRef::to_tree_entry) copies it to keep.
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::object::tree::TreeBuilderBox;
    ///
    /// fn stale(mut builder: TreeBuilderBox<'_>) {
    ///     let entry = builder.as_ref().get(c"README");
    ///     builder.as_mut().clear();
    ///     let _ = entry;
    /// }
    /// ```
    ///
    /// Wraps: git_treebuilder_get
    pub fn get(&self, name: &CStr) -> Option<TreeEntryRef<'a>> {
        // SAFETY: the shared handle's pointer addresses a live builder that nothing
        // writes during `'a`; `name` is NUL-terminated and read for the call only.
        // `git_treebuilder_get` only probes the builder's map, setting no error on a
        // miss, and returns NULL or the entry the map holds, which stays live and
        // unchanged until the builder next removes, replaces or clears its entries,
        // all of which need an exclusive borrow, excluded during `'a`.
        unsafe {
            let entry = ffi::git_treebuilder_get(self.as_ptr().cast_mut(), name.as_ptr());
            TreeEntryRef::from_ptr(entry.cast_mut())
        }
    }
}

impl<'a, 'r> TreeBuilderMut<'a, 'r> {
    /// Inserts an entry named `name` for the object `id` of file mode `mode`, or, when
    /// the builder already holds an entry of that name, gives it `id` and `mode` in its
    /// place.
    ///
    /// libgit2 checks the entry first, refusing it with a `GIT_ERROR_TREE` error naming
    /// `name`:
    ///
    /// - a `mode` that is not one a tree stores: [`FileMode::Unreadable`];
    /// - a `name` that cannot name an entry: empty, `.`, `..`, holding a `/`, or a
    ///   `.git` (and, as the repository's `core.protectNTFS` / `core.protectHFS` ask, the
    ///   names those file systems read as `.git`);
    /// - the zero id;
    /// - an object the repository's object database lacks, or holds with another type
    ///   than `mode` names (a tree for [`FileMode::Tree`], a blob for the others); the
    ///   database is loaded on first use. A [`FileMode::Commit`] entry names a
    ///   submodule's commit, never looked up; the library option
    ///   `GIT_OPT_ENABLE_STRICT_OBJECT_CREATION`, when turned off, skips the lookup for
    ///   every mode.
    ///
    /// An id of another type than the repository's is refused before libgit2 looks it
    /// up, with code `-1` and libgit2's `GIT_ERROR_INVALID` error "specified object ID
    /// type does not match repository object ID type": the object database's loose
    /// backend would format it into a path sized for the repository's type, out of
    /// bounds for a SHA-256 id in a SHA-1 repository (see the
    /// [`object` module](crate::libgit2::object::object#looking-objects-up)). The refusal
    /// comes after libgit2's own checks of the mode and the zero id, which run first,
    /// but before its check of the name. A [`FileMode::Commit`] entry's id is never
    /// looked up, and is stored as it is, so one of another type makes the builder
    /// write a malformed tree, as libgit2 does; so is any id libgit2 would not look up
    /// with strict object creation turned off, which the wrapper refuses all the same.
    ///
    /// Fails, with a `GIT_ERROR_INVALID` error "tree entry path too long", for a `name`
    /// of more than 65535 bytes, which a tree cannot store. libgit2 accepts one, sets
    /// that error and returns success, then writes a corrupt tree object with the name's
    /// length truncated: Rust refuses it before calling libgit2. Fails with libgit2's
    /// error when it runs out of memory.
    ///
    /// Wraps: git_treebuilder_insert
    pub fn insert(&mut self, name: &CStr, id: OidRef<'_>, mode: FileMode) -> Result<(), ErrorBuf> {
        self.insert_entry(name, id, mode).map(|_| ())
    }

    /// As [`insert`](Self::insert), and lends the entry inserted or updated. The entry is
    /// the builder's own: it lives as long as this borrow of the handle, which keeps the
    /// builder from changing meanwhile; [`to_tree_entry`](TreeEntryRef::to_tree_entry)
    /// copies it to keep.
    ///
    /// ```compile_fail,E0499
    /// use libgit2::libgit2::core::types::FileMode;
    /// use libgit2::libgit2::object::tree::TreeBuilderMut;
    /// use libgit2::libgit2::oid::oid::OidRef;
    ///
    /// fn stale(mut builder: TreeBuilderMut<'_, '_>, id: OidRef<'_>) {
    ///     let entry = builder.insert_entry(c"a", id, FileMode::Commit).unwrap();
    ///     builder.remove(c"a").unwrap();
    ///     let _ = entry;
    /// }
    /// ```
    ///
    /// Wraps: git_treebuilder_insert
    pub fn insert_entry(
        &mut self,
        name: &CStr,
        id: OidRef<'_>,
        mode: FileMode,
    ) -> Result<TreeEntryRef<'_>, ErrorBuf> {
        if name.count_bytes() > NAME_MAX {
            set(ErrorClass::Invalid, c"tree entry path too long");
            return Err(last());
        }
        if !matches!(mode, FileMode::Commit | FileMode::Unreadable) && !id.is_zero() {
            // libgit2 looks the object up: its id must be of the repository's type.
            id_fits(self.as_ref().repository(), id)?;
        }
        let mut out = ptr::null();
        // SAFETY: `out` is a writable slot, set only on success; the exclusive handle's
        // pointer addresses a live builder that nothing else reaches meanwhile, whose
        // repository is live for `'r` and used on this thread only, as `RepositoryRef`
        // allows, where libgit2 looks the object up; `name` is NUL-terminated and `id` a
        // live id, both copied; `mode` is a named file mode. A name of at most 65535
        // bytes is stored whole. An id libgit2 looks up (any mode but a commit's or an
        // unreadable one, a non-zero id) is of the repository's type, checked above.
        let rc = unsafe {
            ffi::git_treebuilder_insert(
                &mut out,
                self.as_mut_ptr(),
                name.as_ptr(),
                id.as_ptr(),
                mode.raw(),
            )
        };
        if rc < 0 {
            return Err(last());
        }
        // SAFETY: on success `out` is the builder's entry for `name`, live and unchanged
        // until the builder next removes, replaces or clears its entries, which the
        // borrow of this handle excludes for the result's lifetime.
        let entry = unsafe { TreeEntryRef::from_ptr(out.cast_mut()) };
        Ok(entry.expect("an insertion succeeded without an entry"))
    }

    /// Removes the entry named `name`, freeing it.
    ///
    /// Fails, with a `GIT_ERROR_TREE` error "failed to remove entry: file isn't in the
    /// tree - `name`", when the builder holds no entry of that name.
    ///
    /// Wraps: git_treebuilder_remove
    pub fn remove(&mut self, name: &CStr) -> Result<(), ErrorBuf> {
        // SAFETY: the exclusive handle's pointer addresses a live builder that nothing
        // else reaches meanwhile, and no handle to its entries outlives that borrow;
        // `name` is NUL-terminated and read for the call only. libgit2 frees the entry
        // it removes, without reaching the repository.
        let rc = unsafe { ffi::git_treebuilder_remove(self.as_mut_ptr(), name.as_ptr()) };
        if rc < 0 { Err(last()) } else { Ok(()) }
    }

    /// Writes the builder's entries, sorted as git sorts a tree's, as a tree object into
    /// its repository's object database, and returns the tree's id, of the repository's
    /// type. The builder keeps its entries, and stays usable; writing it again writes
    /// the same tree.
    ///
    /// The database is loaded on first use. The entries are not checked again: an entry
    /// inserted without the lookup, or seeded from a tree, may name an object the
    /// database lacks.
    ///
    /// Fails with the code and error of libgit2, or of the database's backends: when
    /// the database cannot be loaded (always for a repository without a git directory,
    /// [`Repository::new`](crate::libgit2::repository::repository::Repository::new),
    /// unless one was set), the object cannot be written, or memory runs out.
    ///
    /// When memory runs out while the builder's write buffer grows, libgit2 marks that
    /// buffer as out of memory for good: neither a later write nor
    /// [`clear`](Self::clear) resets it, so every later write of this builder fails with
    /// `-1` and whatever error the thread holds then. The builder's entries stay usable;
    /// a new builder writes them. This is libgit2's defect, which the wrapper cannot
    /// reach into the builder to undo.
    ///
    /// Wraps: git_treebuilder_write
    pub fn write(&mut self) -> Result<Oid, RepositoryError> {
        let mut id = Oid::zeroed();
        let out = id.as_mut().as_mut_ptr();
        // SAFETY: `out` addresses `id`, written only on success; the exclusive handle's
        // pointer addresses a live builder that nothing else reaches meanwhile, whose
        // write buffer libgit2 refills; its repository is live for `'r` and used on this
        // thread only, as `RepositoryRef` allows, where libgit2 loads and writes the
        // object database.
        let rc = unsafe { ffi::git_treebuilder_write(out, self.as_mut_ptr()) };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        Ok(id)
    }

    /// Calls `filter` with each entry the builder holds, once, in the builder's
    /// (unspecified) order, and removes and frees each entry it returns `true` for,
    /// right after its call. Every entry is visited: libgit2 cannot stop early.
    ///
    /// A panic of the closure keeps the entry it panicked on and every later one, which
    /// the closure is not called for again, and resumes here once libgit2 has returned
    /// (see [`TreeBuilderFilter`]). The builder never reaches its repository meanwhile.
    ///
    /// The builder is borrowed exclusively for the call, so the closure reaches neither
    /// it nor an entry it lent before:
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::object::tree::{TreeBuilderBox, TreeBuilderFilter, TreeEntryRef};
    ///
    /// fn reenter(mut builder: TreeBuilderBox<'_>) {
    ///     let mut f = |_: TreeEntryRef<'_>| builder.as_ref().len() > 1;
    ///     let mut filter = TreeBuilderFilter::new(&mut f);
    ///     builder.as_mut().remove_if(&mut filter);
    /// }
    /// ```
    ///
    /// ```
    /// # if cfg!(miri) { return; } // Miri cannot call libgit2.
    /// use libgit2::libgit2::core::types::FileMode;
    /// use libgit2::libgit2::object::tree::{TreeBuilder, TreeBuilderFilter, TreeEntryRef};
    /// use libgit2::libgit2::oid::oid::{Oid, OidType};
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::new().unwrap();
    /// let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
    /// let id = Oid::from_string(c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750", OidType::Sha1).unwrap();
    /// for name in [c"keep", c"drop.tmp", c"also.tmp"] {
    ///     builder.as_mut().insert(name, id.as_ref(), FileMode::Commit).unwrap();
    /// }
    /// let mut temporary = |entry: TreeEntryRef<'_>| entry.name().to_bytes().ends_with(b".tmp");
    /// builder.as_mut().remove_if(&mut TreeBuilderFilter::new(&mut temporary));
    /// assert_eq!(builder.as_ref().len(), 1);
    /// assert!(builder.as_ref().get(c"keep").is_some());
    /// ```
    ///
    /// Wraps: git_treebuilder_filter
    pub fn remove_if(&mut self, filter: &mut TreeBuilderFilter<'_>) {
        let builder = self.as_mut_ptr();
        let rc = filter.filter_with(|f, payload| {
            // SAFETY: the exclusive handle's pointer addresses a live builder that
            // nothing else reaches meanwhile, and no handle to its entries outlives that
            // borrow; libgit2 frees the entries it removes, without reaching the
            // repository. `f` and `payload` are the filter's own trampoline and payload,
            // which nothing else touches until the call returns (`filter` is exclusively
            // borrowed).
            unsafe { ffi::git_treebuilder_filter(builder, f, payload) }
        });
        // It fails only on a NULL builder or filter, neither of which reaches it.
        debug_assert_eq!(rc, 0);
    }
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
    /// The entry at `path` under `tree`, as a copy of its own: each `/`-separated
    /// component of `path` is looked up in turn, the subtrees it names read from
    /// `tree`'s repository, and the last component's entry is copied out, as
    /// `git_tree_entry_dup` copies one. The result holds nothing of `tree` or its
    /// repository, and outlives both.
    ///
    /// The components are matched as the tree spells them: `.` and `..` name nothing,
    /// a trailing `/` is accepted only after a tree, and an empty component (an empty
    /// `path`, or a leading or doubled `/`) is an invalid path.
    ///
    /// Fails with [`ErrorCode::NotFound`]
    /// (class `GIT_ERROR_TREE`) for an invalid path, a component the tree lacks, or a
    /// `/` after an entry that is not a tree; and with libgit2's error when a subtree
    /// cannot be read, or the copy cannot be allocated. A commit or tag is peeled to its
    /// tree by [`ObjectRef::lookup_bypath`].
    ///
    /// Wraps: git_tree_entry_bypath
    pub fn bypath(tree: TreeRef<'_, '_>, path: &CStr) -> Result<TreeEntryBox, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `tree` is a live tree,
        // unwritten while the handle lives, whose repository is live for the handle's
        // `'r` and used on this thread only, as `RepositoryRef` allows, where libgit2
        // looks the subtrees up; `path` is NUL-terminated and read for the call only.
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

impl<'a> TreeEntryRef<'a> {
    /// The entry's file name, without any directory: libgit2 stores it inside the
    /// entry, or in the tree's data the entry points into, so it lives as long as the
    /// entry's borrow.
    ///
    /// Wraps: git_tree_entry_name
    pub fn name(&self) -> &'a CStr {
        // SAFETY: the handle addresses a live entry, unchanged while it is borrowed for
        // `'a`; its name is a NUL-terminated string that its owner (the entry's own
        // block, or the tree or builder it belongs to) keeps for at least as long.
        unsafe { CStr::from_ptr(ffi::git_tree_entry_name(self.as_ptr())) }
    }

    /// The id of the object the entry names, held inside the entry and lent for as long
    /// as its borrow. It is of the object id type of the tree the entry was read from.
    ///
    /// Wraps: git_tree_entry_id
    pub fn id(&self) -> OidRef<'a> {
        // SAFETY: the handle addresses a live entry for `'a`; the id is a field of it,
        // which no one writes while the entry is borrowed shared.
        let id = unsafe { OidRef::from_ptr(ffi::git_tree_entry_id(self.as_ptr()).cast_mut()) };
        id.expect("an entry's id is a field of it")
    }

    /// The entry's file mode, normalised as git normalises one: a mode with a
    /// directory's type bits is [`FileMode::Tree`], one with any executable bit
    /// [`FileMode::BlobExecutable`], a gitlink's [`FileMode::Commit`], a symbolic link's
    /// [`FileMode::Link`], and any other [`FileMode::Blob`]. It is never
    /// [`FileMode::Unreadable`]; [`filemode_raw`](Self::filemode_raw) has the mode as
    /// stored.
    ///
    /// Wraps: git_tree_entry_filemode
    pub fn filemode(&self) -> FileMode {
        // SAFETY: the handle addresses a live entry for `'a`, whose mode libgit2 reads.
        let raw = unsafe { ffi::git_tree_entry_filemode(self.as_ptr()) };
        FileMode::from_raw(raw).expect("a normalised file mode is always a named one")
    }

    /// The entry's file mode as the tree stores it, whatever 16-bit value its object
    /// spells (old git's group-writable `0o100664`, say); [`FileMode::try_from`] names
    /// it when it is one of the header's modes.
    ///
    /// Wraps: git_tree_entry_filemode_raw
    pub fn filemode_raw(&self) -> ffi::git_filemode_t {
        // SAFETY: the handle addresses a live entry for `'a`, whose mode libgit2 reads.
        unsafe { ffi::git_tree_entry_filemode_raw(self.as_ptr()) }
    }

    /// The type of the object the entry names, as its stored file mode tells it: a
    /// gitlink's is [`ObjectType::Commit`], a directory's [`ObjectType::Tree`], any
    /// other [`ObjectType::Blob`]. The object itself is not read.
    ///
    /// Wraps: git_tree_entry_type
    pub fn kind(&self) -> ObjectType {
        // SAFETY: the handle addresses a live entry for `'a`, whose mode libgit2 reads.
        let raw = unsafe { ffi::git_tree_entry_type(self.as_ptr()) };
        ObjectType::from_raw(raw).expect("an entry's type is always a named one")
    }

    /// How this entry sorts against `other` in a tree: by name, byte by byte, where a
    /// tree's name compares as if it ended in `/` (so the tree `a` sorts after the file
    /// `a.c`, and the file `a` before both). Entries of equal names and kinds compare
    /// equal whatever their ids.
    ///
    /// Wraps: git_tree_entry_cmp
    pub fn cmp_in_tree(&self, other: TreeEntryRef<'_>) -> Ordering {
        // SAFETY: both handles address live entries, unchanged while borrowed; libgit2
        // reads their names, name lengths and modes.
        let rc = unsafe { ffi::git_tree_entry_cmp(self.as_ptr(), other.as_ptr()) };
        rc.cmp(&0)
    }

    /// The object the entry names, looked up in `repo` by the entry's id, of whatever
    /// type it has: one more counted reference of the repository's cache, or a fresh
    /// parse, as [`Object::lookup`](crate::libgit2::object::object::Object::lookup)
    /// gives. The entry need not come from `repo`.
    ///
    /// Fails as [`Object::lookup`](crate::libgit2::object::object::Object::lookup)
    /// does: with [`ErrorCode::NotFound`] when the repository has no such object (a
    /// submodule's commit, say), and with libgit2's error when it cannot be read. An
    /// entry whose id is of another type than `repo`'s is refused, with code `-1` and a
    /// `GIT_ERROR_INVALID` error "specified object ID type does not match repository
    /// object ID type", where libgit2 would format the id out of bounds.
    ///
    /// Wraps: git_tree_entry_to_object
    pub fn to_object<'r>(&self, repo: RepositoryRef<'r>) -> Result<ObjectArc<'r>, RepositoryError> {
        id_fits(repo, self.id())?;
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `repo` is a live
        // repository on this thread, whose odb libgit2 loads and whose cache it fills
        // through this shared handle, as `RepositoryRef` allows; the entry is live for
        // `'a` and only its id, of `repo`'s type (checked above), is read.
        let rc = unsafe {
            ffi::git_tree_entry_to_object(&mut out, repo.as_ptr().cast_mut(), self.as_ptr())
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: on success `out` holds one counted reference to a live object of
        // `repo`, borrowed for `'r`, given to the result and released by
        // `git_object_free`.
        let object = unsafe { ObjectArc::from_c(out) };
        Ok(object.expect("a lookup succeeded without an object"))
    }

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

/// The closure a [`TreeBuilderFilter`] calls: one entry, lent for the call; `true`
/// removes it.
type FilterFn<'f> = dyn FnMut(TreeEntryRef<'_>) -> bool + 'f;

/// A filter over a tree builder's entries (`git_treebuilder_filter_cb`):
/// `git_treebuilder_filter` calls it once with each entry the builder holds, in the
/// builder's (unspecified) order, on the calling thread, and removes and frees each
/// entry for which it returns non-zero.
///
/// Built over a closure with [`new`](Self::new). Its `extern "C"` trampoline lends the
/// closure each entry as a [`TreeEntryRef`] valid for the call only (the builder frees
/// a removed entry right after), and returns `true` as 1 and `false` as 0. A panic in
/// the closure does not cross into C: libgit2 cannot stop the filtering, so the
/// trampoline keeps that entry and every later one without calling the closure again,
/// and the panic resumes once control is back in Rust, from [`call`](Self::call) or
/// from [`TreeBuilderMut::remove_if`].
///
/// ```
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// use libgit2::libgit2::object::tree::{Tree, TreeBuilderFilter, TreeEntry};
/// use libgit2::libgit2::oid::oid::{Oid, OidType};
/// use libgit2::libgit2::repository::repository::Repository;
///
/// let repo = Repository::open(c"c/tests/resources/testrepo.git").unwrap();
/// let id = Oid::from_string(c"944c0f6e4dfa41595e6eb3ceecdb14f50fe18162", OidType::Sha1);
/// let tree = Tree::lookup(repo.as_ref(), id.unwrap().as_ref()).unwrap();
/// let readme = TreeEntry::bypath(tree.as_ref(), c"README").unwrap();
///
/// let mut seen = Vec::new();
/// let mut readme_only = |entry: libgit2::libgit2::object::tree::TreeEntryRef<'_>| {
///     seen.push(entry.name().to_owned());
///     entry.name() == c"README"
/// };
/// let mut filter = TreeBuilderFilter::new(&mut readme_only);
/// assert!(filter.call(readme.as_ref()));
/// drop(filter);
/// assert_eq!(seen, [c"README".to_owned()]);
/// ```
///
/// The entry cannot outlive the call:
///
/// ```compile_fail,E0521
/// use libgit2::libgit2::object::tree::{TreeBuilderFilter, TreeEntryRef};
///
/// fn keep<'k>(kept: &mut Option<TreeEntryRef<'k>>) {
///     let mut f = |entry: TreeEntryRef<'_>| {
///         *kept = Some(entry);
///         false
///     };
///     let _ = TreeBuilderFilter::new(&mut f);
/// }
/// ```
///
/// It borrows its closure exclusively and stays on its thread: libgit2 calls it
/// synchronously, and the closure need not be `Send`.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::object::tree::{TreeBuilderFilter, TreeEntryRef};
///
/// fn send<T: Send>(_: T) {}
/// let mut f = |_: TreeEntryRef<'_>| false;
/// send(TreeBuilderFilter::new(&mut f));
/// ```
///
/// Wraps: git_treebuilder_filter_cb
/// Handwritten
pub struct TreeBuilderFilter<'f> {
    f: &'f mut FilterFn<'f>,
    /// A panic the closure raised inside a C filter, resumed once the filter returns.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> TreeBuilderFilter<'f> {
    /// The filter that calls `f` with each entry, removing those it returns `true` for.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(TreeEntryRef<'_>) -> bool + 'f,
    {
        Self { f, panic: None }
    }

    /// Calls the filter with `entry`, through the trampoline libgit2 calls, and returns
    /// whether libgit2 would remove it. A panic of the closure resumes here.
    pub fn call(&mut self, entry: TreeEntryRef<'_>) -> bool {
        let rc = self.filter_with(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            // SAFETY: `payload` is this filter, which nothing else uses during the
            // call, and `entry` a live entry lent for the call: the trampoline's
            // contract.
            unsafe { callback(entry.as_ptr(), payload) }
        });
        rc != 0
    }

    /// Runs `filter` with the C callback and its payload, as the wrapper of a routine
    /// that calls the callback (`git_treebuilder_filter`) hands them to it, then resumes
    /// a panic the closure raised meanwhile. The payload is this filter, which nothing
    /// else touches until `filter` returns.
    pub(crate) fn filter_with<R>(
        &mut self,
        filter: impl FnOnce(ffi::git_treebuilder_filter_cb, *mut c_void) -> R,
    ) -> R {
        let result = filter(Some(filter_with), ptr::from_mut(self).cast());
        if let Some(panic) = self.panic.take() {
            panic::resume_unwind(panic);
        }
        result
    }
}

impl fmt::Debug for TreeBuilderFilter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeBuilderFilter")
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_treebuilder_filter_cb` over a [`TreeBuilderFilter`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`TreeBuilderFilter`] that nothing else uses during
/// the call, and `entry` be NULL or address a `git_tree_entry` live and unchanged for
/// the call.
unsafe extern "C" fn filter_with(entry: *const ffi::git_tree_entry, payload: *mut c_void) -> c_int {
    // SAFETY: per this function's contract, `payload` is a live filter borrowed for the
    // call alone.
    let filter = unsafe { &mut *payload.cast::<TreeBuilderFilter<'_>>() };
    if filter.panic.is_some() {
        return 0;
    }
    // SAFETY: per this function's contract, a non-NULL `entry` is live and unchanged for
    // the call; the handle given to the closure is valid for its call only.
    let Some(entry) = (unsafe { TreeEntryRef::from_ptr(entry.cast_mut()) }) else {
        // libgit2 never passes NULL; keep whatever it meant rather than invent an entry.
        return 0;
    };
    match panic::catch_unwind(AssertUnwindSafe(|| (filter.f)(entry))) {
        Ok(remove) => c_int::from(remove),
        Err(panic) => {
            filter.panic = Some(panic);
            0
        }
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

    /// `git_tree`'s owner, handles, lifecycle policy and conversions.
    mod tree_object {
        use crate::ffi;
        use crate::libgit2::core::types::ObjectType;
        use crate::libgit2::object::object::{Object, ObjectArc, ObjectRef};
        use crate::libgit2::object::tree::*;
        use crate::libgit2::oid::oid::{Oid, OidRef, OidType};
        use crate::libgit2::repository::repository::{Repository, RepositoryBox};
        use crate::libgit2::util::errors::{ErrorClass, ErrorCode};
        use core::ffi::CStr;
        use core::mem::size_of;
        use ffibox::{CDrop, CRefClone};

        /// `testrepo.git`: commit `763d71a`, whose tree `ae90f12` holds `README`.
        const TESTREPO: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/c/tests/resources/testrepo.git\0"
        );
        const TREE: &CStr = c"ae90f12eea699729ed24555e40b9fd669da12a12";
        const COMMIT: &CStr = c"763d71aadf09a7951596c9746c024e7eece7c7af";
        const README: &CStr = c"1385f264afb75a56a5bec74243be9b367ba4ca08";

        fn open() -> RepositoryBox {
            assert!(runtime_pinned());
            Repository::open(CStr::from_bytes_with_nul(TESTREPO.as_bytes()).unwrap()).unwrap()
        }

        fn sha1(hex: &CStr) -> Oid {
            Oid::from_string(hex, OidType::Sha1).unwrap()
        }

        /// C's id and type of a live object.
        fn view(object: ObjectRef<'_, '_>) -> (String, ffi::git_object_t) {
            let p = object.as_ptr();
            // SAFETY: `p` is a live object the handle borrows; its id lives as long as it
            // and is formatted at once.
            unsafe {
                let id = OidRef::from_ptr(ffi::git_object_id(p).cast_mut()).unwrap();
                (id.to_hex_string().unwrap(), ffi::git_object_type(p))
            }
        }

        fn tree_view() -> (String, ffi::git_object_t) {
            (TREE.to_str().unwrap().to_owned(), ffi::GIT_OBJECT_TREE)
        }

        #[test]
        fn layout_matches_c() {
            assert_eq!(size_of::<Tree<'_>>(), size_of::<ffi::git_tree>());
            assert_eq!(size_of::<TreeArc<'_>>(), size_of::<*mut ffi::git_tree>());
            assert_eq!(
                size_of::<Option<TreeArc<'_>>>(),
                size_of::<*mut ffi::git_tree>()
            );
            assert_eq!(
                size_of::<TreeRef<'_, '_>>(),
                size_of::<*const ffi::git_tree>()
            );
            assert_eq!(
                size_of::<TreeMut<'_, '_>>(),
                size_of::<*const ffi::git_tree>()
            );
            assert_eq!(size_of::<TreeFree>(), 0);
        }

        /// The handles' seams, and the view as an object, over a pointer nothing reads,
        /// so Miri runs them: `git_tree` is opaque, of size zero, so a dangling aligned
        /// pointer addresses all of it.
        #[test]
        fn handles_without_c() {
            // SAFETY: NULL is refused without being read.
            assert!(unsafe { TreeRef::from_ptr(ptr::null()) }.is_none());
            // SAFETY: as above.
            assert!(unsafe { TreeMut::from_ptr(ptr::null_mut()) }.is_none());
            let p = NonNull::<ffi::git_tree>::dangling().as_ptr();
            // SAFETY: a zero-sized object at an aligned pointer, never read or written,
            // and reached by this handle alone.
            let mut m = unsafe { TreeMut::from_ptr(p) }.unwrap();
            assert_eq!(m.as_mut_ptr(), p);
            assert_eq!(m.as_mut().as_mut_ptr(), p);
            let r = m.as_ref();
            assert_eq!(r.as_ptr(), p.cast_const());
            assert_eq!(r.as_object().as_ptr(), p.cast_const().cast());
            assert_eq!(format!("{r:?}"), "Tree { .. }");
            assert_eq!(format!("{m:?}"), "Tree { .. }");
        }

        /// The policy driven by hand: the up-ref counts one more reference on the same
        /// pointer, which an adopted owner and the free each settle; `to_tree` takes one
        /// through a borrowed handle, which outlives the owner it came from.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn policy_counts_references_on_one_tree() {
            let repo = open();
            let tree = Tree::lookup(repo.as_ref(), sha1(TREE).as_ref()).unwrap();
            let p = NonNull::new(tree.as_ptr()).unwrap();
            // SAFETY: `tree` holds a reference to the live tree at `p`.
            assert!(unsafe { TreeFree.c_up_ref(p) });
            // SAFETY: the reference just counted, given to the owner.
            let adopted = unsafe { TreeArc::from_raw(p.as_ptr()) }.unwrap();
            assert!(TreeArc::ptr_eq(&adopted, &tree));
            let raw = adopted.into_c();
            // SAFETY: `raw` is the reference `into_c` gave up, released once here.
            unsafe { TreeFree.c_drop(NonNull::new(raw).unwrap().cast()) };

            // SAFETY: `tree` keeps the tree alive while the handle lives.
            let borrowed = unsafe { TreeRef::from_ptr(tree.as_c_ptr()) }.unwrap();
            let copy = borrowed.to_tree();
            let clone = copy.clone();
            assert!(TreeArc::ptr_eq(&copy, &tree) && TreeArc::ptr_eq(&clone, &tree));
            drop(tree);
            drop(copy);
            assert_eq!(view(clone.as_ref().as_object()), tree_view());
        }

        /// A tree lookup is the object lookup of type tree: the same cached object, and
        /// another type is not found; an id of another type than the repository's is
        /// refused before C, at every length.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn lookups_find_trees_only() {
            let repo = open();
            let r = repo.as_ref();
            let tree = Tree::lookup(r, sha1(TREE).as_ref()).unwrap();
            let short = Tree::lookup_prefix(r, sha1(TREE).as_ref(), 7).unwrap();
            let object = Object::lookup(r, sha1(TREE).as_ref(), Some(ObjectType::Tree)).unwrap();
            assert!(TreeArc::ptr_eq(&tree, &short));
            assert_eq!(tree.as_c_ptr().cast(), object.as_c_ptr());
            for hex in [COMMIT, README] {
                for e in [
                    Tree::lookup(r, sha1(hex).as_ref()).unwrap_err(),
                    Tree::lookup_prefix(r, sha1(hex).as_ref(), 10).unwrap_err(),
                ] {
                    assert_eq!(e.code(), Some(ErrorCode::NotFound));
                    assert_eq!(e.class(), Some(ErrorClass::Invalid));
                }
            }
            let e = Tree::lookup_prefix(r, sha1(TREE).as_ref(), 3).unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::Ambiguous));

            let mut wide = Oid::from_raw(&sha1(TREE).as_ref().id(), OidType::Sha256).unwrap();
            for kind in [OidType::Sha256.raw() as u8, 0, 9, u8::MAX] {
                wide.as_mut().set_raw_kind(kind);
                let mut refused = vec![Tree::lookup(r, wide.as_ref()).unwrap_err()];
                for len in [0, 4, 7, 40, 41, 64, usize::MAX] {
                    refused.push(Tree::lookup_prefix(r, wide.as_ref(), len).unwrap_err());
                }
                for e in refused {
                    assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
                    assert_eq!(
                        e.message(),
                        c"specified object ID type does not match repository object ID type"
                    );
                }
            }
        }

        /// Owners and handles convert between tree and object, keeping one count per
        /// owner; an object of another type stays an object, untouched.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn conversions_check_the_type_and_keep_the_count() {
            let repo = open();
            let r = repo.as_ref();
            let lookup = |hex: &CStr| Object::lookup(r, sha1(hex).as_ref(), None).unwrap();

            for (hex, kind) in [
                (COMMIT, ffi::GIT_OBJECT_COMMIT),
                (README, ffi::GIT_OBJECT_BLOB),
            ] {
                let other = lookup(hex);
                assert!(TreeRef::from_object(other.as_ref()).is_none());
                let other = Tree::from_object(other).unwrap_err();
                assert_eq!(view(other.as_ref()).1, kind);
            }

            let object = lookup(TREE);
            let viewed = TreeRef::from_object(object.as_ref()).unwrap();
            assert_eq!(viewed.as_ptr().cast(), object.as_ref().as_ptr());
            let from_view = viewed.to_tree();
            let tree = Tree::from_object(object.clone()).unwrap();
            assert!(TreeArc::ptr_eq(&tree, &from_view));
            let back: ObjectArc<'_> = Tree::into_object(tree);
            assert!(ObjectArc::ptr_eq(&back, &object));
            drop((object, back));
            assert_eq!(view(from_view.as_ref().as_object()), tree_view());
        }

        /// The cache's reference is never visible, so an owner never proves itself
        /// sole: no exclusive handle, no box.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn an_owner_is_never_sole() {
            let repo = open();
            let mut tree = Tree::lookup(repo.as_ref(), sha1(TREE).as_ref()).unwrap();
            assert!(tree.get_mut().is_none());
            let tree = tree.try_into_box().unwrap_err();
            assert_eq!(format!("{:?}", tree.as_ref()), "Tree { .. }");
        }
    }

    /// A builder's length is its number of entries: a tree's entries once seeded from
    /// it, none for a new builder or once cleared; it agrees with C's count.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn builder_length_counts_entries() {
        use crate::libgit2::oid::oid::{Oid, OidType};
        use crate::libgit2::repository::repository::Repository;

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/c/tests/resources/testrepo.git\0"
        );
        let repo = Repository::open(CStr::from_bytes_with_nul(path.as_bytes()).unwrap()).unwrap();
        let empty = TreeBuilder::new(repo.as_ref()).unwrap();
        assert_eq!(empty.as_ref().len(), 0);
        assert!(empty.as_ref().is_empty());

        let id = Oid::from_string(c"ae90f12eea699729ed24555e40b9fd669da12a12", OidType::Sha1);
        let tree = Tree::lookup(repo.as_ref(), id.unwrap().as_ref()).unwrap();
        let mut builder = TreeBuilder::from_tree(repo.as_ref(), tree.as_ref()).unwrap();
        assert_eq!(builder.as_ref().len(), 4);
        assert!(!builder.as_mut().as_ref().is_empty());
        // SAFETY: the box's live builder, only read.
        let c_count = unsafe { ffi::git_treebuilder_entrycount(builder.as_c_ptr()) };
        assert_eq!(c_count, 4);
        builder.as_mut().clear();
        assert_eq!(builder.as_ref().len(), 0);
        assert!(builder.as_ref().is_empty());
    }

    /// `git_tree_entry`'s owner, handles, getters and lifecycle policy, and the
    /// builder filter's callback.
    mod tree_entry {
        use crate::libgit2::object::tree::*;
        use crate::libgit2::oid::oid::{Oid, OidRef, OidType};
        use crate::libgit2::repository::repository::{Repository, RepositoryBox};
        use crate::libgit2::util::errors::{ErrorClass, ErrorCode};
        use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
        use core::ffi::{CStr, c_char, c_int, c_void};
        use core::mem::size_of;
        use ffibox::CDrop;
        use std::process::Command;
        use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::SeqCst};

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// `testrepo.git`; commit `763d71a`'s tree holds `README` and the nested
        /// `ab/de/fgh/1.txt`.
        const TESTREPO: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/c/tests/resources/testrepo.git\0"
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

        /// As [`isolated`], with LeakSanitizer off in the child, for bodies that drive
        /// libgit2 down allocation failures on which it leaks.
        fn isolated_tolerating_leaks(test: &str, body: fn()) {
            let name = test.split_once("::").map_or(test, |(_, rest)| rest);
            if std::env::var(CHILD).as_deref() == Ok(name) {
                body();
                return;
            }
            let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
            let separator = if inherited.is_empty() { "" } else { ":" };
            let out = Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, name)
                .env(
                    "ASAN_OPTIONS",
                    format!("{inherited}{separator}detect_leaks=0"),
                )
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "child `{name}` failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        /// Whether [`Refusing`] refuses requests: every one, after the first [`ALLOW`].
        static REFUSE: AtomicBool = AtomicBool::new(false);
        /// How many requests [`Refusing`] still grants while [`REFUSE`] is set.
        static ALLOW: AtomicIsize = AtomicIsize::new(0);

        unsafe extern "C" {
            fn malloc(n: usize) -> *mut c_void;
            fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
            fn free(p: *mut c_void);
        }

        /// libc's allocator, refusing every request while [`REFUSE`] is set.
        struct Refusing;

        impl Refusing {
            fn grant() -> bool {
                !REFUSE.load(SeqCst) || ALLOW.fetch_sub(1, SeqCst) > 0
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

        /// `testrepo.git`, opened read-only.
        fn open() -> RepositoryBox {
            assert!(runtime_pinned());
            Repository::open(CStr::from_bytes_with_nul(TESTREPO.as_bytes()).unwrap()).unwrap()
        }

        /// The tree `TREE` of `repo`.
        fn lookup_tree(repo: &RepositoryBox) -> TreeArc<'_> {
            let id = Oid::from_string(TREE, OidType::Sha1).unwrap();
            Tree::lookup(repo.as_ref(), id.as_ref()).unwrap()
        }

        /// The tree's own entry `name`, borrowed for as long as the tree is.
        fn entry<'a>(tree: TreeRef<'a, '_>, name: &CStr) -> TreeEntryRef<'a> {
            tree.get_by_name(name).unwrap()
        }

        /// `git_tree_entry_bypath(path)`, adopted through `from_c`.
        fn adopt_bypath(tree: TreeRef<'_, '_>, path: &CStr) -> TreeEntryBox {
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot, the live tree and a NUL-terminated path.
            let rc = unsafe { ffi::git_tree_entry_bypath(&mut out, tree.as_ptr(), path.as_ptr()) };
            assert_eq!(rc, 0);
            // SAFETY: a fresh entry the caller owns, freed by `git_tree_entry_free`.
            unsafe { TreeEntryBox::from_c(out) }.unwrap()
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
            let repo = open();
            let tree = lookup_tree(&repo);
            let borrowed = entry(tree.as_ref(), c"README");
            assert_eq!(view(borrowed), expected("README", README));
            let copy = borrowed.to_tree_entry().unwrap();
            let nested = adopt_bypath(tree.as_ref(), c"ab/de/fgh/1.txt");
            let nested_copy = nested.as_ref().to_tree_entry().unwrap();
            drop(tree);
            drop(repo);

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
                    let repo = open();
                    let tree = lookup_tree(&repo);
                    let owned = adopt_bypath(tree.as_ref(), c"README");
                    REFUSE.store(true, SeqCst);
                    let borrowed = entry(tree.as_ref(), c"README").to_tree_entry();
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

        /// `create_updated` with every allocation refused in turn, from the first on: each
        /// refusal is reported as an error (out of memory, mostly), never a crash; with
        /// every allocation granted the same tree is written as without the allocator.
        ///
        /// libgit2 leaks on some of these failures: where `git_tree_create_updated` runs
        /// out of memory for a stack level or a component's name, `GIT_ERROR_CHECK_ALLOC`
        /// returns past its cleanup, keeping every builder, tree and name on its stack.
        /// The child runs with LeakSanitizer off.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn create_updated_reports_allocation_failures() {
            isolated_tolerating_leaks(
                concat!(
                    module_path!(),
                    "::create_updated_reports_allocation_failures"
                ),
                || {
                    install_refusing();
                    let path = std::env::temp_dir().join(format!(
                        "crustify-tree-unit-{}-updated-oom",
                        std::process::id()
                    ));
                    let _ = std::fs::remove_dir_all(&path);
                    let c_path = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
                    let repo = Repository::init(&c_path, true).unwrap();
                    let id = Oid::from_string(README, OidType::Sha1).unwrap();
                    let updates = [
                        TreeUpdate::upsert(c"z/y/x", id.as_ref(), FileMode::Commit),
                        TreeUpdate::upsert(c"a/b", id.as_ref(), FileMode::Commit),
                        TreeUpdate::upsert(c"a/c/d", id.as_ref(), FileMode::Commit),
                        TreeUpdate::upsert(c"top", id.as_ref(), FileMode::Commit),
                        TreeUpdate::remove(c"gone/never"),
                    ];
                    let (expected, valid) = (&updates[..4], &updates);
                    let written = Tree::create_updated(repo.as_ref(), None, expected).unwrap();
                    let mut refused = 0;
                    for set in [expected, &valid[..]] {
                        for k in 0.. {
                            ALLOW.store(k, SeqCst);
                            REFUSE.store(true, SeqCst);
                            let result = Tree::create_updated(repo.as_ref(), None, set);
                            REFUSE.store(false, SeqCst);
                            let exhausted = ALLOW.load(SeqCst) <= 0;
                            match result {
                                Ok(id) => {
                                    assert_eq!(id, written);
                                    if !exhausted {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    refused += usize::from(e.class() == Some(ErrorClass::NoMemory));
                                    if !exhausted {
                                        // The removal of a missing entry, once memory suffices.
                                        assert_eq!(e.class(), Some(ErrorClass::Tree), "{e:?}");
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    assert!(refused > 10, "{refused}");
                    drop(repo);
                    let _ = std::fs::remove_dir_all(&path);
                },
            );
        }

        /// `bypath` looks into a tree at every depth and hands out an owned copy, through
        /// a tree looked up as one or viewed as one from an object; an object of any other
        /// type has no tree handle, so libgit2 never reads it as a tree; and a refused
        /// allocation for the copy is libgit2's out-of-memory error.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn bypath_copies_out_of_trees() {
            isolated(
                concat!(module_path!(), "::bypath_copies_out_of_trees"),
                || {
                    use crate::libgit2::object::object::Object;
                    use crate::libgit2::util::errors::ErrorCode;
                    install_refusing();
                    let repo = open();
                    let object = |hex: &CStr| {
                        let id = Oid::from_string(hex, OidType::Sha1).unwrap();
                        Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap()
                    };
                    let tree = lookup_tree(&repo);
                    let entry = TreeEntry::bypath(tree.as_ref(), c"ab/de/fgh/1.txt").unwrap();
                    assert_eq!(view(entry.as_ref()), expected("1.txt", NESTED));
                    let as_object = object(TREE);
                    let viewed = TreeRef::from_object(as_object.as_ref()).unwrap();
                    assert_eq!(
                        view(TreeEntry::bypath(viewed, c"README").unwrap().as_ref()),
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
                        let other = object(hex);
                        assert!(TreeRef::from_object(other.as_ref()).is_none(), "{hex:?}");
                    }

                    REFUSE.store(true, SeqCst);
                    let refused = TreeEntry::bypath(tree.as_ref(), c"README");
                    REFUSE.store(false, SeqCst);
                    let e = refused.unwrap_err();
                    assert_eq!(e.code(), Some(ErrorCode::Generic));
                    assert_eq!(e.class(), Some(ErrorClass::NoMemory));
                    drop((tree, as_object));
                    drop(repo);
                    assert_eq!(view(entry.as_ref()), expected("1.txt", NESTED));
                },
            );
        }

        /// A Rust-built entry of `name` with mode `attr` and id `id`, for C to read; its
        /// name must outlive it.
        fn entry_of(name: &CStr, attr: u16, id: ffi::git_oid) -> RawEntry {
            RawEntry {
                attr,
                filename_len: name.to_bytes().len() as u16,
                oid: id,
                filename: name.as_ptr(),
            }
        }

        /// `id`'s C value.
        fn raw_id(id: &Oid) -> ffi::git_oid {
            ffi::git_oid {
                type_: id.as_ref().raw_kind(),
                id: id.as_ref().id(),
            }
        }

        /// A handle over a Rust-built entry, live for the borrow.
        fn handle(raw: &mut RawEntry) -> TreeEntryRef<'_> {
            // SAFETY: `raw` is live and unchanged while it is borrowed, and its name
            // outlives it; nothing writes through the shared handle.
            unsafe { TreeEntryRef::from_ptr(ptr::from_mut(raw).cast()) }.unwrap()
        }

        /// The getters read what C reads, on borrowed and owned entries alike, and lend
        /// the name and id for the entry's borrow.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn getters_read_what_c_reads() {
            let repo = open();
            let tree = lookup_tree(&repo);
            let readme = entry(tree.as_ref(), c"README");
            let sub = entry(tree.as_ref(), c"ab");
            let owned = adopt_bypath(tree.as_ref(), c"ab/de/fgh/1.txt");
            for entry in [readme, sub, owned.as_ref()] {
                let (name, hex, raw) = view(entry);
                assert_eq!(entry.name().to_str().unwrap(), name);
                assert_eq!(entry.id().to_hex_string().unwrap(), hex);
                assert_eq!(entry.filemode_raw(), raw);
                assert_eq!(entry.filemode().raw(), raw);
                // SAFETY: a live entry, only read.
                let c_kind = unsafe { ffi::git_tree_entry_type(entry.as_ptr()) };
                assert_eq!(entry.kind().raw(), c_kind);
            }
            assert_eq!(readme.kind(), ObjectType::Blob);
            assert_eq!(sub.kind(), ObjectType::Tree);
            assert_eq!(sub.filemode(), FileMode::Tree);
            let name = owned.as_ref().name();
            let id = owned.as_ref().id();
            drop(tree);
            drop(repo);
            assert_eq!(name, c"1.txt");
            assert_eq!(id.to_hex_string().unwrap(), NESTED.to_str().unwrap());
        }

        /// Modes a parsed tree may hold are normalised as git does, the stored mode
        /// kept as is, and the type follows the stored mode.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn modes_normalise_as_git_does() {
            let cases: [(u16, FileMode, ObjectType); 10] = [
                (0o100644, FileMode::Blob, ObjectType::Blob),
                (0o100664, FileMode::Blob, ObjectType::Blob),
                (0o100755, FileMode::BlobExecutable, ObjectType::Blob),
                (0o100700, FileMode::BlobExecutable, ObjectType::Blob),
                (0o120000, FileMode::Link, ObjectType::Blob),
                (0o040000, FileMode::Tree, ObjectType::Tree),
                (0o040755, FileMode::Tree, ObjectType::Tree),
                (0o160000, FileMode::Commit, ObjectType::Commit),
                (0, FileMode::Blob, ObjectType::Blob),
                (0o000100, FileMode::BlobExecutable, ObjectType::Blob),
            ];
            for (attr, mode, kind) in cases {
                let mut raw = entry_of(c"x", attr, ffi::git_oid::default());
                let entry = handle(&mut raw);
                assert_eq!(entry.filemode_raw(), u32::from(attr), "{attr:o}");
                assert_eq!(entry.filemode(), mode, "{attr:o}");
                assert_eq!(entry.kind(), kind, "{attr:o}");
            }
        }

        /// Entries sort by name, a tree's name as if it ended in `/`; equal names and
        /// kinds are equal whatever the ids; the ordering is C's sign.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn entries_sort_in_tree_order() {
            use core::cmp::Ordering::*;
            const FILE: u16 = 0o100644;
            const DIR: u16 = 0o040000;
            const LINK: u16 = 0o160000;
            let other = Oid::from_string(README, OidType::Sha1).unwrap();
            /// A name and a mode.
            type Side = (&'static CStr, u16);
            let cases: [(Side, Side, Ordering); 11] = [
                ((c"a", FILE), (c"a", FILE), Equal),
                ((c"a", FILE), (c"b", FILE), Less),
                ((c"a", DIR), (c"a.c", FILE), Greater),
                ((c"a", FILE), (c"a.c", FILE), Less),
                ((c"a", FILE), (c"a", DIR), Less),
                ((c"a", DIR), (c"a", DIR), Equal),
                ((c"a", DIR), (c"a/", FILE), Equal),
                ((c"a", LINK), (c"a", FILE), Equal),
                ((c"a", DIR), (c"a0", FILE), Less),
                ((c"\xff", FILE), (c"a", FILE), Greater),
                ((c"", FILE), (c"a", FILE), Less),
            ];
            for ((n1, m1), (n2, m2), expected) in cases {
                let mut a = entry_of(n1, m1, ffi::git_oid::default());
                let mut b = entry_of(n2, m2, raw_id(&other));
                let (a, b) = (handle(&mut a), handle(&mut b));
                assert_eq!(a.cmp_in_tree(b), expected, "{n1:?} {m1:o} / {n2:?} {m2:o}");
                assert_eq!(b.cmp_in_tree(a), expected.reverse());
                // SAFETY: both live entries, only read.
                let c = unsafe { ffi::git_tree_entry_cmp(a.as_ptr(), b.as_ptr()) };
                assert_eq!(c.cmp(&0), expected);
            }
            let repo = open();
            let tree = lookup_tree(&repo);
            let (readme, sub) = (entry(tree.as_ref(), c"README"), entry(tree.as_ref(), c"ab"));
            assert_eq!(readme.cmp_in_tree(sub), Less);
            let owned = adopt_bypath(tree.as_ref(), c"README");
            assert_eq!(readme.cmp_in_tree(owned.as_ref()), Equal);
        }

        /// An entry's object is looked up in the repository given, whatever the entry
        /// came from; a missing object is `NotFound`, and an id of another type than
        /// the repository's is refused before C formats it.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn entries_look_their_objects_up() {
            use crate::libgit2::object::object::Object;
            let repo = open();
            let tree = lookup_tree(&repo);
            let readme = entry(tree.as_ref(), c"README")
                .to_object(repo.as_ref())
                .unwrap();
            assert_eq!(readme.as_ref().kind(), ObjectType::Blob);
            // Trees are cached, so the same id gives the same object.
            let sub_entry = entry(tree.as_ref(), c"ab");
            let sub = sub_entry.to_object(repo.as_ref()).unwrap();
            assert_eq!(sub.as_ref().kind(), ObjectType::Tree);
            let again = Object::lookup(repo.as_ref(), sub_entry.id(), None).unwrap();
            assert!(ObjectArc::ptr_eq(&sub, &again));
            let owned = adopt_bypath(tree.as_ref(), c"ab/de/fgh/1.txt");
            drop(tree);
            let nested = owned.as_ref().to_object(repo.as_ref()).unwrap();
            assert_eq!(nested.as_ref().kind(), ObjectType::Blob);
            let id = Oid::from_string(NESTED, OidType::Sha1).unwrap();

            let mut missing = entry_of(c"sub", 0o160000, raw_id(&Oid::zero(OidType::Sha1)));
            missing.oid.id[..20].fill(0xde);
            let e = handle(&mut missing).to_object(repo.as_ref()).unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::NotFound));

            let mut wide = entry_of(c"x", 0o100644, raw_id(&id));
            wide.oid.type_ = ffi::GIT_OID_SHA256 as u8;
            let e = handle(&mut wide).to_object(repo.as_ref()).unwrap_err();
            assert_eq!(e.raw_code(), -1);
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
            assert_eq!(
                e.message(),
                c"specified object ID type does not match repository object ID type"
            );
        }

        /// A builder seeded with `TREE`'s entries, for `repo`.
        fn seeded(repo: &RepositoryBox) -> TreeBuilderBox<'_> {
            let tree = lookup_tree(repo);
            TreeBuilder::from_tree(repo.as_ref(), tree.as_ref()).unwrap()
        }

        /// `git_treebuilder_filter` through the trampoline: the closure sees every entry
        /// once, and the entries it returns `true` for are removed and freed (ASan).
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn filters_remove_what_the_closure_picks() {
            assert!(runtime_pinned());
            let repo = open();
            let mut builder = seeded(&repo);
            let mut seen = Vec::new();
            let mut drop_txt = |entry: TreeEntryRef<'_>| {
                seen.push((entry.name().to_owned(), entry.filemode()));
                entry.name().to_bytes().ends_with(b".txt")
            };
            let mut filter = TreeBuilderFilter::new(&mut drop_txt);
            assert_eq!(
                format!("{filter:?}"),
                "TreeBuilderFilter { panicked: false, .. }"
            );
            let p = builder.as_mut().as_mut_ptr();
            let rc = filter.filter_with(|callback, payload| {
                // SAFETY: a live builder nothing else reaches; the callback and payload
                // are the filter's, live for the call.
                unsafe { ffi::git_treebuilder_filter(p, callback, payload) }
            });
            assert_eq!(rc, 0);
            drop(filter);
            seen.sort_by(|a, b| a.0.cmp(&b.0));
            let names: Vec<_> = seen.iter().map(|(n, _)| n.to_str().unwrap()).collect();
            assert_eq!(names, ["README", "ab", "branch_file.txt", "new.txt"]);
            assert_eq!(seen[1].1, FileMode::Tree);
            assert_eq!(builder.as_ref().len(), 2);

            // The handle reaches the same trampoline for one entry.
            let mut keep_all = |_: TreeEntryRef<'_>| false;
            let mut filter = TreeBuilderFilter::new(&mut keep_all);
            let tree = lookup_tree(&repo);
            assert!(!filter.call(entry(tree.as_ref(), c"README")));
        }

        /// A panicking closure: libgit2 cannot stop, so the trampoline keeps the entry it
        /// panicked on and every later one without calling the closure again, and the
        /// panic resumes once `git_treebuilder_filter` has returned.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn a_panicking_filter_keeps_the_rest() {
            assert!(runtime_pinned());
            let repo = open();
            let mut builder = seeded(&repo);
            let mut calls = 0;
            let mut boom = |_: TreeEntryRef<'_>| {
                calls += 1;
                if calls == 2 {
                    panic!("filter panicked");
                }
                true
            };
            let p = builder.as_mut().as_mut_ptr();
            let mut filter = TreeBuilderFilter::new(&mut boom);
            let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
                filter.filter_with(|callback, payload| {
                    // SAFETY: as in `filters_remove_what_the_closure_picks`.
                    unsafe { ffi::git_treebuilder_filter(p, callback, payload) }
                })
            }));
            let message = caught.unwrap_err();
            assert_eq!(message.downcast_ref::<&str>(), Some(&"filter panicked"));
            assert_eq!(
                format!("{filter:?}"),
                "TreeBuilderFilter { panicked: false, .. }"
            );
            drop(filter);
            assert_eq!(calls, 2);
            assert_eq!(builder.as_ref().len(), 3);

            let tree = lookup_tree(&repo);
            let mut always = |_: TreeEntryRef<'_>| -> bool { panic!("direct call") };
            let mut filter = TreeBuilderFilter::new(&mut always);
            let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
                filter.call(entry(tree.as_ref(), c"ab"))
            }));
            assert!(caught.is_err());
        }

        /// The trampoline keeps an entry libgit2 would never pass as NULL, and once a
        /// panic is recorded stops calling the closure.
        #[test]
        fn trampoline_guards_without_c() {
            let mut called = false;
            let mut f = |_: TreeEntryRef<'_>| {
                called = true;
                true
            };
            let mut filter = TreeBuilderFilter::new(&mut f);
            let payload: *mut c_void = ptr::from_mut(&mut filter).cast();
            // SAFETY: `payload` is the live filter, unused elsewhere; NULL is allowed.
            assert_eq!(unsafe { filter_with(ptr::null(), payload) }, 0);
            filter.panic = Some(Box::new("recorded"));
            // The write above went through `filter`; borrow it afresh for the call.
            let payload: *mut c_void = ptr::from_mut(&mut filter).cast();
            let mut raw = raw_entry();
            let entry: *const ffi::git_tree_entry = ptr::from_mut(&mut raw).cast();
            // SAFETY: as above; `raw` is live and only read.
            assert_eq!(unsafe { filter_with(entry, payload) }, 0);
            assert_eq!(
                format!("{filter:?}"),
                "TreeBuilderFilter { panicked: true, .. }"
            );
            drop(filter);
            assert!(!called);
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

    /// The builder's entry routines and the walk callback's trampoline.
    mod builder_routines {
        use crate::libgit2::core::types::FileMode;
        use crate::libgit2::object::object::Object;
        use crate::libgit2::object::tree::*;
        use crate::libgit2::oid::oid::{Oid, OidType};
        use crate::libgit2::repository::repository::{Repository, RepositoryBox};
        use crate::libgit2::util::errors::{ErrorClass, ErrorCode, last};
        use crate::libgit2::util::sys::errors::runtime_pinned;
        use core::ffi::{CStr, c_char, c_int, c_void};
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::panic::{self, AssertUnwindSafe};
        use std::path::PathBuf;
        use std::ptr;

        /// A fresh bare repository in a temporary directory, removed on drop after the
        /// repository it holds.
        struct TempRepo {
            path: PathBuf,
            repo: Option<RepositoryBox>,
        }

        impl TempRepo {
            fn new(tag: &str) -> Self {
                assert!(runtime_pinned());
                let path = std::env::temp_dir()
                    .join(format!("crustify-tree-unit-{}-{tag}", std::process::id()));
                let _ = std::fs::remove_dir_all(&path);
                let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
                let repo = Repository::init(&c_path, true).unwrap();
                Self {
                    path,
                    repo: Some(repo),
                }
            }

            fn repo(&self) -> &RepositoryBox {
                self.repo.as_ref().unwrap()
            }
        }

        impl Drop for TempRepo {
            fn drop(&mut self) {
                drop(self.repo.take());
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }

        /// A SHA1 id of `byte` repeated, for submodule entries libgit2 does not check.
        fn id(byte: u8) -> Oid {
            Oid::from_raw(&[byte; 20], OidType::Sha1).unwrap()
        }

        fn count(builder: &TreeBuilderBox<'_>) -> usize {
            // SAFETY: the box's live builder, only read.
            unsafe { ffi::git_treebuilder_entrycount(builder.as_c_ptr()) }
        }

        /// C's name of a live entry.
        fn name_of(entry: TreeEntryRef<'_>) -> CString {
            // SAFETY: a live entry the handle borrows; its name lives with it and is
            // copied at once.
            unsafe { CStr::from_ptr(ffi::git_tree_entry_name(entry.as_ptr())) }.to_owned()
        }

        fn lookup<'r>(repo: &'r RepositoryBox, id: &Oid) -> TreeArc<'r> {
            Tree::lookup(repo.as_ref(), id.as_ref()).unwrap()
        }

        /// The walked tree: submodule leaves `a`, `z`, `dir/b`, `dir/sub/c`, and the
        /// subtrees `dir`, `dir/sub` and `nest/leaf` (holding `e`).
        fn nested_tree(repo: &RepositoryBox) -> Oid {
            let subtree = |entries: &[(&CStr, Oid, FileMode)]| {
                let mut b = TreeBuilder::new(repo.as_ref()).unwrap();
                for (name, id, mode) in entries {
                    b.as_mut().insert(name, id.as_ref(), *mode).unwrap();
                }
                b.as_mut().write().unwrap()
            };
            let sub = subtree(&[(c"c", id(3), FileMode::Commit)]);
            let dir = subtree(&[
                (c"b", id(2), FileMode::Commit),
                (c"sub", sub, FileMode::Tree),
            ]);
            let leaf = subtree(&[(c"e", id(5), FileMode::Commit)]);
            let nest = subtree(&[(c"leaf", leaf, FileMode::Tree)]);
            subtree(&[
                (c"a", id(1), FileMode::Commit),
                (c"dir", dir, FileMode::Tree),
                (c"nest", nest, FileMode::Tree),
                (c"z", id(26), FileMode::Commit),
            ])
        }

        /// A walk's return, its visits as `root` + name, and the thread's error then.
        type Walked = (c_int, Vec<String>, (c_int, String));

        fn error_now() -> (c_int, String) {
            let e = last();
            (e.raw_class(), e.message().to_string_lossy().into_owned())
        }

        /// The answer to the entry `name` under `root`, as a C callback gives it.
        type Rule = fn(&str, &str) -> c_int;

        const RULES: [Rule; 6] = [
            |_, _| 0,
            |_, name| c_int::from(name == "dir"),
            |root, _| c_int::from(!root.is_empty()) * 9,
            |_, name| if name == "sub" { -1 } else { 0 },
            |_, name| if name == "e" { -42 } else { 0 },
            |_, name| {
                if name == "a" {
                    ErrorCode::User.raw()
                } else {
                    0
                }
            },
        ];

        /// The C side: `git_tree_walk` with a plain C callback applying `rule`.
        fn c_walk(tree: &TreeArc<'_>, mode: WalkMode, rule: Rule) -> Walked {
            struct State {
                rule: Rule,
                visits: Vec<String>,
            }
            unsafe extern "C" fn cb(
                root: *const c_char,
                entry: *const ffi::git_tree_entry,
                payload: *mut c_void,
            ) -> c_int {
                // SAFETY: `payload` is the walk's `State`, `root` and `entry` live for the
                // call.
                unsafe {
                    let state = &mut *payload.cast::<State>();
                    let root = CStr::from_ptr(root).to_str().unwrap();
                    let name = CStr::from_ptr(ffi::git_tree_entry_name(entry));
                    let name = name.to_str().unwrap();
                    state.visits.push(format!("{root}{name}"));
                    (state.rule)(root, name)
                }
            }
            let mut state = State {
                rule,
                visits: Vec::new(),
            };
            crate::libgit2::util::sys::errors::clear();
            // SAFETY: a live tree, the callback and its payload, live for the walk.
            let rc = unsafe {
                ffi::git_tree_walk(
                    tree.as_c_ptr(),
                    mode.raw(),
                    Some(cb),
                    ptr::from_mut(&mut state).cast(),
                )
            };
            (rc, state.visits, error_now())
        }

        /// The Rust side: the same walk, through a [`WalkCallback`] whose closure answers
        /// as `rule` does, read back through [`WalkAction`].
        fn rust_walk(tree: &TreeArc<'_>, mode: WalkMode, rule: Rule) -> (Walked, bool) {
            let mut visits = Vec::new();
            let mut f = |root: &CStr, entry: TreeEntryRef<'_>| {
                let (root, name) = (root.to_str().unwrap(), name_of(entry));
                let name = name.to_str().unwrap();
                visits.push(format!("{root}{name}"));
                match rule(root, name) {
                    0 => WalkAction::Continue,
                    1.. => WalkAction::Skip,
                    _ => WalkAction::Stop,
                }
            };
            let mut callback = WalkCallback::new(&mut f);
            crate::libgit2::util::sys::errors::clear();
            let rc = callback.walk_with(|cb, payload| {
                // SAFETY: a live tree, and the trampoline with its payload, live and
                // unused otherwise for the walk.
                unsafe { ffi::git_tree_walk(tree.as_c_ptr(), mode.raw(), cb, payload) }
            });
            let stopped = callback.stopped();
            drop(callback);
            ((rc, visits, error_now()), stopped)
        }

        /// Answers map to the values libgit2 reads, and back as libgit2 reads a value.
        #[test]
        fn walk_actions_round_trip() {
            for action in [WalkAction::Continue, WalkAction::Skip, WalkAction::Stop] {
                assert_eq!(WalkAction::from_raw(action.raw()), action);
            }
            assert_eq!(WalkAction::Continue.raw(), 0);
            assert_eq!(WalkAction::Skip.raw(), 1);
            assert_eq!(WalkAction::Stop.raw(), ffi::GIT_EUSER);
            for (rc, action) in [
                (2, WalkAction::Skip),
                (c_int::MAX, WalkAction::Skip),
                (-1, WalkAction::Stop),
                (c_int::MIN, WalkAction::Stop),
            ] {
                assert_eq!(WalkAction::from_raw(rc), action);
            }
        }

        /// A walk through the trampoline calls back for the same entries, under the same
        /// roots, as one through a C callback answering alike, in both orders, and returns
        /// and leaves the same: skipping, stopping at every depth, `GIT_EUSER` included.
        /// A C callback's own stop code, other than `GIT_EUSER`, is the one difference:
        /// the closure stops with `GIT_EUSER` alone.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn walks_match_a_c_callback() {
            let temp = TempRepo::new("walk");
            let tree_id = nested_tree(temp.repo());
            let tree = lookup(temp.repo(), &tree_id);
            for mode in [WalkMode::PreOrder, WalkMode::PostOrder] {
                for (i, rule) in RULES.into_iter().enumerate() {
                    let c = c_walk(&tree, mode, rule);
                    let (rust, stopped) = rust_walk(&tree, mode, rule);
                    assert_eq!(stopped, c.0 < 0, "{mode:?} rule {i}");
                    if c.0 == 0 || c.0 == ffi::GIT_EUSER {
                        assert_eq!(rust, c, "{mode:?} rule {i}");
                    } else {
                        // The closure stops with `GIT_EUSER`, and libgit2 reports that.
                        assert_eq!(rust.1, c.1, "{mode:?} rule {i}");
                        assert_eq!(rust.0, ffi::GIT_EUSER);
                        assert_eq!(
                            rust.2,
                            (
                                ffi::GIT_ERROR_CALLBACK as c_int,
                                "git_tree_walk callback returned -7".into()
                            )
                        );
                    }
                }
            }
            let (all, _) = rust_walk(&tree, WalkMode::PreOrder, RULES[0]);
            assert_eq!(
                all.1,
                [
                    "a",
                    "dir",
                    "dir/b",
                    "dir/sub",
                    "dir/sub/c",
                    "nest",
                    "nest/leaf",
                    "nest/leaf/e",
                    "z"
                ]
            );
            let (skipped, _) = rust_walk(&tree, WalkMode::PreOrder, RULES[1]);
            assert_eq!(
                skipped.1,
                ["a", "dir", "nest", "nest/leaf", "nest/leaf/e", "z"]
            );
        }

        /// `TreeRef::walk` reads what the walk returned as the trampoline path does: `Ok(Continue)`
        /// for 0, `Ok(Break)` for the closure's stop, and libgit2's failure otherwise, after
        /// the same visits and leaving the same thread error. An empty tree is walked
        /// without a call, and a missing subtree fails the walk with the lookup's error.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn public_walks_report_how_they_ended() {
            let temp = TempRepo::new("public-walk");
            let tree_id = nested_tree(temp.repo());
            let tree = lookup(temp.repo(), &tree_id);
            for mode in [WalkMode::PreOrder, WalkMode::PostOrder] {
                for (i, rule) in RULES.into_iter().enumerate() {
                    let (expected, stopped) = rust_walk(&tree, mode, rule);
                    let mut visits = Vec::new();
                    let mut f = |root: &CStr, entry: TreeEntryRef<'_>| {
                        let (root, name) = (root.to_str().unwrap(), name_of(entry));
                        let name = name.to_str().unwrap();
                        visits.push(format!("{root}{name}"));
                        WalkAction::from_raw(rule(root, name))
                    };
                    let mut callback = WalkCallback::new(&mut f);
                    crate::libgit2::util::sys::errors::clear();
                    let walked = tree.as_ref().walk(mode, &mut callback).unwrap();
                    assert_eq!(callback.stopped(), stopped, "{mode:?} rule {i}");
                    drop(callback);
                    let want = if stopped {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    };
                    assert_eq!(walked, want, "{mode:?} rule {i}");
                    assert_eq!(expected.0 == 0, !stopped);
                    assert_eq!((visits, error_now()), (expected.1, expected.2));
                }
            }

            let empty_id = TreeBuilder::new(temp.repo().as_ref())
                .unwrap()
                .as_mut()
                .write()
                .unwrap();
            let empty = lookup(temp.repo(), &empty_id);
            let mut calls = 0;
            let mut f = |_: &CStr, _: TreeEntryRef<'_>| {
                calls += 1;
                WalkAction::Stop
            };
            let mut callback = WalkCallback::new(&mut f);
            for mode in [WalkMode::PreOrder, WalkMode::PostOrder] {
                let walked = empty.as_ref().walk(mode, &mut callback);
                assert_eq!(walked.unwrap(), ControlFlow::Continue(()));
                assert!(!callback.stopped());
            }
            drop(callback);
            assert_eq!(calls, 0);

            // A subtree the object database lacks: the lookup's error, whatever the
            // closure answered before.
            // A builder refuses a tree entry naming no tree, so it is spelled raw.
            let missing = id(0x77);
            let mut raw = b"160000 a\0".to_vec();
            raw.extend_from_slice(&[0x11; 20]);
            raw.extend_from_slice(b"40000 broken\0");
            raw.extend_from_slice(&missing.as_ref().id()[..20]);
            let broken_id = write_raw_tree(temp.repo(), &raw);
            let broken = lookup(temp.repo(), &broken_id);
            for mode in [WalkMode::PreOrder, WalkMode::PostOrder] {
                let mut f = |_: &CStr, _: TreeEntryRef<'_>| WalkAction::Continue;
                let mut callback = WalkCallback::new(&mut f);
                let e = broken.as_ref().walk(mode, &mut callback).unwrap_err();
                assert_eq!(e.code(), Some(ErrorCode::NotFound), "{e:?}");
                assert!(!callback.stopped());
            }
        }

        /// `data` written into `repo`'s object database as a tree, unparsed; its id.
        fn write_raw_tree(repo: &RepositoryBox, data: &[u8]) -> Oid {
            let odb = repo.as_ref().odb().unwrap();
            let mut out = Oid::zeroed();
            // SAFETY: a live odb, a writable id, and `data` read for the call.
            let rc = unsafe {
                ffi::git_odb_write(
                    out.as_mut().as_mut_ptr(),
                    odb.as_c_ptr(),
                    data.as_ptr().cast(),
                    data.len(),
                    ffi::GIT_OBJECT_TREE,
                )
            };
            assert_eq!(rc, 0);
            out
        }

        /// The tree's entries come out of `iter` in the tree's order, as `get` gives them,
        /// from either end, with an exact length that counts down; past the end, `get`
        /// and the iterator give nothing, and keep giving nothing. A copy of the iterator
        /// goes on independently, and the handle itself iterates.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn tree_entries_iterate_from_either_end() {
            let temp = TempRepo::new("iterate");
            let tree_id = nested_tree(temp.repo());
            let tree = lookup(temp.repo(), &tree_id);
            let t = tree.as_ref();
            assert_eq!((t.len(), t.is_empty()), (4, false));
            let names = |entries: &mut dyn Iterator<Item = TreeEntryRef<'_>>| -> Vec<CString> {
                entries.map(name_of).collect()
            };
            let forward = [c"a", c"dir", c"nest", c"z"].map(CStr::to_owned);
            assert_eq!(names(&mut t.iter()), forward);
            assert_eq!(names(&mut t.into_iter()), forward);
            assert_eq!(
                names(&mut t.iter().rev()),
                forward.iter().rev().cloned().collect::<Vec<_>>()
            );
            for (i, entry) in t.iter().enumerate() {
                assert_eq!(entry.as_ptr(), t.get(i).unwrap().as_ptr());
            }
            assert!(t.get(4).is_none() && t.get(usize::MAX).is_none());

            let mut it = t.iter();
            assert_eq!(it.len(), 4);
            assert_eq!(
                format!("{it:?}"),
                "TreeEntries { tree: Tree { .. }, range: 0..4 }"
            );
            assert_eq!(name_of(it.next().unwrap()), c"a".to_owned());
            assert_eq!(name_of(it.next_back().unwrap()), c"z".to_owned());
            assert_eq!(it.size_hint(), (2, Some(2)));
            let copy = it.clone();
            assert_eq!(name_of(it.nth(1).unwrap()), c"nest".to_owned());
            assert_eq!(it.len(), 0);
            assert!(it.next().is_none() && it.next_back().is_none() && it.next().is_none());
            assert_eq!(
                names(&mut copy.clone()),
                [c"dir", c"nest"].map(CStr::to_owned)
            );
            assert!(copy.clone().nth(2).is_none());

            let empty_id = TreeBuilder::new(temp.repo().as_ref())
                .unwrap()
                .as_mut()
                .write()
                .unwrap();
            let empty = lookup(temp.repo(), &empty_id);
            let e = empty.as_ref();
            assert_eq!((e.len(), e.is_empty(), e.iter().len()), (0, true, 0));
            assert!(e.get(0).is_none() && e.iter().next().is_none());
            assert!(e.get_by_name(c"a").is_none() && e.get_by_id(id(1).as_ref()).is_none());
        }

        /// A panic in the closure stops the walk with `GIT_EUSER` and resumes once the walk
        /// has returned, from the wrapper running it, without unwinding through C; a call
        /// coming after it stops at once, without reaching the closure.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn panics_resume_after_the_walk() {
            let temp = TempRepo::new("panic");
            let tree_id = nested_tree(temp.repo());
            let tree = lookup(temp.repo(), &tree_id);
            let mut calls = 0;
            let mut f = |_: &CStr, _: TreeEntryRef<'_>| {
                calls += 1;
                if calls == 2 {
                    panic!("closure panicked");
                }
                WalkAction::Continue
            };
            let mut callback = WalkCallback::new(&mut f);
            let mut rc = 0;
            let mut again = 0;
            let resumed = panic::catch_unwind(AssertUnwindSafe(|| {
                callback.walk_with(|cb, payload| {
                    // SAFETY: a live tree, and the trampoline with its payload.
                    rc = unsafe {
                        ffi::git_tree_walk(tree.as_c_ptr(), WalkMode::PreOrder.raw(), cb, payload)
                    };
                    // SAFETY: as libgit2 calls it: the payload, a root and a live entry.
                    let entry = unsafe { ffi::git_tree_entry_byindex(tree.as_c_ptr(), 0) };
                    // SAFETY: as above.
                    again = unsafe { cb.unwrap()(c"".as_ptr(), entry, payload) };
                })
            }));
            let payload = resumed.unwrap_err();
            assert_eq!(payload.downcast_ref::<&str>(), Some(&"closure panicked"));
            assert_eq!(rc, ffi::GIT_EUSER);
            assert_eq!(again, ffi::GIT_EUSER);
            assert!(callback.stopped());
            assert_eq!(
                format!("{callback:?}"),
                "WalkCallback { stopped: true, panicked: false, .. }"
            );
            drop(callback);
            assert_eq!(calls, 2);
        }

        /// The trampoline stops, without calling the closure, when libgit2 would pass a
        /// NULL root or entry (it never does). Nothing reaches libgit2, so Miri runs it.
        #[test]
        fn trampoline_refuses_null_arguments() {
            #[repr(C)]
            struct RawEntry {
                attr: u16,
                filename_len: u16,
                oid: ffi::git_oid,
                filename: *const c_char,
            }
            let mut raw = RawEntry {
                attr: ffi::GIT_FILEMODE_BLOB as u16,
                filename_len: 1,
                oid: ffi::git_oid::default(),
                filename: c"a".as_ptr(),
            };
            let entry: *const ffi::git_tree_entry = ptr::from_mut(&mut raw).cast();
            let mut calls = 0;
            let mut f = |_: &CStr, _: TreeEntryRef<'_>| {
                calls += 1;
                WalkAction::Continue
            };
            let mut callback = WalkCallback::new(&mut f);
            assert!(!callback.stopped());
            let rcs = callback.walk_with(|cb, payload| {
                let cb = cb.unwrap();
                // SAFETY: the payload with NULL for a root or entry, which the trampoline
                // checks before reading; `entry` is a live local nothing writes.
                unsafe {
                    [
                        cb(ptr::null(), entry, payload),
                        cb(c"".as_ptr(), ptr::null(), payload),
                        cb(ptr::null(), ptr::null(), payload),
                    ]
                }
            });
            assert_eq!(rcs, [ffi::GIT_EUSER; 3]);
            assert!(callback.stopped());
            let rc = callback.walk_with(|cb, payload| {
                // SAFETY: the payload, a root and a live local entry, unread by the
                // closure.
                unsafe { cb.unwrap()(c"x/".as_ptr(), entry, payload) }
            });
            assert_eq!(rc, 0);
            assert!(!callback.stopped());
            drop(callback);
            assert_eq!(calls, 1);
        }

        /// `repository` reads the repository a builder keeps from libgit2's private
        /// struct: the very repository it was built for, empty or seeded, of either id
        /// type. This pins the layout assumption (`repo` first in `struct
        /// git_treebuilder`) the id-type check of `insert` rests on.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn builders_name_the_repository_they_were_built_for() {
            let temp = TempRepo::new("builder-repo");
            let repo = temp.repo();
            let sha256 = Repository::with_oid_type(OidType::Sha256).unwrap();
            for r in [repo.as_ref(), sha256.as_ref()] {
                let builder = TreeBuilder::new(r).unwrap();
                let kept = builder.as_ref().repository();
                assert_eq!(kept.as_ptr(), r.as_ptr());
                assert_eq!(kept.oid_type(), r.oid_type());
            }
            let empty = TreeBuilder::new(repo.as_ref())
                .unwrap()
                .as_mut()
                .write()
                .unwrap();
            let tree = lookup(repo, &empty);
            let seeded = TreeBuilder::from_tree(sha256.as_ref(), tree.as_ref()).unwrap();
            assert_eq!(
                seeded.as_ref().repository().as_ptr(),
                sha256.as_ref().as_ptr()
            );
        }

        /// The corrected divergence: libgit2 inserts a name of more than 65535 bytes,
        /// leaving its own "tree entry path too long" error unreported, and writes a tree
        /// that cannot be read back; `insert` refuses one with that error before calling
        /// libgit2, leaving the builder as it was, and stores the longest name a tree can
        /// whole.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn overlong_names_are_refused() {
            let temp = TempRepo::new("long-names");
            let repo = temp.repo();
            let long = |n: usize| CString::new(vec![b'n'; n]).unwrap();

            // C: accepted, then written as a corrupt tree.
            let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
            let name = long(65536);
            let raw_id = id(7);
            // SAFETY: the box's live builder, unaliased; a NUL-terminated name and a live
            // id, both copied; no entry wanted back.
            let rc = unsafe {
                ffi::git_treebuilder_insert(
                    ptr::null_mut(),
                    builder.as_c_ptr(),
                    name.as_ptr(),
                    raw_id.as_ref().as_ptr(),
                    ffi::GIT_FILEMODE_COMMIT,
                )
            };
            assert_eq!(rc, 0);
            assert_eq!(last().message(), c"tree entry path too long");
            let corrupt = builder.as_mut().write().unwrap();
            let e = Object::lookup(repo.as_ref(), corrupt.as_ref(), None).unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Tree));
            assert_eq!(e.message(), c"failed to parse tree: object is corrupted");
            drop(builder);

            // Rust: refused, at every length past the limit.
            let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
            builder
                .as_mut()
                .insert(c"keep", id(1).as_ref(), FileMode::Commit)
                .unwrap();
            for n in [65536, 65537, 70000, 131072] {
                let name = long(n);
                for e in [
                    builder
                        .as_mut()
                        .insert(&name, id(2).as_ref(), FileMode::Commit),
                    builder
                        .as_mut()
                        .insert_entry(&name, id(2).as_ref(), FileMode::Commit)
                        .map(|_| ()),
                ] {
                    let e = e.unwrap_err();
                    assert_eq!(e.class(), Some(ErrorClass::Invalid), "{n}");
                    assert_eq!(e.message(), c"tree entry path too long");
                }
                assert!(builder.as_ref().get(&name).is_none());
            }
            assert_eq!(count(&builder), 1);

            // The longest name a tree stores is kept whole, and read back.
            let longest = long(65535);
            let mut m = builder.as_mut();
            let entry = m
                .insert_entry(&longest, id(3).as_ref(), FileMode::Commit)
                .unwrap();
            assert_eq!(name_of(entry), longest);
            let written = builder.as_mut().write().unwrap();
            let tree = lookup(repo, &written);
            let stored = tree.as_ref().get_by_name(&longest).unwrap();
            assert_eq!(stored.name().count_bytes(), 65535);
        }

        /// `create_updated` refuses, before libgit2 runs, what libgit2 would format out
        /// of bounds: a baseline of another id type than the repository's, whose subtrees
        /// it would look up there, and an upsert of an id of another type that the tree
        /// builder would look up. A commit's id, which the builder stores unchecked, and
        /// a removal's id, which is never read, are left to libgit2.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn create_updated_refuses_foreign_ids() {
            use crate::libgit2::repository::repository::{InitFlag, InitOptions};
            let temp = TempRepo::new("updated-foreign");
            let repo = temp.repo();
            let path = temp.path.with_extension("sha256");
            let _ = std::fs::remove_dir_all(&path);
            let mut opts = InitOptions::new();
            opts.as_mut().set_flags(InitFlag::Bare | InitFlag::Mkdir);
            opts.as_mut().set_oid_type(Some(OidType::Sha256));
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let wide = Repository::init_ext(&c_path, opts.as_mut()).unwrap();
            let wide_tree = TreeBuilder::new(wide.as_ref())
                .unwrap()
                .as_mut()
                .write()
                .unwrap();
            let wide_tree = lookup(&wide, &wide_tree);
            let mismatch = |e: RepositoryError| {
                assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
                assert_eq!(
                    e.message(),
                    c"specified object ID type does not match repository object ID type"
                );
            };
            let objects = || {
                std::fs::read_dir(temp.path.join("objects"))
                    .unwrap()
                    .count()
            };
            let before = objects();

            mismatch(
                Tree::create_updated(repo.as_ref(), Some(wide_tree.as_ref()), &[]).unwrap_err(),
            );
            let long_id = Oid::from_raw(&[7; 32], OidType::Sha256).unwrap();
            for mode in [
                FileMode::Blob,
                FileMode::BlobExecutable,
                FileMode::Link,
                FileMode::Tree,
            ] {
                let updates = [
                    TreeUpdate::upsert(c"ok", id(1).as_ref(), FileMode::Commit),
                    TreeUpdate::upsert(c"dir/wide", long_id.as_ref(), mode),
                ];
                mismatch(Tree::create_updated(repo.as_ref(), None, &updates).unwrap_err());
            }
            assert_eq!(objects(), before);

            // A commit's id is stored as it is, and a removal reads no id.
            let mut removal = TreeUpdate::remove(c"gone");
            removal.as_mut().set_id(long_id.as_ref());
            let e = Tree::create_updated(repo.as_ref(), None, &[removal]).unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Tree));
            let commit = [TreeUpdate::upsert(
                c"sub",
                long_id.as_ref(),
                FileMode::Commit,
            )];
            assert!(Tree::create_updated(repo.as_ref(), None, &commit).is_ok());
            // So is a zero id, which the builder refuses before any lookup.
            let zero = Oid::zero(OidType::Sha256);
            let zeroed = [TreeUpdate::upsert(c"z", zero.as_ref(), FileMode::Blob)];
            let e = Tree::create_updated(repo.as_ref(), None, &zeroed).unwrap_err();
            assert_eq!(e.message(), c"failed to insert entry: invalid null OID - z");
            drop(wide_tree);
            drop(wide);
            let _ = std::fs::remove_dir_all(&path);
        }

        /// The corrected divergence: `git_tree_create_updated` upserts a path component
        /// of more than 65535 bytes, as a leaf or a directory, and writes a tree that
        /// cannot be read back, reporting success; `create_updated` refuses one with the
        /// tree builder's "tree entry path too long" before libgit2 runs. A removal's
        /// path is left to libgit2, which fails to find it, and the longest component a
        /// tree stores is written whole.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        #[cfg_attr(
            bsan,
            ignore = "its large inputs outlast the BorrowSanitizer gate, over an hour alone; the other variants run it"
        )]
        fn create_updated_refuses_overlong_components() {
            let temp = TempRepo::new("updated-long");
            let repo = temp.repo();
            let long = |n: usize| "n".repeat(n);

            // C: accepted, then written as a corrupt tree.
            let leaf = CString::new(long(65536)).unwrap();
            let raw = [TreeUpdate::upsert(&leaf, id(7).as_ref(), FileMode::Commit)];
            let mut out = Oid::zero(OidType::Sha1);
            // SAFETY: a live repository, unaliased; a writable id; a live array of one
            // update whose path outlives the call.
            let rc = unsafe {
                ffi::git_tree_create_updated(
                    out.as_mut().as_mut_ptr(),
                    repo.as_c_ptr(),
                    ptr::null_mut(),
                    raw.len(),
                    raw.as_ptr().cast(),
                )
            };
            assert_eq!(rc, 0);
            let e = Object::lookup(repo.as_ref(), out.as_ref(), None).unwrap_err();
            assert_eq!(e.message(), c"failed to parse tree: object is corrupted");

            // Rust: refused, as a leaf or a directory, at every length past the limit.
            for n in [65536, 65537, 131072] {
                for path in [
                    long(n),
                    format!("dir/{}", long(n)),
                    format!("{}/leaf", long(n)),
                ] {
                    let path = CString::new(path).unwrap();
                    let updates = [
                        TreeUpdate::upsert(c"keep", id(1).as_ref(), FileMode::Commit),
                        TreeUpdate::upsert(&path, id(2).as_ref(), FileMode::Commit),
                    ];
                    let e = Tree::create_updated(repo.as_ref(), None, &updates).unwrap_err();
                    assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
                    assert_eq!(e.message(), c"tree entry path too long");
                }
            }

            // A removal of one reaches libgit2, which does not find it.
            let path = CString::new(long(65536)).unwrap();
            let e = Tree::create_updated(repo.as_ref(), None, &[TreeUpdate::remove(&path)])
                .unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Tree));

            // The longest component a tree stores, as a directory and as a leaf.
            let longest = long(65535);
            let path = CString::new(format!("{longest}/{longest}")).unwrap();
            let updates = [TreeUpdate::upsert(&path, id(3).as_ref(), FileMode::Commit)];
            let written = Tree::create_updated(repo.as_ref(), None, &updates).unwrap();
            let root = lookup(repo, &written);
            let entry = TreeEntry::bypath(root.as_ref(), &path).unwrap();
            assert_eq!(entry.as_ref().name().count_bytes(), 65535);
            assert_eq!(entry.as_ref().filemode(), FileMode::Commit);
        }
    }
}
