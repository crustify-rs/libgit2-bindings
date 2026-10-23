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
//!
//! # Routines
//!
//! [`Worktree::lookup`] and [`Worktree::open_from_repository`] build a worktree from a
//! [`RepositoryRef`], [`Worktree::add`] creates one (with [`AddOptions`]), and [`list`]
//! names a repository's worktrees; a [`WorktreeRef`]
//! reads one ([`name`](WorktreeRef::name), [`path`](WorktreeRef::path),
//! [`validate`](WorktreeRef::validate), [`is_locked`](WorktreeRef::is_locked),
//! [`lock_reason`](WorktreeRef::lock_reason)) and prunes it
//! ([`is_prunable`](WorktreeRef::is_prunable), [`prune`](WorktreeRef::prune)); a
//! [`WorktreeMut`] locks and unlocks it, the only routines that write the struct. Each
//! reports a failure as a [`WorktreeError`].
//!
//! The wrappers keep five libgit2 defects from reaching Rust callers:
//!
//! - `git_worktree_lookup` passes the repository's common directory to `strlen` unchecked, which is
//!   NULL for a repository without paths (`git_repository_new`, `git_repository_wrap_odb`):
//!   [`Worktree::lookup`] refuses such a repository with `GIT_ENOTFOUND`, as libgit2 refuses other
//!   paths it has none for, as does [`Worktree::add`] (`git_worktree_add` would create `worktrees/`
//!   in the process's working directory before tripping over the same NULL), and [`list`] reports
//!   that it has no worktrees, where `git_worktree_list` would list the `worktrees/` directory of
//!   the process's working directory.
//! - `git_worktree_list` removes the entries that are not worktrees while iterating over them, so
//!   the entry after each removed one escapes the check: [`list`] drops those too.
//! - `git_worktree_prune` takes an error from `git_worktree_is_prunable` (such as an unreadable
//!   `locked` file) for "prunable" and deletes the worktree: [`WorktreeRef::prune`] asks first, and
//!   reports that error instead.
//! - Several routines fail without setting an error of their own (`GIT_ELOCKED` from
//!   `git_worktree_lock`, a lock file `git_worktree_unlock` cannot remove, a missing or malformed
//!   worktree in `git_worktree_lookup`), leaving whatever an earlier call set as the thread's last
//!   error: each wrapper clears the error first, and gives such a failure an error that describes
//!   it.
//! - `git_worktree_add` refuses a branch "already checked out" by asking the repository the given
//!   reference was read through, not the repository the worktree is added to: a reference of
//!   another repository (which only lends its name) lets a branch be checked out in two of the
//!   repository's working trees. [`Worktree::add`] passes libgit2 the repository's own reference of
//!   that name instead, when it has one.

use core::ffi::{CStr, c_int};
use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{BitOr, BitOrAssign};
use core::ptr::{self, NonNull, addr_of, addr_of_mut};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use ffibox::{CBorrowedPtr, CBox, CCell, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::core::buffer::{Buf, BufVal};
use crate::libgit2::core::strarray::{StrArray, StrArrayVal};
use crate::libgit2::refs::refs::{ReferenceBox, ReferenceRef};
use crate::libgit2::repository::repository::RepositoryRef;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use crate::libgit2::util::sys::errors::{clear, runtime_pinned, set};
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
    type IntoIter = PruneFlagsIter;
    type Item = PruneFlag;

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
    /// opts.as_mut()
    ///     .set_flags(PruneFlag::Valid | PruneFlag::WorkingTree);
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
    PruneOptions,
    PruneOptionsRef,
    PruneOptionsMut,
    ffi::git_worktree_prune_options
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
    Worktree,
    WorktreeRef,
    WorktreeMut,
    ffi::git_worktree
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
    type Mut<'a>
        = AddOptionsMut<'a, 'o>
    where
        Self: 'a;
    type Ref<'a>
        = AddOptionsRef<'a, 'o>
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

/// A worktree routine's failure: the code it returned, and the error it left as the
/// thread's last libgit2 error, copied out.
///
/// The code tells the failures apart: [`ErrorCode::NotFound`] when there is no
/// worktree of that name, [`ErrorCode::Locked`] when locking a locked worktree,
/// [`ErrorCode::Generic`] for the rest. Where libgit2 fails without an error of its
/// own, the wrapper sets one (see the [module docs](self#routines)).
///
/// Handwritten
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct WorktreeError {
    code: c_int,
    error: ErrorBuf,
}

impl WorktreeError {
    /// The failure a routine reported by returning `rc`, with the thread's last error;
    /// when it is `GIT_ERROR_NONE` (the routine set none), `fallback` is set first.
    fn from_libgit2(rc: c_int, fallback: (ErrorClass, &CStr)) -> Self {
        let mut error = last();
        if error.class() == Some(ErrorClass::None) {
            set(fallback.0, fallback.1);
            error = last();
        }
        Self { code: rc, error }
    }

    /// The failure reported when libgit2 cannot be initialised, without calling it.
    fn unpinned() -> Self {
        Self {
            code: ErrorCode::Generic.raw(),
            error: last(),
        }
    }

    /// The returned code; `None` for one the header does not name
    /// ([`raw_code`](Self::raw_code) has it).
    pub fn code(&self) -> Option<ErrorCode> {
        ErrorCode::from_raw(self.code)
    }

    /// The returned code, always negative.
    pub fn raw_code(&self) -> c_int {
        self.code
    }

    /// The error's class; `None` for a class the header does not name
    /// ([`raw_class`](Self::raw_class) has it).
    pub fn class(&self) -> Option<ErrorClass> {
        self.error.class()
    }

    /// The error's class as libgit2 stored it.
    pub fn raw_class(&self) -> c_int {
        self.error.raw_class()
    }

    /// The error message.
    pub fn message(&self) -> &CStr {
        self.error.message()
    }

    /// The error, without the code.
    pub fn error(&self) -> &ErrorBuf {
        &self.error
    }
}

/// The error, without the code.
impl From<WorktreeError> for ErrorBuf {
    fn from(error: WorktreeError) -> Self {
        error.error
    }
}

impl fmt::Debug for WorktreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("WorktreeError");
        match self.code() {
            Some(code) => s.field("code", &code),
            None => s.field("code", &self.code),
        };
        s.field("error", &self.error).finish()
    }
}

impl fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for WorktreeError {}

/// The error libgit2 gives a path a repository without paths cannot have.
const NO_PATH: (ErrorClass, &CStr) = (ErrorClass::Invalid, c"path cannot exist in repository");

/// Runs the libgit2 routine `call` with the runtime pinned and the thread's error
/// cleared, and reports a negative return as a [`WorktreeError`], whose error is
/// `fallback(rc)` when the routine set none.
fn checked(
    call: impl FnOnce() -> c_int,
    fallback: impl FnOnce(c_int) -> (ErrorClass, &'static CStr),
) -> Result<c_int, WorktreeError> {
    if !runtime_pinned() {
        return Err(WorktreeError::unpinned());
    }
    clear();
    let rc = call();
    if rc < 0 {
        return Err(WorktreeError::from_libgit2(rc, fallback(rc)));
    }
    Ok(rc)
}

/// A failure the wrapper reports itself, with code `rc` and the error `error`, without
/// calling the routine.
fn refused(rc: c_int, error: (ErrorClass, &CStr)) -> WorktreeError {
    if !runtime_pinned() {
        return WorktreeError::unpinned();
    }
    clear();
    WorktreeError::from_libgit2(rc, error)
}

/// The fallback of a routine that sets an error on every failing path.
fn generic(_: c_int) -> (ErrorClass, &'static CStr) {
    (ErrorClass::Worktree, c"worktree operation failed")
}

/// The repository's common directory, which holds its `worktrees/`; `None` for a
/// repository without paths.
fn commondir(repo: RepositoryRef<'_>) -> Option<&CStr> {
    // SAFETY: `repo` is a live repository on this thread; the routine only reads its
    // `commondir` field.
    let dir = unsafe { ffi::git_repository_commondir(repo.as_ptr()) };
    // SAFETY: `commondir` is NULL or a NUL-terminated string the repository owns; it is
    // set when the repository is opened and replaced by no routine, so it lives as long
    // as `repo` is borrowed.
    (!dir.is_null()).then(|| unsafe { CStr::from_ptr(dir) })
}

/// Whether `reference` was read through `repo`: whether `repo` is the repository
/// libgit2 asks about it.
fn owned_by(reference: ReferenceRef<'_, '_>, repo: RepositoryRef<'_>) -> bool {
    // SAFETY: `reference` is a live reference attached to a reference database (the
    // handle's contract); the routine only reads the database's repository pointer.
    let owner = unsafe { ffi::git_reference_owner(reference.as_ptr()) };
    ptr::eq(owner.cast_const(), repo.as_ptr())
}

/// `repo`'s own reference with `reference`'s name, read through `repo`; `None` when
/// `repo` has none, or the lookup fails otherwise (libgit2 then gets the reference as
/// given, and fails on it as it would). The thread's error is left as the lookup set
/// it, for the caller to clear.
///
/// A raw seam over `git_reference_lookup` and `git_reference_name` until the
/// reference routines are wrapped.
fn same_name<'r>(
    repo: RepositoryRef<'r>,
    reference: ReferenceRef<'_, '_>,
) -> Option<ReferenceBox<'r>> {
    // SAFETY: a live reference; its name is a NUL-terminated string it owns, which no
    // one frees or writes while the handle borrows it.
    let name = unsafe { ffi::git_reference_name(reference.as_ptr()) };
    let mut out = ptr::null_mut();
    // SAFETY: `out` is a writable slot; `repo` a live repository on this thread, which
    // the lookup reads (and whose reference database it may open); `name` is
    // NUL-terminated and live for the call.
    let rc = unsafe { ffi::git_reference_lookup(&mut out, repo.as_ptr().cast_mut(), name) };
    if rc < 0 {
        return None;
    }
    // SAFETY: on a 0 return `out` is a fresh reference attached to `repo`'s reference
    // database, owned by the caller alone and released by `git_reference_free`; it
    // reaches `repo`, which the box's `'r` keeps borrowed.
    unsafe { ReferenceBox::from_c(out) }
}

/// Whether `dir` is a worktree's administrative directory, as libgit2 tells one:
/// it holds the files `commondir`, `gitdir` and `HEAD`.
fn is_worktree_dir(dir: &Path) -> bool {
    ["commondir", "gitdir", "HEAD"]
        .iter()
        .all(|file| dir.join(file).is_file())
}

/// The names of `repo`'s linked worktrees: the directories under
/// `<commondir>/worktrees/` that hold a worktree's `commondir`, `gitdir` and `HEAD`
/// files. The order is the directory's.
///
/// Empty when there is no such directory, and for a repository without paths
/// (`git_worktree_list` would list the process's working directory then). Every entry
/// is checked, where libgit2 skips the one after each entry it drops (see the
/// [module docs](self#routines)). Fails with libgit2's error when the directory cannot
/// be read.
///
/// Wraps: git_worktree_list
pub fn list(repo: RepositoryRef<'_>) -> Result<StrArrayVal, WorktreeError> {
    let mut names = StrArray::new();
    let Some(commondir) = commondir(repo) else {
        return Ok(names);
    };
    let array = names.as_mut().as_mut_ptr();
    checked(
        || {
            // SAFETY: `array` is a live, empty array the routine fills with an array
            // and strings of libgit2's allocator, which it then owns, or leaves empty
            // on failure; `repo` is a live repository on this thread, only read.
            unsafe { ffi::git_worktree_list(array, repo.as_ptr().cast_mut()) }
        },
        generic,
    )?;
    let base = Path::new(OsStr::from_bytes(commondir.to_bytes())).join("worktrees");
    let mut strings = names.as_mut().take_strings();
    let slice = strings.as_mut_slice();
    let mut kept = 0;
    for i in 0..slice.len() {
        if is_worktree_dir(&base.join(OsStr::from_bytes(slice[i].as_bytes()))) {
            slice.swap(kept, i);
            kept += 1;
        }
    }
    let mut array = names.as_mut();
    array.set_strings(strings);
    array.truncate(kept);
    Ok(names)
}

impl Worktree {
    /// The linked worktree `name` of `repo`: the one whose administrative directory is
    /// `<commondir>/worktrees/<name>`.
    ///
    /// Fails with [`ErrorCode::NotFound`] when there is no such directory (or `repo`
    /// has no paths), and with [`ErrorCode::Generic`] when it is not a worktree's (it
    /// lacks its `commondir`, `gitdir` or `HEAD` file) or a link in it cannot be read.
    ///
    /// Wraps: git_worktree_lookup
    pub fn lookup(repo: RepositoryRef<'_>, name: &CStr) -> Result<WorktreeBox, WorktreeError> {
        if commondir(repo).is_none() {
            return Err(refused(ffi::GIT_ENOTFOUND, NO_PATH));
        }
        let mut out = ptr::null_mut();
        checked(
            || {
                // SAFETY: `out` is a writable slot, which the routine sets to NULL first;
                // `repo` is a live repository on this thread with a common directory,
                // only read; `name` is NUL-terminated, copied and not retained.
                unsafe {
                    ffi::git_worktree_lookup(&mut out, repo.as_ptr().cast_mut(), name.as_ptr())
                }
            },
            |rc| match rc {
                ffi::GIT_ENOTFOUND => (ErrorClass::Worktree, c"worktree does not exist"),
                _ => (ErrorClass::Worktree, c"not a valid worktree directory"),
            },
        )?;
        // SAFETY: on success `out` is a fresh worktree whose strings are all copies, owned
        // by the caller alone and released by `git_worktree_free`.
        let worktree = unsafe { WorktreeBox::from_c(out) };
        Ok(worktree.expect("git_worktree_lookup succeeded without a worktree"))
    }

    /// The linked worktree `repo` was opened from: `repo` must have been opened at a
    /// worktree's working directory (or its administrative directory).
    ///
    /// Fails with `GIT_ERROR_WORKTREE` "cannot open worktree of a non-worktree repo"
    /// otherwise, and with libgit2's error when the worktree's files cannot be read.
    ///
    /// Wraps: git_worktree_open_from_repository
    pub fn open_from_repository(repo: RepositoryRef<'_>) -> Result<WorktreeBox, WorktreeError> {
        let mut out = ptr::null_mut();
        checked(
            || {
                // SAFETY: `out` is a writable slot, NULL so that a failure, which leaves
                // it untouched, leaves it NULL; `repo` is a live repository on this
                // thread, only read.
                unsafe {
                    ffi::git_worktree_open_from_repository(&mut out, repo.as_ptr().cast_mut())
                }
            },
            |_| (ErrorClass::Worktree, c"not a valid worktree directory"),
        )?;
        // SAFETY: on success `out` is a fresh worktree whose strings are all copies, owned
        // by the caller alone and released by `git_worktree_free`.
        let worktree = unsafe { WorktreeBox::from_c(out) };
        Ok(worktree.expect("git_worktree_open_from_repository succeeded without a worktree"))
    }

    /// Adds the linked worktree `name` to `repo`, checked out at the new directory
    /// `path`, and returns it, as `git worktree add` does: libgit2 creates the
    /// administrative directory `<commondir>/worktrees/<name>/` and its files, the
    /// working directory `path` with its `.git` link, and the worktree's own `HEAD`,
    /// which points at the branch `opts` names, then checks that branch out into
    /// `path` with the options' checkout options. `None` takes
    /// [`AddOptions::new`]'s defaults.
    ///
    /// The branch is [`reference`](AddOptionsRef::reference) when one is set (a local
    /// branch that no other working tree has checked out; one of another repository
    /// lends only its name, and the branch of that name in `repo` is what libgit2
    /// then checks and checks out: `git_worktree_add` itself would ask the other
    /// repository's working trees whether the branch is checked out, and let one of
    /// `repo`'s working trees check it out twice; when `repo` has no branch of that
    /// name, libgit2 fails as it would, after the worktree's files are written);
    /// otherwise the local branch `name` when
    /// [`checkout_existing`](AddOptionsRef::checkout_existing) is set and it exists;
    /// otherwise a new branch `name`, created at `repo`'s `HEAD` with a reflog entry
    /// (which fails with [`ErrorCode::Exists`] when it exists already, with
    /// [`ErrorCode::InvalidSpec`] when `name` is not a valid branch name, and with
    /// [`ErrorCode::UnbornBranch`] when `HEAD` is unborn). A branch that is not local,
    /// or that a working tree has checked out, fails with `GIT_ERROR_WORKTREE`; but
    /// libgit2's check of the working trees can let the add through once `worktrees/`
    /// holds directories that are not worktrees' (as failed adds leave, below):
    /// `git_worktree_list` may keep one of them (see [`list`]), and the check stops at
    /// the first it cannot open.
    ///
    /// Only the last component of `path` is created, and of the administrative
    /// directory (and `worktrees/` when it is missing): each fails with
    /// [`ErrorCode::Exists`] when it exists already, and `path` with `GIT_ERROR_OS`
    /// when its parent is missing. libgit2 joins `name` to the administrative path as
    /// given, so a name with `/` or `..` components reaches other directories (or
    /// fails when their parents are missing). Fails with [`ErrorCode::NotFound`] for a
    /// repository without paths, where libgit2 would create `worktrees/` in the
    /// process's working directory and then trip over its missing common directory.
    ///
    /// One failure, of the common directory's path that cannot be resolved once the
    /// worktree's directories are made, returns 1 without a worktree (a precedence slip
    /// in libgit2), which C callers testing for a negative code take for success: it is
    /// reported as an error all the same, with `GIT_ERROR`.
    ///
    /// libgit2 does not undo what it made before a failure. A failed checkout (one
    /// [`CheckoutOptionsMut`] directs elsewhere, say) comes after every file is written
    /// and the branch is set up: the worktree is left behind, looked up by
    /// [`lookup`](Self::lookup) but never checked out, and keeps its branch checked
    /// out, so adding it again fails until it is pruned. A working directory that
    /// cannot be made leaves the administrative directory alone behind, and the branch
    /// a failed add created stays.
    ///
    /// libgit2 reads the options, and copies the reference, for the call only.
    ///
    /// Wraps: git_worktree_add
    pub fn add(
        repo: RepositoryRef<'_>,
        name: &CStr,
        path: &CStr,
        opts: Option<AddOptionsRef<'_, '_>>,
    ) -> Result<WorktreeBox, WorktreeError> {
        if commondir(repo).is_none() {
            return Err(refused(ffi::GIT_ENOTFOUND, NO_PATH));
        }
        // A reference of another repository is replaced by `repo`'s own of that name,
        // so that libgit2 asks `repo`'s working trees whether the branch is checked
        // out; the copy of the options it then reads is the one `git_worktree_add`
        // makes itself, with that reference in place of the given one.
        let local = opts
            .and_then(|o| o.reference())
            .filter(|r| !owned_by(*r, repo))
            .and_then(|r| same_name(repo, r));
        let substituted = match (opts, &local) {
            (Some(given), Some(local)) => {
                // SAFETY: a read of the whole live `git_worktree_add_options` through
                // the shared handle's pointer; it is plain data (integers and pointers).
                let mut copy = unsafe { given.as_ptr().read() };
                copy.ref_ = local.as_c_ptr();
                Some(copy)
            }
            _ => None,
        };
        let opts = match (&substituted, opts) {
            (Some(copy), _) => ptr::from_ref(copy),
            (None, Some(o)) => o.as_ptr(),
            (None, None) => ptr::null(),
        };
        let mut out = ptr::null_mut();
        let rc = checked(
            || {
                // The options are of the current version (no setter changes it); their
                // reference is attached and borrowed, with its repository, for `'o`, which
                // outlives this call (or is `local`, which outlives it too), as do their
                // checkout options' borrows.
                // SAFETY: `out` is a writable slot the routine NULLs first; `repo` a live
                // repository on this thread with a common directory; `name` and `path`
                // NUL-terminated, copied; `opts` NULL or live options, copied and read for
                // the call only.
                unsafe {
                    ffi::git_worktree_add(
                        &mut out,
                        repo.as_ptr().cast_mut(),
                        name.as_ptr(),
                        path.as_ptr(),
                        opts,
                    )
                }
            },
            generic,
        )?;
        // SAFETY: `out` is NULL, or (on a 0 return) the fresh worktree
        // `git_worktree_lookup` built, whose strings are all copies, owned by the caller
        // alone and released by `git_worktree_free`.
        match unsafe { WorktreeBox::from_c(out) } {
            Some(worktree) if rc == 0 => Ok(worktree),
            worktree => {
                drop(worktree);
                Err(WorktreeError::from_libgit2(ffi::GIT_ERROR, generic(rc)))
            }
        }
    }
}

impl<'a> WorktreeRef<'a> {
    /// The worktree's name, the last component of its administrative directory.
    ///
    /// The string is the worktree's, so it cannot outlive the box:
    ///
    /// ```compile_fail,E0505
    /// use core::ffi::CStr;
    /// use libgit2::libgit2::repository::worktree::WorktreeBox;
    ///
    /// fn escape(w: WorktreeBox) -> &'static CStr {
    ///     let name = w.as_ref().name();
    ///     drop(w);
    ///     name
    /// }
    /// ```
    ///
    /// Wraps: git_worktree_name
    pub fn name(&self) -> &'a CStr {
        // SAFETY: a live worktree, only read.
        let name = unsafe { ffi::git_worktree_name(self.as_ptr()) };
        // SAFETY: a worktree's `name` is a non-NULL copy it owns, written when it is
        // built and freed only with it, so it lives for `'a`.
        unsafe { CStr::from_ptr(name) }
    }

    /// The path of the worktree's working directory, as its `gitdir` file records it
    /// (the directory of the `.git` link), without a trailing slash.
    ///
    /// Wraps: git_worktree_path
    pub fn path(&self) -> &'a CStr {
        // SAFETY: a live worktree, only read.
        let path = unsafe { ffi::git_worktree_path(self.as_ptr()) };
        // SAFETY: a worktree's `worktree_path` is a non-NULL copy it owns, written when
        // it is built and freed only with it, so it lives for `'a`.
        unsafe { CStr::from_ptr(path) }
    }

    /// Checks that the worktree's directories still exist: its administrative
    /// directory (with its `commondir`, `gitdir` and `HEAD` files), the parent
    /// repository's working directory, the common directory and its own working
    /// directory, in that order.
    ///
    /// Fails with [`ErrorCode::Generic`] and a `GIT_ERROR_WORKTREE` message naming the
    /// first one missing.
    ///
    /// Wraps: git_worktree_validate
    pub fn validate(&self) -> Result<(), WorktreeError> {
        checked(
            || {
                // SAFETY: a live worktree, only read.
                unsafe { ffi::git_worktree_validate(self.as_ptr()) }
            },
            generic,
        )?;
        Ok(())
    }

    /// Whether the worktree is locked: its administrative directory holds a `locked`
    /// file. Reads the file system, not the state [`WorktreeMut::lock`] recorded.
    ///
    /// Wraps: git_worktree_is_locked
    pub fn is_locked(&self) -> Result<bool, WorktreeError> {
        let rc = checked(
            || {
                // SAFETY: a NULL reason, which the routine does not fill; a live
                // worktree, only read.
                unsafe { ffi::git_worktree_is_locked(ptr::null_mut(), self.as_ptr()) }
            },
            generic,
        )?;
        Ok(rc != 0)
    }

    /// The reason the worktree is locked: the contents of its `locked` file (empty when
    /// none was given), or `None` when it is not locked.
    ///
    /// Fails, with libgit2's error, when the `locked` entry cannot be read (a
    /// directory, say).
    ///
    /// Wraps: git_worktree_is_locked
    pub fn lock_reason(&self) -> Result<Option<BufVal>, WorktreeError> {
        let mut reason = Buf::new();
        let rc = checked(
            || {
                // SAFETY: `reason` is a live, empty buffer the routine fills from
                // libgit2's allocator (or with its static empty string) on success; a
                // live worktree, only read.
                unsafe { ffi::git_worktree_is_locked(reason.as_mut().as_mut_ptr(), self.as_ptr()) }
            },
            generic,
        )?;
        Ok((rc != 0).then_some(reason))
    }

    /// Whether [`prune`](Self::prune) would prune the worktree under `opts`: it is not
    /// locked (unless [`PruneFlag::Locked`]), not valid (unless [`PruneFlag::Valid`];
    /// see [`validate`](Self::validate)), and its administrative directory exists.
    ///
    /// When it is not prunable, libgit2 leaves the reason as the thread's last error,
    /// which [`last`] reads. Fails when `opts`' version is not one libgit2 accepts, or
    /// when the `locked` entry cannot be read.
    ///
    /// Wraps: git_worktree_is_prunable
    pub fn is_prunable(&self, opts: PruneOptionsRef<'_>) -> Result<bool, WorktreeError> {
        let rc = checked(
            || {
                // SAFETY: a live worktree and live options, both only read (the routine
                // copies the options) despite the non-const parameters.
                unsafe {
                    ffi::git_worktree_is_prunable(
                        self.as_ptr().cast_mut(),
                        opts.as_ptr().cast_mut(),
                    )
                }
            },
            generic,
        )?;
        Ok(rc != 0)
    }

    /// Prunes the worktree under `opts`: deletes its administrative directory and,
    /// with [`PruneFlag::WorkingTree`], its working directory if it still has its
    /// `.git` link. The worktree itself stays readable.
    ///
    /// Fails with [`ErrorCode::Generic`] and the reason
    /// [`is_prunable`](Self::is_prunable) gives when it is not prunable, with that
    /// routine's error when it fails (where libgit2 would prune all the same; see the
    /// [module docs](self#routines)), and with libgit2's error when a directory cannot
    /// be removed.
    ///
    /// Wraps: git_worktree_prune
    pub fn prune(&self, opts: PruneOptionsRef<'_>) -> Result<(), WorktreeError> {
        if !self.is_prunable(opts)? {
            return Err(WorktreeError::from_libgit2(
                ErrorCode::Generic.raw(),
                generic(0),
            ));
        }
        checked(
            || {
                // SAFETY: a live worktree and live options, both only read despite the
                // non-const parameters; the routine deletes directories, not the struct.
                unsafe {
                    ffi::git_worktree_prune(self.as_ptr().cast_mut(), opts.as_ptr().cast_mut())
                }
            },
            generic,
        )?;
        Ok(())
    }
}

impl WorktreeMut<'_> {
    /// Locks the worktree: creates its `locked` file, holding `reason` (or nothing).
    ///
    /// Fails with [`ErrorCode::Locked`] when it is already locked, and with libgit2's
    /// error when the file cannot be created.
    ///
    /// It writes the worktree, so a shared handle cannot lock it:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::repository::worktree::WorktreeBox;
    ///
    /// fn lock(w: &WorktreeBox) {
    ///     let _ = w.as_ref().lock(None);
    /// }
    /// ```
    ///
    /// Wraps: git_worktree_lock
    pub fn lock(&mut self, reason: Option<&CStr>) -> Result<(), WorktreeError> {
        let reason = reason.map_or(ptr::null(), CStr::as_ptr);
        let worktree = self.as_mut_ptr();
        checked(
            || {
                // SAFETY: a live worktree, exclusively borrowed, whose `locked` bit the
                // routine writes; `reason` is NULL or NUL-terminated, read for the call.
                unsafe { ffi::git_worktree_lock(worktree, reason) }
            },
            |_| (ErrorClass::Worktree, c"worktree is already locked"),
        )?;
        Ok(())
    }

    /// Unlocks the worktree: removes its `locked` file. `true` when it was locked,
    /// `false` when there was nothing to remove.
    ///
    /// Fails with libgit2's error when the file cannot be checked, and with
    /// `GIT_ERROR_OS` when it cannot be removed.
    ///
    /// Wraps: git_worktree_unlock
    pub fn unlock(&mut self) -> Result<bool, WorktreeError> {
        let worktree = self.as_mut_ptr();
        let rc = checked(
            || {
                // SAFETY: a live worktree, exclusively borrowed, whose `locked` bit the
                // routine writes.
                unsafe { ffi::git_worktree_unlock(worktree) }
            },
            |_| (ErrorClass::Os, c"failed to remove the worktree's lock file"),
        )?;
        Ok(rc == 0)
    }
}

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
                flags: 0xFFFF_0001,
            };
            let opts = PruneOptions::from(raw);
            assert_eq!(opts.as_ref().version(), 7);
            assert_eq!(opts.as_ref().flags().bits(), 0xFFFF_0001);
            let back = ffi::git_worktree_prune_options::from(opts.as_ref().to_options());
            assert_eq!((back.version, back.flags), (7, 0xFFFF_0001));
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
        use core::ptr;
        use std::ffi::{CStr, CString};
        use std::fs;
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        use super::super::*;
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
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        use super::super::*;
        use crate::libgit2::refs::refs::ReferenceBox;
        use crate::libgit2::util::sys::errors::runtime_pinned;

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

    mod routines {
        use std::ffi::CString;
        use std::fs;
        use std::path::PathBuf;

        use super::super::*;
        use crate::libgit2::repository::repository::{Repository, RepositoryBox};
        use crate::libgit2::util::sys::errors::set_str;

        /// A non-bare repository with linked worktrees laid out on disk by
        /// [`add`](Self::add), as `git_worktree_add` leaves them; the directory is
        /// removed on drop.
        struct Linked {
            base: PathBuf,
            repo: RepositoryBox,
        }

        impl Linked {
            fn new(tag: &str) -> Self {
                let base = std::env::temp_dir().join(format!(
                    "crustify-worktree-routines-{}-{tag}",
                    std::process::id()
                ));
                let _ = fs::remove_dir_all(&base);
                let path = CString::new(base.join("repo").as_os_str().as_bytes()).unwrap();
                let repo = Repository::init(&path, false).unwrap();
                Self { base, repo }
            }

            fn admin(&self, name: &str) -> PathBuf {
                self.base.join("repo/.git/worktrees").join(name)
            }

            fn add(&self, name: &str) {
                let admin = self.admin(name);
                let workdir = self.base.join("wt").join(name);
                fs::create_dir_all(&admin).unwrap();
                fs::create_dir_all(&workdir).unwrap();
                fs::write(admin.join("commondir"), "../..\n").unwrap();
                let link = workdir.join(".git");
                fs::write(admin.join("gitdir"), format!("{}\n", link.display())).unwrap();
                fs::write(admin.join("HEAD"), format!("ref: refs/heads/{name}\n")).unwrap();
                fs::write(link, format!("gitdir: {}\n", admin.display())).unwrap();
            }

            /// `git_worktree_list` as C reports it: the names, or the code.
            fn c_list(&self) -> Vec<CString> {
                let mut raw = ffi::git_strarray::default();
                // SAFETY: a live, empty array the routine fills; the live repository,
                // only read.
                let rc = unsafe { ffi::git_worktree_list(&mut raw, self.repo.as_c_ptr()) };
                assert_eq!(rc, 0);
                let names = (0..raw.count)
                    // SAFETY: `count` NUL-terminated strings, live until the dispose.
                    .map(|i| unsafe { CStr::from_ptr(*raw.strings.add(i)) }.to_owned())
                    .collect();
                // SAFETY: the array the routine filled, released once.
                unsafe { ffi::git_strarray_dispose(&mut raw) };
                names
            }
        }

        impl Drop for Linked {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.base);
            }
        }

        fn sorted(names: StrArrayVal) -> Vec<CString> {
            let mut names: Vec<_> = names.as_ref().iter().map(CStr::to_owned).collect();
            names.sort();
            names
        }

        /// A stale error, as an earlier failing call would leave one.
        fn stale() {
            set_str(ErrorClass::Odb, c"stale").unwrap();
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn list_checks_the_entries_libgit2_lets_through() {
            // libgit2 removes a non-worktree entry while iterating, skipping the next
            // one: among four such entries and a worktree, in any directory order, at
            // least one directly follows another, so C lists it.
            let linked = Linked::new("list");
            for junk in ["j1", "j2", "j3", "j4"] {
                fs::create_dir_all(linked.admin(junk)).unwrap();
            }
            linked.add("good");
            let c = linked.c_list();
            assert!(c.len() > 1, "C listed {c:?}");
            assert!(c.contains(&c"good".to_owned()));
            let rust = sorted(list(linked.repo.as_ref()).unwrap());
            assert_eq!(rust, [c"good".to_owned()]);
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn list_keeps_the_directory_order_of_the_worktrees() {
            let linked = Linked::new("order");
            for name in ["a", "b", "c"] {
                linked.add(name);
            }
            fs::create_dir_all(linked.admin("junk")).unwrap();
            fs::write(linked.admin("file"), "").unwrap();
            let c: Vec<_> = linked
                .c_list()
                .into_iter()
                .filter(|n| ![c"junk", c"file"].contains(&n.as_c_str()))
                .collect();
            let rust: Vec<_> = list(linked.repo.as_ref())
                .unwrap()
                .as_ref()
                .iter()
                .map(CStr::to_owned)
                .collect();
            assert_eq!(rust, c);
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn repository_without_paths_has_no_worktrees() {
            let repo = Repository::new().unwrap();
            assert!(list(repo.as_ref()).unwrap().as_ref().is_empty());
            stale();
            // C would pass the NULL common directory to `strlen`.
            let e = Worktree::lookup(repo.as_ref(), c"wt").unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::NotFound));
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
            assert_eq!(e.message(), c"path cannot exist in repository");
            let e = Worktree::open_from_repository(repo.as_ref()).unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Worktree));
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn prune_refuses_when_is_prunable_fails() {
            let linked = Linked::new("prune");
            linked.add("wt");
            let w = Worktree::lookup(linked.repo.as_ref(), c"wt").unwrap();
            // A `locked` directory: unreadable as a lock reason, so `is_prunable` fails,
            // which `git_worktree_prune` would take for "prunable".
            fs::create_dir(linked.admin("wt").join("locked")).unwrap();
            let opts = PruneOptions::from(ffi::git_worktree_prune_options {
                version: PruneOptions::VERSION,
                flags: PruneFlag::Valid.raw(),
            });
            let e = w.as_ref().is_prunable(opts.as_ref()).unwrap_err();
            let again = w.as_ref().prune(opts.as_ref()).unwrap_err();
            assert_eq!(again, e);
            assert_eq!(e.code(), Some(ErrorCode::NotFound));
            assert!(linked.admin("wt").exists(), "the worktree was pruned");
            // With the lock ignored, the check passes and the worktree goes.
            let mut opts = opts;
            opts.as_mut()
                .set_flags(PruneFlag::Valid | PruneFlag::Locked);
            assert_eq!(w.as_ref().is_prunable(opts.as_ref()), Ok(true));
            w.as_ref().prune(opts.as_ref()).unwrap();
            assert!(!linked.admin("wt").exists());
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn not_prunable_is_reported_with_the_reason() {
            let linked = Linked::new("valid");
            linked.add("wt");
            let w = Worktree::lookup(linked.repo.as_ref(), c"wt").unwrap();
            let opts = PruneOptions::new();
            assert_eq!(w.as_ref().is_prunable(opts.as_ref()), Ok(false));
            assert_eq!(last().message(), c"not pruning valid working tree");
            let e = w.as_ref().prune(opts.as_ref()).unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::Generic));
            assert_eq!(e.class(), Some(ErrorClass::Worktree));
            assert_eq!(e.message(), c"not pruning valid working tree");
            assert!(linked.admin("wt").exists());
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn messageless_failures_get_an_error_of_their_own() {
            let linked = Linked::new("messages");
            linked.add("wt");
            fs::create_dir_all(linked.admin("broken")).unwrap();

            stale();
            let e = Worktree::lookup(linked.repo.as_ref(), c"absent").unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::NotFound));
            assert_eq!(e.class(), Some(ErrorClass::Worktree));
            assert_eq!(e.message(), c"worktree does not exist");

            stale();
            let e = Worktree::lookup(linked.repo.as_ref(), c"broken").unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::Generic));
            assert_eq!(e.message(), c"not a valid worktree directory");

            let mut w = Worktree::lookup(linked.repo.as_ref(), c"wt").unwrap();
            w.as_mut().lock(None).unwrap();
            stale();
            let e = w.as_mut().lock(Some(c"again")).unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::Locked));
            assert_eq!(e.class(), Some(ErrorClass::Worktree));
            assert_eq!(e.message(), c"worktree is already locked");
            assert_eq!(w.as_mut().unlock(), Ok(true));

            // A `locked` directory counts as a lock that `unlink` cannot remove.
            fs::create_dir(linked.admin("wt").join("locked")).unwrap();
            stale();
            let e = w.as_mut().unlock().unwrap_err();
            assert_eq!(e.code(), Some(ErrorCode::Generic));
            assert_eq!(e.class(), Some(ErrorClass::Os));
            assert_eq!(
                e.message(),
                c"failed to remove the worktree's lock file: Is a directory"
            );
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn errors_set_by_libgit2_are_kept() {
            let linked = Linked::new("kept");
            linked.add("wt");
            let w = Worktree::lookup(linked.repo.as_ref(), c"wt").unwrap();
            fs::remove_dir_all(linked.base.join("wt/wt")).unwrap();
            stale();
            let e = w.as_ref().validate().unwrap_err();
            assert_eq!(e.class(), Some(ErrorClass::Worktree));
            let message = e.message().to_str().unwrap();
            assert!(message.starts_with("worktree directory '"), "{message}");
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn error_reads_as_its_parts() {
            let linked = Linked::new("error");
            let e = Worktree::lookup(linked.repo.as_ref(), c"absent").unwrap_err();
            assert_eq!(e.raw_code(), ffi::GIT_ENOTFOUND);
            assert_eq!(e.raw_class(), ffi::GIT_ERROR_WORKTREE as c_int);
            assert_eq!(e.error().message(), e.message());
            assert_eq!(e.to_string(), "worktree does not exist");
            assert_eq!(
                format!("{e:?}"),
                "WorktreeError { code: NotFound, error: ErrorBuf { class: Worktree, \
                 message: \"worktree does not exist\" } }"
            );
            let unnamed = WorktreeError {
                code: -12345,
                error: e.error().clone(),
            };
            assert_eq!(unnamed.code(), None);
            assert!(format!("{unnamed:?}").starts_with("WorktreeError { code: -12345,"));
            let source: &dyn std::error::Error = &e;
            assert!(source.source().is_none());
            let buf: ErrorBuf = e.clone().into();
            assert_eq!(&buf, e.error());
        }
    }

    mod adding {
        use std::ffi::CString;
        use std::fs;
        use std::path::PathBuf;
        use std::process::Command;

        use super::super::*;
        use crate::libgit2::refs::refs::ReferenceBox;
        use crate::libgit2::repository::repository::{Repository, RepositoryBox};

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// Runs `body` alone in a child process, which may change its working directory.
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

        fn scratch(tag: &str) -> PathBuf {
            let path = std::env::temp_dir().join(format!(
                "crustify-worktree-add-unit-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            path
        }

        fn c_path(path: &std::path::Path) -> CString {
            CString::new(path.as_os_str().as_bytes()).unwrap()
        }

        /// A repository without paths has no common directory: `git_worktree_add`
        /// joins the NULL to `worktrees` and creates that, and the worktree's own
        /// directory, in the process's working directory, before its
        /// `GIT_ASSERT_ARG` on the NULL fails (or aborts a debug build). The wrapper
        /// refuses such a repository first. Run in a child that works in a scratch
        /// directory, where C's defect is shown too (not in the debug build).
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn add_refuses_a_repository_without_paths() {
            isolated(
                concat!(module_path!(), "::add_refuses_a_repository_without_paths"),
                || {
                    let root = scratch("pathless");
                    let main_path = c_path(&root.join("main"));
                    let main = Repository::init(&main_path, true).unwrap();
                    let line = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\n";
                    fs::write(root.join("main/refs/heads/topic"), line).unwrap();
                    let mut out = ptr::null_mut();
                    let name = c"refs/heads/topic";
                    // SAFETY: a writable slot, the live repository and a NUL-terminated
                    // name.
                    let rc = unsafe {
                        ffi::git_reference_lookup(&mut out, main.as_c_ptr(), name.as_ptr())
                    };
                    assert_eq!(rc, 0);
                    // SAFETY: a fresh attached reference the test owns alone, freed by
                    // `git_reference_free`; `main` outlives it.
                    let topic = unsafe { ReferenceBox::from_c(out) }.unwrap();
                    let cwd = root.join("cwd");
                    fs::create_dir(&cwd).unwrap();
                    std::env::set_current_dir(&cwd).unwrap();
                    let pathless = Repository::new().unwrap();
                    let mut opts = AddOptions::new();
                    opts.as_mut().set_reference(Some(topic.as_ref()));
                    let wt = c_path(&root.join("wt"));
                    let error = Worktree::add(pathless.as_ref(), c"wt", &wt, Some(opts.as_ref()))
                        .unwrap_err();
                    assert_eq!(error.code(), Some(ErrorCode::NotFound));
                    assert_eq!(error.message(), c"path cannot exist in repository");
                    assert_eq!(fs::read_dir(&cwd).unwrap().count(), 0);
                    assert!(!root.join("wt").exists());
                    if std::env::var("CRUSTIFY_BUILD_VARIANT").as_deref() != Ok("coverage") {
                        let mut out = ptr::null_mut();
                        let raw = pathless.as_c_ptr();
                        let opts = opts.as_ref().as_ptr();
                        // SAFETY: a writable slot, the live repository, NUL-terminated
                        // strings and live options; the release build fails the NULL
                        // common directory's assertion.
                        let rc = unsafe {
                            ffi::git_worktree_add(&mut out, raw, c"wt".as_ptr(), wt.as_ptr(), opts)
                        };
                        // `(err = prettify_dir(..) < 0)`: the failure returns 1, and no
                        // worktree.
                        assert_eq!((rc, out.is_null()), (1, true));
                        assert!(cwd.join("worktrees/wt").is_dir());
                    }
                    // The options' last use is above; the reference they borrowed goes now.
                    drop(topic);
                    drop(main);
                    std::env::set_current_dir(std::env::temp_dir()).unwrap();
                    fs::remove_dir_all(&root).unwrap();
                },
            );
        }

        /// A non-bare repository at `path` with one commit, which `main` and `feature`
        /// name and `HEAD` points at through `main`.
        fn committed(path: &std::path::Path) -> RepositoryBox {
            let repo = Repository::init(&c_path(path), false).unwrap();
            let mut odb = ptr::null_mut();
            // SAFETY: a writable slot and the live repository; the odb is released below.
            let rc = unsafe { ffi::git_repository_odb(&mut odb, repo.as_c_ptr()) };
            assert_eq!(rc, 0);
            let write = |data: &[u8], kind: ffi::git_object_t| {
                let mut id = ffi::git_oid::default();
                let (bytes, len) = (data.as_ptr().cast(), data.len());
                // SAFETY: a writable id, the live odb and `data` readable for `len` bytes
                // at a non-NULL address.
                let rc = unsafe { ffi::git_odb_write(&mut id, odb, bytes, len, kind) };
                assert_eq!(rc, 0);
                let mut hex = [0 as std::ffi::c_char; 65];
                // SAFETY: `hex` bounds the write, and the routine NUL-terminates in it.
                unsafe { ffi::git_oid_tostr(hex.as_mut_ptr(), hex.len(), &id) };
                // SAFETY: NUL-terminated within `hex`.
                unsafe { CStr::from_ptr(hex.as_ptr()) }
                    .to_str()
                    .unwrap()
                    .to_owned()
            };
            const WHO: &str = "A U Thor <author@example.com> 1700000000 +0000";
            let tree = write(b"", ffi::GIT_OBJECT_TREE);
            let commit = format!("tree {tree}\nauthor {WHO}\ncommitter {WHO}\n\nfirst\n");
            let commit = write(commit.as_bytes(), ffi::GIT_OBJECT_COMMIT);
            // SAFETY: the odb reference taken above, released once.
            unsafe { ffi::git_odb_free(odb) };
            for branch in ["main", "feature"] {
                let reference = path.join(".git/refs/heads").join(branch);
                fs::write(reference, format!("{commit}\n")).unwrap();
            }
            fs::write(path.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            repo
        }

        /// `git_reference_lookup(name)` in `repo`, as an owned attached reference.
        fn lookup<'r>(repo: &'r RepositoryBox, name: &CStr) -> ReferenceBox<'r> {
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot, the live repository and a NUL-terminated name.
            let rc = unsafe { ffi::git_reference_lookup(&mut out, repo.as_c_ptr(), name.as_ptr()) };
            assert_eq!(rc, 0, "{name:?}");
            // SAFETY: a fresh attached reference the test owns alone, released by
            // `git_reference_free`; `repo` outlives it.
            unsafe { ReferenceBox::from_c(out) }.unwrap()
        }

        /// `git_worktree_add(repo, name, path, opts)`, raw: the return and whether a
        /// worktree came back (freed).
        fn c_add(
            repo: &RepositoryBox,
            name: &CStr,
            path: &CStr,
            opts: AddOptionsRef<'_, '_>,
        ) -> (c_int, bool) {
            let mut out = ptr::null_mut();
            let (raw, opts) = (repo.as_c_ptr(), opts.as_ptr());
            let (name, path) = (name.as_ptr(), path.as_ptr());
            // SAFETY: a writable slot, the live repository, NUL-terminated strings and
            // live options whose reference outlives the call.
            let rc = unsafe { ffi::git_worktree_add(&mut out, raw, name, path, opts) };
            let added = !out.is_null();
            // SAFETY: NULL or the fresh worktree, freed once.
            unsafe { ffi::git_worktree_free(out) };
            (rc, added)
        }

        /// `git_worktree_add` asks the repository the given reference belongs to whether
        /// its branch is checked out, not the repository the worktree is added to. With
        /// a reference of another repository (which lends only its name), C lets the
        /// branch of that name be checked out in two of the repository's working trees,
        /// and refuses it while the other repository has its own checked out. The
        /// wrapper passes libgit2 the repository's own reference of that name: the
        /// check then concerns the right working trees. C's defect is shown alongside.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn add_checks_the_branch_in_the_repository_added_to() {
            let root = scratch("lent-reference");
            let feature = c"refs/heads/feature";
            for side in ["c", "rust"] {
                let base = root.join(side);
                fs::create_dir_all(base.join("wt")).unwrap();
                let main = committed(&base.join("main"));
                let other = committed(&base.join("other"));
                let lent = lookup(&other, feature);
                let mut opts = AddOptions::new();
                opts.as_mut().set_reference(Some(lent.as_ref()));
                let at = |rel: &str| c_path(&base.join(rel));
                let rust = side == "rust";
                type Added = (c_int, bool, Option<CString>);
                let add = |repo: &RepositoryBox, name: &CStr, rel: &str| -> Added {
                    if rust {
                        match Worktree::add(repo.as_ref(), name, &at(rel), Some(opts.as_ref())) {
                            Ok(_) => (0, true, None),
                            Err(e) => (e.raw_code(), false, Some(e.message().to_owned())),
                        }
                    } else {
                        let (rc, added) = c_add(repo, name, &at(rel), opts.as_ref());
                        (rc, added, (rc < 0).then(|| last().message().to_owned()))
                    }
                };
                // The branch is free in both repositories: the worktree is added, and
                // `main`'s `feature` is what it checks out.
                assert_eq!(add(&main, c"one", "wt/one"), (0, true, None));
                let head = fs::read_to_string(base.join("main/.git/worktrees/one/HEAD")).unwrap();
                assert_eq!(head, "ref: refs/heads/feature\n");
                // `main`'s `feature` is checked out now: C asks `other` and adds a second
                // worktree on it; the wrapper refuses.
                let again = add(&main, c"two", "wt/two");
                if rust {
                    let message =
                        Some(c"reference refs/heads/feature is already checked out".to_owned());
                    assert_eq!(again, (-1, false, message));
                    assert!(!base.join("main/.git/worktrees/two").exists());
                } else {
                    assert_eq!(again, (0, true, None));
                }
                // `other`'s `feature` is checked out in `other`'s own working tree, and
                // `main`'s is free in a third repository: C refuses, the wrapper adds.
                let own = c_add(&other, c"own", &at("wt/own"), opts.as_ref());
                assert_eq!(own, (0, true));
                let third = committed(&base.join("third"));
                let free = add(&third, c"three", "wt/three");
                if rust {
                    assert_eq!(free, (0, true, None));
                } else {
                    let message =
                        Some(c"reference refs/heads/feature is already checked out".to_owned());
                    assert_eq!(free, (-1, false, message));
                }
                drop(lent);
                drop((main, other, third));
            }
            fs::remove_dir_all(&root).unwrap();
        }
    }
}
