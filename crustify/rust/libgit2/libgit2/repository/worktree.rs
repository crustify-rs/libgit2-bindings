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

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::ops::{BitOr, BitOrAssign};
use core::ptr::{addr_of, addr_of_mut};

use ffibox::define_ctype;

use crate::ffi;

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
}
