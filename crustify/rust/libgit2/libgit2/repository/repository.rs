//! Safe wrappers over `include/git2/repository.h`.

use core::fmt;
use core::ops::{BitOr, BitOrAssign};

use crate::ffi;

/// One option of `git_repository_init_ext`, a single bit of an [`InitFlags`] set
/// (`git_repository_init_flag_t`). Each is off by default.
///
/// [`from_raw`](Self::from_raw) gives `None` for any value without a variant: `0`,
/// a combination of bits, or a bit this version of libgit2's header does not name.
///
/// Wraps: git_repository_init_flag_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum InitFlag {
    /// Create a bare repository, with no working directory.
    Bare = ffi::GIT_REPOSITORY_INIT_BARE,
    /// Fail with `GIT_EEXISTS` if the path already holds a repository, rather than
    /// reinitialising it.
    NoReinit = ffi::GIT_REPOSITORY_INIT_NO_REINIT,
    /// Create the last component of the repository and working directory paths as
    /// needed; the `.git` directory itself is always created.
    Mkdir = ffi::GIT_REPOSITORY_INIT_MKDIR,
    /// Create every missing component of the repository and working directory paths.
    ///
    /// Its bit is `GIT_REPOSITORY_OPEN_FROM_ENV`'s too, and `git_repository_init_ext`
    /// tests that open flag on the same `flags`: with this flag set, init also honours
    /// `$GIT_COMMON_DIR` when deciding whether the path already holds a repository,
    /// and reads the global and system configuration from the environment's paths.
    Mkpath = ffi::GIT_REPOSITORY_INIT_MKPATH,
    /// Initialise from an external template: the options' template path, else the
    /// `init.templatedir` setting, else the system's template directory, rather than
    /// libgit2's built-in one.
    ExternalTemplate = ffi::GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE,
    /// With a working directory away from the repository, record the repository's
    /// location in the `.git` file and `core.worktree` as relative paths.
    RelativeGitlink = ffi::GIT_REPOSITORY_INIT_RELATIVE_GITLINK,
}

impl InitFlag {
    /// Every variant, in ascending bit order.
    pub const ALL: [InitFlag; 6] = [
        Self::Bare,
        Self::NoReinit,
        Self::Mkdir,
        Self::Mkpath,
        Self::ExternalTemplate,
        Self::RelativeGitlink,
    ];

    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_repository_init_flag_t) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i] as ffi::git_repository_init_flag_t == raw {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }

    /// The C value: the flag's bit.
    pub const fn raw(self) -> ffi::git_repository_init_flag_t {
        self as ffi::git_repository_init_flag_t
    }
}

impl TryFrom<ffi::git_repository_init_flag_t> for InitFlag {
    type Error = UnnamedInitFlag;

    fn try_from(raw: ffi::git_repository_init_flag_t) -> Result<Self, UnnamedInitFlag> {
        Self::from_raw(raw).ok_or(UnnamedInitFlag(raw))
    }
}

impl From<InitFlag> for ffi::git_repository_init_flag_t {
    fn from(value: InitFlag) -> ffi::git_repository_init_flag_t {
        value.raw()
    }
}

// An `InitFlag` passes where C takes a `git_repository_init_flag_t`.
const _: () = assert!(
    size_of::<InitFlag>() == size_of::<ffi::git_repository_init_flag_t>()
        && align_of::<InitFlag>() == align_of::<ffi::git_repository_init_flag_t>()
);

/// An init flag value that this version of libgit2's header does not name: `0`, a
/// combination of flags, or another bit. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedInitFlag(ffi::git_repository_init_flag_t);

impl UnnamedInitFlag {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_repository_init_flag_t {
        self.0
    }
}

impl fmt::Display for UnnamedInitFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 repository init flag {:#x} is not named by git2/repository.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedInitFlag {}

/// A set of [`InitFlag`]s: the `uint32_t` `flags` of `git_repository_init_options`,
/// OR-ing `GIT_REPOSITORY_INIT_*` bits. The default is the empty set, as
/// `git_repository_init_options_init` leaves it.
///
/// libgit2 reads bits no [`InitFlag`] names too: `git_repository_init_ext` ORs its
/// own internal bits (`1 << 16` for a path ending in `.git`, `1 << 17` for a working
/// directory that is its parent, `1 << 18` for a reinitialisation) into the options
/// it is given, and reads them back on the next call with those options.
/// [`from_bits_retain`](Self::from_bits_retain) keeps every bit, so
/// [`bits`](Self::bits) returns what C produced; [`from_bits_truncate`](Self::from_bits_truncate)
/// keeps the named ones only. [`iter`](Self::iter) and [`contains`](Self::contains)
/// see the named flags, and [`unnamed_bits`](Self::unnamed_bits) the rest.
///
/// Wraps: git_repository_init_flag_t
/// Handwritten
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct InitFlags(u32);

impl InitFlags {
    /// The bits of every [`InitFlag`].
    const NAMED: u32 = {
        let mut bits = 0;
        let mut i = 0;
        while i < InitFlag::ALL.len() {
            bits |= InitFlag::ALL[i].raw();
            i += 1;
        }
        bits
    };

    /// The set with no flag.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The set of every bit of `bits`, as the options' `flags` hold them; bits that no
    /// [`InitFlag`] names are kept.
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// The set of the bits of `bits` that an [`InitFlag`] names; the rest are dropped.
    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & Self::NAMED)
    }

    /// The bits of the set, as the options' `flags` take them.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The bits of the set that no [`InitFlag`] names.
    pub const fn unnamed_bits(self) -> u32 {
        self.0 & !Self::NAMED
    }

    /// Whether the set has no bit, named or not.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `flag` is in the set.
    pub const fn contains(self, flag: InitFlag) -> bool {
        self.0 & flag.raw() != 0
    }

    /// Adds `flag` to the set.
    pub fn insert(&mut self, flag: InitFlag) {
        self.0 |= flag.raw();
    }

    /// Removes `flag` from the set.
    pub fn remove(&mut self, flag: InitFlag) {
        self.0 &= !flag.raw();
    }

    /// The named flags in the set, in ascending bit order.
    pub fn iter(self) -> InitFlagsIter {
        InitFlagsIter { set: self, next: 0 }
    }
}

impl From<InitFlag> for InitFlags {
    fn from(flag: InitFlag) -> Self {
        Self(flag.raw())
    }
}

impl BitOr for InitFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<InitFlag> for InitFlags {
    type Output = Self;

    fn bitor(self, rhs: InitFlag) -> Self {
        self | Self::from(rhs)
    }
}

impl BitOr for InitFlag {
    type Output = InitFlags;

    fn bitor(self, rhs: Self) -> InitFlags {
        InitFlags::from(self) | rhs
    }
}

impl BitOrAssign for InitFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitOrAssign<InitFlag> for InitFlags {
    fn bitor_assign(&mut self, rhs: InitFlag) {
        self.insert(rhs);
    }
}

impl FromIterator<InitFlag> for InitFlags {
    fn from_iter<I: IntoIterator<Item = InitFlag>>(iter: I) -> Self {
        let mut set = Self::empty();
        set.extend(iter);
        set
    }
}

impl Extend<InitFlag> for InitFlags {
    fn extend<I: IntoIterator<Item = InitFlag>>(&mut self, iter: I) {
        for flag in iter {
            self.insert(flag);
        }
    }
}

impl IntoIterator for InitFlags {
    type Item = InitFlag;
    type IntoIter = InitFlagsIter;

    fn into_iter(self) -> InitFlagsIter {
        self.iter()
    }
}

/// Lists the named flags, then any unnamed bits in hex: `{Bare | Mkpath, 0x10000}`.
impl fmt::Debug for InitFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InitFlags {")?;
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

/// The named flags of an [`InitFlags`] set, in ascending bit order
/// ([`InitFlags::iter`]).
///
/// Handwritten
#[derive(Clone, Debug)]
pub struct InitFlagsIter {
    set: InitFlags,
    /// The index into [`InitFlag::ALL`] of the next flag to test.
    next: usize,
}

impl Iterator for InitFlagsIter {
    type Item = InitFlag;

    fn next(&mut self) -> Option<InitFlag> {
        while let Some(&flag) = InitFlag::ALL.get(self.next) {
            self.next += 1;
            if self.set.contains(flag) {
                return Some(flag);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(InitFlag::ALL.len().saturating_sub(self.next)))
    }
}

impl core::iter::FusedIterator for InitFlagsIter {}

/// The permissions `git_repository_init_ext` gives the directories it creates: the
/// `uint32_t` `mode` of `git_repository_init_options`, either one of the
/// `git_repository_init_mode_t` constants or a custom mode.
///
/// Every `u32` is a mode C accepts, so [`from_raw`](Self::from_raw) is total; it
/// gives the named variant for a constant's value, so a [`CustomMode`] never holds
/// one and equal modes compare equal.
///
/// libgit2 creates the repository's directories with the mode's permission bits
/// (`0777` for [`SharedUmask`](Self::SharedUmask)), trimmed by the process umask
/// except where it `chmod`s them: with any other mode, the built-in template's leaf
/// directories (`objects/info`, `objects/pack`, `hooks`, `info`) and an external
/// template's directories get the mode exactly, and a mode with the setgid bit, as
/// both shared ones have, is forced onto the repository directory too. Parent
/// directories that [`InitFlag::Mkdir`] or [`InitFlag::Mkpath`] create never get the
/// setgid bit, and [`InitFlag::Mkpath`]'s ancestors no world-write bit either.
///
/// ```compile_fail,E0423
/// use libgit2::libgit2::repository::repository::{CustomMode, InitMode};
/// // A custom mode comes from `from_raw` only, which names the shared modes.
/// let _ = InitMode::Custom(CustomMode(0o2775));
/// ```
///
/// Wraps: git_repository_init_mode_t
/// Handwritten
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InitMode {
    /// `0`: directories get `0777` less the umask, and nothing is shared. The
    /// default.
    #[default]
    SharedUmask,
    /// `02775`, git's `--shared=group`: group-writable directories that pass their
    /// group on (`g+sx`); the configuration records `core.sharedrepository = 1` and
    /// `receive.denyNonFastforwards`.
    SharedGroup,
    /// `02777`, git's `--shared=all`: as [`SharedGroup`](Self::SharedGroup), and
    /// world-readable; the configuration records `core.sharedrepository = 2`.
    SharedAll,
    /// Any other value: the directories' permission bits, recorded nowhere in the
    /// configuration.
    Custom(CustomMode),
}

impl InitMode {
    /// The mode whose C value is `raw`: a named variant for a
    /// `git_repository_init_mode_t` constant, [`Custom`](Self::Custom) otherwise.
    pub const fn from_raw(raw: ffi::git_repository_init_mode_t) -> Self {
        match raw {
            ffi::GIT_REPOSITORY_INIT_SHARED_UMASK => Self::SharedUmask,
            ffi::GIT_REPOSITORY_INIT_SHARED_GROUP => Self::SharedGroup,
            ffi::GIT_REPOSITORY_INIT_SHARED_ALL => Self::SharedAll,
            bits => Self::Custom(CustomMode(bits)),
        }
    }

    /// The C value, as the options' `mode` takes it.
    pub const fn raw(self) -> ffi::git_repository_init_mode_t {
        match self {
            Self::SharedUmask => ffi::GIT_REPOSITORY_INIT_SHARED_UMASK,
            Self::SharedGroup => ffi::GIT_REPOSITORY_INIT_SHARED_GROUP,
            Self::SharedAll => ffi::GIT_REPOSITORY_INIT_SHARED_ALL,
            Self::Custom(mode) => mode.0,
        }
    }
}

impl From<ffi::git_repository_init_mode_t> for InitMode {
    fn from(raw: ffi::git_repository_init_mode_t) -> Self {
        Self::from_raw(raw)
    }
}

impl From<InitMode> for ffi::git_repository_init_mode_t {
    fn from(value: InitMode) -> ffi::git_repository_init_mode_t {
        value.raw()
    }
}

/// A custom [`InitMode`]: permission bits other than the three shared modes' values.
/// Build one with [`InitMode::from_raw`].
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CustomMode(ffi::git_repository_init_mode_t);

impl CustomMode {
    /// The permission bits, as the options' `mode` takes them.
    pub const fn bits(self) -> ffi::git_repository_init_mode_t {
        self.0
    }
}

#[cfg(test)]
mod unit_tests {
    mod init_flags_and_modes {
        use super::super::*;

        #[test]
        fn flags_debug_lists_named_flags_then_unnamed_bits() {
            assert_eq!(format!("{:?}", InitFlags::empty()), "InitFlags {}");
            assert_eq!(
                format!("{:?}", InitFlag::Bare | InitFlag::Mkpath),
                "InitFlags {Bare | Mkpath}"
            );
            assert_eq!(
                format!("{:?}", InitFlags::from_bits_retain(0x10004)),
                "InitFlags {0x10004}"
            );
            assert_eq!(
                format!("{:?}", InitFlags::from_bits_retain(0x70011)),
                "InitFlags {Bare | Mkpath, 0x70000}"
            );
        }

        #[test]
        fn flags_build_from_operators_and_iterators() {
            let mut set: InitFlags = [InitFlag::Mkdir, InitFlag::Bare].into_iter().collect();
            assert_eq!(set.bits(), 0x9);
            set |= InitFlag::NoReinit;
            set |= InitFlags::from(InitFlag::RelativeGitlink);
            assert_eq!(set, InitFlags::from_bits_retain(0x4b));
            assert_eq!(
                set.into_iter().collect::<Vec<_>>(),
                [
                    InitFlag::Bare,
                    InitFlag::NoReinit,
                    InitFlag::Mkdir,
                    InitFlag::RelativeGitlink
                ]
            );
            set.remove(InitFlag::Bare);
            set.remove(InitFlag::Bare);
            assert!(!set.contains(InitFlag::Bare));
            set.extend([InitFlag::ExternalTemplate]);
            assert_eq!(set.bits(), 0x6a);
            assert_eq!(InitFlags::default(), InitFlags::empty());
            assert!(InitFlags::default().is_empty());
        }

        #[test]
        fn flags_keep_or_drop_unnamed_bits() {
            // Bit 2 sits between `NoReinit` and `Mkdir`; 16..18 are init_ext's own.
            let raw = 0x7_0000 | 0x4 | InitFlag::Bare.raw();
            let kept = InitFlags::from_bits_retain(raw);
            assert_eq!(kept.bits(), raw);
            assert_eq!(kept.unnamed_bits(), 0x7_0004);
            assert!(!kept.is_empty());
            let truncated = InitFlags::from_bits_truncate(raw);
            assert_eq!(truncated, InitFlag::Bare.into());
            assert_eq!(truncated.unnamed_bits(), 0);
            assert_eq!(InitFlags::from_bits_truncate(u32::MAX).bits(), 0x7b);
            assert!(InitFlags::from_bits_retain(0x4).iter().next().is_none());
        }

        #[test]
        fn flags_iter_is_fused_and_bounded() {
            let mut it = InitFlags::from_bits_retain(u32::MAX).iter();
            assert_eq!(it.size_hint(), (0, Some(6)));
            assert_eq!(it.by_ref().count(), 6);
            assert_eq!(it.size_hint(), (0, Some(0)));
            assert_eq!(it.next(), None);
        }

        #[test]
        fn unnamed_flag_reports_its_value() {
            let e = InitFlag::try_from(4).unwrap_err();
            assert_eq!(e.raw(), 4);
            assert_eq!(
                e.to_string(),
                "libgit2 repository init flag 0x4 is not named by git2/repository.h"
            );
            assert_eq!(InitFlag::try_from(0x20), Ok(InitFlag::ExternalTemplate));
        }

        #[test]
        fn modes_name_the_constants_and_keep_custom_bits() {
            assert_eq!(InitMode::from_raw(0), InitMode::SharedUmask);
            assert_eq!(InitMode::from_raw(0o2775), InitMode::SharedGroup);
            assert_eq!(InitMode::from_raw(0o2777), InitMode::SharedAll);
            assert_eq!(InitMode::default(), InitMode::SharedUmask);
            // The group mode without its setgid bit is a custom mode, not the group's.
            let custom = InitMode::from(0o775);
            let InitMode::Custom(mode) = custom else {
                panic!("{custom:?}")
            };
            assert_eq!(mode.bits(), 0o775);
            assert_eq!(u32::from(custom), 0o775);
            for raw in [1, 0o700, 0o2776, 0o7777, u32::MAX] {
                assert_eq!(InitMode::from_raw(raw).raw(), raw);
                assert!(matches!(InitMode::from_raw(raw), InitMode::Custom(_)));
            }
        }
    }
}
