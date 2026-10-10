//! Safe wrappers over `include/git2/worktree.h`.
//!
//! # Pruning options
//!
//! `git_worktree_is_prunable` and `git_worktree_prune` take a
//! `git_worktree_prune_options` ([`PruneOptions`]): a structure version and a `uint32_t`
//! of `git_worktree_prune_t` bits ([`PruneFlags`], each bit a [`PruneFlag`]). Both
//! routines only read it, for the call: they refuse a version of 0 or above
//! [`PruneOptions::VERSION`] with `GIT_ERROR_INVALID`, then copy it and test each named
//! bit, ignoring the others. So every value is one C accepts without UB, and a flag
//! set keeps the bits it does not name ([`PruneFlags::from_bits_retain`]); a single
//! raw flag is converted fallibly ([`PruneFlag::from_raw`]) and never transmuted.
//!
//! The options are a plain value, never allocated or released by libgit2.
//! [`PruneOptions::new`] fills one through `git_worktree_prune_options_init`, as
//! `GIT_WORKTREE_PRUNE_OPTIONS_INIT` does: the current version and no flags. All-zero
//! is not that default (libgit2 refuses version 0), so there is no `zeroed`.
//!
//! # Worktrees
//!
//! A [`Worktree`] is a `git_worktree`: one of a repository's linked working trees, as
//! described by its administrative directory `<commondir>/worktrees/<name>/`. The
//! header only forward-declares the struct, so Rust never reads its fields; libgit2's
//! own accessors (`git_worktree_name`, `git_worktree_path`, ...) take it by pointer.
//!
//! libgit2 hands a worktree out from three constructors, each through an
//! out-parameter and each given a repository: `git_worktree_lookup` (by name),
//! `git_worktree_open_from_repository` (the worktree a repository was opened from)
//! and `git_worktree_add` (which creates one first). Each allocates the struct and six
//! strings from libgit2's allocator, and the caller owns the result: Rust holds it as
//! a [`WorktreeBox`], which `git_worktree_free` releases on drop. libgit2 has no copy
//! and no reference count for a worktree, so the box is its sole owner and not
//! `Clone`.
//!
//! A worktree copies every string it needs (its name, its paths, the parent's working
//! directory), so it does not borrow the repository it was looked up in and may
//! outlive it: the box carries no lifetime.
//!
//! Two of the constructors need care from their wrappers: `git_worktree_lookup` sets
//! its out slot to NULL before failing, but `git_worktree_open_from_repository` leaves
//! it untouched on failure (so the slot must start NULL), and dereferences its
//! repository without the NULL check the others assert.
//!
//! # Adding options
//!
//! `git_worktree_add` takes a `git_worktree_add_options` ([`AddOptions`]): whether to
//! lock the new worktree, whether to check out an existing branch named after it, the
//! branch reference its `HEAD` is to point at, and the options of its first checkout
//! (a `git_checkout_options`, embedded by value). libgit2 never allocates or frees one:
//! the caller builds it with `git_worktree_add_options_init`, and `git_worktree_add`
//! refuses a version of 0 or above [`AddOptions::VERSION`], copies the struct, and
//! reads it for the call, taking a copy of the reference of its own.
//!
//! So nothing it points at is owned. Rust holds the struct by value as an
//! [`AddOptions<'o>`] (filled by [`AddOptions::new`]), which borrows the reference, and
//! everything the checkout options point at, for `'o`. The reference is an attached
//! [`ReferenceRef`]: `git_worktree_add` copies its database link and reaches its
//! repository through it. The embedded checkout options are reached in place, as a
//! [`CheckoutOptionsRef`] or [`CheckoutOptionsMut`] over the same `'o`.

use core::ffi::{CStr, c_int};
use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{BitOr, BitOrAssign};
use core::ptr::{self, NonNull, addr_of, addr_of_mut};

use ffibox::{CBorrowedPtr, CBox, CCell, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::refs::refs::ReferenceRef;
use crate::libgit2::workdir::checkout::{CheckoutOptionsMut, CheckoutOptionsRef};

/// One `git_worktree_prune_t` bit: a check `git_worktree_is_prunable` skips, or the
/// extra removal `git_worktree_prune` makes. Combine them in a [`PruneFlags`].
///
/// [`from_raw`](Self::from_raw) gives `None` for any value without a variant: `0`,
/// a combination of bits, or a bit this version of libgit2's header does not name.
///
/// Wraps: git_worktree_prune_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum PruneFlag {
    /// Prune the worktree even if it is valid: its gitdir, the parent repository's
    /// working directory, the common directory and its own working directory all
    /// exist.
    Valid = ffi::GIT_WORKTREE_PRUNE_VALID,
    /// Prune the worktree even if it is locked.
    Locked = ffi::GIT_WORKTREE_PRUNE_LOCKED,
    /// Also delete the worktree's working directory, if it still has its `.git`
    /// link, rather than only its gitdir in the parent repository.
    WorkingTree = ffi::GIT_WORKTREE_PRUNE_WORKING_TREE,
}

impl PruneFlag {
    /// Every variant, in ascending bit order.
    pub const ALL: [PruneFlag; 3] = [Self::Valid, Self::Locked, Self::WorkingTree];

    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_worktree_prune_t) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i] as ffi::git_worktree_prune_t == raw {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }

    /// The C value, a single `git_worktree_prune_t` bit.
    pub const fn raw(self) -> ffi::git_worktree_prune_t {
        self as ffi::git_worktree_prune_t
    }

    /// The flag's bit in `git_worktree_prune_options.flags`.
    const fn bit(self) -> u32 {
        self as u32
    }
}

impl TryFrom<ffi::git_worktree_prune_t> for PruneFlag {
    type Error = UnnamedPruneFlag;

    fn try_from(raw: ffi::git_worktree_prune_t) -> Result<Self, UnnamedPruneFlag> {
        Self::from_raw(raw).ok_or(UnnamedPruneFlag(raw))
    }
}

impl From<PruneFlag> for ffi::git_worktree_prune_t {
    fn from(value: PruneFlag) -> ffi::git_worktree_prune_t {
        value.raw()
    }
}

/// A prune flag value that this version of libgit2's header does not name: `0`, a
/// combination of flags, or a later header's bit. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedPruneFlag(ffi::git_worktree_prune_t);

impl UnnamedPruneFlag {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_worktree_prune_t {
        self.0
    }
}

impl fmt::Display for UnnamedPruneFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 worktree prune flag {:#x} is not named by git2/worktree.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedPruneFlag {}

/// A set of [`PruneFlag`]s: the `uint32_t` combination of `GIT_WORKTREE_PRUNE_*` bits
/// that `git_worktree_prune_options.flags` holds.
///
/// It keeps every bit it is given, named or not, as the field does (libgit2 ignores
/// the bits it does not name), so [`bits`](Self::bits) returns what was stored;
/// [`iter`](Self::iter) and [`contains`](Self::contains) see the named flags, and
/// [`unnamed_bits`](Self::unnamed_bits) the rest.
///
/// Wraps: git_worktree_prune_t
/// Handwritten
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PruneFlags(u32);

impl PruneFlags {
    /// The bits of every [`PruneFlag`].
    const NAMED: u32 = {
        let mut bits = 0;
        let mut i = 0;
        while i < PruneFlag::ALL.len() {
            bits |= PruneFlag::ALL[i].bit();
            i += 1;
        }
        bits
    };

    /// The set with no flag: every check applies, and only the gitdir is removed.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The set of every bit of `bits`, as `git_worktree_prune_options.flags` holds
    /// them; bits that no [`PruneFlag`] names are kept.
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// The bits of the set, as `git_worktree_prune_options.flags` holds them.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The bits of the set that no [`PruneFlag`] names.
    pub const fn unnamed_bits(self) -> u32 {
        self.0 & !Self::NAMED
    }

    /// Whether the set has no bit, named or not.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `flag` is in the set.
    pub const fn contains(self, flag: PruneFlag) -> bool {
        self.0 & flag.bit() != 0
    }

    /// Adds `flag` to the set.
    pub fn insert(&mut self, flag: PruneFlag) {
        self.0 |= flag.bit();
    }

    /// Removes `flag` from the set.
    pub fn remove(&mut self, flag: PruneFlag) {
        self.0 &= !flag.bit();
    }

    /// The named flags in the set, in ascending bit order.
    pub fn iter(self) -> PruneFlagsIter {
        PruneFlagsIter { set: self, next: 0 }
    }
}

impl From<PruneFlag> for PruneFlags {
    fn from(flag: PruneFlag) -> Self {
        Self(flag.bit())
    }
}

impl BitOr for PruneFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<PruneFlag> for PruneFlags {
    type Output = Self;

    fn bitor(self, rhs: PruneFlag) -> Self {
        self | Self::from(rhs)
    }
}

impl BitOr for PruneFlag {
    type Output = PruneFlags;

    fn bitor(self, rhs: Self) -> PruneFlags {
        PruneFlags::from(self) | rhs
    }
}

impl BitOrAssign for PruneFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitOrAssign<PruneFlag> for PruneFlags {
    fn bitor_assign(&mut self, rhs: PruneFlag) {
        self.insert(rhs);
    }
}

impl FromIterator<PruneFlag> for PruneFlags {
    fn from_iter<I: IntoIterator<Item = PruneFlag>>(iter: I) -> Self {
        let mut set = Self::empty();
        set.extend(iter);
        set
    }
}

impl Extend<PruneFlag> for PruneFlags {
    fn extend<I: IntoIterator<Item = PruneFlag>>(&mut self, iter: I) {
        for flag in iter {
            self.insert(flag);
        }
    }
}

impl IntoIterator for PruneFlags {
    type Item = PruneFlag;
    type IntoIter = PruneFlagsIter;

    fn into_iter(self) -> PruneFlagsIter {
        self.iter()
    }
}

/// Lists the named flags, then any unnamed bits in hex: `{Valid | Locked, 0x80}`.
impl fmt::Debug for PruneFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PruneFlags {")?;
        let mut sep = "";
        for flag in self.iter() {
            write!(f, "{sep}{flag:?}")?;
            sep = " | ";
        }
        if self.unnamed_bits() != 0 {
            let sep = if sep.is_empty() { "" } else { ", " };
            write!(f, "{sep}{:#x}", self.unnamed_bits())?;
        }
        f.write_str("}")
    }
}

/// The named flags of a [`PruneFlags`] set, in ascending bit order
/// ([`PruneFlags::iter`]).
///
/// Handwritten
#[derive(Clone, Debug)]
pub struct PruneFlagsIter {
    set: PruneFlags,
    /// The index into [`PruneFlag::ALL`] of the next flag to test.
    next: usize,
}

impl Iterator for PruneFlagsIter {
    type Item = PruneFlag;

    fn next(&mut self) -> Option<PruneFlag> {
        while let Some(&flag) = PruneFlag::ALL.get(self.next) {
            self.next += 1;
            if self.set.contains(flag) {
                return Some(flag);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(PruneFlag::ALL.len().saturating_sub(self.next)))
    }
}

impl core::iter::FusedIterator for PruneFlagsIter {}

define_ctype!(
    /// Options for pruning a worktree (`git_worktree_prune_options`): the structure
    /// version, and the [`PruneFlags`] that relax `git_worktree_is_prunable`'s checks
    /// or widen what `git_worktree_prune` removes.
    ///
    /// A plain value of 8 bytes, resource-free: `Copy`, held inline, never released.
    /// Build one with [`new`](Self::new) (or `Default`), then read and write its fields
    /// through [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut):
    ///
    /// ```
    /// use libgit2::libgit2::repository::worktree::{PruneFlag, PruneOptions};
    ///
    /// let mut opts = PruneOptions::new();
    /// opts.as_mut().set_flags(PruneFlag::Valid | PruneFlag::WorkingTree);
    /// assert_eq!(opts.as_ref().version(), PruneOptions::VERSION);
    /// assert!(opts.as_ref().flags().contains(PruneFlag::Valid));
    /// ```
    ///
    /// A handle borrows the value, so it cannot outlive it:
    ///
    /// ```compile_fail,E0597
    /// use libgit2::libgit2::repository::worktree::PruneOptions;
    ///
    /// let handle = {
    ///     let opts = PruneOptions::new();
    ///     opts.as_ref()
    /// };
    /// let _ = handle.flags();
    /// ```
    ///
    /// nor coexist with an exclusive one:
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::repository::worktree::{PruneFlags, PruneOptions};
    ///
    /// let mut opts = PruneOptions::new();
    /// let mut m = opts.as_mut();
    /// let r = opts.as_ref();
    /// m.set_flags(PruneFlags::empty());
    /// let _ = r.flags();
    /// ```
    ///
    /// and the shared handle only reads:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::repository::worktree::{PruneFlags, PruneOptions};
    ///
    /// let opts = PruneOptions::new();
    /// opts.as_ref().set_flags(PruneFlags::empty());
    /// ```
    ///
    /// Wraps: git_worktree_prune_options
    #[derive(Clone, Copy)]
    PruneOptions, PruneOptionsRef, PruneOptionsMut, ffi::git_worktree_prune_options
);

// SAFETY: a `PruneOptions` is 8 plain bytes: no pointer, no interior mutability, no
// thread affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for PruneOptions {}
// SAFETY: `PruneOptionsMut` is an exclusive borrow of such a value, as
// `&mut PruneOptions`.
unsafe impl Send for PruneOptionsMut<'_> {}

// SAFETY: shared access only reads the fields (every write goes through
// `PruneOptionsMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for PruneOptions {}
// SAFETY: as above; `PruneOptionsRef` is a shared borrow, as `&PruneOptions`.
unsafe impl Send for PruneOptionsRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for PruneOptionsRef<'_> {}
// SAFETY: `&PruneOptionsMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for PruneOptionsMut<'_> {}

impl PruneOptions {
    /// The structure version this crate's header declares
    /// (`GIT_WORKTREE_PRUNE_OPTIONS_VERSION`), the only one libgit2 accepts.
    pub const VERSION: u32 = ffi::GIT_WORKTREE_PRUNE_OPTIONS_VERSION;

    /// The default options, as `git_worktree_prune_options_init` fills them for
    /// [`VERSION`](Self::VERSION) and `GIT_WORKTREE_PRUNE_OPTIONS_INIT` spells them:
    /// the current version and no flags, so every check applies and only the
    /// worktree's gitdir is removed. Needs no initialised libgit2.
    ///
    /// Wraps: git_worktree_prune_options_init
    pub fn new() -> PruneOptions {
        let mut raw = ffi::git_worktree_prune_options::default();
        // SAFETY: `raw` is a live `git_worktree_prune_options` the call may write. With
        // the header's own version the routine's version check passes, so it only
        // copies the 8-byte `GIT_WORKTREE_PRUNE_OPTIONS_INIT` template into `raw`:
        // no allocation and no error state, so no initialised runtime is needed.
        let rc = unsafe { ffi::git_worktree_prune_options_init(&mut raw, Self::VERSION) };
        assert_eq!(
            rc, 0,
            "git_worktree_prune_options_init refused its own version"
        );
        PruneOptions(raw, PhantomData)
    }
}

impl Default for PruneOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The `git_worktree_prune_options` value as it is, for code that already holds one by
/// value.
impl From<ffi::git_worktree_prune_options> for PruneOptions {
    fn from(raw: ffi::git_worktree_prune_options) -> PruneOptions {
        PruneOptions(raw, PhantomData)
    }
}

/// The `git_worktree_prune_options` value, for code that passes one by value.
impl From<PruneOptions> for ffi::git_worktree_prune_options {
    fn from(opts: PruneOptions) -> ffi::git_worktree_prune_options {
        opts.0
    }
}

impl PruneOptionsRef<'_> {
    /// The structure version. [`PruneOptions::new`] sets
    /// [`PruneOptions::VERSION`]; libgit2 refuses the options with
    /// `GIT_ERROR_INVALID` if it is 0 or above that.
    ///
    /// Field: git_worktree_prune_options.version
    pub fn version(&self) -> u32 {
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_worktree_prune_options`; any value is a valid `u32`.
        unsafe { addr_of!((*self.as_ptr()).version).read() }
    }

    /// The prune flags, every stored bit kept.
    ///
    /// Field: git_worktree_prune_options.flags
    pub fn flags(&self) -> PruneFlags {
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_worktree_prune_options`; any value is a valid `u32`.
        let bits = unsafe { addr_of!((*self.as_ptr()).flags).read() };
        PruneFlags::from_bits_retain(bits)
    }

    /// A copy of the options, as a value of its own.
    pub fn to_options(&self) -> PruneOptions {
        PruneOptions::from(ffi::git_worktree_prune_options {
            version: self.version(),
            flags: self.flags().bits(),
        })
    }
}

impl PruneOptionsMut<'_> {
    /// Replaces the structure version. Any value is accepted; libgit2 refuses
    /// options whose version is 0 or above [`PruneOptions::VERSION`].
    ///
    /// Field: git_worktree_prune_options.version
    pub fn set_version(&mut self, version: u32) {
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_worktree_prune_options`; the field is a plain integer.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).version).write(version) }
    }

    /// Replaces the prune flags, every bit as given.
    ///
    /// Field: git_worktree_prune_options.flags
    pub fn set_flags(&mut self, flags: PruneFlags) {
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_worktree_prune_options`; the field is a plain integer.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).flags).write(flags.bits()) }
    }
}

impl fmt::Debug for PruneOptionsRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PruneOptions")
            .field("version", &self.version())
            .field("flags", &self.flags())
            .finish()
    }
}

impl fmt::Debug for PruneOptionsMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for PruneOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'b> PartialEq<PruneOptionsRef<'b>> for PruneOptionsRef<'_> {
    /// Field by field, every flag bit included.
    fn eq(&self, other: &PruneOptionsRef<'b>) -> bool {
        self.version() == other.version() && self.flags() == other.flags()
    }
}

impl Eq for PruneOptionsRef<'_> {}

impl Hash for PruneOptionsRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.version().hash(state);
        self.flags().hash(state);
    }
}

impl PartialEq for PruneOptions {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for PruneOptions {}

impl Hash for PruneOptions {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

// A `PruneOptions` is a `git_worktree_prune_options`, so C reads a value Rust holds in
// place.
const _: () = assert!(
    size_of::<PruneOptions>() == size_of::<ffi::git_worktree_prune_options>()
        && align_of::<PruneOptions>() == align_of::<ffi::git_worktree_prune_options>()
);

define_ctype!(
    /// A linked working tree of a repository (`git_worktree`): its name, its working
    /// directory, and the paths that tie it to the repository it shares objects and
    /// references with.
    ///
    /// Only ever behind a pointer (see the [module docs](self#worktrees)): owned as a
    /// [`WorktreeBox`], read through a [`WorktreeRef`], and written (locked or
    /// unlocked) through a [`WorktreeMut`]. There is no bare value: libgit2 alone
    /// builds a worktree, and the header does not publish its layout.
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::repository::worktree::Worktree;
    ///
    /// let _ = <Worktree as ffibox::CZeroable>::zeroed();
    /// ```
    ///
    /// Wraps: git_worktree
    Worktree, WorktreeRef, WorktreeMut, ffi::git_worktree
);

// SAFETY: a worktree is a struct and six strings, all allocations of libgit2's
// allocator that the struct owns exclusively; `git__free` releases them on any thread,
// and no libgit2 routine keeps thread-local state about a worktree or records its
// address, so moving the owner moves sole ownership of them.
unsafe impl Send for Worktree {}
// SAFETY: `WorktreeMut` is an exclusive borrow of such a worktree, as `&mut Worktree`.
unsafe impl Send for WorktreeMut<'_> {}

// SAFETY: libgit2 writes a worktree only in `git_worktree_lock` and
// `git_worktree_unlock` (its `locked` bit), whose wrappers take a `WorktreeMut`, which
// excludes every other handle. Every other routine taking one (`git_worktree_name`,
// `_path`, `_validate`, `_is_locked`, `_is_prunable`, `_prune`,
// `git_repository_open_from_worktree`) only reads the struct and its strings, so
// shared handles race nothing in memory.
unsafe impl Sync for Worktree {}
// SAFETY: as above; `WorktreeRef` is a shared borrow, as `&Worktree`.
unsafe impl Send for WorktreeRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for WorktreeRef<'_> {}
// SAFETY: `&WorktreeMut` only reaches the reads, through `as_ref`.
unsafe impl Sync for WorktreeMut<'_> {}

/// Releases a worktree through `git_worktree_free`, which hands its six strings, then
/// the struct, to `git__free`.
///
/// Wraps: git_worktree_free
#[derive(Clone, Copy, Debug, Default)]
pub struct WorktreeFree;

impl_cdrop!(WorktreeFree, Worktree, ffi::git_worktree_free);

/// An owned [`Worktree`], freed by `git_worktree_free` on drop.
///
/// Not `Clone`: libgit2 cannot copy a worktree. Look it up again for a second owner.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::repository::worktree::WorktreeBox;
///
/// fn copyable<T: Clone>() {}
/// copyable::<WorktreeBox>();
/// ```
///
/// Its handles borrow it, so none outlives it:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::repository::worktree::{WorktreeBox, WorktreeRef};
///
/// fn escape(w: WorktreeBox) -> WorktreeRef<'static> {
///     let r = w.as_ref();
///     drop(w);
///     r
/// }
/// ```
///
/// and an exclusive handle excludes every other:
///
/// ```compile_fail,E0499
/// use libgit2::libgit2::repository::worktree::WorktreeBox;
///
/// fn alias(w: &mut WorktreeBox) {
///     let a = w.as_mut();
///     let b = w.as_mut();
///     drop((a, b));
/// }
/// ```
///
/// ```compile_fail,E0502
/// use libgit2::libgit2::repository::worktree::WorktreeBox;
///
/// fn alias(w: &mut WorktreeBox) {
///     let shared = w.as_ref();
///     let exclusive = w.as_mut();
///     drop((shared, exclusive));
/// }
/// ```
pub type WorktreeBox = CBox<Worktree, WorktreeFree>;

/// Options for creating a linked worktree (`git_worktree_add_options`), held by value in
/// Rust storage.
///
/// Built by [`new`](Self::new) (or [`Default`]) with libgit2's defaults: the worktree is
/// not locked, its `HEAD` is a new branch named after it at the repository's `HEAD`, and
/// it is checked out with the default
/// [`CheckoutOptions`](crate::libgit2::workdir::checkout::CheckoutOptions). Read it through
/// [`as_ref`](Self::as_ref) and change it through [`as_mut`](Self::as_mut):
///
/// ```
/// use libgit2::libgit2::repository::worktree::AddOptions;
///
/// # if cfg!(miri) { return; }
/// let mut opts = AddOptions::new();
/// opts.as_mut().set_lock(true);
/// opts.as_mut().checkout_options().set_disable_filters(true);
/// assert_eq!(opts.as_ref().version(), AddOptions::VERSION);
/// assert!(opts.as_ref().lock() && !opts.as_ref().checkout_existing());
/// assert!(opts.as_ref().reference().is_none());
/// assert!(opts.as_ref().checkout_options().disable_filters());
/// ```
///
/// `#[repr(transparent)]` over `git_worktree_add_options`, so a pointer to it is a
/// pointer to the C struct, which C reads in place. `'o` is the lifetime of what it
/// points at: the branch reference and everything the embedded checkout options borrow.
/// The options therefore outlive neither the reference:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::refs::refs::ReferenceBox;
/// use libgit2::libgit2::repository::worktree::AddOptions;
///
/// fn escape(branch: ReferenceBox<'_>) {
///     let mut opts = AddOptions::new();
///     opts.as_mut().set_reference(Some(branch.as_ref()));
///     drop(branch);
///     let _ = opts.as_ref().reference();
/// }
/// ```
///
/// nor the repository the reference was read through:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::refs::refs::ReferenceBox;
/// use libgit2::libgit2::repository::repository::RepositoryBox;
/// use libgit2::libgit2::repository::worktree::AddOptions;
///
/// fn escape(repo: RepositoryBox, lookup: fn(&RepositoryBox) -> ReferenceBox<'_>) {
///     let branch = lookup(&repo);
///     let mut opts = AddOptions::new();
///     opts.as_mut().set_reference(Some(branch.as_ref()));
///     drop(repo);
///     let _ = opts.as_ref().reference();
/// }
/// ```
///
/// and no exclusive handle to the reference exists while the options borrow it:
///
/// ```compile_fail,E0502
/// use libgit2::libgit2::refs::refs::ReferenceBox;
/// use libgit2::libgit2::repository::worktree::AddOptions;
///
/// fn alias(mut branch: ReferenceBox<'_>) {
///     let mut opts = AddOptions::new();
///     opts.as_mut().set_reference(Some(branch.as_ref()));
///     let _exclusive = branch.as_mut();
///     let _ = opts.as_ref().reference();
/// }
/// ```
///
/// The exclusive handle is invariant in `'o`, so one cannot be shrunk to accept a
/// shorter-lived borrow than its options were built for:
///
/// ```compile_fail
/// use libgit2::libgit2::repository::worktree::AddOptionsMut;
///
/// fn shrink<'a, 'o>(m: AddOptionsMut<'a, 'static>) -> AddOptionsMut<'a, 'o> {
///     m
/// }
/// ```
///
/// Neither `Send` nor `Sync`, as neither its reference (which reaches its repository)
/// nor its checkout options (whose raw seams carry payloads) are:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::repository::worktree::AddOptions;
///
/// fn send<T: Send>(_: T) {}
/// send(AddOptions::new());
/// ```
///
/// Not `Clone`, as its checkout options are not: their callbacks' payloads may be
/// exclusive borrows, which a copy would alias. Build another with [`new`](Self::new).
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::repository::worktree::AddOptions;
///
/// fn needs<T: Clone>() {}
/// needs::<AddOptions<'static>>();
/// ```
///
/// Wraps: git_worktree_add_options
/// Handwritten
#[repr(transparent)]
pub struct AddOptions<'o>(
    ffi::git_worktree_add_options,
    PhantomData<(*const (), &'o CStr)>,
);

/// Shared borrow of an [`AddOptions`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct AddOptionsRef<'a, 'o>(CBorrowedPtr<'a, AddOptions<'o>>);

/// Exclusive borrow of an [`AddOptions`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct AddOptionsMut<'a, 'o>(
    AddOptionsRef<'a, 'o>,
    // Invariant in the options, and so in `'o`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut AddOptions<'o>>,
);

// SAFETY: `AddOptions` is `#[repr(transparent)]` over `git_worktree_add_options` with a
// zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, AddOptions<'o>>` (`AddOptionsMut` through `AddOptionsRef`) with
// zero-sized markers and no `Drop`, hand out no reference to the options, and
// `AddOptionsRef` has no write operation. `AddOptionsMut` is invariant in the options
// through its `PhantomData<&'a mut AddOptions<'o>>`.
unsafe impl<'o> CCell for AddOptions<'o> {
    type C = ffi::git_worktree_add_options;
    type Ref<'a>
        = AddOptionsRef<'a, 'o>
    where
        Self: 'a;
    type Mut<'a>
        = AddOptionsMut<'a, 'o>
    where
        Self: 'a;
}

impl<'o> AddOptions<'o> {
    /// The structure version this crate's header declares
    /// (`GIT_WORKTREE_ADD_OPTIONS_VERSION`), the only one libgit2 accepts.
    pub const VERSION: u32 = ffi::GIT_WORKTREE_ADD_OPTIONS_VERSION;

    /// libgit2's default options, as `git_worktree_add_options_init` fills them for
    /// [`VERSION`](Self::VERSION) and `GIT_WORKTREE_ADD_OPTIONS_INIT` spells them: not
    /// locked, no existing branch checked out, no reference, and the default checkout
    /// options.
    ///
    /// `git_worktree_add_options_init` only fails for a version it does not know, and
    /// with its own it reaches no global, no allocator and no thread state, so this needs
    /// no initialised library.
    ///
    /// Wraps: git_worktree_add_options_init
    pub fn new() -> Self {
        let mut raw = MaybeUninit::<ffi::git_worktree_add_options>::uninit();
        // SAFETY: `raw` is writable storage for one `git_worktree_add_options`, which
        // the routine overwrites whole with the template for this version (a memcpy);
        // nothing else reads it during the call.
        let rc = unsafe { ffi::git_worktree_add_options_init(raw.as_mut_ptr(), Self::VERSION) };
        assert_eq!(
            rc, 0,
            "git_worktree_add_options_init refused its own version"
        );
        // SAFETY: a 0 return means the template was copied in: every field is
        // initialised, the reference NULL and the checkout options their own template.
        Self(unsafe { raw.assume_init() }, PhantomData)
    }

    /// Shared handle to the options — the getters, and the pointer for C calls that
    /// read them.
    pub fn as_ref(&self) -> AddOptionsRef<'_, 'o> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        AddOptionsRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the options — the setters.
    pub fn as_mut(&mut self) -> AddOptionsMut<'_, 'o> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        AddOptionsMut(AddOptionsRef(ptr), PhantomData)
    }
}

impl Default for AddOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AddOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'o> AddOptionsRef<'a, 'o> {
    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_worktree_add_options *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_worktree_add_options {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The structure's version, `GIT_WORKTREE_ADD_OPTIONS_VERSION`; fixed by
    /// [`AddOptions::new`].
    ///
    /// Field: git_worktree_add_options.version
    pub fn version(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options; any value
        // is a valid `u32`.
        unsafe { addr_of!((*p).version).read() }
    }

    /// Whether the new worktree is created locked: `git_worktree_add` then leaves an
    /// empty `locked` file (no reason) in its administrative directory.
    ///
    /// Field: git_worktree_add_options.lock
    pub fn lock(&self) -> bool {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        let lock = unsafe { addr_of!((*p).lock).read() };
        lock != 0
    }

    /// Whether, when no [`reference`](Self::reference) is set, an existing local branch
    /// named after the worktree is checked out rather than a new one created (which
    /// then fails, the branch existing).
    ///
    /// Field: git_worktree_add_options.checkout_existing
    pub fn checkout_existing(&self) -> bool {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        let existing = unsafe { addr_of!((*p).checkout_existing).read() };
        existing != 0
    }

    /// The branch the new worktree's `HEAD` points at; `None` means one chosen by
    /// [`checkout_existing`](Self::checkout_existing) or a new branch named after the
    /// worktree. `git_worktree_add` refuses a reference that is not a local branch
    /// (`refs/heads/...`) or that another working tree has checked out.
    ///
    /// Field: git_worktree_add_options.ref
    pub fn reference(&self) -> Option<ReferenceRef<'o, 'o>> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        let reference = unsafe { addr_of!((*p).ref_).read() };
        // SAFETY: the field is NULL (as `new` leaves it) or was stored by
        // `set_reference` from a `ReferenceRef<'o, 'o>`: an attached reference that no
        // one frees or writes during `'o`, read through a repository that outlives
        // `'o`, which the options themselves cannot outlive.
        unsafe { ReferenceRef::from_ptr(reference) }
    }

    /// The options of the checkout of the new worktree's `HEAD`, which libgit2 copies
    /// into `git_checkout_head` on the new worktree.
    ///
    /// Field: git_worktree_add_options.checkout_options
    pub fn checkout_options(&self) -> CheckoutOptionsRef<'a, 'o> {
        let p = self.as_ptr().cast_mut();
        // SAFETY: the projection stays inside the live options this handle borrows.
        let checkout = unsafe { addr_of_mut!((*p).checkout_options) };
        // SAFETY: an embedded, initialised `git_checkout_options` that lives as long as
        // the options, which the shared handle keeps borrowed (and so unwritten) for
        // `'a`; every borrow its setters stored outlives `'o`, as
        // `CheckoutOptionsMut::from_ptr` requires of what reaches them through
        // `AddOptionsMut::checkout_options`, and `CheckoutOptions::new`'s template
        // stores none. The pointer is not NULL.
        unsafe { CheckoutOptionsRef::from_ptr(checkout) }
            .expect("embedded checkout options are never NULL")
    }
}

impl fmt::Debug for AddOptionsRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AddOptions")
            .field("version", &self.version())
            .field("lock", &self.lock())
            .field("checkout_existing", &self.checkout_existing())
            .field("reference", &self.reference())
            .field("checkout_options", &self.checkout_options())
            .finish()
    }
}

impl<'a, 'o> AddOptionsMut<'a, 'o> {
    /// Writable pointer to the C struct, for the setters.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_worktree_add_options {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> AddOptionsRef<'_, 'o> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> AddOptionsMut<'_, 'o> {
        AddOptionsMut(self.0, PhantomData)
    }

    /// Sets whether the new worktree is created locked.
    ///
    /// Field: git_worktree_add_options.lock
    pub fn set_lock(&mut self, lock: bool) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).lock).write(c_int::from(lock)) }
    }

    /// Sets whether an existing local branch named after the worktree is checked out
    /// when no reference is set.
    ///
    /// Field: git_worktree_add_options.checkout_existing
    pub fn set_checkout_existing(&mut self, existing: bool) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).checkout_existing).write(c_int::from(existing)) }
    }

    /// Sets the branch the new worktree's `HEAD` points at; `None` means none.
    /// `git_worktree_add` only reads it, copying it for the call.
    ///
    /// The reference is borrowed for `'o`, and through it the repository it was read
    /// from, which then both outlive the options; its owner lends no exclusive handle
    /// meanwhile. Only an attached [`Reference`](crate::libgit2::refs::refs::Reference)
    /// is accepted: libgit2 copies the reference's database link and reaches its
    /// repository, which an
    /// [`UnattachedReference`](crate::libgit2::refs::refs::UnattachedReference) lacks.
    /// The reference may belong to another repository than the one the worktree is
    /// added to; libgit2 then only takes its name.
    ///
    /// Field: git_worktree_add_options.ref
    pub fn set_reference(&mut self, reference: Option<ReferenceRef<'o, 'o>>) {
        let reference = reference.map_or(ptr::null_mut(), |r| r.as_ptr().cast_mut());
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop. The reference is NULL or
        // one a `ReferenceRef<'o, 'o>` vouches for, as `reference()` requires; libgit2
        // never writes through it.
        unsafe { addr_of_mut!((*p).ref_).write(reference) }
    }

    /// The exclusive handle to the embedded checkout options, which takes borrows for
    /// the same `'o` as these options.
    ///
    /// Field: git_worktree_add_options.checkout_options
    pub fn checkout_options(&mut self) -> CheckoutOptionsMut<'_, 'o> {
        let p = self.as_mut_ptr();
        // SAFETY: the projection stays inside the live options this handle borrows.
        let checkout = unsafe { addr_of_mut!((*p).checkout_options) };
        // SAFETY: an embedded, initialised `git_checkout_options` that lives as long as
        // the options, reborrowed exclusively from this handle, so nothing else reaches
        // it while the result lives; the result is invariant in `'o`, so its setters
        // only store borrows that outlive these options. The pointer is not NULL.
        unsafe { CheckoutOptionsMut::from_ptr(checkout) }
            .expect("embedded checkout options are never NULL")
    }
}

impl fmt::Debug for AddOptionsMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// An `AddOptions` is a `git_worktree_add_options`, so C reads a value Rust holds in place.
const _: () = assert!(
    size_of::<AddOptions<'_>>() == size_of::<ffi::git_worktree_add_options>()
        && align_of::<AddOptions<'_>>() == align_of::<ffi::git_worktree_add_options>()
);

#[cfg(test)]
mod unit_tests {
    mod prune_options {
        use super::super::*;

        #[test]
        fn named_bits_are_the_three_flags() {
            assert_eq!(PruneFlags::NAMED, 0b111);
            assert_eq!(PruneFlag::ALL.map(PruneFlag::bit), [1, 2, 4]);
        }

        #[test]
        fn flags_debug_lists_named_then_unnamed_bits() {
            let cases = [
                (0, "PruneFlags {}"),
                (0b101, "PruneFlags {Valid | WorkingTree}"),
                (0x80, "PruneFlags {0x80}"),
                (0x8000_0002, "PruneFlags {Locked, 0x80000000}"),
                (
                    u32::MAX,
                    "PruneFlags {Valid | Locked | WorkingTree, 0xfffffff8}",
                ),
            ];
            for (bits, debug) in cases {
                assert_eq!(format!("{:?}", PruneFlags::from_bits_retain(bits)), debug);
            }
        }

        /// A sink that takes `.0` more bytes, then fails.
        struct Budget(usize);

        impl fmt::Write for Budget {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                self.0 = self.0.checked_sub(s.len()).ok_or(fmt::Error)?;
                Ok(())
            }
        }

        #[test]
        fn flags_debug_passes_on_a_failing_sink() {
            use fmt::Write as _;
            // Every piece: the prefix, both named flags, the unnamed bits, the suffix.
            let set = PruneFlags::from_bits_retain(0b011 | 1 << 8);
            let full = format!("{set:?}").len();
            for budget in 0..full {
                let written = write!(Budget(budget), "{set:?}");
                assert_eq!(written, Err(fmt::Error), "{budget}");
            }
            assert_eq!(write!(Budget(full), "{set:?}"), Ok(()));
        }

        #[test]
        fn unnamed_flag_reports_its_value() {
            let e = PruneFlag::try_from(3).unwrap_err();
            assert_eq!(e.raw(), 3);
            assert_eq!(
                e.to_string(),
                "libgit2 worktree prune flag 0x3 is not named by git2/worktree.h"
            );
            let e: Box<dyn std::error::Error> = Box::new(e);
            assert!(e.source().is_none());
            assert_eq!(PruneFlag::try_from(4), Ok(PruneFlag::WorkingTree));
            assert_eq!(ffi::git_worktree_prune_t::from(PruneFlag::Locked), 2);
        }

        #[test]
        fn iterator_counts_down_and_stays_done() {
            let set = PruneFlags::from_bits_retain(0b101 | 1 << 9);
            let mut it = set.iter();
            assert_eq!(it.size_hint(), (0, Some(3)));
            assert_eq!(it.next(), Some(PruneFlag::Valid));
            assert_eq!(it.size_hint(), (0, Some(2)));
            let rest = it.clone();
            assert_eq!(it.next(), Some(PruneFlag::WorkingTree));
            assert_eq!(it.size_hint(), (0, Some(0)));
            assert_eq!(it.next(), None);
            assert_eq!(it.next(), None);
            assert_eq!(rest.collect::<Vec<_>>(), [PruneFlag::WorkingTree]);
            let mut seen = Vec::new();
            for flag in set {
                seen.push(flag);
            }
            assert_eq!(seen, [PruneFlag::Valid, PruneFlag::WorkingTree]);
        }

        #[test]
        fn set_operators_combine_bits() {
            assert!(PruneFlags::default().is_empty());
            assert_eq!(PruneFlags::default(), PruneFlags::empty());
            let a = PruneFlag::Valid | PruneFlag::Locked;
            assert_eq!(a.bits(), 0b011);
            assert_eq!((a | PruneFlag::WorkingTree).bits(), 0b111);
            assert_eq!((a | PruneFlags::from_bits_retain(1 << 8)).bits(), 0x103);
            let mut b = PruneFlags::empty();
            b |= PruneFlag::WorkingTree;
            b |= PruneFlags::from_bits_retain(1 << 31);
            assert_eq!(b.bits(), 0x8000_0004);
            assert!(!b.is_empty());
            b.extend([PruneFlag::Valid]);
            assert_eq!(b.bits(), 0x8000_0005);
            let c: PruneFlags = [PruneFlag::Locked, PruneFlag::Locked].into_iter().collect();
            assert_eq!(c, PruneFlag::Locked.into());
            assert_eq!(
                PruneFlags::from_bits_retain(1 << 31).unnamed_bits(),
                1 << 31
            );
        }

        #[test]
        fn options_debug_shows_both_fields() {
            let mut opts = PruneOptions::from(ffi::git_worktree_prune_options {
                version: 1,
                flags: 0b001,
            });
            let debug = "PruneOptions { version: 1, flags: PruneFlags {Valid} }";
            assert_eq!(format!("{opts:?}"), debug);
            assert_eq!(format!("{:?}", opts.as_ref()), debug);
            assert_eq!(format!("{:?}", opts.as_mut()), debug);
        }

        #[test]
        fn options_convert_by_value_both_ways() {
            let raw = ffi::git_worktree_prune_options {
                version: 7,
                flags: 0xffff_0001,
            };
            let opts = PruneOptions::from(raw);
            assert_eq!(opts.as_ref().version(), 7);
            assert_eq!(opts.as_ref().flags().bits(), 0xffff_0001);
            let back = ffi::git_worktree_prune_options::from(opts.as_ref().to_options());
            assert_eq!((back.version, back.flags), (7, 0xffff_0001));
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn new_and_default_are_the_init_template() {
            let opts = PruneOptions::new();
            assert_eq!(opts, PruneOptions::default());
            assert_eq!(opts.as_ref().version(), PruneOptions::VERSION);
            assert!(opts.as_ref().flags().is_empty());
        }
    }

    mod worktree {
        use super::super::*;
        use core::ptr;
        use std::ffi::{CStr, CString};
        use std::fs;
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        use crate::libgit2::util::sys::errors::runtime_pinned;

        /// A non-bare repository with a linked worktree `wt` laid out on disk, as
        /// `git_worktree_add` leaves one; the repository is freed and the directory
        /// removed on drop.
        struct Layout {
            base: PathBuf,
            repo: *mut ffi::git_repository,
        }

        impl Layout {
            fn new(tag: &str) -> Self {
                assert!(runtime_pinned());
                let base = std::env::temp_dir().join(format!(
                    "crustify-worktree-unit-{}-{tag}",
                    std::process::id()
                ));
                let _ = fs::remove_dir_all(&base);
                let repo_dir = base.join("repo");
                let wt_dir = base.join("wt");
                let admin = repo_dir.join(".git/worktrees/wt");
                let c_path = CString::new(repo_dir.as_os_str().as_bytes()).unwrap();
                let mut opts = ffi::git_repository_init_options::default();
                // SAFETY: `opts` is a live options struct, filled with the version's
                // defaults.
                let rc = unsafe {
                    ffi::git_repository_init_options_init(
                        &mut opts,
                        ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
                    )
                };
                assert_eq!(rc, 0);
                opts.flags = ffi::GIT_REPOSITORY_INIT_MKPATH;
                let mut repo = ptr::null_mut();
                let path = c_path.as_ptr();
                // SAFETY: a live slot, a NUL-terminated path and live options, all read
                // for the call only.
                let rc = unsafe { ffi::git_repository_init_ext(&mut repo, path, &mut opts) };
                assert_eq!(rc, 0);
                fs::create_dir_all(&admin).unwrap();
                fs::create_dir_all(&wt_dir).unwrap();
                fs::write(admin.join("commondir"), "../..\n").unwrap();
                fs::write(
                    admin.join("gitdir"),
                    format!("{}\n", wt_dir.join(".git").display()),
                )
                .unwrap();
                fs::write(admin.join("HEAD"), "ref: refs/heads/wt\n").unwrap();
                fs::write(
                    wt_dir.join(".git"),
                    format!("gitdir: {}\n", admin.display()),
                )
                .unwrap();
                Self { base, repo }
            }

            /// `git_worktree_lookup(name)`: the raw worktree, or the error code.
            fn lookup(&self, name: &CStr) -> Result<*mut ffi::git_worktree, i32> {
                let mut out = ptr::null_mut();
                // SAFETY: a live slot, the live repository and a NUL-terminated name,
                // only read.
                let rc = unsafe { ffi::git_worktree_lookup(&mut out, self.repo, name.as_ptr()) };
                if rc == 0 { Ok(out) } else { Err(rc) }
            }
        }

        impl Drop for Layout {
            fn drop(&mut self) {
                // SAFETY: the repository this layout opened, freed once.
                unsafe { ffi::git_repository_free(self.repo) };
                let _ = fs::remove_dir_all(&self.base);
            }
        }

        /// The worktree's name, as `git_worktree_name` reads it through the box.
        fn name(w: &WorktreeBox) -> CString {
            // SAFETY: the box's live worktree, only read; its name lives as long as it.
            unsafe { CStr::from_ptr(ffi::git_worktree_name(w.as_c_ptr())) }.to_owned()
        }

        #[test]
        fn box_is_one_pointer_with_a_null_niche() {
            assert_eq!(
                size_of::<WorktreeBox>(),
                size_of::<*mut ffi::git_worktree>()
            );
            assert_eq!(
                size_of::<Option<WorktreeBox>>(),
                size_of::<*mut ffi::git_worktree>()
            );
            // SAFETY: NULL adopts nothing.
            assert!(unsafe { WorktreeBox::from_c(ptr::null_mut()) }.is_none());
        }

        #[test]
        fn owners_and_handles_cross_threads() {
            fn send_sync<T: Send + Sync>() {}
            send_sync::<Worktree>();
            send_sync::<WorktreeBox>();
            send_sync::<WorktreeRef<'static>>();
            send_sync::<WorktreeMut<'static>>();
            send_sync::<WorktreeFree>();
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn adopted_lookup_is_released_by_its_policy() {
            let layout = Layout::new("adopt");
            let raw = layout.lookup(c"wt").unwrap();
            // SAFETY: a fresh worktree from `git_worktree_lookup`, owned by the caller
            // and released by `git_worktree_free`.
            let w = unsafe { WorktreeBox::from_c(raw) }.unwrap();
            assert_eq!(w.as_c_ptr(), raw);
            assert_eq!(w.as_ref().as_ptr().cast::<ffi::git_worktree>(), raw);
            assert_eq!(name(&w), c"wt");
            drop(w);
            // ASan's leak check sees the six strings and the struct released.
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn into_c_surrenders_the_worktree_to_c() {
            let layout = Layout::new("into-c");
            // SAFETY: as above.
            let w = unsafe { WorktreeBox::from_c(layout.lookup(c"wt").unwrap()) }.unwrap();
            let raw = w.into_c();
            // SAFETY: the box gave the worktree up, so it is freed here once.
            unsafe { ffi::git_worktree_free(raw) };
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn worktree_outlives_its_repository() {
            let layout = Layout::new("outlive");
            // SAFETY: as above.
            let w = unsafe { WorktreeBox::from_c(layout.lookup(c"wt").unwrap()) }.unwrap();
            drop(layout);
            // The repository is freed and its directory gone: the worktree holds copies.
            assert_eq!(name(&w), c"wt");
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn failed_lookups_leave_nothing_to_adopt() {
            let layout = Layout::new("missing");
            assert_eq!(layout.lookup(c"absent"), Err(ffi::GIT_ENOTFOUND));
            fs::remove_file(layout.base.join("repo/.git/worktrees/wt/HEAD")).unwrap();
            assert_eq!(layout.lookup(c"wt"), Err(-1));
        }
    }

    mod add_options {
        use super::super::*;
        use crate::libgit2::refs::refs::ReferenceBox;
        use crate::libgit2::util::sys::errors::runtime_pinned;
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        /// `GIT_WORKTREE_ADD_OPTIONS_INIT`, built in Rust so Miri can run the accessors;
        /// `new_matches_the_template` ties it to `git_worktree_add_options_init`.
        fn template<'o>() -> AddOptions<'o> {
            let raw = ffi::git_worktree_add_options {
                version: ffi::GIT_WORKTREE_ADD_OPTIONS_VERSION,
                checkout_options: ffi::git_checkout_options {
                    version: ffi::GIT_CHECKOUT_OPTIONS_VERSION,
                    ..Default::default()
                },
                ..Default::default()
            };
            AddOptions(raw, PhantomData)
        }

        /// Every field, as the getters read them.
        fn fields(o: AddOptionsRef<'_, '_>) -> String {
            format!(
                "{o:?} {:?} {}",
                o.reference().map(|r| r.as_ptr()),
                (o.as_ptr() as usize) % align_of::<ffi::git_worktree_add_options>(),
            )
        }

        #[test]
        fn handles_are_one_pointer_and_the_value_is_the_c_struct() {
            let one = size_of::<*const ffi::git_worktree_add_options>();
            assert_eq!(size_of::<AddOptionsRef<'_, '_>>(), one);
            assert_eq!(size_of::<AddOptionsMut<'_, '_>>(), one);
            assert_eq!(size_of::<Option<AddOptionsRef<'_, '_>>>(), one);
            assert_eq!(
                size_of::<AddOptions<'_>>(),
                size_of::<ffi::git_worktree_add_options>()
            );
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn new_matches_the_template() {
            let opts = AddOptions::new();
            assert_eq!(fields(opts.as_ref()), fields(template().as_ref()));
            assert_eq!(
                fields(AddOptions::default().as_ref()),
                fields(opts.as_ref())
            );
            let raw = &opts.0;
            assert_eq!(raw.version, 1);
            assert_eq!((raw.lock, raw.checkout_existing), (0, 0));
            assert!(raw.ref_.is_null());
            let checkout = &raw.checkout_options;
            assert_eq!(checkout.version, ffi::GIT_CHECKOUT_OPTIONS_VERSION);
            assert_eq!((checkout.checkout_strategy, checkout.dir_mode), (0, 0));
            assert!(checkout.target_directory.is_null() && checkout.paths.strings.is_null());
        }

        #[test]
        fn defaults_read_as_documented() {
            let opts = template();
            let o = opts.as_ref();
            assert_eq!(o.version(), AddOptions::VERSION);
            assert!(!o.lock() && !o.checkout_existing());
            assert!(o.reference().is_none());
            assert_eq!(
                o.checkout_options().version(),
                ffi::GIT_CHECKOUT_OPTIONS_VERSION
            );
            assert_eq!(
                format!("{opts:?}"),
                format!(
                    "AddOptions {{ version: 1, lock: false, checkout_existing: false, \
                     reference: None, checkout_options: {:?} }}",
                    o.checkout_options()
                )
            );
        }

        #[test]
        fn setters_write_the_c_fields() {
            let mut opts = template();
            let mut m = opts.as_mut();
            m.set_lock(true);
            m.as_mut().set_checkout_existing(true);
            assert!(m.as_ref().lock() && m.as_ref().checkout_existing());
            assert_eq!(format!("{m:?}"), format!("{:?}", m.as_ref()));
            assert_eq!((opts.0.lock, opts.0.checkout_existing), (1, 1));
            let mut m = opts.as_mut();
            m.set_lock(false);
            m.set_checkout_existing(false);
            assert_eq!((opts.0.lock, opts.0.checkout_existing), (0, 0));
        }

        #[test]
        fn any_nonzero_c_boolean_reads_true() {
            for (value, expected) in [
                (0, false),
                (1, true),
                (2, true),
                (-1, true),
                (i32::MIN, true),
            ] {
                let mut opts = template();
                opts.0.lock = value;
                opts.0.checkout_existing = value;
                assert_eq!(opts.as_ref().lock(), expected, "{value}");
                assert_eq!(opts.as_ref().checkout_existing(), expected, "{value}");
            }
        }

        #[test]
        fn checkout_options_project_into_the_embedded_struct() {
            let label = CString::new("mine").unwrap();
            let mut opts = template();
            {
                let mut m = opts.as_mut();
                let mut checkout = m.checkout_options();
                checkout.set_strategy(0x20);
                checkout.set_our_label(Some(&label));
                checkout.as_mut().set_dir_mode(0o700);
                m.set_lock(true);
            }
            let raw = &opts.0.checkout_options;
            assert_eq!((raw.checkout_strategy, raw.dir_mode), (0x20, 0o700));
            assert_eq!(raw.our_label, label.as_ptr());
            let o = opts.as_ref();
            let checkout = o.checkout_options();
            assert_eq!(checkout.strategy(), 0x20);
            assert_eq!(checkout.our_label(), Some(label.as_c_str()));
            assert_eq!(
                checkout.as_ptr(),
                addr_of!(opts.0.checkout_options),
                "the handle addresses the embedded struct, not a copy"
            );
            assert!(o.lock());
        }

        /// A repository holding the loose branch `refs/heads/main`, freed and removed
        /// on drop; the reference wrappers have no lookup yet.
        struct Repo {
            path: std::path::PathBuf,
            repo: *mut ffi::git_repository,
        }

        impl Repo {
            fn new(tag: &str) -> Self {
                assert!(runtime_pinned());
                let path = std::env::temp_dir().join(format!(
                    "crustify-worktree-add-unit-{}-{tag}",
                    std::process::id()
                ));
                let _ = std::fs::remove_dir_all(&path);
                let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
                let mut repo = ptr::null_mut();
                // SAFETY: a live slot and a NUL-terminated path, read for the call.
                let rc = unsafe { ffi::git_repository_init(&mut repo, c_path.as_ptr(), 1) };
                assert_eq!(rc, 0);
                std::fs::write(
                    path.join("refs/heads/main"),
                    format!("{}\n", "11".repeat(20)),
                )
                .unwrap();
                Self { path, repo }
            }

            /// `git_reference_lookup(name)`, adopted.
            fn lookup(&self, name: &CStr) -> ReferenceBox<'_> {
                let mut out = ptr::null_mut();
                // SAFETY: a live slot, the live repository and a NUL-terminated name.
                let rc = unsafe { ffi::git_reference_lookup(&mut out, self.repo, name.as_ptr()) };
                assert_eq!(rc, 0);
                // SAFETY: a fresh attached reference the caller owns, released by
                // `git_reference_free`; it borrows `self`, which frees the repository.
                unsafe { ReferenceBox::from_c(out) }.unwrap()
            }
        }

        impl Drop for Repo {
            fn drop(&mut self) {
                // SAFETY: the repository this fixture opened, freed once.
                unsafe { ffi::git_repository_free(self.repo) };
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn reference_is_stored_and_read_back_as_borrowed() {
            let repo = Repo::new("ref");
            let main = repo.lookup(c"refs/heads/main");
            let mut opts = AddOptions::new();
            opts.as_mut().set_reference(Some(main.as_ref()));
            assert_eq!(opts.0.ref_.cast_const(), main.as_ref().as_ptr());
            let read = opts.as_ref().reference().unwrap();
            assert_eq!(read.as_ptr(), main.as_ref().as_ptr());
            assert!(format!("{opts:?}").contains("reference: Some(Reference { .. })"));
            opts.as_mut().set_reference(None);
            assert!(opts.0.ref_.is_null() && opts.as_ref().reference().is_none());
            // The options never owned it: the box still frees it once.
            drop(main);
        }
    }
}
