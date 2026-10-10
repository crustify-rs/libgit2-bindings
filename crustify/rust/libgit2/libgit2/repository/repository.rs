//! Safe wrappers over `include/git2/repository.h`.
//!
//! # Layout items and open flags
//!
//! An [`Item`] is a `git_repository_item_t`: one file or directory of a repository's
//! layout, which `git_repository_item_path` resolves against the repository's git,
//! common or working directory. libgit2 indexes a static table of
//! `GIT_REPOSITORY_ITEM__LAST` entries with the value unchecked, so the sentinel and
//! every value past it read beyond that table: [`Item`] has no variant for them, and
//! [`Item::from_raw`] refuses them.
//!
//! An [`OpenFlag`] is a `git_repository_open_flag_t`, one bit of the `unsigned int`
//! that `git_repository_open_ext` takes; an [`OpenFlags`] set ORs them. libgit2 only
//! tests the bits it names and ignores the others, so a set keeps any bit it is given
//! ([`OpenFlags::from_bits_retain`]) and passes it through unchanged.
//!
//! # Repository state
//!
//! A [`RepositoryState`] is a `git_repository_state_t`: the multi-step operation a
//! repository is in the middle of, as `git_repository_state` reads it from the marker
//! files git leaves in the git directory. Its one producer returns it as an `int`, and
//! `-1` when it cannot build the path it probes, so a raw value is converted fallibly
//! ([`RepositoryState::from_raw`] and `TryFrom`) and never transmuted.

use core::ffi::c_uint;
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

/// One file or directory of a repository's layout, as `git_repository_item_path`
/// resolves it: against the git directory ([`GitDir`](Self::GitDir), the `.git`
/// directory or a bare repository itself), the working directory
/// ([`WorkDir`](Self::WorkDir)), or the common directory a worktree shares with its
/// main repository ([`CommonDir`](Self::CommonDir)), falling back to the git directory
/// for a repository without one. A directory resolves with a trailing `/`; an item
/// whose path cannot exist (the working directory of a bare repository) resolves to
/// `GIT_ENOTFOUND`.
///
/// [`from_raw`](Self::from_raw) gives `None` for `GIT_REPOSITORY_ITEM__LAST` and every
/// value after it, which libgit2 would read past its item table with.
///
/// Wraps: git_repository_item_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum Item {
    /// The git directory, `<gitdir>/`.
    GitDir = ffi::GIT_REPOSITORY_ITEM_GITDIR,
    /// The working directory, `<workdir>/`; not found in a bare repository.
    WorkDir = ffi::GIT_REPOSITORY_ITEM_WORKDIR,
    /// The common directory, `<commondir>/`: the main repository's git directory for
    /// a worktree, the git directory otherwise.
    CommonDir = ffi::GIT_REPOSITORY_ITEM_COMMONDIR,
    /// The index file, `<gitdir>/index`.
    Index = ffi::GIT_REPOSITORY_ITEM_INDEX,
    /// The object database, `<commondir>/objects/`.
    Objects = ffi::GIT_REPOSITORY_ITEM_OBJECTS,
    /// The loose references, `<commondir>/refs/`.
    Refs = ffi::GIT_REPOSITORY_ITEM_REFS,
    /// The packed references file, `<commondir>/packed-refs`.
    PackedRefs = ffi::GIT_REPOSITORY_ITEM_PACKED_REFS,
    /// The legacy remotes directory, `<commondir>/remotes/`.
    Remotes = ffi::GIT_REPOSITORY_ITEM_REMOTES,
    /// The repository's configuration file, `<commondir>/config`.
    Config = ffi::GIT_REPOSITORY_ITEM_CONFIG,
    /// The info directory, `<commondir>/info/`.
    Info = ffi::GIT_REPOSITORY_ITEM_INFO,
    /// The hooks directory, `<commondir>/hooks/`.
    Hooks = ffi::GIT_REPOSITORY_ITEM_HOOKS,
    /// The reflogs, `<commondir>/logs/`.
    Logs = ffi::GIT_REPOSITORY_ITEM_LOGS,
    /// The submodules' git directories, `<gitdir>/modules/`.
    Modules = ffi::GIT_REPOSITORY_ITEM_MODULES,
    /// The worktrees' administrative directories, `<commondir>/worktrees/`.
    Worktrees = ffi::GIT_REPOSITORY_ITEM_WORKTREES,
    /// The per-worktree configuration file, `<gitdir>/config.worktree`.
    WorktreeConfig = ffi::GIT_REPOSITORY_ITEM_WORKTREE_CONFIG,
}

impl Item {
    /// Every variant, in C value order: `ALL[n]` is the item whose value is `n`.
    pub const ALL: [Item; 15] = [
        Self::GitDir,
        Self::WorkDir,
        Self::CommonDir,
        Self::Index,
        Self::Objects,
        Self::Refs,
        Self::PackedRefs,
        Self::Remotes,
        Self::Config,
        Self::Info,
        Self::Hooks,
        Self::Logs,
        Self::Modules,
        Self::Worktrees,
        Self::WorktreeConfig,
    ];

    /// The variant whose C value is `raw`; `None` for `GIT_REPOSITORY_ITEM__LAST` and
    /// every value after it.
    pub const fn from_raw(raw: ffi::git_repository_item_t) -> Option<Self> {
        // `ALL` is indexed by value and ends at `__LAST`, the bound of libgit2's table.
        const {
            assert!(Item::ALL.len() == ffi::GIT_REPOSITORY_ITEM__LAST as usize);
            let mut i = 0;
            while i < Item::ALL.len() {
                assert!(Item::ALL[i] as usize == i);
                i += 1;
            }
        }
        if raw < ffi::GIT_REPOSITORY_ITEM__LAST {
            Some(Self::ALL[raw as usize])
        } else {
            None
        }
    }

    /// The C value, as `git_repository_item_path` takes it; always below
    /// `GIT_REPOSITORY_ITEM__LAST`.
    pub const fn raw(self) -> ffi::git_repository_item_t {
        self as ffi::git_repository_item_t
    }
}

impl TryFrom<ffi::git_repository_item_t> for Item {
    type Error = UnnamedItem;

    fn try_from(raw: ffi::git_repository_item_t) -> Result<Self, UnnamedItem> {
        Self::from_raw(raw).ok_or(UnnamedItem(raw))
    }
}

impl From<Item> for ffi::git_repository_item_t {
    fn from(value: Item) -> ffi::git_repository_item_t {
        value.raw()
    }
}

/// A repository item value that names no item: `GIT_REPOSITORY_ITEM__LAST` or a value
/// after it. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedItem(ffi::git_repository_item_t);

impl UnnamedItem {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_repository_item_t {
        self.0
    }
}

impl fmt::Display for UnnamedItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 repository item {} is not named by git2/repository.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedItem {}

/// One option of `git_repository_open_ext`'s search for a repository, a single bit of
/// an [`OpenFlags`] set.
///
/// Without any, the search starts at the given path, tries `<path>/.git` and then
/// `<path>` itself, and walks up through the parent directories until a ceiling
/// directory, staying on the starting file system.
///
/// [`from_raw`](Self::from_raw) gives `None` for any value without a variant: `0`, a
/// combination of bits, or a bit this version of libgit2's header does not name.
///
/// Wraps: git_repository_open_flag_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum OpenFlag {
    /// Only look at the starting path (and its `.git`); do not walk up through its
    /// parent directories.
    NoSearch = ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
    /// Keep walking up across file system boundaries (a change of `st_dev`).
    CrossFs = ffi::GIT_REPOSITORY_OPEN_CROSS_FS,
    /// Open the repository as bare whatever its `core.bare`, deferring the loading of
    /// its configuration; unlike `git_repository_open_bare`, a gitlink is followed.
    /// The search tries the starting path itself and not its `.git`; the parents it
    /// walks up to are still tried with `.git` first.
    Bare = ffi::GIT_REPOSITORY_OPEN_BARE,
    /// Never append `/.git`: only a git directory itself is a repository.
    NoDotGit = ffi::GIT_REPOSITORY_OPEN_NO_DOTGIT,
    /// Respect the git command-line tools' environment variables: `GIT_DIR` (for a
    /// NULL path, which it then opens as a git directory without searching),
    /// `GIT_CEILING_DIRECTORIES` (for NULL ceiling directories),
    /// `GIT_DISCOVERY_ACROSS_FILESYSTEM` (a boolean, adding [`CrossFs`](Self::CrossFs);
    /// any other value fails the open), `GIT_COMMON_DIR`, `GIT_WORK_TREE`,
    /// `GIT_INDEX_FILE`, `GIT_NAMESPACE`, `GIT_OBJECT_DIRECTORY`,
    /// `GIT_ALTERNATE_OBJECT_DIRECTORIES` and the `GIT_CONFIG_*` paths. Unlike what
    /// libgit2's header says, the other flags and given ceiling directories still
    /// apply.
    FromEnv = ffi::GIT_REPOSITORY_OPEN_FROM_ENV,
}

impl OpenFlag {
    /// Every variant, in ascending bit order.
    pub const ALL: [OpenFlag; 5] = [
        Self::NoSearch,
        Self::CrossFs,
        Self::Bare,
        Self::NoDotGit,
        Self::FromEnv,
    ];

    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_repository_open_flag_t) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i] as ffi::git_repository_open_flag_t == raw {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }

    /// The C value, a single bit.
    pub const fn raw(self) -> ffi::git_repository_open_flag_t {
        self as ffi::git_repository_open_flag_t
    }

    /// The flag's bit in the `unsigned int` `git_repository_open_ext` takes.
    const fn bit(self) -> c_uint {
        self as c_uint
    }
}

impl TryFrom<ffi::git_repository_open_flag_t> for OpenFlag {
    type Error = UnnamedOpenFlag;

    fn try_from(raw: ffi::git_repository_open_flag_t) -> Result<Self, UnnamedOpenFlag> {
        Self::from_raw(raw).ok_or(UnnamedOpenFlag(raw))
    }
}

impl From<OpenFlag> for ffi::git_repository_open_flag_t {
    fn from(value: OpenFlag) -> ffi::git_repository_open_flag_t {
        value.raw()
    }
}

/// An open flag value that this version of libgit2's header does not name: `0`, a
/// combination of flags, or a later header's bit. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedOpenFlag(ffi::git_repository_open_flag_t);

impl UnnamedOpenFlag {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_repository_open_flag_t {
        self.0
    }
}

impl fmt::Display for UnnamedOpenFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 repository open flag {} is not named by git2/repository.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedOpenFlag {}

/// A set of [`OpenFlag`]s: the `unsigned int` combination of `GIT_REPOSITORY_OPEN_*`
/// bits that `git_repository_open_ext` takes. The empty set is the default search.
///
/// It keeps every bit it is given, named or not, so [`bits`](Self::bits) passes on
/// what it was built from; libgit2 ignores the bits no [`OpenFlag`] names.
/// [`iter`](Self::iter) and [`contains`](Self::contains) see the named flags, and
/// [`unnamed_bits`](Self::unnamed_bits) the rest.
///
/// Wraps: git_repository_open_flag_t
/// Handwritten
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OpenFlags(c_uint);

impl OpenFlags {
    /// The bits of every [`OpenFlag`].
    const NAMED: c_uint = {
        let mut bits = 0;
        let mut i = 0;
        while i < OpenFlag::ALL.len() {
            bits |= OpenFlag::ALL[i].bit();
            i += 1;
        }
        bits
    };

    /// The set with no flag: the default search.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The set of every bit of `bits`; bits that no [`OpenFlag`] names are kept.
    pub const fn from_bits_retain(bits: c_uint) -> Self {
        Self(bits)
    }

    /// The bits of the set, as `git_repository_open_ext` takes them.
    pub const fn bits(self) -> c_uint {
        self.0
    }

    /// The bits of the set that no [`OpenFlag`] names.
    pub const fn unnamed_bits(self) -> c_uint {
        self.0 & !Self::NAMED
    }

    /// Whether the set has no bit, named or not.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `flag` is in the set.
    pub const fn contains(self, flag: OpenFlag) -> bool {
        self.0 & flag.bit() != 0
    }

    /// Adds `flag` to the set.
    pub fn insert(&mut self, flag: OpenFlag) {
        self.0 |= flag.bit();
    }

    /// Removes `flag` from the set.
    pub fn remove(&mut self, flag: OpenFlag) {
        self.0 &= !flag.bit();
    }

    /// The named flags in the set, in ascending bit order.
    pub fn iter(self) -> OpenFlagsIter {
        OpenFlagsIter { set: self, next: 0 }
    }
}

impl From<OpenFlag> for OpenFlags {
    fn from(flag: OpenFlag) -> Self {
        Self(flag.bit())
    }
}

impl BitOr for OpenFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<OpenFlag> for OpenFlags {
    type Output = Self;

    fn bitor(self, rhs: OpenFlag) -> Self {
        self | Self::from(rhs)
    }
}

impl BitOr for OpenFlag {
    type Output = OpenFlags;

    fn bitor(self, rhs: Self) -> OpenFlags {
        OpenFlags::from(self) | rhs
    }
}

impl BitOrAssign for OpenFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitOrAssign<OpenFlag> for OpenFlags {
    fn bitor_assign(&mut self, rhs: OpenFlag) {
        self.insert(rhs);
    }
}

impl FromIterator<OpenFlag> for OpenFlags {
    fn from_iter<I: IntoIterator<Item = OpenFlag>>(iter: I) -> Self {
        let mut set = Self::empty();
        set.extend(iter);
        set
    }
}

impl Extend<OpenFlag> for OpenFlags {
    fn extend<I: IntoIterator<Item = OpenFlag>>(&mut self, iter: I) {
        for flag in iter {
            self.insert(flag);
        }
    }
}

impl IntoIterator for OpenFlags {
    type Item = OpenFlag;
    type IntoIter = OpenFlagsIter;

    fn into_iter(self) -> OpenFlagsIter {
        self.iter()
    }
}

/// Lists the named flags, then any unnamed bits in hex: `{NoSearch | Bare, 0x20}`.
impl fmt::Debug for OpenFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OpenFlags {")?;
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

/// The named flags of an [`OpenFlags`] set, in ascending bit order
/// ([`OpenFlags::iter`]).
///
/// Handwritten
#[derive(Clone, Debug)]
pub struct OpenFlagsIter {
    set: OpenFlags,
    /// The index into [`OpenFlag::ALL`] of the next flag to test.
    next: usize,
}

impl Iterator for OpenFlagsIter {
    type Item = OpenFlag;

    fn next(&mut self) -> Option<OpenFlag> {
        while let Some(&flag) = OpenFlag::ALL.get(self.next) {
            self.next += 1;
            if self.set.contains(flag) {
                return Some(flag);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(OpenFlag::ALL.len().saturating_sub(self.next)))
    }
}

impl core::iter::FusedIterator for OpenFlagsIter {}

/// The operation a repository is in the middle of: a merge, revert, cherry-pick,
/// bisect, rebase or mailbox application that has stopped, typically on a conflict,
/// and left its marker files in the git directory for a later step to resume.
///
/// `git_repository_state` probes the markers in a fixed order and reports the first it
/// finds, so a repository carrying several (an interrupted rebase whose last step
/// stopped on a merge) reports the earliest-probed one. The order is
/// [`RebaseInteractive`](Self::RebaseInteractive), [`RebaseMerge`](Self::RebaseMerge),
/// [`Rebase`](Self::Rebase), [`ApplyMailbox`](Self::ApplyMailbox),
/// [`ApplyMailboxOrRebase`](Self::ApplyMailboxOrRebase), [`Merge`](Self::Merge), the
/// revert states, the cherry-pick states, then [`Bisect`](Self::Bisect); a sequence
/// state refines its single-commit one. `git_repository_state_cleanup` removes every
/// marker, the rebase directories and the sequencer included, which leaves the
/// repository [`Clean`](Self::Clean).
///
/// [`from_raw`](Self::from_raw) gives `None` for every value the header does not name.
///
/// Wraps: git_repository_state_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum RepositoryState {
    /// No operation is in progress (`GIT_REPOSITORY_STATE_NONE`).
    Clean = ffi::GIT_REPOSITORY_STATE_NONE,
    /// A merge has stopped: `MERGE_HEAD` exists.
    Merge = ffi::GIT_REPOSITORY_STATE_MERGE,
    /// A revert of one commit has stopped: the `REVERT_HEAD` reference resolves.
    Revert = ffi::GIT_REPOSITORY_STATE_REVERT,
    /// A revert of a series of commits has stopped: `REVERT_HEAD` resolves and
    /// `sequencer/todo` exists.
    RevertSequence = ffi::GIT_REPOSITORY_STATE_REVERT_SEQUENCE,
    /// A cherry-pick of one commit has stopped: the `CHERRY_PICK_HEAD` reference
    /// resolves.
    CherryPick = ffi::GIT_REPOSITORY_STATE_CHERRYPICK,
    /// A cherry-pick of a series of commits has stopped: `CHERRY_PICK_HEAD` resolves
    /// and `sequencer/todo` exists.
    CherryPickSequence = ffi::GIT_REPOSITORY_STATE_CHERRYPICK_SEQUENCE,
    /// A bisection is in progress: `BISECT_LOG` exists.
    Bisect = ffi::GIT_REPOSITORY_STATE_BISECT,
    /// A patch-based rebase has stopped: `rebase-apply/rebasing` exists.
    Rebase = ffi::GIT_REPOSITORY_STATE_REBASE,
    /// An interactive rebase has stopped: `rebase-merge/interactive` exists.
    RebaseInteractive = ffi::GIT_REPOSITORY_STATE_REBASE_INTERACTIVE,
    /// A merge-based rebase has stopped: the `rebase-merge/` directory exists.
    RebaseMerge = ffi::GIT_REPOSITORY_STATE_REBASE_MERGE,
    /// A mailbox application (`git am`) has stopped: `rebase-apply/applying` exists.
    ApplyMailbox = ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX,
    /// A mailbox application or a patch-based rebase has stopped: the `rebase-apply/`
    /// directory exists, without the file that tells the two apart.
    ApplyMailboxOrRebase = ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE,
}

impl RepositoryState {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_repository_state_t) -> Option<Self> {
        match raw {
            ffi::GIT_REPOSITORY_STATE_NONE => Some(Self::Clean),
            ffi::GIT_REPOSITORY_STATE_MERGE => Some(Self::Merge),
            ffi::GIT_REPOSITORY_STATE_REVERT => Some(Self::Revert),
            ffi::GIT_REPOSITORY_STATE_REVERT_SEQUENCE => Some(Self::RevertSequence),
            ffi::GIT_REPOSITORY_STATE_CHERRYPICK => Some(Self::CherryPick),
            ffi::GIT_REPOSITORY_STATE_CHERRYPICK_SEQUENCE => Some(Self::CherryPickSequence),
            ffi::GIT_REPOSITORY_STATE_BISECT => Some(Self::Bisect),
            ffi::GIT_REPOSITORY_STATE_REBASE => Some(Self::Rebase),
            ffi::GIT_REPOSITORY_STATE_REBASE_INTERACTIVE => Some(Self::RebaseInteractive),
            ffi::GIT_REPOSITORY_STATE_REBASE_MERGE => Some(Self::RebaseMerge),
            ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX => Some(Self::ApplyMailbox),
            ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE => Some(Self::ApplyMailboxOrRebase),
            _ => None,
        }
    }

    /// The C value, as libgit2 reports it.
    pub const fn raw(self) -> ffi::git_repository_state_t {
        self as ffi::git_repository_state_t
    }

    /// Whether no operation is in progress: the state is [`Clean`](Self::Clean).
    pub const fn is_clean(self) -> bool {
        matches!(self, Self::Clean)
    }
}

impl TryFrom<ffi::git_repository_state_t> for RepositoryState {
    type Error = UnnamedRepositoryState;

    fn try_from(raw: ffi::git_repository_state_t) -> Result<Self, UnnamedRepositoryState> {
        Self::from_raw(raw).ok_or(UnnamedRepositoryState(raw))
    }
}

impl From<RepositoryState> for ffi::git_repository_state_t {
    fn from(value: RepositoryState) -> ffi::git_repository_state_t {
        value.raw()
    }
}

/// A repository state that this version of libgit2's header does not name.
/// [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedRepositoryState(ffi::git_repository_state_t);

impl UnnamedRepositoryState {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_repository_state_t {
        self.0
    }
}

impl fmt::Display for UnnamedRepositoryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 repository state {} is not named by git2/repository.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedRepositoryState {}

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

    mod items_and_open_flags {
        use super::super::*;

        #[test]
        fn items_are_indexed_by_value() {
            for (i, item) in Item::ALL.into_iter().enumerate() {
                assert_eq!(item.raw() as usize, i, "{item:?}");
                assert_eq!(Item::from_raw(item.raw()), Some(item));
                assert_eq!(Item::try_from(item.raw()), Ok(item));
                assert_eq!(ffi::git_repository_item_t::from(item), item.raw());
            }
            // `Ord` is the C value order.
            assert!(Item::ALL.windows(2).all(|w| w[0] < w[1]));
        }

        #[test]
        fn item_rejects_the_sentinel_and_beyond() {
            for raw in [
                ffi::GIT_REPOSITORY_ITEM__LAST,
                ffi::GIT_REPOSITORY_ITEM__LAST + 1,
                u32::MAX,
            ] {
                assert_eq!(Item::from_raw(raw), None);
                let err = Item::try_from(raw).unwrap_err();
                assert_eq!(err.raw(), raw);
                assert_eq!(
                    err.to_string(),
                    format!("libgit2 repository item {raw} is not named by git2/repository.h")
                );
            }
        }

        #[test]
        fn open_flag_bits_are_distinct_single_bits() {
            let mut seen = 0;
            for (i, flag) in OpenFlag::ALL.into_iter().enumerate() {
                assert_eq!(flag.raw(), 1 << i, "{flag:?}");
                assert_eq!(flag.bit() & seen, 0);
                seen |= flag.bit();
            }
            assert_eq!(OpenFlags::NAMED, seen);
            assert_eq!(OpenFlags::NAMED, 0x1f);
        }

        #[test]
        fn open_flag_conversions_round_trip() {
            for flag in OpenFlag::ALL {
                assert_eq!(OpenFlag::from_raw(flag.raw()), Some(flag));
                assert_eq!(OpenFlag::try_from(flag.raw()), Ok(flag));
                assert_eq!(ffi::git_repository_open_flag_t::from(flag), flag.raw());
            }
            for raw in [0, 3, 1 << 5, u32::MAX] {
                assert_eq!(OpenFlag::from_raw(raw), None);
                let err = OpenFlag::try_from(raw).unwrap_err();
                assert_eq!(err.raw(), raw);
                assert_eq!(
                    err.to_string(),
                    format!("libgit2 repository open flag {raw} is not named by git2/repository.h")
                );
            }
        }

        #[test]
        fn open_flags_build_and_take_apart() {
            let mut set = OpenFlags::default();
            assert!(set.is_empty());
            assert_eq!(set, OpenFlags::empty());
            set |= OpenFlag::Bare;
            set.insert(OpenFlag::NoSearch);
            assert_eq!(set.bits(), 0b101);
            assert_eq!(set, OpenFlag::NoSearch | OpenFlag::Bare);
            assert_eq!(set, OpenFlags::from(OpenFlag::Bare) | OpenFlag::NoSearch);
            assert_eq!(
                set.iter().collect::<Vec<_>>(),
                [OpenFlag::NoSearch, OpenFlag::Bare]
            );
            set.remove(OpenFlag::NoSearch);
            set.remove(OpenFlag::NoSearch);
            assert_eq!(set.into_iter().collect::<Vec<_>>(), [OpenFlag::Bare]);
            let mut more = OpenFlags::from_bits_retain(1 << 7);
            more |= set;
            assert_eq!(more.bits(), 1 << 7 | 0b100);
            assert_eq!(more.unnamed_bits(), 1 << 7);
            assert_eq!(
                OpenFlag::ALL.into_iter().collect::<OpenFlags>().bits(),
                0x1f
            );
        }

        #[test]
        fn open_flags_iter_skips_unnamed_bits_and_fuses() {
            let mut iter = OpenFlags::from_bits_retain(u32::MAX).iter();
            assert_eq!(iter.size_hint(), (0, Some(5)));
            assert_eq!(iter.by_ref().count(), 5);
            assert_eq!(iter.size_hint(), (0, Some(0)));
            assert_eq!(iter.next(), None);
            assert_eq!(OpenFlags::from_bits_retain(!0x1f).iter().next(), None);
        }

        /// A formatter target refusing every write after the first `ok`.
        struct FailAfter {
            ok: usize,
        }

        impl fmt::Write for FailAfter {
            fn write_str(&mut self, _: &str) -> fmt::Result {
                self.ok = self.ok.checked_sub(1).ok_or(fmt::Error)?;
                Ok(())
            }
        }

        #[test]
        fn open_flags_debug_passes_on_a_refused_write() {
            use core::fmt::Write;
            let set = OpenFlags::from_bits_retain(1 << 5 | 0b11);
            let writes = {
                let mut all = FailAfter { ok: usize::MAX };
                write!(all, "{set:?}").unwrap();
                usize::MAX - all.ok
            };
            for ok in 0..writes {
                assert_eq!(write!(FailAfter { ok }, "{set:?}"), Err(fmt::Error), "{ok}");
            }
        }

        #[test]
        fn open_flags_debug_names_flags_then_unnamed_bits() {
            assert_eq!(format!("{:?}", OpenFlags::empty()), "OpenFlags {}");
            assert_eq!(
                format!("{:?}", OpenFlag::NoSearch | OpenFlag::Bare),
                "OpenFlags {NoSearch | Bare}"
            );
            assert_eq!(
                format!("{:?}", OpenFlags::from_bits_retain(1 << 5 | 1)),
                "OpenFlags {NoSearch, 0x20}"
            );
            assert_eq!(
                format!("{:?}", OpenFlags::from_bits_retain(1 << 5)),
                "OpenFlags {0x20}"
            );
        }
    }

    mod state {
        use super::super::*;
        use std::collections::HashSet;
        use std::error::Error;

        /// Every `git_repository_state_t` constant, beside the variant that must carry it.
        const STATES: [(ffi::git_repository_state_t, RepositoryState); 12] = [
            (ffi::GIT_REPOSITORY_STATE_NONE, RepositoryState::Clean),
            (ffi::GIT_REPOSITORY_STATE_MERGE, RepositoryState::Merge),
            (ffi::GIT_REPOSITORY_STATE_REVERT, RepositoryState::Revert),
            (
                ffi::GIT_REPOSITORY_STATE_REVERT_SEQUENCE,
                RepositoryState::RevertSequence,
            ),
            (
                ffi::GIT_REPOSITORY_STATE_CHERRYPICK,
                RepositoryState::CherryPick,
            ),
            (
                ffi::GIT_REPOSITORY_STATE_CHERRYPICK_SEQUENCE,
                RepositoryState::CherryPickSequence,
            ),
            (ffi::GIT_REPOSITORY_STATE_BISECT, RepositoryState::Bisect),
            (ffi::GIT_REPOSITORY_STATE_REBASE, RepositoryState::Rebase),
            (
                ffi::GIT_REPOSITORY_STATE_REBASE_INTERACTIVE,
                RepositoryState::RebaseInteractive,
            ),
            (
                ffi::GIT_REPOSITORY_STATE_REBASE_MERGE,
                RepositoryState::RebaseMerge,
            ),
            (
                ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX,
                RepositoryState::ApplyMailbox,
            ),
            (
                ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE,
                RepositoryState::ApplyMailboxOrRebase,
            ),
        ];

        /// Every value around the declared ones, and the extremes; `u32::MAX` is the
        /// `-1` `git_repository_state` returns on failure, read as the enum's type.
        fn probes() -> impl Iterator<Item = ffi::git_repository_state_t> {
            (0..=300).chain([u32::MAX - 1, u32::MAX])
        }

        #[test]
        fn variants_carry_their_c_values() {
            for (raw, state) in STATES {
                assert_eq!(state.raw(), raw);
                assert_eq!(ffi::git_repository_state_t::from(state), raw);
                assert_eq!(RepositoryState::from_raw(raw), Some(state));
                assert_eq!(RepositoryState::try_from(raw), Ok(state));
            }
            let distinct: HashSet<_> = STATES.iter().map(|(_, s)| *s).collect();
            assert_eq!(distinct.len(), STATES.len());
        }

        #[test]
        fn unnamed_values_are_refused_with_their_value() {
            for raw in probes() {
                match STATES.iter().find(|(r, _)| *r == raw) {
                    Some((_, state)) => assert_eq!(RepositoryState::from_raw(raw), Some(*state)),
                    None => {
                        assert_eq!(RepositoryState::from_raw(raw), None);
                        let err = RepositoryState::try_from(raw).unwrap_err();
                        assert_eq!(err, UnnamedRepositoryState(raw));
                        assert_eq!(err.raw(), raw);
                        assert_eq!(
                            err.to_string(),
                            format!(
                                "libgit2 repository state {raw} is not named by git2/repository.h"
                            )
                        );
                        assert!(err.source().is_none());
                    }
                }
            }
        }

        #[test]
        fn only_clean_is_clean() {
            for (raw, state) in STATES {
                assert_eq!(state.is_clean(), raw == ffi::GIT_REPOSITORY_STATE_NONE);
            }
        }

        #[test]
        fn option_niche_never_aliases_a_variant() {
            // `Option<RepositoryState>` stores `None` in a value no variant uses.
            assert_eq!(
                size_of::<Option<RepositoryState>>(),
                size_of::<ffi::git_repository_state_t>()
            );
            for raw in probes() {
                let state = core::hint::black_box(RepositoryState::from_raw(raw));
                assert_eq!(state.map(RepositoryState::raw).unwrap_or(raw), raw);
            }
        }

        #[test]
        fn debug_names_the_variant() {
            assert_eq!(format!("{:?}", RepositoryState::Clean), "Clean");
            assert_eq!(
                format!("{:?}", RepositoryState::CherryPickSequence),
                "CherryPickSequence"
            );
            assert_eq!(
                format!("{:?}", UnnamedRepositoryState(12)),
                "UnnamedRepositoryState(12)"
            );
        }
    }
}
